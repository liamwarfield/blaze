//! `ParsedOptionDescription` and `OptionInstanceOrigin`.

use std::fmt;
use std::sync::Arc;

use crate::converter::ConversionContext;
use crate::definition::OptionDefinitionRef;
use crate::exception::OptionsParsingException;
use crate::priority::OptionPriority;
use crate::tags::{OptionDocumentationCategory, OptionMetadataTag};
use crate::value::Value;

/// Shared handle on a parsed option. Java compares these by identity; use [`Arc::ptr_eq`].
pub type ParsedOptionRef = Arc<ParsedOptionDescription>;

// Java: com/google/devtools/common/options/OptionInstanceOrigin.java:22 OptionInstanceOrigin
/// Where an option instance came from: priority, source and expansion/implicit parents.
#[derive(Clone, Debug)]
pub struct OptionInstanceOrigin {
    priority: OptionPriority,
    source: Option<String>,
    implicit_dependent: Option<ParsedOptionRef>,
    expanded_from: Option<ParsedOptionRef>,
}

impl OptionInstanceOrigin {
    pub fn new(
        priority: OptionPriority,
        source: Option<String>,
        implicit_dependent: Option<ParsedOptionRef>,
        expanded_from: Option<ParsedOptionRef>,
    ) -> Self {
        OptionInstanceOrigin {
            priority,
            source,
            implicit_dependent,
            expanded_from,
        }
    }

    // Java: com/google/devtools/common/options/OptionInstanceOrigin.java:39 OptionInstanceOrigin.getPriority
    pub fn get_priority(&self) -> &OptionPriority {
        &self.priority
    }

    // Java: com/google/devtools/common/options/OptionInstanceOrigin.java:43 OptionInstanceOrigin.getSource
    pub fn get_source(&self) -> Option<&str> {
        self.source.as_deref()
    }

    // Java: com/google/devtools/common/options/OptionInstanceOrigin.java:48 OptionInstanceOrigin.getImplicitDependent
    pub fn get_implicit_dependent(&self) -> Option<&ParsedOptionRef> {
        self.implicit_dependent.as_ref()
    }

    // Java: com/google/devtools/common/options/OptionInstanceOrigin.java:53 OptionInstanceOrigin.getExpandedFrom
    pub fn get_expanded_from(&self) -> Option<&ParsedOptionRef> {
        self.expanded_from.as_ref()
    }
}

// Java: com/google/devtools/common/options/ParsedOptionDescription.java:28 ParsedOptionDescription
/// The representation of a parsed option instance.
pub struct ParsedOptionDescription {
    option_definition: OptionDefinitionRef,
    command_line_form: Option<String>,
    unconverted_value: Option<String>,
    origin: OptionInstanceOrigin,
    conversion_context: Option<Arc<ConversionContext>>,
    old_name_used: bool,
}

impl ParsedOptionDescription {
    /// PORT: Java's two overloads; the one without `oldNameUsed` passes `false`.
    // Java: com/google/devtools/common/options/ParsedOptionDescription.java:52 ParsedOptionDescription.newParsedOptionDescription
    // Java: com/google/devtools/common/options/ParsedOptionDescription.java:69 ParsedOptionDescription.newParsedOptionDescription
    pub fn new_parsed_option_description(
        option_definition: OptionDefinitionRef,
        command_line_form: impl Into<String>,
        unconverted_value: Option<String>,
        origin: OptionInstanceOrigin,
        conversion_context: Option<Arc<ConversionContext>>,
        old_name_used: bool,
    ) -> ParsedOptionRef {
        Arc::new(ParsedOptionDescription {
            option_definition,
            command_line_form: Some(command_line_form.into()),
            unconverted_value,
            origin,
            conversion_context,
            old_name_used,
        })
    }

    // Java: com/google/devtools/common/options/ParsedOptionDescription.java:91 ParsedOptionDescription.newDummyInstance
    pub fn new_dummy_instance(
        option_definition: OptionDefinitionRef,
        origin: OptionInstanceOrigin,
        conversion_context: Option<Arc<ConversionContext>>,
    ) -> ParsedOptionRef {
        Arc::new(ParsedOptionDescription {
            option_definition,
            command_line_form: None,
            unconverted_value: None,
            origin,
            conversion_context,
            old_name_used: false,
        })
    }

    // Java: com/google/devtools/common/options/ParsedOptionDescription.java:99 ParsedOptionDescription.getOptionDefinition
    pub fn get_option_definition(&self) -> OptionDefinitionRef {
        self.option_definition
    }

    // Java: com/google/devtools/common/options/ParsedOptionDescription.java:103 ParsedOptionDescription.getCommandLineForm
    pub fn get_command_line_form(&self) -> Option<&str> {
        self.command_line_form.as_deref()
    }

    // Java: com/google/devtools/common/options/ParsedOptionDescription.java:108 ParsedOptionDescription.getCanonicalForm
    pub fn get_canonical_form(&self) -> String {
        self.get_canonical_form_with_value_escaper(|s| s.to_string())
    }

