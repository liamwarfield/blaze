//! `OptionsParserImpl`: the implementation of the options parser.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock};

use indexmap::{IndexMap, IndexSet};

use crate::converter::ConversionContext;
use crate::data::OptionsData;
use crate::definition::{OptionDefinitionRef, OptionsBase, OptionsClass, OptionsClassRef};
use crate::exception::OptionsParsingException;
use crate::parsed::{OptionInstanceOrigin, ParsedOptionDescription, ParsedOptionRef};
use crate::parser::{ArgAndFallbackData, ArgsPreProcessor, OptionDescription};
use crate::priority::{OptionPriority, PriorityCategory};
use crate::spelling::SpellChecker;
use crate::tags::OptionMetadataTag;
use crate::value::Value;
use crate::value_description::OptionValueDescription;

/// A function mapping an option to the source string recorded for it.
pub type SourceFunction<'a> = &'a dyn Fn(OptionDefinitionRef) -> Option<String>;

// Java: com/google/devtools/common/options/OptionsParserImpl.java:204 OptionsParserImpl.skippedArgs
crate::options! {
    /// Holder of the synthetic "skipped args" option of `OptionsParserImpl`.
    pub(crate) struct SkippedArgsHolder {
        #[option(
            name = "skipped args",
            allow_multiple = true,
            default_value = "null",
            documentation_category = UNDOCUMENTED,
            effect_tags = [NO_OP],
            help = "Only used internally by OptionsParserImpl",
        )]
        pub skipped_args: Vec<String>,
    }
}

// Java: com/google/devtools/common/options/OptionsParserImpl.java:214 OptionsParserImpl.skippedArgsDefinition
pub(crate) fn skipped_args_definition() -> OptionDefinitionRef {
    static DEF: LazyLock<OptionDefinitionRef> = LazyLock::new(|| {
        SkippedArgsHolder::class()
            .definitions()
            .next()
            .expect("skipped args definition")
    });
    *DEF
}

/// Result of parsing a list of args.
// Java: com/google/devtools/common/options/OptionsParserImpl.java:583 OptionsParserImpl.OptionsParserImplResult
pub(crate) struct OptionsParserImplResult {
    pub post_double_dash_residue: Vec<String>,
    pub pre_double_dash_residue: Vec<String>,
    pub ignored_args: Vec<String>,
    pub next_priority: OptionPriority,
    pub aliases: IndexMap<String, String>,
}

impl OptionsParserImplResult {
    pub fn get_residue(&self) -> Vec<String> {
        let mut r = self.pre_double_dash_residue.clone();
        r.extend(self.post_double_dash_residue.iter().cloned());
        r
    }
}

struct OptionLookupResult {
    definition: OptionDefinitionRef,
    from_fallback: bool,
}

/// `ParsedOptionDescriptionOrIgnoredArgs`.
enum Identified {
    Parsed(ParsedOptionRef),
    Ignored(String),
}

// Java: com/google/devtools/common/options/OptionsParserImpl.java:904 OptionsParserImpl.equivalentForParsing
pub fn equivalent_for_parsing(
    definition: OptionDefinitionRef,
    other_definition: OptionDefinitionRef,
) -> bool {
    if definition == other_definition {
        return true;
    }
    definition.uses_boolean_value_syntax() == other_definition.uses_boolean_value_syntax()
        && definition.is_void_field() == other_definition.is_void_field()
        && definition
            .get_option_metadata_tags()
            .contains(&OptionMetadataTag::INTERNAL)
            == other_definition
                .get_option_metadata_tags()
                .contains(&OptionMetadataTag::INTERNAL)
}

/// Java `String.trim()`.
fn java_trim(s: &str) -> &str {
    s.trim_matches(|c: char| c <= ' ')
}

