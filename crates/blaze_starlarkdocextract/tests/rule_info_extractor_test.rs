//! Port of `src/test/java/com/google/devtools/build/lib/starlarkdocextract/RuleInfoExtractorTest.java`.

use blaze_packages::attribute::attr;
use blaze_packages::native_rules::rule_class_provider::RuleClassProvider;
use blaze_packages::rule_class::{RuleClassBuilder, RuleClassType};
use blaze_packages::types::Type;
use blaze_protos::stardoc_output::{AttributeInfo, AttributeType, OriginKey, RuleInfo};
use blaze_starlarkdocextract::attribute_info_extractor::{
    UNREPRESENTABLE_VALUE, get_attribute_type,
};
use blaze_starlarkdocextract::extractor_context::ExtractorContext;
use blaze_starlarkdocextract::label_renderer::LabelRenderer;
use blaze_starlarkdocextract::rule_info_extractor::{build_rule_info, implicit_rule_attributes};

const DUMMY_CONFIGURED_TARGET_FACTORY: &str = "DummyConfiguredTargetFactory";

/// `ExtractorContext.builder().labelRenderer(LabelRenderer.DEFAULT)
/// .extractNativelyDefinedAttrs(true).build()`.
fn context_extracting_natively_defined_attrs() -> ExtractorContext {
    ExtractorContext {
        extract_natively_defined_attrs: true,
        ..ExtractorContext::new(LabelRenderer::default_renderer())
    }
}

// Java: src/test/java/com/google/devtools/build/lib/starlarkdocextract/RuleInfoExtractorTest.java:46 RuleInfoExtractorTest.basicFunctionality
#[test]
fn basic_functionality() {
    let rule_class = RuleClassBuilder::new("test_rule", RuleClassType::Normal, false, &[])
        .factory(DUMMY_CONFIGURED_TARGET_FACTORY)
        .add(&attr("tags", Type::StringList))
        .build();
    let extractor_context = context_extracting_natively_defined_attrs();
    let rule_info = build_rule_info(&extractor_context, "namespace.test_rule", &rule_class);
    let mut attribute: Vec<AttributeInfo> = implicit_rule_attributes()
        .into_iter()
        .map(|(_, a)| a)
        .collect();
    attribute.push(AttributeInfo {
        name: "tags".to_owned(),
        r#type: AttributeType::StringList as i32,
        default_value: "[]".to_owned(),
        mandatory: false,
        natively_defined: true,
        ..Default::default()
    });
    assert_eq!(
        rule_info,
        RuleInfo {
            rule_name: "namespace.test_rule".to_owned(),
            origin_key: Some(OriginKey {
                name: "test_rule".to_owned(),
                file: "<native>".to_owned(),
            }),
            attribute,
            ..Default::default()
        }
    );
}

/// `BuildTypeTestHelper.getAllBuildTypes(/* publicOnly= */ true)`: every static `Type` field of
/// `Type`, `Types` and `BuildType` (all of them are public at the pin).
fn all_build_types() -> Vec<Type> {
    vec![
        Type::Integer,
        Type::String,
        Type::StringNoIntern,
        Type::Boolean,
        Type::StringList,
        Type::StringSet,
        Type::IntegerList,
        Type::StringDict,
        Type::StringListDict,
        Type::Label,
        Type::LabelDictUnary,
        Type::LabelKeyedStringDict,
        Type::LabelList,
        Type::LabelListDict,
        Type::NodepLabel,
        Type::NodepLabelList,
        Type::DormantLabel,
        Type::DormantLabelList,
        Type::GenqueryScopeType,
        Type::GenqueryScopeTypeList,
        Type::License,
        Type::Output,
        Type::OutputList,
        Type::Tristate,
    ]
}

// Java: src/test/java/com/google/devtools/build/lib/starlarkdocextract/RuleInfoExtractorTest.java:77 RuleInfoExtractorTest.allBuiltinAttributeTypesSupported
#[test]
fn all_builtin_attribute_types_supported() {
    for type_ in all_build_types() {
        assert_ne!(
            get_attribute_type(type_, "test_attr"),
            AttributeType::Unknown,
            "attr type '{type_:?}'"
        );
    }
}

// Java: src/test/java/com/google/devtools/build/lib/starlarkdocextract/RuleInfoExtractorTest.java:88 RuleInfoExtractorTest.allNativeRulesAreSupported
#[test]
fn all_native_rules_are_supported() {
    let extractor_context = context_extracting_natively_defined_attrs();
    let rule_class_provider = RuleClassProvider::create_bazel();
    let implicit_name = implicit_rule_attributes()
        .into_iter()
        .find(|(n, _)| *n == "name")
        .map(|(_, a)| a)
        .unwrap();
    for rule_class in rule_class_provider.get_rule_class_map().values() {
        let rule_info = build_rule_info(&extractor_context, rule_class.get_name(), rule_class);
        let name = rule_class.get_name();
        assert_eq!(rule_info.rule_name, name);
        let origin_key = rule_info.origin_key.as_ref().unwrap();
        assert_eq!(origin_key.name, name);
        assert_eq!(origin_key.file, "<native>", "rule '{name}'");
        assert_eq!(rule_info.attribute[0], implicit_name, "rule '{name}'");
        let mut names: Vec<&str> = rule_info
            .attribute
            .iter()
            .map(|a| a.name.as_str())
            .collect();
        let count = names.len();
        names.sort();
        names.dedup();
        assert_eq!(
            names.len(),
            count,
            "rule '{name}': duplicate attribute names"
        );
        assert!(
            !rule_info
                .attribute
                .iter()
                .any(|a| a.default_value == UNREPRESENTABLE_VALUE),
            "rule '{name}'"
        );
    }
}
