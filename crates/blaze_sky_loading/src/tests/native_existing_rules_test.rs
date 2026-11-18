//! Port of NativeExistingRulesTest (tests for the `native.existing_rule` and
//! `native.existing_rules` functions) on the PACKAGE harness.
//!
//! Java: src/test/java/com/google/devtools/build/lib/packages/NativeExistingRulesTest.java.
//!
//! Most of the Java tests capture Starlark values through a test-only bzl toplevel
//! (`test.save(name, value)`, a `TestStarlarkBuiltin` added to the rule class provider) and
//! assert on them in Java; several also load `@rules_cc`. The harness cannot add a toplevel to
//! Bazel's rule class provider and has no external repositories, so those tests are ignored
//! below. The three ported tests assert on loading errors only: Java's
//! `getConfiguredTarget("//test:BUILD")` returning null becomes "the package is in error".

use crate::tests::package_loading_test_case::{PackageLoadingTestCase, assert_contains_event};

// Java: NativeExistingRulesTest.existingRule_returnsImmutableObject
#[test]
#[ignore = "existing_rule() returns an immutable dict where Java returns a non-dict view (ExistingRuleView): the assignment fails with 'trying to mutate a frozen dict value', Java 'can only assign an element in a dictionary or a list'"]
fn existing_rule_returns_immutable_object() {
    let t = PackageLoadingTestCase::new();
    t.scratch_file("test/BUILD", &["load(\"inc.bzl\", \"f\")", "", "f()"]);
    t.scratch_file(
        "test/inc.bzl",
        &[
            "def f():",
            "    native.config_setting(name = \"x\", define_values = {\"key\": \"value\"})",
            "    r = native.existing_rule(\"x\")",
            "    r[\"no_such_attribute\"] = 123",
        ],
    ); // mutate the view

    let events = t.load_failure_events("test"); // mutation fails
    assert_contains_event(
        &events,
        "can only assign an element in a dictionary or a list",
    );
}

// Java: NativeExistingRulesTest.existingRules_returnsImmutableObject
#[test]
#[ignore = "existing_rules() returns an immutable dict where Java returns a non-dict view (ExistingRulesView): the assignment fails with 'trying to mutate a frozen dict value', Java 'can only assign an element in a dictionary or a list'"]
fn existing_rules_returns_immutable_object() {
    let t = PackageLoadingTestCase::new();
    t.scratch_file("test/BUILD", &["load(\"inc.bzl\", \"f\")", "", "f()"]);
    t.scratch_file(
        "test/inc.bzl",
        &[
            "def f():",
            "    native.config_setting(name = \"x\", define_values = {\"key\": \"value\"})",
            "    rs = native.existing_rules()",
            "    rs[\"no_such_rule\"] = {\"name\": \"no_such_rule\", \"kind\": \"config_setting\"}",
        ],
    ); // mutate

    let events = t.load_failure_events("test"); // mutation fails
    assert_contains_event(
        &events,
        "can only assign an element in a dictionary or a list",
    );
}

// Java: NativeExistingRulesTest.existingRules_returnsDeeplyImmutableView
#[test]
fn existing_rules_returns_deeply_immutable_view() {
    let t = PackageLoadingTestCase::new();
    t.scratch_file("test/BUILD", &["load(\"inc.bzl\", \"f\")", "", "f()"]);
    t.scratch_file(
        "test/inc.bzl",
        &[
            "def f():",
            "    native.config_setting(name = \"x\", define_values = {\"key\": \"value\"})",
            "    rs = native.existing_rules()",
            "    rs[\"x\"][\"define_values\"][\"key\"] = 123",
        ],
    ); // mutate an attribute value within the view

    let events = t.load_failure_events("test");
    assert_contains_event(&events, "trying to mutate a frozen dict value");
}

macro_rules! ignored {
    ($($name:ident: $reason:literal,)*) => {
        $(
            #[test]
            #[ignore = $reason]
            fn $name() {}
        )*
    };
}

ignored! {
    existing_rule_handles_select: "loads @rules_cc//cc:cc_library.bzl and sets cc_library's srcs: needs the @rules_cc repository in the scratch workspace (the harness has no external repositories)",
    existing_rule_handles_select_with_none_values_for_label_valued_attributes: "asserts on a value captured by the test-only bzl toplevel test.save (Java TestStarlarkBuiltin added to the rule class provider); the harness cannot add a toplevel to Bazel's rule class provider",
    existing_rule_returns_none: "loads @rules_cc//cc:cc_library.bzl: needs the @rules_cc repository in the scratch workspace (the harness has no external repositories)",
    existing_rule_round_trips_select: "asserts on values captured by the test-only bzl toplevel test.save (Java TestStarlarkBuiltin added to the rule class provider), and loads @rules_cc; the harness has neither",
    existing_rule_label_stringification: "asserts on a value captured by the test-only bzl toplevel test.save (Java TestStarlarkBuiltin added to the rule class provider), and loads @rules_cc; the harness has neither",
    existing_rules_finds_rules_and_attributes: "asserts on values captured by the test-only bzl toplevel test.save (Java TestStarlarkBuiltin added to the rule class provider); the harness cannot add a toplevel to Bazel's rule class provider",
    existing_rule_ignores_hidden_attributes: "asserts on values captured by the test-only bzl toplevel test.save (Java TestStarlarkBuiltin added to the rule class provider); the harness cannot add a toplevel to Bazel's rule class provider",
    existing_rule_returns_dict_like_object: "asserts on values captured by the test-only bzl toplevel test.save (Java TestStarlarkBuiltin added to the rule class provider); the harness cannot add a toplevel to Bazel's rule class provider",
    existing_rule_as_dict_argument: "asserts on values captured by the test-only bzl toplevel test.save (Java TestStarlarkBuiltin added to the rule class provider), and loads @rules_cc; the harness has neither",
    existing_rule_as_dict_update_argument: "asserts on values captured by the test-only bzl toplevel test.save (Java TestStarlarkBuiltin added to the rule class provider), and loads @rules_cc; the harness has neither",
    existing_rule_unionable_with_dict: "asserts on values captured by the test-only bzl toplevel test.save (Java TestStarlarkBuiltin added to the rule class provider), and loads @rules_cc; the harness has neither",
    existing_rule_as_kwargs: "asserts on values captured by the test-only bzl toplevel test.save (Java TestStarlarkBuiltin added to the rule class provider), and loads @rules_cc; the harness has neither",
    existing_rule_encodes_to_json: "asserts on a value captured by the test-only bzl toplevel test.save (Java TestStarlarkBuiltin added to the rule class provider); the harness cannot add a toplevel to Bazel's rule class provider",
    existing_rules_returns_dict_like_object: "asserts on values captured by the test-only bzl toplevel test.save (Java TestStarlarkBuiltin added to the rule class provider); the harness cannot add a toplevel to Bazel's rule class provider",
    existing_rules_returns_snapshot_of_only_rules_instantiated_up_to_that_point: "asserts on values captured by the test-only bzl toplevel test.save (Java TestStarlarkBuiltin added to the rule class provider); the harness cannot add a toplevel to Bazel's rule class provider",
    existing_rules_encode_to_json: "asserts on a value captured by the test-only bzl toplevel test.save (Java TestStarlarkBuiltin added to the rule class provider); the harness cannot add a toplevel to Bazel's rule class provider",
    existing_rule_round_trip_through_rule: "asserts on values captured by the test-only bzl toplevel test.save (Java TestStarlarkBuiltin added to the rule class provider); the harness cannot add a toplevel to Bazel's rule class provider",
}
