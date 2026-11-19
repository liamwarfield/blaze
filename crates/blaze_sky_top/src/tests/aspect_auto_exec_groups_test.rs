//! Port of `src/test/java/com/google/devtools/build/lib/analysis/AspectAutoExecGroupsTest.java`
//! (a `BuildViewTestCase`): automatic exec groups of Starlark aspects.
//!
//! Java runs each `@TestParameters` test once per `action` value; each value is one Rust test
//! here, named after the Java method with the value as a suffix (`_run` for `ctx.actions.run`,
//! `_run_shell` for `ctx.actions.run_shell`).

use std::sync::Arc;

use blaze_analysis::aspect_key_creator::AspectKey;
use blaze_analysis::aspect_value::AspectValue;
use blaze_analysis::starlark::starlark_toolchain_context::StarlarkExecGroupCollection;
use blaze_packages::rule_class::ProviderKey;
use starlark::values::{Heap, OwnedFrozenValue, Value, ValueLike};

use crate::testing::build_view_test_case::parse_label;
use crate::testing::{BuildViewTestCase, assert_contains_exactly};

/// A fresh `BuildViewTestCase` after the class's `@Before setup()`.
fn new_test() -> BuildViewTestCase {
    let mut t = BuildViewTestCase::new();
    setup(&mut t);
    t
}

// Java: AspectAutoExecGroupsTest.setup
fn setup(t: &mut BuildViewTestCase) {
    use_configuration(t, &[]);
}

