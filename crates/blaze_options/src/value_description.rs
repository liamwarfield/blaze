//! `OptionValueDescription`: tracks the final value as instances of an option are parsed.

use std::collections::BTreeMap;
use std::sync::Arc;

use indexmap::IndexSet;

use crate::converter::ConversionContext;
use crate::data::OptionsData;
use crate::definition::OptionDefinitionRef;
use crate::exception::{ConstructionException, OptionsParsingException};
use crate::parsed::ParsedOptionRef;
use crate::priority::{OptionPriority, PriorityCategory};
use crate::value::Value;

// Java: com/google/devtools/common/options/OptionValueDescription.java:74 OptionValueDescription.ExpansionBundle
/// Arguments that need to be parsed further, with an option-appropriate source string.
#[derive(Clone, Debug)]
pub struct ExpansionBundle {
    pub expansion_args: Vec<String>,
    pub source_of_expansion_args: String,
}

/// State of a single-valued option (shared by plain and implicit-requirement options).
#[derive(Clone, Debug)]
pub struct SingleValue {
    effective_option_instance: Option<ParsedOptionRef>,
    effective_value: Option<Value>,
    contains_errors: bool,
}

// Java: com/google/devtools/common/options/OptionValueDescription.java:37 OptionValueDescription
/// The value of an option. The variants mirror the Java subclasses.
#[derive(Clone, Debug)]
pub enum OptionValueDescription {
    // Java: com/google/devtools/common/options/OptionValueDescription.java:123 OptionValueDescription.DefaultOptionValueDescription
    Default {
        option_definition: OptionDefinitionRef,
        conversion_context: Option<Arc<ConversionContext>>,
    },
    // Java: com/google/devtools/common/options/OptionValueDescription.java:162 OptionValueDescription.SingleOptionValueDescription
    Single {
        option_definition: OptionDefinitionRef,
        conversion_context: Option<Arc<ConversionContext>>,
        state: SingleValue,
    },
    // Java: com/google/devtools/common/options/OptionValueDescription.java:303 OptionValueDescription.RepeatableOptionValueDescription
    Repeatable {
        option_definition: OptionDefinitionRef,
        parsed_options: BTreeMap<OptionPriority, Vec<ParsedOptionRef>>,
        option_values: BTreeMap<OptionPriority, Vec<Option<Value>>>,
        contains_errors: bool,
    },
    // Java: com/google/devtools/common/options/OptionValueDescription.java:389 OptionValueDescription.ExpansionOptionValueDescription
    Expansion {
        option_definition: OptionDefinitionRef,
        expansion: Vec<String>,
    },
    // Java: com/google/devtools/common/options/OptionValueDescription.java:447 OptionValueDescription.OptionWithImplicitRequirementsValueDescription
    WithImplicitRequirements {
        option_definition: OptionDefinitionRef,
        conversion_context: Option<Arc<ConversionContext>>,
        state: SingleValue,
    },
}

impl OptionValueDescription {
    // Java: com/google/devtools/common/options/OptionValueDescription.java:101 OptionValueDescription.createOptionValueDescription
    pub fn create_option_value_description(
        option: OptionDefinitionRef,
        options_data: &OptionsData,
        conversion_context: Option<Arc<ConversionContext>>,
    ) -> OptionValueDescription {
        let single = || SingleValue {
            effective_option_instance: None,
            effective_value: None,
            contains_errors: false,
        };
        if option.is_expansion_option() {
            OptionValueDescription::Expansion {
                option_definition: option,
                expansion: options_data.get_evaluated_expansion(option),
            }
        } else if option.allows_multiple() {
            OptionValueDescription::Repeatable {
                option_definition: option,
                parsed_options: BTreeMap::new(),
                option_values: BTreeMap::new(),
                contains_errors: false,
            }
        } else if option.has_implicit_requirements() {
            OptionValueDescription::WithImplicitRequirements {
                option_definition: option,
                conversion_context,
                state: single(),
            }
        } else {
            OptionValueDescription::Single {
                option_definition: option,
                conversion_context,
                state: single(),
            }
        }
    }