// Java: com/google/devtools/common/options/OptionsParserImpl.java:57 OptionsParserImpl
#[derive(Clone)]
pub(crate) struct OptionsParserImpl {
    pub(crate) options_data: Arc<OptionsData>,
    option_values: IndexMap<OptionDefinitionRef, OptionValueDescription>,
    next_priority_per_priority_category: HashMap<PriorityCategory, OptionPriority>,
    parsed_options: Vec<ParsedOptionRef>,
    skipped_options: Vec<ParsedOptionRef>,
    flag_alias_mappings: IndexMap<String, String>,
    warnings: IndexSet<String>,
    pub(crate) args_pre_processor: Option<Arc<dyn ArgsPreProcessor>>,
    pub(crate) skipped_prefixes: Vec<String>,
    pub(crate) ignore_internal_options: bool,
    pub(crate) alias_flag: Option<String>,
    pub(crate) conversion_context: Option<Arc<ConversionContext>>,
    is_first_round_of_parsing: bool,
}

impl OptionsParserImpl {
    // Java: com/google/devtools/common/options/OptionsParserImpl.java:226 OptionsParserImpl.<init>
    pub(crate) fn new(
        options_data: Arc<OptionsData>,
        args_pre_processor: Option<Arc<dyn ArgsPreProcessor>>,
        skipped_prefixes: Vec<String>,
        ignore_internal_options: bool,
        alias_flag: Option<String>,
        conversion_context: Option<Arc<ConversionContext>>,
        aliases: IndexMap<String, String>,
        is_first_round_of_parsing: bool,
    ) -> Self {
        OptionsParserImpl {
            options_data,
            option_values: IndexMap::new(),
            next_priority_per_priority_category: PriorityCategory::VALUES
                .iter()
                .map(|p| (*p, OptionPriority::lowest_option_priority_at_category(*p)))
                .collect(),
            parsed_options: Vec::new(),
            skipped_options: Vec::new(),
            flag_alias_mappings: aliases,
            warnings: IndexSet::new(),
            args_pre_processor,
            skipped_prefixes,
            ignore_internal_options,
            alias_flag,
            conversion_context,
            is_first_round_of_parsing,
        }
    }

    pub(crate) fn flag_alias_mappings(&self) -> &IndexMap<String, String> {
        &self.flag_alias_mappings
    }

    fn ctx(&self) -> Option<Arc<ConversionContext>> {
        self.conversion_context.clone()
    }

    // Java: com/google/devtools/common/options/OptionsParserImpl.java:272 OptionsParserImpl.asCompleteListOfParsedOptions
    pub(crate) fn as_complete_list_of_parsed_options(&self) -> Vec<ParsedOptionRef> {
        let mut v = self.parsed_options.clone();
        v.sort_by(|a, b| a.get_priority().cmp(b.get_priority()));
        v
    }

    // Java: com/google/devtools/common/options/OptionsParserImpl.java:281 OptionsParserImpl.asListOfExplicitOptions
    pub(crate) fn as_list_of_explicit_options(&self) -> Vec<ParsedOptionRef> {
        let mut v: Vec<ParsedOptionRef> = self
            .parsed_options
            .iter()
            .filter(|p| p.is_explicit())
            .cloned()
            .collect();
        v.sort_by(|a, b| a.get_priority().cmp(b.get_priority()));
        v
    }

    pub(crate) fn get_skipped_options(&self) -> &[ParsedOptionRef] {
        &self.skipped_options
    }

    // Java: com/google/devtools/common/options/OptionsParserImpl.java:295 OptionsParserImpl.asCanonicalizedList
    pub(crate) fn as_canonicalized_list(&self) -> Vec<String> {
        self.as_canonicalized_list_of_parsed_options()
            .iter()
            .map(|p| p.get_deprecated_canonical_form())
            .collect()
    }

    // Java: com/google/devtools/common/options/OptionsParserImpl.java:302 OptionsParserImpl.asCanonicalizedListOfParsedOptions
    pub(crate) fn as_canonicalized_list_of_parsed_options(&self) -> Vec<ParsedOptionRef> {
        let skipped = skipped_args_definition();
        let mut v: Vec<ParsedOptionRef> = self
            .option_values
            .iter()
            .filter(|(k, _)| **k != skipped)
            .flat_map(|(_, v)| v.get_canonical_instances().unwrap_or_default())
            .collect();
        v.sort_by(|a, b| a.get_priority().cmp(b.get_priority()));
        v
    }

