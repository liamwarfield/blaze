//! Port of
//! `src/test/java/com/google/devtools/build/lib/skyframe/toolchains/ToolchainsForTargetsTest.java`
//! (an `AnalysisTestCase`): toolchain contexts computed for targets by
//! `DependencyResolver.getDependencyContext`.
//!
//! Java injects a test SkyFunction (`CONFIGURED_TARGET_FUNCTION_COMPUTE_UNLOADED_TOOLCHAIN_CONTEXTS`,
//! through `AnalysisMock.Delegate.getSkyFunctions`) that returns the
//! `ToolchainCollection<UnloadedToolchainContext>` of a target; here it is registered with
//! `BuildViewTestCase::set_extra_sky_functions`.

use std::fmt;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use blaze_analysis::analysis_services::AnalysisServices;
use blaze_analysis::configured_target::ConfiguredTarget;
use blaze_analysis::configured_target_key::ConfiguredTargetKey;
use blaze_analysis::toolchains::{ToolchainCollection, UnloadedToolchainContext};
use blaze_sky_analysis::dependency_resolver::{
    CaughtException, DependencyResolverState, get_dependency_context,
};
use blaze_sky_analysis::producers::errors::TargetAndConfiguration;
use blaze_skyframe::{Key, SkyFunctionException, SkyFunctionName, SkyKey, Transience};
use parking_lot::Mutex;

use crate::testing::{AnalysisTestCase, BuildViewTestCase, has_error};
use crate::tests::toolchain_test_case::{
    assert_has_execution_platform, assert_has_resolved_toolchain, assert_has_toolchain_type, label,
};
use blaze_analysis::toolchains::{ToolchainContext, ToolchainTypeRequirement};
use blaze_packages::rule_class::DEFAULT_EXEC_GROUP_NAME;

/// Java `ToolchainsForTargetsTest.Key`.
///
/// PORT: equality and hash use the configured target key only; the target and configuration
/// are those of that key.
// Java: ToolchainsForTargetsTest.Key
#[derive(Clone)]
struct ComputeUnloadedToolchainContextsKey {
    target_and_configuration: TargetAndConfiguration,
    configured_target_key: ConfiguredTargetKey,
}

impl PartialEq for ComputeUnloadedToolchainContextsKey {
    fn eq(&self, other: &Self) -> bool {
        self.configured_target_key == other.configured_target_key
    }
}

impl Eq for ComputeUnloadedToolchainContextsKey {}

impl Hash for ComputeUnloadedToolchainContextsKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.configured_target_key.hash(state);
    }
}

impl fmt::Debug for ComputeUnloadedToolchainContextsKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Key{{configuredTargetKey={:?}}}",
            self.configured_target_key
        )
    }
}

impl Key for ComputeUnloadedToolchainContextsKey {
    type Value = ComputeUnloadedToolchainContextsValue;
    const FUNCTION: SkyFunctionName =
        SkyFunctionName("CONFIGURED_TARGET_FUNCTION_COMPUTE_UNLOADED_TOOLCHAIN_CONTEXTS");
}

/// Java `ToolchainsForTargetsTest.Value`.
// Java: ToolchainsForTargetsTest.Value
#[derive(Debug)]
struct ComputeUnloadedToolchainContextsValue {
    toolchain_collection: Option<Arc<ToolchainCollection<UnloadedToolchainContext>>>,
}

/// Java `LateBoundStateProvider`: the analysis services of the current executor.
type LateBoundStateProvider = Arc<Mutex<Option<Arc<AnalysisServices>>>>;