/// Sets up two toolchains types, each with a single toolchain implementation and a single
/// exec_compatible_with platform.
///
/// toolchain_type_1 -> foo_toolchain -> exec_compatible_with platform_1 toolchain_type_2 ->
/// bar_toolchain -> exec_compatible_with platform_2
// Java: AspectAutoExecGroupsTest.createToolchainsAndPlatforms
fn create_toolchains_and_platforms(t: &mut BuildViewTestCase) {
    t.scratch.overwrite_file(
        "rule/test_toolchain.bzl",
        &[r##"def _impl(ctx):
    return [platform_common.ToolchainInfo(
        tool = ctx.executable._tool,
        files_to_run = ctx.attr._tool[DefaultInfo].files_to_run,
    )]

test_toolchain = rule(
    implementation = _impl,
    attrs = {
        "_tool": attr.label(
            default = "//toolchain:b_tool",
            executable = True,
            cfg = "exec",
        ),
    },
)
"##],
    );
    t.scratch.overwrite_file(
        "rule/BUILD",
        &[r##"exports_files(["test_toolchain/bzl"])

toolchain_type(name = "toolchain_type_1")

toolchain_type(name = "toolchain_type_2")
"##],
    );
    t.scratch.overwrite_file(
        "toolchain/BUILD",
        &[r##"load("//rule:test_toolchain.bzl", "test_toolchain")

genrule(
    name = "a_tool",
    outs = ["atool"],
    cmd = "",
    executable = True,
)

genrule(
    name = "b_tool",
    outs = ["btool"],
    cmd = "",
    executable = True,
)

test_toolchain(
    name = "foo",
)

toolchain(
    name = "foo_toolchain",
    exec_compatible_with = ["//platforms:constraint_1"],
    target_compatible_with = ["//platforms:constraint_1"],
    toolchain = ":foo",
    toolchain_type = "//rule:toolchain_type_1",
)

test_toolchain(
    name = "bar",
)

toolchain(
    name = "bar_toolchain",
    exec_compatible_with = ["//platforms:constraint_2"],
    target_compatible_with = ["//platforms:constraint_1"],
    toolchain = ":bar",
    toolchain_type = "//rule:toolchain_type_2",
)
"##],
    );

    t.scratch.overwrite_file(
        "platforms/BUILD",
        &[r##"constraint_setting(name = "setting")

constraint_value(
    name = "constraint_1",
    constraint_setting = ":setting",
)

constraint_value(
    name = "constraint_2",
    constraint_setting = ":setting",
)

platform(
    name = "platform_1",
    constraint_values = [":constraint_1"],
)

platform(
    name = "platform_2",
    constraint_values = [":constraint_2"],
    exec_properties = {
        "watermelon.ripeness": "unripe",
        "watermelon.color": "red",
    },
)
"##],
    );
}

/// The class's `useConfiguration` override.
// Java: AspectAutoExecGroupsTest.useConfiguration
fn use_configuration(t: &mut BuildViewTestCase, args: &[&str]) {
    // These need to be defined before the configuration is parsed.
    create_toolchains_and_platforms(t);
    let flags = [
        "--extra_toolchains=//toolchain:foo_toolchain,//toolchain:bar_toolchain",
        "--platforms=//platforms:platform_1",
        "--extra_execution_platforms=//platforms:platform_1,//platforms:platform_2",
    ];

    let all: Vec<&str> = flags.iter().copied().chain(args.iter().copied()).collect();
    t.use_configuration(&all);
}

/// Creates custom rule which produces action with `action_parameters`, adds
/// `extra_attributes`, defines `toolchains`, and adds custom exec groups from `exec_groups`.
/// Depending on `action` parameter, `actions.run` or `actions.run_shell` is created. This rule
/// also defines an aspect on its `deps` attribute.
// Java: AspectAutoExecGroupsTest.createCustomRule
fn create_custom_rule(
    t: &mut BuildViewTestCase,
    action: &str,
    action_parameters: &str,
    extra_attributes: &str,
    toolchains: &str,
    exec_groups: &str,
) {
    let action_call = format!("  {action}(");
    let toolchains_line = format!("  toolchains = {toolchains},");
    t.scratch.file(
        "test/defs.bzl",
        &[
            "load('//test:aspect.bzl', 'custom_aspect')",
            "def _impl(ctx):",
            "  output_jar = ctx.actions.declare_file(ctx.label.name + '_dummy_output.jar')",
            &action_call,
            action_parameters,
            "    outputs = [output_jar],",
            if action == "ctx.actions.run" {
                if action_parameters.contains("executable =") {
                    // avoid adding executable parameter twice
                    ""
                } else {
                    "executable = ctx.toolchains['//rule:toolchain_type_1'].tool,"
                }
            } else {
                "    command = 'echo',"
            },
            "  )",
            "  return [DefaultInfo(files = depset([output_jar]))]",
            "custom_rule = rule(",
            "  implementation = _impl,",
            "  attrs = {",
            "    '_tool': attr.label(default = '//toolchain:a_tool', cfg = 'exec', executable = True),",
            "    'deps': attr.label_list(aspects = [custom_aspect]),",
            extra_attributes,
            "  },",
            "  exec_groups = {",
            exec_groups,
            "  },",
            &toolchains_line,
            ")",
        ],
    );
    t.scratch.file(
        "test/BUILD",
        &[r##"load("//test:defs.bzl", "custom_rule")

custom_rule(name = "custom_rule_dep")

custom_rule(
    name = "custom_rule_name",
    deps = ["custom_rule_dep"],
)
"##],
    );
}

/// The `test/aspect.bzl` lines of `createCustomAspect` and `createCustomAspectAndEmptyRule`
/// (written out in each Java method).
fn custom_aspect_lines(
    t: &mut BuildViewTestCase,
    action: &str,
    action_parameters: &str,
    extra_attributes: &str,
    toolchains: &str,
    exec_groups: &str,
) {
    let action_call = format!("  {action}(");
    let toolchains_line = format!("  toolchains = {toolchains},");
    t.scratch.file(
        "test/aspect.bzl",
        &[
            "OutputFile = provider(fields = {'file': 'Output file', 'exec_groups': 'Exec groups'})",
            "def _impl(target, ctx):",
            "  output_jar = ctx.actions.declare_file(ctx.label.name + '_dummy_output_aspect.jar')",
            &action_call,
            action_parameters,
            "    outputs = [output_jar],",
            if action == "ctx.actions.run" {
                "    executable = '//toolchain:foo_toolchain',"
            } else {
                "    command = 'echo',"
            },
            "  )",
            "  exec_groups = ctx.exec_groups",
            "  return [OutputFile(file = output_jar, exec_groups = exec_groups)]",
            "custom_aspect = aspect(",
            "  implementation = _impl,",
            "  attrs = {",
            "     ",
            extra_attributes,
            "  },",
            "  attr_aspects = ['deps'],",
            "  exec_groups = {",
            exec_groups,
            "  },",
            &toolchains_line,
            ")",
        ],
    );
}

/// Creates custom aspect which produces action with `action_parameters`, adds
/// `extra_attributes`, defines `toolchains`, and adds custom exec groups from `exec_groups`.
/// Depending on `action` parameter, `actions.run` or `actions.run_shell` is created.
// Java: AspectAutoExecGroupsTest.createCustomAspect
fn create_custom_aspect(
    t: &mut BuildViewTestCase,
    action: &str,
    action_parameters: &str,
    extra_attributes: &str,
    toolchains: &str,
    exec_groups: &str,
) {
    custom_aspect_lines(
        t,
        action,
        action_parameters,
        extra_attributes,
        toolchains,
        exec_groups,
    );
}

/// Creates empty rule and custom aspect on rule's dependencies. Custom aspect produces action
/// with `action_parameters`, adds `extra_attributes`, defines `toolchains`, and adds custom exec
/// groups from `exec_groups`. Depending on `action` parameter, `actions.run` or
/// `actions.run_shell` is created. This function is used only for testing the aspect, not the
/// rule.
// Java: AspectAutoExecGroupsTest.createCustomAspectAndEmptyRule
fn create_custom_aspect_and_empty_rule(
    t: &mut BuildViewTestCase,
    action: &str,
    action_parameters: &str,
    extra_attributes: &str,
    toolchains: &str,
    exec_groups: &str,
) {
    custom_aspect_lines(
        t,
        action,
        action_parameters,
        extra_attributes,
        toolchains,
        exec_groups,
    );
    t.scratch.file(
        "test/defs.bzl",
        &[r##"load("//test:aspect.bzl", "custom_aspect")

def _impl(ctx):
    return []

custom_rule = rule(
    implementation = _impl,
    attrs = {
        "deps": attr.label_list(aspects = [custom_aspect]),
    },
)
"##],
    );
    t.scratch.file(
        "test/BUILD",
        &[r##"load("//test:defs.bzl", "custom_rule")

custom_rule(name = "custom_rule_dep")

custom_rule(
    name = "custom_rule_name",
    deps = ["custom_rule_dep"],
)
"##],
    );
}

/// `BuildViewTestCase.getAspect(label)`: the only done aspect node whose aspect name is `label`.
///
/// PORT: the harness has no `getAspect`; written here over the evaluator's done nodes.
// Java: com/google/devtools/build/lib/analysis/util/BuildViewTestCase.java:1205 BuildViewTestCase.getAspect
fn get_aspect(t: &mut BuildViewTestCase, label: &str) -> Arc<AspectValue> {
    let ev = t.get_evaluator();
    let values: Vec<Arc<AspectValue>> = ev
        .get_all_keys_for_testing()
        .into_iter()
        .filter(|k| {
            k.downcast::<AspectKey>()
                .is_some_and(|a| a.get_aspect_name() == label)
        })
        .filter_map(|k| ev.get_existing_value(&k))
        .map(|v| {
            v.downcast::<AspectValue>()
                .unwrap_or_else(|_| panic!("ClassCastException: AspectValue"))
        })
        .collect();
    assert_eq!(
        values.len(),
        1,
        "onlyElement: {} aspect values named {label}",
        values.len()
    );
    values.into_iter().next().unwrap()
}

/// `info` as a value usable with a temporary heap.
fn frozen<'v>(info: &OwnedFrozenValue) -> Value<'v> {
    // SAFETY: the value is used only while `info` (which keeps its frozen heap alive) is
    // borrowed.
    unsafe { info.unchecked_frozen_value() }.to_value()
}

/// `getExecGroupsFromAspectProvider(configuredAspect).getToolchainCollectionForTesting().keySet()`.
// Java: AspectAutoExecGroupsTest.getExecGroupsFromAspectProvider
fn get_exec_groups_from_aspect_provider(configured_aspect: &AspectValue) -> Vec<String> {
    let key = ProviderKey::Starlark {
        extension_label: parse_label("//test:aspect.bzl"),
        exported_name: "OutputFile".to_string(),
    };
    let key_info = configured_aspect
        .get_providers()
        .get(&key)
        .expect("OutputFile")
        .clone();
    Heap::temp(|heap| {
        let exec_groups = frozen(&key_info)
            .get_attr("exec_groups", heap)
            .unwrap_or_else(|e| panic!("{e}"))
            .expect("exec_groups");
        match exec_groups
            .downcast_ref::<StarlarkExecGroupCollection>()
            .unwrap_or_else(|| panic!("ClassCastException: {exec_groups}"))
        {
            // Java: StarlarkExecGroupCollection.getToolchainCollectionForTesting()
            // (toolchainCollection().contextMap()).
            StarlarkExecGroupCollection::Valid {
                exec_group_names, ..
            } => exec_group_names.clone(),
            StarlarkExecGroupCollection::NotValid => {
                panic!("EXEC_GROUP_COLLECTION_NOT_VALID has no toolchain collection")
            }
        }
    })
}

/// The Java test's `action` parameter (`ctx.actions.run` or `ctx.actions.run_shell`).
fn action_of(run_shell: bool) -> &'static str {
    if run_shell {
        "ctx.actions.run_shell"
    } else {
        "ctx.actions.run"
    }
}

// Java: AspectAutoExecGroupsTest.automaticExecutionGroups_disabledAndAttributeFalse_disabled
fn automatic_execution_groups_disabled_and_attribute_false_disabled(action: &str) {
    let mut t = new_test();
    create_custom_aspect_and_empty_rule(
        &mut t,
        /* action= */ action,
        /* action_parameters= */ "toolchain = '//rule:toolchain_type_1',",
        /* extra_attributes= */ "'_use_auto_exec_groups': attr.bool(default = False),",
        /* toolchains= */ "['//rule:toolchain_type_1']",
        /* exec_groups= */ "",
    );

    t.get_configured_target("//test:custom_rule_name");
    let configured_aspect = get_aspect(&mut t, "//test:aspect.bzl%custom_aspect");
    let aspect_exec_groups = get_exec_groups_from_aspect_provider(&configured_aspect);

    assert_contains_exactly(&aspect_exec_groups, &["default-exec-group"]);
}

#[test]
fn automatic_execution_groups_disabled_and_attribute_false_disabled_run() {
    automatic_execution_groups_disabled_and_attribute_false_disabled(action_of(false));
}

#[test]
fn automatic_execution_groups_disabled_and_attribute_false_disabled_run_shell() {
    automatic_execution_groups_disabled_and_attribute_false_disabled(action_of(true));
}

// Java: AspectAutoExecGroupsTest.automaticExecutionGroups_disabledAndAttributeTrue_enabled
fn automatic_execution_groups_disabled_and_attribute_true_enabled(action: &str) {
    let mut t = new_test();
    create_custom_aspect_and_empty_rule(
        &mut t,
        /* action= */ action,
        /* action_parameters= */ "toolchain = '//rule:toolchain_type_1',",
        /* extra_attributes= */ "'_use_auto_exec_groups': attr.bool(default = True),",
        /* toolchains= */ "['//rule:toolchain_type_1']",
        /* exec_groups= */ "",
    );

    t.get_configured_target("//test:custom_rule_name");
    let configured_aspect = get_aspect(&mut t, "//test:aspect.bzl%custom_aspect");
    let aspect_exec_groups = get_exec_groups_from_aspect_provider(&configured_aspect);

    assert_contains_exactly(
        &aspect_exec_groups,
        &["default-exec-group", "//rule:toolchain_type_1"],
    );
}

#[test]
fn automatic_execution_groups_disabled_and_attribute_true_enabled_run() {
    automatic_execution_groups_disabled_and_attribute_true_enabled(action_of(false));
}

#[test]
fn automatic_execution_groups_disabled_and_attribute_true_enabled_run_shell() {
    automatic_execution_groups_disabled_and_attribute_true_enabled(action_of(true));
}

// Java: AspectAutoExecGroupsTest.automaticExecutionGroups_disabledAndAttributeNotSet_disabled
fn automatic_execution_groups_disabled_and_attribute_not_set_disabled(action: &str) {
    let mut t = new_test();
    create_custom_aspect_and_empty_rule(
        &mut t,
        /* action= */ action,
        /* action_parameters= */ "toolchain = '//rule:toolchain_type_1',",
        /* extra_attributes= */ "",
        /* toolchains= */ "['//rule:toolchain_type_1']",
        /* exec_groups= */ "",
    );
    use_configuration(&mut t, &["--incompatible_auto_exec_groups=False"]);

    t.get_configured_target("//test:custom_rule_name");
    let configured_aspect = get_aspect(&mut t, "//test:aspect.bzl%custom_aspect");
    let aspect_exec_groups = get_exec_groups_from_aspect_provider(&configured_aspect);

    assert_contains_exactly(&aspect_exec_groups, &["default-exec-group"]);
}

#[test]
fn automatic_execution_groups_disabled_and_attribute_not_set_disabled_run() {
    automatic_execution_groups_disabled_and_attribute_not_set_disabled(action_of(false));
}

#[test]
fn automatic_execution_groups_disabled_and_attribute_not_set_disabled_run_shell() {
    automatic_execution_groups_disabled_and_attribute_not_set_disabled(action_of(true));
}

// Java: AspectAutoExecGroupsTest.automaticExecutionGroups_enabledAndAttributeFalse_disabled
fn automatic_execution_groups_enabled_and_attribute_false_disabled(action: &str) {
    let mut t = new_test();
    create_custom_aspect_and_empty_rule(
        &mut t,
        /* action= */ action,
        /* action_parameters= */ "toolchain = '//rule:toolchain_type_1',",
        /* extra_attributes= */ "'_use_auto_exec_groups': attr.bool(default = False),",
        /* toolchains= */ "['//rule:toolchain_type_1']",
        /* exec_groups= */ "",
    );
    use_configuration(&mut t, &["--incompatible_auto_exec_groups"]);

    t.get_configured_target("//test:custom_rule_name");
    let configured_aspect = get_aspect(&mut t, "//test:aspect.bzl%custom_aspect");
    let aspect_exec_groups = get_exec_groups_from_aspect_provider(&configured_aspect);

    assert_contains_exactly(&aspect_exec_groups, &["default-exec-group"]);
}

#[test]
fn automatic_execution_groups_enabled_and_attribute_false_disabled_run() {
    automatic_execution_groups_enabled_and_attribute_false_disabled(action_of(false));
}

#[test]
fn automatic_execution_groups_enabled_and_attribute_false_disabled_run_shell() {
    automatic_execution_groups_enabled_and_attribute_false_disabled(action_of(true));
}

// Java: AspectAutoExecGroupsTest.automaticExecutionGroups_enabledAndAttributeTrue_enabled
fn automatic_execution_groups_enabled_and_attribute_true_enabled(action: &str) {
    let mut t = new_test();
    create_custom_aspect_and_empty_rule(
        &mut t,
        /* action= */ action,
        /* action_parameters= */ "toolchain = '//rule:toolchain_type_1',",
        /* extra_attributes= */ "'_use_auto_exec_groups': attr.bool(default = True),",
        /* toolchains= */ "['//rule:toolchain_type_1']",
        /* exec_groups= */ "",
    );
    use_configuration(&mut t, &["--incompatible_auto_exec_groups"]);

    t.get_configured_target("//test:custom_rule_name");
    let configured_aspect = get_aspect(&mut t, "//test:aspect.bzl%custom_aspect");
    let aspect_exec_groups = get_exec_groups_from_aspect_provider(&configured_aspect);

    assert_contains_exactly(
        &aspect_exec_groups,
        &["default-exec-group", "//rule:toolchain_type_1"],
    );
}

#[test]
fn automatic_execution_groups_enabled_and_attribute_true_enabled_run() {
    automatic_execution_groups_enabled_and_attribute_true_enabled(action_of(false));
}

#[test]
fn automatic_execution_groups_enabled_and_attribute_true_enabled_run_shell() {
    automatic_execution_groups_enabled_and_attribute_true_enabled(action_of(true));
}

// Java: AspectAutoExecGroupsTest.automaticExecutionGroups_enabledAndAttributeNotSet_enabled
fn automatic_execution_groups_enabled_and_attribute_not_set_enabled(action: &str) {
    let mut t = new_test();
    create_custom_aspect_and_empty_rule(
        &mut t,
        /* action= */ action,
        /* action_parameters= */ "toolchain = '//rule:toolchain_type_1',",
        /* extra_attributes= */ "",
        /* toolchains= */ "['//rule:toolchain_type_1']",
        /* exec_groups= */ "",
    );
    use_configuration(&mut t, &["--incompatible_auto_exec_groups"]);

    t.get_configured_target("//test:custom_rule_name");
    let configured_aspect = get_aspect(&mut t, "//test:aspect.bzl%custom_aspect");
    let aspect_exec_groups = get_exec_groups_from_aspect_provider(&configured_aspect);

    assert_contains_exactly(
        &aspect_exec_groups,
        &["default-exec-group", "//rule:toolchain_type_1"],
    );
}

#[test]
fn automatic_execution_groups_enabled_and_attribute_not_set_enabled_run() {
    automatic_execution_groups_enabled_and_attribute_not_set_enabled(action_of(false));
}

#[test]
fn automatic_execution_groups_enabled_and_attribute_not_set_enabled_run_shell() {
    automatic_execution_groups_enabled_and_attribute_not_set_enabled(action_of(true));
}

const CUSTOM_EXEC_GROUPS: &str = "    'aspect_custom_exec_group': exec_group(\n      exec_compatible_with = ['//platforms:constraint_1'],\n      toolchains = ['//rule:toolchain_type_1'],\n    ),\n";

// Java: AspectAutoExecGroupsTest.execGroups_customAspectOnCustomRule
fn exec_groups_custom_aspect_on_custom_rule(action: &str) {
    let custom_exec_groups = CUSTOM_EXEC_GROUPS;
    let mut t = new_test();
    create_custom_aspect(
        &mut t,
        /* action= */ action,
        /* action_parameters= */ "toolchain = '//rule:toolchain_type_2',",
        /* extra_attributes= */ "",
        /* toolchains= */ "['//rule:toolchain_type_2']",
        /* exec_groups= */ custom_exec_groups,
    );
    create_custom_rule(
        &mut t,
        /* action= */ action,
        /* action_parameters= */ "toolchain = '//rule:toolchain_type_1',",
        /* extra_attributes= */ "",
        /* toolchains= */ "['//rule:toolchain_type_1']",
        /* exec_groups= */ "",
    );
    use_configuration(&mut t, &["--incompatible_auto_exec_groups"]);

    let target = t
        .get_configured_target("//test:custom_rule_name")
        .expect("//test:custom_rule_name");
    let target_dep = t
        .get_rule_context(&target)
        .get_prerequisite("deps")
        .expect("getPrerequisite(deps)");
    let configured_aspect = get_aspect(&mut t, "//test:aspect.bzl%custom_aspect");
    let aspect_exec_groups = get_exec_groups_from_aspect_provider(&configured_aspect);

    let exec_group_names = |t: &mut crate::testing::AnalysisTestCase,
                            ct: &blaze_analysis::configured_target::ConfiguredTarget|
     -> Vec<String> {
        t.get_rule_context(ct)
            .get_exec_groups()
            .exec_groups()
            .keys()
            .cloned()
            .collect()
    };
    assert_contains_exactly(
        &exec_group_names(&mut t, &target),
        &["//rule:toolchain_type_1"],
    );
    assert_contains_exactly(
        &exec_group_names(&mut t, &target_dep),
        &["//rule:toolchain_type_1"],
    );
    assert_contains_exactly(
        &aspect_exec_groups,
        &[
            "//rule:toolchain_type_2",
            "default-exec-group",
            "aspect_custom_exec_group",
        ],
    );
}

#[test]
fn exec_groups_custom_aspect_on_custom_rule_run() {
    exec_groups_custom_aspect_on_custom_rule(action_of(false));
}

#[test]
fn exec_groups_custom_aspect_on_custom_rule_run_shell() {
    exec_groups_custom_aspect_on_custom_rule(action_of(true));
}

// Java: AspectAutoExecGroupsTest.execPlatforms_customAspectOnCustomRule
fn exec_platforms_custom_aspect_on_custom_rule(action: &str) {
    let custom_exec_groups = CUSTOM_EXEC_GROUPS;
    let mut t = new_test();
    create_custom_aspect(
        &mut t,
        /* action= */ action,
        /* action_parameters= */ "toolchain = '//rule:toolchain_type_2',",
        /* extra_attributes= */ "",
        /* toolchains= */ "['//rule:toolchain_type_2']",
        /* exec_groups= */ custom_exec_groups,
    );
    create_custom_rule(
        &mut t,
        /* action= */ action,
        /* action_parameters= */ "toolchain = '//rule:toolchain_type_1',",
        /* extra_attributes= */ "",
        /* toolchains= */ "['//rule:toolchain_type_1']",
        /* exec_groups= */ "",
    );
    use_configuration(&mut t, &["--incompatible_auto_exec_groups"]);

    let target = t
        .get_configured_target("//test:custom_rule_name")
        .expect("//test:custom_rule_name");
    let target_dep = t
        .get_rule_context(&target)
        .get_prerequisite("deps")
        .expect("getPrerequisite(deps)");
    let configured_aspect = get_aspect(&mut t, "//test:aspect.bzl%custom_aspect");
    let target_action = t
        .get_generating_action_for_output(&target, "test/custom_rule_name_dummy_output.jar")
        .expect("generating action");
    let target_dep_action = t
        .get_generating_action_for_output(&target_dep, "test/custom_rule_dep_dummy_output.jar")
        .expect("generating action");
    let aspect_action = configured_aspect.get_actions()[0].clone();

    assert_eq!(
        target_action
            .get_owner()
            .get_execution_platform()
            .expect("execution platform")
            .label(),
        &parse_label("//platforms:platform_1")
    );
    assert_eq!(
        target_dep_action
            .get_owner()
            .get_execution_platform()
            .expect("execution platform")
            .label(),
        &parse_label("//platforms:platform_1")
    );
    assert_eq!(
        aspect_action
            .get_owner()
            .get_execution_platform()
            .expect("execution platform")
            .label(),
        &parse_label("//platforms:platform_2")
    );
}

#[test]
fn exec_platforms_custom_aspect_on_custom_rule_run() {
    exec_platforms_custom_aspect_on_custom_rule(action_of(false));
}

#[test]
fn exec_platforms_custom_aspect_on_custom_rule_run_shell() {
    exec_platforms_custom_aspect_on_custom_rule(action_of(true));
}