    // Java: com/google/devtools/common/options/OptionValueDescription.java:118 OptionValueDescription.getDefaultOptionValue
    pub fn get_default_option_value(
        option: OptionDefinitionRef,
        conversion_context: Option<Arc<ConversionContext>>,
    ) -> OptionValueDescription {
        OptionValueDescription::Default {
            option_definition: option,
            conversion_context,
        }
    }

    pub fn get_option_definition(&self) -> OptionDefinitionRef {
        match self {
            OptionValueDescription::Default {
                option_definition, ..
            }
            | OptionValueDescription::Single {
                option_definition, ..
            }
            | OptionValueDescription::Repeatable {
                option_definition, ..
            }
            | OptionValueDescription::Expansion {
                option_definition, ..
            }
            | OptionValueDescription::WithImplicitRequirements {
                option_definition, ..
            } => option_definition,
        }
    }

    /// Returns the current or final value of this option.
    pub fn get_value(&self) -> Option<Value> {
        match self {
            // Java: com/google/devtools/common/options/OptionValueDescription.java:131 OptionValueDescription.DefaultOptionValueDescription.getValue
            OptionValueDescription::Default {
                option_definition,
                conversion_context,
            } => option_definition.get_default_value(conversion_context.as_deref()),
            OptionValueDescription::Single { state, .. }
            | OptionValueDescription::WithImplicitRequirements { state, .. } => {
                state.effective_value.clone()
            }
            // Java: com/google/devtools/common/options/OptionValueDescription.java:332 OptionValueDescription.RepeatableOptionValueDescription.getValue
            OptionValueDescription::Repeatable { option_values, .. } => Some(Value::list(
                option_values
                    .values()
                    .flatten()
                    .map(|v| {
                        v.clone()
                            .expect("NullPointerException: null element in ImmutableList")
                    })
                    .collect(),
            )),
            OptionValueDescription::Expansion { .. } => None,
        }
    }

    /// `getValue()` of a default value, returning the `ConstructionException` instead of
    /// throwing it.
    pub(crate) fn try_get_value(&self) -> Result<Option<Value>, ConstructionException> {
        match self {
            OptionValueDescription::Default {
                option_definition,
                conversion_context,
            } => option_definition.try_get_default_value(conversion_context.as_deref()),
            _ => Ok(self.get_value()),
        }
    }

    pub fn contains_errors(&self) -> bool {
        match self {
            OptionValueDescription::Default { .. } | OptionValueDescription::Expansion { .. } => {
                false
            }
            OptionValueDescription::Single { state, .. }
            | OptionValueDescription::WithImplicitRequirements { state, .. } => {
                state.contains_errors
            }
            OptionValueDescription::Repeatable {
                contains_errors, ..
            } => *contains_errors,
        }
    }

    /// Returns the source(s) of this option; if there were multiple, duplicates are removed.
    pub fn get_source_string(&self) -> Option<String> {
        match self {
            OptionValueDescription::Default { .. } | OptionValueDescription::Expansion { .. } => {
                None
            }
            OptionValueDescription::Single { state, .. }
            | OptionValueDescription::WithImplicitRequirements { state, .. } => state
                .effective_option_instance
                .as_ref()
                .expect("NullPointerException: no effective option instance")
                .get_source()
                .map(str::to_string),
            // Java: com/google/devtools/common/options/OptionValueDescription.java:321 OptionValueDescription.RepeatableOptionValueDescription.getSourceString
            OptionValueDescription::Repeatable { parsed_options, .. } => {
                let mut seen: IndexSet<String> = IndexSet::new();
                for p in parsed_options.values().flatten() {
                    seen.insert(p.get_source().unwrap_or("null").to_string());
                }
                Some(seen.into_iter().collect::<Vec<_>>().join(", "))
            }
        }
    }

