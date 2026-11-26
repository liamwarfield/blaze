//! `BuildConfigurationValue` methods analysis reads that `blaze_build_config` does not expose yet.
//! Each is a field read of `CoreOptions` (or a fragment) exactly as in Java.
//!
//! PORT: move to `blaze_build_config::build_configuration_value` (lead-owned); kept here as free
//! functions so analysis can call them by their Java names.

use std::sync::Arc;

use blaze_build_config::build_configuration_value::BuildConfigurationValue;
use blaze_build_config::fragment::Fragment;
use blaze_cmdline::Label;
use blaze_optdefs::fragment::core_options::CoreOptions;
use blaze_optdefs::fragment::run_under::RunUnder;
use indexmap::IndexMap;

/// `options` of a configuration (Java `BuildConfigurationValue.options`, the `CoreOptions`).
pub fn core_options(config: &BuildConfigurationValue) -> &CoreOptions {
    config
        .get_options()
        .get::<CoreOptions>()
        .expect("NullPointerException: BuildConfigurationValue without CoreOptions")
}

// Java: com/google/devtools/build/lib/analysis/config/BuildConfigurationValue.java:784 BuildConfigurationValue.allowAnalysisFailures
pub fn allow_analysis_failures(config: &BuildConfigurationValue) -> bool {
    core_options(config).allow_analysis_failures
}

// Java: com/google/devtools/build/lib/analysis/config/BuildConfigurationValue.java:780 BuildConfigurationValue.enforceConstraints
pub fn enforce_constraints(config: &BuildConfigurationValue) -> bool {
    core_options(config).enforce_constraints
}

// Java: com/google/devtools/build/lib/analysis/config/BuildConfigurationValue.java:764 BuildConfigurationValue.enforceTransitiveVisibility
pub fn enforce_transitive_visibility(config: &BuildConfigurationValue) -> bool {
    core_options(config).enforce_transitive_visibility
}

// Java: com/google/devtools/build/lib/analysis/config/BuildConfigurationValue.java:788 BuildConfigurationValue.evaluatingForAnalysisTest
pub fn evaluating_for_analysis_test(config: &BuildConfigurationValue) -> bool {
    core_options(config).evaluating_for_analysis_test
}

// Java: com/google/devtools/build/lib/analysis/config/BuildConfigurationValue.java:792 BuildConfigurationValue.analysisTestingDepsLimit
pub fn analysis_testing_deps_limit(config: &BuildConfigurationValue) -> i32 {
    core_options(config).analysis_testing_deps_limit
}

/// Returns true if we should collect coverage data (`--collect_code_coverage`).
// Java: com/google/devtools/build/lib/analysis/config/BuildConfigurationValue.java:731 BuildConfigurationValue.isCodeCoverageEnabled
pub fn is_code_coverage_enabled(config: &BuildConfigurationValue) -> bool {
    core_options(config).collect_code_coverage
}

// Java: com/google/devtools/build/lib/analysis/config/BuildConfigurationValue.java:597 BuildConfigurationValue.shouldCollectCodeCoverageForGeneratedFiles
pub fn should_collect_code_coverage_for_generated_files(config: &BuildConfigurationValue) -> bool {
    core_options(config).collect_code_coverage_for_generated_files
}

// Java: com/google/devtools/build/lib/analysis/config/BuildConfigurationValue.java:592 BuildConfigurationValue.shouldInstrumentTestTargets
pub fn should_instrument_test_targets(config: &BuildConfigurationValue) -> bool {
    core_options(config).instrument_test_targets
}

// Java: com/google/devtools/build/lib/analysis/config/BuildConfigurationValue.java:583 BuildConfigurationValue.getInstrumentationFilter
pub fn instrumentation_filter_is_included(config: &BuildConfigurationValue, label: &str) -> bool {
    core_options(config)
        .instrumentation_filter
        .is_included(label)
}

/// Returns the `--run_under` value, if any.
// Java: com/google/devtools/build/lib/analysis/config/BuildConfigurationValue.java:741 BuildConfigurationValue.getRunUnder
pub fn get_run_under(config: &BuildConfigurationValue) -> Option<&RunUnder> {
    core_options(config).run_under.as_ref()
}

// Java: com/google/devtools/build/lib/analysis/config/BuildConfigurationValue.java:746 BuildConfigurationValue.runUnderExecConfigForTests
pub fn run_under_exec_config_for_tests(config: &BuildConfigurationValue) -> bool {
    core_options(config).bazel_test_exec_run_under
}

