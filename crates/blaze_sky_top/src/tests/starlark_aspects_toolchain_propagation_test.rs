//! Port of
//! `src/test/java/com/google/devtools/build/lib/starlark/StarlarkAspectsToolchainPropagationTest.java`
//! (an `AnalysisTestCase`): Starlark aspects propagating to the toolchain dependencies of targets
//! (`toolchains_aspects`, `ctx.rule.toolchains`, `ctx.rule.exec_groups`, the base target's
//! toolchain contexts).
//!
//! Java runs the parameterized tests (`@TestParameters` / `@TestParameter boolean`) once per
//! value; each value is one Rust test here, named after the Java method with the parameter as a
//! suffix (`_auto_exec_groups_true` / `_auto_exec_groups_false`).

use std::sync::Arc;

use blaze_analysis::aspect_key_creator::AspectKey;
use blaze_analysis::aspect_value::AspectValue;
use blaze_analysis::configured_target::ConfiguredTarget;
use blaze_analysis::configured_target_key::ConfiguredTargetKey;
use blaze_cmdline::Label;
use blaze_events::EventKind;
use blaze_optdefs::command::analysis_options::AnalysisOptions;
use blaze_optdefs::command::loading_options::LoadingOptions;
use blaze_optdefs::fragment::platform_options::PlatformOptions;
use blaze_options::Options;
use blaze_packages::rule_class::ProviderKey;
use blaze_skyframe::{Evaluator, SkyKey, WalkableGraph};
use blaze_vfs::PathFragment;
use indexmap::{IndexMap, IndexSet};
use starlark::values::dict::DictRef;
use starlark::values::{Heap, OwnedFrozenValue};

use crate::build_view::{self, AnalysisResult, BuildViewError, BuildViewUpdateOptions};
use crate::skyframe_executor::TargetPatternPhaseError;
use crate::testing::build_view_test_case::{LOADING_PHASE_THREADS, parse_label};
use crate::testing::{AnalysisTestCase, BuildViewTestCase, assert_contains_exactly};

/// The test class's `@Before setup()` on a fresh `AnalysisTestCase`.
fn new_test() -> AnalysisTestCase {
    let mut t = BuildViewTestCase::analysis_test_case();
    setup(&mut t);
    t
}

// Java: StarlarkAspectsToolchainPropagationTest.setup
fn setup(t: &mut AnalysisTestCase) {
    create_toolchains_and_platforms(t);
}

