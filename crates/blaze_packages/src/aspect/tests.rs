//! Ports of `src/test/java/.../analysis/AspectDescriptorTest.java` and
//! `src/test/java/.../analysis/AspectDefinitionTest.java` (they test `packages` classes).
//! Expected values come from the Java tests.

use std::any::Any;
use std::sync::Arc;
use std::sync::LazyLock;

use blaze_cmdline::Label;
use blaze_starlark::StarlarkSemantics;
use starlark::values::FrozenHeap;

use super::*;
use crate::attribute::attr;
use crate::attribute::computed_default::LateBoundDefault;
use crate::attribute::computed_default::LateBoundDefaultKind;
use crate::rule_class::AdvertisedProviderSet;
use crate::rule_class::MissingFragmentPolicy;
use crate::rule_class::ProviderKey;
use crate::rule_class::StarlarkCallable;
use crate::rule_class::StarlarkProviderIdentifier;
use crate::target::RuleOrMacroInstance;
use crate::types::AttrValue;
use crate::types::FileTypeSet;
use crate::types::Type;

// ---------------------------------------------------------------------------------------------
// AspectDescriptorTest
// ---------------------------------------------------------------------------------------------

/// Java `AspectDescriptorTest.aspectClass(name)`: an anonymous `NativeAspectClass` whose
/// `getDefinition` throws `UnsupportedOperationException`.
struct NamedAspectClass(String);

impl NativeAspectClass for NamedAspectClass {
    fn get_name(&self) -> &str {
        &self.0
    }