// Java: com/google/devtools/build/lib/analysis/config/BuildConfigurationValue.java:796 BuildConfigurationValue.getActionListeners
pub fn get_action_listeners(config: &BuildConfigurationValue) -> Vec<Label> {
    core_options(config)
        .action_listeners
        .iter()
        .map(|l| l.get().clone())
        .collect()
}

// Java: com/google/devtools/build/lib/analysis/config/BuildConfigurationValue.java:701 BuildConfigurationValue.alwaysIncludeFilesToBuildInData
pub fn always_include_files_to_build_in_data(config: &BuildConfigurationValue) -> bool {
    core_options(config).always_include_files_to_build_in_data
}

// Java: com/google/devtools/build/lib/analysis/config/BuildConfigurationValue.java:505 BuildConfigurationValue.useAutoExecGroups
pub fn use_auto_exec_groups(config: &BuildConfigurationValue) -> bool {
    core_options(config).use_auto_exec_groups
}

// Java: com/google/devtools/build/lib/analysis/config/BuildConfigurationValue.java:534 BuildConfigurationValue.isSiblingRepositoryLayout
pub fn is_sibling_repository_layout(config: &BuildConfigurationValue) -> bool {
    config.sibling_repository_layout()
}

/// Java `BuildConfigurationValue.modifyExecutionInfo(executionInfo, mnemonic)`.
// Java: com/google/devtools/build/lib/analysis/config/BuildConfigurationValue.java:952 BuildConfigurationValue.modifyExecutionInfo
pub fn modify_execution_info(
    config: &BuildConfigurationValue,
    execution_info: &mut IndexMap<String, String>,
    mnemonic: &str,
) {
    let options = core_options(config);
    // Java: ExecutionInfoModifier.apply(executionInfoList, isAdditive, mnemonic, executionInfo)
    // Java: com/google/devtools/build/lib/analysis/config/ExecutionInfoModifier.java:165 ExecutionInfoModifier.apply
    let list = &options.execution_info_modifier;
    if list.is_empty() {
        return;
    }
    if options.additive_modify_execution_info {
        for eim in list {
            apply_execution_info_modifier(eim, mnemonic, execution_info);
        }
    } else {
        apply_execution_info_modifier(list.last().unwrap(), mnemonic, execution_info);
    }
}

/// `ExecutionInfoModifier.apply(mnemonic, executionInfo)` on the caller's insertion-ordered map
/// (Java mutates a `LinkedHashMap` in place: `remove` keeps the other entries' order, `put` of a
/// present key keeps its position).
// Java: com/google/devtools/build/lib/analysis/config/ExecutionInfoModifier.java:182 ExecutionInfoModifier.apply
fn apply_execution_info_modifier(
    eim: &blaze_optdefs::fragment::execution_info_modifier::ExecutionInfoModifier,
    mnemonic: &str,
    execution_info: &mut IndexMap<String, String>,
) {
    for expr in eim.expressions() {
        if expr
            .pattern()
            .matches(&blaze_util::string_encoding::internal_to_unicode(mnemonic))
        {
            if expr.remove() {
                execution_info.shift_remove(expr.key());
            } else {
                execution_info.insert(expr.key().to_owned(), String::new());
            }
        }
    }
}

/// `config.hasFragment(fragmentClass)` where the fragment class is named by its Java simple name
/// (how `ConfigurationFragmentPolicy` names fragments in Blaze).
// Java: com/google/devtools/build/lib/analysis/config/BuildConfigurationValue.java:650 BuildConfigurationValue.hasFragment
pub fn has_fragment_named(config: &BuildConfigurationValue, simple_name: &str) -> bool {
    config
        .get_fragments()
        .iter()
        .any(|(class, _)| class.get_simple_name() == simple_name || class.get_name() == simple_name)
}

/// `config.getFragment(T.class)`.
pub fn get_fragment<T: Fragment>(config: &Arc<BuildConfigurationValue>) -> Option<&T> {
    config.get_fragment::<T>()
}

/// The configuration's BEP event (Java `BuildConfigurationValue.toBuildEvent()`).
// Java: com/google/devtools/build/lib/analysis/config/BuildConfigurationValue.java:1004 BuildConfigurationValue.createBuildEvent
pub fn to_build_event(config: &BuildConfigurationValue) -> blaze_actions::BuildConfigurationEvent {
    use blaze_protos::build_event_stream;
    let options = core_options(config);
    let mut cpu = config.get_cpu().to_owned();
    if options.incompatible_bep_cpu_from_platform {
        cpu = config.platform_cpu().to_owned();
    }
    let make_variable = config
        .get_make_environment()
        .into_iter()
        .map(|(key, value)| build_event_stream::configuration::MakeVariableEntry { key, value })
        .collect();
    blaze_actions::BuildConfigurationEvent {
        // Java: BuildEventIdUtil.configurationId(checksum()).
        event_id: config.checksum().to_owned(),
        configuration: build_event_stream::Configuration {
            mnemonic: config.get_mnemonic().to_owned(),
            platform_name: cpu.clone(),
            cpu,
            make_variable,
            is_tool: config.is_tool_configuration(),
        },
    }
}

