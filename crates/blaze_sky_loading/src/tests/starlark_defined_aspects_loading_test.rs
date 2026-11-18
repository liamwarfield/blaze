//! Ports of the aspect-parameter tests of StarlarkDefinedAspectsTest whose asserted error is
//! reported while the package (or one of its .bzl files) is loaded.
//!
//! Java: src/test/java/com/google/devtools/build/lib/starlark/StarlarkDefinedAspectsTest.java.
//!
//! Each Java test calls `update(...)` on a target, expects the build to fail
//! (`result.hasError()` with --keep_going, an exception without) and asserts on an event. The
//! events asserted here come from evaluating `aspect()` / `rule()` in the .bzl file or from
//! instantiating the rule in the BUILD file, so the port is: PACKAGE for the target's package
//! throws or yields a package in error (`PackageLoadingTestCase::load_failure_events`), and the
//! event is present. Nothing of analysis is exercised. The Starlark sources and the expected texts
//! are Java's; `/workspace` is the scratch workspace (`workspace_path`).
//!
//! The rest of StarlarkDefinedAspectsTest needs analysis (`update(...)` results, providers,
//! actions) and is not ported here.

use crate::tests::package_loading_test_case::{PackageLoadingTestCase, assert_contains_event};

// Java: StarlarkDefinedAspectsTest.aspectParametersUncovered
#[test]
fn aspect_parameters_uncovered() {
    let t = PackageLoadingTestCase::new();
    t.scratch_file(
        "test/aspect.bzl",
        &[
            "def _impl(target, ctx):",
            "   return []",
            "def _rule_impl(ctx):",
            "   return []",
            "MyAspectUncovered = aspect(",
            "    implementation=_impl,",
            "    attrs = { 'my_attr' : attr.string(values=['aaa']) },",
            ")",
            "my_rule = rule(",
            "    implementation=_rule_impl,",
            "    attrs = { 'deps' : attr.label_list(aspects=[MyAspectUncovered]) },",
            ")",
        ],
    );
    t.scratch_file(
        "test/BUILD",
        &[
            "load('//test:aspect.bzl', 'my_rule')",
            "my_rule(name = 'xxx')",
        ],
    );
    let events = t.load_failure_events("test");
    assert_contains_event(
        &events,
        "Aspect //test:aspect.bzl%MyAspectUncovered requires rule my_rule to specify attribute 'my_attr' with type string.",
    );
}

// Java: StarlarkDefinedAspectsTest.aspectParametersTypeMismatch
#[test]
fn aspect_parameters_type_mismatch() {
    let t = PackageLoadingTestCase::new();
    t.scratch_file(
        "test/aspect.bzl",
        &[
            "def _impl(target, ctx):",
            "   return []",
            "def _rule_impl(ctx):",
            "   return []",
            "MyAspectMismatch = aspect(",
            "    implementation=_impl,",
            "    attrs = { 'my_attr' : attr.string(values=['aaa']) },",
            ")",
            "my_rule = rule(",
            "    implementation=_rule_impl,",
            "    attrs = { 'deps' : attr.label_list(aspects=[MyAspectMismatch]),",
            "              'my_attr' : attr.int() },",
            ")",
        ],
    );
    t.scratch_file(
        "test/BUILD",
        &[
            "load('//test:aspect.bzl', 'my_rule')",
            "my_rule(name = 'xxx', my_attr = 4)",
        ],
    );
    let events = t.load_failure_events("test");
    assert_contains_event(
        &events,
        "Aspect //test:aspect.bzl%MyAspectMismatch requires rule my_rule to specify attribute 'my_attr' with type string.",
    );
}