    // Java: com/google/devtools/common/options/OptionsParserImpl.java:313 OptionsParserImpl.asListOfEffectiveOptions
    pub(crate) fn as_list_of_effective_options(&self) -> Vec<OptionValueDescription> {
        self.options_data
            .get_all_option_definitions()
            .map(
                |(_, option_definition)| match self.option_values.get(option_definition) {
                    None => OptionValueDescription::get_default_option_value(
                        option_definition,
                        self.ctx(),
                    ),
                    Some(v) => v.clone(),
                },
            )
            .collect()
    }

    // Java: com/google/devtools/common/options/OptionsParserImpl.java:328 OptionsParserImpl.allOptionValues
    pub(crate) fn all_option_values(&self) -> Vec<OptionValueDescription> {
        self.options_data
            .get_all_option_definitions()
            .filter_map(|(_, d)| self.option_values.get(d).cloned())
            .collect()
    }

    // Java: com/google/devtools/common/options/OptionsParserImpl.java:336 OptionsParserImpl.maybeAddDeprecationWarning
    fn maybe_add_deprecation_warning(
        &mut self,
        option_definition: OptionDefinitionRef,
        priority: PriorityCategory,
    ) {
        if priority == PriorityCategory::INVOCATION_POLICY {
            return;
        }
        let warning = option_definition.get_deprecation_warning();
        if !warning.is_empty() || option_definition.is_deprecated() {
            self.add_deprecation_warning(option_definition.get_option_name(), warning);
        }
    }

    // Java: com/google/devtools/common/options/OptionsParserImpl.java:349 OptionsParserImpl.maybeAddOldNameWarning
    fn maybe_add_old_name_warning(&mut self, parsed_option: &ParsedOptionDescription) {
        if parsed_option.get_priority().get_priority_category()
            == PriorityCategory::INVOCATION_POLICY
        {
            return;
        }
        let option_definition = parsed_option.get_option_definition();
        if !option_definition.get_old_name_warning() {
            return;
        }
        if parsed_option.is_old_name_used() {
            self.add_deprecation_warning(
                option_definition.get_old_option_name(),
                &format!("Use --{} instead", option_definition.get_option_name()),
            );
        }
    }

    // Java: com/google/devtools/common/options/OptionsParserImpl.java:365 OptionsParserImpl.addDeprecationWarning
    fn add_deprecation_warning(&mut self, option_name: &str, warning: &str) {
        self.warnings.insert(format!(
            "Option '{}' is deprecated{}",
            option_name,
            if warning.is_empty() {
                String::new()
            } else {
                format!(": {}", warning)
            }
        ));
    }

    // Java: com/google/devtools/common/options/OptionsParserImpl.java:371 OptionsParserImpl.clearValue
    pub(crate) fn clear_value(
        &mut self,
        option_definition: OptionDefinitionRef,
    ) -> Option<OptionValueDescription> {
        self.option_values.shift_remove(option_definition)
    }

    // Java: com/google/devtools/common/options/OptionsParserImpl.java:376 OptionsParserImpl.getOptionValueDescription
    pub(crate) fn get_option_value_description(
        &self,
        name: &str,
    ) -> Option<OptionValueDescription> {
        let Some(option_definition) = self.options_data.get_option_definition_from_name(name)
        else {
            panic!("IllegalArgumentException: No such option '{}'", name);
        };
        self.option_values.get(option_definition).cloned()
    }

    // Java: com/google/devtools/common/options/OptionsParserImpl.java:385 OptionsParserImpl.getOptionDescription
    pub(crate) fn get_option_description(&self, name: &str) -> Option<OptionDescription> {
        let option_definition = self.options_data.get_option_definition_from_name(name)?;
        Some(OptionDescription::new(
            option_definition,
            &self.options_data,
        ))
    }