/// The `BuildConfigurationInfo` actions carry for a configuration (Java: the
/// `BuildConfigurationValue` itself, which implements `BuildConfigurationInfo`).
// Java: com/google/devtools/build/lib/analysis/config/BuildConfigurationInfo.java:31 BuildConfigurationInfo
pub fn to_build_configuration_info(
    config: &BuildConfigurationValue,
) -> blaze_actions::BuildConfigurationInfo {
    let mut info = blaze_actions::BuildConfigurationInfo::create(
        config.get_mnemonic(),
        config.checksum(),
        Some(to_build_event(config)),
        config.is_tool_configuration(),
    );
    // Java: BuildConfigurationValue.getCommandLineLimits().
    info.command_line_limits_max_length = config.get_command_line_limits_max_length();
    info
}

/// A configuration as the action layer's `SpawnActionConfiguration` (the values
/// `SpawnAction.Builder.build(owner, configuration)` reads from `BuildConfigurationValue`).
// Java: com/google/devtools/build/lib/analysis/config/BuildConfigurationValue.java:89 BuildConfigurationValue
pub struct ConfigurationSpawnActionConfiguration(pub Arc<BuildConfigurationValue>);

impl blaze_actions::analysis::actions::spawn_action::SpawnActionConfiguration
    for ConfigurationSpawnActionConfiguration
{
    // Java: com/google/devtools/build/lib/analysis/config/BuildConfigurationValue.java BuildConfigurationValue.getActionEnvironment
    fn get_action_environment(&self) -> Arc<blaze_actions::ActionEnvironment> {
        Arc::new(crate::rule::action_environments::to_actions_env(
            self.0.get_action_environment(),
        ))
    }

    // Java: com/google/devtools/build/lib/analysis/config/BuildConfigurationValue.java:926 BuildConfigurationValue.modifiedExecutionInfo
    fn modified_execution_info(
        &self,
        execution_info: &IndexMap<String, String>,
        mnemonic: &str,
    ) -> IndexMap<String, String> {
        use blaze_optdefs::fragment::execution_info_modifier::ExecutionInfoModifier;
        let options = core_options(&self.0);
        if !ExecutionInfoModifier::matches_list(
            &options.execution_info_modifier,
            options.additive_modify_execution_info,
            mnemonic,
        ) {
            return execution_info.clone();
        }
        let as_hash_map: std::collections::HashMap<String, String> = execution_info
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        if !ExecutionInfoModifier::would_change_list(
            &options.execution_info_modifier,
            options.additive_modify_execution_info,
            mnemonic,
            &as_hash_map,
        ) {
            return execution_info.clone();
        }
        // modifiedExecutionInfoInterner.get(key): the loader applies the modifiers to a
        // `new HashMap<>(executionInfo)` and returns `ImmutableSortedMap.copyOf` of it, so the
        // result iterates in key order (String.compareTo: UTF-16 code units; internal strings
        // hold one char per byte, whose UTF-8 byte order is the same).
        // Java: com/google/devtools/build/lib/analysis/config/BuildConfigurationValue.java:905 BuildConfigurationValue.modifiedExecutionInfoInterner
        let mut mutable_copy = as_hash_map;
        ExecutionInfoModifier::apply_list(
            &options.execution_info_modifier,
            options.additive_modify_execution_info,
            mnemonic,
            &mut mutable_copy,
        );
        let mut sorted: Vec<(String, String)> = mutable_copy.into_iter().collect();
        sorted.sort_by(|a, b| a.0.cmp(&b.0));
        sorted.into_iter().collect()
    }

    // Java: com/google/devtools/build/lib/analysis/actions/PathMappers.java:145 PathMappers.getOutputPathsMode
    fn get_output_paths_mode(&self) -> blaze_actions::actions::path_mapper::OutputPathsMode {
        use blaze_actions::actions::path_mapper::OutputPathsMode;
        use blaze_optdefs::fragment::core_options::CoreOptionsOutputPathsMode;
        match core_options(&self.0).output_paths_mode {
            CoreOptionsOutputPathsMode::OFF => OutputPathsMode::OFF,
            CoreOptionsOutputPathsMode::STRIP => OutputPathsMode::STRIP,
        }
    }
}