// Java: StarlarkDefinedAspectsTest.aspectParametersDontSupportSelect
#[test]
fn aspect_parameters_dont_support_select() {
    let t = PackageLoadingTestCase::new();
    t.scratch_file(
        "test/aspect.bzl",
        &[
            "def _impl(target, ctx):",
            "   return []",
            "def _rule_impl(ctx):",
            "   return []",
            "MyAspectMismatch = aspect(",
            "    implementation=_impl,",
            "    attrs = { 'my_attr' : attr.string(values=['aaa']) },",
            ")",
            "my_rule = rule(",
            "    implementation=_rule_impl,",
            "    attrs = { 'deps' : attr.label_list(aspects=[MyAspectMismatch]),",
            "              'my_attr' : attr.string() },",
            ")",
        ],
    );
    t.scratch_file(
        "test/BUILD",
        &[
            "load(\"//test:aspect.bzl\", \"my_rule\")",
            "",
            "# Needed to avoid the select() being eliminated as trivial.",
            "config_setting(",
            "    name = \"config\",",
            "    values = {\"defines\": \"something\"},",
            ")",
            "",
            "my_rule(",
            "    name = \"xxx\",",
            "    my_attr = select({",
            "        \":config\": \"foo\",",
            "        \"//conditions:default\": \"bar\",",
            "    }),",
            ")",
        ],
    );
    let events = t.load_failure_events("test");
    assert_contains_event(
        &events,
        "//test:xxx: attribute 'my_attr' has a select() and aspect //test:aspect.bzl%MyAspectMismatch also declares '//test:xxx'. Aspect attributes don't currently support select().",
    );
}

// Java: StarlarkDefinedAspectsTest.aspectParametersBadDefault
#[test]
fn aspect_parameters_bad_default() {
    let t = PackageLoadingTestCase::new();
    t.scratch_file(
        "test/aspect.bzl",
        &[
            "def _impl(target, ctx):",
            "   return []",
            "def _rule_impl(ctx):",
            "   return []",
            "MyAspectBadDefault = aspect(",
            "    implementation=_impl,",
            "    attrs = { 'my_attr' : attr.string(values=['a'], default='b') },",
            ")",
            "my_rule = rule(",
            "    implementation=_rule_impl,",
            "    attrs = { 'deps' : attr.label_list(aspects=[MyAspectBadDefault]) },  # line 11",
            ")",
        ],
    );
    t.scratch_file(
        "test/BUILD",
        &[
            "load('//test:aspect.bzl', 'my_rule')",
            "my_rule(name = 'xxx')",
        ],
    );
    let events = t.load_failure_events("test");
    // aspect fails, stack = [<toplevel>@:5:28, aspect@<builtin>]
    assert_contains_event(
        &events,
        &format!(
            "File \"{}\", line 5, column 28, in <toplevel>",
            t.workspace_path("test/aspect.bzl")
        ),
    );
    assert_contains_event(
        &events,
        "Error in aspect: Aspect parameter attribute 'my_attr' has a bad default value: has to be one of 'a' instead of 'b'",
    );
}

// Java: StarlarkDefinedAspectsTest.aspectParametersBadValue
#[test]
fn aspect_parameters_bad_value() {
    let t = PackageLoadingTestCase::new();
    t.scratch_file(
        "test/aspect.bzl",
        &[
            "def _impl(target, ctx):",
            "   return []",
            "def _rule_impl(ctx):",
            "   return []",
            "MyAspectBadValue = aspect(",
            "    implementation=_impl,",
            "    attrs = { 'my_attr' : attr.string(values=['a']) },",
            ")",
            "my_rule = rule(",
            "    implementation=_rule_impl,",
            "    attrs = { 'deps' : attr.label_list(aspects=[MyAspectBadValue]),",
            "              'my_attr' : attr.string() },",
            ")",
        ],
    );
    t.scratch_file(
        "test/BUILD",
        &[
            "load('//test:aspect.bzl', 'my_rule')",
            "my_rule(name = 'xxx', my_attr='b')",
        ],
    );
    let events = t.load_failure_events("test");
    assert_contains_event(
        &events,
        &format!(
            "ERROR {}:2:8: //test:xxx: invalid value in 'my_attr' attribute: has to be one of 'a' \
             instead of 'b'",
            t.workspace_path("test/BUILD")
        ),
    );
}

