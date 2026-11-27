//! Port of the analysis tests of
//! `src/test/java/com/google/devtools/build/lib/starlark/StarlarkRuleClassFunctionsTest.java` (a
//! `BuildViewTestCase`): the tests that analyze configured targets. The tests that only evaluate
//! .bzl code or load packages are in `blaze_packages` and `blaze_sky_loading` (see
//! `crates/blaze_packages/src/starlark_defs/tests_ignored.rs`).
//!
//! The test bodies were transcribed from the Java statement by statement (listed in
//! `docs/analysis-harness.md`).

use blaze_build_config::build_options::StarlarkOptionValue;
use blaze_cmdline::Label;
use blaze_events::{EventKind, EventKindSet};
use blaze_packages::rule_class::ProviderKey;
use blaze_packages::rule_class::rule_class::RuleClassType;

use crate::testing::{
    BuildViewTestCase, assert_contains_exactly, assert_contains_exactly_in_order,
};

/// `new StarlarkProvider.Key(keyForBuild(Label.parseCanonical(bzl)), name)`.
fn provider_key(bzl: &str, name: &str) -> ProviderKey {
    ProviderKey::Starlark {
        extension_label: Label::parse_canonical_unchecked(bzl),
        exported_name: name.to_string(),
    }
}

/// Runs `f` on the frozen provider value (`StructImpl` in Java).
fn with_info<R>(
    info: &starlark::values::OwnedFrozenValue,
    f: impl for<'v> FnOnce(starlark::values::Value<'v>, starlark::values::Heap<'v>) -> R,
) -> R {
    starlark::values::Heap::temp(|heap| {
        // SAFETY: the value is used only while `info` (which keeps its frozen heap alive) is
        // borrowed.
        let value = unsafe { info.unchecked_frozen_value() }.to_value();
        f(value, heap)
    })
}

fn field<'v>(
    info: starlark::values::Value<'v>,
    name: &str,
    heap: starlark::values::Heap<'v>,
) -> starlark::values::Value<'v> {
    info.get_attr(name, heap)
        .unwrap_or_else(|e| panic!("{e}"))
        .unwrap_or_else(|| panic!("no field {name}"))
}

fn strings<'v>(
    value: starlark::values::Value<'v>,
    heap: starlark::values::Heap<'v>,
) -> Vec<String> {
    value
        .iterate(heap)
        .unwrap_or_else(|e| panic!("{e}"))
        .map(|v| v.unpack_str().expect("a string").to_string())
        .collect()
}

/// `(List<String>) info.getValue(name)`.
fn field_strings(info: &starlark::values::OwnedFrozenValue, name: &str) -> Vec<String> {
    with_info(info, |v, heap| strings(field(v, name, heap), heap))
}

/// `((Map<String, ?>) info.getValue(name)).keySet()`.
fn dict_keys(info: &starlark::values::OwnedFrozenValue, name: &str) -> Vec<String> {
    with_info(info, |v, heap| strings(field(v, name, heap), heap))
}

/// `((Map<String, List<String>>) info.getValue(name)).get(key)`.
fn dict_get_strings(
    info: &starlark::values::OwnedFrozenValue,
    name: &str,
    key: &str,
) -> Vec<String> {
    with_info(info, |v, heap| {
        let dict = field(v, name, heap);
        strings(
            dict.at(heap.alloc(key), heap)
                .unwrap_or_else(|e| panic!("{e}")),
            heap,
        )
    })
}

/// `(StarlarkInt) info.getValue(name)`.
fn field_int(info: &starlark::values::OwnedFrozenValue, name: &str) -> i64 {
    with_info(info, |v, heap| {
        field(v, name, heap)
            .unpack_i32()
            .map(i64::from)
            .expect("an int")
    })
}

/// `Sequence.cast(info.getValue(name), ConfiguredTarget.class, ...)` mapped to label names.
fn field_target_names(info: &starlark::values::OwnedFrozenValue, name: &str) -> Vec<String> {
    with_info(info, |v, heap| {
        field(v, name, heap)
            .iterate(heap)
            .unwrap_or_else(|e| panic!("{e}"))
            .map(|target| {
                let label = field(target, "label", heap);
                field(label, "name", heap)
                    .unpack_str()
                    .expect("a string")
                    .to_string()
            })
            .collect()
    })
}

/// `Sequence.cast(info.getValue(name), Artifact.class, ...)` mapped to file names
/// (`Artifact.getFilename`, Starlark `basename`).
fn field_file_names(info: &starlark::values::OwnedFrozenValue, name: &str) -> Vec<String> {
    with_info(info, |v, heap| {
        field(v, name, heap)
            .iterate(heap)
            .unwrap_or_else(|e| panic!("{e}"))
            .map(|file| {
                field(file, "basename", heap)
                    .unpack_str()
                    .expect("a string")
                    .to_string()
            })
            .collect()
    })
}

/// The labels of the configured target (or list of configured targets) at `key` of the dict
/// field `name`.
fn dict_value_labels(
    info: &starlark::values::OwnedFrozenValue,
    name: &str,
    key: &str,
) -> Vec<Label> {
    with_info(info, |v, heap| {
        let value = field(v, name, heap)
            .at(heap.alloc(key), heap)
            .unwrap_or_else(|e| panic!("{e}"));
        fn label_of<'v>(
            target: starlark::values::Value<'v>,
            heap: starlark::values::Heap<'v>,
        ) -> Label {
            let label = field(target, "label", heap);
            Label::parse_canonical_unchecked(&label.to_str())
        }
        if value.get_attr("label", heap).ok().flatten().is_some() {
            vec![label_of(value, heap)]
        } else {
            value
                .iterate(heap)
                .unwrap_or_else(|e| panic!("{e}"))
                .map(|target| label_of(target, heap))
                .collect()
        }
    })
}

/// Truth `containsExactly` on labels: the same labels, in any order.
fn assert_same_labels(mut actual: Vec<Label>, mut expected: Vec<Label>) {
    actual.sort();
    expected.sort();
    assert_eq!(actual, expected);
}

// Java: StarlarkRuleClassFunctionsTest.scratchParentRule
fn scratch_parent_rule(t: &BuildViewTestCase, rule: &str, rule_args: &[&str]) {
    t.scratch.file("extend_rule_testing/parent/BUILD", &[]);
    t.scratch.file(
        "extend_rule_testing/parent/parent.bzl",
        &[
            "ParentInfo = provider()",
            "def _impl(ctx):",
            "  return [ParentInfo()]",
            &format!("{rule} = rule("),
            "  implementation = _impl,",
            "  extendable = True,",
            "  attrs = { ",
            "    'srcs': attr.label_list(allow_files = ['.parent']),",
            "    'deps': attr.label_list(providers = [ParentInfo]),",
            "  },",
            &rule_args.join("\n"),
            ")",
        ],
    );
}