    // Java: com/google/devtools/common/options/OptionsParserImpl.java:397 OptionsParserImpl.getExpansionValueDescriptions
    pub(crate) fn get_expansion_value_descriptions(
        &mut self,
        expansion_flag_def: OptionDefinitionRef,
        origin_of_expansion_flag: &OptionInstanceOrigin,
    ) -> Result<Vec<ParsedOptionRef>, OptionsParsingException> {
        let mut builder = Vec::new();
        let mut next_option_priority =
            OptionPriority::get_child_priority(origin_of_expansion_flag.get_priority())?;
        let expansion_flag_parsed_dummy = ParsedOptionDescription::new_dummy_instance(
            expansion_flag_def,
            origin_of_expansion_flag.clone(),
            self.ctx(),
        );
        let options: Vec<String>;
        let source: String;
        let mut implicit_dependent = None;
        let mut expanded_from = None;
        let origin_source = origin_of_expansion_flag
            .get_source()
            .unwrap_or("null")
            .to_string();
        if expansion_flag_def.has_implicit_requirements() {
            options = expansion_flag_def
                .get_implicit_requirements()
                .iter()
                .map(|s| s.to_string())
                .collect();
            source = format!(
                "implicitly required by {} (source: {})",
                expansion_flag_def, origin_source
            );
            implicit_dependent = Some(expansion_flag_parsed_dummy);
        } else if expansion_flag_def.is_expansion_option() {
            options = self
                .options_data
                .get_evaluated_expansion(expansion_flag_def);
            source = format!(
                "expanded by {} (source: {})",
                expansion_flag_def, origin_source
            );
            expanded_from = Some(expansion_flag_parsed_dummy);
        } else {
            return Ok(vec![]);
        }

        let mut options_iterator = options.into_iter();
        while let Some(unparsed_flag_expression) = options_iterator.next() {
            let src = source.clone();
            let source_fn = move |_: OptionDefinitionRef| Some(src.clone());
            if let Identified::Parsed(p) = self.identify_option_and_possible_argument(
                &unparsed_flag_expression,
                &mut options_iterator,
                next_option_priority.clone(),
                &source_fn,
                implicit_dependent.clone(),
                expanded_from.clone(),
                None,
            )? {
                builder.push(p);
            }
            next_option_priority = OptionPriority::next_option_priority(&next_option_priority);
        }
        Ok(builder)
    }

    // Java: com/google/devtools/common/options/OptionsParserImpl.java:448 OptionsParserImpl.containsExplicitOption
    pub(crate) fn contains_explicit_option(&self, name: &str) -> bool {
        let Some(option_definition) = self.options_data.get_option_definition_from_name(name)
        else {
            panic!("IllegalArgumentException: No such option '{}'", name);
        };
        self.option_values.contains_key(option_definition)
    }

    // Java: com/google/devtools/common/options/OptionsParserImpl.java:457 OptionsParserImpl.getSkippedArgs
    pub(crate) fn get_skipped_args(&self) -> Vec<String> {
        match self.option_values.get(skipped_args_definition()) {
            None => vec![],
            Some(v) => v
                .get_value()
                .and_then(|v| {
                    v.as_list()
                        .map(|l| l.iter().map(|e| e.get::<String>()).collect())
                })
                .unwrap_or_default(),
        }
    }

    // Java: com/google/devtools/common/options/OptionsParserImpl.java:472 OptionsParserImpl.parse
    pub(crate) fn parse_category(
        &mut self,
        priority_cat: PriorityCategory,
        source_function: SourceFunction<'_>,
        args: Vec<ArgAndFallbackData>,
    ) -> Result<OptionsParserImplResult, OptionsParsingException> {
        let start = self.next_priority_per_priority_category[&priority_cat].clone();
        let result = self.parse(start, source_function, None, None, args)?;
        self.next_priority_per_priority_category
            .insert(priority_cat, result.next_priority.clone());
        Ok(result)
    }

