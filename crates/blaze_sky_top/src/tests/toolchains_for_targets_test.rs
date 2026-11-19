//! Port of
//! `src/test/java/com/google/devtools/build/lib/skyframe/toolchains/ToolchainsForTargetsTest.java`
//! (an `AnalysisTestCase`): toolchain contexts computed for targets by
//! `DependencyResolver.getDependencyContext`.
//!
//! Java injects a test SkyFunction (`CONFIGURED_TARGET_FUNCTION_COMPUTE_UNLOADED_TOOLCHAIN_CONTEXTS`,
//! through `AnalysisMock.Delegate.getSkyFunctions`) that returns the
//! `ToolchainCollection<UnloadedToolchainContext>` of a target; the harness cannot register extra
//! SkyFunctions in its `SkyframeExecutor`, so the tests that read it (`getToolchainCollection`) run
//! up to that call and stay ignored, with the Java assertions after it kept as comments. The tests
//! that only call `update` run. `updateExplicitTarget` needs `explicitTargetPatterns`, which the
//! harness's `update_with` does not take.

use blaze_analysis::configured_target::ConfiguredTarget;
use blaze_analysis::configured_target_key::ConfiguredTargetKey;

use crate::testing::{AnalysisTestCase, BuildViewTestCase, has_error};
use crate::tests::toolchain_test_case::label;

const NEEDS_FUNCTION: &str = "needs DependencyResolver.getDependencyContext and \
                              DependencyResolver.State.createForTesting, private to \
                              blaze_sky_analysis (ComputeUnloadedToolchainContextsFunction)";

const NEEDS_EXPLICIT_TARGET_PATTERNS: &str =
    "needs AnalysisTestCase.update with explicitTargetPatterns in the harness";

/// A fresh `AnalysisTestCase` after the class's `@Before createToolchains()`.
fn new_test() -> AnalysisTestCase {
    let mut t = BuildViewTestCase::analysis_test_case();
    create_toolchains(&mut t);
    t
}

