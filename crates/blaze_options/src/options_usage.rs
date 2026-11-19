//! Port of `OptionsUsage`: a renderer for usage messages for any combination of options classes.

use crate::converters::HelpVerbosity;
use crate::data::OptionsData;
use crate::definition::{OptionDefinitionRef, OptionsClassRef};
use crate::tags::{OptionEffectTag, OptionMetadataTag};
use crate::value::FieldKind;

/// A renderer for usage messages for any combination of options classes.
// Java: com/google/devtools/common/options/OptionsUsage.java:29 OptionsUsage
pub struct OptionsUsage;

impl OptionsUsage {
    /// Given an options class, render the usage string into the usage, which is passed in as an
    /// argument. This will not include information about expansions for options using expansion
    /// functions (it would be unsafe to report this as we cannot know what options from other
    /// `OptionsBase` subclasses they depend on until a complete parser is constructed).
    // Java: com/google/devtools/common/options/OptionsUsage.java:40 OptionsUsage.getUsage
    pub fn get_usage_for_class(options_class: OptionsClassRef, usage: &mut String) {
        let data = OptionsData::get(&[options_class], false).unwrap_or_else(|e| e.throw());
        // getAllOptionDefinitionsForClass is sorted by option name already; Java sorts again
        // with BY_OPTION_NAME.
        let mut option_definitions: Vec<OptionDefinitionRef> =
            options_class.definitions().collect();
        option_definitions.sort_by(|a, b| a.get_option_name().cmp(b.get_option_name()));
        for option_definition in option_definitions {
            Self::get_usage(option_definition, usage, HelpVerbosity::LONG, &data, false);
        }
    }

    /// Appends the usage message for a single option-field message to 'usage'.
    // Java: com/google/devtools/common/options/OptionsUsage.java:51 OptionsUsage.getUsage
    pub fn get_usage(
        option_definition: OptionDefinitionRef,
        usage: &mut String,
        help_verbosity: HelpVerbosity,
        options_data: &OptionsData,
        include_tags: bool,
    ) {
        let flag_name = Self::get_flag_name(option_definition);
        let type_description = Self::get_type_description(option_definition);
        usage.push_str("  --");
        usage.push_str(&flag_name);
        if help_verbosity == HelpVerbosity::SHORT {
            usage.push('\n');
            return;
        }

        // Add the option's type and default information. Stop there for "medium" verbosity.
        if option_definition.get_abbreviation() != '\0' {
            usage.push_str(" [-");
            usage.push(option_definition.get_abbreviation());
            usage.push(']');
        }
        if !type_description.is_empty() {
            usage.push_str(" (");
            usage.push_str(&type_description);
            usage.push_str("; ");
            if option_definition.allows_multiple() {
                usage.push_str("may be used multiple times");
            } else {
                // Don't call the annotation directly (we must allow overrides to certain defaults)
                let default_value_string = option_definition.get_unparsed_default_value();
                if option_definition.is_special_null_default() {
                    usage.push_str("default: see description");
                } else {
                    usage.push_str("default: \"");
                    usage.push_str(default_value_string);
                    usage.push('"');
                }
            }
            usage.push(')');
        }
        usage.push('\n');
        if help_verbosity == HelpVerbosity::MEDIUM {
            return;
        }

        // For verbosity "long," add the full description and expansion, along with the tag
        // information if requested.
        if !option_definition.get_help_text().is_empty() {
            usage.push_str(&Self::paragraph_fill(
                option_definition.get_help_text(),
                /*indent=*/ 4,
                /*width=*/ 80,
            ));
            usage.push('\n');
        }
        let expansion = options_data.get_evaluated_expansion(option_definition);
        if !expansion.is_empty() {
            let mut expands_msg = String::from("Expands to: ");
            for exp in &expansion {
                expands_msg.push_str(exp);
                expands_msg.push(' ');
            }
            usage.push_str(&Self::paragraph_fill(
                &expands_msg,
                /*indent=*/ 6,
                /*width=*/ 80,
            ));
            usage.push('\n');
        }
        if option_definition.has_implicit_requirements() {
            let mut required_msg = String::from("Using this option will also add: ");
            for req in option_definition.get_implicit_requirements() {
                required_msg.push_str(req);
                required_msg.push(' ');
            }
            usage.push_str(&Self::paragraph_fill(&required_msg, 6, 80)); // (indent, width)
            usage.push('\n');
        }
        if !include_tags {
            return;
        }

        // If we are expected to include the tags, add them for high verbosity.
        let tag_list: Vec<String> = option_definition
            .get_option_effect_tags()
            .iter()
            .filter(|t| Self::should_effect_tag_be_listed(**t))
            .map(|t| t.name().to_ascii_lowercase())
            .chain(
                option_definition
                    .get_option_metadata_tags()
                    .iter()
                    .filter(|t| Self::should_metadata_tag_be_listed(**t))
                    .map(|t| t.name().to_ascii_lowercase()),
            )
            .collect();
        let tag_list = tag_list.join(", ");
        if !tag_list.is_empty() {
            usage.push_str(&Self::paragraph_fill(&format!("Tags: {}", tag_list), 6, 80)); // (indent, width)
            usage.push('\n');
        }
    }

