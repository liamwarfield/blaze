//! Port of `src/test/java/com/google/devtools/build/lib/analysis/RequiredConfigFragmentsTest.java`
//! (a `BuildViewTestCase`): `RequiredFragmentsUtil` and the `RequiredConfigFragmentsProvider` of
//! rules and aspects.
//!
//! Java's test class builds its own rule class provider (`createRuleClassProvider`): the standard
//! rules plus the test rules `requires_fragment_a`, `requires_fragment_b`, a rule attaching the
//! native aspect `AspectWithConfigFragmentRequirements`, and the fragments `TestFragmentA` (with
//! options `AOptions`) and `TestFragmentB`. The harness analyzes with Bazel's rule class provider
//! and cannot register them, so the tests that use them are ignored.
//!
//! Java runs the parameterized tests (`@TestParameter`) once per value; each value is one Rust
//! test here, named after the Java method with the value as a suffix.

use blaze_analysis::providers::required_config_fragments_provider::RequiredConfigFragmentsProvider;

use crate::testing::BuildViewTestCase;

fn required_fragments(t: &mut BuildViewTestCase, label: &str) -> RequiredConfigFragmentsProvider {
    t.get_configured_target(label)
        .unwrap_or_else(|| panic!("NullPointerException: no configured target {label}"))
        .get_provider::<RequiredConfigFragmentsProvider>()
        .unwrap_or_else(|| {
            panic!("NullPointerException: {label} has no RequiredConfigFragmentsProvider")
        })
        .clone()
}

fn aspect_required_fragments(
    t: &mut BuildViewTestCase,
    name: &str,
) -> RequiredConfigFragmentsProvider {
    t.get_aspect(name)
        .get_providers()
        .get_provider::<RequiredConfigFragmentsProvider>()
        .unwrap_or_else(|| {
            panic!("NullPointerException: {name} has no RequiredConfigFragmentsProvider")
        })
        .clone()
}

fn strings(set: &indexmap::IndexSet<String>) -> Vec<String> {
    set.iter().cloned().collect()
}

// Java: RequiredConfigFragmentsTest.provideTransitiveRequiredFragmentsMode
#[test]
#[ignore = "needs the test rules requires_fragment_a/_b and fragments TestFragmentA/B (createRuleClassProvider)"]
fn provide_transitive_required_fragments_mode() {}

// Java: RequiredConfigFragmentsTest.configSettingProvideTransitiveRequiresFragment
#[test]
#[ignore = "needs the test options class AOptions (createRuleClassProvider)"]
fn config_setting_provide_transitive_requires_fragment() {}

// Java: RequiredConfigFragmentsTest.provideDirectRequiredFragmentsMode
#[test]
#[ignore = "needs the test rules requires_fragment_a/_b and fragments TestFragmentA/B (createRuleClassProvider)"]
fn provide_direct_required_fragments_mode() {}

// Java: RequiredConfigFragmentsTest.configSettingProvideDirectRequiresFragment
#[test]
#[ignore = "needs the test options class AOptions (createRuleClassProvider)"]
fn config_setting_provide_direct_requires_fragment() {}

// Java: RequiredConfigFragmentsTest.requiresMakeVariablesSuppliedByDefine
#[test]
fn requires_make_variables_supplied_by_define() {
    let mut t = BuildViewTestCase::new();
    t.use_configuration(&[
        "--include_config_fragments_provider=direct",
        "--define",
        "myvar=myval",
    ]);
    t.scratch.file(
        "a/BUILD",
        &[r#"genrule(
    name = "myrule",
    srcs = [],
    outs = ["myrule.out"],
    cmd = "echo $(myvar) $(COMPILATION_MODE) > $@",
)
"#],
    );
    let required = required_fragments(&mut t, "//a:myrule");
    assert_eq!(strings(required.defines()), vec!["myvar".to_owned()]);
}

