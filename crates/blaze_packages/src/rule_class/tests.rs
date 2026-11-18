//! Ports of `src/test/java/com/google/devtools/build/lib/packages/RuleClassBuilderTest.java`,
//! `RuleClassTest.java` (the tests that need no package loading), `RequiredProvidersTest.java`
//! and `AdvertisedProviderSetTest.java`.

use std::sync::Arc;

use blaze_cmdline::Label;
use indexmap::IndexMap;
use indexmap::IndexSet;

use crate::attribute::attr;
use crate::rule_class::AdvertisedProviderSet;
use crate::rule_class::BuildSetting;
use crate::rule_class::DeclaredExecGroup;
use crate::rule_class::ProviderKey;
use crate::rule_class::RequiredProviders;
use crate::rule_class::RuleClass;
use crate::rule_class::RuleClassBuilder;
use crate::rule_class::RuleClassNamePredicate;
use crate::rule_class::RuleClassType;
use crate::rule_class::StarlarkProviderIdentifier;
use crate::rule_class::ToolchainTypeRequirement;
use crate::rule_class::rule_class::APPLICABLE_METADATA_ATTR;
use crate::rule_class::rule_class::STARLARK_BUILD_SETTING_DEFAULT_ATTR_NAME;
use crate::types::AttrValue;
use crate::types::Type;

const DUMMY_CONFIGURED_TARGET_FACTORY: &str = "DummyConfiguredTargetFactory";

fn label(s: &str) -> Label {
    Label::parse_canonical_unchecked(s)
}

fn builder(name: &str, type_: RuleClassType, parents: &[Arc<RuleClass>]) -> RuleClassBuilder {
    RuleClassBuilder::new(name, type_, false, parents)
}

// ---- RuleClassBuilderTest

#[test]
fn test_rule_class_builder_basics() {
    let rule_class_a = builder("ruleA", RuleClassType::Normal, &[])
        .factory(DUMMY_CONFIGURED_TARGET_FACTORY)
        .add(attr("srcs", Type::LabelList).legacy_allow_any_file_type())
        .add(&attr("tags", Type::StringList))
        .add(attr("X", Type::Integer).mandatory())
        .build();

    assert_eq!(rule_class_a.get_name(), "ruleA");
    let p = rule_class_a.get_attribute_provider();
    assert_eq!(p.get_attribute_count(), 4);
    assert!(rule_class_a.outputs_to_bindir());

    assert_eq!(p.get_attribute_index("srcs"), Some(1));
    assert_eq!(p.get_attribute_by_name("srcs"), p.get_attribute(1));
    assert_eq!(p.get_attribute_index("tags"), Some(2));
    assert_eq!(p.get_attribute_by_name("tags"), p.get_attribute(2));
    assert_eq!(p.get_attribute_index("X"), Some(3));
    assert_eq!(p.get_attribute_by_name("X"), p.get_attribute(3));
}

#[test]
fn test_rule_class_builder_test_is_binary() {
    let rule_class_a = builder("rule_test", RuleClassType::Test, &[])
        .factory(DUMMY_CONFIGURED_TARGET_FACTORY)
        .add(&attr("tags", Type::StringList))
        .add(attr("size", Type::String).value(Some(AttrValue::String("medium".to_owned()))))
        .add(&attr("timeout", Type::String))
        .add(attr("flaky", Type::Boolean).value(Some(AttrValue::Bool(false))))
        .add(attr("shard_count", Type::Integer).value(Some(AttrValue::Int(-1))))
        .add(&attr("local", Type::Boolean))
        .build();
    assert!(rule_class_a.outputs_to_bindir());
}

#[test]
fn test_rule_class_builder_genrule_is_not_binary() {
    let rule_class_a = builder("ruleA", RuleClassType::Normal, &[])
        .factory(DUMMY_CONFIGURED_TARGET_FACTORY)
        .set_output_to_genfiles()
        .add(&attr("tags", Type::StringList))
        .build();
    assert!(!rule_class_a.outputs_to_bindir());
}

#[test]
#[should_panic(expected = "IllegalArgumentException")]
fn test_rule_class_test_name_validity() {
    builder("ruleA", RuleClassType::Test, &[]).build();
}

#[test]
#[should_panic(expected = "IllegalArgumentException")]
fn test_rule_class_normal_name_validity() {
    builder("ruleA_test", RuleClassType::Normal, &[]).build();
}