/// Java `ComputeUnloadedToolchainContextsFunction`, registered on the test case.
///
/// PORT: Java keeps `DependencyResolver.State` across restarts with `env.getState`; Blaze's
/// evaluator re-runs the function with a fresh `State.createForTesting(...)`, whose dependencies
/// are already computed.
// Java: ToolchainsForTargetsTest.ComputeUnloadedToolchainContextsFunction
fn register_compute_unloaded_toolchain_contexts_function(
    t: &mut AnalysisTestCase,
) -> LateBoundStateProvider {
    let state_provider: LateBoundStateProvider = Arc::new(Mutex::new(None));
    let provider = state_provider.clone();
    t.set_extra_sky_functions(move |builder| {
        let provider = provider.clone();
        builder.typed_function::<ComputeUnloadedToolchainContextsKey, _>(move |key, env| {
            let services = provider
                .lock()
                .clone()
                .expect("LateBoundStateProvider: the executor's analysis services");
            let mut state = DependencyResolverState::new(false);
            state.target_and_configuration = Some(key.target_and_configuration.clone());
            let result = match get_dependency_context(
                &mut state,
                &key.configured_target_key,
                &services,
                env,
            ) {
                Ok(r) => r,
                Err(CaughtException::Interrupted(i)) => return Err(i.into()),
                Err(CaughtException::Toolchain(e)) => {
                    return Err(SkyFunctionException::new(e, Transience::Persistent).into());
                }
                Err(_) => panic!(
                    "ComputeUnloadedToolchainContextsException for {:?}",
                    key.configured_target_key
                ),
            };
            if state.transitive_state.has_root_cause() {
                panic!("IllegalStateException: expected empty transitive root causes");
            }
            Ok(result.map(|r| ComputeUnloadedToolchainContextsValue {
                toolchain_collection: r.unloaded_toolchain_contexts,
            }))
        })
    });
    state_provider
}

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
    state_provider: &LateBoundStateProvider,
    configured_target: &ConfiguredTarget,
    configured_target_key: ConfiguredTargetKey,
) -> Option<Arc<ToolchainCollection<UnloadedToolchainContext>>> {
    let target_label = configured_target.get_original_label().to_string();
    let target = t
        .get_target(&target_label)
        .unwrap_or_else(|e| panic!("{e:?}"));
    let configuration = t.get_configuration(configured_target);
    let key = ComputeUnloadedToolchainContextsKey {
        target_and_configuration: TargetAndConfiguration::new(target, configuration),
        configured_target_key,
    };
    *state_provider.lock() = t.skyframe_executor().get_analysis_services().cloned();
    let eval_result = t.evaluate(&[SkyKey::new(key.clone())], false);
    eval_result
        .get(&key)
        .unwrap_or_else(|| panic!("NullPointerException: {:?}", eval_result.error_map()))
        .toolchain_collection
        .clone()
}