    // Java: com/google/devtools/common/options/ParsedOptionDescription.java:112 ParsedOptionDescription.getCanonicalFormWithValueEscaper
    pub fn get_canonical_form_with_value_escaper(
        &self,
        escaping_function: impl Fn(&str) -> String,
    ) -> String {
        if self.option_definition.is_boolean_field() {
            let value = self
                .get_converted_value()
                .unwrap_or_else(|e| panic!("Unexpected parsing exception: {}", e));
            let b = value.and_then(|v| v.downcast_ref::<bool>().copied()) == Some(true);
            format!(
                "{}{}",
                if b { "--" } else { "--no" },
                self.option_definition.get_option_name()
            )
        } else {
            let mut option_string = format!("--{}", self.option_definition.get_option_name());
            if let Some(v) = &self.unconverted_value {
                option_string.push('=');
                option_string.push_str(&escaping_function(v));
            }
            option_string
        }
    }

    // Java: com/google/devtools/common/options/ParsedOptionDescription.java:133 ParsedOptionDescription.getDeprecatedCanonicalForm
    pub fn get_deprecated_canonical_form(&self) -> String {
        let value = if self.option_definition.is_boolean_field() {
            let v = self
                .get_converted_value()
                .unwrap_or_else(|e| panic!("Unexpected parsing exception: {}", e));
            let b = v.and_then(|v| v.downcast_ref::<bool>().copied()) == Some(true);
            if b { "1" } else { "0" }.to_string()
        } else {
            self.unconverted_value
                .clone()
                .unwrap_or_else(|| "null".to_string())
        };
        format!("--{}={}", self.option_definition.get_option_name(), value)
    }

    // Java: com/google/devtools/common/options/ParsedOptionDescription.java:156 ParsedOptionDescription.isDocumented
    pub fn is_documented(&self) -> bool {
        // Java: com/google/devtools/common/options/ParsedOptionDescription.java:148 ParsedOptionDescription.documentationCategory
        self.option_definition.get_documentation_category()
            != OptionDocumentationCategory::UNDOCUMENTED
            && !self.is_hidden()
    }

    // Java: com/google/devtools/common/options/ParsedOptionDescription.java:160 ParsedOptionDescription.isHidden
    pub fn is_hidden(&self) -> bool {
        // Java: com/google/devtools/common/options/ParsedOptionDescription.java:152 ParsedOptionDescription.metadataTags
        let tags = self.option_definition.get_option_metadata_tags();
        tags.contains(&OptionMetadataTag::HIDDEN) || tags.contains(&OptionMetadataTag::INTERNAL)
    }

    // Java: com/google/devtools/common/options/ParsedOptionDescription.java:165 ParsedOptionDescription.getUnconvertedValue
    pub fn get_unconverted_value(&self) -> Option<&str> {
        self.unconverted_value.as_deref()
    }

    // Java: com/google/devtools/common/options/ParsedOptionDescription.java:170 ParsedOptionDescription.getOrigin
    pub fn get_origin(&self) -> &OptionInstanceOrigin {
        &self.origin
    }

    // Java: com/google/devtools/common/options/ParsedOptionDescription.java:174 ParsedOptionDescription.getPriority
    pub fn get_priority(&self) -> &OptionPriority {
        self.origin.get_priority()
    }

    // Java: com/google/devtools/common/options/ParsedOptionDescription.java:178 ParsedOptionDescription.isOldNameUsed
    pub fn is_old_name_used(&self) -> bool {
        self.old_name_used
    }

    // Java: com/google/devtools/common/options/ParsedOptionDescription.java:182 ParsedOptionDescription.getSource
    pub fn get_source(&self) -> Option<&str> {
        self.origin.get_source()
    }

    // Java: com/google/devtools/common/options/ParsedOptionDescription.java:187 ParsedOptionDescription.getImplicitDependent
    pub fn get_implicit_dependent(&self) -> Option<&ParsedOptionRef> {
        self.origin.get_implicit_dependent()
    }

    // Java: com/google/devtools/common/options/ParsedOptionDescription.java:192 ParsedOptionDescription.getExpandedFrom
    pub fn get_expanded_from(&self) -> Option<&ParsedOptionRef> {
        self.origin.get_expanded_from()
    }

    // Java: com/google/devtools/common/options/ParsedOptionDescription.java:197 ParsedOptionDescription.isExplicit
    pub fn is_explicit(&self) -> bool {
        self.origin.get_expanded_from().is_none()
            && self.origin.get_implicit_dependent().is_none()
            && !self
                .origin
                .get_source()
                .is_some_and(|s| s.ends_with("PROJECT.scl"))
    }

    // Java: com/google/devtools/common/options/ParsedOptionDescription.java:204 ParsedOptionDescription.getConvertedValue
    pub fn get_converted_value(&self) -> Result<Option<Value>, OptionsParsingException> {
        let converter = self.option_definition.get_converter();
        converter
            .convert(
                self.unconverted_value.as_deref(),
                self.conversion_context.as_deref(),
            )
            .map_err(|e| {
                OptionsParsingException::with_cause(
                    format!(
                        "While parsing option {}: {}",
                        self.command_line_form.as_deref().unwrap_or("null"),
                        e.get_message()
                    ),
                    e.get_message(),
                )
            })
    }
}

impl fmt::Display for ParsedOptionDescription {
    // Java: com/google/devtools/common/options/ParsedOptionDescription.java:216 ParsedOptionDescription.toString
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.command_line_form {
            None => write!(f, "{}", self.option_definition),
            Some(form) => {
                write!(f, "option '{}'", form)?;
                if let Some(source) = self.origin.get_source() {
                    write!(f, " (source {})", source)?;
                }
                Ok(())
            }
        }
    }
}

impl fmt::Debug for ParsedOptionDescription {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}
