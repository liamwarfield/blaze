//! Ports of `src/test/java/com/google/devtools/build/lib/packages/AttributeTest.java` and
//! `AttributeValueSourceTest.java`.

use std::sync::Arc;

use blaze_cmdline::Label;

use crate::attribute::AllowedValueSet;
use crate::attribute::AttributeValueSource;
use crate::attribute::ComputedDefault;
use crate::attribute::LateBoundDefault;
use crate::attribute::TransitionFactory;
use crate::attribute::attr;
use crate::rule_class::ProviderKey;
use crate::rule_class::RuleClassNamePredicate;
use crate::rule_class::StarlarkProviderIdentifier;
use crate::types::AttrValue;
use crate::types::FileType;
use crate::types::FileTypeSet;
use crate::types::Type;

fn label(s: &str) -> Label {
    Label::parse_canonical(s).unwrap()
}

// ---- AttributeTest

#[test]
fn test_basics() {
    let attr_ = attr("foo", Type::Integer)
        .mandatory()
        .value(Some(AttrValue::Int(3)))
        .build();
    assert_eq!(attr_.get_name(), "foo");
    assert_eq!(attr_.get_default_value(None), Some(AttrValue::Int(3)));
    assert_eq!(attr_.get_type(), Type::Integer);
    assert!(attr_.is_mandatory());
    assert!(attr_.is_documented());
    assert!(!attr_.starlark_defined());
    let attr_ = attr("$foo", Type::Integer).build();
    assert!(!attr_.is_documented());
}

#[test]
#[should_panic(expected = "attribute 'foo' must be a list")]
fn test_non_empty_requires_list_type() {
    attr("foo", Type::Integer)
        .non_empty()
        .value(Some(AttrValue::Int(3)))
        .build();
}

#[test]
fn test_non_empty() {
    let attr_ = attr("foo", Type::LabelList)
        .non_empty()
        .legacy_allow_any_file_type()
        .build();
    assert_eq!(attr_.get_name(), "foo");
    assert_eq!(attr_.get_type(), Type::LabelList);
    assert!(attr_.is_non_empty());
}

#[test]
#[should_panic(expected = "attribute 'foo' must be a label-valued type")]
fn test_single_artifact_requires_label_type() {
    attr("foo", Type::Integer)
        .single_artifact()
        .value(Some(AttrValue::Int(3)))
        .build();
}

#[test]
#[should_panic(expected = "IllegalStateException")]
fn test_setting_configuration_twice_disallowed() {
    let mut builder = attr("x", Type::String);
    builder
        .mandatory()
        .cfg(TransitionFactory::Exec { exec_group: None })
        .undocumented("")
        .value(Some(AttrValue::String("y".to_owned())));
    builder.cfg(TransitionFactory::Exec { exec_group: None });
}

/// Tests the "convenience factories" (string, label, etc) for default values.
#[test]
fn test_convenience_factories_default_values() {
    assert_eq!(
        attr("x", Type::Integer).build().get_default_value(None),
        Some(AttrValue::Int(0))
    );
    assert_eq!(
        attr("x", Type::Integer)
            .value(Some(AttrValue::Int(42)))
            .build()
            .get_default_value(None),
        Some(AttrValue::Int(42))
    );
    assert_eq!(
        attr("x", Type::String).build().get_default_value(None),
        Some(AttrValue::String(String::new()))
    );
    assert_eq!(
        attr("x", Type::String)
            .value(Some(AttrValue::String("foo".to_owned())))
            .build()
            .get_default_value(None),
        Some(AttrValue::String("foo".to_owned()))
    );
    let l = label("//foo:bar");
    assert_eq!(
        attr("x", Type::Label)
            .legacy_allow_any_file_type()
            .build()
            .get_default_value(None),
        None
    );
    assert_eq!(
        attr("x", Type::Label)
            .legacy_allow_any_file_type()
            .value(Some(AttrValue::Label(l.clone())))
            .build()
            .get_default_value(None),
        Some(AttrValue::Label(l))
    );
    let slist = vec!["foo".to_owned(), "bar".to_owned()];
    assert_eq!(
        attr("x", Type::StringList).build().get_default_value(None),
        Some(AttrValue::StringList(Vec::new()))
    );
    assert_eq!(
        attr("x", Type::StringList)
            .value(Some(AttrValue::StringList(slist.clone())))
            .build()
            .get_default_value(None),
        Some(AttrValue::StringList(slist))
    );
    let llist = vec![label("//foo:bar"), label("//foo:wiz")];
    assert_eq!(
        attr("x", Type::LabelList)
            .legacy_allow_any_file_type()
            .build()
            .get_default_value(None),
        Some(AttrValue::LabelList(Vec::new()))
    );
    assert_eq!(
        attr("x", Type::LabelList)
            .legacy_allow_any_file_type()
            .value(Some(AttrValue::LabelList(llist.clone())))
            .build()
            .get_default_value(None),
        Some(AttrValue::LabelList(llist))
    );
}