// Java: RequiredConfigFragmentsTest.starlarkExpandMakeVariables
#[test]
fn starlark_expand_make_variables() {
    let mut t = BuildViewTestCase::new();
    t.use_configuration(&[
        "--include_config_fragments_provider=direct",
        "--define=myvar=myval",
    ]);
    t.scratch.file(
        "a/defs.bzl",
        &[r#"def _impl(ctx):
    print(ctx.expand_make_variables("dummy attribute", "string with $(myvar)!", {}))

simple_rule = rule(
    implementation = _impl,
    attrs = {},
)
"#],
    );
    t.scratch.file(
        "a/BUILD",
        &[r#"load("//a:defs.bzl", "simple_rule")

simple_rule(name = "simple")
"#],
    );
    let required = required_fragments(&mut t, "//a:simple");
    assert_eq!(strings(required.defines()), vec!["myvar".to_owned()]);
}

// Java: RequiredConfigFragmentsTest.starlarkCtxVar
#[test]
fn starlark_ctx_var() {
    let mut t = BuildViewTestCase::new();
    t.use_configuration(&[
        "--include_config_fragments_provider=direct",
        "--define=required_var=1,irrelevant_var=1",
    ]);
    t.scratch.file(
        "a/defs.bzl",
        &[r#"def _impl(ctx):
    # Defined, so reported as required.
    if "required_var" not in ctx.var:
        fail("Missing required_var")

    # Not defined, so not reported as required.
    if "prohibited_var" in ctx.var:
        fail("Not allowed to set prohibited_var")

    # Present but not a define variable, so not reported as required.
    if "COMPILATION_MODE" not in ctx.var:
        fail("Missing COMPILATION_MODE")

simple_rule = rule(
    implementation = _impl,
    attrs = {},
)
"#],
    );
    t.scratch.file(
        "a/BUILD",
        &[r#"load("//a:defs.bzl", "simple_rule")

simple_rule(name = "simple")
"#],
    );
    let required = required_fragments(&mut t, "//a:simple");
    assert_eq!(strings(required.defines()), vec!["required_var".to_owned()]);
}

// Java: RequiredConfigFragmentsTest.addAspectImplSpecificRequiredConfigFragments
#[test]
#[ignore = "needs the native aspect AspectWithConfigFragmentRequirements and its rule (createRuleClassProvider)"]
fn add_aspect_impl_specific_required_config_fragments() {}

// Java: RequiredConfigFragmentsTest.aspectRequiresFragments
#[test]
#[ignore = "needs the native aspect AspectWithConfigFragmentRequirements and its rule (createRuleClassProvider)"]
fn aspect_requires_fragments() {}

// Java: RequiredConfigFragmentsTest.writeStarlarkTransitionsAndAllowList
fn write_starlark_transitions_and_allow_list(t: &BuildViewTestCase) {
    t.scratch.overwrite_file(
        "tools/allowlists/function_transition_allowlist/BUILD",
        &[r#"package_group(
    name = "function_transition_allowlist",
    packages = [
        "//a/...",
    ],
)
"#],
    );
    t.scratch.file(
        "transitions/defs.bzl",
        &[r#"def _java_write_transition_impl(settings, attr):
    return {"//command_line_option:javacopt": ["foo"]}

java_write_transition = transition(
    implementation = _java_write_transition_impl,
    inputs = [],
    outputs = ["//command_line_option:javacopt"],
)

def _cpp_read_transition_impl(settings, attr):
    return {}

cpp_read_transition = transition(
    implementation = _cpp_read_transition_impl,
    inputs = ["//command_line_option:copt"],
    outputs = [],
)
"#],
    );
    t.scratch.file("transitions/BUILD", &[]);
}

// Java: RequiredConfigFragmentsTest.starlarkRuleTransitionReadsFragment
#[test]
fn starlark_rule_transition_reads_fragment() {
    let mut t = BuildViewTestCase::new();
    write_starlark_transitions_and_allow_list(&t);
    t.scratch.file(
        "a/defs.bzl",
        &[r#"load("//transitions:defs.bzl", "cpp_read_transition")

def _impl(ctx):
    pass

has_cpp_aware_rule_transition = rule(
    implementation = _impl,
    cfg = cpp_read_transition,
)
"#],
    );
    t.scratch.file(
        "a/BUILD",
        &[r#"load("//a:defs.bzl", "has_cpp_aware_rule_transition")

has_cpp_aware_rule_transition(name = "cctarget")
"#],
    );
    t.use_configuration(&["--include_config_fragments_provider=direct"]);
    let required = required_fragments(&mut t, "//a:cctarget");
    assert!(required.options_classes().contains("CppOptions"));
    assert!(!required.options_classes().contains("JavaOptions"));
}

// Java: RequiredConfigFragmentsTest.starlarkRuleTransitionWritesFragment
#[test]
fn starlark_rule_transition_writes_fragment() {
    let mut t = BuildViewTestCase::new();
    write_starlark_transitions_and_allow_list(&t);
    t.scratch.file(
        "a/defs.bzl",
        &[r#"load("//transitions:defs.bzl", "java_write_transition")

def _impl(ctx):
    pass

has_java_aware_rule_transition = rule(
    implementation = _impl,
    cfg = java_write_transition,
)
"#],
    );
    t.scratch.file(
        "a/BUILD",
        &[r#"load("//a:defs.bzl", "has_java_aware_rule_transition")

has_java_aware_rule_transition(name = "javatarget")
"#],
    );
    t.use_configuration(&["--include_config_fragments_provider=direct"]);
    let required = required_fragments(&mut t, "//a:javatarget");
    assert!(required.options_classes().contains("JavaOptions"));
    assert!(!required.options_classes().contains("CppOptions"));
}

// Java: RequiredConfigFragmentsTest.starlarkAttrTransition
#[test]
fn starlark_attr_transition() {
    let mut t = BuildViewTestCase::new();
    write_starlark_transitions_and_allow_list(&t);
    t.scratch.file(
        "a/defs.bzl",
        &[
            r#"load("//transitions:defs.bzl", "cpp_read_transition", "java_write_transition")

def _impl(ctx):
    pass

has_java_aware_attr_transition = rule(
    implementation = _impl,
    attrs = {
        "deps": attr.label_list(cfg = java_write_transition),
    },
)
has_cpp_aware_rule_transition = rule(
    implementation = _impl,
    cfg = cpp_read_transition,
)
"#,
        ],
    );
    t.scratch.file(
        "a/BUILD",
        &[r#"load("//a:defs.bzl", "has_cpp_aware_rule_transition", "has_java_aware_attr_transition")

has_cpp_aware_rule_transition(name = "ccchild")

has_java_aware_attr_transition(
    name = "javaparent",
    deps = [":ccchild"],
)
"#],
    );
    t.use_configuration(&["--include_config_fragments_provider=direct"]);
    let required = required_fragments(&mut t, "//a:javaparent");
    // We consider the attribute transition over the parent -> child edge a property of the
    // parent.
    assert!(required.options_classes().contains("JavaOptions"));
    // But not the child's rule transition.
    assert!(!required.options_classes().contains("CppOptions"));
}

// Java: RequiredConfigFragmentsTest.aspectInheritsTransitiveFragmentsFromBaseCT
fn aspect_inherits_transitive_fragments_from_base_ct(setting: &str) {
    let mut t = BuildViewTestCase::new();
    write_starlark_transitions_and_allow_list(&t);
    t.scratch.file(
        "a/defs.bzl",
        &[r#"A1Info = provider()

def _a1_impl(target, ctx):
    return []

a1 = aspect(implementation = _a1_impl)

def _java_depender_impl(ctx):
    return []

java_depender = rule(
    implementation = _java_depender_impl,
    fragments = ["java"],
    attrs = {},
)

def _r_impl(ctx):
    return []

r = rule(
    implementation = _r_impl,
    attrs = {"dep": attr.label(aspects = [a1])},
)
"#],
    );
    t.scratch.file(
        "a/BUILD",
        &[r#"load(":defs.bzl", "java_depender", "r")

java_depender(name = "lib")

r(
    name = "r",
    dep = ":lib",
)
"#],
    );
    t.use_configuration(&[&format!("--include_config_fragments_provider={setting}")]);
    t.get_configured_target("//a:r");
    let required = aspect_required_fragments(&mut t, "//a:defs.bzl%a1");
    if setting == "TRANSITIVE" {
        assert!(required.fragment_classes().contains("JavaConfiguration"));
    } else {
        assert!(!required.fragment_classes().contains("JavaConfiguration"));
    }
}

#[test]
fn aspect_inherits_transitive_fragments_from_base_ct_direct() {
    aspect_inherits_transitive_fragments_from_base_ct("DIRECT");
}

#[test]
fn aspect_inherits_transitive_fragments_from_base_ct_transitive() {
    aspect_inherits_transitive_fragments_from_base_ct("TRANSITIVE");
}

// Java: RequiredConfigFragmentsTest.aspectInheritsTransitiveFragmentsFromRequiredAspect
fn aspect_inherits_transitive_fragments_from_required_aspect(setting: &str) {
    let mut t = BuildViewTestCase::new();
    t.scratch.file(
        "a/defs.bzl",
        &[r#"A1Info = provider()

def _a1_impl(target, ctx):
    return A1Info(var = ctx.var.get("my_var", "0"))

a1 = aspect(implementation = _a1_impl, provides = [A1Info])

A2Info = provider()

def _a2_impl(target, ctx):
    return A2Info()

a2 = aspect(implementation = _a2_impl, required_aspect_providers = [A1Info])

def _simple_rule_impl(ctx):
    return []

simple_rule = rule(
    implementation = _simple_rule_impl,
    attrs = {},
)

def _r_impl(ctx):
    return []

r = rule(
    implementation = _r_impl,
    attrs = {"dep": attr.label(aspects = [a1, a2])},
)
"#],
    );
    t.scratch.file(
        "a/BUILD",
        &[r#"load(":defs.bzl", "r", "simple_rule")

simple_rule(name = "lib")

r(
    name = "r",
    dep = ":lib",
)
"#],
    );
    t.use_configuration(&[
        &format!("--include_config_fragments_provider={setting}"),
        "--define",
        "my_var=1",
    ]);
    t.get_configured_target("//a:r");
    let required = aspect_required_fragments(&mut t, "//a:defs.bzl%a2");
    if setting == "TRANSITIVE" {
        assert!(required.defines().contains("my_var"));
    } else {
        assert!(!required.defines().contains("my_var"));
    }
}

#[test]
fn aspect_inherits_transitive_fragments_from_required_aspect_direct() {
    aspect_inherits_transitive_fragments_from_required_aspect("DIRECT");
}

#[test]
fn aspect_inherits_transitive_fragments_from_required_aspect_transitive() {
    aspect_inherits_transitive_fragments_from_required_aspect("TRANSITIVE");
}

// Java: RequiredConfigFragmentsTest.invalidStarlarkFragmentsFiltered
#[test]
fn invalid_starlark_fragments_filtered() {
    let mut t = BuildViewTestCase::new();
    t.scratch.file(
        "a/defs.bzl",
        &[r#"def _my_rule_impl(ctx):
    pass

my_rule = rule(implementation = _my_rule_impl, fragments = ["java", "doesnotexist"])
"#],
    );
    t.scratch.file(
        "a/BUILD",
        &[r#"load(":defs.bzl", "my_rule")

my_rule(name = "example")
"#],
    );
    t.use_configuration(&["--include_config_fragments_provider=direct"]);
    let required = required_fragments(&mut t, "//a:example");
    assert!(required.fragment_classes().contains("JavaConfiguration"));
}

// Java: RequiredConfigFragmentsTest.aspectInErrorWithAllowAnalysisFailures
#[test]
fn aspect_in_error_with_allow_analysis_failures() {
    let mut t = BuildViewTestCase::new();
    t.scratch.file(
        "a/defs.bzl",
        &[r#"def _error_aspect_impl(target, ctx):
    fail(ctx.var["FAIL_MESSAGE"])

error_aspect = aspect(implementation = _error_aspect_impl)

def _my_rule_impl(ctx):
    pass

my_rule = rule(
    implementation = _my_rule_impl,
    attrs = {"dep": attr.label(aspects = [error_aspect])},
)
"#],
    );
    t.scratch.file(
        "a/BUILD",
        &[r#"load(":defs.bzl", "error_aspect", "my_rule")

my_rule(name = "a")

my_rule(
    name = "b",
    dep = ":a",
)
"#],
    );
    t.use_configuration(&[
        "--allow_analysis_failures",
        "--define=FAIL_MESSAGE=abc",
        "--include_config_fragments_provider=direct",
    ]);
    t.get_configured_target("//a:b");
    let required = aspect_required_fragments(&mut t, "//a:defs.bzl%error_aspect");
    assert_eq!(strings(required.defines()), vec!["FAIL_MESSAGE".to_owned()]);
}

// Java: RequiredConfigFragmentsTest.configuredTargetInErrorWithAllowAnalysisFailures
#[test]
fn configured_target_in_error_with_allow_analysis_failures() {
    let mut t = BuildViewTestCase::new();
    t.scratch.file(
        "a/defs.bzl",
        &[r#"def _error_rule_impl(ctx):
    fail(ctx.var["FAIL_MESSAGE"])

error_rule = rule(implementation = _error_rule_impl)
"#],
    );
    t.scratch.file(
        "a/BUILD",
        &[r#"load(":defs.bzl", "error_rule")

error_rule(name = "error")
"#],
    );
    t.use_configuration(&[
        "--allow_analysis_failures",
        "--define=FAIL_MESSAGE=abc",
        "--include_config_fragments_provider=direct",
    ]);
    let required = required_fragments(&mut t, "//a:error");
    assert_eq!(strings(required.defines()), vec!["FAIL_MESSAGE".to_owned()]);
}

// Java: RequiredConfigFragmentsTest.aliasWithSelectResolvesToConfigSetting
#[test]
fn alias_with_select_resolves_to_config_setting() {
    let mut t = BuildViewTestCase::new();
    t.scratch.file(
        "a/BUILD",
        &[r#"config_setting(
    name = "define_x",
    define_values = {"x": "1"},
)

config_setting(
    name = "k8",
    constraint_values = ["@platforms//cpu:x86_64"]
)

alias(
    name = "alias_to_setting",
    actual = select({":define_x": ":k8"}),
)

genrule(
    name = "gen",
    outs = ["gen.out"],
    cmd = select({":alias_to_setting": "touch $@"}),
)
"#],
    );
    t.use_configuration(&[
        "--define=x=1",
        &format!(
            "--platforms={}",
            crate::testing::analysis_mock::PLATFORM_LABEL
        ),
        "--include_config_fragments_provider=transitive",
    ]);
    let required = required_fragments(&mut t, "//a:gen");
    assert_eq!(strings(required.defines()), vec!["x".to_owned()]);
}