// Java: StarlarkDefinedAspectsTest.aspectParameterComputedDefault
#[test]
fn aspect_parameter_computed_default() {
    let t = PackageLoadingTestCase::new();
    t.scratch_file(
        "test/aspect.bzl",
        &[
            "def _impl(target, ctx):",
            "   return []",
            "def _rule_impl(ctx):",
            "   return []",
            "def _defattr():",
            "   return Label('//foo/bar:baz')",
            "MyAspect = aspect(",
            "    implementation=_impl,",
            "    attrs = { '_extra' : attr.label(default = _defattr) }",
            ")",
            "my_rule = rule(",
            "    implementation=_rule_impl,",
            "    attrs = { 'deps' : attr.label_list(aspects=[MyAspect]) },",
            ")",
        ],
    );
    t.scratch_file(
        "test/BUILD",
        &[
            "load('//test:aspect.bzl', 'my_rule')",
            "my_rule(name = 'xxx')",
        ],
    );
    let events = t.load_failure_events("test");
    assert_contains_event(
        &events,
        "Aspect attribute '_extra' (label) with computed default value is unsupported.",
    );
}

// Java: StarlarkDefinedAspectsTest.testAttrAspectParameterMissingRequiredValues
#[test]
fn test_attr_aspect_parameter_missing_required_values() {
    let t = PackageLoadingTestCase::new();
    t.scratch_file(
        "test/defs.bzl",
        &[
            "def _impl(target, ctx):",
            "   pass",
            "my_aspect = aspect(_impl,",
            "   attrs = { 'param' : attr.string(default = 'c') }",
            ")",
            "def _rule_impl(ctx):",
            "   pass",
            "r1 = rule(_rule_impl, attrs={'dep': attr.label(aspects = [my_aspect])})",
        ],
    );
    t.scratch_file(
        "test/BUILD",
        &["load('//test:defs.bzl', 'r1')", "r1(name = 'main_target')"],
    );
    let events = t.load_failure_events("test");
    assert_contains_event(
        &events,
        "Aspect //test:defs.bzl%my_aspect: Aspect parameter attribute 'param' must use the 'values' restriction.",
    );
}

// Java: StarlarkDefinedAspectsTest.testAttrRequiredAspectParameterMissingRequiredValues
#[test]
fn test_attr_required_aspect_parameter_missing_required_values() {
    let t = PackageLoadingTestCase::new();
    t.scratch_file(
        "test/defs.bzl",
        &[
            "def _impl(target, ctx):",
            "   pass",
            "required_aspect = aspect(_impl,",
            "   attrs = { 'p1' : attr.string(default = 'b') }",
            ")",
            "my_aspect = aspect(_impl,",
            "   attrs = { 'p2' : attr.string(default = 'c', values = ['c']) },",
            "   requires = [required_aspect],",
            ")",
            "def _rule_impl(ctx):",
            "   pass",
            "r1 = rule(_rule_impl, attrs={'dep': attr.label(aspects = [my_aspect])})",
        ],
    );
    t.scratch_file(
        "test/BUILD",
        &["load('//test:defs.bzl', 'r1')", "r1(name = 'main_target')"],
    );
    let events = t.load_failure_events("test");
    assert_contains_event(
        &events,
        "Aspect //test:defs.bzl%required_aspect: Aspect parameter attribute 'p1' must use the 'values' restriction.",
    );
}