    /// Returns the canonical instances of this option - the instances that affect the current
    /// value. `None` for an unset (default) value.
    pub fn get_canonical_instances(&self) -> Option<Vec<ParsedOptionRef>> {
        match self {
            OptionValueDescription::Default { .. } => None,
            // Java: com/google/devtools/common/options/OptionValueDescription.java:292 OptionValueDescription.SingleOptionValueDescription.getCanonicalInstances
            OptionValueDescription::Single { state, .. }
            | OptionValueDescription::WithImplicitRequirements { state, .. } => {
                let effective = state
                    .effective_option_instance
                    .as_ref()
                    .expect("NullPointerException: no effective option instance");
                if effective.get_implicit_dependent().is_none() {
                    Some(vec![effective.clone()])
                } else {
                    Some(vec![])
                }
            }
            // Java: com/google/devtools/common/options/OptionValueDescription.java:373 OptionValueDescription.RepeatableOptionValueDescription.getCanonicalInstances
            OptionValueDescription::Repeatable { parsed_options, .. } => Some(
                parsed_options
                    .values()
                    .flatten()
                    .filter(|d| d.get_implicit_dependent().is_none())
                    .cloned()
                    .collect(),
            ),
            OptionValueDescription::Expansion { .. } => Some(vec![]),
        }
    }

    /// Adds an instance of the option to this value; returns arguments to be parsed further.
    pub(crate) fn add_option_instance(
        &mut self,
        parsed_option: &ParsedOptionRef,
        warnings: &mut IndexSet<String>,
    ) -> Result<Option<ExpansionBundle>, OptionsParsingException> {
        match self {
            // Java: com/google/devtools/common/options/OptionValueDescription.java:146 OptionValueDescription.DefaultOptionValueDescription.addOptionInstance
            OptionValueDescription::Default { .. } => panic!(
                "IllegalStateException: Cannot add values to the default option value. Create a \
                 modifiable OptionValueDescription using createOptionValueDescription() instead."
            ),
            OptionValueDescription::Single {
                option_definition,
                state,
                ..
            } => {
                single_add_option_instance(option_definition, state, parsed_option, warnings)?;
                Ok(None)
            }
            // Java: com/google/devtools/common/options/OptionValueDescription.java:349 OptionValueDescription.RepeatableOptionValueDescription.addOptionInstance
            OptionValueDescription::Repeatable {
                parsed_options,
                option_values,
                contains_errors,
                ..
            } => {
                let converted_value = match parsed_option.get_converted_value() {
                    Ok(v) => v,
                    Err(e) => {
                        *contains_errors = true;
                        return Err(e);
                    }
                };
                let priority = parsed_option.get_priority().clone();
                parsed_options
                    .entry(priority.clone())
                    .or_default()
                    .push(parsed_option.clone());
                let values = option_values.entry(priority).or_default();
                match converted_value {
                    Some(v) if v.as_list().is_some() => {
                        values.extend(v.as_list().unwrap().iter().cloned().map(Some))
                    }
                    other => values.push(other),
                }
                Ok(None)
            }
            // Java: com/google/devtools/common/options/OptionValueDescription.java:420 OptionValueDescription.ExpansionOptionValueDescription.addOptionInstance
            OptionValueDescription::Expansion {
                option_definition,
                expansion,
            } => {
                if let Some(v) = parsed_option.get_unconverted_value()
                    && !v.is_empty()
                {
                    warnings.insert(format!(
                        "{} is an expansion option. It does not accept values, and does not \
                         change its expansion based on the value provided. Value '{}' will be \
                         ignored.",
                        option_definition, v
                    ));
                }
                Ok(Some(ExpansionBundle {
                    expansion_args: expansion.clone(),
                    source_of_expansion_args: match parsed_option.get_source() {
                        None => format!("expanded from {}", option_definition),
                        Some(source) => {
                            format!("expanded from {} (source {})", option_definition, source)
                        }
                    },
                }))
            }
            // Java: com/google/devtools/common/options/OptionValueDescription.java:461 OptionValueDescription.OptionWithImplicitRequirementsValueDescription.addOptionInstance
            OptionValueDescription::WithImplicitRequirements {
                option_definition,
                conversion_context,
                state,
            } => {
                single_add_option_instance(option_definition, state, parsed_option, warnings)?;
                if parsed_option.get_converted_value()?
                    == option_definition.get_default_value(conversion_context.as_deref())
                {
                    warnings.insert(format!(
                        "{} sets {} to its default value. Since this option has implicit \
                         requirements that are set whenever the option is explicitly provided, \
                         regardless of the value, this will behave differently than letting a \
                         default be a default. Specifically, this options expands to {{{}}}.",
                        parsed_option.get_command_line_form().unwrap_or("null"),
                        option_definition,
                        option_definition.get_implicit_requirements().join(" ")
                    ));
                }
                Ok(Some(ExpansionBundle {
                    expansion_args: option_definition
                        .get_implicit_requirements()
                        .iter()
                        .map(|s| s.to_string())
                        .collect(),
                    source_of_expansion_args: match parsed_option.get_source() {
                        None => format!("implicit requirement of {}", option_definition),
                        Some(source) => format!(
                            "implicit requirement of {} (source {})",
                            option_definition, source
                        ),
                    },
                }))
            }
        }
    }
}