#[test]
#[should_panic(expected = "IllegalStateException")]
fn test_duplicate_attribute() {
    let mut b = builder("ruleA", RuleClassType::Normal, &[]);
    b.add(&attr("a", Type::String));
    b.add(&attr("a", Type::String));
}

#[test]
fn test_properties_of_abstract_rule_class() {
    let r = std::panic::catch_unwind(|| {
        builder("$ruleA", RuleClassType::Abstract, &[]).set_output_to_genfiles();
    });
    assert!(r.is_err());
    let r = std::panic::catch_unwind(|| {
        builder("$ruleB", RuleClassType::Abstract, &[])
            .set_implicit_outputs_function(crate::attribute::ImplicitOutputsFunction::None);
    });
    assert!(r.is_err());
}

fn rule_with_a(name: &str, value: &str) -> Arc<RuleClass> {
    Arc::new(
        builder(name, RuleClassType::Normal, &[])
            .factory(DUMMY_CONFIGURED_TARGET_FACTORY)
            .add(attr("a", Type::String).value(Some(AttrValue::String(value.to_owned()))))
            .add(&attr("tags", Type::StringList))
            .build(),
    )
}

#[test]
#[should_panic(expected = "Attribute a is inherited multiple times in ruleC ruleclass")]
fn test_duplicate_inherited_attribute() {
    let a = rule_with_a("ruleA", "A");
    let b = rule_with_a("ruleB", "B");
    builder("ruleC", RuleClassType::Normal, &[a, b]).build();
}

#[test]
fn test_remove_attribute() {
    let a = Arc::new(
        builder("rule", RuleClassType::Normal, &[])
            .factory(DUMMY_CONFIGURED_TARGET_FACTORY)
            .add(&attr("a", Type::String))
            .add(&attr("b", Type::String))
            .add(&attr("tags", Type::StringList))
            .build(),
    );
    let mut b = builder("c", RuleClassType::Normal, &[a]);
    b.factory(DUMMY_CONFIGURED_TARGET_FACTORY);
    let c = b
        .remove_attribute("a")
        .add(&attr("a", Type::Integer))
        .remove_attribute("b")
        .build();
    assert!(!c.get_attribute_provider().has_attr("a", Type::String));
    assert!(c.get_attribute_provider().has_attr("a", Type::Integer));
    assert!(!c.get_attribute_provider().has_attr("b", Type::String));

    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        b.remove_attribute("c");
    }));
    assert!(r.is_err());
}

#[test]
fn test_required_toolchains_are_inherited() {
    let mock_toolchain_type = label("//mock_toolchain_type");
    let parent = Arc::new(
        builder("$parent", RuleClassType::Abstract, &[])
            .add(&attr("tags", Type::StringList))
            .add_toolchain_types([ToolchainTypeRequirement::create(
                mock_toolchain_type.clone(),
            )])
            .build(),
    );
    let child = builder("child", RuleClassType::Normal, &[parent])
        .factory(DUMMY_CONFIGURED_TARGET_FACTORY)
        .add(&attr("attr", Type::String))
        .build();
    assert!(
        child
            .get_toolchain_types()
            .iter()
            .any(|t| t.toolchain_type == mock_toolchain_type)
    );
}

fn exec_group(toolchains: &[Label], constraints: &[Label]) -> DeclaredExecGroup {
    DeclaredExecGroup {
        toolchain_types_map: toolchains
            .iter()
            .map(|t| (t.clone(), ToolchainTypeRequirement::create(t.clone())))
            .collect(),
        exec_compatible_with: constraints.iter().cloned().collect(),
        copy_from_default: false,
    }
}

fn groups(entries: &[(&str, DeclaredExecGroup)]) -> IndexMap<String, DeclaredExecGroup> {
    entries
        .iter()
        .map(|(k, v)| ((*k).to_owned(), v.clone()))
        .collect()
}

#[test]
fn test_exec_groups_are_inherited() {
    let parent_group = exec_group(
        &[label("//mock_toolchain_type")],
        &[label("//mock_constraint")],
    );
    let child_group = exec_group(&[], &[]);
    let parent = Arc::new(
        builder("$parent", RuleClassType::Abstract, &[])
            .add(&attr("tags", Type::StringList))
            .add_exec_groups(&groups(&[("group", parent_group.clone())]), false)
            .build(),
    );
    let child = builder("child", RuleClassType::Normal, &[parent])
        .factory(DUMMY_CONFIGURED_TARGET_FACTORY)
        .add(&attr("attr", Type::String))
        .add_exec_groups(&groups(&[("child-group", child_group.clone())]), false)
        .build();
    assert_eq!(child.get_declared_exec_groups()["group"], parent_group);
    assert_eq!(child.get_declared_exec_groups()["child-group"], child_group);
}