// Java: StarlarkDefinedAspectsTest.integerAspectParameter_mandatoryAttrNotCoveredByRule
#[test]
fn integer_aspect_parameter_mandatory_attr_not_covered_by_rule() {
    let t = PackageLoadingTestCase::new();
    t.scratch_file(
        "test/aspect.bzl",
        &[
            "def _impl(target, ctx):",
            "   return []",
            "def _rule_impl(ctx):",
            "   return []",
            "MyAspectUncovered = aspect(",
            "    implementation=_impl,",
            "    attrs = { 'my_attr' : attr.int(default = 1, values = [1, 2], mandatory = True) },",
            ")",
            "my_rule = rule(",
            "    implementation=_rule_impl,",
            "    attrs = { 'deps' : attr.label_list(aspects=[MyAspectUncovered]) },",
            ")",
        ],
    );
    t.scratch_file(
        "test/BUILD",
        &[
            "load('//test:aspect.bzl', 'my_rule')",
            "my_rule(name ='main')",
        ],
    );
    let events = t.load_failure_events("test");
    assert_contains_event(
        &events,
        "Aspect //test:aspect.bzl%MyAspectUncovered requires rule my_rule to specify attribute 'my_attr' with type int.",
    );
}

// Java: StarlarkDefinedAspectsTest.integerAspectParameter_mandatoryAttrWithWrongTypeInRule
#[test]
fn integer_aspect_parameter_mandatory_attr_with_wrong_type_in_rule() {
    let t = PackageLoadingTestCase::new();
    t.scratch_file(
        "test/aspect.bzl",
        &[
            "def _impl(target, ctx):",
            "   return []",
            "def _rule_impl(ctx):",
            "   return []",
            "MyAspectUncovered = aspect(",
            "    implementation=_impl,",
            "    attrs = { 'my_attr' : attr.int(default = 1, values = [1, 2], mandatory = True) },",
            ")",
            "my_rule = rule(",
            "    implementation=_rule_impl,",
            "    attrs = { 'deps' : attr.label_list(aspects=[MyAspectUncovered]),",
            "              'my_attr': attr.string() },",
            ")",
        ],
    );
    t.scratch_file(
        "test/BUILD",
        &[
            "load('//test:aspect.bzl', 'my_rule')",
            "my_rule(name ='main')",
        ],
    );
    let events = t.load_failure_events("test");
    assert_contains_event(
        &events,
        "Aspect //test:aspect.bzl%MyAspectUncovered requires rule my_rule to specify attribute 'my_attr' with type int.",
    );
}

// Java: StarlarkDefinedAspectsTest.integerAspectParameter_attrWithoutDefaultNotCoveredByRule
#[test]
fn integer_aspect_parameter_attr_without_default_not_covered_by_rule() {
    let t = PackageLoadingTestCase::new();
    t.scratch_file(
        "test/aspect.bzl",
        &[
            "def _impl(target, ctx):",
            "   return []",
            "def _rule_impl(ctx):",
            "   return []",
            "MyAspectUncovered = aspect(",
            "    implementation=_impl,",
            "    attrs = { 'my_attr' : attr.int(values = [1, 2]) },",
            ")",
            "my_rule = rule(",
            "    implementation=_rule_impl,",
            "    attrs = { 'deps' : attr.label_list(aspects=[MyAspectUncovered]) },",
            ")",
        ],
    );
    t.scratch_file(
        "test/BUILD",
        &[
            "load('//test:aspect.bzl', 'my_rule')",
            "my_rule(name ='main')",
        ],
    );
    let events = t.load_failure_events("test");
    assert_contains_event(
        &events,
        "Aspect //test:aspect.bzl%MyAspectUncovered requires rule my_rule to specify attribute 'my_attr' with type int.",
    );
}