    // Java: com/google/devtools/common/options/OptionsParserImpl.java:491 OptionsParserImpl.parse
    fn parse(
        &mut self,
        mut priority: OptionPriority,
        source_function: SourceFunction<'_>,
        implicit_dependent: Option<ParsedOptionRef>,
        expanded_from: Option<ParsedOptionRef>,
        args: Vec<ArgAndFallbackData>,
    ) -> Result<OptionsParserImplResult, OptionsParsingException> {
        let mut unparsed_args = Vec::new();
        let mut unparsed_post_double_dash_args = Vec::new();
        let mut ignored_args = Vec::new();

        let args = match &self.args_pre_processor {
            Some(p) => p.pre_process(args)?,
            None => args,
        };
        let mut args_iterator = args.into_iter();
        while let Some(arg_and_fallback_data) = args_iterator.next() {
            let mut arg = arg_and_fallback_data.arg;
            let fallback_data = arg_and_fallback_data.fallback_data;

            if !arg.starts_with('-') {
                unparsed_args.push(arg);
                continue;
            }

            if arg.starts_with("-//") || arg.starts_with("-@") {
                return Err(OptionsParsingException::new(format!(
                    "Invalid options syntax: {}\nNote: Negative target patterns can only appear \
                     after the end of options marker ('--'). Flags corresponding to \
                     Starlark-defined build settings always start with '--', not '-'.",
                    arg
                )));
            }

            arg = self.swap_shorthand_alias(&arg);

            if arg == "--" {
                unparsed_post_double_dash_args.extend(args_iterator.by_ref().map(|a| a.arg));
                break;
            }

            let parsed_option = if self.contains_skipped_prefix(&arg) {
                let skipped = skipped_args_definition();
                Some(ParsedOptionDescription::new_parsed_option_description(
                    skipped,
                    arg.clone(),
                    Some(arg.clone()),
                    OptionInstanceOrigin::new(
                        priority.clone(),
                        source_function(skipped),
                        implicit_dependent.clone(),
                        expanded_from.clone(),
                    ),
                    self.ctx(),
                    false,
                ))
            } else {
                let mut next_args = args_iterator.by_ref().map(|a| a.arg);
                match self.identify_option_and_possible_argument(
                    &arg,
                    &mut next_args,
                    priority.clone(),
                    source_function,
                    implicit_dependent.clone(),
                    expanded_from.clone(),
                    fallback_data.as_deref(),
                )? {
                    Identified::Parsed(p) => Some(p),
                    Identified::Ignored(i) => {
                        ignored_args.push(i);
                        None
                    }
                }
            };
            if let Some(p) = parsed_option {
                self.handle_new_parsed_option(&p, fallback_data)?;
            }
            priority = OptionPriority::next_option_priority(&priority);
        }

        // Go through the final values and make sure they are valid values for their option.
        // getValue() throws (ConstructionException) if a default value is invalid.
        for value_description in self.as_list_of_effective_options() {
            if let Err(e) = value_description.try_get_value() {
                e.throw();
            }
        }

        Ok(OptionsParserImplResult {
            pre_double_dash_residue: unparsed_args,
            post_double_dash_residue: unparsed_post_double_dash_args,
            ignored_args,
            next_priority: priority,
            aliases: self.flag_alias_mappings.clone(),
        })
    }

    // Java: com/google/devtools/common/options/OptionsParserImpl.java:613 OptionsParserImpl.parseArgsAsExpansionOfOption
    pub(crate) fn parse_args_as_expansion_of_option(
        &mut self,
        option_to_expand: &ParsedOptionRef,
        source_function: SourceFunction<'_>,
        args: Vec<ArgAndFallbackData>,
    ) -> Result<OptionsParserImplResult, OptionsParsingException> {
        let priority = OptionPriority::get_child_priority(option_to_expand.get_priority())?;
        self.parse(
            priority,
            source_function,
            None,
            Some(option_to_expand.clone()),
            args,
        )
    }

    // Java: com/google/devtools/common/options/OptionsParserImpl.java:631 OptionsParserImpl.setOptionValueAtSpecificPriorityWithoutExpansion
    pub(crate) fn set_option_value_at_specific_priority_without_expansion(
        &mut self,
        origin: OptionInstanceOrigin,
        option: OptionDefinitionRef,
        unconverted_value: &str,
    ) -> Result<(), OptionsParsingException> {
        let priority_category = origin.get_priority().get_priority_category();
        if priority_category == PriorityCategory::DEFAULT {
            panic!(
                "IllegalArgumentException: Attempt to assign value '{}' to {} at priority {} failed. \
                 Cannot set options at default priority - by definition, that means the option is \
                 unset.",
                unconverted_value, option, priority_category
            );
        }
        let parsed = ParsedOptionDescription::new_parsed_option_description(
            option,
            format!("--{}={}", option.get_option_name(), unconverted_value),
            Some(unconverted_value.to_string()),
            origin,
            self.ctx(),
            false,
        );
        self.set_option_value(&parsed)?;
        Ok(())
    }