// Java: StarlarkRuleClassFunctionsTest.scratchStarlarkTransition
fn scratch_starlark_transition(t: &BuildViewTestCase) {
    // TestConstants.PRODUCT_NAME is "bazel": the allowlist overwrite is skipped.
    t.scratch.file(
        "test/build_settings.bzl",
        &["def _impl(ctx):
    return []

string_flag = rule(implementation = _impl, build_setting = config.string(flag = True))
"],
    );
    t.scratch.file(
        "test/BUILD",
        &[r#"load("//test:build_settings.bzl", "string_flag")

string_flag(
    name = "parent-flag",
    build_setting_default = "default-parent",
)

string_flag(
    name = "parent-child-flag",
    build_setting_default = "default-parent-child",
)

string_flag(
    name = "child-flag",
    build_setting_default = "child-default",
)
"#],
    );
    t.scratch.file(
        "test/transitions.bzl",
        &[r#"def _parent_trans_impl(settings, attr):
    return {
        "//test:parent-flag": "parent-changed",
        "//test:parent-child-flag": "parent-child-changed-in-parent",
    }

parent_transition = transition(
    implementation = _parent_trans_impl,
    inputs = [],
    outputs = ["//test:parent-flag", "//test:parent-child-flag"],
)

def _child_trans_impl(settings, attr):
    return {
        "//test:child-flag": "child-changed",
        "//test:parent-child-flag": "parent-child-changed-in-child",
    }

child_transition = transition(
    implementation = _child_trans_impl,
    inputs = [],
    outputs = ["//test:child-flag", "//test:parent-child-flag"],
)
"#],
    );
}

// Java: StarlarkRuleClassFunctionsTest.testRuleCfg_starlark
#[test]
fn test_rule_cfg_starlark() {
    let mut t = BuildViewTestCase::new();
    scratch_starlark_transition(&t);
    t.scratch.file(
        "rule_testing/rule.bzl",
        &[r#"load("//test:transitions.bzl", "parent_transition")

def _impl(ctx):
    pass

demo_rule = rule(
    implementation = _impl,
    cfg = parent_transition,
)
"#],
    );
    t.scratch.file(
        "rule_testing/BUILD",
        &[r#"load(":rule.bzl", "demo_rule")

demo_rule(name = "my_target")
"#],
    );
    let configuration = {
        let ct = t
            .get_configured_target("//rule_testing:my_target")
            .expect("configured target");
        t.get_configuration(&ct).expect("configuration")
    };
    let options = configuration.get_options().get_starlark_options().clone();
    assert_eq!(
        options.get(&Label::parse_canonical_unchecked("//test:parent-flag")),
        Some(&StarlarkOptionValue::Str("parent-changed".to_string()))
    );
    assert_eq!(
        options.get(&Label::parse_canonical_unchecked(
            "//test:parent-child-flag"
        )),
        Some(&StarlarkOptionValue::Str(
            "parent-child-changed-in-parent".to_string()
        ))
    );
    assert_eq!(
        options.get(&Label::parse_canonical_unchecked("//test:child-flag")),
        None
    );
}

// Java: StarlarkRuleClassFunctionsTest.extendRule_ccBinary
#[test]
fn extend_rule_cc_binary() {
    let _t = BuildViewTestCase::new();
    // `if (analysisMock.isThisBazel()) return;`: Bazel's AnalysisMock is "this Bazel".
}

// Java: StarlarkRuleClassFunctionsTest.extendRule_superNotCalled
#[test]
fn extend_rule_super_not_called() {
    let mut t = BuildViewTestCase::new();
    scratch_parent_rule(&t, "parent_library", &[]);
    // parent has srcs and deps attribute
    t.scratch.file(
        "extend_rule_testing/child.bzl",
        &[
            r#"load("//extend_rule_testing/parent:parent.bzl", "parent_library")

def _impl(ctx):
    return []

my_library = rule(
    implementation = _impl,
    parent = parent_library,
)
"#,
        ],
    );
    t.scratch.file(
        "extend_rule_testing/BUILD",
        &[r#"load(":child.bzl", "my_library")

my_library(
    name = "my_target",
    srcs = ["a.parent"],
)
"#],
    );
    t.remove_fail_fast_handler();
    // reporter.addHandler(ev.getEventCollector()): the harness collects the reporter's events.
    t.get_configured_target("//extend_rule_testing:my_target");
    t.assert_contains_event_of_kinds(
        "in my_library rule //extend_rule_testing:my_target: 'super' was not called.",
        EventKindSet::of(&[EventKind::Error]),
    );
}

// Java: StarlarkRuleClassFunctionsTest.extendRule_superCalledTwice
#[test]
fn extend_rule_super_called_twice() {
    let mut t = BuildViewTestCase::new();
    scratch_parent_rule(&t, "parent_library", &[]);
    // parent has srcs and deps attribute
    t.scratch.file(
        "extend_rule_testing/child.bzl",
        &[
            r#"load("//extend_rule_testing/parent:parent.bzl", "parent_library")

def _impl(ctx):
    ctx.super()
    ctx.super()
    return []

my_library = rule(
    implementation = _impl,
    parent = parent_library,
)
"#,
        ],
    );
    t.scratch.file(
        "extend_rule_testing/BUILD",
        &[r#"load(":child.bzl", "my_library")

my_library(
    name = "my_target",
    srcs = ["a.parent"],
)
"#],
    );
    t.remove_fail_fast_handler();
    // reporter.addHandler(ev.getEventCollector()): the harness collects the reporter's events.
    t.get_configured_target("//extend_rule_testing:my_target");
    t.assert_contains_event_of_kinds(
        "Error in super: 'super' called the second time.",
        EventKindSet::of(&[EventKind::Error]),
    );
}

// Java: StarlarkRuleClassFunctionsTest.extendRule_noParent_superCalled
#[test]
fn extend_rule_no_parent_super_called() {
    let mut t = BuildViewTestCase::new();
    scratch_parent_rule(&t, "parent_library", &[]);
    // parent has srcs and deps attribute
    t.scratch.file(
        "extend_rule_testing/child.bzl",
        &["def _impl(ctx):
    ctx.super()
    return []

my_library = rule(
    implementation = _impl,
)
"],
    );
    t.scratch.file(
        "extend_rule_testing/BUILD",
        &[r#"load(":child.bzl", "my_library")

my_library(name = "my_target")
"#],
    );
    t.remove_fail_fast_handler();
    // reporter.addHandler(ev.getEventCollector()): the harness collects the reporter's events.
    t.get_configured_target("//extend_rule_testing:my_target");
    t.assert_contains_event_of_kinds(
        "Error in super: Can't use 'super' call, the rule has no parent.",
        EventKindSet::of(&[EventKind::Error]),
    );
}

// Java: StarlarkRuleClassFunctionsTest.extendRule_extendRuleTwice
#[test]
fn extend_rule_extend_rule_twice() {
    let mut t = BuildViewTestCase::new();
    scratch_parent_rule(&t, "parent_library", &[]);
    // parent has srcs and deps attribute
    t.scratch.file(
        "extend_rule_testing/first_extension.bzl",
        &[
            r#"load("//extend_rule_testing/parent:parent.bzl", "parent_library")

MyInfo1 = provider()

def _impl(ctx):
    return ctx.super() + [MyInfo1()]

library_extended_once = rule(
    implementation = _impl,
    parent = parent_library,
    extendable = True,
)
"#,
        ],
    );
    t.scratch.file(
        "extend_rule_testing/second_extension.bzl",
        &[
            r#"load("//extend_rule_testing:first_extension.bzl", "library_extended_once")

MyInfo2 = provider()

def _impl(ctx):
    return ctx.super() + [MyInfo2()]

library_extended_twice = rule(
    implementation = _impl,
    parent = library_extended_once,
)
"#,
        ],
    );
    t.scratch.file(
        "extend_rule_testing/BUILD",
        &[r#"load(":second_extension.bzl", "library_extended_twice")

library_extended_twice(name = "my_target")
"#],
    );
    let my_target = t.get_configured_target("//extend_rule_testing:my_target");
    let my_info1_key = provider_key("//extend_rule_testing:first_extension.bzl", "MyInfo1");
    let my_info2_key = provider_key("//extend_rule_testing:second_extension.bzl", "MyInfo2");
    let parent_info_key = provider_key("//extend_rule_testing/parent:parent.bzl", "ParentInfo");
    assert!(
        my_target
            .as_ref()
            .expect("configured target")
            .get(&my_info1_key)
            .is_some()
    );
    assert!(
        my_target
            .as_ref()
            .expect("configured target")
            .get(&my_info2_key)
            .is_some()
    );
    assert!(
        my_target
            .as_ref()
            .expect("configured target")
            .get(&parent_info_key)
            .is_some()
    );
}

// Java: StarlarkRuleClassFunctionsTest.extendRule_extendRuleTwice_superNotCalled
#[test]
fn extend_rule_extend_rule_twice_super_not_called() {
    let mut t = BuildViewTestCase::new();
    scratch_parent_rule(&t, "parent_library", &[]);
    // parent has srcs and deps attribute
    t.scratch.file(
        "extend_rule_testing/first_extension.bzl",
        &[
            r#"load("//extend_rule_testing/parent:parent.bzl", "parent_library")

def _impl(ctx):
    # <- here we didn't call ctx.super()
    return []

library_extended_once = rule(
    implementation = _impl,
    parent = parent_library,
    extendable = True,
)
"#,
        ],
    );
    t.scratch.file(
        "extend_rule_testing/second_extension.bzl",
        &[
            r#"load("//extend_rule_testing:first_extension.bzl", "library_extended_once")

def _impl(ctx):
    return ctx.super()

library_extended_twice = rule(
    implementation = _impl,
    parent = library_extended_once,
)
"#,
        ],
    );
    t.scratch.file(
        "extend_rule_testing/BUILD",
        &[r#"load(":second_extension.bzl", "library_extended_twice")

library_extended_twice(name = "my_target")
"#],
    );
    t.remove_fail_fast_handler();
    // reporter.addHandler(ev.getEventCollector()): the harness collects the reporter's events.
    t.get_configured_target("//extend_rule_testing:my_target");
    t.assert_contains_event_of_kinds("in library_extended_twice rule //extend_rule_testing:my_target: in library_extended_once rule: 'super' was not called.", EventKindSet::of(&[EventKind::Error]));
}

// Java: StarlarkRuleClassFunctionsTest.ctxSuper_calledFromAspect
#[test]
fn ctx_super_called_from_aspect() {
    let mut t = BuildViewTestCase::new();
    t.scratch.file(
        "extend_rule_testing/child.bzl",
        &[r#"def _aspect_impl(target, ctx):
    ctx.super()
    return []

my_aspect = aspect(_aspect_impl)

def _impl(ctx):
    pass

my_library = rule(
    implementation = _impl,
    attrs = {"deps": attr.label_list(aspects = [my_aspect])},
)
"#],
    );
    t.scratch.file(
        "extend_rule_testing/BUILD",
        &[r#"load(":child.bzl", "my_library")

my_library(
    name = "my_target",
    deps = [":dep"],
)

filegroup(name = "dep")
"#],
    );
    t.remove_fail_fast_handler();
    // reporter.addHandler(ev.getEventCollector()): the harness collects the reporter's events.
    t.get_configured_target("//extend_rule_testing:my_target");
    t.assert_contains_event_of_kinds(
        "Error in super: Can't use 'super' call in an aspect.",
        EventKindSet::of(&[EventKind::Error]),
    );
}

// Java: StarlarkRuleClassFunctionsTest.testAspectWithInvalidToolchainsAspects_fails
#[test]
fn test_aspect_with_invalid_toolchains_aspects_fails() {
    let mut t = BuildViewTestCase::new();
    t.scratch.file(
        "pkg/def.bzl",
        &[r#"def _aspect_impl(target, ctx):
    return []
my_aspect = aspect(
    implementation = _aspect_impl,
    toolchains_aspects = ["@:invalid_toolchain_label"]
)

def _rule_impl(ctx):
    pass
my_rule = rule(
    implementation = _rule_impl,
    attrs = {"deps": attr.label_list(aspects = [my_aspect])})
"#],
    );
    t.scratch.file(
        "pkg/BUILD",
        &[r#"load(":def.bzl", "my_rule")
my_rule(
    name = "t1",
    deps = [":t2"],
)
my_rule(
    name = "t2",
)
"#],
    );
    t.remove_fail_fast_handler();
    let target = t.get_configured_target("//pkg:t1");
    assert!(target.is_none());
    t.assert_contains_event("Error in aspect: Unable to parse label '@:invalid_toolchain_label' in attribute 'toolchains_aspects'");
}

// Java: StarlarkRuleClassFunctionsTest.extendRule_overridePrivateAttribute_fails
#[test]
fn extend_rule_override_private_attribute_fails() {
    let mut t = BuildViewTestCase::new();
    //
    t.scratch.file(
        "extend_rule_testing/parent/BUILD",
        &["filegroup(name = 'parent_tool')"],
    );
    t.scratch.file(
        "extend_rule_testing/parent/parent.bzl",
        &[r#"def _impl(ctx):
    return []

parent_library = rule(
    implementation = _impl,
    attrs = {
        "_tool": attr.label(default = ":parent_tool"),
    },
)
"#],
    );
    t.scratch.file(
        "extend_rule_testing/child.bzl",
        &[
            r#"load("//extend_rule_testing/parent:parent.bzl", "parent_library")

def _impl(ctx):
    return ctx.super()

my_library = rule(
    implementation = _impl,
    parent = parent_library,
    attrs = {
        "_tool": attr.label(default = ":child_tool"),
    },
)
"#,
        ],
    );
    t.scratch.file(
        "extend_rule_testing/BUILD",
        &[r#"load(":child.bzl", "my_library")

my_library(name = "my_target")

filegroup(name = "child_tool")
"#],
    );
    t.remove_fail_fast_handler();
    // reporter.addHandler(ev.getEventCollector()): the harness collects the reporter's events.
    t.get_configured_target("//extend_rule_testing:BUILD");
    t.assert_contains_event_of_kinds(
        "Error in rule: attribute `_tool`: private attributes cannot be overridden.",
        EventKindSet::of(&[EventKind::Error]),
    );
}

// Java: StarlarkRuleClassFunctionsTest.extendRule_parentPrivateAttrDefault_visibilityCheckedAgainstParent
#[test]
fn extend_rule_parent_private_attr_default_visibility_checked_against_parent() {
    let mut t = BuildViewTestCase::new();
    t.scratch.file(
        "extend_rule_testing/parent/BUILD",
        &[r#"exports_files(
    ["parent_tool.txt"],
    visibility = ["//extend_rule_testing/parent:__pkg__"],
)
"#],
    );
    t.scratch
        .file("extend_rule_testing/parent/parent_tool.txt", &[]);
    t.scratch.file(
        "extend_rule_testing/parent/parent.bzl",
        &[r#"def _impl(ctx):
    return []

parent_library = rule(
    implementation = _impl,
    extendable = True,
    attrs = {
        "_tool": attr.label(
            allow_single_file = True,
            default = "//extend_rule_testing/parent:parent_tool.txt",
        ),
    },
)
"#],
    );
    t.scratch.file(
        "extend_rule_testing/child.bzl",
        &[
            r#"load("//extend_rule_testing/parent:parent.bzl", "parent_library")

def _impl(ctx):
    return ctx.super()

my_library = rule(
    implementation = _impl,
    parent = parent_library,
)
"#,
        ],
    );
    t.scratch.file(
        "extend_rule_testing/BUILD",
        &[r#"load(":child.bzl", "my_library")

my_library(name = "my_target")
"#],
    );
    // This should succeed because the visibility of the parent's private attribute default should
    // be checked against the parent rule's package, not the child rule's package.
    // See https://github.com/bazelbuild/bazel/issues/28618
    t.get_configured_target("//extend_rule_testing:my_target");
    t.assert_no_events();
}

// Java: StarlarkRuleClassFunctionsTest.extendRule_extendableAllowed
#[test]
fn extend_rule_extendable_allowed() {
    let mut t = BuildViewTestCase::new();
    t.scratch.file("extend_rule_testing/parent/BUILD", &[]);
    t.scratch.file(
        "extend_rule_testing/parent/parent.bzl",
        &["ParentInfo = provider()

def _impl(ctx):
    return [ParentInfo()]

parent_library = rule(
    implementation = _impl,
    extendable = True,
)
"],
    );
    t.scratch.file(
        "extend_rule_testing/child.bzl",
        &[
            r#"load("//extend_rule_testing/parent:parent.bzl", "parent_library")

def _impl(ctx):
    ctx.super()

my_library = rule(
    implementation = _impl,
    parent = parent_library,
)
"#,
        ],
    );
    t.scratch.file(
        "extend_rule_testing/BUILD",
        &[r#"load(":child.bzl", "my_library")

my_library(name = "my_target")
"#],
    );
    t.get_configured_target("//extend_rule_testing:my_target");
    t.assert_no_events();
}

// Java: StarlarkRuleClassFunctionsTest.extendRule_extendableAllowlisted
#[test]
fn extend_rule_extendable_allowlisted() {
    let mut t = BuildViewTestCase::new();
    t.scratch.file(
        "extend_rule_testing/parent/BUILD",
        &[r#"package_group(
    name = "allowlist",
    packages = ["//extend_rule_testing"],
)
"#],
    );
    t.scratch.file(
        "extend_rule_testing/parent/parent.bzl",
        &[r#"ParentInfo = provider()

def _impl(ctx):
    return [ParentInfo()]

parent_library = rule(
    implementation = _impl,
    extendable = "//extend_rule_testing/parent:allowlist",
)
"#],
    );
    t.scratch.file(
        "extend_rule_testing/child.bzl",
        &[
            r#"load("//extend_rule_testing/parent:parent.bzl", "parent_library")

def _impl(ctx):
    ctx.super()

my_library = rule(
    implementation = _impl,
    parent = parent_library,
)
"#,
        ],
    );
    t.scratch.file(
        "extend_rule_testing/BUILD",
        &[r#"load(":child.bzl", "my_library")

my_library(name = "my_target")
"#],
    );
    t.scratch.file(
        "not_on_allowlist/BUILD",
        &[r#"load("//extend_rule_testing:child.bzl", "my_library")

my_library(name = "my_target")
"#],
    );
    t.get_configured_target("//extend_rule_testing:my_target");
    t.get_configured_target("//not_on_allowlist:my_target");
    t.assert_no_events();
}

// Java: StarlarkRuleClassFunctionsTest.extendRule_extendableAllowlistDenied
#[test]
fn extend_rule_extendable_allowlist_denied() {
    let mut t = BuildViewTestCase::new();
    t.scratch.file(
        "extend_rule_testing/parent/BUILD",
        &[r#"package_group(
    name = "allowlist",
    packages = [],
)
"#],
    );
    t.scratch.file(
        "extend_rule_testing/parent/parent.bzl",
        &[r#"ParentInfo = provider()

def _impl(ctx):
    return [ParentInfo()]

parent_library = rule(
    implementation = _impl,
    extendable = "//extend_rule_testing/parent:allowlist",
)
"#],
    );
    t.scratch.file(
        "extend_rule_testing/child.bzl",
        &[
            r#"load("//extend_rule_testing/parent:parent.bzl", "parent_library")

def _impl(ctx):
    ctx.super()

my_library = rule(
    implementation = _impl,
    parent = parent_library,
)
"#,
        ],
    );
    t.scratch.file(
        "extend_rule_testing/BUILD",
        &[r#"load(":child.bzl", "my_library")

my_library(name = "my_target")
"#],
    );
    t.remove_fail_fast_handler();
    // reporter.addHandler(ev.getEventCollector()): the harness collects the reporter's events.
    t.get_configured_target("//extend_rule_testing:my_target");
    t.assert_contains_event_of_kinds(
        "Non-allowlisted attempt to extend a rule.",
        EventKindSet::of(&[EventKind::Error]),
    );
}

// Java: StarlarkRuleClassFunctionsTest.extendRule_extendableDefault
#[test]
fn extend_rule_extendable_default() {
    let mut t = BuildViewTestCase::new();
    t.scratch.file("extend_rule_testing/parent/BUILD", &[]);
    t.scratch.file(
        "extend_rule_testing/parent/parent.bzl",
        &["ParentInfo = provider()

def _impl(ctx):
    return [ParentInfo()]

parent_library = rule(
    implementation = _impl,
)
"],
    );
    t.scratch.file(
        "extend_rule_testing/child.bzl",
        &[
            r#"load("//extend_rule_testing/parent:parent.bzl", "parent_library")

def _impl(ctx):
    ctx.super()

my_library = rule(
    implementation = _impl,
    parent = parent_library,
)
"#,
        ],
    );
    t.scratch.file(
        "extend_rule_testing/BUILD",
        &[r#"load(":child.bzl", "my_library")

my_library(name = "my_target")
"#],
    );
    // (not Bazel only)
    t.get_configured_target("//extend_rule_testing:my_target");
    t.assert_no_events();
}

// Java: StarlarkRuleClassFunctionsTest.extendRule_cfg_fromParent
#[test]
fn extend_rule_cfg_from_parent() {
    let mut t = BuildViewTestCase::new();
    scratch_starlark_transition(&t);
    t.scratch.file("extend_rule_testing/parent/BUILD", &[]);
    t.scratch.file(
        "extend_rule_testing/parent/parent.bzl",
        &[r#"load("//test:transitions.bzl", "parent_transition")

def _impl(ctx):
    pass

parent_rule = rule(
    implementation = _impl,
    extendable = True,
    cfg = parent_transition,
)
"#],
    );
    t.scratch.file(
        "extend_rule_testing/child.bzl",
        &[
            r#"load("//extend_rule_testing/parent:parent.bzl", "parent_rule")

def _impl(ctx):
    ctx.super()

my_library = rule(
    implementation = _impl,
    parent = parent_rule,
)
"#,
        ],
    );
    t.scratch.file(
        "extend_rule_testing/BUILD",
        &[r#"load(":child.bzl", "my_library")

my_library(name = "my_target")
"#],
    );
    let configuration = {
        let ct = t
            .get_configured_target("//extend_rule_testing:my_target")
            .expect("configured target");
        t.get_configuration(&ct).expect("configuration")
    };
    let options = configuration.get_options().get_starlark_options().clone();
    assert_eq!(
        options.get(&Label::parse_canonical_unchecked("//test:parent-flag")),
        Some(&StarlarkOptionValue::Str("parent-changed".to_string()))
    );
    assert_eq!(
        options.get(&Label::parse_canonical_unchecked(
            "//test:parent-child-flag"
        )),
        Some(&StarlarkOptionValue::Str(
            "parent-child-changed-in-parent".to_string()
        ))
    );
    assert_eq!(
        options.get(&Label::parse_canonical_unchecked("//test:child-flag")),
        None
    );
}

// Java: StarlarkRuleClassFunctionsTest.extendRule_cfg_onChild
#[test]
fn extend_rule_cfg_on_child() {
    let mut t = BuildViewTestCase::new();
    scratch_starlark_transition(&t);
    t.scratch.file("extend_rule_testing/parent/BUILD", &[]);
    t.scratch.file(
        "extend_rule_testing/parent/parent.bzl",
        &["def _impl(ctx):
    pass

parent_rule = rule(
    implementation = _impl,
    extendable = True,
)
"],
    );
    t.scratch.file(
        "extend_rule_testing/child.bzl",
        &[
            r#"load("//extend_rule_testing/parent:parent.bzl", "parent_rule")
load("//test:transitions.bzl", "child_transition")

def _impl(ctx):
    ctx.super()

my_library = rule(
    implementation = _impl,
    parent = parent_rule,
    cfg = child_transition,
)
"#,
        ],
    );
    t.scratch.file(
        "extend_rule_testing/BUILD",
        &[r#"load(":child.bzl", "my_library")

my_library(name = "my_target")
"#],
    );
    let configuration = {
        let ct = t
            .get_configured_target("//extend_rule_testing:my_target")
            .expect("configured target");
        t.get_configuration(&ct).expect("configuration")
    };
    let options = configuration.get_options().get_starlark_options().clone();
    assert_eq!(
        options.get(&Label::parse_canonical_unchecked("//test:parent-flag")),
        None
    );
    assert_eq!(
        options.get(&Label::parse_canonical_unchecked(
            "//test:parent-child-flag"
        )),
        Some(&StarlarkOptionValue::Str(
            "parent-child-changed-in-child".to_string()
        ))
    );
    assert_eq!(
        options.get(&Label::parse_canonical_unchecked("//test:child-flag")),
        Some(&StarlarkOptionValue::Str("child-changed".to_string()))
    );
}

// Java: StarlarkRuleClassFunctionsTest.extendRule_cfg_onChildAndFromParent
#[test]
fn extend_rule_cfg_on_child_and_from_parent() {
    let mut t = BuildViewTestCase::new();
    scratch_starlark_transition(&t);
    t.scratch.file("extend_rule_testing/parent/BUILD", &[]);
    t.scratch.file(
        "extend_rule_testing/parent/parent.bzl",
        &[r#"load("//test:transitions.bzl", "parent_transition")

def _impl(ctx):
    pass

parent_rule = rule(
    implementation = _impl,
    extendable = True,
    cfg = parent_transition,
)
"#],
    );
    t.scratch.file(
        "extend_rule_testing/child.bzl",
        &[
            r#"load("//extend_rule_testing/parent:parent.bzl", "parent_rule")
load("//test:transitions.bzl", "child_transition")

def _impl(ctx):
    ctx.super()

my_library = rule(
    implementation = _impl,
    parent = parent_rule,
    cfg = child_transition,
)
"#,
        ],
    );
    t.scratch.file(
        "extend_rule_testing/BUILD",
        &[r#"load(":child.bzl", "my_library")

my_library(name = "my_target")
"#],
    );
    let configuration = {
        let ct = t
            .get_configured_target("//extend_rule_testing:my_target")
            .expect("configured target");
        t.get_configuration(&ct).expect("configuration")
    };
    let options = configuration.get_options().get_starlark_options().clone();
    assert_eq!(
        options.get(&Label::parse_canonical_unchecked("//test:parent-flag")),
        Some(&StarlarkOptionValue::Str("parent-changed".to_string()))
    );
    assert_eq!(
        options.get(&Label::parse_canonical_unchecked(
            "//test:parent-child-flag"
        )),
        Some(&StarlarkOptionValue::Str(
            "parent-child-changed-in-parent".to_string()
        ))
    );
    assert_eq!(
        options.get(&Label::parse_canonical_unchecked("//test:child-flag")),
        Some(&StarlarkOptionValue::Str("child-changed".to_string()))
    );
}

// Java: StarlarkRuleClassFunctionsTest.testAnalysisTest
#[test]
fn test_analysis_test() {
    let mut t = BuildViewTestCase::new();
    t.scratch.file(
        "p/b.bzl",
        &[r#"def impl(ctx):
    return [AnalysisTestResultInfo(
        success = True,
        message = "",
    )]

def my_test_macro(name):
    testing.analysis_test(name = name, implementation = impl)
"#],
    );
    t.scratch.file(
        "p/BUILD",
        &[r#"load(":b.bzl", "my_test_macro")

my_test_macro(name = "my_test_target")
"#],
    );
    t.get_configured_target("//p:my_test_target");
    t.assert_no_events();
}

// Java: StarlarkRuleClassFunctionsTest.testAnalysisTestAttrs
#[test]
fn test_analysis_test_attrs() {
    let mut t = BuildViewTestCase::new();
    t.scratch.file(
        "p/b.bzl",
        &[r#"def impl(ctx):
    ctx.attr.target_under_test
    return [AnalysisTestResultInfo(
        success = True,
        message = "",
    )]

def my_test_macro(name):
    native.filegroup(name = "my_subject", srcs = [])
    testing.analysis_test(
        name = name,
        implementation = impl,
        attrs = {"target_under_test": attr.label_list()},
        attr_values = {"target_under_test": [":my_subject"]},
    )
"#],
    );
    t.scratch.file(
        "p/BUILD",
        &[r#"load(":b.bzl", "my_test_macro")

my_test_macro(name = "my_test_target")
"#],
    );
    t.get_configured_target("//p:my_test_target");
    t.assert_no_events();
}

// Java: StarlarkRuleClassFunctionsTest.testAnalysisTestDuplicateName_samePackage
#[test]
fn test_analysis_test_duplicate_name_same_package() {
    let mut t = BuildViewTestCase::new();
    t.scratch.file(
        "p/a.bzl",
        &[r#"def impl(ctx):
    return [AnalysisTestResultInfo(
        success = True,
        message = "",
    )]

def my_test_macro1(name):
    testing.analysis_test(name = name, implementation = impl)
"#],
    );
    t.scratch.file(
        "p/b.bzl",
        &[r#"def impl(ctx):
    return [AnalysisTestResultInfo(
        success = True,
        message = "",
    )]

def my_test_macro2(name):
    testing.analysis_test(name = name, implementation = impl)
"#],
    );
    t.scratch.file(
        "p/BUILD",
        &[r#"load(":a.bzl", "my_test_macro1")
load(":b.bzl", "my_test_macro2")

my_test_macro1(name = "my_test_target")

my_test_macro2(name = "my_test_target")
"#],
    );
    t.remove_fail_fast_handler();
    // reporter.addHandler(ev.getEventCollector()): the harness collects the reporter's events.
    t.get_configured_target("//p:my_test_target");
    t.assert_contains_event_of_kinds("Error in analysis_test: my_test_target_test rule 'my_test_target' conflicts with existing my_test_target_test rule", EventKindSet::of(&[EventKind::Error]));
}

// Java: StarlarkRuleClassFunctionsTest.testAnalysisTestBadName
#[test]
fn test_analysis_test_bad_name() {
    let mut t = BuildViewTestCase::new();
    t.scratch.file(
        "p/b.bzl",
        &[r#"def impl(ctx):
    return [AnalysisTestResultInfo(
        success = True,
        message = "",
    )]

def my_test_macro(name):
    testing.analysis_test(name = name, implementation = impl)
"#],
    );
    t.scratch.file(
        "p/BUILD",
        &[r#"load(":b.bzl", "my_test_macro")

my_test_macro(name = "my+test+target")
"#],
    );
    t.remove_fail_fast_handler();
    // reporter.addHandler(ev.getEventCollector()): the harness collects the reporter's events.
    t.get_configured_target("//p:my+test+target");
    t.assert_contains_event_of_kinds(
        "Error in analysis_test: 'name' is limited to Starlark identifiers, got my+test+target",
        EventKindSet::of(&[EventKind::Error]),
    );
}

// Java: StarlarkRuleClassFunctionsTest.testAnalysisTestBadArgs
#[test]
fn test_analysis_test_bad_args() {
    let mut t = BuildViewTestCase::new();
    t.scratch.file(
        "p/b.bzl",
        &[r#"def impl(ctx):
    return [AnalysisTestResultInfo(
        success = True,
        message = "",
    )]

def my_test_macro(name):
    testing.analysis_test(
        name = name,
        implementation = impl,
        attr_values = {"notthere": []},
    )
"#],
    );
    t.scratch.file(
        "p/BUILD",
        &[r#"load(":b.bzl", "my_test_macro")

my_test_macro(name = "my_test_target")
"#],
    );
    t.remove_fail_fast_handler();
    // reporter.addHandler(ev.getEventCollector()): the harness collects the reporter's events.
    t.get_configured_target("//p:my_test_target");
    t.assert_contains_event_of_kinds(
        "no such attribute 'notthere' in 'my_test_target_test' rule",
        EventKindSet::of(&[EventKind::Error]),
    );
}

// Java: StarlarkRuleClassFunctionsTest.testAnalysisTestErrorOnExport
#[test]
fn test_analysis_test_error_on_export() {
    let mut t = BuildViewTestCase::new();
    t.scratch.file("p/b.bzl", &["def impl(ctx): ", "  return  [AnalysisTestResultInfo(", "    success = True,", "    message = ''", "  )]", "def my_test_macro(name):", "  testing.analysis_test(name = name, implementation = impl, attrs = {'name': attr.string()})"]);
    t.scratch.file(
        "p/BUILD",
        &[r#"load(":b.bzl", "my_test_macro")

my_test_macro(name = "my_test_target")
"#],
    );
    t.remove_fail_fast_handler();
    // reporter.addHandler(ev.getEventCollector()): the harness collects the reporter's events.
    t.get_configured_target("//p:my_test_target");
    t.assert_contains_event_of_kinds(
        "Error in analysis_test: attribute `name`: built-in attributes cannot be overridden",
        EventKindSet::of(&[EventKind::Error]),
    );
}

// Java: StarlarkRuleClassFunctionsTest.testAnalysisTestErrorOverridingName
#[test]
fn test_analysis_test_error_overriding_name() {
    let mut t = BuildViewTestCase::new();
    t.scratch.file("p/b.bzl", &["def impl(ctx): ", "  return  [AnalysisTestResultInfo(", "    success = True,", "    message = ''", "  )]", "def my_test_macro(name):", "  testing.analysis_test(name = name, implementation = impl, attr_values = {'name': 'override'})"]);
    t.scratch.file(
        "p/BUILD",
        &[r#"load(":b.bzl", "my_test_macro")

my_test_macro(name = "my_test_target")
"#],
    );
    t.remove_fail_fast_handler();
    // reporter.addHandler(ev.getEventCollector()): the harness collects the reporter's events.
    t.get_configured_target("//p:override");
    t.assert_contains_event_of_kinds(
        "Error in analysis_test: 'name' cannot be set or overridden in 'attr_values'",
        EventKindSet::of(&[EventKind::Error]),
    );
}

// Java: StarlarkRuleClassFunctionsTest.testPackageMetadataAttrOnAllRules
#[test]
fn test_package_metadata_attr_on_all_rules() {
    let mut t = BuildViewTestCase::new();
    t.scratch.file(
        "p/b.bzl",
        &[r#"def _my_rule_impl(ctx):
    license_file = ctx.attr.package_metadata[0][DefaultInfo].files.to_list()[0]
    print("ctx.attr.package_metadata: %s" % license_file.path)

my_rule = rule(_my_rule_impl)
"#],
    );
    t.scratch.file(
        "p/BUILD",
        &[r#"load(":b.bzl", "my_rule")

filegroup(
    name = "licenses",
    srcs = ["LICENSE"],
)

my_rule(
    name = "my_target",
    package_metadata = [":licenses"],
)
"#],
    );
    t.get_configured_target("//p:my_target");
    t.assert_contains_event("ctx.attr.package_metadata: p/LICENSE");
}

// Java: StarlarkRuleClassFunctionsTest.initializer_basic
#[test]
fn initializer_basic() {
    let mut t = BuildViewTestCase::new();
    //
    t.scratch.file(
        "BUILD",
        &["filegroup(name = 'initial')", "filegroup(name = 'added')"],
    );
    t.scratch.file(
        "initializer_testing/b.bzl",
        &[r#"MyInfo = provider()

def initializer(name, srcs = [], deps = []):
    return {"deps": deps + ["//:added"]}

def impl(ctx):
    return [MyInfo(
        srcs = [s.short_path for s in ctx.files.srcs],
        deps = [str(d.label) for d in ctx.attr.deps],
    )]

my_rule = rule(
    impl,
    initializer = initializer,
    attrs = {
        "srcs": attr.label_list(allow_files = ["ml"]),
        "deps": attr.label_list(),
    },
)
"#],
    );
    t.scratch.file(
        "initializer_testing/BUILD",
        &[r#"load(":b.bzl", "my_rule")

my_rule(
    name = "my_target",
    srcs = ["a.ml"],
    deps = ["//:initial"],
)
"#],
    );
    let my_target = t.get_configured_target("//initializer_testing:my_target");
    let info = my_target
        .as_ref()
        .expect("configured target")
        .get(&provider_key("//initializer_testing:b.bzl", "MyInfo"))
        .expect("provider");
    assert_contains_exactly(&field_strings(&info, "srcs"), &["initializer_testing/a.ml"]);
    assert_contains_exactly(
        &field_strings(&info, "deps"),
        &["@@//:initial", "@@//:added"],
    );
}

// Java: StarlarkRuleClassFunctionsTest.initializer_stringListDict
#[test]
fn initializer_string_list_dict() {
    let mut t = BuildViewTestCase::new();
    t.scratch.file(
        "initializer_testing/b.bzl",
        &[r#"def initializer(**kwargs):
    return {}

MyInfo = provider()

def impl(ctx):
    return [MyInfo(dict = ctx.attr.dict)]

my_rule = rule(
    impl,
    initializer = initializer,
    attrs = {
        "dict": attr.string_list_dict(),
    },
)
"#],
    );
    t.scratch.file(
        "initializer_testing/BUILD",
        &[r#"load(":b.bzl", "my_rule")

my_rule(
    name = "my_target",
    dict = {"k": ["val"]},
)
"#],
    );
    let my_target = t.get_configured_target("//initializer_testing:my_target");
    let info = my_target
        .as_ref()
        .expect("configured target")
        .get(&provider_key("//initializer_testing:b.bzl", "MyInfo"))
        .expect("provider");
    assert_contains_exactly(&dict_keys(&info, "dict"), &["k"]);
    assert_contains_exactly(&dict_get_strings(&info, "dict", "k"), &["val"]);
}

// Java: StarlarkRuleClassFunctionsTest.initializer_legacyAnyType
#[test]
fn initializer_legacy_any_type() {
    let mut t = BuildViewTestCase::new();
    t.scratch.file(
        "initializer_testing/b.bzl",
        &[r#"MyInfo = provider()

def initializer(name, tristate = -1):
    return {"tristate": int(tristate)}

def impl(ctx):
    return [MyInfo(tristate = ctx.attr.tristate)]

my_rule = rule(
    impl,
    initializer = initializer,
    attrs = {
        "tristate": attr.int(),
        "_legacy_any_type_attrs": attr.string_list(default = ["tristate"]),
    },
)
"#],
    );
    t.scratch.file(
        "initializer_testing/BUILD",
        &[r#"load(":b.bzl", "my_rule")

my_rule(
    name = "my_target",
    tristate = True,
)
"#],
    );
    let my_target = t.get_configured_target("//initializer_testing:my_target");
    let info = my_target
        .as_ref()
        .expect("configured target")
        .get(&provider_key("//initializer_testing:b.bzl", "MyInfo"))
        .expect("provider");
    assert_eq!(field_int(&info, "tristate"), 1);
}

// Java: StarlarkRuleClassFunctionsTest.initializer_withSelect
#[test]
fn initializer_with_select() {
    let mut t = BuildViewTestCase::new();
    t.scratch.file(
        "initializer_testing/b.bzl",
        &[r#"MyInfo = provider()

def initializer(name, srcs = []):
    return {"srcs": srcs + ["b.ml"]}

def impl(ctx):
    return [MyInfo(
        srcs = [s.short_path for s in ctx.files.srcs],
    )]

my_rule = rule(
    impl,
    initializer = initializer,
    attrs = {
        "srcs": attr.label_list(allow_files = ["ml"]),
    },
)
"#],
    );
    t.scratch.file(
        "initializer_testing/BUILD",
        &[r#"load(":b.bzl", "my_rule")

my_rule(
    name = "my_target",
    srcs = select({"//conditions:default": ["a.ml"]}),
)
"#],
    );
    let my_target = t.get_configured_target("//initializer_testing:my_target");
    let info = my_target
        .as_ref()
        .expect("configured target")
        .get(&provider_key("//initializer_testing:b.bzl", "MyInfo"))
        .expect("provider");
    assert_contains_exactly(
        &field_strings(&info, "srcs"),
        &["initializer_testing/a.ml", "initializer_testing/b.ml"],
    );
}

// Java: StarlarkRuleClassFunctionsTest.initializer_overridesAttributeDefault
#[test]
fn initializer_overrides_attribute_default() {
    let mut t = BuildViewTestCase::new();
    //
    t.scratch.file(
        "BUILD",
        &[
            "filegroup(name = 'initializer_default')",
            "filegroup(name = 'attr_default')",
        ],
    );
    t.scratch.file(
        "initializer_testing/b.bzl",
        &[r#"MyInfo = provider()

def initializer(name, deps = ["//:initializer_default"]):
    return {"deps": deps}

def impl(ctx):
    return [MyInfo(
        deps = [str(d.label) for d in ctx.attr.deps],
    )]

my_rule = rule(
    impl,
    initializer = initializer,
    attrs = {
        "deps": attr.label_list(default = ["//:attr_default"]),
    },
)
"#],
    );
    t.scratch.file(
        "initializer_testing/BUILD",
        &[r#"load(":b.bzl", "my_rule")

my_rule(name = "my_target")
"#],
    );
    let my_target = t.get_configured_target("//initializer_testing:my_target");
    let info = my_target
        .as_ref()
        .expect("configured target")
        .get(&provider_key("//initializer_testing:b.bzl", "MyInfo"))
        .expect("provider");
    assert_contains_exactly(&field_strings(&info, "deps"), &["@@//:initializer_default"]);
}

// Java: StarlarkRuleClassFunctionsTest.initializer_returningNoneSetsDefault
#[test]
fn initializer_returning_none_sets_default() {
    let mut t = BuildViewTestCase::new();
    //
    t.scratch.file(
        "BUILD",
        &[
            "filegroup(name = 'initializer_default')",
            "filegroup(name = 'attr_default')",
        ],
    );
    t.scratch.file(
        "initializer_testing/b.bzl",
        &[r#"MyInfo = provider()

def initializer(name, deps = ["//:initializer_default"]):
    return {"deps": None}

def impl(ctx):
    return [MyInfo(
        deps = [str(d.label) for d in ctx.attr.deps],
    )]

my_rule = rule(
    impl,
    initializer = initializer,
    attrs = {
        "deps": attr.label_list(default = ["//:attr_default"]),
    },
)
"#],
    );
    t.scratch.file(
        "initializer_testing/BUILD",
        &[r#"load(":b.bzl", "my_rule")

my_rule(name = "my_target")
"#],
    );
    let my_target = t.get_configured_target("//initializer_testing:my_target");
    let info = my_target
        .as_ref()
        .expect("configured target")
        .get(&provider_key("//initializer_testing:b.bzl", "MyInfo"))
        .expect("provider");
    assert_contains_exactly(&field_strings(&info, "deps"), &["@@//:attr_default"]);
}

// Java: StarlarkRuleClassFunctionsTest.extendRule_attributeOverrideDefault
#[test]
fn extend_rule_attribute_override_default() {
    let mut t = BuildViewTestCase::new();
    t.scratch.file("extend_rule_testing/parent/BUILD", &[]);
    t.scratch.file(
        "extend_rule_testing/parent/parent.bzl",
        &[r#"ParentInfo = provider()

def _impl(ctx):
    return [ParentInfo(deps = ctx.attr.deps, tools = [ctx.attr.tool])]

parent_library = rule(
    implementation = _impl,
    extendable = True,
    attrs = {
        "srcs": attr.label_list(allow_files = [".parent"]),
        "deps": attr.label_list(),
        "tool": attr.label(default = ":tool_parent"),
    },
)
"#],
    );
    t.scratch.file(
        "extend_rule_testing/child.bzl",
        &[
            r#"load("//extend_rule_testing/parent:parent.bzl", "parent_library")

def _impl(ctx):
    return ctx.super()

my_library = rule(
    implementation = _impl,
    parent = parent_library,
    attrs = {
        "deps": attr.label_list(default = [":dep"]),
        "tool": attr.label(default = ":tool_child"),
    },
)
"#,
        ],
    );
    t.scratch.file(
        "extend_rule_testing/BUILD",
        &[r#"load(":child.bzl", "my_library")

my_library(name = "my_target")

filegroup(name = "dep")

filegroup(name = "tool_child")
"#],
    );
    let my_target = t.get_configured_target("//extend_rule_testing:my_target");
    let parent_info_key = provider_key("//extend_rule_testing/parent:parent.bzl", "ParentInfo");
    let parent_info = my_target
        .as_ref()
        .expect("configured target")
        .get(&parent_info_key)
        .expect("provider");
    t.assert_no_events();
    assert_contains_exactly(&field_target_names(&parent_info, "deps"), &["dep"]);
    assert_contains_exactly(&field_target_names(&parent_info, "tools"), &["tool_child"]);
}

// Java: StarlarkRuleClassFunctionsTest.extendRule_basicUse
#[test]
fn extend_rule_basic_use() {
    let mut t = BuildViewTestCase::new();
    scratch_parent_rule(&t, "parent_library", &[]);
    // parent has srcs and deps attribute
    t.scratch.file(
        "extend_rule_testing/child.bzl",
        &[
            r#"load("//extend_rule_testing/parent:parent.bzl", "parent_library")

MyInfo = provider()

def _impl(ctx):
    return ctx.super() + [MyInfo(
        srcs = ctx.files.srcs,
        deps = ctx.attr.deps,
        runtime_deps = ctx.attr.runtime_deps,
    )]

my_library = rule(
    implementation = _impl,
    parent = parent_library,
    attrs = {
        "runtime_deps": attr.label_list(),
    },
)
"#,
        ],
    );
    t.scratch.file(
        "extend_rule_testing/BUILD",
        &[r#"load(":child.bzl", "my_library")

my_library(
    name = "my_target",
    srcs = ["a.parent"],
    runtime_deps = [":dep"],
)

filegroup(name = "dep")
"#],
    );
    let my_target = t.get_configured_target("//extend_rule_testing:my_target");
    let rule = t
        .get_rule_context(my_target.as_ref().expect("configured target"))
        .get_rule()
        .clone();
    let my_info_key = provider_key("//extend_rule_testing:child.bzl", "MyInfo");
    let my_info = my_target
        .as_ref()
        .expect("configured target")
        .get(&my_info_key)
        .expect("provider");
    t.assert_no_events();
    assert!(!rule.get_rule_class_object().is_executable_starlark());
    assert_eq!(
        rule.get_rule_class_object().get_rule_class_type(),
        RuleClassType::Normal
    );
    assert_contains_exactly(&field_file_names(&my_info, "srcs"), &["a.parent"]);
    assert_contains_exactly(&field_target_names(&my_info, "deps"), &[]);
    assert_contains_exactly(&field_target_names(&my_info, "runtime_deps"), &["dep"]);
    let parent_info_key = provider_key("//extend_rule_testing/parent:parent.bzl", "ParentInfo");
    assert!(
        my_target
            .as_ref()
            .expect("configured target")
            .get(&parent_info_key)
            .is_some()
    );
}

// Java: StarlarkRuleClassFunctionsTest.extendRule_executableMatches
#[test]
fn extend_rule_executable_matches() {
    let mut t = BuildViewTestCase::new();
    scratch_parent_rule(&t, "parent_binary", &["executable = True,"]);
    t.scratch.file(
        "extend_rule_testing/child.bzl",
        &[
            r#"load("//extend_rule_testing/parent:parent.bzl", "parent_binary")

MyInfo = provider()

def _impl(ctx):
    exec = ctx.actions.declare_file("my_exec")
    ctx.actions.write(exec, "")
    ctx.super()
    return DefaultInfo(executable = exec)

my_binary = rule(
    implementation = _impl,
    parent = parent_binary,
)
"#,
        ],
    );
    t.scratch.file(
        "extend_rule_testing/BUILD",
        &[r#"load(":child.bzl", "my_binary")

my_binary(
    name = "my_target",
    srcs = ["a.parent"],
)
"#],
    );
    let my_target = t.get_configured_target("//extend_rule_testing:my_target");
    let rule = t
        .get_rule_context(my_target.as_ref().expect("configured target"))
        .get_rule()
        .clone();
    t.assert_no_events();
    assert!(rule.get_rule_class_object().is_executable_starlark());
    assert_eq!(
        rule.get_rule_class_object().get_rule_class_type(),
        RuleClassType::Normal
    );
}

// Java: StarlarkRuleClassFunctionsTest.extendRule_testMatches
#[test]
fn extend_rule_test_matches() {
    let mut t = BuildViewTestCase::new();
    scratch_parent_rule(&t, "parent_test", &["test = True,"]);
    t.scratch.file(
        "extend_rule_testing/child.bzl",
        &[
            r#"load("//extend_rule_testing/parent:parent.bzl", "parent_test")

MyInfo = provider()

def _impl(ctx):
    exec = ctx.actions.declare_file("my_exec")
    ctx.actions.write(exec, "")
    ctx.super()
    return DefaultInfo(executable = exec)

my_test = rule(
    implementation = _impl,
    parent = parent_test,
)
"#,
        ],
    );
    t.scratch.file(
        "extend_rule_testing/BUILD",
        &[r#"load(":child.bzl", "my_test")

my_test(
    name = "my_target",
    srcs = ["a.parent"],
)
"#],
    );
    let my_target = t.get_configured_target("//extend_rule_testing:my_target");
    let rule = t
        .get_rule_context(my_target.as_ref().expect("configured target"))
        .get_rule()
        .clone();
    t.assert_no_events();
    assert!(rule.get_rule_class_object().is_executable_starlark());
    assert_eq!(
        rule.get_rule_class_object().get_rule_class_type(),
        RuleClassType::Test
    );
}

// Java: StarlarkRuleClassFunctionsTest.extendRule_fragments_merged
#[test]
fn extend_rule_fragments_merged() {
    let mut t = BuildViewTestCase::new();
    //
    scratch_parent_rule(&t, "parent_library", &["fragments = ['java']"]);
    t.scratch.file(
        "extend_rule_testing/child.bzl",
        &[
            r#"load("//extend_rule_testing/parent:parent.bzl", "parent_library")

MyInfo = provider()

def _impl(ctx):
    ctx.super()

my_library = rule(
    implementation = _impl,
    parent = parent_library,
    fragments = ["cc"],
)
"#,
        ],
    );
    t.scratch.file(
        "extend_rule_testing/BUILD",
        &[r#"load(":child.bzl", "my_library")

my_library(name = "my_target")
"#],
    );
    let my_target = t.get_configured_target("//extend_rule_testing:my_target");
    let rule = t
        .get_rule_context(my_target.as_ref().expect("configured target"))
        .get_rule()
        .clone();
    t.assert_no_events();
    assert_contains_exactly(
        &rule
            .get_rule_class_object()
            .get_configuration_fragment_policy()
            .get_required_starlark_fragments()
            .iter()
            .cloned()
            .collect::<Vec<_>>(),
        &["java", "cc"],
    );
}

// Java: StarlarkRuleClassFunctionsTest.extendRule_execGroups_merged
#[test]
fn extend_rule_exec_groups_merged() {
    let mut t = BuildViewTestCase::new();
    //
    scratch_parent_rule(
        &t,
        "parent_library",
        &["exec_groups = {'parent_exec_group': exec_group()}"],
    );
    t.scratch.file(
        "extend_rule_testing/child.bzl",
        &[
            r#"load("//extend_rule_testing/parent:parent.bzl", "ParentInfo", "parent_library")

MyInfo = provider()

def _impl(ctx):
    ctx.super()
    return [MyInfo(), ParentInfo()]

my_library = rule(
    implementation = _impl,
    parent = parent_library,
    exec_groups = {"child_exec_group": exec_group()},
)
"#,
        ],
    );
    t.scratch.file(
        "extend_rule_testing/BUILD",
        &[r#"load(":child.bzl", "my_library")

my_library(name = "my_target")
"#],
    );
    let my_target = t.get_configured_target("//extend_rule_testing:my_target");
    let rule = t
        .get_rule_context(my_target.as_ref().expect("configured target"))
        .get_rule()
        .clone();
    t.assert_no_events();
    assert_contains_exactly(
        &rule
            .get_rule_class_object()
            .get_declared_exec_groups()
            .keys()
            .cloned()
            .collect::<Vec<_>>(),
        &["parent_exec_group", "child_exec_group"],
    );
}

// Java: StarlarkRuleClassFunctionsTest.stringKeyedLabelDict
#[test]
fn string_keyed_label_dict() {
    let mut t = BuildViewTestCase::new();
    t.scratch.file(
        "a/BUILD",
        &[r#"load(":a.bzl", "a")
a(name="a", dict={"foo_key": ":foo", "gen_key": ":gen"})

filegroup(name="foo", srcs=["foo.txt"])
genrule(name="gen", srcs=[], outs=["gen.txt"], cmd="exit 1")
"#],
    );
    t.scratch.file(
        "a/a.bzl",
        &[r#"DictInfo = provider(fields=["dict"])

def _a_impl(ctx):
    return [DictInfo(dict = ctx.attr.dict)]

a = rule(implementation=_a_impl, attrs={"dict": attr.string_keyed_label_dict()})
"#],
    );
    let a = t.get_configured_target("//a:a");
    let info = a
        .as_ref()
        .expect("configured target")
        .get(&provider_key("//a:a.bzl", "DictInfo"))
        .expect("provider");
    // dict: the dict field dict of info.
    assert_contains_exactly(&dict_keys(&info, "dict"), &["foo_key", "gen_key"]);
    assert_eq!(
        dict_value_labels(&info, "dict", "foo_key"),
        vec![Label::parse_canonical_unchecked("//a:foo")]
    );
    assert_eq!(
        dict_value_labels(&info, "dict", "gen_key"),
        vec![Label::parse_canonical_unchecked("//a:gen")]
    );
}

// Java: StarlarkRuleClassFunctionsTest.labelListDict
#[test]
fn label_list_dict() {
    let mut t = BuildViewTestCase::new();
    t.scratch.file(
        "a/BUILD",
        &[r#"load(":a.bzl", "a")
a(name="a", dict={"foo_key": [":foo", ":gen"], "gen_key": [":gen"]})

filegroup(name="foo", srcs=["foo.txt"])
genrule(name="gen", srcs=[], outs=["gen.txt"], cmd="exit 1")
"#],
    );
    t.scratch.file(
        "a/a.bzl",
        &[r#"DictInfo = provider(fields=["dict"])

def _a_impl(ctx):
    return [DictInfo(dict = ctx.attr.dict)]

a = rule(implementation=_a_impl, attrs={"dict": attr.label_list_dict()})
"#],
    );
    let a = t.get_configured_target("//a:a");
    let info = a
        .as_ref()
        .expect("configured target")
        .get(&provider_key("//a:a.bzl", "DictInfo"))
        .expect("provider");
    // dict: the dict field dict of info.
    assert_contains_exactly(&dict_keys(&info, "dict"), &["foo_key", "gen_key"]);
    assert_same_labels(
        dict_value_labels(&info, "dict", "foo_key"),
        vec![
            Label::parse_canonical_unchecked("//a:foo"),
            Label::parse_canonical_unchecked("//a:gen"),
        ],
    );
    assert_same_labels(
        dict_value_labels(&info, "dict", "gen_key"),
        vec![Label::parse_canonical_unchecked("//a:gen")],
    );
}