// Java: StarlarkDefinedAspectsTest.integerAspectParameter_attrWithoutDefaultWrongTypeInRule
#[test]
fn integer_aspect_parameter_attr_without_default_wrong_type_in_rule() {
    let t = PackageLoadingTestCase::new();
    t.scratch_file(
        "test/aspect.bzl",
        &[
            "def _impl(target, ctx):",
            "   return []",
            "def _rule_impl(ctx):",
            "   return []",
            "MyAspectUncovered = aspect(",
            "    implementation=_impl,",
            "    attrs = { 'my_attr' : attr.int(values = [1, 2]) },",
            ")",
            "my_rule = rule(",
            "    implementation=_rule_impl,",
            "    attrs = { 'deps' : attr.label_list(aspects=[MyAspectUncovered]),",
            "              'my_attr': attr.string() },",
            ")",
        ],
    );
    t.scratch_file(
        "test/BUILD",
        &[
            "load('//test:aspect.bzl', 'my_rule')",
            "my_rule(name ='main')",
        ],
    );
    let events = t.load_failure_events("test");
    assert_contains_event(
        &events,
        "Aspect //test:aspect.bzl%MyAspectUncovered requires rule my_rule to specify attribute 'my_attr' with type int.",
    );
}

// Java: StarlarkDefinedAspectsTest.integerAspectParameter_missingValuesRestriction
#[test]
fn integer_aspect_parameter_missing_values_restriction() {
    let t = PackageLoadingTestCase::new();
    t.scratch_file(
        "test/aspect.bzl",
        &[
            "def _impl(target, ctx):",
            "   return []",
            "def _rule_impl(ctx):",
            "   return []",
            "MyAspectUncovered = aspect(",
            "    implementation=_impl,",
            "    attrs = { 'my_attr' : attr.int() },",
            ")",
            "my_rule = rule(",
            "    implementation=_rule_impl,",
            "    attrs = { 'deps' : attr.label_list(aspects=[MyAspectUncovered]),",
            "              'my_attr' : attr.int() },",
            ")",
        ],
    );
    t.scratch_file(
        "test/BUILD",
        &[
            "load('//test:aspect.bzl', 'my_rule')",
            "my_rule(name ='main')",
        ],
    );
    let events = t.load_failure_events("test");
    assert_contains_event(
        &events,
        "Aspect //test:aspect.bzl%MyAspectUncovered: Aspect parameter attribute 'my_attr' must use the 'values' restriction.",
    );
}

// Java: StarlarkDefinedAspectsTest.integerAspectParameter_invalidDefault
#[test]
fn integer_aspect_parameter_invalid_default() {
    let t = PackageLoadingTestCase::new();
    t.scratch_file(
        "test/aspect.bzl",
        &[
            "def _impl(target, ctx):",
            "   return []",
            "def _rule_impl(ctx):",
            "   return []",
            "MyAspectUncovered = aspect(",
            "    implementation=_impl,",
            "    attrs = { 'my_attr' : attr.int(default = 2, values = [0, 1]) },",
            ")",
            "my_rule = rule(",
            "    implementation=_rule_impl,",
            "    attrs = { 'deps' : attr.label_list(aspects=[MyAspectUncovered]) },",
            ")",
        ],
    );
    t.scratch_file(
        "test/BUILD",
        &[
            "load('//test:aspect.bzl', 'my_rule')",
            "my_rule(name ='main')",
        ],
    );
    let events = t.load_failure_events("test");
    assert_contains_event(
        &events,
        "Aspect parameter attribute 'my_attr' has a bad default value: has to be one of '0' or '1' instead of '2'",
    );
}

// Java: StarlarkDefinedAspectsTest.booleanAspectParameter_mandatoryAttrNotCoveredByRule
#[test]
fn boolean_aspect_parameter_mandatory_attr_not_covered_by_rule() {
    let t = PackageLoadingTestCase::new();
    t.scratch_file(
        "test/aspect.bzl",
        &[
            "def _impl(target, ctx):",
            "   return []",
            "def _rule_impl(ctx):",
            "   return []",
            "MyAspectUncovered = aspect(",
            "    implementation=_impl,",
            "    attrs = { 'my_attr' : attr.bool(default = True, mandatory = True) },",
            ")",
            "my_rule = rule(",
            "    implementation=_rule_impl,",
            "    attrs = { 'deps' : attr.label_list(aspects=[MyAspectUncovered]) },",
            ")",
        ],
    );
    t.scratch_file(
        "test/BUILD",
        &[
            "load('//test:aspect.bzl', 'my_rule')",
            "my_rule(name ='main')",
        ],
    );
    let events = t.load_failure_events("test");
    assert_contains_event(
        &events,
        "Aspect //test:aspect.bzl%MyAspectUncovered requires rule my_rule to specify attribute 'my_attr' with type boolean.",
    );
}