    // Java: com/google/devtools/common/options/OptionsParserImpl.java:664 OptionsParserImpl.handleNewParsedOption
    fn handle_new_parsed_option(
        &mut self,
        parsed_option: &ParsedOptionRef,
        fallback_data: Option<Arc<OptionsData>>,
    ) -> Result<(), OptionsParsingException> {
        let option_definition = parsed_option.get_option_definition();
        let expansion_bundle = self.set_option_value(parsed_option)?;
        let unconverted_value = parsed_option.get_unconverted_value();

        if let Some(expansion_bundle) = expansion_bundle {
            let source = expansion_bundle.source_of_expansion_args.clone();
            let source_fn = move |_: OptionDefinitionRef| Some(source.clone());
            let args = expansion_bundle
                .expansion_args
                .iter()
                .map(|a| ArgAndFallbackData::new(a.clone(), fallback_data.clone()))
                .collect();
            let result = self.parse(
                OptionPriority::get_child_priority(parsed_option.get_priority())?,
                &source_fn,
                if option_definition.has_implicit_requirements() {
                    Some(parsed_option.clone())
                } else {
                    None
                },
                if option_definition.is_expansion_option() {
                    Some(parsed_option.clone())
                } else {
                    None
                },
                args,
            )?;
            if !result.get_residue().is_empty() {
                panic!(
                    "AssertionError: Unparsed options remain after processing {}: {}",
                    unconverted_value.unwrap_or("null"),
                    result.get_residue().join(" ")
                );
            }
        }
        Ok(())
    }

    // Java: com/google/devtools/common/options/OptionsParserImpl.java:692 OptionsParserImpl.setOptionValue
    fn set_option_value(
        &mut self,
        parsed_option: &ParsedOptionRef,
    ) -> Result<Option<crate::value_description::ExpansionBundle>, OptionsParsingException> {
        let option_definition = parsed_option.get_option_definition();
        self.maybe_add_deprecation_warning(
            option_definition,
            parsed_option.get_priority().get_priority_category(),
        );
        self.maybe_add_old_name_warning(parsed_option);
        if !self.option_values.contains_key(option_definition) {
            let d = OptionValueDescription::create_option_value_description(
                option_definition,
                &self.options_data,
                self.ctx(),
            );
            self.option_values.insert(option_definition, d);
        }
        let entry = self.option_values.get_mut(option_definition).unwrap();
        let expansion_bundle = entry.add_option_instance(parsed_option, &mut self.warnings)?;

        if parsed_option.get_implicit_dependent().is_none() {
            if parsed_option.get_option_definition() == skipped_args_definition() {
                self.skipped_options.push(parsed_option.clone());
            } else {
                self.parsed_options.push(parsed_option.clone());
            }

            if let Some(alias_flag) = &self.alias_flag
                && parsed_option
                    .get_command_line_form()
                    .unwrap_or("")
                    .starts_with(&format!("--{}", alias_flag))
            {
                let value = parsed_option.get_unconverted_value().unwrap_or("");
                let mut alias = value.splitn(2, '=');
                let k = alias.next().unwrap_or("").to_string();
                let v = alias
                    .next()
                    .unwrap_or_else(|| panic!("IndexOutOfBoundsException: alias '{}'", value))
                    .to_string();
                self.flag_alias_mappings.insert(k, v);
            }
        }
        Ok(expansion_bundle)
    }