/// Tests the "convenience factories" (string, label, etc) for types.
#[test]
fn test_convenience_factories_types() {
    assert_eq!(attr("x", Type::Integer).build().get_type(), Type::Integer);
    assert_eq!(
        attr("x", Type::Integer)
            .value(Some(AttrValue::Int(42)))
            .build()
            .get_type(),
        Type::Integer
    );
    assert_eq!(attr("x", Type::String).build().get_type(), Type::String);
    assert_eq!(
        attr("x", Type::Label)
            .legacy_allow_any_file_type()
            .value(Some(AttrValue::Label(label("//foo:bar"))))
            .build()
            .get_type(),
        Type::Label
    );
    assert_eq!(
        attr("x", Type::StringList).build().get_type(),
        Type::StringList
    );
    assert_eq!(
        attr("x", Type::LabelList)
            .legacy_allow_any_file_type()
            .build()
            .get_type(),
        Type::LabelList
    );
}

#[test]
fn test_clone_builder() {
    // PORT: the Java test also adds TestAspects.SIMPLE_ASPECT / ERROR_ASPECT (native aspects
    // with definitions) and checks getAspects(); aspects are not instantiated during loading.
    let txt_files = FileTypeSet::of(vec![FileType::of(&["txt"])]);
    let rule_classes = RuleClassNamePredicate::only(&["mock_rule"]);

    let parent_attr = attr("x", Type::LabelList)
        .allowed_file_types(txt_files.clone())
        .mandatory()
        .build();

    {
        let child_attr1 = parent_attr.clone_builder().build();
        assert_eq!(child_attr1.get_name(), "x");
        assert_eq!(
            child_attr1.get_allowed_file_types_predicate(),
            Some(&txt_files)
        );
        assert_eq!(
            child_attr1.get_allowed_rule_classes_for_labels(),
            &RuleClassNamePredicate::unspecified()
        );
        assert!(child_attr1.is_mandatory());
        assert!(!child_attr1.is_non_empty());
    }

    {
        let child_attr2 = parent_attr
            .clone_builder()
            .non_empty()
            .allowed_rule_classes_predicate(rule_classes.clone())
            .build();
        assert_eq!(child_attr2.get_name(), "x");
        assert_eq!(
            child_attr2.get_allowed_file_types_predicate(),
            Some(&txt_files)
        );
        assert_eq!(
            child_attr2.get_allowed_rule_classes_for_labels(),
            &rule_classes
        );
        assert!(child_attr2.is_mandatory());
        assert!(child_attr2.is_non_empty());
    }

    // Check if the parent attribute is unchanged
    assert!(!parent_attr.is_non_empty());
    assert_eq!(
        parent_attr.get_allowed_rule_classes_for_labels(),
        &RuleClassNamePredicate::unspecified()
    );
}

/// Tests that configurability settings are properly received.
#[test]
fn test_configurability() {
    assert!(
        attr("foo_configurable", Type::LabelList)
            .legacy_allow_any_file_type()
            .build()
            .is_configurable()
    );
    assert!(
        !attr("foo_nonconfigurable", Type::LabelList)
            .legacy_allow_any_file_type()
            .nonconfigurable("test")
            .build()
            .is_configurable()
    );
}