#[test]
fn test_duplicate_exec_groups_that_inherit_from_rule_is_ok() {
    let mk = |name: &str, toolchain: &str| {
        Arc::new(
            builder(name, RuleClassType::Normal, &[])
                .factory(DUMMY_CONFIGURED_TARGET_FACTORY)
                .add_exec_groups(
                    &groups(&[("blueberry", DeclaredExecGroup::copy_from_default_group())]),
                    false,
                )
                .add(&attr("tags", Type::StringList))
                .add_toolchain_types([ToolchainTypeRequirement::create(label(toolchain))])
                .build(),
        )
    };
    let a = mk("ruleA", "//some/toolchain");
    let b = mk("ruleB", "//some/other/toolchain");
    let c = builder("$ruleC", RuleClassType::Abstract, &[a, b])
        .add_toolchain_types([ToolchainTypeRequirement::create(label(
            "//actual/toolchain/we/care/about",
        ))])
        .build();
    assert!(c.get_declared_exec_groups().contains_key("blueberry"));
    assert!(c.get_declared_exec_groups()["blueberry"].copy_from_default);
}

#[test]
#[should_panic(
    expected = "An execution group named 'blueberry' is inherited multiple times with different requirements in ruleC ruleclass"
)]
fn test_duplicate_exec_groups_throws_error() {
    let a = Arc::new(
        builder("ruleA", RuleClassType::Normal, &[])
            .factory(DUMMY_CONFIGURED_TARGET_FACTORY)
            .add_exec_groups(
                &groups(&[("blueberry", exec_group(&[label("//some/toolchain")], &[]))]),
                false,
            )
            .add(&attr("tags", Type::StringList))
            .build(),
    );
    let b = Arc::new(
        builder("ruleB", RuleClassType::Normal, &[])
            .factory(DUMMY_CONFIGURED_TARGET_FACTORY)
            .add_exec_groups(&groups(&[("blueberry", exec_group(&[], &[]))]), false)
            .add(&attr("tags", Type::StringList))
            .build(),
    );
    builder("ruleC", RuleClassType::Normal, &[a, b]).build();
}

#[test]
fn test_duplicate_exec_groups_overwrite() {
    let parent_group = exec_group(
        &[label("//mock_toolchain_type")],
        &[label("//mock_constraint")],
    );
    let child_group = exec_group(&[], &[]);
    let parent = Arc::new(
        builder("$parent", RuleClassType::Abstract, &[])
            .add(&attr("tags", Type::StringList))
            .add_exec_groups(&groups(&[("group", parent_group)]), false)
            .build(),
    );
    let child = builder("child", RuleClassType::Normal, &[parent])
        .factory(DUMMY_CONFIGURED_TARGET_FACTORY)
        .add(&attr("attr", Type::String))
        .add_exec_groups(&groups(&[("group", child_group.clone())]), true)
        .build();
    assert_eq!(child.get_declared_exec_groups()["group"], child_group);
}

#[test]
fn test_basic_rule_name_predicates() {
    let abcdef = RuleClassNamePredicate::only(&["abc", "def"]);
    assert!(abcdef.as_predicate_of_rule_class("abc"));
    assert!(abcdef.as_predicate_of_rule_class("def"));
    assert!(!abcdef.as_predicate_of_rule_class("ghi"));
}

#[test]
fn test_two_rule_name_predicate_factories_equivalent() {
    let a = RuleClassNamePredicate::only(&["abc", "def"]);
    let b = RuleClassNamePredicate::only(&["abc", "def"]);
    assert_eq!(a, b);
    assert_eq!(a.describe(), b.describe());
}

#[test]
fn test_everything_but_rule_name_predicates() {
    let abcdef = RuleClassNamePredicate::all_except(&["abc", "def"]);
    assert!(!abcdef.as_predicate_of_rule_class("abc"));
    assert!(!abcdef.as_predicate_of_rule_class("def"));
    assert!(abcdef.as_predicate_of_rule_class("ghi"));
}