    // Java: com/google/devtools/common/options/OptionsParserImpl.java:749 OptionsParserImpl.identifyOptionAndPossibleArgument
    fn identify_option_and_possible_argument(
        &mut self,
        arg: &str,
        next_args: &mut dyn Iterator<Item = String>,
        priority: OptionPriority,
        source_function: SourceFunction<'_>,
        implicit_dependent: Option<ParsedOptionRef>,
        expanded_from: Option<ParsedOptionRef>,
        fallback_data: Option<&OptionsData>,
    ) -> Result<Identified, OptionsParsingException> {
        let mut command_line_form = arg.to_string();
        let mut unconverted_value: Option<String> = None;
        let lookup_result: Option<OptionLookupResult>;
        let mut boolean_value = true;
        let mut parsed_option_name = String::new();
        let chars: Vec<char> = arg.chars().collect();

        if chars.len() == 2 {
            lookup_result = self.get_with_fallback_abbrev(chars[1], fallback_data);
            boolean_value = true;
        } else if chars.len() == 3 && chars[2] == '-' {
            lookup_result = self.get_with_fallback_abbrev(chars[1], fallback_data);
            boolean_value = false;
        } else if let Some(after) = arg.strip_prefix("--") {
            let (mut name, value) = match after.find('=') {
                None => (after.to_string(), None),
                Some(i) => (after[..i].to_string(), Some(after[i + 1..].to_string())),
            };
            if java_trim(&name).is_empty() {
                return Err(OptionsParsingException::with_argument(
                    format!("Invalid options syntax: {}", arg),
                    arg,
                ));
            }
            unconverted_value = value;
            let mut lr = self.get_with_fallback_name(&name, fallback_data);

            if lr.is_none()
                && let Some(stripped) = name.strip_prefix("no")
            {
                name = stripped.to_string();
                lr = self.get_with_fallback_name(&name, fallback_data);
                boolean_value = false;
                if let Some(l) = &lr {
                    if !l.definition.uses_boolean_value_syntax() {
                        return Err(OptionsParsingException::with_argument(
                            format!("Illegal use of 'no' prefix on non-boolean option: {}", arg),
                            arg,
                        ));
                    }
                    if unconverted_value.is_some() {
                        return Err(OptionsParsingException::with_argument(
                            format!("Unexpected value after boolean option: {}", arg),
                            arg,
                        ));
                    }
                    unconverted_value = Some("0".to_string());
                }
            }
            lookup_result = lr;
            parsed_option_name = name;
        } else {
            return Err(OptionsParsingException::with_argument(
                format!("Invalid options syntax: {}", arg),
                arg,
            ));
        }

        let lookup_result = match lookup_result {
            Some(l) if !self.should_ignore_option(l.definition) => l,
            _ => {
                if self.is_first_round_of_parsing {
                    return Ok(Identified::Ignored(arg.to_string()));
                }
                let suggestion = if arg.starts_with("--") {
                    SpellChecker::did_you_mean(arg, self.get_all_valid_args())
                } else {
                    String::new()
                };
                return Err(OptionsParsingException::with_argument(
                    format!("Unrecognized option: {}{}", arg, suggestion),
                    arg,
                ));
            }
        };

        if unconverted_value.is_none() {
            if lookup_result.definition.uses_boolean_value_syntax() {
                unconverted_value = Some(if boolean_value { "1" } else { "0" }.to_string());
            } else if lookup_result.definition.is_void_field() {
                // Void type options have no args.
            } else if let Some(next) = next_args.next() {
                command_line_form.push(' ');
                command_line_form.push_str(&next);
                unconverted_value = Some(next);
            } else {
                return Err(OptionsParsingException::new(format!(
                    "Expected value after {}",
                    arg
                )));
            }
        }

        if lookup_result.from_fallback {
            return Ok(Identified::Ignored(command_line_form));
        }

        let old_name_used = !parsed_option_name.is_empty()
            && lookup_result.definition.get_old_option_name() == parsed_option_name;
        Ok(Identified::Parsed(
            ParsedOptionDescription::new_parsed_option_description(
                lookup_result.definition,
                command_line_form,
                unconverted_value,
                OptionInstanceOrigin::new(
                    priority,
                    source_function(lookup_result.definition),
                    implicit_dependent,
                    expanded_from,
                ),
                self.ctx(),
                old_name_used,
            ),
        ))
    }

    // Java: com/google/devtools/common/options/OptionsParserImpl.java:865 OptionsParserImpl.getAllValidArgs
    fn get_all_valid_args(&self) -> Vec<String> {
        let mut out = Vec::new();
        for (name, definition) in self.options_data.get_all_option_definitions() {
            if self.should_ignore_option(definition) {
                continue;
            }
            out.push(format!("--{}", name));
            if definition.uses_boolean_value_syntax() {
                out.push(format!("--no{}", name));
            }
        }
        out
    }

    // Java: com/google/devtools/common/options/OptionsParserImpl.java:943 OptionsParserImpl.getWithFallback
    fn get_with_fallback_name(
        &self,
        name: &str,
        fallback_data: Option<&OptionsData>,
    ) -> Option<OptionLookupResult> {
        if let Some(d) = self.options_data.get_option_definition_from_name(name) {
            return Some(OptionLookupResult {
                definition: d,
                from_fallback: false,
            });
        }
        if let Some(d) = fallback_data.and_then(|f| f.get_option_definition_from_name(name)) {
            return Some(OptionLookupResult {
                definition: d,
                from_fallback: true,
            });
        }
        None
    }