    fn get_definition(&self, _aspect_parameters: &AspectParameters) -> Arc<AspectDefinition> {
        panic!("UnsupportedOperationException")
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

fn aspect_class(name: &str) -> AspectClass {
    AspectClass::Native(Arc::new(NamedAspectClass(name.to_owned())))
}

fn aspect_descriptor(aspect_class: AspectClass, parameters: &[&str]) -> AspectDescriptor {
    assert_eq!(parameters.len() % 2, 0);
    let mut params = AspectParameters::builder();
    for pair in parameters.chunks(2) {
        params.add_attribute(pair[0], pair[1]);
    }
    AspectDescriptor::of(aspect_class, params.build())
}

fn assert_description(description: &str, aspect_class_name: &str, params: &[&str]) {
    assert_eq!(
        aspect_descriptor(aspect_class(aspect_class_name), params).get_description(),
        description
    );
}

#[test]
fn serialize_descriptor_no_arguments() {
    assert_description("foobar", "foobar", &[]);
}

#[test]
fn serialize_descriptor_argument() {
    assert_description("foobar[x=\"1\"]", "foobar", &["x", "1"]);
}

#[test]
fn serialize_descriptor_argument_escaped() {
    assert_description("foobar[x=\"\\\"1\\\"\"]", "foobar", &["x", "\"1\""]);
}

#[test]
fn serialize_descriptor_two_arguments() {
    assert_description("foobar[x=\"1\",y=\"2\"]", "foobar", &["x", "1", "y", "2"]);
}

#[test]
fn serialize_descriptor_two_arguments_multi() {
    assert_description(
        "foobar[x=\"1\",y=\"2\",y=\"3\"]",
        "foobar",
        &["x", "1", "y", "2", "y", "3"],
    );
}

// ---------------------------------------------------------------------------------------------
// AspectDefinitionTest
// ---------------------------------------------------------------------------------------------

/// Java `AspectDefinitionTest.TestAspectClass` (a `NativeAspectClass` returning a set
/// definition; the ConfiguredAspectFactory part is analysis).
struct TestAspectClass;

impl NativeAspectClass for TestAspectClass {
    fn get_name(&self) -> &str {
        "TestAspectClass"
    }

    fn get_definition(&self, _aspect_parameters: &AspectParameters) -> Arc<AspectDefinition> {
        // Java returns the definition set with setAspectDefinition (null when unset); no test
        // here calls it.
        panic!("TestAspectClass definition not set")
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

fn test_aspect_class() -> AspectClass {
    AspectClass::Native(Arc::new(TestAspectClass))
}

static FAKE_LABEL: LazyLock<Label> =
    LazyLock::new(|| Label::parse_canonical_unchecked("//fake/label.bzl"));

fn starlark_p(name: &str) -> StarlarkProviderIdentifier {
    StarlarkProviderIdentifier::for_key(ProviderKey::Starlark {
        extension_label: FAKE_LABEL.clone(),
        exported_name: name.to_owned(),
    })
}

fn advertised(providers: &[StarlarkProviderIdentifier]) -> AdvertisedProviderSet {
    let mut b = AdvertisedProviderSet::builder();
    for p in providers {
        b.add_starlark(p.clone());
    }
    b.build()
}

/// Java `new AspectPropagationPredicate(null, null)`: a predicate that is never called.
fn dummy_predicate(heap: &FrozenHeap) -> AspectPropagationPredicate {
    AspectPropagationPredicate::new(
        StarlarkCallable::new(heap.alloc("predicate")),
        Arc::new(StarlarkSemantics::default()),
    )
}

#[test]
fn test_aspect_with_implicit_or_latebound_attribute_adds_to_attribute_map() {
    let implicit = attr("$runtime", Type::Label)
        .value(Some(AttrValue::Label(Label::parse_canonical_unchecked(
            "//run:time",
        ))))
        .build();
    // Java: LateBoundDefault.fromConstantForTesting(Label.parseCanonicalUnchecked("//run:away"))
    let away = Label::parse_canonical_unchecked("//run:away");
    let latebound = Arc::new(LateBoundDefault::new(
        None,
        Arc::new(move |_: Option<&dyn RuleOrMacroInstance>| Some(AttrValue::Label(away.clone()))),
        LateBoundDefaultKind::Label,
    ));
    let simple = AspectDefinitionBuilder::new(test_aspect_class())
        .add(implicit.clone())
        .add_builder(attr(":latebound", Type::Label).value_late_bound(latebound.clone()))
        .build();
    assert_eq!(simple.get_attributes().get("$runtime"), Some(&implicit));
    assert!(simple.get_attributes().contains_key(":latebound"));
    assert!(Arc::ptr_eq(
        simple
            .get_attributes()
            .get(":latebound")
            .unwrap()
            .get_late_bound_default()
            .unwrap(),
        &latebound
    ));
}

#[test]
#[should_panic(expected = "IllegalArgumentException")]
fn test_aspect_with_duplicate_attribute_fails_to_add() {
    AspectDefinitionBuilder::new(test_aspect_class())
        .add_builder(attr("$runtime", Type::Label).value(Some(AttrValue::Label(
            Label::parse_canonical_unchecked("//run:time"),
        ))))
        .add_builder(attr("$runtime", Type::Label).value(Some(AttrValue::Label(
            Label::parse_canonical_unchecked("//oops"),
        ))));
}

#[test]
#[should_panic(expected = "IllegalArgumentException")]
fn test_aspect_with_user_visible_attribute_fails_to_add() {
    AspectDefinitionBuilder::new(test_aspect_class())
        .add_builder(
            attr("invalid", Type::Label)
                .value(Some(AttrValue::Label(Label::parse_canonical_unchecked(
                    "//run:time",
                ))))
                .allowed_file_types(FileTypeSet::NoFile),
        )
        .build();
}

#[test]
fn test_require_starlark_providers_adds_flat_set_of_required_providers() {
    let requires_providers = AspectDefinitionBuilder::new(test_aspect_class())
        .require_starlark_providers(&[starlark_p("STARLARK_P1"), starlark_p("STARLARK_P2")])
        .build();

    let expected_ok_set = advertised(&[
        starlark_p("STARLARK_P1"),
        starlark_p("STARLARK_P2"),
        starlark_p("STARLARK_P3"),
    ]);
    assert!(
        requires_providers
            .get_required_providers()
            .is_satisfied_by(&expected_ok_set)
    );

    let expected_fail_set = advertised(&[starlark_p("STARLARK_P1")]);
    assert!(
        !requires_providers
            .get_required_providers()
            .is_satisfied_by(&expected_fail_set)
    );

    assert!(
        requires_providers
            .get_required_providers()
            .is_satisfied_by(&AdvertisedProviderSet::any())
    );
    assert!(
        !requires_providers
            .get_required_providers()
            .is_satisfied_by(&AdvertisedProviderSet::empty())
    );
}

#[test]
fn test_require_starlark_providers_adds_two_sets_of_required_providers() {
    let requires_providers = AspectDefinitionBuilder::new(test_aspect_class())
        .require_starlark_provider_sets(vec![
            vec![starlark_p("STARLARK_P1"), starlark_p("STARLARK_P2")],
            vec![starlark_p("STARLARK_P3")],
        ])
        .build();

    let expected_ok_set1 = advertised(&[starlark_p("STARLARK_P1"), starlark_p("STARLARK_P2")]);
    assert!(
        requires_providers
            .get_required_providers()
            .is_satisfied_by(&expected_ok_set1)
    );

    let expected_ok_set2 = advertised(&[starlark_p("STARLARK_P3")]);
    assert!(
        requires_providers
            .get_required_providers()
            .is_satisfied_by(&expected_ok_set2)
    );

    let expected_fail_set = advertised(&[starlark_p("STARLARK_P4")]);
    assert!(
        !requires_providers
            .get_required_providers()
            .is_satisfied_by(&expected_fail_set)
    );

    assert!(
        requires_providers
            .get_required_providers()
            .is_satisfied_by(&AdvertisedProviderSet::any())
    );
    assert!(
        !requires_providers
            .get_required_providers()
            .is_satisfied_by(&AdvertisedProviderSet::empty())
    );
}

#[test]
fn test_require_providers_default_accepts_everything() {
    let no_required_providers = AspectDefinitionBuilder::new(test_aspect_class()).build();

    let mut b = AdvertisedProviderSet::builder();
    b.add_builtin("P1").add_starlark(starlark_p("STARLARK_P4"));
    let expected_ok_set = b.build();
    assert!(
        no_required_providers
            .get_required_providers()
            .is_satisfied_by(&expected_ok_set)
    );

    assert!(
        no_required_providers
            .get_required_providers()
            .is_satisfied_by(&AdvertisedProviderSet::any())
    );
    assert!(
        no_required_providers
            .get_required_providers()
            .is_satisfied_by(&AdvertisedProviderSet::empty())
    );
}

#[test]
fn test_require_aspect_class_default_accepts_nothing() {
    let no_aspects = AspectDefinitionBuilder::new(test_aspect_class()).build();

    let mut b = AdvertisedProviderSet::builder();
    b.add_builtin("P1");
    let expected_fail_set = b.build();

    assert!(
        !no_aspects
            .get_required_providers_for_aspects()
            .is_satisfied_by(&AdvertisedProviderSet::any())
    );
    assert!(
        !no_aspects
            .get_required_providers_for_aspects()
            .is_satisfied_by(&AdvertisedProviderSet::empty())
    );

    assert!(
        !no_aspects
            .get_required_providers_for_aspects()
            .is_satisfied_by(&expected_fail_set)
    );
}

#[test]
fn test_no_configuration_fragment_policy_setup_has_non_null_policy() {
    let no_policy = AspectDefinitionBuilder::new(test_aspect_class()).build();
    // Java: isNotNull(); the Rust policy is never absent.
    let _policy = no_policy.get_configuration_fragment_policy();
}

#[test]
fn test_requires_configuration_fragments_propagated_to_configuration_fragment_policy() {
    let requires_fragments = AspectDefinitionBuilder::new(test_aspect_class())
        .requires_configuration_fragments(&["FooFragment", "BarFragment"])
        .build();
    assert_eq!(
        requires_fragments
            .get_configuration_fragment_policy()
            .get_required_configuration_fragments()
            .iter()
            .cloned()
            .collect::<Vec<_>>(),
        vec!["FooFragment".to_owned(), "BarFragment".to_owned()]
    );
}

#[test]
fn test_missing_fragment_policy_propagated_to_configuration_fragment_policy() {
    let missing_fragments = AspectDefinitionBuilder::new(test_aspect_class())
        .set_missing_fragment_policy("FooFragment", MissingFragmentPolicy::Ignore)
        .build();
    assert_eq!(
        missing_fragments
            .get_configuration_fragment_policy()
            .get_missing_fragment_policy("FooFragment"),
        MissingFragmentPolicy::Ignore
    );
}

#[test]
fn test_requires_configuration_fragment_names_propagated_to_configuration_fragment_policy() {
    let requires_fragments = AspectDefinitionBuilder::new(test_aspect_class())
        .requires_configuration_fragments_by_starlark_builtin_name(&["test_fragment".to_owned()])
        .build();
    // Java: TestFragment is `@StarlarkBuiltin(name = "test_fragment")`.
    assert!(
        requires_fragments
            .get_configuration_fragment_policy()
            .is_legal_configuration_fragment("TestFragment", Some("test_fragment"))
    );
}

#[test]
#[should_panic(
    expected = "An aspect cannot simultaneously have required providers and apply to files."
)]
fn test_aspect_with_apply_to_files_requires_providers_fails() {
    AspectDefinitionBuilder::new(test_aspect_class())
        .apply_to_files(true)
        .require_starlark_providers(&[starlark_p("STARLARK_P1"), starlark_p("STARLARK_P2")])
        .build();
}

#[test]
#[should_panic(
    expected = "An aspect cannot simultaneously have required providers and apply to generating \
                rules."
)]
fn test_aspect_with_apply_to_generating_rules_requires_providers_fails() {
    AspectDefinitionBuilder::new(test_aspect_class())
        .apply_to_generating_rules(true)
        .require_starlark_providers(&[starlark_p("STARLARK_P1"), starlark_p("STARLARK_P2")])
        .build();
}