#[test]
fn test_rule_class_name_predicate_intersection() {
    let nothing_but = |n: &[&str]| RuleClassNamePredicate::only(n);
    let all_but = |n: &[&str]| RuleClassNamePredicate::all_except(n);
    // two positives intersect iff they contain any of the same items
    assert!(nothing_but(&["abc", "def"]).considered_overlapping(&nothing_but(&["abc"])));
    assert!(!nothing_but(&["abc", "def"]).considered_overlapping(&nothing_but(&["ghi"])));

    // negatives are never considered to overlap...
    assert!(!all_but(&["abc", "def"]).considered_overlapping(&all_but(&["abc", "def"])));
    assert!(!all_but(&["abc", "def"]).considered_overlapping(&all_but(&["ghi", "jkl"])));

    assert!(!all_but(&["abc", "def"]).considered_overlapping(&nothing_but(&["abc", "def"])));
    assert!(!nothing_but(&["abc", "def"]).considered_overlapping(&all_but(&["abc", "def"])));

    assert!(!all_but(&["abc", "def"]).considered_overlapping(&nothing_but(&["abc"])));
    assert!(!all_but(&["abc"]).considered_overlapping(&nothing_but(&["abc", "def"])));
}

// ---- RuleClassTest

// PORT: Java's createParentRuleClass calls the RuleClass constructor directly (with
// DummyFragment); here the parent is built through the builder.
fn create_parent_rule_class() -> Arc<RuleClass> {
    Arc::new(
        builder("parent_rule", RuleClassType::Normal, &[])
            .factory(DUMMY_CONFIGURED_TARGET_FACTORY)
            .requires_configuration_fragments(&["DummyFragment"])
            .add(&attr("tags", Type::StringList))
            .add(&attr("attr", Type::String))
            .build(),
    )
}

#[test]
#[should_panic(
    expected = "The type of the new attribute 'int' is different from the original one 'string'."
)]
fn test_override_with_wrong_type() {
    let parent_rule_class = create_parent_rule_class();
    let mut child = builder("child_rule", RuleClassType::Normal, &[parent_rule_class]);
    child.override_(&attr("attr", Type::Integer));
}

#[test]
fn test_override_with_right_type() {
    let parent_rule_class = create_parent_rule_class();
    let mut child = builder("child_rule", RuleClassType::Normal, &[parent_rule_class]);
    child.override_(&attr("attr", Type::String));
}

#[test]
#[ignore = "not ported yet: instantiates a RuleClass built in the test with RuleClass.createRule on a Package.Builder (RuleClassTest.createRule over packageFactory.newPackageBuilder); no BUILD file is loaded, so blaze_sky_loading's PACKAGE harness does not apply, and this module has no Package.Builder fixture (tests/package_builder_test.rs builds one with Package::new_package_builder)"]
fn test_copy_and_override_attribute() {}

#[test]
#[ignore = "not ported yet: instantiates a RuleClass built in the test with RuleClass.createRule on a Package.Builder (RuleClassTest.createRule over packageFactory.newPackageBuilder); no BUILD file is loaded, so blaze_sky_loading's PACKAGE harness does not apply, and this module has no Package.Builder fixture (tests/package_builder_test.rs builds one with Package::new_package_builder)"]
fn test_copy_and_override_attribute_mandatory_missing() {}

#[test]
fn test_required_fragment_inheritance() {
    let parent_rule_class = create_parent_rule_class();
    let mut child = builder(
        "child_rule",
        RuleClassType::Normal,
        std::slice::from_ref(&parent_rule_class),
    );
    let mut copied = child.factory(DUMMY_CONFIGURED_TARGET_FACTORY).copy("attr");
    copied.mandatory();
    let child_rule_class = child.override_(&copied).build();
    assert_eq!(
        parent_rule_class
            .get_configuration_fragment_policy()
            .get_required_configuration_fragments()
            .iter()
            .collect::<Vec<_>>(),
        vec!["DummyFragment"]
    );
    assert_eq!(
        child_rule_class
            .get_configuration_fragment_policy()
            .get_required_configuration_fragments()
            .iter()
            .collect::<Vec<_>>(),
        vec!["DummyFragment"]
    );
}

#[test]
fn test_bad_rule_class_names() {
    for name in ["8abc", "!abc", "a b"] {
        let r = std::panic::catch_unwind(|| RuleClassType::Normal.check_name(name));
        assert!(r.is_err(), "{name}");
    }
}

#[test]
fn test_toolchain_types() {
    let mut b = builder("ruleClass", RuleClassType::Normal, &[]);
    b.factory(DUMMY_CONFIGURED_TARGET_FACTORY)
        .add(&attr("tags", Type::StringList));
    b.add_toolchain_types([
        ToolchainTypeRequirement::create(label("//toolchain:tc1")),
        ToolchainTypeRequirement::create(label("//toolchain:tc2")),
    ]);
    let rule_class = b.build();
    let types: Vec<Label> = rule_class
        .get_toolchain_types()
        .iter()
        .map(|t| t.toolchain_type.clone())
        .collect();
    assert!(types.contains(&label("//toolchain:tc1")));
    assert!(types.contains(&label("//toolchain:tc2")));
}