/// `getToolchainCollection(targetLabel)`.
// Java: ToolchainsForTargetsTest.getToolchainCollection
fn get_toolchain_collection(
    t: &mut AnalysisTestCase,
    state_provider: &LateBoundStateProvider,
    target_label: &str,
) -> Option<Arc<ToolchainCollection<UnloadedToolchainContext>>> {
    let target = only_target_to_build(t, &[target_label]);
    let key = ConfiguredTargetKey::builder()
        .set_label(target.get_original_label())
        .set_configuration_key(target.get_configuration_key())
        .build();
    get_toolchain_collection_for(t, state_provider, &target, key)
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
/// Returns `result.hasError()`.
fn update_explicit_target(t: &mut AnalysisTestCase, label: &str) -> bool {
    let flags = t.default_flags().with(crate::testing::Flag::KeepGoing);
    match t.update_with_explicit_target_patterns(&flags, &[label], &[], &[], &[label]) {
        Ok(result) => has_error(result),
        Err(e) => panic!("{e}"),
    }
}

// Java: ToolchainsForTargetsTest.basicToolchains
#[test]
fn basic_toolchains() {
    let mut t = new_test();
    let state_provider = register_compute_unloaded_toolchain_contexts_function(&mut t);
    t.scratch.file(
        "a/BUILD",
        &[r#"load("//toolchain:rule.bzl", "my_rule")

my_rule(name = "a")
"#],
    );

    let toolchain_collection = get_toolchain_collection(&mut t, &state_provider, "//a");
    let toolchain_collection = toolchain_collection
        .as_deref()
        .expect("assertThat(toolchainCollection).isNotNull()");
    assert!(toolchain_collection.has_toolchain_context(DEFAULT_EXEC_GROUP_NAME));
    let default_context = toolchain_collection.get_default_toolchain_context();
    assert_has_toolchain_type(default_context, &label("//toolchain:test_toolchain"));
    assert_has_resolved_toolchain(default_context, "//toolchains:toolchain_1_impl");
    let toolchain_type = label("//toolchain:test_toolchain");
    assert_eq!(
        default_context.toolchain_types(),
        &[ToolchainTypeRequirement::create(toolchain_type)]
    );
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
fn exec_platform() {
    let mut t = new_test();
    let state_provider = register_compute_unloaded_toolchain_contexts_function(&mut t);
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

    let toolchain_collection = get_toolchain_collection(&mut t, &state_provider, "//a");
    let toolchain_collection = toolchain_collection
        .as_deref()
        .expect("assertThat(toolchainCollection).isNotNull()");
    assert!(toolchain_collection.has_toolchain_context(DEFAULT_EXEC_GROUP_NAME));
    let default_context = toolchain_collection.get_default_toolchain_context();
    // First execution platform will be used.
    assert_has_execution_platform(default_context, "//platforms:local_platform_a");
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
fn exec_platform_with_exec_constraint() {
    let mut t = new_test();
    let state_provider = register_compute_unloaded_toolchain_contexts_function(&mut t);
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

    let toolchain_collection = get_toolchain_collection(&mut t, &state_provider, "//a");
    let toolchain_collection = toolchain_collection
        .as_deref()
        .expect("assertThat(toolchainCollection).isNotNull()");
    assert!(toolchain_collection.has_toolchain_context(DEFAULT_EXEC_GROUP_NAME));
    let default_context = toolchain_collection.get_default_toolchain_context();
    // Exec constraint forces the use of this exec platform.
    assert_has_execution_platform(default_context, "//platforms:local_platform_b");
}

// Java: ToolchainsForTargetsTest.execGroups_named
#[test]
fn exec_groups_named() {
    let mut t = new_test();
    let state_provider = register_compute_unloaded_toolchain_contexts_function(&mut t);
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

    let toolchain_collection = get_toolchain_collection(&mut t, &state_provider, "//a");
    let toolchain_collection = toolchain_collection
        .as_deref()
        .expect("assertThat(toolchainCollection).isNotNull()");
    assert!(toolchain_collection.has_toolchain_context(DEFAULT_EXEC_GROUP_NAME));
    let default_context = toolchain_collection.get_default_toolchain_context();
    assert!(default_context.toolchain_types().is_empty());
    assert!(default_context.resolved_toolchain_labels().is_empty());
    assert!(toolchain_collection.has_toolchain_context("temp"));
    let temp = toolchain_collection
        .get_toolchain_context("temp")
        .expect("execGroup(temp)");
    assert_has_toolchain_type(temp, &label("//toolchain:test_toolchain"));
    assert_has_resolved_toolchain(temp, "//toolchains:toolchain_1_impl");
    assert_has_toolchain_type(temp, &label("//toolchain:test_toolchain"));
    assert_has_resolved_toolchain(temp, "//toolchains:toolchain_1_impl");
}

// Java: ToolchainsForTargetsTest.execGroups_defaultAndNamed
#[test]
fn exec_groups_default_and_named() {
    let mut t = new_test();
    let state_provider = register_compute_unloaded_toolchain_contexts_function(&mut t);
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
    let toolchain_collection = get_toolchain_collection(&mut t, &state_provider, "//a");
    let toolchain_collection = toolchain_collection
        .as_deref()
        .expect("assertThat(toolchainCollection).isNotNull()");
    assert!(toolchain_collection.has_toolchain_context(DEFAULT_EXEC_GROUP_NAME));
    let default_context = toolchain_collection.get_default_toolchain_context();
    assert_has_toolchain_type(default_context, &label("//extra:extra_toolchain"));
    assert_has_resolved_toolchain(default_context, "//extra:toolchain_impl");
    assert!(toolchain_collection.has_toolchain_context("temp"));
    let temp = toolchain_collection
        .get_toolchain_context("temp")
        .expect("execGroup(temp)");
    assert_has_toolchain_type(temp, &label("//toolchain:test_toolchain"));
    assert_has_resolved_toolchain(temp, "//toolchains:toolchain_1_impl");
}

// Java: ToolchainsForTargetsTest.keepParentToolchainContext
#[test]
fn keep_parent_toolchain_context() {
    let mut t = new_test();
    let state_provider = register_compute_unloaded_toolchain_contexts_function(&mut t);
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
    let toolchain_collection = get_toolchain_collection_for(&mut t, &state_provider, &target, key);
    let toolchain_collection = toolchain_collection
        .as_deref()
        .expect("assertThat(toolchainCollection).isNotNull()");
    assert!(toolchain_collection.has_toolchain_context(DEFAULT_EXEC_GROUP_NAME));
    let default_context = toolchain_collection.get_default_toolchain_context();
    // This should have the same exec platform as parentToolchainKey, which is local_platform_b.
    assert_has_execution_platform(default_context, "//platforms:local_platform_b");
}

/// Regression test for b/214105142, https://github.com/bazelbuild/bazel/issues/14521
// Java: ToolchainsForTargetsTest.toolchainWithDifferentExecutionPlatforms_doesNotGenerateConflictingCoverageAction
#[test]
#[ignore = "BLAZE GAP: unported!(\"BaselineCoverageAction.create\") (blaze_analysis test/instrumented_files_info.rs)"]
fn toolchain_with_different_execution_platforms_does_not_generate_conflicting_coverage_action() {
    let mut t = new_test();
    let state_provider = register_compute_unloaded_toolchain_contexts_function(&mut t);
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
    let collection_a = get_toolchain_collection(&mut t, &state_provider, "//a:a")
        .expect("toolchain collection of //a:a");
    let toolchain_a_context = collection_a.get_default_toolchain_context();
    assert_has_execution_platform(toolchain_a_context, "//platforms:local_platform_a");
    assert_has_toolchain_type(toolchain_a_context, &label("//toolchain:test_toolchain"));
    assert_has_resolved_toolchain(toolchain_a_context, "//toolchains:toolchain_1_impl");
    let collection_b = get_toolchain_collection(&mut t, &state_provider, "//a:b")
        .expect("toolchain collection of //a:b");
    let toolchain_b_context = collection_b.get_default_toolchain_context();
    assert_has_execution_platform(toolchain_b_context, "//platforms:local_platform_b");
    assert_has_toolchain_type(toolchain_b_context, &label("//toolchain:test_toolchain"));
    assert_has_resolved_toolchain(toolchain_b_context, "//toolchains:toolchain_1_impl");
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
fn target_compatible_with_matches_exec_compatible_with() {
    let mut t = new_test();
    crate::testing::mock_cc_support::BazelMockCcSupport.setup_cc_toolchain_config_with(
        &t.mock_tools_config,
        crate::testing::mock_cc_support::CcToolchainConfig::builder()
            .with_toolchain_target_constraints(&["@@//platforms:local_value_a"])
            .with_toolchain_exec_constraints(&[])
            .with_cpu("fake"),
    );
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

    let has_error = update_explicit_target(&mut t, "//foo:runtool");

    assert!(!has_error);
    t.assert_no_events();
}

// Java: ToolchainsForTargetsTest.targetCompatibleWith_mismatchesExecCompatibleWith
#[test]
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
    let has_error = update_explicit_target(&mut t, "//foo:runtool");

    assert!(has_error);
    t.assert_contains_event(
        "Target //foo:runtool is incompatible and cannot be built, but was explicitly requested",
    );
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
    let has_error = update_explicit_target(&mut t, "//foo:target_in_different_package");

    assert!(has_error);
    t.assert_contains_event(
        "Target //foo:target_in_different_package is incompatible and cannot be built, but was \
         explicitly requested",
    );
}

// Java: ToolchainsForTargetsTest.targetCompatibleWith_mismatchesExecCompatibleDepInDifferentPackage
#[test]
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
    let has_error = update_explicit_target(&mut t, "//foo:target_with_dep");

    assert!(has_error);
    t.assert_contains_event(
        "Target //foo:target_with_dep is incompatible and cannot be built, but was explicitly \
         requested",
    );
}

// Java: ToolchainsForTargetsTest.targetCompatibleWith_mismatchesExecCompatibleWithinAspect
#[test]
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
    let has_error = update_explicit_target(&mut t, "//foo:target_with_aspect");
    // TODO(bazel-team): This should report an error similarly to {@code
    // #targetCompatibleWith_mismatchesExecCompatibleDepInDifferentPackage}
    assert!(!has_error);
}
