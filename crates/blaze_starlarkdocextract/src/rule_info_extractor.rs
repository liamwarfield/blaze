//! Port of `starlarkdocextract/RuleInfoExtractor.java`.

use blaze_packages::rule_class::rule_class::{RuleClass, RuleClassType};
use blaze_packages::target::IS_EXECUTABLE_ATTRIBUTE_NAME;
use blaze_packages::types::Type;
use blaze_protos::stardoc_output::{AttributeInfo, AttributeType, OriginKey, RuleInfo};

use crate::attribute_info_extractor::add_documentable_attributes;
use crate::extractor_context::ExtractorContext;
use crate::provider_name_group_extractor::build_provider_name_group;

// Java: com/google/devtools/build/lib/starlarkdocextract/RuleInfoExtractor.java:35 RuleInfoExtractor.IMPLICIT_RULE_ATTRIBUTES
pub fn implicit_rule_attributes() -> Vec<(&'static str, AttributeInfo)> {
    vec![(
        "name",
        AttributeInfo {
            name: "name".to_owned(),
            r#type: AttributeType::Name as i32,
            mandatory: true,
            doc_string: "A unique name for this target.".to_owned(),
            ..AttributeInfo::default()
        },
    )]
}

/// Extracts API documentation for a rule in the form of a `RuleInfo` proto.
///
/// - `qualified_name`: a human-readable name for the rule to use in documentation output; for a
///   Starlark rule defined in a .bzl file, this would typically be the name under which users of
///   the module would use the rule
/// - `rule_class`: a rule class; if it is a repository rule class, it must be an exported one
// Java: com/google/devtools/build/lib/starlarkdocextract/RuleInfoExtractor.java:56 RuleInfoExtractor.buildRuleInfo
pub fn build_rule_info(
    context: &ExtractorContext,
    qualified_name: &str,
    rule_class: &RuleClass,
) -> RuleInfo {
    let mut rule_info = RuleInfo {
        // Record the name under which this symbol is made accessible, which may differ from the
        // symbol's exported name
        rule_name: qualified_name.to_owned(),
        ..RuleInfo::default()
    };
    // ... but record the origin rule key for cross references.
    let mut origin_key = OriginKey {
        name: rule_class.get_name().to_owned(),
        ..OriginKey::default()
    };
    if rule_class.is_starlark() {
        if let Some(extension_label) = rule_class.get_starlark_extension_label() {
            // Most common case: exported Starlark-defined rule class
            origin_key.file = context.label_renderer.render(extension_label);
        } else {
            // Unexported Starlark-defined rule class; this only possible for a repository rule.
            // Fall back to the rule definition environment label.
            origin_key.file = context.label_renderer.render(
                rule_class
                    .get_rule_definition_environment_label()
                    .expect("a Starlark rule class has a rule definition environment label"),
            );
        }
    } else {
        // Non-Starlark-defined rule class
        origin_key.file = "<native>".to_owned();
    }
    rule_info.origin_key = Some(origin_key);

    if let Some(doc) = rule_class.get_starlark_documentation() {
        rule_info.doc_string = doc.to_owned();
    }

    if rule_class.get_rule_class_type() == RuleClassType::Test {
        rule_info.test = true;
    }
    if rule_class
        .get_attribute_provider()
        .has_attr(IS_EXECUTABLE_ATTRIBUTE_NAME, Type::Boolean)
    {
        rule_info.executable = true;
    }

    add_documentable_attributes(
        context,
        &implicit_rule_attributes(),
        rule_class.get_attribute_provider().get_attributes(),
        &mut rule_info.attribute,
    );
    let advertised_providers = rule_class
        .get_advertised_providers()
        .get_starlark_providers();
    if !advertised_providers.is_empty() {
        rule_info.advertised_providers =
            Some(build_provider_name_group(context, advertised_providers));
    }
    rule_info
}