#[test]
fn test_execution_platform_constraints() {
    let mut b = builder("ruleClass", RuleClassType::Normal, &[]);
    b.factory(DUMMY_CONFIGURED_TARGET_FACTORY)
        .add(&attr("tags", Type::StringList));
    b.add_execution_platform_constraints([label("//constraints:cv1"), label("//constraints:cv2")]);
    let rule_class = b.build();
    assert_eq!(
        rule_class.get_execution_platform_constraints(),
        &IndexSet::from([label("//constraints:cv1"), label("//constraints:cv2")])
    );
}

#[test]
fn test_execution_platform_constraints_inherit_constraints_from_parent() {
    let parent = Arc::new(
        builder("$parentRuleClass", RuleClassType::Abstract, &[])
            .add(&attr("tags", Type::StringList))
            .add_execution_platform_constraints([
                label("//constraints:cv1"),
                label("//constraints:cv2"),
            ])
            .build(),
    );
    let child = builder("childRuleClass", RuleClassType::Normal, &[parent])
        .factory(DUMMY_CONFIGURED_TARGET_FACTORY)
        .build();
    assert_eq!(
        child.get_execution_platform_constraints(),
        &IndexSet::from([label("//constraints:cv1"), label("//constraints:cv2")])
    );
}

#[test]
fn test_execution_platform_constraints_inherit_and_add_constraints() {
    let parent = Arc::new(
        builder("$parentRuleClass", RuleClassType::Abstract, &[])
            .add(&attr("tags", Type::StringList))
            .build(),
    );
    let child = builder("childRuleClass", RuleClassType::Normal, &[parent])
        .factory(DUMMY_CONFIGURED_TARGET_FACTORY)
        .add_execution_platform_constraints([
            label("//constraints:cv1"),
            label("//constraints:cv2"),
        ])
        .build();
    assert_eq!(
        child.get_execution_platform_constraints(),
        &IndexSet::from([label("//constraints:cv1"), label("//constraints:cv2")])
    );
}

#[test]
fn test_declared_exec_groups() {
    let toolchain = label("//toolchain");
    let constraint = label("//constraint");
    let mut b = builder("ruleClass", RuleClassType::Normal, &[]);
    b.factory(DUMMY_CONFIGURED_TARGET_FACTORY)
        .add(&attr("tags", Type::StringList));
    b.add_exec_groups(
        &groups(&[(
            "cherry",
            exec_group(
                std::slice::from_ref(&toolchain),
                std::slice::from_ref(&constraint),
            ),
        )]),
        false,
    );
    let rule_class = b.build();
    assert_eq!(rule_class.get_declared_exec_groups().len(), 1);
    let cherry = &rule_class.get_declared_exec_groups()["cherry"];
    assert!(cherry.toolchain_type(&toolchain).is_some());
    assert!(cherry.toolchain_type(&toolchain).unwrap().mandatory);
    assert!(cherry.exec_compatible_with.contains(&constraint));
}

#[test]
fn test_build_setting_creates_default_attribute() {
    let label_flag = builder("label_flag", RuleClassType::Normal, &[])
        .factory(DUMMY_CONFIGURED_TARGET_FACTORY)
        .add(&attr("tags", Type::StringList))
        .set_build_setting(BuildSetting::create(true, Type::NodepLabel))
        .build();
    let string_setting = builder("string_setting", RuleClassType::Normal, &[])
        .factory(DUMMY_CONFIGURED_TARGET_FACTORY)
        .add(&attr("tags", Type::StringList))
        .set_build_setting(BuildSetting::create(false, Type::String))
        .build();
    assert!(
        label_flag
            .get_attribute_provider()
            .has_attr(STARLARK_BUILD_SETTING_DEFAULT_ATTR_NAME, Type::NodepLabel)
    );
    assert!(
        string_setting
            .get_attribute_provider()
            .has_attr(STARLARK_BUILD_SETTING_DEFAULT_ATTR_NAME, Type::String)
    );
}