#[test]
fn test_split_transition_provider() {
    let split = TransitionFactory::Native {
        name: "TestSplitTransitionProvider".to_owned(),
        is_tool: false,
        is_split: true,
    };
    let attr_ = attr("foo", Type::Label)
        .cfg(split)
        .allowed_file_types(FileTypeSet::of(Vec::new()))
        .build();
    assert!(attr_.get_transition_factory().is_split());
}

#[test]
fn test_exec_transition() {
    let attr_ = attr("foo", Type::Label)
        .cfg(TransitionFactory::Exec { exec_group: None })
        .allowed_file_types(FileTypeSet::of(Vec::new()))
        .build();
    assert!(attr_.get_transition_factory().is_tool());
    assert!(!attr_.get_transition_factory().is_split());
}

#[test]
#[should_panic(expected = "may not contain the same rule classes")]
fn allowed_rule_classes_and_allowed_rule_classes_with_warnings_cannot_overlap() {
    attr("x", Type::LabelList)
        .allowed_rule_classes(&["foo", "bar", "baz"])
        .allowed_rule_classes_with_warning(&["bar"])
        .allowed_file_types(FileTypeSet::of(Vec::new()))
        .build();
}

#[test]
fn factory_equality() {
    // PORT: EqualsTester; the TestAspects group is omitted (native test aspects).
    let ab = || AttrValue::Label(Label::parse_canonical_unchecked("//a:b"));
    let groups: Vec<Vec<crate::attribute::ImmutableAttributeFactory>> = vec![
        vec![
            attr("foo", Type::Label).build_partial(),
            attr("foo", Type::Label).build_partial(),
        ],
        vec![
            attr("foo", Type::Label).value(Some(ab())).build_partial(),
            attr("foo", Type::Label).value(Some(ab())).build_partial(),
        ],
        vec![
            attr("foo", Type::NodepLabel)
                .value(Some(ab()))
                .build_partial(),
            attr("foo", Type::NodepLabel)
                .value(Some(ab()))
                .build_partial(),
        ],
        vec![
            attr("foo", Type::Label)
                .value(Some(AttrValue::Label(Label::parse_canonical_unchecked(
                    "//c:d",
                ))))
                .build_partial(),
            attr("foo", Type::Label)
                .value(Some(AttrValue::Label(Label::parse_canonical_unchecked(
                    "//c:d",
                ))))
                .build_partial(),
        ],
        vec![
            attr("foo", Type::Label)
                .value(Some(ab()))
                .set_doc(Some("My doc".to_owned()))
                .build_partial(),
            attr("foo", Type::Label)
                .value(Some(ab()))
                .set_doc(Some("My doc".to_owned()))
                .build_partial(),
        ],
        // PredicateWithMessage does not define any particular equality semantics
        vec![
            attr("foo", Type::Label)
                .value(Some(ab()))
                .allowed_values(Arc::new(AllowedValueSet::new(vec![AttrValue::Label(
                    label("//a:b"),
                )])))
                .build_partial(),
        ],
        vec![
            attr("foo", Type::Label)
                .value(Some(ab()))
                .allowed_rule_classes(&["java_binary"])
                .build_partial(),
            attr("foo", Type::Label)
                .value(Some(ab()))
                .allowed_rule_classes(&["java_binary"])
                .build_partial(),
        ],
        vec![
            attr("foo", Type::Label)
                .value(Some(ab()))
                .allowed_file_types(FileTypeSet::AnyFile)
                .build_partial(),
            attr("foo", Type::Label)
                .value(Some(ab()))
                .allowed_file_types(FileTypeSet::AnyFile)
                .build_partial(),
        ],
        vec![
            attr("foo", Type::Label)
                .value(Some(ab()))
                .mandatory_providers(vec![StarlarkProviderIdentifier::for_key(
                    ProviderKey::Builtin {
                        name: "DefaultInfo".to_owned(),
                        provider_class: "DefaultInfo".to_owned(),
                    },
                )])
                .build_partial(),
            attr("foo", Type::Label)
                .value(Some(ab()))
                .mandatory_providers(vec![StarlarkProviderIdentifier::for_key(
                    ProviderKey::Builtin {
                        name: "DefaultInfo".to_owned(),
                        provider_class: "DefaultInfo".to_owned(),
                    },
                )])
                .build_partial(),
        ],
    ];
    for (i, g) in groups.iter().enumerate() {
        for a in g {
            for b in g {
                assert_eq!(a, b, "group {i}");
            }
        }
        for (j, h) in groups.iter().enumerate() {
            if i != j {
                assert_ne!(g[0], h[0], "groups {i} and {j}");
            }
        }
    }
}