    fn get_with_fallback_abbrev(
        &self,
        abbrev: char,
        fallback_data: Option<&OptionsData>,
    ) -> Option<OptionLookupResult> {
        if let Some(d) = self.options_data.get_field_for_abbrev(abbrev) {
            return Some(OptionLookupResult {
                definition: d,
                from_fallback: false,
            });
        }
        if let Some(d) = fallback_data.and_then(|f| f.get_field_for_abbrev(abbrev)) {
            return Some(OptionLookupResult {
                definition: d,
                from_fallback: true,
            });
        }
        None
    }

    // Java: com/google/devtools/common/options/OptionsParserImpl.java:955 OptionsParserImpl.shouldIgnoreOption
    fn should_ignore_option(&self, option_definition: OptionDefinitionRef) -> bool {
        self.ignore_internal_options
            && option_definition
                .get_option_metadata_tags()
                .contains(&OptionMetadataTag::INTERNAL)
    }

    // Java: com/google/devtools/common/options/OptionsParserImpl.java:962 OptionsParserImpl.getParsedOptions
    pub(crate) fn get_parsed_options_dyn(
        &self,
        options_class: OptionsClassRef,
    ) -> Option<Box<dyn OptionsBase>> {
        if !self.options_data.contains_class(options_class) {
            return None;
        }
        let mut options_instance = options_class.new_raw_instance();
        for option_definition in options_class.definitions() {
            let value: Option<Value> = match self.option_values.get(option_definition) {
                Some(v) if !v.contains_errors() => v.get_value(),
                _ => option_definition.get_default_value(self.conversion_context.as_deref()),
            };
            // A type mismatch (e.g. a boolean option that got a string value) is ignored, leaving
            // the field unchanged. Fixes https://github.com/bazelbuild/bazel/issues/7847
            let _ = option_definition.set_value(&mut *options_instance, value);
        }
        Some(options_instance)
    }

    pub(crate) fn get_parsed_options<O: OptionsClass>(&self) -> Option<O> {
        self.get_parsed_options_dyn(O::class())
            .map(|b| b.as_any().downcast_ref::<O>().unwrap().clone())
    }

    // Java: com/google/devtools/common/options/OptionsParserImpl.java:995 OptionsParserImpl.getWarnings
    pub(crate) fn get_warnings(&self) -> Vec<String> {
        self.warnings.iter().cloned().collect()
    }

    // Java: com/google/devtools/common/options/OptionsParserImpl.java:1012 OptionsParserImpl.swapShorthandAlias
    fn swap_shorthand_alias(&mut self, arg: &str) -> String {
        if self.alias_flag.is_none() || !arg.starts_with("--") {
            return arg.to_string();
        }
        let equal_sign = arg.find('=');
        let actual_arg = match equal_sign {
            Some(i) => &arg[2..i],
            None => &arg[2..],
        };
        if let Some(alias) = self.flag_alias_mappings.get(actual_arg) {
            return match equal_sign {
                Some(i) => format!("--{}{}", alias, &arg[i..]),
                None => format!("--{}", alias),
            };
        }
        let Some(name_without_no) = actual_arg.strip_prefix("no") else {
            return arg.to_string();
        };
        let def = self
            .options_data
            .get_option_definition_from_name(name_without_no);
        let (true, Some(def)) = (self.flag_alias_mappings.contains_key(name_without_no), def)
        else {
            return arg.to_string();
        };
        self.maybe_add_deprecation_warning(def, PriorityCategory::COMMAND_LINE);
        if def.get_deprecation_warning().is_empty() {
            self.warnings.insert(format!(
                "Flag --no{} is deprecated. Use --{}=false instead.",
                name_without_no, name_without_no
            ));
        }
        arg.to_string()
    }

    // Java: com/google/devtools/common/options/OptionsParserImpl.java:1053 OptionsParserImpl.containsSkippedPrefix
    fn contains_skipped_prefix(&self, arg: &str) -> bool {
        self.skipped_prefixes.iter().any(|p| arg.starts_with(p))
    }
}