#[test]
fn test_build_setting_does_not_create_default_attribute_if_not_build_setting() {
    let string_setting = builder("non_build_setting", RuleClassType::Normal, &[])
        .factory(DUMMY_CONFIGURED_TARGET_FACTORY)
        .add(&attr("tags", Type::StringList))
        .build();
    assert!(
        !string_setting
            .get_attribute_provider()
            .has_attr(STARLARK_BUILD_SETTING_DEFAULT_ATTR_NAME, Type::Label)
    );
}

#[test]
#[should_panic(expected = "Rule class myclass declared too many attributes (202 > 200)")]
fn test_build_too_many_attributes_rejected() {
    let mut b = builder("myclass", RuleClassType::Normal, &[]);
    b.factory(DUMMY_CONFIGURED_TARGET_FACTORY)
        .add(&attr("tags", Type::StringList));
    for i in 0..200 {
        b.add(&attr(&format!("attr{i}"), Type::String));
    }
    b.build();
}

#[test]
fn test_build_too_long_attribute_name_rejected() {
    let r = std::panic::catch_unwind(|| {
        builder("myclass", RuleClassType::Normal, &[])
            .factory(DUMMY_CONFIGURED_TARGET_FACTORY)
            .add(&attr("tags", Type::StringList))
            .add(&attr(&"x".repeat(150), Type::String))
            .build();
    });
    let e = r.unwrap_err();
    let msg = e.downcast_ref::<String>().cloned().unwrap_or_default();
    assert!(
        msg.ends_with(&format!(
            "Attribute myclass.{}'s name is too long (150 > 128)",
            "x".repeat(150)
        )),
        "{msg}"
    );
}

#[test]
#[ignore = "not ported yet: instantiates a RuleClass built in the test with RuleClass.createRule on a Package.Builder (RuleClassTest.createRule over packageFactory.newPackageBuilder); no BUILD file is loaded, so blaze_sky_loading's PACKAGE harness does not apply, and this module has no Package.Builder fixture (tests/package_builder_test.rs builds one with Package::new_package_builder)"]
fn test_package_metadata_alternate_name() {
    let _ = APPLICABLE_METADATA_ATTR;
}

#[test]
#[ignore = "not ported yet: needs only the RuleClass builder (RuleClassTest.createRuleClassA / createRuleClassB); no package or rule instance is involved"]
fn test_rule_class_basics() {}

#[test]
#[ignore = "not ported yet: needs only the RuleClass builder (RuleClassTest.createRuleClassA / createRuleClassB); no package or rule instance is involved"]
fn test_rule_class_inheritance() {}

#[test]
#[ignore = "not ported yet: instantiates a RuleClass built in the test with RuleClass.createRule on a Package.Builder (RuleClassTest.createRule over packageFactory.newPackageBuilder); no BUILD file is loaded, so blaze_sky_loading's PACKAGE harness does not apply, and this module has no Package.Builder fixture (tests/package_builder_test.rs builds one with Package::new_package_builder)"]
fn test_duplicated_deps() {}

#[test]
#[ignore = "not ported yet: instantiates a RuleClass built in the test with RuleClass.createRule on a Package.Builder (RuleClassTest.createRule over packageFactory.newPackageBuilder); no BUILD file is loaded, so blaze_sky_loading's PACKAGE harness does not apply, and this module has no Package.Builder fixture (tests/package_builder_test.rs builds one with Package::new_package_builder)"]
fn test_create_rule() {}

#[test]
#[ignore = "not ported yet: instantiates a RuleClass built in the test with RuleClass.createRule on a Package.Builder (RuleClassTest.createRule over packageFactory.newPackageBuilder); no BUILD file is loaded, so blaze_sky_loading's PACKAGE harness does not apply, and this module has no Package.Builder fixture (tests/package_builder_test.rs builds one with Package::new_package_builder)"]
fn test_implicit_outputs() {}

#[test]
#[ignore = "not ported yet: instantiates a RuleClass built in the test with RuleClass.createRule on a Package.Builder (RuleClassTest.createRule over packageFactory.newPackageBuilder); no BUILD file is loaded, so blaze_sky_loading's PACKAGE harness does not apply, and this module has no Package.Builder fixture (tests/package_builder_test.rs builds one with Package::new_package_builder)"]
fn test_computed_default() {}

#[test]
#[ignore = "not ported yet: instantiates a RuleClass built in the test with RuleClass.createRule on a Package.Builder (RuleClassTest.createRule over packageFactory.newPackageBuilder); no BUILD file is loaded, so blaze_sky_loading's PACKAGE harness does not apply, and this module has no Package.Builder fixture (tests/package_builder_test.rs builds one with Package::new_package_builder)"]
fn test_outputs_are_ordered() {}