    /// Paragraph-fill the specified input text, indenting lines to 'indent' and wrapping lines
    /// at 'width'. Returns the formatted result.
    // Java: com/google/devtools/common/options/OptionsUsage.java:132 OptionsUsage.paragraphFill
    pub fn paragraph_fill(input: &str, indent: usize, width: usize) -> String {
        let indent_string = " ".repeat(indent);
        let mut out = String::new();
        let mut sep = "";
        for paragraph in input.split('\n') {
            // TODO(ccalvarin) break iterators expect hyphenated words to be line-breakable,
            // which looks funny for --flag
            out.push_str(sep);
            out.push_str(&indent_string);
            let mut cursor = indent;
            for word in crate::line_break_iterator::line_segments(paragraph) {
                // (may include trailing space); Java's length() counts UTF-16 units.
                let word_length = word.encode_utf16().count();
                if word_length + cursor > width {
                    out.push('\n');
                    out.push_str(&indent_string);
                    cursor = indent;
                }
                out.push_str(word);
                cursor += word_length;
            }
            sep = "\n";
        }
        out
    }

    /// Placeholder tag "UNKNOWN" is ignored.
    // Java: com/google/devtools/common/options/OptionsUsage.java:160 OptionsUsage.shouldEffectTagBeListed
    pub fn should_effect_tag_be_listed(effect_tag: OptionEffectTag) -> bool {
        effect_tag != OptionEffectTag::UNKNOWN
    }

    /// Tags that only apply to undocumented options are excluded.
    // Java: com/google/devtools/common/options/OptionsUsage.java:165 OptionsUsage.shouldMetadataTagBeListed
    pub fn should_metadata_tag_be_listed(metadata_tag: OptionMetadataTag) -> bool {
        metadata_tag != OptionMetadataTag::HIDDEN && metadata_tag != OptionMetadataTag::INTERNAL
    }

    /// Returns the available completion for the given option field. The completions are the
    /// exact command line option (with the prepending '--') that one should pass. It is suitable
    /// for completion script to use. If the option expect an argument, the kind of argument is
    /// given after the equals. If the kind is a enum, the various enum values are given inside an
    /// accolade in a comma separated list. For other special kind, the type is given as a name
    /// (e.g., `label`, `float`, `path`...).
    // Java: com/google/devtools/common/options/OptionsUsage.java:196 OptionsUsage.getCompletion
    pub fn get_completion(option_definition: OptionDefinitionRef, builder: &mut String) {
        // Return the list of possible completions for this option
        let flag_name = option_definition.get_option_name();
        let field = option_definition.field_info();
        builder.push_str("--");
        builder.push_str(flag_name);
        let simple_name = simple_name(field.java_type_name);
        if field.kind == FieldKind::Boolean && field.is_primitive {
            builder.push('\n');
            builder.push_str("--no");
            builder.push_str(flag_name);
            builder.push('\n');
        } else if field.kind == FieldKind::TriState {
            builder.push_str("={auto,yes,no}\n");
            builder.push_str("--no");
            builder.push_str(flag_name);
            builder.push('\n');
        } else if let Some(constants) = (field.enum_constants)() {
            builder.push_str("={");
            // Locale.ENGLISH lower-casing.
            builder.push_str(&constants.join(",").to_lowercase());
            builder.push_str("}\n");
        } else if simple_name == "Label" {
            // String comparison so we don't introduce a dependency to
            // com.google.devtools.build.lib.
            builder.push_str("=label\n");
        } else if simple_name == "PathFragment" {
            builder.push_str("=path\n");
        } else if field.kind == FieldKind::Void {
            builder.push('\n');
        } else {
            // TODO(bazel-team): add more types. Maybe even move the completion type
            // to the @Option annotation?
            builder.push_str("=\n");
        }
    }

    // Java: com/google/devtools/common/options/OptionsUsage.java:229 OptionsUsage.getTypeDescription
    pub fn get_type_description(options_definition: OptionDefinitionRef) -> String {
        options_definition.get_converter().get_type_description()
    }

    // Java: com/google/devtools/common/options/OptionsUsage.java:233 OptionsUsage.getFlagName
    pub fn get_flag_name(option_definition: OptionDefinitionRef) -> String {
        let name = option_definition.get_option_name();
        if option_definition.uses_boolean_value_syntax() {
            format!("[no]{}", name)
        } else {
            name.to_string()
        }
    }
}

/// `Class.getSimpleName()` of a class given by its binary name.
fn simple_name(java_type_name: &str) -> &str {
    let after_package = java_type_name.rsplit('.').next().unwrap_or(java_type_name);
    after_package.rsplit('$').next().unwrap_or(after_package)
}