#[test]
#[should_panic(
    expected = "An aspect cannot simultaneously have a propagation predicate and apply to files."
)]
fn test_aspect_with_apply_to_files_has_propagation_predicate_fails() {
    let heap = FrozenHeap::new();
    AspectDefinitionBuilder::new(test_aspect_class())
        .apply_to_files(true)
        .propagation_predicate(Some(dummy_predicate(&heap)))
        .build();
}

#[test]
#[should_panic(
    expected = "An aspect cannot simultaneously have a propagation predicate and apply to \
                generating rules."
)]
fn test_aspect_with_apply_to_generating_rules_has_propagation_predicate_fails() {
    let heap = FrozenHeap::new();
    AspectDefinitionBuilder::new(test_aspect_class())
        .apply_to_generating_rules(true)
        .propagation_predicate(Some(dummy_predicate(&heap)))
        .build();
}

// ---------------------------------------------------------------------------------------------
// Blaze checks of Java semantics without a JUnit counterpart (expected values from the Java
// sources named in each test).
// ---------------------------------------------------------------------------------------------

/// `AspectParameters.equals` is Guava multimap equality; `toString` is the multimap's.
#[test]
fn aspect_parameters_multimap_semantics() {
    let mut a = AspectParameters::builder();
    a.add_attribute("x", "1")
        .add_attribute("y", "2")
        .add_attribute("y", "3");
    let a = a.build();
    let mut b = AspectParameters::builder();
    b.add_attribute("y", "2")
        .add_attribute("x", "1")
        .add_attribute("y", "3");
    let b = b.build();
    assert_eq!(a, b);
    assert_eq!(a.to_string(), "{x=[1], y=[2, 3]}");
    let mut c = AspectParameters::builder();
    c.add_attribute("y", "3")
        .add_attribute("y", "2")
        .add_attribute("x", "1");
    assert_ne!(a, c.build());
    assert!(AspectParameters::builder().build().is_empty());
    assert!(!a.is_empty());
    assert_eq!(a.get_only_value_of_attribute("x"), "1");
    assert_eq!(a.get_attribute("missing"), &[] as &[String]);
}