// Java: StarlarkDefinedAspectsTest.booleanAspectParameter_mandatoryAttrWithWrongTypeInRule
#[test]
fn boolean_aspect_parameter_mandatory_attr_with_wrong_type_in_rule() {
    let t = PackageLoadingTestCase::new();
    t.scratch_file(
        "test/aspect.bzl",
        &[
            "def _impl(target, ctx):",
            "   return []",
            "def _rule_impl(ctx):",
            "   return []",
            "MyAspectUncovered = aspect(",
            "    implementation=_impl,",
            "    attrs = { 'my_attr' : attr.bool(default = True, mandatory = True) },",
            ")",
            "my_rule = rule(",
            "    implementation=_rule_impl,",
            "    attrs = { 'deps' : attr.label_list(aspects=[MyAspectUncovered]),",
            "              'my_attr': attr.string() },",
            ")",
        ],
    );
    t.scratch_file(
        "test/BUILD",
        &[
            "load('//test:aspect.bzl', 'my_rule')",
            "my_rule(name ='main')",
        ],
    );
    let events = t.load_failure_events("test");
    assert_contains_event(
        &events,
        "Aspect //test:aspect.bzl%MyAspectUncovered requires rule my_rule to specify attribute 'my_attr' with type boolean.",
    );
}

// Java: StarlarkDefinedAspectsTest.booleanAspectParameter_attrWithoutDefaultNotCoveredByRule
#[test]
fn boolean_aspect_parameter_attr_without_default_not_covered_by_rule() {
    let t = PackageLoadingTestCase::new();
    t.scratch_file(
        "test/aspect.bzl",
        &[
            "def _impl(target, ctx):",
            "   return []",
            "def _rule_impl(ctx):",
            "   return []",
            "MyAspectUncovered = aspect(",
            "    implementation=_impl,",
            "    attrs = { 'my_attr' : attr.bool() },",
            ")",
            "my_rule = rule(",
            "    implementation=_rule_impl,",
            "    attrs = { 'deps' : attr.label_list(aspects=[MyAspectUncovered]) },",
            ")",
        ],
    );
    t.scratch_file(
        "test/BUILD",
        &[
            "load('//test:aspect.bzl', 'my_rule')",
            "my_rule(name ='main')",
        ],
    );
    let events = t.load_failure_events("test");
    assert_contains_event(
        &events,
        "Aspect //test:aspect.bzl%MyAspectUncovered requires rule my_rule to specify attribute 'my_attr' with type boolean.",
    );
}

// Java: StarlarkDefinedAspectsTest.booleanAspectParameter_attrWithoutDefaultWrongTypeInRule
#[test]
fn boolean_aspect_parameter_attr_without_default_wrong_type_in_rule() {
    let t = PackageLoadingTestCase::new();
    t.scratch_file(
        "test/aspect.bzl",
        &[
            "def _impl(target, ctx):",
            "   return []",
            "def _rule_impl(ctx):",
            "   return []",
            "MyAspectUncovered = aspect(",
            "    implementation=_impl,",
            "    attrs = { 'my_attr' : attr.bool() },",
            ")",
            "my_rule = rule(",
            "    implementation=_rule_impl,",
            "    attrs = { 'deps' : attr.label_list(aspects=[MyAspectUncovered]),",
            "              'my_attr': attr.string() },",
            ")",
        ],
    );
    t.scratch_file(
        "test/BUILD",
        &[
            "load('//test:aspect.bzl', 'my_rule')",
            "my_rule(name ='main')",
        ],
    );
    let events = t.load_failure_events("test");
    assert_contains_event(
        &events,
        "Aspect //test:aspect.bzl%MyAspectUncovered requires rule my_rule to specify attribute 'my_attr' with type boolean.",
    );
}