#[test]
#[ignore = "not ported yet: instantiates a RuleClass built in the test with RuleClass.createRule on a Package.Builder (RuleClassTest.createRule over packageFactory.newPackageBuilder); no BUILD file is loaded, so blaze_sky_loading's PACKAGE harness does not apply, and this module has no Package.Builder fixture (tests/package_builder_test.rs builds one with Package::new_package_builder)"]
fn test_order_independent_attribute() {}

// ---- RequiredProvidersTest

fn id_native() -> StarlarkProviderIdentifier {
    StarlarkProviderIdentifier::for_key(ProviderKey::Builtin {
        name: "p_native".to_owned(),
        provider_class: "p_native".to_owned(),
    })
}

fn id_starlark() -> StarlarkProviderIdentifier {
    StarlarkProviderIdentifier::for_key(ProviderKey::Starlark {
        extension_label: Label::create("foo/bar", "x.bzl").unwrap(),
        exported_name: "p_starlark".to_owned(),
    })
}

fn satisfies(providers: &AdvertisedProviderSet, required: &RequiredProviders) -> bool {
    let result = required.is_satisfied_by(providers);
    assert_eq!(
        required
            .is_satisfied_by_predicates(&|p| providers.get_builtin_providers().contains(p), &|p| {
                providers.get_starlark_providers().contains(p)
            },),
        result
    );
    result
}

fn builtin_set(names: &[&str]) -> AdvertisedProviderSet {
    let mut b = AdvertisedProviderSet::builder();
    for n in names {
        b.add_builtin(n);
    }
    b.build()
}

#[test]
fn any() {
    let any = RequiredProviders::accept_any_builder().build();
    assert!(satisfies(&AdvertisedProviderSet::empty(), &any));
    assert!(satisfies(&AdvertisedProviderSet::any(), &any));
    assert!(satisfies(&builtin_set(&["P1"]), &any));
}

#[test]
fn none() {
    let none = RequiredProviders::accept_none_builder().build();
    assert!(!satisfies(&AdvertisedProviderSet::empty(), &none));
    assert!(!satisfies(&AdvertisedProviderSet::any(), &none));
    assert!(!satisfies(&builtin_set(&["P1"]), &none));
    assert!(!satisfies(&AdvertisedProviderSet::builder().build(), &none));
}

fn validate_native(provider_set: &AdvertisedProviderSet, missing: &str, sets: &[&[&str]]) -> bool {
    let mut any_builder = RequiredProviders::accept_any_builder();
    let mut none_builder = RequiredProviders::accept_none_builder();
    for set in sets {
        any_builder.add_builtin_set(set.iter().map(|s| (*s).to_owned()).collect());
        none_builder.add_builtin_set(set.iter().map(|s| (*s).to_owned()).collect());
    }
    let rp_any = any_builder.build();
    let result = satisfies(provider_set, &rp_any);
    assert_eq!(rp_any.get_missing(provider_set).get_description(), missing);
    let rp_none = none_builder.build();
    assert_eq!(satisfies(provider_set, &rp_none), result);
    assert_eq!(rp_none.get_missing(provider_set).get_description(), missing);
    result
}

fn validate_starlark(
    provider_set: &AdvertisedProviderSet,
    missing: &str,
    sets: &[Vec<StarlarkProviderIdentifier>],
) -> bool {
    let mut any_builder = RequiredProviders::accept_any_builder();
    let mut none_builder = RequiredProviders::accept_none_builder();
    for set in sets {
        any_builder.add_starlark_set(set.clone());
        none_builder.add_starlark_set(set.clone());
    }
    let rp_any = any_builder.build();
    let result = satisfies(provider_set, &rp_any);
    assert_eq!(rp_any.get_missing(provider_set).get_description(), missing);
    let rp_none = none_builder.build();
    assert_eq!(satisfies(provider_set, &rp_none), result);
    assert_eq!(rp_none.get_missing(provider_set).get_description(), missing);
    result
}

const NO_PROVIDERS_REQUIRED: &str = "no providers required";

#[test]
fn builtin_providers_all_match() {
    assert!(validate_native(
        &builtin_set(&["P1", "P2"]),
        NO_PROVIDERS_REQUIRED,
        &[&["P1", "P2"]]
    ));
}

#[test]
fn builtin_providers_branch_match() {
    assert!(validate_native(
        &builtin_set(&["P1"]),
        NO_PROVIDERS_REQUIRED,
        &[&["P1"], &["P2"]]
    ));
}