/// `Aspect.forNative` caches definitions per class and parameters, so equal aspects share
/// their definition; `Aspect.equals` needs the same definition object.
#[test]
fn aspect_for_native_caches_definition() {
    struct CachedAspectClass;
    impl NativeAspectClass for CachedAspectClass {
        fn get_name(&self) -> &str {
            "CachedAspectClassForTest"
        }
        fn get_definition(&self, _p: &AspectParameters) -> Arc<AspectDefinition> {
            AspectDefinitionBuilder::new(AspectClass::Native(Arc::new(CachedAspectClass))).build()
        }
        fn as_any(&self) -> &dyn Any {
            self
        }
    }
    let class: Arc<dyn NativeAspectClass> = Arc::new(CachedAspectClass);
    let a = Aspect::for_native_without_parameters(&class);
    let b = Aspect::for_native(&class, AspectParameters::empty());
    assert_eq!(a, b);
    assert!(Arc::ptr_eq(a.get_definition(), b.get_definition()));
    assert_eq!(a.to_string(), "Aspect CachedAspectClassForTest");

    let other = Aspect::for_starlark(
        StarlarkAspectClass::new(FAKE_LABEL.clone(), "a"),
        a.get_definition().clone(),
        AspectParameters::empty(),
    );
    assert_ne!(a, other);
}

/// `StarlarkAspectClass.getAspectClassFromName` error texts.
#[test]
fn starlark_aspect_class_from_name() {
    let c = StarlarkAspectClass::get_aspect_class_from_name("//foo:bar.bzl%my_aspect").unwrap();
    assert_eq!(c.get_name(), "//foo:bar.bzl%my_aspect");
    assert_eq!(c.get_exported_name(), "my_aspect");
    assert_eq!(
        c,
        StarlarkAspectClass::new(
            Label::parse_canonical_unchecked("//foo:bar.bzl"),
            "my_aspect"
        )
    );
    assert_eq!(
        StarlarkAspectClass::get_aspect_class_from_name("//foo:bar.bzl")
            .unwrap_err()
            .0,
        "--exec_aspects must include the aspect name, preceded by '%', e.g. \
         //foo/bar:baz.bzl%my_aspect, @repo//foo/bar:baz%my_aspect, or \
         /foo/bar:baz.bzl%my_aspect. Found: //foo:bar.bzl"
    );
    assert_eq!(
        StarlarkAspectClass::get_aspect_class_from_name("//foo:bar%a")
            .unwrap_err()
            .0,
        "--exec_aspects files must end with .bzl. Found: //foo:bar%a"
    );
}