/// Sets up 3 toolchain rules:
///
/// test_toolchain: has no attribute dependency and no advertised providers
///
/// test_toolchain_with_provider: has an advertised provider but no attribute dependency
///
/// test_toolchain_with_dep: has an attribute dependency but no advertised providers
///
/// We also set up 3 toolchain types:
///
/// toolchain_type_1: resolved by `foo` of rule `test_toolchain`
///
/// toolchain_type_2: resolved by `foo_with_provider` of rule `test_toolchain_with_provider`
///
/// toolchain_type_3: resolved by `foo_with_dep` of rule `test_toolchain_with_dep`
///
/// Toolchain `foo_for_all` resolved both toolchain_type_2 and toolchain_type_3
// Java: StarlarkAspectsToolchainPropagationTest.createToolchainsAndPlatforms
fn create_toolchains_and_platforms(t: &mut AnalysisTestCase) {
    t.scratch.overwrite_file(
        "rule/test_toolchain.bzl",
        &[r##"MyProvider = provider()

def _impl(ctx):
    toolchain = platform_common.ToolchainInfo(
        tool = ctx.executable._tool,
        files_to_run = ctx.attr._tool[DefaultInfo].files_to_run,
    )
    my_provider = MyProvider(value = str(ctx.label))
    vars = platform_common.TemplateVariableInfo(ctx.attr.vars)
    return [toolchain, my_provider, vars]

test_toolchain = rule(
    implementation = _impl,
    attrs = {
        "vars": attr.string_dict(),
        "_tool": attr.label(
            default = "//toolchain:a_tool",
            executable = True,
            cfg = "exec",
        ),
    },
)

test_toolchain_with_provider = rule(
    implementation = _impl,
    attrs = {
        "vars": attr.string_dict(),
        "_tool": attr.label(
            default = "//toolchain:a_tool",
            executable = True,
            cfg = "exec",
        ),
    },
    provides = [MyProvider]
)

test_toolchain_with_dep = rule(
    implementation = _impl,
    attrs = {
        "vars": attr.string_dict(),
        "_tool": attr.label(
            default = "//toolchain:a_tool",
            executable = True,
            cfg = "exec",
        ),
        "toolchain_dep": attr.label(),
    },
)

"##],
    );
    t.scratch.overwrite_file(
        "rule/BUILD",
        &[r##"exports_files(["test_toolchain/bzl"])

toolchain_type(name = "toolchain_type_1")
alias(name = "toolchain_type_1_alias", actual = ":toolchain_type_1")

toolchain_type(name = "toolchain_type_2")

toolchain_type(name = "toolchain_type_3")
"##],
    );
    t.scratch.overwrite_file(
        "toolchain/BUILD",
        &[r##"load("//rule:test_toolchain.bzl", "test_toolchain",
      "test_toolchain_with_provider", "test_toolchain_with_dep")

genrule(
    name = "a_tool",
    outs = ["atool"],
    cmd = "",
    executable = True,
)

test_toolchain(
    name = "foo",
    vars = {
        "type_1_key": "type_1_value",
    },
)

toolchain(
    name = "foo_toolchain",
    toolchain = ":foo",
    toolchain_type = "//rule:toolchain_type_1",
)

test_toolchain_with_provider(
    name = "foo_with_provider",
    vars = {
        "type_2_key": "type_2_value",
    },
)

toolchain(
    name = "foo_toolchain_with_provider",
    toolchain = ":foo_with_provider",
    toolchain_type = "//rule:toolchain_type_2",
)

filegroup(name = "toolchain_dep")

test_toolchain_with_dep(
    name = "foo_with_dep",
    toolchain_dep = ":toolchain_dep",
)

toolchain(
    name = "foo_toolchain_with_dep",
    toolchain = ":foo_with_dep",
    toolchain_type = "//rule:toolchain_type_3",
)

test_toolchain(name = "foo_for_all")

toolchain(
    name = "foo_type_2",
    toolchain = ":foo_for_all",
    toolchain_type = "//rule:toolchain_type_2",
)

toolchain(
    name = "foo_type_3",
    toolchain = ":foo_for_all",
    toolchain_type = "//rule:toolchain_type_3",
)

toolchain(
    name = "foo_toolchain_exec_1",
    toolchain = ":foo",
    exec_compatible_with = ['//platforms:constraint_1'],
    toolchain_type = "//rule:toolchain_type_1",
)

toolchain(
    name = "foo_toolchain_exec_2",
    toolchain = ":foo",
    exec_compatible_with = ['//platforms:constraint_2'],
    toolchain_type = "//rule:toolchain_type_2",
)
"##],
    );

    t.scratch.overwrite_file(
        "platforms/BUILD",
        &[r##"constraint_setting(name = "setting_1")
constraint_setting(name = "setting_2")

constraint_value(
    name = "constraint_1",
    constraint_setting = ":setting_1",
)
constraint_value(
    name = "constraint_2",
    constraint_setting = ":setting_2",
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
    // PORT: Java then calls
    //   getAnalysisMock().ccSupport().setupCcToolchainConfig(mockToolsConfig,
    //       CcToolchainConfig.builder()
    //           .withToolchainTargetConstraints("@@//platforms:constraint_1")
    //           .withToolchainExecConstraints("@@//platforms:constraint_1")
    //           .withCpu("fake"));
    // The mock C++ toolchain (`MockCcSupport`) is not ported (testing/analysis_mock.rs). No test of
    // this class analyzes a C++ rule; the crosstool only adds an unused registered toolchain.
}

// -----------------------------------------------------------------------------------------------
// Private helpers of the Java test class and of `AnalysisTestCase` that the harness lacks.

/// `AnalysisTestCase.update(aspects, aspectsParameters, labels...)`.
///
/// PORT: the harness's `update_with` passes no aspect parameters; this repeats its steps (with
/// default `LoadingOptions`, as every test of this class has) and calls
/// `BuildView.update` with the parameters. The fail-fast check is done on the collected events
/// (the fail-fast handler is registered in every caller).
// Java: com/google/devtools/build/lib/analysis/util/AnalysisTestCase.java:364 AnalysisTestCase.update
fn update_with_aspects_parameters(
    t: &mut AnalysisTestCase,
    aspects: &[&str],
    aspects_parameters: &[(&str, &str)],
    labels: &[&str],
) -> Result<AnalysisResult, BuildViewError> {
    let keep_going = t.default_flags().contains(crate::testing::Flag::KeepGoing);
    t.event_bus.clear();
    // Java: invalidateFilesUnderPathForTesting(EVERYTHING_MODIFIED).
    t.invalidate_packages();
    let handler = t.event_handler();
    let target_options = t.get_target_configuration().get_options().clone();
    let loading_options: LoadingOptions = Options::<LoadingOptions>::get_defaults();
    let analysis_options: AnalysisOptions = Options::<AnalysisOptions>::get_defaults();
    let targets: Vec<String> = labels.iter().map(|s| s.to_string()).collect();
    let executor = t.skyframe_executor();
    let loading_result = match executor.load_target_patterns_with_filters(
        handler.clone(),
        &targets,
        &PathFragment::create(""),
        &loading_options,
        LOADING_PHASE_THREADS,
        keep_going,
        /* determine_tests= */ false,
    ) {
        Ok(v) => v,
        Err(TargetPatternPhaseError::TargetParsing(e)) => {
            return Err(BuildViewError::ViewCreationFailed {
                message: e.get_message().to_string(),
                failure_detail: Default::default(),
            });
        }
        Err(TargetPatternPhaseError::Interrupted) => return Err(BuildViewError::Interrupted),
    };
    let options = BuildViewUpdateOptions {
        keep_going,
        check_for_action_conflicts: true,
        aspects: aspects.iter().map(|s| s.to_string()).collect(),
        explicit_target_patterns: IndexSet::new(),
        skip_incompatible_explicit_targets: analysis_options.skip_incompatible_explicit_targets,
        report_incompatible_targets: true,
        include_execution_phase: false,
    };
    let parameters: IndexMap<String, String> = aspects_parameters
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    let result = build_view::update_with_aspects_parameters(
        executor,
        &loading_result,
        &target_options,
        &options,
        &parameters,
        handler,
    );
    // failFastHandler.
    if let Some(error) = t
        .event_collector
        .iter()
        .find(|e| EventKind::ERRORS.contains(e.get_kind()))
    {
        panic!("{error}");
    }
    result
}

/// `skyframeExecutor.getEvaluator()`.
fn evaluator(t: &mut AnalysisTestCase) -> Arc<Evaluator> {
    t.get_evaluator()
}

/// `getDoneValues()` restricted to `AspectKey`s: the done aspect nodes with their values.
fn done_aspect_values(t: &mut AnalysisTestCase) -> Vec<(AspectKey, Arc<AspectValue>)> {
    let ev = evaluator(t);
    ev.get_all_keys_for_testing()
        .into_iter()
        .filter_map(|k| {
            let aspect_key = k.downcast::<AspectKey>()?.clone();
            let value = ev.get_existing_value(&k)?;
            let value = value
                .downcast::<AspectValue>()
                .unwrap_or_else(|_| panic!("ClassCastException: value of {aspect_key:?}"));
            Some((aspect_key, value))
        })
        .collect()
}

/// `getInMemoryGraph().getAllNodeEntries()...filter(n -> n.getKey().equals(key))` is not null.
fn assert_node_exists(t: &mut AnalysisTestCase, key: &AspectKey) {
    let ev = evaluator(t);
    let sky_key = SkyKey::new(key.clone());
    assert!(
        ev.get_all_keys_for_testing().contains(&sky_key),
        "no node for {key:?}"
    );
}

// Java: StarlarkAspectsToolchainPropagationTest.getDirectDeps
fn get_direct_deps(t: &mut AnalysisTestCase, key: SkyKey) -> Vec<SkyKey> {
    evaluator(t).get_direct_deps(&key)
}

fn get_aspect_direct_deps(t: &mut AnalysisTestCase, key: &AspectKey) -> Vec<SkyKey> {
    get_direct_deps(t, SkyKey::new(key.clone()))
}

/// `Iterables.filter(deps, AspectKey.class)`.
fn aspect_keys_of(deps: &[SkyKey]) -> Vec<AspectKey> {
    deps.iter()
        .filter_map(|d| d.downcast::<AspectKey>().cloned())
        .collect()
}

// Java: StarlarkAspectsToolchainPropagationTest.isConfiguredTarget
fn is_configured_target(key: &SkyKey, label: &str) -> bool {
    key.downcast::<ConfiguredTargetKey>()
        .is_some_and(|k| k.get_label().to_string() == label)
}

// Java: StarlarkAspectsToolchainPropagationTest.getConfiguredTargetKey
fn get_configured_target_key(
    t: &mut AnalysisTestCase,
    target_label: &str,
) -> Vec<ConfiguredTargetKey> {
    evaluator(t)
        .get_all_keys_for_testing()
        .into_iter()
        .filter(|k| is_configured_target(k, target_label))
        .map(|k| k.downcast::<ConfiguredTargetKey>().unwrap().clone())
        .collect()
}

// Java: StarlarkAspectsToolchainPropagationTest.getAspectKeys
fn get_aspect_keys_names(t: &mut AnalysisTestCase, aspect_label: &str) -> Vec<String> {
    done_aspect_values(t)
        .into_iter()
        .filter(|(k, _)| k.get_aspect_class().get_name() == aspect_label)
        .map(|(k, _)| format!("{} on {}", k.get_aspect_class().get_name(), k.get_label()))
        .collect()
}

// Java: StarlarkAspectsToolchainPropagationTest.getAspectKeys
fn get_aspect_keys(
    t: &mut AnalysisTestCase,
    target_label: &str,
    aspect_label: &str,
) -> Vec<AspectKey> {
    done_aspect_values(t)
        .into_iter()
        .filter(|(k, _)| {
            k.get_aspect_class().get_name() == aspect_label
                && k.get_label().to_string() == target_label
        })
        .map(|(k, _)| k)
        .collect()
}

// Java: StarlarkAspectsToolchainPropagationTest.getToplevelConfiguredAspect
fn get_toplevel_configured_aspect(
    analysis_result: &AnalysisResult,
    aspect_name: &str,
    target_label: &str,
) -> Arc<AspectValue> {
    get_only_element(
        analysis_result
            .get_aspects_map()
            .iter()
            .filter(|(k, _)| {
                k.get_aspect_name() == aspect_name && k.get_label().to_string() == target_label
            })
            .map(|(_, v)| v.clone())
            .collect(),
    )
}

// Java: StarlarkAspectsToolchainPropagationTest.getAspectResult
fn get_aspect_result(
    aspects_map: &IndexMap<AspectKey, Arc<AspectValue>>,
    aspect_name: &str,
    target_label: Option<&str>,
    provider_name: &str,
) -> Vec<String> {
    for (key, value) in aspects_map {
        if let Some(starlark_aspect_class) = key.get_aspect_class().as_starlark() {
            let aspect_exported_name = starlark_aspect_class.get_exported_name();
            if aspect_exported_name == aspect_name
                && target_label.is_none_or(|l| key.get_label().to_string() == l)
            {
                let provider = get_starlark_provider(value, "//test:defs.bzl", provider_name)
                    .expect("provider");
                return get_sequence(&provider, "res");
            }
        }
    }
    panic!("Aspect result not found for aspect: {aspect_name}");
}

/// `Iterables.getOnlyElement`.
fn get_only_element<T>(items: Vec<T>) -> T {
    assert_eq!(items.len(), 1, "expected one element, got {}", items.len());
    items.into_iter().next().unwrap()
}

/// `getStarlarkProvider(configuredAspect, bzl, name)` / `configuredAspect.get(new
/// StarlarkProvider.Key(keyForBuild(bzl), name))`.
fn get_starlark_provider(
    aspect: &AspectValue,
    bzl_label: &str,
    provider_name: &str,
) -> Option<OwnedFrozenValue> {
    aspect
        .get_providers()
        .get(&provider_key(bzl_label, provider_name))
        .cloned()
}

fn provider_key(bzl_label: &str, provider_name: &str) -> ProviderKey {
    ProviderKey::Starlark {
        extension_label: parse_label(bzl_label),
        exported_name: provider_name.to_string(),
    }
}

/// `info` as a value usable with a temporary heap.
fn frozen<'v>(info: &OwnedFrozenValue) -> starlark::values::Value<'v> {
    // SAFETY: every caller uses the value only while `info` (which keeps its frozen heap alive)
    // is borrowed.
    unsafe { info.unchecked_frozen_value() }.to_value()
}

/// `((StarlarkInfo) info).getValue(name)` as an iterable of strings.
fn get_sequence(info: &OwnedFrozenValue, name: &str) -> Vec<String> {
    Heap::temp(|heap| {
        let value = frozen(info)
            .get_attr(name, heap)
            .unwrap_or_else(|e| panic!("{e}"))
            .unwrap_or_else(|| panic!("no field {name}"));
        value
            .iterate(heap)
            .unwrap_or_else(|e| panic!("ClassCastException: {e}"))
            .map(|v| {
                v.unpack_str()
                    .unwrap_or_else(|| panic!("not a string: {v}"))
                    .to_string()
            })
            .collect()
    })
}

/// `(String) ((StarlarkInfo) info).getValue(name)`.
fn get_string(info: &OwnedFrozenValue, name: &str) -> String {
    Heap::temp(|heap| {
        let value = frozen(info)
            .get_attr(name, heap)
            .unwrap_or_else(|e| panic!("{e}"))
            .unwrap_or_else(|| panic!("no field {name}"));
        value
            .unpack_str()
            .unwrap_or_else(|| panic!("ClassCastException: {value}"))
            .to_string()
    })
}

/// `((StarlarkInfo) info).getValue(name)` if it is a `Dict` (string keys and values), else
/// `None` (`isInstanceOf(Dict.class)` fails).
fn get_dict(info: &OwnedFrozenValue, name: &str) -> Option<IndexMap<String, String>> {
    Heap::temp(|heap| {
        let value = frozen(info)
            .get_attr(name, heap)
            .unwrap_or_else(|e| panic!("{e}"))
            .unwrap_or_else(|| panic!("no field {name}"));
        let dict = DictRef::from_value(value)?;
        Some(
            dict.iter()
                .map(|(k, v)| {
                    (
                        k.to_str(),
                        v.unpack_str().map(str::to_string).unwrap_or(v.to_str()),
                    )
                })
                .collect(),
        )
    })
}

/// `ConfiguredTargetKey.fromConfiguredTarget`.
// Java: com/google/devtools/build/lib/skyframe/ConfiguredTargetKey.java:236 ConfiguredTargetKey.fromConfiguredTarget
fn configured_target_key_from_configured_target(
    configured_target: &ConfiguredTarget,
) -> ConfiguredTargetKey {
    configured_target
        .unwrap_if_merged()
        .get_lookup_key()
        .as_any()
        .downcast_ref::<ConfiguredTargetKey>()
        .expect("ClassCastException: ConfiguredTargetKey")
        .clone()
}

/// `key.getConfigurationKey().getOptions().get(PlatformOptions.class).platforms`.
fn platforms_of(key: &ConfiguredTargetKey) -> Vec<Label> {
    key.get_configuration_key()
        .expect("configuration key")
        .get_options()
        .get::<PlatformOptions>()
        .expect("PlatformOptions")
        .platforms
        .iter()
        .map(|l| l.0.clone())
        .collect()
}

/// `String.valueOf(label)` of a nullable label.
fn label_or_null(label: Option<&Label>) -> String {
    label.map_or_else(|| "null".to_string(), |l| l.to_string())
}

/// The assertions shared by the `*_singleDepAdded` tests and
/// `aspectPropagatesToToolchainUsingToolchainTypeAlias` (written out in each Java test).
fn assert_toolchain_aspect_adds_single_dep(t: &mut AnalysisTestCase) {
    let toolchain_aspect = get_only_element(get_aspect_keys(
        t,
        "//test:t1",
        "//test:defs.bzl%toolchain_aspect",
    ));
    assert_node_exists(t, &toolchain_aspect);

    let no_toolchain_aspect = get_only_element(get_aspect_keys(
        t,
        "//test:t1",
        "//test:defs.bzl%no_toolchain_aspect",
    ));
    assert_node_exists(t, &no_toolchain_aspect);

    let toolchain_aspect_direct_deps: IndexSet<SkyKey> =
        get_aspect_direct_deps(t, &toolchain_aspect)
            .into_iter()
            .collect();
    let no_toolchain_aspect_direct_deps: IndexSet<SkyKey> =
        get_aspect_direct_deps(t, &no_toolchain_aspect)
            .into_iter()
            .collect();

    // only one extra dependency is added for the toolchain propagating aspect
    assert_eq!(
        toolchain_aspect_direct_deps.len() as i64 - no_toolchain_aspect_direct_deps.len() as i64,
        1,
        "toolchain_aspect deps {toolchain_aspect_direct_deps:?}, no_toolchain_aspect deps \
         {no_toolchain_aspect_direct_deps:?}"
    );
    for d in &no_toolchain_aspect_direct_deps {
        assert!(
            toolchain_aspect_direct_deps.contains(d),
            "containsAtLeastElementsIn: missing {d:?}"
        );
    }

    // the extra dependency is the aspect application on the target's resolved toolchain
    let aspect_on_toolchain_dep = get_only_element(
        toolchain_aspect_direct_deps
            .difference(&no_toolchain_aspect_direct_deps)
            .cloned()
            .collect(),
    );
    let aspect_on_toolchain_dep = aspect_on_toolchain_dep
        .downcast::<AspectKey>()
        .expect("isInstanceOf(AspectKey.class)");
    assert_eq!(
        aspect_on_toolchain_dep.get_aspect_name(),
        "//test:defs.bzl%toolchain_aspect"
    );
    assert_eq!(
        aspect_on_toolchain_dep.get_label().to_string(),
        "//toolchain:foo"
    );
}

/// `k.getAspectName() + " on " + k.getLabel()` of each aspect key.
fn aspect_names_on(keys: &[AspectKey]) -> Vec<String> {
    keys.iter()
        .map(|k| format!("{} on {}", k.get_aspect_name(), k.get_label()))
        .collect()
}

// -----------------------------------------------------------------------------------------------
// Tests.

// Java: StarlarkAspectsToolchainPropagationTest.aspectPropagatesToToolchain_singleDepAdded
fn aspect_propagates_to_toolchain_single_dep_added(auto_exec_groups: &str) {
    let mut t = new_test();
    t.scratch.file(
        "test/defs.bzl",
        &[r##"def _impl(target, ctx):
  return []

toolchain_aspect = aspect(
  implementation = _impl,
  toolchains_aspects = ['//rule:toolchain_type_1'],
)

no_toolchain_aspect = aspect(
  implementation = _impl,
)

def _rule_impl(ctx):
  pass

r1 = rule(
  implementation = _rule_impl,
  toolchains = ['//rule:toolchain_type_1'],
)
"##],
    );
    t.scratch.file(
        "test/BUILD",
        &[r##"load('//test:defs.bzl', 'r1')
r1(name = 't1')
"##],
    );
    t.use_configuration(&[
        "--extra_toolchains=//toolchain:foo_toolchain",
        &format!("--incompatible_auto_exec_groups={auto_exec_groups}"),
    ]);

    let _unused = t
        .update_with_aspects(
            &[
                "//test:defs.bzl%toolchain_aspect",
                "//test:defs.bzl%no_toolchain_aspect",
            ],
            &["//test:t1"],
        )
        .unwrap();

    assert_toolchain_aspect_adds_single_dep(&mut t);
}

#[test]
fn aspect_propagates_to_toolchain_single_dep_added_auto_exec_groups_true() {
    aspect_propagates_to_toolchain_single_dep_added("True");
}

#[test]
fn aspect_propagates_to_toolchain_single_dep_added_auto_exec_groups_false() {
    aspect_propagates_to_toolchain_single_dep_added("False");
}

// Java: StarlarkAspectsToolchainPropagationTest.aspectPropagatesToExecGpToolchain_singleDepAdded
#[test]
fn aspect_propagates_to_exec_gp_toolchain_single_dep_added() {
    let mut t = new_test();
    t.scratch.file(
        "test/defs.bzl",
        &[r##"def _impl(target, ctx):
  return []

toolchain_aspect = aspect(
  implementation = _impl,
  toolchains_aspects = ['//rule:toolchain_type_1'],
)

no_toolchain_aspect = aspect(
  implementation = _impl,
)

def _rule_impl(ctx):
  pass

r1 = rule(
  implementation = _rule_impl,
  exec_groups = {"gp": exec_group(toolchains = ['//rule:toolchain_type_1'])},
)
"##],
    );
    t.scratch.file(
        "test/BUILD",
        &[r##"load('//test:defs.bzl', 'r1')
r1(name = 't1')
"##],
    );
    t.use_configuration(&["--extra_toolchains=//toolchain:foo_toolchain"]);

    let _unused = t
        .update_with_aspects(
            &[
                "//test:defs.bzl%toolchain_aspect",
                "//test:defs.bzl%no_toolchain_aspect",
            ],
            &["//test:t1"],
        )
        .unwrap();

    assert_toolchain_aspect_adds_single_dep(&mut t);
}

// Java: StarlarkAspectsToolchainPropagationTest.aspectHasToolchains_dependencyEdgeCreated
fn aspect_has_toolchains_dependency_edge_created(auto_exec_groups: &str) {
    let mut t = new_test();
    t.scratch.file(
        "test/defs.bzl",
        &[r##"def _impl(target, ctx):
  return []

toolchain_aspect = aspect(
  implementation = _impl,
  toolchains_aspects = ['//rule:toolchain_type_1'],
  toolchains = ['//rule:toolchain_type_2'],
)

def _rule_impl(ctx):
  pass

r1 = rule(
  implementation = _rule_impl,
  toolchains = ['//rule:toolchain_type_1'],
)
"##],
    );
    t.scratch.file(
        "test/BUILD",
        &[r##"load('//test:defs.bzl', 'r1')
r1(name = 't1')
"##],
    );
    t.use_configuration(&[
        "--extra_toolchains=//toolchain:foo_toolchain,//toolchain:foo_toolchain_with_provider",
        &format!("--incompatible_auto_exec_groups={auto_exec_groups}"),
    ]);

    let _unused = t
        .update_with_aspects(&["//test:defs.bzl%toolchain_aspect"], &["//test:t1"])
        .unwrap();

    let toolchain_aspect = get_only_element(get_aspect_keys(
        &mut t,
        "//test:t1",
        "//test:defs.bzl%toolchain_aspect",
    ));
    assert_node_exists(&mut t, &toolchain_aspect);

    // A dependency edge is created from the aspect to its own toolchain but not to the target's
    // toolchain.
    let aspect_configured_target_deps: Vec<String> =
        get_aspect_direct_deps(&mut t, &toolchain_aspect)
            .iter()
            .filter_map(|d| d.downcast::<ConfiguredTargetKey>())
            .map(|d| d.get_label().to_string())
            .collect();
    assert_contains_exactly(
        &aspect_configured_target_deps,
        &["//toolchain:foo_with_provider", "//test:t1"],
    );
}

#[test]
fn aspect_has_toolchains_dependency_edge_created_auto_exec_groups_true() {
    aspect_has_toolchains_dependency_edge_created("True");
}

#[test]
fn aspect_has_toolchains_dependency_edge_created_auto_exec_groups_false() {
    aspect_has_toolchains_dependency_edge_created("False");
}

// Java: StarlarkAspectsToolchainPropagationTest.aspectPropagatesToToolchainUsingToolchainTypeAlias
fn aspect_propagates_to_toolchain_using_toolchain_type_alias(auto_exec_groups: &str) {
    let mut t = new_test();
    t.scratch.file(
        "test/defs.bzl",
        &[r##"def _impl(target, ctx):
  return []

toolchain_aspect = aspect(
  implementation = _impl,
  toolchains_aspects = ['//rule:toolchain_type_1'],
)

no_toolchain_aspect = aspect(
  implementation = _impl,
)

def _rule_impl(ctx):
  pass

r1 = rule(
  implementation = _rule_impl,
  toolchains = ['//rule:toolchain_type_1_alias'],
)
"##],
    );
    t.scratch.file(
        "test/BUILD",
        &[r##"load('//test:defs.bzl', 'r1')
r1(name = 't1')
"##],
    );
    t.use_configuration(&[
        "--extra_toolchains=//toolchain:foo_toolchain",
        &format!("--incompatible_auto_exec_groups={auto_exec_groups}"),
    ]);

    let _unused = t
        .update_with_aspects(
            &[
                "//test:defs.bzl%toolchain_aspect",
                "//test:defs.bzl%no_toolchain_aspect",
            ],
            &["//test:t1"],
        )
        .unwrap();

    assert_toolchain_aspect_adds_single_dep(&mut t);
}

#[test]
fn aspect_propagates_to_toolchain_using_toolchain_type_alias_auto_exec_groups_true() {
    aspect_propagates_to_toolchain_using_toolchain_type_alias("True");
}

#[test]
fn aspect_propagates_to_toolchain_using_toolchain_type_alias_auto_exec_groups_false() {
    aspect_propagates_to_toolchain_using_toolchain_type_alias("False");
}

// Java: StarlarkAspectsToolchainPropagationTest.toolchainPropagationBasedOnAspectRequiredProviders
fn toolchain_propagation_based_on_aspect_required_providers(auto_exec_groups: &str) {
    let mut t = new_test();
    t.scratch.file(
        "test/defs.bzl",
        &[r##"load("//rule:test_toolchain.bzl", "MyProvider")

def _impl(target, ctx):
  return []

toolchain_aspect = aspect(
  implementation = _impl,
  toolchains_aspects = ['//rule:toolchain_type_1', '//rule:toolchain_type_2'],
  required_providers = [MyProvider],
)

def _rule_impl(ctx):
  return [MyProvider()]

r1 = rule(
  implementation = _rule_impl,
  toolchains = ['//rule:toolchain_type_1'],
  exec_groups = {"gp": exec_group(toolchains = ['//rule:toolchain_type_2'])},
  provides = [MyProvider],
)
"##],
    );
    t.scratch.file(
        "test/BUILD",
        &[r##"load('//test:defs.bzl', 'r1')
r1(name = 't1')
"##],
    );
    t.use_configuration(&[
        "--extra_toolchains=//toolchain:foo_toolchain,//toolchain:foo_toolchain_with_provider",
        &format!("--incompatible_auto_exec_groups={auto_exec_groups}"),
    ]);

    let _unused = t
        .update_with_aspects(&["//test:defs.bzl%toolchain_aspect"], &["//test:t1"])
        .unwrap();

    let aspect_on_target = get_only_element(get_aspect_keys(
        &mut t,
        "//test:t1",
        "//test:defs.bzl%toolchain_aspect",
    ));
    assert_node_exists(&mut t, &aspect_on_target);

    // aspect propagated only to //toolchain:foo_with_provider
    let aspect_on_toolchain = get_only_element(aspect_keys_of(&get_aspect_direct_deps(
        &mut t,
        &aspect_on_target,
    )));
    assert_eq!(
        aspect_on_toolchain.get_label().to_string(),
        "//toolchain:foo_with_provider"
    );
    assert_eq!(
        aspect_on_toolchain.get_aspect_name(),
        "//test:defs.bzl%toolchain_aspect"
    );
}

#[test]
fn toolchain_propagation_based_on_aspect_required_providers_auto_exec_groups_true() {
    toolchain_propagation_based_on_aspect_required_providers("True");
}

#[test]
fn toolchain_propagation_based_on_aspect_required_providers_auto_exec_groups_false() {
    toolchain_propagation_based_on_aspect_required_providers("False");
}

// Java: StarlarkAspectsToolchainPropagationTest.aspectPropagatesToToolchainDeps
#[test]
#[ignore = "BLAZE GAP: unported License as a Starlark value: building ctx.rule.attr of the filegroup //toolchain:toolchain_dep converts its `licenses` attribute (Java Attribute.valueToStarlark returns the License StarlarkValue; blaze_packages attribute.rs Attribute::value_to_starlark panics)"]
fn aspect_propagates_to_toolchain_deps() {
    let mut t = new_test();
    t.scratch.file(
        "test/defs.bzl",
        &[r##"def _impl(target, ctx):
  return []

toolchain_aspect = aspect(
  implementation = _impl,
  toolchains_aspects = ['//rule:toolchain_type_3'],
  attr_aspects = ['toolchain_dep'],
)

def _rule_impl(ctx):
  pass

r1 = rule(
  implementation = _rule_impl,
  exec_groups = {"gp": exec_group(toolchains = ['//rule:toolchain_type_3'])},
)
"##],
    );
    t.scratch.file(
        "test/BUILD",
        &[r##"load('//test:defs.bzl', 'r1')
r1(name = 't1')
"##],
    );
    t.use_configuration(&["--extra_toolchains=//toolchain:foo_toolchain_with_dep"]);

    let _unused = t
        .update_with_aspects(&["//test:defs.bzl%toolchain_aspect"], &["//test:t1"])
        .unwrap();

    let aspect_on_target = get_only_element(get_aspect_keys(
        &mut t,
        "//test:t1",
        "//test:defs.bzl%toolchain_aspect",
    ));
    assert_node_exists(&mut t, &aspect_on_target);

    let aspect_on_toolchain = get_only_element(aspect_keys_of(&get_aspect_direct_deps(
        &mut t,
        &aspect_on_target,
    )));
    assert_node_exists(&mut t, &aspect_on_toolchain);
    assert_eq!(
        aspect_on_toolchain.get_label().to_string(),
        "//toolchain:foo_with_dep"
    );

    let aspect_on_toolchain_dep = get_only_element(aspect_keys_of(&get_aspect_direct_deps(
        &mut t,
        &aspect_on_toolchain,
    )));
    assert_eq!(
        aspect_on_toolchain_dep.get_label().to_string(),
        "//toolchain:toolchain_dep"
    );
    assert_eq!(
        aspect_on_toolchain_dep.get_aspect_name(),
        "//test:defs.bzl%toolchain_aspect"
    );
}

// Java: StarlarkAspectsToolchainPropagationTest.requiredAspectPropagatesToToolchain
#[test]
fn required_aspect_propagates_to_toolchain() {
    let mut t = new_test();
    t.scratch.file(
        "test/defs.bzl",
        &[r##"def _impl(target, ctx):
  return []

required_aspect = aspect(implementation = _impl)

toolchain_aspect = aspect(
  implementation = _impl,
  toolchains_aspects = ['//rule:toolchain_type_1'],
  requires = [required_aspect],
)

def _rule_impl(ctx):
  pass

r1 = rule(
  implementation = _rule_impl,
  exec_groups = {"gp": exec_group(toolchains = ['//rule:toolchain_type_1'])},
)
"##],
    );
    t.scratch.file(
        "test/BUILD",
        &[r##"load('//test:defs.bzl', 'r1')
r1(name = 't1')
"##],
    );
    t.use_configuration(&["--extra_toolchains=//toolchain:foo_toolchain"]);

    let _unused = t
        .update_with_aspects(&["//test:defs.bzl%toolchain_aspect"], &["//test:t1"])
        .unwrap();

    let aspect_on_target = get_only_element(get_aspect_keys(
        &mut t,
        "//test:t1",
        "//test:defs.bzl%toolchain_aspect",
    ));
    assert_node_exists(&mut t, &aspect_on_target);

    let aspects_deps = aspect_names_on(&aspect_keys_of(&get_aspect_direct_deps(
        &mut t,
        &aspect_on_target,
    )));
    assert_eq!(aspects_deps.len(), 3, "{aspects_deps:?}");
    // toolchain_aspect requires required_aspect so required_aspect will be propagated before
    // toolchain_aspect to //test:t1 and its toolchain
    assert_contains_exactly(
        &aspects_deps,
        &[
            "//test:defs.bzl%required_aspect on //test:t1",
            "//test:defs.bzl%toolchain_aspect on //toolchain:foo",
            "//test:defs.bzl%required_aspect on //toolchain:foo",
        ],
    );
}

// Java: StarlarkAspectsToolchainPropagationTest.aspectOnAspectPropagateToToolchain
fn aspect_on_aspect_propagate_to_toolchain(auto_exec_groups: &str) {
    let mut t = new_test();
    t.scratch.file(
        "test/defs.bzl",
        &[r##"Prov1 = provider()
Prov2 = provider()

def _impl(target, ctx):
  return []

def _impl_1(target, ctx):
  return [Prov1()]

def _impl_2(target, ctx):
  return [Prov2()]

toolchain_aspect_1 = aspect(
  implementation = _impl,
  toolchains_aspects = ['//rule:toolchain_type_1'],
  required_aspect_providers = [Prov1]
)

no_toolchain_aspect = aspect(
  implementation = _impl_1,
  provides = [Prov1],
  required_aspect_providers = [Prov2]
)

toolchain_aspect_2 = aspect(
  implementation = _impl_2,
  toolchains_aspects = ['//rule:toolchain_type_1'],
  provides = [Prov2],
)

def _rule_impl(ctx):
  pass

r1 = rule(
  implementation = _rule_impl,
  toolchains = ['//rule:toolchain_type_1'],
)
"##],
    );
    t.scratch.file(
        "test/BUILD",
        &[r##"load('//test:defs.bzl', 'r1')
r1(name = 't1')
"##],
    );
    t.use_configuration(&[
        "--extra_toolchains=//toolchain:foo_toolchain",
        &format!("--incompatible_auto_exec_groups={auto_exec_groups}"),
    ]);

    let _unused = t
        .update_with_aspects(
            &[
                "//test:defs.bzl%toolchain_aspect_2",
                "//test:defs.bzl%no_toolchain_aspect",
                "//test:defs.bzl%toolchain_aspect_1",
            ],
            &["//test:t1"],
        )
        .unwrap();

    let aspect_on_target = get_only_element(get_aspect_keys(
        &mut t,
        "//test:t1",
        "//test:defs.bzl%toolchain_aspect_1",
    ));
    assert_eq!(aspect_on_target.get_base_keys().len(), 1);
    assert_eq!(
        aspect_on_target.get_base_keys()[0].get_aspect_name(),
        "//test:defs.bzl%no_toolchain_aspect"
    );

    assert_node_exists(&mut t, &aspect_on_target);

    let deps = aspect_keys_of(&get_aspect_direct_deps(&mut t, &aspect_on_target));
    let aspects_on_toolchain = aspect_names_on(&deps);
    assert_eq!(aspects_on_toolchain.len(), 4, "{aspects_on_toolchain:?}");
    // Only `toolchain_aspect_1` and `toolchain_aspect_2` are propagated to the toolchain
    assert_contains_exactly(
        &aspects_on_toolchain,
        &[
            "//test:defs.bzl%toolchain_aspect_2 on //test:t1",
            "//test:defs.bzl%no_toolchain_aspect on //test:t1",
            "//test:defs.bzl%toolchain_aspect_1 on //toolchain:foo",
            "//test:defs.bzl%toolchain_aspect_2 on //toolchain:foo",
        ],
    );

    let toolchain_aspect1 = get_only_element(
        deps.iter()
            .filter(|k| {
                k.get_aspect_name() == "//test:defs.bzl%toolchain_aspect_1"
                    && k.get_label().to_string() == "//toolchain:foo"
            })
            .cloned()
            .collect(),
    );
    // Since `toolchain_aspect_1` only depends on `no_toolchain_aspect`, it will have no base keys
    // when applied on the toolchain.
    assert!(toolchain_aspect1.get_base_keys().is_empty());
}

#[test]
fn aspect_on_aspect_propagate_to_toolchain_auto_exec_groups_true() {
    aspect_on_aspect_propagate_to_toolchain("True");
}

#[test]
fn aspect_on_aspect_propagate_to_toolchain_auto_exec_groups_false() {
    aspect_on_aspect_propagate_to_toolchain("False");
}

// Java: StarlarkAspectsToolchainPropagationTest.execGroupWithMultipleToolchainTypes_aspectsPropagateToRelevantTypes
#[test]
fn exec_group_with_multiple_toolchain_types_aspects_propagate_to_relevant_types() {
    let mut t = new_test();
    t.scratch.file(
        "test/defs.bzl",
        &[r##"Prov1 = provider()
Prov2 = provider()

def _impl(target, ctx):
  return []

def _impl_1(target, ctx):
  return [Prov1()]

def _impl_2(target, ctx):
  return [Prov2()]

toolchain_aspect_0 = aspect(
  implementation = _impl,
  toolchains_aspects = ['//rule:toolchain_type_1'],
  required_aspect_providers = [[Prov1], [Prov2]]
)

toolchain_aspect_1 = aspect(
  implementation = _impl_1,
  toolchains_aspects = ['//rule:toolchain_type_3'],
  provides = [Prov1],
  required_aspect_providers = [Prov2]
)

toolchain_aspect_2 = aspect(
  implementation = _impl_2,
  toolchains_aspects = ['//rule:toolchain_type_1'],
  provides = [Prov2],
)

def _rule_impl(ctx):
  pass

r1 = rule(
  implementation = _rule_impl,
  attrs = {
    'dep': attr.label(),
  },
  exec_groups = {"gp": exec_group(
      toolchains = ['//rule:toolchain_type_1', '//rule:toolchain_type_3'])},
)
"##],
    );
    t.scratch.file(
        "test/BUILD",
        &[r##"load('//test:defs.bzl', 'r1')
r1(name = 't1')
"##],
    );
    t.use_configuration(&[
        "--extra_toolchains=//toolchain:foo_toolchain,//toolchain:foo_toolchain_with_dep",
    ]);

    let _unused = t
        .update_with_aspects(
            &[
                "//test:defs.bzl%toolchain_aspect_2",
                "//test:defs.bzl%toolchain_aspect_1",
                "//test:defs.bzl%toolchain_aspect_0",
            ],
            &["//test:t1"],
        )
        .unwrap();

    let aspect_on_target = get_only_element(get_aspect_keys(
        &mut t,
        "//test:t1",
        "//test:defs.bzl%toolchain_aspect_0",
    ));
    assert_node_exists(&mut t, &aspect_on_target);

    let deps = aspect_keys_of(&get_aspect_direct_deps(&mut t, &aspect_on_target));
    let aspects_on_toolchain = aspect_names_on(&deps);
    assert_eq!(aspects_on_toolchain.len(), 5, "{aspects_on_toolchain:?}");
    assert_contains_exactly(
        &aspects_on_toolchain,
        &[
            "//test:defs.bzl%toolchain_aspect_1 on //test:t1",
            "//test:defs.bzl%toolchain_aspect_2 on //test:t1",
            // toolchain_aspect_0 and toolchain_aspect_2 propagate to //toolchain:foo of
            // //rule:toolchain_type_1
            "//test:defs.bzl%toolchain_aspect_0 on //toolchain:foo",
            "//test:defs.bzl%toolchain_aspect_2 on //toolchain:foo",
            // toolchain_aspect_1 propagates to //toolchain:foo_with_dep of //rule:toolchain_type_3
            "//test:defs.bzl%toolchain_aspect_1 on //toolchain:foo_with_dep",
        ],
    );

    let toolchain_aspect1 = get_only_element(
        deps.iter()
            .filter(|k| {
                k.get_aspect_name() == "//test:defs.bzl%toolchain_aspect_0"
                    && k.get_label().to_string() == "//toolchain:foo"
            })
            .cloned()
            .collect(),
    );
    // Since `toolchain_aspect_0` depends on `toolchain_aspect_2` when applied on //toolchain:foo,
    assert_eq!(
        get_only_element(toolchain_aspect1.get_base_keys().to_vec()).get_aspect_name(),
        "//test:defs.bzl%toolchain_aspect_2"
    );
}

// Java: StarlarkAspectsToolchainPropagationTest.toolchainTypesResolvedToSameToolchain_aspectsPropagateToSameToolchain
#[test]
fn toolchain_types_resolved_to_same_toolchain_aspects_propagate_to_same_toolchain() {
    let mut t = new_test();
    t.scratch.file(
        "test/defs.bzl",
        &[r##"prov = provider()

def _impl(target, ctx):
  return []

def _impl_1(target, ctx):
  return [prov()]

toolchain_aspect_1 = aspect(
  implementation = _impl,
  toolchains_aspects = ['//rule:toolchain_type_2'],
  required_aspect_providers = [prov]
)

toolchain_aspect_2 = aspect(
  implementation = _impl_1,
  toolchains_aspects = ['//rule:toolchain_type_3'],
  provides = [prov],
)

def _rule_impl(ctx):
  pass

r1 = rule(
  implementation = _rule_impl,
  exec_groups = {"gp": exec_group(
      toolchains = ['//rule:toolchain_type_2', '//rule:toolchain_type_3'])},
)
"##],
    );
    t.scratch.file(
        "test/BUILD",
        &[r##"load('//test:defs.bzl', 'r1')
r1(name = 't1')
"##],
    );
    t.use_configuration(&[
        "--extra_toolchains=//toolchain:foo_type_2",
        "--extra_toolchains=//toolchain:foo_type_3",
    ]);

    let _unused = t
        .update_with_aspects(
            &[
                "//test:defs.bzl%toolchain_aspect_2",
                "//test:defs.bzl%toolchain_aspect_1",
            ],
            &["//test:t1"],
        )
        .unwrap();

    let aspect_on_target = get_only_element(get_aspect_keys(
        &mut t,
        "//test:t1",
        "//test:defs.bzl%toolchain_aspect_1",
    ));
    assert_node_exists(&mut t, &aspect_on_target);

    let aspects_on_toolchain = aspect_names_on(&aspect_keys_of(&get_aspect_direct_deps(
        &mut t,
        &aspect_on_target,
    )));
    assert_eq!(aspects_on_toolchain.len(), 3, "{aspects_on_toolchain:?}");

    assert_contains_exactly(
        &aspects_on_toolchain,
        &[
            "//test:defs.bzl%toolchain_aspect_2 on //test:t1",
            // both aspects propagated to //toolchain:foo_for_all because it resolves both the
            // toolchain types
            "//test:defs.bzl%toolchain_aspect_1 on //toolchain:foo_for_all",
            "//test:defs.bzl%toolchain_aspect_2 on //toolchain:foo_for_all",
        ],
    );
}

// Java: StarlarkAspectsToolchainPropagationTest.toolchainTypesResolvedToSameToolchainDiffExecPlatform_aspectPropagateTwice
#[test]
fn toolchain_types_resolved_to_same_toolchain_diff_exec_platform_aspect_propagate_twice() {
    let mut t = new_test();
    t.scratch.file(
        "test/defs.bzl",
        &[r##"def _impl(target, ctx):
  return []

toolchain_aspect = aspect(
  implementation = _impl,
  toolchains_aspects = ['//rule:toolchain_type_1'],
)

def _rule_impl(ctx):
  pass

r1 = rule(
  implementation = _rule_impl,
  exec_groups = {
    "gp1": exec_group(
      toolchains = ['//rule:toolchain_type_1'],
      exec_compatible_with = ['//platforms:constraint_2']
      ),
    "gp2": exec_group(toolchains = ['//rule:toolchain_type_1'])},
)
"##],
    );
    t.scratch.file(
        "test/BUILD",
        &[r##"load('//test:defs.bzl', 'r1')
r1(name = 't1')
"##],
    );
    t.use_configuration(&[
        "--extra_toolchains=//toolchain:foo_toolchain",
        "--extra_execution_platforms=//platforms:platform_1,//platforms:platform_2",
    ]);

    let _unused = t
        .update_with_aspects(&["//test:defs.bzl%toolchain_aspect"], &["//test:t1"])
        .unwrap();

    let aspect_on_target = get_only_element(get_aspect_keys(
        &mut t,
        "//test:t1",
        "//test:defs.bzl%toolchain_aspect",
    ));
    assert_node_exists(&mut t, &aspect_on_target);

    let aspects_on_toolchain: Vec<String> =
        aspect_keys_of(&get_aspect_direct_deps(&mut t, &aspect_on_target))
            .iter()
            .map(|k| {
                format!(
                    "{} on {}, exec_platform: {}",
                    k.get_aspect_name(),
                    k.get_label(),
                    label_or_null(
                        k.get_base_configured_target_key()
                            .get_execution_platform_label()
                    )
                )
            })
            .collect();
    assert_eq!(aspects_on_toolchain.len(), 2, "{aspects_on_toolchain:?}");
    // aspect propagated twice on the same toolchain target but with different execution platform
    assert_contains_exactly(
        &aspects_on_toolchain,
        &[
            "//test:defs.bzl%toolchain_aspect on //toolchain:foo, exec_platform: //platforms:platform_2",
            "//test:defs.bzl%toolchain_aspect on //toolchain:foo, exec_platform: //platforms:platform_1",
        ],
    );
}

// Java: StarlarkAspectsToolchainPropagationTest.aspectPropagatesToToolchain_providersCollected
fn aspect_propagates_to_toolchain_providers_collected(auto_exec_groups: &str) {
    let mut t = new_test();
    t.scratch.file(
        "test/defs.bzl",
        &[r##"AspectProvider = provider()
def _impl(target, ctx):
  target_res = "toolchain_aspect has param = " + ctx.attr.param
  target_res += " on " + str(target.label)
  if platform_common.ToolchainInfo in target:
    target_res += " with tool in ToolchainInfo = "
    target_res += str(target[platform_common.ToolchainInfo].tool)

  result = [target_res]
  if ctx.rule.toolchains and '//rule:toolchain_type_1' in ctx.rule.toolchains:
      result.extend(ctx.rule.toolchains['//rule:toolchain_type_1'][AspectProvider].value)
  return [AspectProvider(value = result)]

toolchain_aspect = aspect(
  implementation = _impl,
  toolchains_aspects = ['//rule:toolchain_type_1'],
  attrs = {
    "param": attr.string(),
  },
)

def _rule_impl(ctx):
  pass

r1 = rule(
  implementation = _rule_impl,
  toolchains = ['//rule:toolchain_type_1'],
)
"##],
    );
    t.scratch.file(
        "test/BUILD",
        &[r##"load('//test:defs.bzl', 'r1')
r1(name = 't1')
"##],
    );
    t.use_configuration(&[
        "--extra_toolchains=//toolchain:foo_toolchain",
        &format!("--incompatible_auto_exec_groups={auto_exec_groups}"),
    ]);

    let analysis_result = update_with_aspects_parameters(
        &mut t,
        &["//test:defs.bzl%toolchain_aspect"],
        &[("param", "xxx")],
        &["//test:t1"],
    )
    .unwrap();

    let configured_aspect = get_only_element(
        analysis_result
            .get_aspects_map()
            .values()
            .cloned()
            .collect(),
    );

    let value = get_sequence(
        &get_starlark_provider(&configured_aspect, "//test:defs.bzl", "AspectProvider")
            .expect("AspectProvider"),
        "value",
    );
    assert_contains_exactly(
        &value,
        &[
            "toolchain_aspect has param = xxx on @@//test:t1",
            "toolchain_aspect has param = xxx on @@//toolchain:foo with tool in ToolchainInfo = <generated file toolchain/atool>",
        ],
    );
}

#[test]
fn aspect_propagates_to_toolchain_providers_collected_auto_exec_groups_true() {
    aspect_propagates_to_toolchain_providers_collected("True");
}

#[test]
fn aspect_propagates_to_toolchain_providers_collected_auto_exec_groups_false() {
    aspect_propagates_to_toolchain_providers_collected("False");
}

// Java: StarlarkAspectsToolchainPropagationTest.aspectPropagatesToExecGpToolchain_providersCollected
#[test]
fn aspect_propagates_to_exec_gp_toolchain_providers_collected() {
    let mut t = new_test();
    t.scratch.file(
        "test/defs.bzl",
        &[r##"AspectProvider = provider()

def _impl(target, ctx):
  target_res = "toolchain_aspect has param = " + ctx.attr.param
  target_res += " on " + str(target.label)
  if platform_common.ToolchainInfo in target:
    target_res += " with tool in ToolchainInfo = "
    target_res += str(target[platform_common.ToolchainInfo].tool)

  result = [target_res]
  if ctx.rule.exec_groups and 'gp' in ctx.rule.exec_groups:
      result.extend(
          ctx.
          rule.
          exec_groups['gp'].
          toolchains['//rule:toolchain_type_1'][AspectProvider].value)

  return [AspectProvider(value = result)]

toolchain_aspect = aspect(
  implementation = _impl,
  toolchains_aspects = ['//rule:toolchain_type_1'],
  attrs = {
    "param": attr.string(),
  },
)

def _rule_impl(ctx):
  pass

r1 = rule(
  implementation = _rule_impl,
  exec_groups = {"gp": exec_group(toolchains = ['//rule:toolchain_type_1'])},
)
"##],
    );
    t.scratch.file(
        "test/BUILD",
        &[r##"load('//test:defs.bzl', 'r1')
r1(name = 't1')
"##],
    );
    t.use_configuration(&["--extra_toolchains=//toolchain:foo_toolchain"]);

    let analysis_result = update_with_aspects_parameters(
        &mut t,
        &["//test:defs.bzl%toolchain_aspect"],
        &[("param", "xxx")],
        &["//test:t1"],
    )
    .unwrap();

    let configured_aspect = get_only_element(
        analysis_result
            .get_aspects_map()
            .values()
            .cloned()
            .collect(),
    );

    let value = get_sequence(
        &get_starlark_provider(&configured_aspect, "//test:defs.bzl", "AspectProvider")
            .expect("AspectProvider"),
        "value",
    );
    assert_contains_exactly(
        &value,
        &[
            "toolchain_aspect has param = xxx on @@//test:t1",
            "toolchain_aspect has param = xxx on @@//toolchain:foo with tool in ToolchainInfo = <generated file toolchain/atool>",
        ],
    );
}

// Java: StarlarkAspectsToolchainPropagationTest.aspectPropagatesToToolchainUsingAlias_providersCollected
fn aspect_propagates_to_toolchain_using_alias_providers_collected(auto_exec_groups: &str) {
    let mut t = new_test();
    t.scratch.file(
        "test/defs.bzl",
        &[r##"AspectProvider = provider()
def _impl(target, ctx):
  target_res = "toolchain_aspect has param = " + ctx.attr.param
  target_res += " on " + str(target.label)
  if platform_common.ToolchainInfo in target:
    target_res += " with tool in ToolchainInfo = "
    target_res += str(target[platform_common.ToolchainInfo].tool)

  result = [target_res]
  if ctx.rule.toolchains and '//rule:toolchain_type_1_alias' in ctx.rule.toolchains:
      result.extend(
          ctx.rule.toolchains['//rule:toolchain_type_1_alias'][AspectProvider].value)
  return [AspectProvider(value = result)]

toolchain_aspect = aspect(
  implementation = _impl,
  toolchains_aspects = ['//rule:toolchain_type_1_alias'],
  attrs = {
    "param": attr.string(),
  },
)

def _rule_impl(ctx):
  pass

r1 = rule(
  implementation = _rule_impl,
  toolchains = ['//rule:toolchain_type_1_alias'],
)
"##],
    );
    t.scratch.file(
        "test/BUILD",
        &[r##"load('//test:defs.bzl', 'r1')
r1(name = 't1')
"##],
    );
    t.use_configuration(&[
        "--extra_toolchains=//toolchain:foo_toolchain",
        &format!("--incompatible_auto_exec_groups={auto_exec_groups}"),
    ]);

    let analysis_result = update_with_aspects_parameters(
        &mut t,
        &["//test:defs.bzl%toolchain_aspect"],
        &[("param", "xxx")],
        &["//test:t1"],
    )
    .unwrap();

    let configured_aspect = get_only_element(
        analysis_result
            .get_aspects_map()
            .values()
            .cloned()
            .collect(),
    );

    let value = get_sequence(
        &get_starlark_provider(&configured_aspect, "//test:defs.bzl", "AspectProvider")
            .expect("AspectProvider"),
        "value",
    );
    assert_contains_exactly(
        &value,
        &[
            "toolchain_aspect has param = xxx on @@//test:t1",
            "toolchain_aspect has param = xxx on @@//toolchain:foo with tool in ToolchainInfo = <generated file toolchain/atool>",
        ],
    );
}

#[test]
fn aspect_propagates_to_toolchain_using_alias_providers_collected_auto_exec_groups_true() {
    aspect_propagates_to_toolchain_using_alias_providers_collected("True");
}

#[test]
fn aspect_propagates_to_toolchain_using_alias_providers_collected_auto_exec_groups_false() {
    aspect_propagates_to_toolchain_using_alias_providers_collected("False");
}

// Java: StarlarkAspectsToolchainPropagationTest.aspectPropagatesToToolchain_cannotSeeToolchainInfoOfDeps
#[test]
fn aspect_propagates_to_toolchain_cannot_see_toolchain_info_of_deps() {
    let mut t = new_test();
    t.scratch.file(
        "test/defs.bzl",
        &[r##"AspectProvider = provider()
def _impl(target, ctx):
  if ctx.rule.toolchains and '//rule:toolchain_type_1' in ctx.rule.toolchains:
      print(ctx.rule.toolchains['//rule:toolchain_type_1'][platform_common.ToolchainInfo])

  return [AspectProvider(value = [])]

toolchain_aspect = aspect(
  implementation = _impl,
  toolchains_aspects = ['//rule:toolchain_type_1'],
)

def _rule_impl(ctx):
  pass

r1 = rule(
  implementation = _rule_impl,
  toolchains = ['//rule:toolchain_type_1'],
)
"##],
    );
    t.scratch.file(
        "test/BUILD",
        &[r##"load('//test:defs.bzl', 'r1')
r1(name = 't1')
"##],
    );
    t.use_configuration(&["--extra_toolchains=//toolchain:foo_toolchain"]);

    t.remove_fail_fast_handler();
    // expect to fail
    let _ = t.update_with_aspects(&["//test:defs.bzl%toolchain_aspect"], &["//test:t1"]);
    t.assert_contains_event(
        "<ToolchainAspectsProviders for toolchain target: //toolchain:foo> doesn't contain declared provider 'ToolchainInfo'",
    );
}

// Java: StarlarkAspectsToolchainPropagationTest.aspectDoesNotPropagatesToToolchain_cannotSeeTargetToolchains
fn aspect_does_not_propagates_to_toolchain_cannot_see_target_toolchains(auto_exec_groups: bool) {
    let mut t = new_test();
    t.scratch.file(
        "test/defs.bzl",
        &[r##"AspectProvider = provider()
def _impl(target, ctx):
  print(ctx.rule.toolchains['//rule:toolchain_type_1'][AspectProvider])
  return [AspectProvider(value = [])]

non_toolchain_aspect = aspect(
  implementation = _impl,
)

def _rule_impl(ctx):
  pass

r1 = rule(
  implementation = _rule_impl,
  toolchains = ['//rule:toolchain_type_1'],
)
"##],
    );
    t.scratch.file(
        "test/BUILD",
        &[r##"load('//test:defs.bzl', 'r1')
r1(name = 't1')
"##],
    );
    t.use_configuration(&[
        "--extra_toolchains=//toolchain:foo_toolchain",
        &format!("--incompatible_auto_exec_groups={auto_exec_groups}"),
    ]);

    t.remove_fail_fast_handler();
    let result = t.update_with_aspects(&["//test:defs.bzl%non_toolchain_aspect"], &["//test:t1"]);
    assert!(
        matches!(result, Err(BuildViewError::ViewCreationFailed { .. })),
        "expected ViewCreationFailedException"
    );
    t.assert_contains_event(
        "Error: <ToolchainAspectsProviders for toolchain target: //toolchain:foo> doesn't contain declared provider 'AspectProvider'",
    );
}

#[test]
fn aspect_does_not_propagates_to_toolchain_cannot_see_target_toolchains_auto_exec_groups_true() {
    aspect_does_not_propagates_to_toolchain_cannot_see_target_toolchains(true);
}

#[test]
fn aspect_does_not_propagates_to_toolchain_cannot_see_target_toolchains_auto_exec_groups_false() {
    aspect_does_not_propagates_to_toolchain_cannot_see_target_toolchains(false);
}

// Java: StarlarkAspectsToolchainPropagationTest.aspectDoesNotPropagatesToToolchain_cannotSeeTargetExecGroups
fn aspect_does_not_propagates_to_toolchain_cannot_see_target_exec_groups(auto_exec_groups: bool) {
    let mut t = new_test();
    t.scratch.file(
        "test/defs.bzl",
        &[r##"AspectProvider = provider()
def _impl(target, ctx):
  print(ctx.rule.exec_groups['gp'].toolchains['//rule:toolchain_type_1'][AspectProvider])
  return [AspectProvider(value = [])]

non_toolchain_aspect = aspect(
  implementation = _impl,
)

def _rule_impl(ctx):
  pass

r1 = rule(
  implementation = _rule_impl,
  exec_groups = {"gp": exec_group(toolchains = ['//rule:toolchain_type_1'])},
)
"##],
    );
    t.scratch.file(
        "test/BUILD",
        &[r##"load('//test:defs.bzl', 'r1')
r1(name = 't1')
"##],
    );
    t.use_configuration(&[
        "--extra_toolchains=//toolchain:foo_toolchain",
        &format!("--incompatible_auto_exec_groups={auto_exec_groups}"),
    ]);

    t.remove_fail_fast_handler();
    let result = t.update_with_aspects(&["//test:defs.bzl%non_toolchain_aspect"], &["//test:t1"]);
    assert!(
        matches!(result, Err(BuildViewError::ViewCreationFailed { .. })),
        "expected ViewCreationFailedException"
    );
    t.assert_contains_event(
        "Error: <ToolchainAspectsProviders for toolchain target: //toolchain:foo> doesn't contain declared provider 'AspectProvider'",
    );
}

#[test]
fn aspect_does_not_propagates_to_toolchain_cannot_see_target_exec_groups_auto_exec_groups_true() {
    aspect_does_not_propagates_to_toolchain_cannot_see_target_exec_groups(true);
}

#[test]
fn aspect_does_not_propagates_to_toolchain_cannot_see_target_exec_groups_auto_exec_groups_false() {
    aspect_does_not_propagates_to_toolchain_cannot_see_target_exec_groups(false);
}

// Java: StarlarkAspectsToolchainPropagationTest.requiredAspectPropagatesToToolchain_providersCollected
#[test]
fn required_aspect_propagates_to_toolchain_providers_collected() {
    let mut t = new_test();
    t.scratch.file(
        "test/defs.bzl",
        &[r##"ToolchainAspectProvider = provider()
RequiredAspectProvider = provider()

def _toolchain_aspect_impl(target, ctx):
  target_res = "toolchain_aspect on " + str(target.label)
  target_res += " can see required_aspect (" + target[RequiredAspectProvider].value + ")"

  result = [target_res]
  if ctx.rule.toolchains and '//rule:toolchain_type_1' in ctx.rule.toolchains:
      result.extend(
          ctx.rule.toolchains['//rule:toolchain_type_1'][ToolchainAspectProvider].value)
  return [ToolchainAspectProvider(value = result)]

def _required_aspect_impl(target, ctx):
  target_res = "required_aspect on " + str(target.label)
  if platform_common.ToolchainInfo in target:
    target_res += " with tool in ToolchainInfo = "
    target_res += str(target[platform_common.ToolchainInfo].tool)

  return [RequiredAspectProvider(value = target_res)]

required_aspect = aspect(implementation = _required_aspect_impl)

toolchain_aspect = aspect(
  implementation = _toolchain_aspect_impl,
  toolchains_aspects = ['//rule:toolchain_type_1'],
  requires = [required_aspect],
)

def _rule_impl(ctx):
  pass

r1 = rule(
  implementation = _rule_impl,
  toolchains = ['//rule:toolchain_type_1'],
)
"##],
    );
    t.scratch.file(
        "test/BUILD",
        &[r##"load('//test:defs.bzl', 'r1')
r1(name = 't1')
"##],
    );
    t.use_configuration(&["--extra_toolchains=//toolchain:foo_toolchain"]);

    let aspects_map = t
        .update_with_aspects(&["//test:defs.bzl%toolchain_aspect"], &["//test:t1"])
        .unwrap()
        .get_aspects_map()
        .clone();

    let key = get_aspect_keys(&mut t, "//test:t1", "//test:defs.bzl%toolchain_aspect")[0].clone();
    let configured_aspect = aspects_map.get(&key).expect("aspect in the aspects map");

    let value = get_sequence(
        &get_starlark_provider(
            configured_aspect,
            "//test:defs.bzl",
            "ToolchainAspectProvider",
        )
        .expect("ToolchainAspectProvider"),
        "value",
    );
    assert_contains_exactly(
        &value,
        &[
            "toolchain_aspect on @@//test:t1 can see required_aspect (required_aspect on @@//test:t1)",
            "toolchain_aspect on @@//toolchain:foo can see required_aspect (required_aspect on @@//toolchain:foo with tool in ToolchainInfo = <generated file toolchain/atool>)",
        ],
    );
}

// Java: StarlarkAspectsToolchainPropagationTest.aspectPropagatesToToolchainFromRule_providersCollected
#[test]
#[ignore = "BLAZE GAP: unported License as a Starlark value: building ctx.rule.attr of the filegroup //toolchain:toolchain_dep converts its `licenses` attribute (Java Attribute.valueToStarlark returns the License StarlarkValue; blaze_packages attribute.rs Attribute::value_to_starlark panics)"]
fn aspect_propagates_to_toolchain_from_rule_providers_collected() {
    let mut t = new_test();
    t.scratch.file(
        "test/defs.bzl",
        &[r##"AspectProvider = provider()
RuleProvider = provider()

def _impl(target, ctx):
  result = ["toolchain_aspect on " + str(target.label)]

  if ctx.rule.toolchains and '//rule:toolchain_type_3' in ctx.rule.toolchains:
      result.extend(
          ctx.rule.toolchains['//rule:toolchain_type_3'][AspectProvider].value)

  if hasattr(ctx.rule.attr, 'toolchain_dep'):
      result.extend(ctx.rule.attr.toolchain_dep[AspectProvider].value)

  return [AspectProvider(value = result)]

toolchain_aspect = aspect(
    implementation = _impl,
    toolchains_aspects = ['//rule:toolchain_type_3'],
    attr_aspects = ['toolchain_dep'],
)

def _rule_1_impl(ctx):
  return [RuleProvider(value = ctx.attr.rule_dep[AspectProvider].value)]

r1 = rule(
  implementation = _rule_1_impl,
  attrs = {
    "rule_dep": attr.label(aspects = [toolchain_aspect]),
  },
)

def _rule_2_impl(ctx):
  pass

r2 = rule(
  implementation = _rule_2_impl,
  toolchains = ['//rule:toolchain_type_3'],
)

"##],
    );
    t.scratch.file(
        "test/BUILD",
        &[r##"load('//test:defs.bzl', 'r1', 'r2')
r1(name = 't1', rule_dep = ':t2')
r2(name = 't2')
"##],
    );
    t.use_configuration(&["--extra_toolchains=//toolchain:foo_toolchain_with_dep"]);

    let analysis_result = t.update(&["//test:t1"]).unwrap();

    let configured_target = get_only_element(analysis_result.get_targets_to_build().to_vec());

    let value = get_sequence(
        &configured_target
            .get(&provider_key("//test:defs.bzl", "RuleProvider"))
            .expect("RuleProvider"),
        "value",
    );
    assert_contains_exactly(
        &value,
        &[
            "toolchain_aspect on @@//test:t2",
            "toolchain_aspect on @@//toolchain:foo_with_dep",
            "toolchain_aspect on @@//toolchain:toolchain_dep",
        ],
    );
}

// Java: StarlarkAspectsToolchainPropagationTest.toolchainAspectOnOutputFile_notPropagatedToDeps
#[test]
fn toolchain_aspect_on_output_file_not_propagated_to_deps() {
    let mut t = new_test();
    t.scratch.file(
        "test/defs.bzl",
        &[r##"AspectProvider = provider()
def _impl(target, ctx):
  return [AspectProvider(val="hi")]

toolchain_aspect = aspect(
    implementation = _impl,
    toolchains_aspects = ['//rule:toolchain_type_1'],
    attr_aspects = ['dep'],
)

def _rule_1_impl(ctx):
  if ctx.outputs.out:
    ctx.actions.write(ctx.outputs.out, 'hi')
  return []

r1 = rule(
  implementation = _rule_1_impl,
  attrs = {
    "out": attr.output(),
    "dep": attr.label(),
  },
  toolchains = ['//rule:toolchain_type_1'],
)
"##],
    );
    t.scratch.file(
        "test/BUILD",
        &[r##"load('//test:defs.bzl', 'r1')
r1(name = 't1', out = 'my_out.txt', dep = ':t2')
r1(name = 't2')
"##],
    );
    t.use_configuration(&["--extra_toolchains=//toolchain:foo_toolchain"]);

    let _unused = t
        .update_with_aspects(
            &["//test:defs.bzl%toolchain_aspect"],
            &["//test:my_out.txt"],
        )
        .unwrap();

    // {@link AspectKey} is created for toolchain_aspect on the output file //test:my_out.txt but
    // the aspect is not applied (no returned providers) because the aspect cannot be applied to
    // output files. The aspect does not propagate to any of the generating rule dependencies.
    let nodes: Vec<(AspectKey, Arc<AspectValue>)> = done_aspect_values(&mut t)
        .into_iter()
        .filter(|(k, _)| k.get_aspect_class().get_name() == "//test:defs.bzl%toolchain_aspect")
        .collect();
    assert_eq!(
        nodes.len(),
        1,
        "{:?}",
        nodes.iter().map(|n| &n.0).collect::<Vec<_>>()
    );

    let (aspect_key, aspect_value) = get_only_element(nodes);
    assert_eq!(aspect_key.get_label().to_string(), "//test:my_out.txt");

    assert_eq!(aspect_value.get_providers().get_provider_count(), 0);
}

// Java: StarlarkAspectsToolchainPropagationTest.toolchainAspectApplyToGeneratingRule_propagateToDeps
#[test]
fn toolchain_aspect_apply_to_generating_rule_propagate_to_deps() {
    let mut t = new_test();
    t.scratch.file(
        "test/defs.bzl",
        &[r##"def _impl(target, ctx):
  return []

toolchain_aspect = aspect(
    implementation = _impl,
    toolchains_aspects = ['//rule:toolchain_type_1'],
    attr_aspects = ['dep'],
    apply_to_generating_rules = True,
)

def _rule_1_impl(ctx):
  if ctx.outputs.out:
    ctx.actions.write(ctx.outputs.out, 'hi')
  return []

r1 = rule(
  implementation = _rule_1_impl,
  attrs = {
    "out": attr.output(),
    "dep": attr.label(),
  },
  toolchains = ['//rule:toolchain_type_1'],
)
"##],
    );
    t.scratch.file(
        "test/BUILD",
        &[r##"load('//test:defs.bzl', 'r1')
r1(name = 't1', out = 'my_out.txt', dep = ':t2')
r1(name = 't2')
"##],
    );
    t.use_configuration(&["--extra_toolchains=//toolchain:foo_toolchain"]);

    let _unused = t
        .update_with_aspects(
            &["//test:defs.bzl%toolchain_aspect"],
            &["//test:my_out.txt"],
        )
        .unwrap();

    let visited_targets: Vec<String> = done_aspect_values(&mut t)
        .into_iter()
        .filter(|(k, _)| k.get_aspect_class().get_name() == "//test:defs.bzl%toolchain_aspect")
        .map(|(k, _)| k.get_label().to_string())
        .collect();

    // toolchain_aspect is applied to the generating rule of the output file and propagated to its
    // attribute dependency and toolchain dependency.
    assert_contains_exactly(
        &visited_targets,
        &[
            "//test:my_out.txt",
            "//test:t1",
            "//test:t2",
            "//toolchain:foo",
        ],
    );
}

// Java: StarlarkAspectsToolchainPropagationTest.toolchainAspectApplyToFiles_notPropagatedToDeps
#[test]
#[ignore = "needs TestRuleClassProvider's native test aspects (TestAspects.DepsVisitingFileAspect) and setRulesAndAspectsAvailableInTests"]
#[allow(unreachable_code, unused_variables, clippy::diverging_sub_expression)]
fn toolchain_aspect_apply_to_files_not_propagated_to_deps() {
    // Java:
    //   DepsVisitingFileAspect aspect = new DepsVisitingFileAspect("dep", "//rule:toolchain_type_1");
    //   setRulesAndAspectsAvailableInTests(ImmutableList.of(aspect), ImmutableList.of());
    let aspect_name: &str = blaze_util::unported!(
        "TestAspects.DepsVisitingFileAspect",
        "the native test aspect"
    );
    let mut t = new_test();
    t.scratch.file(
        "test/defs.bzl",
        &[r##"def _rule_1_impl(ctx):
  if ctx.outputs.out:
    ctx.actions.write(ctx.outputs.out, 'hi')
  return []

r1 = rule(
  implementation = _rule_1_impl,
  attrs = {
    "out": attr.output(),
    "dep": attr.label(),
  },
  toolchains = ['//rule:toolchain_type_1'],
)
"##],
    );
    t.scratch.file(
        "test/BUILD",
        &[r##"load('//test:defs.bzl', 'r1')
r1(name = 't1', out = 'my_out.txt', dep = ':t2')
r1(name = 't2')
"##],
    );
    t.use_configuration(&["--extra_toolchains=//toolchain:foo_toolchain"]);

    let _unused = t
        .update_with_aspects(&[aspect_name], &["//test:my_out.txt"])
        .unwrap();

    // {@link DepsVisitingFileAspect} is only applied to //test:my_out.txt file therefore it does
    // not propagate to the dependencies of its generating rule.
    let nodes: Vec<(AspectKey, Arc<AspectValue>)> = done_aspect_values(&mut t)
        .into_iter()
        .filter(|(k, _)| k.get_aspect_class().get_name() == aspect_name)
        .collect();
    assert_eq!(nodes.len(), 1);

    let (aspect_key, _aspect_value) = get_only_element(nodes);
    assert_eq!(aspect_key.get_label().to_string(), "//test:my_out.txt");

    // Java:
    //   StarlarkInfo provider =
    //       (StarlarkInfo) aspectValue.get(DepsVisitingFileAspect.PROVIDER.getKey());
    //   assertThat(provider.getValue("val")).isEqualTo("//test:my_out.txt");
}

// Java: StarlarkAspectsToolchainPropagationTest.toolchainAspectOnTargetWithoutToolchain_success
#[allow(unreachable_code, unused_variables, clippy::diverging_sub_expression)]
fn toolchain_aspect_on_target_without_toolchain_success(auto_exec_groups: &str) {
    // Java:
    //   MockRule ruleWithoutToolchain =
    //       () -> MockRule.define("rule_without_toolchain",
    //           (builder, env) -> builder.toolchainResolutionMode(ToolchainResolutionMode.DISABLED));
    //   setRulesAndAspectsAvailableInTests(ImmutableList.of(), ImmutableList.of(ruleWithoutToolchain));
    blaze_util::unported!("MockRule.define", "rule_without_toolchain");
    let mut t = new_test();
    t.scratch.file(
        "test/defs.bzl",
        &[r##"AspectProvider = provider()

def _aspect_impl(target, ctx):
  return [AspectProvider(val = 'toolchain_aspect on %s' % str(target.label))]

toolchain_aspect = aspect(
    implementation = _aspect_impl,
    toolchains_aspects = ['//rule:toolchain_type_1'],
)

def _rule_impl(ctx):
  pass

rule_with_toolchain = rule(
  implementation = _rule_impl,
  toolchains = ['//rule:toolchain_type_1'],
)
"##],
    );
    t.scratch.file(
        "test/BUILD",
        &[r##"load('//test:defs.bzl', 'rule_with_toolchain')
rule_with_toolchain(name = 'target_with_toolchain')
rule_without_toolchain(name = 'target_without_toolchain')
"##],
    );
    t.use_configuration(&[
        "--extra_toolchains=//toolchain:foo_toolchain",
        &format!("--incompatible_auto_exec_groups={auto_exec_groups}"),
    ]);

    let _unused = t
        .update_with_aspects(&["//test:defs.bzl%toolchain_aspect"], &["//test:all"])
        .unwrap();

    let aspect_on_visited_targets: Vec<String> = done_aspect_values(&mut t)
        .into_iter()
        .filter(|(k, _)| k.get_aspect_class().get_name() == "//test:defs.bzl%toolchain_aspect")
        .map(|(_, a)| {
            get_string(
                &get_starlark_provider(&a, "//test:defs.bzl", "AspectProvider")
                    .expect("AspectProvider"),
                "val",
            )
        })
        .collect();

    // aspect successfully propagates to the 2 targets in //test package and to the toolchain of
    // //test:target_with_toolchain
    assert_contains_exactly(
        &aspect_on_visited_targets,
        &[
            "toolchain_aspect on @@//test:target_with_toolchain",
            "toolchain_aspect on @@//test:target_without_toolchain",
            "toolchain_aspect on @@//toolchain:foo",
        ],
    );
}

#[test]
#[ignore = "needs TestRuleClassProvider's MockRule (rule_without_toolchain) and setRulesAndAspectsAvailableInTests"]
fn toolchain_aspect_on_target_without_toolchain_success_auto_exec_groups_true() {
    toolchain_aspect_on_target_without_toolchain_success("True");
}

#[test]
#[ignore = "needs TestRuleClassProvider's MockRule (rule_without_toolchain) and setRulesAndAspectsAvailableInTests"]
fn toolchain_aspect_on_target_without_toolchain_success_auto_exec_groups_false() {
    toolchain_aspect_on_target_without_toolchain_success("False");
}

// Java: StarlarkAspectsToolchainPropagationTest.requiredToolchainAspectOnTargetWithoutToolchain_success
#[allow(unreachable_code, unused_variables, clippy::diverging_sub_expression)]
fn required_toolchain_aspect_on_target_without_toolchain_success(auto_exec_groups: &str) {
    // Java:
    //   MockRule ruleWithoutToolchain =
    //       () -> MockRule.define("rule_without_toolchain",
    //           (builder, env) -> builder.toolchainResolutionMode(ToolchainResolutionMode.DISABLED));
    //   setRulesAndAspectsAvailableInTests(ImmutableList.of(), ImmutableList.of(ruleWithoutToolchain));
    blaze_util::unported!("MockRule.define", "rule_without_toolchain");
    let mut t = new_test();
    t.scratch.file(
        "test/defs.bzl",
        &[r##"MainAspectProvider = provider()
RequiredAspectProvider = provider()

def _required_aspect_impl(target, ctx):
  target_res = "required_aspect on " + str(target.label)
  if platform_common.ToolchainInfo in target:
    target_res += " with tool in ToolchainInfo = "
    target_res += str(target[platform_common.ToolchainInfo].tool)

  result = [target_res]
  if ctx.rule.toolchains and '//rule:toolchain_type_1' in ctx.rule.toolchains:
      result.extend(
          ctx.rule.toolchains['//rule:toolchain_type_1'][RequiredAspectProvider].val)
  return [RequiredAspectProvider(val = result)]

required_aspect = aspect(
  implementation = _required_aspect_impl,
  toolchains_aspects = ['//rule:toolchain_type_1'],
)

def _main_aspect_impl(target, ctx):
  res = 'main_aspect on %s' % str(target.label)
  return [MainAspectProvider(
      main_aspect_val = res,
      required_aspect_val = target[RequiredAspectProvider].val)]

main_aspect = aspect(
    implementation = _main_aspect_impl,
    requires = [required_aspect]
)

def _rule_impl(ctx):
  pass

rule_with_toolchain = rule(
  implementation = _rule_impl,
  toolchains = ['//rule:toolchain_type_1'],
)
"##],
    );
    t.scratch.file(
        "test/BUILD",
        &[r##"load('//test:defs.bzl', 'rule_with_toolchain')
rule_with_toolchain(name = 'target_with_toolchain')
rule_without_toolchain(name = 'target_without_toolchain')
"##],
    );
    t.use_configuration(&[
        "--extra_toolchains=//toolchain:foo_toolchain",
        &format!("--incompatible_auto_exec_groups={auto_exec_groups}"),
    ]);

    let analysis_result = t
        .update_with_aspects(&["//test:defs.bzl%main_aspect"], &["//test:all"])
        .unwrap();

    // results on //test:target_with_toolchain
    let aspect_on_with_toolchain_target = get_toplevel_configured_aspect(
        analysis_result,
        "//test:defs.bzl%main_aspect",
        "//test:target_with_toolchain",
    );
    let provider = get_starlark_provider(
        &aspect_on_with_toolchain_target,
        "//test:defs.bzl",
        "MainAspectProvider",
    )
    .expect("MainAspectProvider");

    let main_aspect_value = get_string(&provider, "main_aspect_val");
    assert_eq!(
        main_aspect_value,
        "main_aspect on @@//test:target_with_toolchain"
    );

    let required_aspect_value = get_sequence(&provider, "required_aspect_val");
    assert_contains_exactly(
        &required_aspect_value,
        &[
            "required_aspect on @@//test:target_with_toolchain",
            "required_aspect on @@//toolchain:foo with tool in ToolchainInfo = <generated file toolchain/atool>",
        ],
    );

    // test:target_without_toolchain
    let aspect_on_without_toolchain_target = get_toplevel_configured_aspect(
        analysis_result,
        "//test:defs.bzl%main_aspect",
        "//test:target_without_toolchain",
    );
    let provider = get_starlark_provider(
        &aspect_on_without_toolchain_target,
        "//test:defs.bzl",
        "MainAspectProvider",
    )
    .expect("MainAspectProvider");

    let main_aspect_value = get_string(&provider, "main_aspect_val");
    assert_eq!(
        main_aspect_value,
        "main_aspect on @@//test:target_without_toolchain"
    );

    let required_aspect_value = get_sequence(&provider, "required_aspect_val");
    assert_contains_exactly(
        &required_aspect_value,
        &["required_aspect on @@//test:target_without_toolchain"],
    );
}

#[test]
#[ignore = "needs TestRuleClassProvider's MockRule (rule_without_toolchain) and setRulesAndAspectsAvailableInTests"]
fn required_toolchain_aspect_on_target_without_toolchain_success_auto_exec_groups_true() {
    required_toolchain_aspect_on_target_without_toolchain_success("True");
}

#[test]
#[ignore = "needs TestRuleClassProvider's MockRule (rule_without_toolchain) and setRulesAndAspectsAvailableInTests"]
fn required_toolchain_aspect_on_target_without_toolchain_success_auto_exec_groups_false() {
    required_toolchain_aspect_on_target_without_toolchain_success("False");
}

/// The body shared by the two `aspectUsesBaseTargetToolchainsToConfigureTargetDepsWithDefaultExecGp_*`
/// tests and `aspectUsesBaseTargetToolchainsToConfigureTargetDepsWithCustomExecGp` after
/// `update` (written out in each Java test).
fn assert_tool_dependency_configured_once(t: &mut AnalysisTestCase, expected_platform: &str) {
    let analysis_result = t
        .update_with_aspects(&["//test:defs.bzl%my_aspect"], &["//test:t1"])
        .unwrap();

    let top_level_target = get_only_element(analysis_result.get_targets_to_build().to_vec());
    let top_level_target_deps = get_direct_deps(
        t,
        SkyKey::new(configured_target_key_from_configured_target(
            &top_level_target,
        )),
    );

    let tool_dependency_from_target = get_only_element(
        top_level_target_deps
            .iter()
            .filter(|e| is_configured_target(e, "//test:tool"))
            .cloned()
            .collect(),
    );
    let tool_dependency_from_target = tool_dependency_from_target
        .downcast::<ConfiguredTargetKey>()
        .unwrap()
        .clone();

    let aspect_on_tool_dependnecy_key = get_only_element(get_aspect_keys(
        t,
        "//test:tool",
        "//test:defs.bzl%my_aspect",
    ));

    // The aspect used the base target's toolchain to request the target's dependency, so the
    // two keys are equal.
    assert_eq!(
        &tool_dependency_from_target,
        aspect_on_tool_dependnecy_key.get_base_configured_target_key()
    );

    // The //test:tool target is requested only once and its key contains the execution platform of
    // its parent's (//test:t1) toolchain
    let tool_dependency_key = get_configured_target_key(t, "//test:tool");
    assert_eq!(tool_dependency_key.len(), 1, "{tool_dependency_key:?}");

    assert_eq!(
        platforms_of(&tool_dependency_key[0]),
        vec![parse_label(expected_platform)]
    );
}

// Java: StarlarkAspectsToolchainPropagationTest.aspectUsesBaseTargetToolchainsToConfigureTargetDepsWithDefaultExecGp_autoExecGps
#[test]
fn aspect_uses_base_target_toolchains_to_configure_target_deps_with_default_exec_gp_auto_exec_gps()
{
    let mut t = new_test();
    t.scratch.file(
        "test/defs.bzl",
        &[r##"def _impl(target, ctx):
  return []

my_aspect = aspect(
  implementation = _impl,
  toolchains = ['//rule:toolchain_type_1'],
  attr_aspects = ['_tool'],
)

def _rule_impl(ctx):
  pass

r1 = rule(
  implementation = _rule_impl,
  toolchains = ['//rule:toolchain_type_2'],
  attrs = {
    "_tool": attr.label(default='//test:tool', cfg='exec'),
  },
)
"##],
    );
    t.scratch.file(
        "test/BUILD",
        &[r##"load('//test:defs.bzl', 'r1')
load('//test_defs:foo_binary.bzl', 'foo_binary')
r1(name = 't1')
foo_binary(name = 'tool', srcs = ['test.sh'])
"##],
    );
    t.scratch.file("test/test.sh", &[""]);
    t.use_configuration(&[
        "--extra_toolchains=//toolchain:foo_toolchain_exec_1,//toolchain:foo_toolchain_exec_2",
        "--extra_execution_platforms=//platforms:platform_1,//platforms:platform_2",
        "--incompatible_auto_exec_groups=True",
        "--incompatible_enable_cc_toolchain_resolution",
    ]);

    // //test:tool gets the execution platform of the default exec gp, when automatic execution
    // groups are enabled, the default exec gp will have the basic execution platform.
    assert_tool_dependency_configured_once(&mut t, "//platforms:platform_1");
}

// Java: StarlarkAspectsToolchainPropagationTest.aspectUsesBaseTargetToolchainsToConfigureTargetDepsWithDefaultExecGp_noAutoExecGps
#[test]
fn aspect_uses_base_target_toolchains_to_configure_target_deps_with_default_exec_gp_no_auto_exec_gps()
 {
    let mut t = new_test();
    t.scratch.file(
        "test/defs.bzl",
        &[r##"def _impl(target, ctx):
  return []

my_aspect = aspect(
  implementation = _impl,
  toolchains = ['//rule:toolchain_type_1'],
  attr_aspects = ['_tool'],
)

def _rule_impl(ctx):
  pass

r1 = rule(
  implementation = _rule_impl,
  toolchains = ['//rule:toolchain_type_2'],
  attrs = {
    "_tool": attr.label(default='//test:tool', cfg='exec'),
  },
)
"##],
    );
    t.scratch.file(
        "test/BUILD",
        &[r##"load('//test:defs.bzl', 'r1')
load('//test_defs:foo_binary.bzl', 'foo_binary')
r1(name = 't1')
foo_binary(name = 'tool', srcs = ['test.sh'])
"##],
    );
    t.scratch.file("test/test.sh", &[""]);
    t.use_configuration(&[
        "--extra_toolchains=//toolchain:foo_toolchain_exec_1,//toolchain:foo_toolchain_exec_2",
        "--extra_execution_platforms=//platforms:platform_1,//platforms:platform_2",
        "--incompatible_auto_exec_groups=False",
        "--incompatible_enable_cc_toolchain_resolution",
    ]);

    // //test:tool gets the execution platform of the default exec gp, when automatic execution
    // groups are disabled, the default exec gp will have the execution platform of the only
    // toolchain type it has.
    assert_tool_dependency_configured_once(&mut t, "//platforms:platform_2");
}

// Java: StarlarkAspectsToolchainPropagationTest.aspectUsesBaseTargetToolchainsToConfigureTargetDepsWithCustomExecGp
fn aspect_uses_base_target_toolchains_to_configure_target_deps_with_custom_exec_gp(
    auto_exec_groups: bool,
) {
    let mut t = new_test();
    t.scratch.file(
        "test/defs.bzl",
        &[r##"def _impl(target, ctx):
  return []

my_aspect = aspect(
  implementation = _impl,
  exec_groups = {"gp": exec_group(toolchains = ['//rule:toolchain_type_1'])},
  attr_aspects = ['_tool'],
)

def _rule_impl(ctx):
  pass

r1 = rule(
  implementation = _rule_impl,
  exec_groups = {"gp": exec_group(toolchains = ['//rule:toolchain_type_2'])},
  attrs = {
    "_tool": attr.label(default='//test:tool', cfg = config.exec(exec_group = 'gp')),
  },
)
"##],
    );
    t.scratch.file(
        "test/BUILD",
        &[r##"load('//test:defs.bzl', 'r1')
load('//test_defs:foo_binary.bzl', 'foo_binary')
r1(name = 't1')
foo_binary(name = 'tool', srcs = ['test.sh'])
"##],
    );
    t.scratch.file("test/test.sh", &[""]);
    t.use_configuration(&[
        "--extra_toolchains=//toolchain:foo_toolchain_exec_1,//toolchain:foo_toolchain_exec_2",
        "--extra_execution_platforms=//platforms:platform_1,//platforms:platform_2",
        &format!("--incompatible_auto_exec_groups={auto_exec_groups}"),
        "--incompatible_enable_cc_toolchain_resolution",
    ]);

    // The //test:tool target is requested only once and its key contains the execution platform of
    // the exec group 'gp' from its parent (//test:t1).
    assert_tool_dependency_configured_once(&mut t, "//platforms:platform_2");
}

#[test]
fn aspect_uses_base_target_toolchains_to_configure_target_deps_with_custom_exec_gp_auto_exec_groups_true()
 {
    aspect_uses_base_target_toolchains_to_configure_target_deps_with_custom_exec_gp(true);
}

#[test]
fn aspect_uses_base_target_toolchains_to_configure_target_deps_with_custom_exec_gp_auto_exec_groups_false()
 {
    aspect_uses_base_target_toolchains_to_configure_target_deps_with_custom_exec_gp(false);
}

// Java: StarlarkAspectsToolchainPropagationTest.aspectAndRuleHaveDifferentExecutionPlatforms_buildSucceeds
fn aspect_and_rule_have_different_execution_platforms_build_succeeds(auto_exec_groups: bool) {
    let mut t = new_test();
    t.scratch.file(
        "test/defs.bzl",
        &[r##"def _impl(target, ctx):
  return []

toolchain_aspect = aspect(
  implementation = _impl,
  exec_groups = {"gp": exec_group(toolchains = ['//rule:toolchain_type_1'])},
  attrs = {
    "_tool": attr.label(default='//test:aspect_tool', cfg=config.exec(exec_group = 'gp')),
  },
  toolchains_aspects = ['//rule:toolchain_type_1'],
)

def _rule_impl(ctx):
  pass

r1 = rule(
  implementation = _rule_impl,
  exec_groups = {
    "gp": exec_group(
      toolchains = ['//rule:toolchain_type_1'],
      exec_compatible_with = ['//platforms:constraint_2']
    )
  },
  attrs = {
    "_tool": attr.label(default='//test:rule_tool', cfg = config.exec(exec_group = 'gp')),
  },
)
"##],
    );
    t.scratch.file(
        "test/BUILD",
        &[r##"load('//test:defs.bzl', 'r1')
load('//test_defs:foo_binary.bzl', 'foo_binary')
r1(name = 't1')
foo_binary(name = 'rule_tool', srcs = ['test.sh'])
foo_binary(name = 'aspect_tool', srcs = ['test.sh'])
"##],
    );
    t.use_configuration(&[
        "--extra_toolchains=//toolchain:foo_toolchain",
        "--extra_execution_platforms=//platforms:platform_1,//platforms:platform_2",
        &format!("--incompatible_auto_exec_groups={auto_exec_groups}"),
        "--incompatible_enable_cc_toolchain_resolution",
    ]);

    let _unused = t
        .update_with_aspects(&["//test:defs.bzl%toolchain_aspect"], &["//test:t1"])
        .unwrap();

    // //test:rule_tool uses //platforms:platform_2
    let rule_tool = get_only_element(get_configured_target_key(&mut t, "//test:rule_tool"));
    assert_eq!(
        platforms_of(&rule_tool),
        vec![parse_label("//platforms:platform_2")]
    );

    // //test:aspect_tool uses //platforms:platform_1
    let aspect_tool = get_only_element(get_configured_target_key(&mut t, "//test:aspect_tool"));
    assert_eq!(
        platforms_of(&aspect_tool),
        vec![parse_label("//platforms:platform_1")]
    );

    // aspect propagates to the rule's toolchain (with //platforms:platform_2 execution platform)
    // not to its own toolchain
    let aspect_on_target = get_only_element(get_aspect_keys(
        &mut t,
        "//test:t1",
        "//test:defs.bzl%toolchain_aspect",
    ));
    let aspect_on_target_deps = get_aspect_direct_deps(&mut t, &aspect_on_target);

    let aspects_on_toolchain: Vec<String> = aspect_keys_of(&aspect_on_target_deps)
        .iter()
        .map(|k| {
            format!(
                "{} on {}, exec_platform: {}",
                k.get_aspect_name(),
                k.get_label(),
                label_or_null(
                    k.get_base_configured_target_key()
                        .get_execution_platform_label()
                )
            )
        })
        .collect();
    assert_contains_exactly(
        &aspects_on_toolchain,
        &[
            "//test:defs.bzl%toolchain_aspect on //toolchain:foo, exec_platform: //platforms:platform_2",
        ],
    );
}

#[test]
fn aspect_and_rule_have_different_execution_platforms_build_succeeds_auto_exec_groups_true() {
    aspect_and_rule_have_different_execution_platforms_build_succeeds(true);
}

#[test]
fn aspect_and_rule_have_different_execution_platforms_build_succeeds_auto_exec_groups_false() {
    aspect_and_rule_have_different_execution_platforms_build_succeeds(false);
}

// Java: StarlarkAspectsToolchainPropagationTest.aspectPropagatesToToolchain_seesToolchainLabel
fn aspect_propagates_to_toolchain_sees_toolchain_label(auto_exec_groups: bool) {
    let mut t = new_test();
    t.scratch.file(
        "test/defs.bzl",
        &[r##"AspectProvider = provider()
def _impl(target, ctx):
  if ctx.rule.toolchains and '//rule:toolchain_type_1' in ctx.rule.toolchains:
      return [
        AspectProvider(value = str(target.label) + ' has toolchain ' +
          str(ctx.rule.toolchains['//rule:toolchain_type_1'].label))]
  return []

toolchain_aspect = aspect(
  implementation = _impl,
  toolchains_aspects = ['//rule:toolchain_type_1'],
)

def _rule_impl(ctx):
  pass

r1 = rule(
  implementation = _rule_impl,
  toolchains = ['//rule:toolchain_type_1'],
)
"##],
    );
    t.scratch.file(
        "test/BUILD",
        &[r##"load('//test:defs.bzl', 'r1')
r1(name = 't1')
"##],
    );
    t.use_configuration(&[
        "--extra_toolchains=//toolchain:foo_toolchain",
        &format!("--incompatible_auto_exec_groups={auto_exec_groups}"),
    ]);

    let analysis_result = t
        .update_with_aspects(&["//test:defs.bzl%toolchain_aspect"], &["//test:t1"])
        .unwrap();

    let configured_aspect = get_only_element(
        analysis_result
            .get_aspects_map()
            .values()
            .cloned()
            .collect(),
    );

    let value = get_string(
        &get_starlark_provider(&configured_aspect, "//test:defs.bzl", "AspectProvider")
            .expect("AspectProvider"),
        "value",
    );
    assert_eq!(value, "@@//test:t1 has toolchain @@//toolchain:foo");
}

#[test]
fn aspect_propagates_to_toolchain_sees_toolchain_label_auto_exec_groups_true() {
    aspect_propagates_to_toolchain_sees_toolchain_label(true);
}

#[test]
fn aspect_propagates_to_toolchain_sees_toolchain_label_auto_exec_groups_false() {
    aspect_propagates_to_toolchain_sees_toolchain_label(false);
}

// Java: StarlarkAspectsToolchainPropagationTest.aspectCollectsAttributeVars
#[test]
fn aspect_collects_attribute_vars() {
    let mut t = new_test();
    t.scratch.file(
        "test/defs.bzl",
        &[
            r##"VarProvider = provider(fields = ["aspect_vars", "rule_vars"])

def _impl(target, ctx):
  aspect_vars = ctx.var
  rule_vars = ctx.rule.var
  return [VarProvider(aspect_vars = aspect_vars, rule_vars = rule_vars)]

var_aspect = aspect(
  implementation = _impl,
  attrs = {
      "_toolchains": attr.label(default = '//test:aspect_vars'),
  },
)

def _rule_impl(ctx):
  pass

r1 = rule(
  implementation = _rule_impl,
)

def _var_supplier_impl(ctx):
  return [platform_common.TemplateVariableInfo(ctx.attr.vars)]

var_supplier = rule(
  implementation = _var_supplier_impl,
  attrs = {
      "vars": attr.string_dict(),
  },
)
"##,
        ],
    );
    t.scratch.file(
        "test/BUILD",
        &[r##"load('//test:defs.bzl', 'r1', 'var_supplier')
r1(
    name = 't1',
    # The toolchains attribute also supplies make variables, separate from toolchain resolution.
    toolchains = [":rule_vars"],
)

var_supplier(
    name = "rule_vars",
    vars = {
        "rule_var_key": "rule_var_value",
    },
)

var_supplier(
    name = "aspect_vars",
    vars = {
        "aspect_var_key": "aspect_var_value",
    },
)
"##],
    );
    t.use_configuration(&[]);

    let analysis_result = t
        .update_with_aspects(&["//test:defs.bzl%var_aspect"], &["//test:t1"])
        .unwrap();

    let configured_aspect = get_only_element(
        analysis_result
            .get_aspects_map()
            .values()
            .cloned()
            .collect(),
    );

    let provider = get_starlark_provider(&configured_aspect, "//test:defs.bzl", "VarProvider")
        .expect("VarProvider");

    // Check vars from the aspect itself.
    let vars = get_dict(&provider, "aspect_vars").expect("isInstanceOf(Dict.class)");
    // This will have several standard keys as well, so don't check exact keys.
    assert!(vars.contains_key("aspect_var_key"), "{vars:?}");
    assert_eq!(vars["aspect_var_key"], "aspect_var_value");
    // Should not contain keys from the rule.
    assert!(!vars.contains_key("rule_var_key"), "{vars:?}");

    // Check vars from the underlying rule.
    let vars = get_dict(&provider, "rule_vars").expect("isInstanceOf(Dict.class)");
    // This will have several standard keys as well, so don't check exact keys.
    assert!(vars.contains_key("rule_var_key"), "{vars:?}");
    assert_eq!(vars["rule_var_key"], "rule_var_value");
    // Should not contain keys from the rule.
    assert!(!vars.contains_key("aspect_var_key"), "{vars:?}");
}

// Java: StarlarkAspectsToolchainPropagationTest.aspectCollectsToolchainVars
#[test]
fn aspect_collects_toolchain_vars() {
    let mut t = new_test();
    t.scratch.file(
        "test/defs.bzl",
        &[
            r##"VarProvider = provider(fields = ["aspect_vars", "rule_vars"])

def _impl(target, ctx):
  aspect_vars = ctx.var
  rule_vars = ctx.rule.var
  return [VarProvider(aspect_vars = aspect_vars, rule_vars = rule_vars)]

var_aspect = aspect(
  implementation = _impl,
  toolchains = ['//rule:toolchain_type_1'],
  toolchains_aspects = ['//rule:toolchain_type_2'],
)

def _rule_impl(ctx):
  pass

r1 = rule(
  implementation = _rule_impl,
  toolchains = ['//rule:toolchain_type_2'],
)
"##,
        ],
    );
    t.scratch.file(
        "test/BUILD",
        &[r##"load('//test:defs.bzl', 'r1')
r1(name = 't1')
"##],
    );
    t.use_configuration(&[
        "--extra_toolchains=//toolchain:foo_toolchain,//toolchain:foo_toolchain_with_provider",
    ]);

    let analysis_result = t
        .update_with_aspects(&["//test:defs.bzl%var_aspect"], &["//test:t1"])
        .unwrap();

    let configured_aspect = get_only_element(
        analysis_result
            .get_aspects_map()
            .values()
            .cloned()
            .collect(),
    );

    let provider = get_starlark_provider(&configured_aspect, "//test:defs.bzl", "VarProvider")
        .expect("VarProvider");

    // Check vars from the aspect itself.
    let vars = get_dict(&provider, "aspect_vars").expect("isInstanceOf(Dict.class)");
    // This will have several standard keys as well, so don't check exact keys.
    assert!(vars.contains_key("type_1_key"), "{vars:?}");
    assert_eq!(vars["type_1_key"], "type_1_value");
    // Should not contain keys from the rule.
    assert!(!vars.contains_key("type_2_key"), "{vars:?}");

    // Check vars from the underlying rule.
    let vars = get_dict(&provider, "rule_vars").expect("isInstanceOf(Dict.class)");
    // This will have several standard keys as well, so don't check exact keys.
    assert!(vars.contains_key("type_2_key"), "{vars:?}");
    assert_eq!(vars["type_2_key"], "type_2_value");
    // Should not contain keys from the aspect.
    assert!(!vars.contains_key("type_1_key"), "{vars:?}");
}

// Java: StarlarkAspectsToolchainPropagationTest.propagationPredicateAppliedOnToolchain_aspectPropagatedToSatisfyingToolchain
fn propagation_predicate_applied_on_toolchain_aspect_propagated_to_satisfying_toolchain(
    auto_exec_groups: bool,
) {
    let mut t = new_test();
    t.scratch.file(
        "test/defs.bzl",
        &[r##"AspectInfo = provider()

def _propagation_predicate(ctx):
  if ctx.rule.label == Label('//toolchain:foo'):
    return False
  return True

def _impl(target, ctx):
  res = ['my_aspect on ' + str(target.label)]
  toolchains_types = ['//rule:toolchain_type_1', '//rule:toolchain_type_2']

  for toolchain_type in toolchains_types:
    if toolchain_type in ctx.rule.toolchains:
      if AspectInfo in ctx.rule.toolchains[toolchain_type]:
        res.extend(ctx.rule.toolchains[toolchain_type][AspectInfo].res)

  return [AspectInfo(res = res)]

toolchain_aspect = aspect(
  implementation = _impl,
  toolchains_aspects = ['//rule:toolchain_type_1', '//rule:toolchain_type_2'],
  propagation_predicate = _propagation_predicate,
)

def _rule_impl(ctx):
  pass

r1 = rule(
  implementation = _rule_impl,
  toolchains = ['//rule:toolchain_type_1', '//rule:toolchain_type_2'],
)
"##],
    );
    t.scratch.file(
        "test/BUILD",
        &[r##"load('//test:defs.bzl', 'r1')
r1(name = 't1')
"##],
    );
    t.use_configuration(&[
        "--extra_toolchains=//toolchain:foo_toolchain,//toolchain:foo_toolchain_with_provider",
        &format!("--incompatible_auto_exec_groups={auto_exec_groups}"),
    ]);

    let aspects_map = t
        .update_with_aspects(&["//test:defs.bzl%toolchain_aspect"], &["//test:t1"])
        .unwrap()
        .get_aspects_map()
        .clone();

    let aspect_keys = get_aspect_keys_names(&mut t, "//test:defs.bzl%toolchain_aspect");
    // Only the keys to the targets that satisfy the aspect's propagation predicate are present.
    assert_contains_exactly(
        &aspect_keys,
        &[
            "//test:defs.bzl%toolchain_aspect on //test:t1",
            "//test:defs.bzl%toolchain_aspect on //toolchain:foo_with_provider",
        ],
    );

    let configured_aspect = get_only_element(aspects_map.values().cloned().collect());

    assert_contains_exactly(
        &get_sequence(
            &get_starlark_provider(&configured_aspect, "//test:defs.bzl", "AspectInfo")
                .expect("AspectInfo"),
            "res",
        ),
        &[
            "my_aspect on @@//test:t1",
            "my_aspect on @@//toolchain:foo_with_provider",
        ],
    );
}

#[test]
fn propagation_predicate_applied_on_toolchain_aspect_propagated_to_satisfying_toolchain_auto_exec_groups_true()
 {
    propagation_predicate_applied_on_toolchain_aspect_propagated_to_satisfying_toolchain(true);
}

#[test]
fn propagation_predicate_applied_on_toolchain_aspect_propagated_to_satisfying_toolchain_auto_exec_groups_false()
 {
    propagation_predicate_applied_on_toolchain_aspect_propagated_to_satisfying_toolchain(false);
}

// Java: StarlarkAspectsToolchainPropagationTest.toolchainTypesFunc_invalidToolchainType_fails
fn toolchain_types_func_invalid_toolchain_type_fails(auto_exec_groups: bool) {
    let mut t = new_test();
    t.scratch.file(
        "test/defs.bzl",
        &[r##"AspectInfo = provider()

def _toolchain_aspects(ctx):
  return [Label('@:')]

def _impl(target, ctx):
  return []

toolchain_aspect = aspect(
  implementation = _impl,
  toolchains_aspects = _toolchain_aspects,
)

def _rule_impl(ctx):
  pass

r1 = rule(
  implementation = _rule_impl,
  toolchains = ['//rule:toolchain_type_1'],
)
"##],
    );
    t.scratch.file(
        "test/BUILD",
        &[r##"load('//test:defs.bzl', 'r1')
r1(name = 't1')
"##],
    );
    t.use_configuration(&[
        "--extra_toolchains=//toolchain:foo_toolchain",
        &format!("--incompatible_auto_exec_groups={auto_exec_groups}"),
    ]);

    t.remove_fail_fast_handler();
    let result = t.update_with_aspects(&["//test:defs.bzl%toolchain_aspect"], &["//test:t1"]);
    assert!(
        matches!(result, Err(BuildViewError::ViewCreationFailed { .. })),
        "expected ViewCreationFailedException"
    );
    t.assert_contains_event("invalid label in Label(): invalid repository name ':'");
}

#[test]
fn toolchain_types_func_invalid_toolchain_type_fails_auto_exec_groups_true() {
    toolchain_types_func_invalid_toolchain_type_fails(true);
}

#[test]
fn toolchain_types_func_invalid_toolchain_type_fails_auto_exec_groups_false() {
    toolchain_types_func_invalid_toolchain_type_fails(false);
}

// Java: StarlarkAspectsToolchainPropagationTest.toolchainTypesFunc_invalidReturnValue_fails
#[test]
fn toolchain_types_func_invalid_return_value_fails() {
    let mut t = new_test();
    t.scratch.file(
        "test/defs.bzl",
        &[r##"AspectInfo = provider()

def _toolchains_aspects(ctx):
  return ['//rule:toolchain_type_1']

def _impl(target, ctx):
  return []

toolchain_aspect = aspect(
  implementation = _impl,
  toolchains_aspects = _toolchains_aspects,
)

def _rule_impl(ctx):
  pass

r1 = rule(
  implementation = _rule_impl,
  toolchains = ['//rule:toolchain_type_1'],
)
"##],
    );
    t.scratch.file(
        "test/BUILD",
        &[r##"load('//test:defs.bzl', 'r1')
r1(name = 't1')
"##],
    );
    t.use_configuration(&["--extra_toolchains=//toolchain:foo_toolchain"]);

    t.remove_fail_fast_handler();
    let result = t.update_with_aspects(&["//test:defs.bzl%toolchain_aspect"], &["//test:t1"]);
    assert!(
        matches!(result, Err(BuildViewError::ViewCreationFailed { .. })),
        "expected ViewCreationFailedException"
    );
    t.assert_contains_event(
        "at index 0 of toolchains_aspects, got element of type string, want Label",
    );
}

// Java: StarlarkAspectsToolchainPropagationTest.toolchainTypesFunc_wildcardEnumeratesResolvedToolchains
fn toolchain_types_func_wildcard_enumerates_resolved_toolchains(auto_exec_groups: bool) {
    let mut t = new_test();
    t.scratch.file(
        "test/defs.bzl",
        &[r##"AspectInfo = provider()

def _impl(target, ctx):
  res = ['my_aspect on ' + str(target.label)]
  for toolchain_type in ctx.rule.toolchains.toolchain_types():
    if AspectInfo in ctx.rule.toolchains[toolchain_type]:
      res.extend(ctx.rule.toolchains[toolchain_type][AspectInfo].res)
  return [AspectInfo(res = res)]

toolchain_aspect = aspect(
  implementation = _impl,
  toolchains_aspects = ['*'],
)

def _rule_impl(ctx):
  pass

r1 = rule(
  implementation = _rule_impl,
  toolchains = ['//rule:toolchain_type_1', '//rule:toolchain_type_2'],
)
"##],
    );
    t.scratch.file(
        "test/BUILD",
        &[r##"load('//test:defs.bzl', 'r1')
r1(name = 't1')
"##],
    );
    t.use_configuration(&[
        "--extra_toolchains=//toolchain:foo_toolchain,//toolchain:foo_toolchain_with_provider",
        &format!("--incompatible_auto_exec_groups={auto_exec_groups}"),
    ]);

    let aspects_map = t
        .update_with_aspects(&["//test:defs.bzl%toolchain_aspect"], &["//test:t1"])
        .unwrap()
        .get_aspects_map()
        .clone();

    let aspect_keys = get_aspect_keys_names(&mut t, "//test:defs.bzl%toolchain_aspect");
    assert_contains_exactly(
        &aspect_keys,
        &[
            "//test:defs.bzl%toolchain_aspect on //test:t1",
            "//test:defs.bzl%toolchain_aspect on //toolchain:foo",
            "//test:defs.bzl%toolchain_aspect on //toolchain:foo_with_provider",
        ],
    );

    let configured_aspect = get_only_element(aspects_map.values().cloned().collect());
    assert_contains_exactly(
        &get_sequence(
            &get_starlark_provider(&configured_aspect, "//test:defs.bzl", "AspectInfo")
                .expect("AspectInfo"),
            "res",
        ),
        &[
            "my_aspect on @@//test:t1",
            "my_aspect on @@//toolchain:foo",
            "my_aspect on @@//toolchain:foo_with_provider",
        ],
    );
}

#[test]
fn toolchain_types_func_wildcard_enumerates_resolved_toolchains_auto_exec_groups_true() {
    toolchain_types_func_wildcard_enumerates_resolved_toolchains(true);
}

#[test]
fn toolchain_types_func_wildcard_enumerates_resolved_toolchains_auto_exec_groups_false() {
    toolchain_types_func_wildcard_enumerates_resolved_toolchains(false);
}

// Java: StarlarkAspectsToolchainPropagationTest.toolchainTypesFunc_propagateToSelectedTypes
fn toolchain_types_func_propagate_to_selected_types(auto_exec_groups: bool) {
    let mut t = new_test();
    t.scratch.file(
        "test/defs.bzl",
        &[r##"AspectInfo = provider()

def _toolchains_aspects(ctx):
  if ctx.rule.label == Label('//test:t1'):
    return [Label('//rule:toolchain_type_1')]
  elif ctx.rule.label == Label('//test:t2'):
    return [Label('//rule:toolchain_type_2')]
  return []

def _impl(target, ctx):
  res = ['my_aspect on ' + str(target.label)]
  toolchains_types = ['//rule:toolchain_type_1', '//rule:toolchain_type_2']

  for toolchain_type in toolchains_types:
    if toolchain_type in ctx.rule.toolchains:
      if AspectInfo in ctx.rule.toolchains[toolchain_type]:
        res.extend(ctx.rule.toolchains[toolchain_type][AspectInfo].res)

  return [AspectInfo(res = res)]

toolchain_aspect = aspect(
  implementation = _impl,
  toolchains_aspects = _toolchains_aspects,
)

def _rule_impl(ctx):
  pass

r1 = rule(
  implementation = _rule_impl,
  toolchains = ['//rule:toolchain_type_1', '//rule:toolchain_type_2'],
)
"##],
    );
    t.scratch.file(
        "test/BUILD",
        &[r##"load('//test:defs.bzl', 'r1')
r1(name = 't1')
r1(name = 't2')
r1(name = 't3')
"##],
    );
    t.use_configuration(&[
        "--extra_toolchains=//toolchain:foo_toolchain,//toolchain:foo_toolchain_with_provider",
        &format!("--incompatible_auto_exec_groups={auto_exec_groups}"),
    ]);

    let aspects_map = t
        .update_with_aspects(
            &["//test:defs.bzl%toolchain_aspect"],
            &["//test:t1", "//test:t2", "//test:t3"],
        )
        .unwrap()
        .get_aspects_map()
        .clone();

    let aspect_keys = get_aspect_keys_names(&mut t, "//test:defs.bzl%toolchain_aspect");
    assert_contains_exactly(
        &aspect_keys,
        &[
            "//test:defs.bzl%toolchain_aspect on //test:t1",
            "//test:defs.bzl%toolchain_aspect on //test:t2",
            "//test:defs.bzl%toolchain_aspect on //test:t3",
            "//test:defs.bzl%toolchain_aspect on //toolchain:foo",
            "//test:defs.bzl%toolchain_aspect on //toolchain:foo_with_provider",
        ],
    );

    let t1_aspect_result = get_aspect_result(
        &aspects_map,
        "toolchain_aspect",
        Some("//test:t1"),
        "AspectInfo",
    );
    assert_contains_exactly(
        &t1_aspect_result,
        &["my_aspect on @@//test:t1", "my_aspect on @@//toolchain:foo"],
    );

    let t2_aspect_result = get_aspect_result(
        &aspects_map,
        "toolchain_aspect",
        Some("//test:t2"),
        "AspectInfo",
    );
    assert_contains_exactly(
        &t2_aspect_result,
        &[
            "my_aspect on @@//test:t2",
            "my_aspect on @@//toolchain:foo_with_provider",
        ],
    );

    let t3_aspect_result = get_aspect_result(
        &aspects_map,
        "toolchain_aspect",
        Some("//test:t3"),
        "AspectInfo",
    );
    assert_contains_exactly(&t3_aspect_result, &["my_aspect on @@//test:t3"]);
}

#[test]
fn toolchain_types_func_propagate_to_selected_types_auto_exec_groups_true() {
    toolchain_types_func_propagate_to_selected_types(true);
}

#[test]
fn toolchain_types_func_propagate_to_selected_types_auto_exec_groups_false() {
    toolchain_types_func_propagate_to_selected_types(false);
}
