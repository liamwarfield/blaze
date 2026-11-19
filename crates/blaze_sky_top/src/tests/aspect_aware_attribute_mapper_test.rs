//! Port of `src/test/java/com/google/devtools/build/lib/analysis/AspectAwareAttributeMapperTest.java`
//! (a `BuildViewTestCase`).
//!
//! Supersedes the ignored stubs in `blaze_analysis/src/aspect_aware_attribute_mapper.rs`, which
//! were written before this harness existed.

use std::sync::Arc;

use blaze_analysis::aspect_aware_attribute_mapper::AspectAwareAttributeMapper;
use blaze_analysis::config::configured_attribute_mapper::ConfiguredAttributeMapper;
use blaze_analysis::configured_target::ConfiguredTarget;
use blaze_packages::attribute::{Attribute, AttributeMap, attr};
use blaze_packages::target::Rule;
use blaze_packages::types::{AttrValue, FileTypeSet, Type};
use indexmap::IndexMap;

use crate::testing::BuildViewTestCase;
use crate::testing::build_view_test_case::parse_label;

/// The fixture of the Java test class (`rule`, `aspectAttributes`, `mapper`).
struct Fixture {
    _t: BuildViewTestCase,
    rule: Arc<Rule>,
    mapper: AspectAwareAttributeMapper<Arc<ConfiguredAttributeMapper>>,
}

// Java: AspectAwareAttributeMapperTest.createMapper
fn create_mapper() -> Fixture {
    let mut t = BuildViewTestCase::new();
    let ctad = t
        .scratch_configured_target_and_data(
            "foo",
            "myrule",
            &[r#"load("@rules_cc//cc:cc_binary.bzl", "cc_binary")

# Needed to avoid select() being eliminated as trivial.
config_setting(
    name = "config",
    values = {"define": "pi=3"},
)

cc_binary(
    name = "myrule",
    srcs = [":a.cc"],
    linkstatic = select({
        ":config": 1,
        "//conditions:default": 1,
    }),
)
"#],
        )
        .expect("configured target //foo:myrule");

    let ConfiguredTarget::Rule(ct) = ctad.get_configured_target() else {
        panic!("ClassCastException: not a RuleConfiguredTarget");
    };
    let rule = ctad
        .get_target()
        .as_rule()
        .expect("ClassCastException: not a Rule")
        .clone();
    let mut aspect_attr_builder = attr("fromaspect", Type::Label);
    aspect_attr_builder.allowed_file_types(FileTypeSet::AnyFile);
    let aspect_attr = aspect_attr_builder.build();
    let mut aspect_attributes: IndexMap<String, Attribute> = IndexMap::new();
    aspect_attributes.insert(aspect_attr.get_name().to_string(), aspect_attr);
    let mapper = AspectAwareAttributeMapper::new(
        Arc::new(ConfiguredAttributeMapper::of_hash(
            rule.clone(),
            ct.get_config_conditions().clone(),
            ctad.get_configured_target()
                .get_configuration_checksum()
                .expect("configuration checksum"),
            /* always_succeed= */ false,
        )),
        aspect_attributes,
    );
    Fixture {
        _t: t,
        rule,
        mapper,
    }
}

/// The message of the exception `f` throws (a panic `"<ExceptionClass>: <message>"` here).
fn assert_throws(exception_class: &str, f: impl FnOnce()) -> String {
    let payload = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f))
        .expect_err("expected an exception");
    let message = payload
        .downcast_ref::<String>()
        .cloned()
        .or_else(|| payload.downcast_ref::<&str>().map(|s| s.to_string()))
        .unwrap_or_default();
    let prefix = format!("{exception_class}: ");
    message
        .strip_prefix(&prefix)
        .unwrap_or_else(|| panic!("expected {exception_class}, got: {message}"))
        .to_string()
}

// Java: AspectAwareAttributeMapperTest.getName
#[test]
#[ignore = "needs the mock C++ toolchains (cc_binary from the mock rules_cc)"]
fn get_name() {
    let f = create_mapper();
    assert_eq!(f.mapper.get_label().get_name(), f.rule.get_name());
}

// Java: AspectAwareAttributeMapperTest.getLabel
#[test]
#[ignore = "needs the mock C++ toolchains (cc_binary from the mock rules_cc)"]
fn get_label() {
    let f = create_mapper();
    assert_eq!(f.mapper.get_label(), f.rule.get_label());
}