// ---- AttributeValueSourceTest

const ALL_SOURCES: [AttributeValueSource; 5] = [
    AttributeValueSource::NativeComputedDefault,
    AttributeValueSource::ComputedDefault,
    AttributeValueSource::LateBound,
    AttributeValueSource::Materializer,
    AttributeValueSource::Direct,
];

#[test]
fn test_validate_starlark_name() {
    // Success means "no exception is being thrown".
    AttributeValueSource::ComputedDefault
        .validate_starlark_name("_name")
        .unwrap();
    AttributeValueSource::LateBound
        .validate_starlark_name("_name")
        .unwrap();
    AttributeValueSource::Materializer
        .validate_starlark_name("_name")
        .unwrap();
    AttributeValueSource::Direct
        .validate_starlark_name("_name")
        .unwrap();
    AttributeValueSource::Direct
        .validate_starlark_name("name")
        .unwrap();
}

fn assert_name_is_not_valid(source: AttributeValueSource, name: &str, expected: &str) {
    let ex = source.validate_starlark_name(name).unwrap_err();
    assert_eq!(ex.get_message(), expected);
}

#[test]
fn test_validate_starlark_name_empty_name() {
    for source in ALL_SOURCES {
        assert_name_is_not_valid(source, "", "Attribute name must not be empty.");
    }
}

#[test]
fn test_validate_starlark_name_missing_prefix() {
    let msg = "When an attribute value is a function, the attribute must be private (i.e. start \
               with '_'). Found 'my_name'";
    assert_name_is_not_valid(AttributeValueSource::ComputedDefault, "my_name", msg);
    assert_name_is_not_valid(AttributeValueSource::LateBound, "my_name", msg);
    assert_name_is_not_valid(AttributeValueSource::Materializer, "my_name", msg);
}

#[test]
fn test_convert_to_native_name() {
    let check = |source: AttributeValueSource, starlark: &str, native: &str| {
        assert_eq!(source.convert_to_native_name(starlark).unwrap(), native);
    };
    check(AttributeValueSource::ComputedDefault, "_name", "$name");
    check(AttributeValueSource::LateBound, "_name", ":name");
    check(AttributeValueSource::Materializer, "_name", ":name");
    check(AttributeValueSource::Direct, "_name", "$name");
    check(AttributeValueSource::Direct, "name", "name");
}

#[test]
fn test_convert_to_native_name_invalid_name() {
    for source in [
        AttributeValueSource::ComputedDefault,
        AttributeValueSource::LateBound,
        AttributeValueSource::Materializer,
    ] {
        let ex = source.convert_to_native_name("name").unwrap_err();
        assert_eq!(
            ex.get_message(),
            "When an attribute value is a function, the attribute must be private (i.e. start \
             with '_'). Found 'name'"
        );
    }
}

#[test]
fn test_builder_get_value_source() {
    let mut computed_default_builder = attr("x", Type::String);
    computed_default_builder.value_computed_default(Arc::new(ComputedDefault::new_native(
        Vec::new(),
        Arc::new(|_: &dyn crate::attribute::AttributeMap| None),
    )));
    let mut late_bound_builder = attr(":x", Type::String);
    late_bound_builder.value_late_bound(Arc::new(LateBoundDefault::always_null()));
    let mut direct_builder = attr("x", Type::String);
    direct_builder.value(Some(AttrValue::String("value".to_owned())));
    assert_eq!(
        computed_default_builder.get_value_source(),
        AttributeValueSource::ComputedDefault
    );
    assert_eq!(
        late_bound_builder.get_value_source(),
        AttributeValueSource::LateBound
    );
    assert_eq!(
        direct_builder.get_value_source(),
        AttributeValueSource::Direct
    );
}