// Java: ToolchainsForTargetsTest.createToolchains
fn create_toolchains(t: &mut AnalysisTestCase) {
    t.scratch
        .append_file("MODULE.bazel", &["register_toolchains('//toolchains:all')"]);

    t.scratch.file(
        "toolchain/toolchain_def.bzl",
        &[r#"def _impl(ctx):
    toolchain = platform_common.ToolchainInfo(
        data = ctx.attr.data,
    )
    return [toolchain]

test_toolchain = rule(
    implementation = _impl,
    attrs = {
        "data": attr.string(),
    },
)
"#],
    );

    t.scratch.file(
        "toolchain/BUILD",
        &["toolchain_type(name = 'test_toolchain')"],
    );

    t.scratch.append_file(
        "toolchains/BUILD",
        &[r#"load("//toolchain:toolchain_def.bzl", "test_toolchain")

toolchain(
    name = "toolchain_1",
    exec_compatible_with = [],
    target_compatible_with = [],
    toolchain = ":toolchain_1_impl",
    toolchain_type = "//toolchain:test_toolchain",
)

test_toolchain(
    name = "toolchain_1_impl",
    data = "foo",
)

toolchain(
    name = "toolchain_2",
    exec_compatible_with = [],
    target_compatible_with = [],
    toolchain = ":toolchain_2_impl",
    toolchain_type = "//toolchain:test_toolchain",
)

test_toolchain(
    name = "toolchain_2_impl",
    data = "bar",
)
"#],
    );

    t.scratch.append_file(
        "toolchain/rule.bzl",
        &[r#"def _impl(ctx):
    data = ctx.toolchains["//toolchain:test_toolchain"].data
    return [
        coverage_common.instrumented_files_info(
            ctx,
            source_attributes = ["srcs"],
        )
    ]

my_rule = rule(
    implementation = _impl,
    attrs = {
        "srcs": attr.label_list(allow_files = True),
    },
    toolchains = ["//toolchain:test_toolchain"],
)
"#],
    );
}

/// `getToolchainCollection(configuredTarget, configuredTargetKey)`.
// Java: ToolchainsForTargetsTest.getToolchainCollection
fn get_toolchain_collection_for(
    t: &mut AnalysisTestCase,
    configured_target: &ConfiguredTarget,
    _configured_target_key: ConfiguredTargetKey,
) -> ! {
    let target_label = configured_target.get_original_label().to_string();
    let _target = t.get_target(&target_label);
    let _configuration = t.get_configuration(configured_target);
    panic!("{NEEDS_FUNCTION}")
}

/// `getToolchainCollection(targetLabel)`.
// Java: ToolchainsForTargetsTest.getToolchainCollection
fn get_toolchain_collection(t: &mut AnalysisTestCase, target_label: &str) -> ! {
    let target = only_target_to_build(t, &[target_label]);
    let key = ConfiguredTargetKey::builder()
        .set_label(target.get_original_label())
        .set_configuration_key(target.get_configuration_key())
        .build();
    get_toolchain_collection_for(t, &target, key)
}

/// `Iterables.getOnlyElement(update(labels).getTargetsToBuild())`.
fn only_target_to_build(t: &mut AnalysisTestCase, labels: &[&str]) -> ConfiguredTarget {
    let result = t.update(labels).unwrap_or_else(|e| panic!("{e}"));
    let targets = result.get_targets_to_build();
    assert_eq!(targets.len(), 1, "getOnlyElement");
    targets[0].clone()
}

/// `updateExplicitTarget(label)`.
// Java: ToolchainsForTargetsTest.updateExplicitTarget
fn update_explicit_target(_t: &mut AnalysisTestCase, _label: &str) -> ! {
    // Java: update(new EventBus(), defaultFlags().with(Flag.KEEP_GOING),
    //     /* explicitTargetPatterns= */ ImmutableSet.of(Label.parseCanonicalUnchecked(label)),
    //     /* aspects= */ ImmutableList.of(), /* aspectsParameters= */ ImmutableMap.of(), label);
    panic!("{NEEDS_EXPLICIT_TARGET_PATTERNS}")
}

// Java: ToolchainsForTargetsTest.basicToolchains
#[test]
#[ignore = "the test SkyFunction (ComputeUnloadedToolchainContextsFunction) can be registered \
            (BuildViewTestCase::set_extra_sky_functions) but calls DependencyResolver.getDependencyContext \
            with a DependencyResolver.State.createForTesting, both private to blaze_sky_analysis \
            (dependency_resolver.rs get_dependency_context, DependencyResolverState)"]
fn basic_toolchains() {
    let mut t = new_test();
    t.scratch.file(
        "a/BUILD",
        &[r#"load("//toolchain:rule.bzl", "my_rule")

my_rule(name = "a")
"#],
    );

    get_toolchain_collection(&mut t, "//a");
    // assertThat(toolchainCollection).isNotNull();
    // assertThat(toolchainCollection).hasDefaultExecGroup();
    // assertThat(toolchainCollection).defaultToolchainContext()
    //     .hasToolchainType("//toolchain:test_toolchain");
    // assertThat(toolchainCollection).defaultToolchainContext()
    //     .hasResolvedToolchain("//toolchains:toolchain_1_impl");
    // Label toolchainType = Label.parseCanonicalUnchecked("//toolchain:test_toolchain");
    // assertThat(toolchainCollection).defaultToolchainContext().toolchainTypes()
    //     .containsExactly(toolchainType, ToolchainTypeRequirement.create(toolchainType));
}

const ALIAS_TOOLCHAIN_TYPE_BUILD: &str = r#"alias(
    name = "alias_toolchain_type",
    actual = "//toolchain:test_toolchain",
)
"#;

const ALIAS_CUSTOM_RULE_DEFS: &str = r#"def _impl(ctx):
    print(ctx.toolchains["//test/alias:alias_toolchain_type"])
    print(ctx.toolchains["//toolchain:test_toolchain"])
    return []

custom_rule = rule(
    implementation = _impl,
    toolchains = ["//test/alias:alias_toolchain_type"],
)
"#;

const ALIAS_CUSTOM_RULE_BUILD: &str = r#"load("//test:defs.bzl", "custom_rule")

custom_rule(
    name = "custom_rule_name",
)
"#;

// Java: ToolchainsForTargetsTest.basicToolchainsWithAliasAutoExecGroups
#[test]
fn basic_toolchains_with_alias_auto_exec_groups() {
    let mut t = new_test();
    t.scratch
        .file("test/alias/BUILD", &[ALIAS_TOOLCHAIN_TYPE_BUILD]);
    t.scratch.file("test/defs.bzl", &[ALIAS_CUSTOM_RULE_DEFS]);
    t.scratch.file("test/BUILD", &[ALIAS_CUSTOM_RULE_BUILD]);
    t.use_configuration(&["--incompatible_auto_exec_groups"]);

    let result = t
        .update(&["//test:custom_rule_name"])
        .unwrap_or_else(|e| panic!("{e}"));
    assert!(!has_error(result));
}

// Java: ToolchainsForTargetsTest.basicToolchainsWithAliasNoAutoExecGroups
#[test]
fn basic_toolchains_with_alias_no_auto_exec_groups() {
    let mut t = new_test();
    t.scratch
        .file("test/alias/BUILD", &[ALIAS_TOOLCHAIN_TYPE_BUILD]);
    t.scratch.file("test/defs.bzl", &[ALIAS_CUSTOM_RULE_DEFS]);
    t.scratch.file("test/BUILD", &[ALIAS_CUSTOM_RULE_BUILD]);
    t.use_configuration(&["--noincompatible_auto_exec_groups"]);

    let result = t
        .update(&["//test:custom_rule_name"])
        .unwrap_or_else(|e| panic!("{e}"));
    assert!(!has_error(result));
}

// Java: ToolchainsForTargetsTest.basicToolchainsWithAliasNoAutoExecGroups_test
#[test]
fn basic_toolchains_with_alias_no_auto_exec_groups_test() {
    let mut t = new_test();
    t.scratch.append_file(
        "toolchain/exec_group_rule.bzl",
        &[r#"def _impl(ctx):
    if "//toolchain:test_toolchain" in ctx.toolchains:
        fail("this is not expected, it's an exec gp toolchain")
    if ctx.exec_groups["temp"].toolchains["//toolchain:test_toolchain"] == None:
        fail("this is not expected, it's an exec gp toolchain")
    return []

my_exec_group_rule = rule(
    implementation = _impl,
    exec_groups = {
        "temp": exec_group(
            toolchains = ["//toolchain:test_toolchain"],
        ),
    },
)
"#],
    );

    t.scratch.file(
        "a/BUILD",
        &[
            r#"load("//toolchain:exec_group_rule.bzl", "my_exec_group_rule")

my_exec_group_rule(name = "a")
"#,
        ],
    );

    t.use_configuration(&["--incompatible_auto_exec_groups"]);

    let result = t.update(&["//a:a"]).unwrap_or_else(|e| panic!("{e}"));
    assert!(!has_error(result));
}

// Java: ToolchainsForTargetsTest.execPlatform
#[test]
#[ignore = "the test SkyFunction (ComputeUnloadedToolchainContextsFunction) can be registered \
            (BuildViewTestCase::set_extra_sky_functions) but calls DependencyResolver.getDependencyContext \
            with a DependencyResolver.State.createForTesting, both private to blaze_sky_analysis \
            (dependency_resolver.rs get_dependency_context, DependencyResolverState)"]
fn exec_platform() {
    let mut t = new_test();
    // Add some platforms and custom constraints.
    t.scratch
        .file("platforms/BUILD", &["platform(name = 'local_platform_a')"]);

    // Test normal resolution, and with a per-target exec constraint.
    t.scratch.file(
        "a/BUILD",
        &[r#"load("//toolchain:rule.bzl", "my_rule")

my_rule(name = "a")
"#],
    );

    t.use_configuration(&["--extra_execution_platforms=//platforms:local_platform_a"]);

    get_toolchain_collection(&mut t, "//a");
    // assertThat(toolchainCollection).isNotNull();
    // assertThat(toolchainCollection).hasDefaultExecGroup();
    // assertThat(toolchainCollection).defaultToolchainContext()
    //     // First execution platform will be used.
    //     .hasExecutionPlatform("//platforms:local_platform_a");
}

const LOCAL_PLATFORMS_BUILD: &str = r#"constraint_setting(name = "local_setting")

constraint_value(
    name = "local_value_a",
    constraint_setting = ":local_setting",
)

constraint_value(
    name = "local_value_b",
    constraint_setting = ":local_setting",
)

platform(
    name = "local_platform_a",
    constraint_values = [":local_value_a"],
)

platform(
    name = "local_platform_b",
    constraint_values = [":local_value_b"],
)
"#;

// Java: ToolchainsForTargetsTest.execPlatform_withExecConstraint
#[test]
#[ignore = "the test SkyFunction (ComputeUnloadedToolchainContextsFunction) can be registered \
            (BuildViewTestCase::set_extra_sky_functions) but calls DependencyResolver.getDependencyContext \
            with a DependencyResolver.State.createForTesting, both private to blaze_sky_analysis \
            (dependency_resolver.rs get_dependency_context, DependencyResolverState)"]
fn exec_platform_with_exec_constraint() {
    let mut t = new_test();
    // Add some platforms and custom constraints.
    t.scratch.file("platforms/BUILD", &[LOCAL_PLATFORMS_BUILD]);

    // Test normal resolution, and with a per-target exec constraint.
    t.scratch.file(
        "a/BUILD",
        &[r#"load("//toolchain:rule.bzl", "my_rule")

my_rule(
    name = "a",
    exec_compatible_with = ["//platforms:local_value_b"],
)
"#],
    );

    t.use_configuration(&[
        "--extra_execution_platforms=//platforms:local_platform_a,//platforms:local_platform_b",
    ]);

    get_toolchain_collection(&mut t, "//a");
    // assertThat(toolchainCollection).isNotNull();
    // assertThat(toolchainCollection).hasDefaultExecGroup();
    // assertThat(toolchainCollection).defaultToolchainContext()
    //     // Exec constraint forces the use of this exec platform.
    //     .hasExecutionPlatform("//platforms:local_platform_b");
}

// Java: ToolchainsForTargetsTest.execGroups_named
#[test]
#[ignore = "the test SkyFunction (ComputeUnloadedToolchainContextsFunction) can be registered \
            (BuildViewTestCase::set_extra_sky_functions) but calls DependencyResolver.getDependencyContext \
            with a DependencyResolver.State.createForTesting, both private to blaze_sky_analysis \
            (dependency_resolver.rs get_dependency_context, DependencyResolverState)"]
fn exec_groups_named() {
    let mut t = new_test();
    // Write a rule with exec groups.
    t.scratch.append_file(
        "toolchain/exec_group_rule.bzl",
        &[r#"def _impl(ctx):
    pass

my_exec_group_rule = rule(
    implementation = _impl,
    exec_groups = {
        "temp": exec_group(
            toolchains = ["//toolchain:test_toolchain"],
        ),
    },
)
"#],
    );

    t.scratch.file(
        "a/BUILD",
        &[
            r#"load("//toolchain:exec_group_rule.bzl", "my_exec_group_rule")

my_exec_group_rule(name = "a")
"#,
        ],
    );

    get_toolchain_collection(&mut t, "//a");
    // assertThat(toolchainCollection).isNotNull();
    // assertThat(toolchainCollection).hasDefaultExecGroup();
    // assertThat(toolchainCollection).defaultToolchainContext().toolchainTypes().isEmpty();
    // assertThat(toolchainCollection).defaultToolchainContext().resolvedToolchainLabels().isEmpty();
    // assertThat(toolchainCollection).hasExecGroup("temp");
    // assertThat(toolchainCollection).execGroup("temp")
    //     .hasToolchainType("//toolchain:test_toolchain");
    // assertThat(toolchainCollection).execGroup("temp")
    //     .hasResolvedToolchain("//toolchains:toolchain_1_impl");
    // assertThat(toolchainCollection).execGroup("temp")
    //     .hasToolchainType("//toolchain:test_toolchain");
    // assertThat(toolchainCollection).execGroup("temp")
    //     .hasResolvedToolchain("//toolchains:toolchain_1_impl");
}

// Java: ToolchainsForTargetsTest.execGroups_defaultAndNamed
#[test]
#[ignore = "the test SkyFunction (ComputeUnloadedToolchainContextsFunction) can be registered \
            (BuildViewTestCase::set_extra_sky_functions) but calls DependencyResolver.getDependencyContext \
            with a DependencyResolver.State.createForTesting, both private to blaze_sky_analysis \
            (dependency_resolver.rs get_dependency_context, DependencyResolverState)"]
fn exec_groups_default_and_named() {
    let mut t = new_test();
    // Add another toolchain type.
    t.scratch.append_file(
        "extra/BUILD",
        &[r#"load("//toolchain:toolchain_def.bzl", "test_toolchain")

toolchain_type(name = "extra_toolchain")

toolchain(
    name = "toolchain",
    exec_compatible_with = [],
    target_compatible_with = [],
    toolchain = ":toolchain_impl",
    toolchain_type = ":extra_toolchain",
)

test_toolchain(
    name = "toolchain_impl",
    data = "foo",
)
"#],
    );

    // Write a rule with exec groups.
    t.scratch.append_file(
        "toolchain/exec_group_rule.bzl",
        &[r#"def _impl(ctx):
    pass

my_exec_group_rule = rule(
    implementation = _impl,
    toolchains = ["//extra:extra_toolchain"],
    exec_groups = {
        "temp": exec_group(
            toolchains = ["//toolchain:test_toolchain"],
        ),
    },
)
"#],
    );

    t.scratch.file(
        "a/BUILD",
        &[
            r#"load("//toolchain:exec_group_rule.bzl", "my_exec_group_rule")

my_exec_group_rule(name = "a")
"#,
        ],
    );

    t.use_configuration(&["--extra_toolchains=//extra:toolchain"]);
    get_toolchain_collection(&mut t, "//a");
    // assertThat(toolchainCollection).isNotNull();
    // assertThat(toolchainCollection).hasDefaultExecGroup();
    // assertThat(toolchainCollection).defaultToolchainContext()
    //     .hasToolchainType("//extra:extra_toolchain");
    // assertThat(toolchainCollection).defaultToolchainContext()
    //     .hasResolvedToolchain("//extra:toolchain_impl");
    // assertThat(toolchainCollection).hasExecGroup("temp");
    // assertThat(toolchainCollection).execGroup("temp")
    //     .hasToolchainType("//toolchain:test_toolchain");
    // assertThat(toolchainCollection).execGroup("temp")
    //     .hasResolvedToolchain("//toolchains:toolchain_1_impl");
}

// Java: ToolchainsForTargetsTest.keepParentToolchainContext
#[test]
#[ignore = "the test SkyFunction (ComputeUnloadedToolchainContextsFunction) can be registered \
            (BuildViewTestCase::set_extra_sky_functions) but calls DependencyResolver.getDependencyContext \
            with a DependencyResolver.State.createForTesting, both private to blaze_sky_analysis \
            (dependency_resolver.rs get_dependency_context, DependencyResolverState)"]
fn keep_parent_toolchain_context() {
    let mut t = new_test();
    // Add some platforms and custom constraints.
    t.scratch.file("platforms/BUILD", &[LOCAL_PLATFORMS_BUILD]);

    // Test normal resolution, and with a per-target exec constraint.
    t.scratch.file(
        "a/BUILD",
        &[r#"load("//toolchain:rule.bzl", "my_rule")

my_rule(name = "a")
"#],
    );

    t.use_configuration(&[
        "--extra_execution_platforms=//platforms:local_platform_a,//platforms:local_platform_b",
    ]);

    let target = only_target_to_build(&mut t, &["//a"]);
    let key = ConfiguredTargetKey::builder()
        .set_label(target.get_original_label())
        .set_configuration_key(target.get_configuration_key())
        .set_execution_platform_label(Some(label("//platforms:local_platform_b")))
        .build();
    get_toolchain_collection_for(&mut t, &target, key);
    // assertThat(toolchainCollection).isNotNull();
    // assertThat(toolchainCollection).hasDefaultExecGroup();
    // // This should have the same exec platform as parentToolchainKey, which is local_platform_b.
    // assertThat(toolchainCollection).defaultToolchainContext()
    //     .hasExecutionPlatform("//platforms:local_platform_b");
}

/// Regression test for b/214105142, https://github.com/bazelbuild/bazel/issues/14521
// Java: ToolchainsForTargetsTest.toolchainWithDifferentExecutionPlatforms_doesNotGenerateConflictingCoverageAction
#[test]
#[ignore = "the test SkyFunction (ComputeUnloadedToolchainContextsFunction) can be registered \
            (BuildViewTestCase::set_extra_sky_functions) but calls DependencyResolver.getDependencyContext \
            with a DependencyResolver.State.createForTesting, both private to blaze_sky_analysis \
            (dependency_resolver.rs get_dependency_context, DependencyResolverState)"]
fn toolchain_with_different_execution_platforms_does_not_generate_conflicting_coverage_action() {
    let mut t = new_test();
    t.scratch.file("platforms/BUILD", &[LOCAL_PLATFORMS_BUILD]);
    t.scratch.file(
        "a/BUILD",
        &[r#"load("//toolchain:rule.bzl", "my_rule")

my_rule(
    name = "a",
    srcs = ["a.c"],
    exec_compatible_with = ["//platforms:local_value_a"],
)

my_rule(
    name = "b",
    srcs = ["b.c"],
    exec_compatible_with = ["//platforms:local_value_b"],
)
"#],
    );
    t.use_configuration(&[
        "--collect_code_coverage",
        "--extra_execution_platforms=//platforms:local_platform_a,//platforms:local_platform_b",
    ]);

    t.update(&["//a:a", "//a:b"])
        .unwrap_or_else(|e| panic!("{e}"));

    // Sanity check that a coverage action was generated for the rule itself.
    assert_has_baseline_coverage_action(&mut t, "//a:a", "Writing file a/a/baseline_coverage.dat");
    assert_has_baseline_coverage_action(&mut t, "//a:b", "Writing file a/b/baseline_coverage.dat");
    assert!(t.get_actions("//toolchains:toolchain_1_impl").is_empty());
    get_toolchain_collection(&mut t, "//a:a");
    // ToolchainContext toolchainAContext =
    //     getToolchainCollection("//a:a").getDefaultToolchainContext();
    // assertThat(toolchainAContext).hasExecutionPlatform("//platforms:local_platform_a");
    // assertThat(toolchainAContext).hasToolchainType("//toolchain:test_toolchain");
    // assertThat(toolchainAContext).hasResolvedToolchain("//toolchains:toolchain_1_impl");
    // ToolchainContext toolchainBContext =
    //     getToolchainCollection("//a:b").getDefaultToolchainContext();
    // assertThat(toolchainBContext).hasExecutionPlatform("//platforms:local_platform_b");
    // assertThat(toolchainBContext).hasToolchainType("//toolchain:test_toolchain");
    // assertThat(toolchainBContext).hasResolvedToolchain("//toolchains:toolchain_1_impl");
}

// Java: ToolchainsForTargetsTest.assertHasBaselineCoverageAction
fn assert_has_baseline_coverage_action(
    t: &mut AnalysisTestCase,
    label: &str,
    progress_message: &str,
) {
    let actions = t.get_actions(label);
    assert_eq!(actions.len(), 1, "getOnlyElement: {actions:?}");
    let coverage_action = &actions[0];
    // assertThat(coverageAction).isInstanceOf(BaselineCoverageAction.class);
    assert_eq!(coverage_action.get_mnemonic(), "BaselineCoverage");
    assert_eq!(
        coverage_action.get_progress_message(None).as_deref(),
        Some(progress_message)
    );
}

// Java: ToolchainsForTargetsTest.targetCompatibleWith_matchesExecCompatibleWith
#[test]
#[ignore = "needs the mock C++ toolchains (ccSupport().setupCcToolchainConfig) and \
            AnalysisTestCase.update with explicitTargetPatterns in the harness"]
fn target_compatible_with_matches_exec_compatible_with() {
    let mut t = new_test();
    // Java: getAnalysisMock().ccSupport().setupCcToolchainConfig(mockToolsConfig,
    //     CcToolchainConfig.builder()
    //         .withToolchainTargetConstraints("@@//platforms:local_value_a")
    //         .withToolchainExecConstraints()
    //         .withCpu("fake"));
    t.scratch.file("platforms/BUILD", &[LOCAL_PLATFORMS_BUILD]);
    t.use_configuration(&[
        "--extra_execution_platforms=//platforms:local_platform_a,//platforms:local_platform_b",
    ]);
    t.scratch.file(
        "foo/BUILD",
        &[r#"load('//test_defs:foo_binary.bzl', 'foo_binary')
foo_binary(
    name = "tool",
    srcs = ["a.sh"],
    target_compatible_with = ["//platforms:local_value_b"],
)

genrule(
    name = "runtool",
    outs = ["b.txt"],
    cmd = "",
    exec_compatible_with = ["//platforms:local_value_b"],
    tools = [":tool"],
)
"#],
    );

    update_explicit_target(&mut t, "//foo:runtool");
    // assertThat(result.hasError()).isFalse();
    // assertNoEvents();
}

// Java: ToolchainsForTargetsTest.targetCompatibleWith_mismatchesExecCompatibleWith
#[test]
#[ignore = "needs AnalysisTestCase.update with explicitTargetPatterns in the harness"]
fn target_compatible_with_mismatches_exec_compatible_with() {
    let mut t = new_test();
    t.scratch.file("platforms/BUILD", &[LOCAL_PLATFORMS_BUILD]);
    t.use_configuration(&[
        "--extra_execution_platforms=//platforms:local_platform_a,//platforms:local_platform_b",
    ]);
    t.scratch.file(
        "foo/BUILD",
        &[r#"load('//test_defs:foo_binary.bzl', 'foo_binary')
foo_binary(
    name = "tool",
    srcs = ["a.sh"],
    target_compatible_with = ["//platforms:local_value_a"],
)

genrule(
    name = "runtool",
    outs = ["b.txt"],
    cmd = "",
    exec_compatible_with = ["//platforms:local_value_b"],
    tools = [":tool"],
)
"#],
    );

    t.remove_fail_fast_handler();
    update_explicit_target(&mut t, "//foo:runtool");
    // assertThat(result.hasError()).isTrue();
    // assertContainsEvent(
    //     "Target //foo:runtool is incompatible and cannot be built, but was explicitly requested");
}

const TOOL_LIB_BZL: &str = r#"def _impl(ctx):
    pass

my_rule = rule(
    _impl,
    attrs = {"_my_tool": attr.label(default = "//tool")},
)
"#;

const TOOL_BUILD_VALUE_A: &str = r#"load('//test_defs:foo_binary.bzl', 'foo_binary')
foo_binary(
    name = "tool",
    srcs = ["a.cc"],
    target_compatible_with = ["//platforms:local_value_a"],
)
"#;

// Java: ToolchainsForTargetsTest.targetCompatibleWith_mismatchesExecCompatibleInDifferentPackage
#[test]
#[ignore = "needs AnalysisTestCase.update with explicitTargetPatterns in the harness"]
fn target_compatible_with_mismatches_exec_compatible_in_different_package() {
    // Regression test for a case where incompatibility happens in an aspect tool in a different
    // package over a Starlark target with
    // --incompatible_visibility_private_attributes_at_definition
    // The tool is replaced with a fake target {@link
    // IncommpatibleTargetChecker#createIncompatibleRuleConfiguredTarget)
    // and it's verified if the tool is visible to the Starlark target.
    let mut t = new_test();
    t.scratch.file("platforms/BUILD", &[LOCAL_PLATFORMS_BUILD]);

    t.use_configuration(&[
        "--extra_execution_platforms=//platforms:local_platform_a,//platforms:local_platform_b",
    ]);
    t.scratch.file("foo/lib.bzl", &[TOOL_LIB_BZL]);
    t.scratch.file("tool/BUILD", &[TOOL_BUILD_VALUE_A]);
    t.scratch.file(
        "foo/BUILD",
        &[r#"load(":lib.bzl", "my_rule")

my_rule(name = "target_in_different_package")
"#],
    );

    t.remove_fail_fast_handler();
    update_explicit_target(&mut t, "//foo:target_in_different_package");
    // assertThat(result.hasError()).isTrue();
    // assertContainsEvent(
    //     "Target //foo:target_in_different_package is incompatible and cannot be built, but was"
    //         + " explicitly requested");
}

// Java: ToolchainsForTargetsTest.targetCompatibleWith_mismatchesExecCompatibleDepInDifferentPackage
#[test]
#[ignore = "needs AnalysisTestCase.update with explicitTargetPatterns in the harness"]
fn target_compatible_with_mismatches_exec_compatible_dep_in_different_package() {
    let mut t = new_test();
    t.scratch.file("platforms/BUILD", &[LOCAL_PLATFORMS_BUILD]);

    t.use_configuration(&[
        "--extra_execution_platforms=//platforms:local_platform_a,//platforms:local_platform_b",
    ]);
    t.scratch.file("foo/lib.bzl", &[TOOL_LIB_BZL]);
    t.scratch.file("tool/BUILD", &[TOOL_BUILD_VALUE_A]);
    t.scratch.file(
        "foo/BUILD",
        &[r#"load(":lib.bzl", "my_rule")

my_rule(name = "dep")

filegroup(
    name = "target_with_dep",
    srcs = [":dep"],
)
"#],
    );

    t.remove_fail_fast_handler();
    update_explicit_target(&mut t, "//foo:target_with_dep");
    // assertThat(result.hasError()).isTrue();
    // assertContainsEvent(
    //     "Target //foo:target_with_dep is incompatible and cannot be built, but was"
    //         + " explicitly requested");
}

// Java: ToolchainsForTargetsTest.targetCompatibleWith_mismatchesExecCompatibleWithinAspect
#[test]
#[ignore = "needs AnalysisTestCase.update with explicitTargetPatterns in the harness"]
fn target_compatible_with_mismatches_exec_compatible_within_aspect() {
    // Regression test for a case where incompatibility happens in an aspect tool in a different
    // package over a Starlark target with
    // --incompatible_visibility_private_attributes_at_definition
    // The tool is replaced with a fake target {@link
    // IncommpatibleTargetChecker#createIncompatibleRuleConfiguredTarget)
    // and it's verified if the tool is visible to the Starlark target the aspect is over.
    let mut t = new_test();
    t.scratch.file("platforms/BUILD", &[LOCAL_PLATFORMS_BUILD]);

    t.use_configuration(&[
        "--extra_execution_platforms=//platforms:local_platform_a,//platforms:local_platform_b",
    ]);
    t.scratch.file(
        "foo/lib.bzl",
        &[r#"def _impl_aspect(ctx, target):
    return []

my_aspect = aspect(
    _impl_aspect,
    attrs = {"_my_tool": attr.label(default = "//tool")},
    exec_compatible_with = ["//platforms:local_value_a"],
)

def _impl(ctx):
    pass

my_rule = rule(
    _impl,
    attrs = {"deps": attr.label_list(aspects = [my_aspect])},
)
simple_starlark_rule = rule(
    _impl,
)
"#],
    );
    t.scratch.file(
        "tool/BUILD",
        &[r#"load('//test_defs:foo_binary.bzl', 'foo_binary')
foo_binary(
    name = "tool",
    srcs = ["a.cc"],
    target_compatible_with = ["//platforms:local_value_b"],
)
"#],
    );
    t.scratch.file(
        "foo/BUILD",
        &[r#"load(":lib.bzl", "my_rule", "simple_starlark_rule")

simple_starlark_rule(name = "simple_dep")

my_rule(
    name = "target_with_aspect",
    deps = [":simple_dep"],
)
"#],
    );

    t.remove_fail_fast_handler();
    update_explicit_target(&mut t, "//foo:target_with_aspect");
    // TODO(bazel-team): This should report an error similarly to {@code
    // #targetCompatibleWith_mismatchesExecCompatibleDepInDifferentPackage}
    // assertThat(result.hasError()).isFalse();
}