// Java: AspectAwareAttributeMapperTest.getRuleAttributeValue
#[test]
#[ignore = "needs the mock C++ toolchains (cc_binary from the mock rules_cc)"]
fn get_rule_attribute_value() {
    let f = create_mapper();
    match f.mapper.get("srcs", Type::LabelList) {
        Some(AttrValue::LabelList(labels)) => {
            assert_eq!(labels, vec![parse_label("//foo:a.cc")]);
        }
        other => panic!("expected a label list, got {other:?}"),
    }
}

// Java: AspectAwareAttributeMapperTest.getAspectAttributeValue
#[test]
#[ignore = "needs the mock C++ toolchains (cc_binary from the mock rules_cc)"]
fn get_aspect_attribute_value() {
    let f = create_mapper();
    assert_throws("UnsupportedOperationException", || {
        f.mapper.get("fromaspect", Type::Label);
    });
}

// Java: AspectAwareAttributeMapperTest.getAspectValueWrongType
#[test]
#[ignore = "needs the mock C++ toolchains (cc_binary from the mock rules_cc)"]
fn get_aspect_value_wrong_type() {
    let f = create_mapper();
    let e = assert_throws("IllegalArgumentException", || {
        f.mapper.get("fromaspect", Type::LabelList);
    });
    assert_eq!(
        e,
        "attribute fromaspect has type label, not expected type list(label)"
    );
}

// Java: AspectAwareAttributeMapperTest.getMissingAttributeValue
#[test]
#[ignore = "needs the mock C++ toolchains (cc_binary from the mock rules_cc)"]
fn get_missing_attribute_value() {
    let f = create_mapper();
    let e = assert_throws("IllegalArgumentException", || {
        f.mapper.get("noexist", Type::Label);
    });
    // matches("no attribute 'noexist' in either cc_binary //foo:myrule \\([^)]+\\) or its aspects")
    let middle = e
        .strip_prefix("no attribute 'noexist' in either cc_binary //foo:myrule (")
        .and_then(|rest| rest.strip_suffix(") or its aspects"))
        .unwrap_or_else(|| panic!("message does not match: {e}"));
    assert!(
        !middle.is_empty() && !middle.contains(')'),
        "message does not match: {e}"
    );
}

// Java: AspectAwareAttributeMapperTest.isConfigurable
#[test]
#[ignore = "needs the mock C++ toolchains (cc_binary from the mock rules_cc)"]
fn is_configurable() {
    let f = create_mapper();
    assert!(f.mapper.is_configurable("linkstatic"));
    assert!(!f.mapper.is_configurable("fromaspect"));
}

// Java: AspectAwareAttributeMapperTest.getAttributeNames
#[test]
#[ignore = "needs the mock C++ toolchains (cc_binary from the mock rules_cc)"]
fn get_attribute_names() {
    let f = create_mapper();
    let names = f.mapper.get_attribute_names();
    for name in ["srcs", "linkstatic", "fromaspect"] {
        assert!(names.iter().any(|n| n == name), "{name} not in {names:?}");
    }
}

// Java: AspectAwareAttributeMapperTest.getAttributeType
#[test]
#[ignore = "needs the mock C++ toolchains (cc_binary from the mock rules_cc)"]
fn get_attribute_type() {
    let f = create_mapper();
    assert_eq!(f.mapper.get_attribute_type("srcs"), Some(Type::LabelList));
    assert_eq!(f.mapper.get_attribute_type("fromaspect"), Some(Type::Label));
}

// Java: AspectAwareAttributeMapperTest.getAttributeDefinition
#[test]
#[ignore = "needs the mock C++ toolchains (cc_binary from the mock rules_cc)"]
fn get_attribute_definition() {
    let f = create_mapper();
    assert_eq!(
        f.mapper
            .get_attribute_definition("srcs")
            .unwrap()
            .get_name(),
        "srcs"
    );
    assert_eq!(
        f.mapper
            .get_attribute_definition("fromaspect")
            .unwrap()
            .get_name(),
        "fromaspect"
    );
}

// Java: AspectAwareAttributeMapperTest.has
#[test]
#[ignore = "needs the mock C++ toolchains (cc_binary from the mock rules_cc)"]
fn has() {
    let f = create_mapper();
    assert!(f.mapper.has("srcs"));
    assert!(f.mapper.has("fromaspect"));
}