// Java: StarlarkDefinedAspectsTest.testRuleAspectWithMandatoryParameterNotProvided
#[test]
fn test_rule_aspect_with_mandatory_parameter_not_provided() {
    let t = PackageLoadingTestCase::new();
    t.scratch_file(
        "test/defs.bzl",
        &[
            "def _aspect_a_impl(target, ctx):",
            "  result = ['aspect_a on target {}, p = {}'.",
            "                                    format(target.label, ctx.attr.p)]",
            "  if ctx.rule.attr.dep:",
            "    result += ctx.rule.attr.dep.aspect_a_result",
            "  return struct(aspect_a_result = result)",
            "",
            "aspect_a = aspect(",
            "  implementation = _aspect_a_impl,",
            "  attr_aspects = ['dep'],",
            "  attrs = { 'p' : attr.string(default = 'p_v', values = ['p_v'], mandatory = True) },",
            ")",
            "",
            "def _my_rule_impl(ctx):",
            "  pass",
            "my_rule = rule(",
            "  implementation = _my_rule_impl,",
            "   attrs = { 'dep' : attr.label(aspects = [aspect_a]) },",
            ")",
        ],
    );
    t.scratch_file(
        "test/BUILD",
        &[
            "load('//test:defs.bzl', 'my_rule')",
            "my_rule(",
            "  name = 'main_target',",
            "  dep = ':dep_target_1',",
            ")",
            "my_rule(",
            "  name = 'dep_target_1',",
            "  dep = ':dep_target_2',",
            ")",
            "my_rule(",
            "  name = 'dep_target_2',",
            ")",
        ],
    );
    let events = t.load_failure_events("test");
    assert_contains_event(
        &events,
        "Aspect //test:defs.bzl%aspect_a requires rule my_rule to specify attribute 'p' with type string",
    );
}

// Java: StarlarkDefinedAspectsTest.testRuleAspectWithMandatoryParameterProvidedWrongType
#[test]
fn test_rule_aspect_with_mandatory_parameter_provided_wrong_type() {
    let t = PackageLoadingTestCase::new();
    t.scratch_file(
        "test/defs.bzl",
        &[
            "def _aspect_a_impl(target, ctx):",
            "  result = ['aspect_a on target {}, p = {}'.",
            "                                    format(target.label, ctx.attr.p)]",
            "  if ctx.rule.attr.dep:",
            "    result += ctx.rule.attr.dep.aspect_a_result",
            "  return struct(aspect_a_result = result)",
            "",
            "aspect_a = aspect(",
            "  implementation = _aspect_a_impl,",
            "  attr_aspects = ['dep'],",
            "  attrs = { 'p' : attr.string(default = 'p_v', values = ['p_v'], mandatory = True) },",
            ")",
            "",
            "def _my_rule_impl(ctx):",
            "  pass",
            "my_rule = rule(",
            "  implementation = _my_rule_impl,",
            "   attrs = { 'dep' : attr.label(aspects = [aspect_a]),",
            "             'p': attr.int() }",
            ")",
        ],
    );
    t.scratch_file(
        "test/BUILD",
        &[
            "load('//test:defs.bzl', 'my_rule')",
            "my_rule(",
            "  name = 'main_target',",
            "  dep = ':dep_target_1',",
            ")",
            "my_rule(",
            "  name = 'dep_target_1',",
            "  dep = ':dep_target_2',",
            ")",
            "my_rule(",
            "  name = 'dep_target_2',",
            ")",
        ],
    );
    let events = t.load_failure_events("test");
    assert_contains_event(
        &events,
        "Aspect //test:defs.bzl%aspect_a requires rule my_rule to specify attribute 'p' with type string",
    );
}