fn convert_value(
    state: &mut SingleValue,
    parsed_option: &ParsedOptionRef,
) -> Result<Option<Value>, OptionsParsingException> {
    match parsed_option.get_converted_value() {
        Ok(v) => Ok(v),
        Err(e) => {
            state.contains_errors = true;
            Err(e)
        }
    }
}

fn same(a: Option<&ParsedOptionRef>, b: Option<&ParsedOptionRef>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => Arc::ptr_eq(a, b),
        (None, None) => true,
        _ => false,
    }
}

// Java: com/google/devtools/common/options/OptionValueDescription.java:209 OptionValueDescription.SingleOptionValueDescription.addOptionInstance
fn single_add_option_instance(
    option_definition: OptionDefinitionRef,
    state: &mut SingleValue,
    parsed_option: &ParsedOptionRef,
    warnings: &mut IndexSet<String>,
) -> Result<(), OptionsParsingException> {
    let Some(effective) = state.effective_option_instance.clone() else {
        state.effective_option_instance = Some(parsed_option.clone());
        state.effective_value = convert_value(state, parsed_option)?;
        return Ok(());
    };

    if parsed_option.get_priority() >= effective.get_priority() {
        let implicit_dependent = parsed_option.get_implicit_dependent();
        let expanded_from = parsed_option.get_expanded_from();
        let option_that_depends_on_effective_value = effective.get_implicit_dependent();
        let option_that_expanded_to_effective_value = effective.get_expanded_from();

        let new_value = convert_value(state, parsed_option)?;
        if let Some(effective_value) = &state.effective_value
            && Some(effective_value) != new_value.as_ref()
        {
            let same_priority_category = parsed_option.get_priority().get_priority_category()
                == effective.get_priority().get_priority_category();
            if let (Some(implicit_dependent), Some(depends)) =
                (implicit_dependent, option_that_depends_on_effective_value)
            {
                if !same(Some(implicit_dependent), Some(depends)) {
                    warnings.insert(format!(
                        "{} is implicitly defined by both {} and {}",
                        option_definition, depends, implicit_dependent
                    ));
                }
            } else if let (Some(implicit_dependent), true) =
                (implicit_dependent, same_priority_category)
            {
                warnings.insert(format!(
                    "{} is implicitly defined by {}; the implicitly set value overrides the \
                     previous one",
                    option_definition, implicit_dependent
                ));
            } else if let Some(depends) = option_that_depends_on_effective_value {
                warnings.insert(format!(
                    "A new value for {} overrides a previous implicit setting of that option by {}",
                    option_definition, depends
                ));
            } else if same_priority_category
                && parsed_option.get_priority().get_priority_category()
                    == PriorityCategory::COMMAND_LINE
                && option_that_expanded_to_effective_value.is_none()
                && let Some(expanded_from) = expanded_from
            {
                warnings.insert(format!(
                    "{} was expanded and now overrides the explicit option {} with {}",
                    expanded_from,
                    effective.get_command_line_form().unwrap_or("null"),
                    parsed_option.get_command_line_form().unwrap_or("null")
                ));
            } else if let (Some(expanded_to), Some(expanded_from)) =
                (option_that_expanded_to_effective_value, expanded_from)
                && !(same_priority_category
                    && parsed_option.get_priority().get_priority_category()
                        == PriorityCategory::RC_FILE)
            {
                warnings.insert(format!(
                    "{} was expanded from both {} and {}",
                    option_definition, expanded_to, expanded_from
                ));
            }
        }

        state.effective_option_instance = Some(parsed_option.clone());
        state.effective_value = new_value;
    }
    Ok(())
}