#[test]
fn builtins_providers_no_match() {
    assert!(!validate_native(
        &builtin_set(&["P3"]),
        "P1 or P2",
        &[&["P1"], &["P2"]]
    ));
}

#[test]
fn starlark_providers_all_match() {
    let mut b = AdvertisedProviderSet::builder();
    b.add_starlark(id_native()).add_starlark(id_starlark());
    let provider_set = b.build();
    assert!(validate_starlark(
        &provider_set,
        NO_PROVIDERS_REQUIRED,
        &[vec![id_starlark(), id_native()]]
    ));
}

#[test]
fn check_descriptions() {
    assert_eq!(
        RequiredProviders::accept_any_builder()
            .build()
            .get_description(),
        "no providers required"
    );
    assert_eq!(
        RequiredProviders::accept_none_builder()
            .build()
            .get_description(),
        "no providers accepted"
    );
    let mut b = RequiredProviders::accept_any_builder();
    b.add_starlark_set(vec![id_starlark()])
        .add_builtin_set(vec!["P1".to_owned(), "P2".to_owned()]);
    assert_eq!(b.build().get_description(), "[P1, P2] or 'p_starlark'");
}

#[test]
fn get_starlark_providers() {
    assert!(
        RequiredProviders::accept_any_builder()
            .build()
            .get_starlark_providers()
            .is_empty()
    );
    assert!(
        RequiredProviders::accept_none_builder()
            .build()
            .get_starlark_providers()
            .is_empty()
    );
    let mut b = RequiredProviders::accept_any_builder();
    b.add_starlark_set(vec![id_starlark()])
        .add_builtin_set(vec!["P1".to_owned(), "P2".to_owned()])
        .add_builtin_set(vec!["P3".to_owned()]);
    let rp = b.build();
    assert_eq!(rp.get_starlark_providers().len(), 1);
    assert_eq!(
        rp.get_starlark_providers()[0],
        IndexSet::from([id_starlark()])
    );
}

// ---- AdvertisedProviderSetTest

fn get_fingerprint(set: &AdvertisedProviderSet) -> String {
    let mut fp = blaze_util::Fingerprint::new();
    set.fingerprint(&mut fp);
    hex::encode(fp.digest_and_reset())
}

fn starlark_id(bzl: &str, name: &str) -> StarlarkProviderIdentifier {
    StarlarkProviderIdentifier::for_key(ProviderKey::Starlark {
        extension_label: label(bzl),
        exported_name: name.to_owned(),
    })
}

#[test]
fn fingerprints_match_exactly() {
    // PORT: the builtin-provider cases (String.class, Integer.class) are fingerprinted by Java
    // canonical class name, which builtin providers here do not carry.
    assert_eq!(
        get_fingerprint(&AdvertisedProviderSet::any()),
        "4bf5122f344554c53bde2ebb8cd2b7e3d1600ad631c385a5d7cce23c7785459a"
    );
    assert_eq!(
        get_fingerprint(&AdvertisedProviderSet::empty()),
        "6e340b9cffb37a989ca544e6bb780a2c78901d3fb33738768511a30617afa01d"
    );
    let mut b = AdvertisedProviderSet::builder();
    b.add_starlark(starlark_id("//my:label1.bzl", "exportedName1"));
    assert_eq!(
        get_fingerprint(&b.build()),
        "a17a99d19f39ae61e927da9169d42274b7121fa0fbc79bbb85cce7b199576a42"
    );
    let mut b = AdvertisedProviderSet::builder();
    b.add_starlark(starlark_id("//my:label1.bzl", "exportedName1"))
        .add_starlark(starlark_id("//my:label2.bzl", "exportedName2"));
    assert_eq!(
        get_fingerprint(&b.build()),
        "2909013264231d50a3550d41e2eaf0fbd501bfffacf6303019cc8a93e187bf68"
    );
    let mut b = AdvertisedProviderSet::builder();
    b.add_starlark(starlark_id("//my:label2.bzl", "exportedName2"));
    assert_eq!(
        get_fingerprint(&b.build()),
        "4d5efc81e801feaeef6b5549d2c4b9d943ae673c76456f85631dc61cae698e4c"
    );
}

#[test]
fn predicate_describe() {
    // Java RuleClassNamePredicate.toString (used in attribute error messages).
    assert_eq!(
        RuleClassNamePredicate::only(&["a", "b", "c"]).describe(),
        "a, b, or c"
    );
    assert_eq!(
        RuleClassNamePredicate::all_except(&["a"]).describe(),
        "all but a"
    );
}
