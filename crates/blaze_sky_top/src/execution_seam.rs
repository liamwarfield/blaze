//! The real [`AnalysisSeam`] of the execution phase: the execution SkyFunctions
//! (`blaze_sky_exec`) read the analysis values of the same evaluator through it.
//!
//! Java has no seam: `ActionExecutionFunction` (`ArtifactFunction.getActionLookupValue`),
//! `BuildDriverFunction`, `TargetCompletor` and `TestCompletionFunction` call
//! `env.getValue(actionLookupKey)` and read providers off the `ConfiguredTarget`. This file holds
//! those reads, plus the parts of `SkyframeBuildView.analyzeAndExecuteTargets`,
//! `BuildRequest.getTopLevelArtifactContext` and `AnalysisAndExecutionPhaseRunner` that create
//! the `BuildDriverKey`s of a build or test request.

use std::fmt;
use std::sync::Arc;

use blaze_sky_exec::action_template_expansion_value::ActionTemplateExpansionKey;
use std::sync::atomic::Ordering;

use blaze_actions::analysis::workspace_status_action::BUILD_INFO_FUNCTION_NAME;
use blaze_actions::{ActionLookupKeyRef, ActionRef, Artifact};
use blaze_analysis::analysis_services::{
    AnalysisServices, WorkspaceStatusKey, WorkspaceStatusValue,
};
use blaze_analysis::aspect_key_creator::{
    ASPECT, AspectKey, TOP_LEVEL_ASPECTS, TopLevelAspectsKey, TopLevelAspectsValue,
};
use blaze_analysis::aspect_value::AspectValue;
use blaze_analysis::configured_target::ConfiguredTarget;
use blaze_analysis::configured_target_key::{CONFIGURED_TARGET, ConfiguredTargetKey};
use blaze_analysis::configured_target_value::ConfiguredTargetValue;
use blaze_analysis::configuredtargets::{get_files_to_run_provider, get_rule_class_string};
use blaze_analysis::info::downcast_info;
use blaze_analysis::providers::file_provider::FileProvider;
use blaze_analysis::providers::output_group_info::{
    self, DEFAULT, HIDDEN_OUTPUT_GROUP_PREFIX, OutputGroupInfo, ValidationMode,
};
use blaze_analysis::test::test_provider::TestProvider;
use blaze_build_config::build_configuration_value::BuildConfigurationValue;
use blaze_buildtool::buildtool::build_result_printer::ConfiguredAspectView;
use blaze_cmdline::repository_name::MAIN;
use blaze_cmdline::{Label, RepositoryMapping, TargetPatternParser, TargetPatternType};
use blaze_collect::{NestedSet, NestedSetBuilder};
use blaze_events::{Event, ExtendedEventHandler};
use blaze_optdefs::command::build_request_options::BuildRequestOptions;
use blaze_packages::starlark_defs::builtin_providers::{DEFAULT_INFO, OUTPUT_GROUP_INFO};
use blaze_packages::target::target_utils;
use blaze_packages::target::{InputFile, OutputFile, Target};
use blaze_sky_exec::analysis_seam::{
    ActionLookupValue, AnalysisSeam, ConfiguredTargetView, TestParams,
};
use blaze_sky_exec::keys::{BuildDriverKey, TestType};
use blaze_skyframe::{ComputeError, Environment};
use blaze_util::unported;
use blaze_vfs::PathFragment;
use indexmap::{IndexMap, IndexSet};

// -------------------------------------------------------------------------------------------
// Action lookup values

/// `RuleConfiguredTargetValue` / `NonRuleConfiguredTargetValue` as an `ActionLookupValue`.
// Java: com/google/devtools/build/lib/skyframe/RuleConfiguredTargetValue.java:32 RuleConfiguredTargetValue
// PORT: a newtype because ConfiguredTargetValue (blaze_analysis) and ActionLookupValue
// (blaze_sky_exec) are both foreign here.
struct ConfiguredTargetActionLookupValue(Arc<ConfiguredTargetValue>);

impl ActionLookupValue for ConfiguredTargetActionLookupValue {
    // Java: com/google/devtools/build/lib/skyframe/RuleConfiguredTargetValue.java:46 RuleConfiguredTargetValue.getActions
    fn get_actions(&self) -> &[ActionRef] {
        self.0.get_actions()
    }
}

/// `AspectValue` as an `ActionLookupValue` (Java `AspectValue extends BasicActionLookupValue`).
// Java: com/google/devtools/build/lib/analysis/AspectValue.java:37 AspectValue
// PORT: a newtype because AspectValue (blaze_analysis) and ActionLookupValue (blaze_sky_exec)
// are both foreign here.
pub struct AspectActionLookupValue(pub Arc<AspectValue>);

impl ActionLookupValue for AspectActionLookupValue {
    // Java: com/google/devtools/build/lib/actions/BasicActionLookupValue.java:33 BasicActionLookupValue.getActions
    fn get_actions(&self) -> &[ActionRef] {
        self.0.get_actions()
    }
}

/// `TopLevelAspectsValue` as an `ActionLookupValue`: it owns no actions.
// Java: com/google/devtools/build/lib/skyframe/TopLevelAspectsValue.java:28 TopLevelAspectsValue
pub struct TopLevelAspectsActionLookupValue(pub Arc<TopLevelAspectsValue>);

impl ActionLookupValue for TopLevelAspectsActionLookupValue {
    // Java: com/google/devtools/build/lib/skyframe/TopLevelAspectsValue.java:40 TopLevelAspectsValue.getActions
    fn get_actions(&self) -> &[ActionRef] {
        self.0.get_actions()
    }
}

/// The `ActionLookupValue` view of the workspace status node's value.
// Java: com/google/devtools/build/lib/skyframe/WorkspaceStatusValue.java:33 WorkspaceStatusValue
pub struct WorkspaceStatusActionLookupValue(pub Arc<WorkspaceStatusValue>);

impl ActionLookupValue for WorkspaceStatusActionLookupValue {
    fn get_actions(&self) -> &[ActionRef] {
        &self.0.actions
    }
}

/// The `ActionLookupValue` view of the coverage report node's value.
// Java: com/google/devtools/build/lib/skyframe/CoverageReportValue.java:29 CoverageReportValue
pub struct CoverageReportActionLookupValue(
    pub Arc<blaze_analysis::coverage_report_value::CoverageReportValue>,
);

impl ActionLookupValue for CoverageReportActionLookupValue {
    fn get_actions(&self) -> &[ActionRef] {
        &self.0.actions
    }
}

// -------------------------------------------------------------------------------------------
// The seam

/// [`AnalysisSeam`] over the analysis values of the command's evaluator.
pub struct SkyframeAnalysisSeam {
    services: Arc<AnalysisServices>,
}

impl SkyframeAnalysisSeam {
    pub fn new(services: Arc<AnalysisServices>) -> SkyframeAnalysisSeam {
        SkyframeAnalysisSeam { services }
    }

    /// The artifacts every build requests besides its top-level targets' outputs.
    // Java: com/google/devtools/build/lib/skyframe/SkyframeExecutor.java:1604 SkyframeExecutor.getWorkspaceStatusArtifacts
    pub fn get_workspace_status_artifacts(&self) -> Vec<Artifact> {
        let artifacts = self.services.get_workspace_status_artifacts();
        vec![
            artifacts.stable_artifact.clone(),
            artifacts.volatile_artifact.clone(),
        ]
    }
}

/// The `AspectKey` inside an `ActionLookupKeyRef`, `None` for other key kinds.
pub fn as_aspect_key(key: &ActionLookupKeyRef) -> Option<&AspectKey> {
    key.as_any().downcast_ref::<AspectKey>()
}

/// The `ConfiguredTargetKey` inside an `ActionLookupKeyRef`, `None` for other key kinds.
fn as_configured_target_key(key: &ActionLookupKeyRef) -> Option<&ConfiguredTargetKey> {
    key.as_any().downcast_ref::<ConfiguredTargetKey>()
}

impl AnalysisSeam for SkyframeAnalysisSeam {
    // Java: com/google/devtools/build/lib/skyframe/BuildDriverFunction.java:691 BuildDriverFunction.addArtifactsToBuilder
    fn is_extra_action_owner_included(&self, owner_label: &str) -> bool {
        match &*self.services.extra_action_filter.read() {
            Some(filter) => filter.is_included(owner_label),
            None => true,
        }
    }

    // Java: com/google/devtools/build/lib/skyframe/ArtifactFunction.java:378 ArtifactFunction.getActionLookupValue
    fn get_action_lookup_value(
        &self,
        key: &ActionLookupKeyRef,
        env: &mut Environment<'_>,
    ) -> Result<Option<Arc<dyn ActionLookupValue>>, ComputeError> {
        match key.function_name() {
            name if name == CONFIGURED_TARGET.0 => {
                let ct_key = as_configured_target_key(key).unwrap_or_else(|| {
                    panic!("ClassCastException: {key} is not a ConfiguredTargetKey")
                });
                let Some(value) = env.get_value(ct_key.clone())? else {
                    return Ok(None);
                };
                Ok(Some(Arc::new(ConfiguredTargetActionLookupValue(value))
                    as Arc<dyn ActionLookupValue>))
            }
            BUILD_INFO_FUNCTION_NAME => {
                let Some(value) = env.get_value(WorkspaceStatusKey)? else {
                    return Ok(None);
                };
                Ok(Some(
                    Arc::new(WorkspaceStatusActionLookupValue(value)) as Arc<dyn ActionLookupValue>
                ))
            }
            // Java: AspectKeyCreator.AspectKey -> AspectValue.
            name if name == ASPECT.0 => {
                let aspect_key = as_aspect_key(key)
                    .unwrap_or_else(|| panic!("ClassCastException: {key} is not an AspectKey"));
                let Some(value) = env.get_value(aspect_key.clone())? else {
                    return Ok(None);
                };
                Ok(Some(
                    Arc::new(AspectActionLookupValue(value)) as Arc<dyn ActionLookupValue>
                ))
            }
            // Java: AspectKeyCreator.TopLevelAspectsKey -> TopLevelAspectsValue.
            name if name == TOP_LEVEL_ASPECTS.0 => {
                let top_level_key = key
                    .as_any()
                    .downcast_ref::<TopLevelAspectsKey>()
                    .unwrap_or_else(|| {
                        panic!("ClassCastException: {key} is not a TopLevelAspectsKey")
                    });
                let Some(value) = env.get_value(top_level_key.clone())? else {
                    return Ok(None);
                };
                Ok(Some(
                    Arc::new(TopLevelAspectsActionLookupValue(value)) as Arc<dyn ActionLookupValue>
                ))
            }
            // Java: ActionTemplateExpansionValue.ActionTemplateExpansionKey.
            "ACTION_TEMPLATE_EXPANSION" => {
                let expansion_key = key
                    .as_any()
                    .downcast_ref::<ActionTemplateExpansionKey>()
                    .unwrap_or_else(|| {
                        panic!("ClassCastException: {key} is not an ActionTemplateExpansionKey")
                    });
                let Some(value) = env.get_value(expansion_key.clone())? else {
                    return Ok(None);
                };
                Ok(Some(value as Arc<dyn ActionLookupValue>))
            }
            // Java: CoverageReportValue.COVERAGE_REPORT_KEY.
            blaze_analysis::coverage_report_value::COVERAGE_REPORT_FUNCTION_NAME => {
                let Some(value) =
                    env.get_value(blaze_analysis::coverage_report_value::COVERAGE_REPORT_KEY)?
                else {
                    return Ok(None);
                };
                Ok(Some(
                    Arc::new(CoverageReportActionLookupValue(value)) as Arc<dyn ActionLookupValue>
                ))
            }
            other => unported!("ArtifactFunction.getActionLookupValue", "{other} ({key})"),
        }
    }

    // Java: com/google/devtools/build/lib/skyframe/TargetCompletor.java:66 TargetCompletor.getValueAndArtifactsToBuild
    fn get_configured_target(
        &self,
        key: &ActionLookupKeyRef,
        env: &mut Environment<'_>,
    ) -> Result<Option<Arc<dyn ConfiguredTargetView>>, ComputeError> {
        // PORT: a top-level aspect is read through [`AnalysisSeam::get_top_level_aspects`] /
        // [`AnalysisSeam::get_configured_aspect`] (BuildDriverFunction, AspectCompletor).
        check_top_level_aspect_drivers_created();
        let Some(ct_key) = as_configured_target_key(key) else {
            panic!("ClassCastException: {key} is not a ConfiguredTargetKey");
        };
        let Some(value) = env.get_value(ct_key.clone())? else {
            return Ok(None);
        };
        let configured_target = value.get_configured_target().clone();
        // Java: BuildDriverFunction reads the target's BuildConfigurationValue
        // (getConfiguration(env, configuredTarget)) for the TestAnalyzedEvent.
        let mut configuration_mnemonic = None;
        if let Some(config_key) = configured_target.get_configuration_key() {
            let Some(configuration) = env.get_value(config_key)? else {
                return Ok(None);
            };
            configuration_mnemonic = Some(configuration.get_mnemonic().to_owned());
        }
        let location = value
            .get_target()
            .map(|t| blaze_sky_loading::starlark_util::event_location(&t.get_location()));
        Ok(Some(Arc::new(SkyframeConfiguredTargetView {
            configured_target,
            transitive_packages: value.get_transitive_packages().cloned(),
            configuration_mnemonic,
            location,
        }) as Arc<dyn ConfiguredTargetView>))
    }

    /// `env.getValue(topLevelAspectsKey)` as `BuildDriverFunction` reads it for a
    /// `TopLevelAspectsKey`: the top-level `AspectValue`s by key, in their order. `Ok(None)` when
    /// the value is missing.
    // Java: com/google/devtools/build/lib/skyframe/BuildDriverFunction.java:323 BuildDriverFunction.compute
    fn get_top_level_aspects(
        &self,
        key: &ActionLookupKeyRef,
        env: &mut Environment<'_>,
    ) -> Result<Option<Vec<Arc<dyn ConfiguredAspectView>>>, ComputeError> {
        let top_level_key = key
            .as_any()
            .downcast_ref::<TopLevelAspectsKey>()
            .unwrap_or_else(|| panic!("ClassCastException: {key} is not a TopLevelAspectsKey"));
        let Some(value) = env.get_value(top_level_key.clone())? else {
            return Ok(None);
        };
        Ok(Some(
            value
                .get_top_level_aspects_map()
                .iter()
                .map(|(k, v)| {
                    Arc::new(SkyframeConfiguredAspectView::new(k.clone(), v.clone()))
                        as Arc<dyn ConfiguredAspectView>
                })
                .collect(),
        ))
    }

    /// `AspectCompletor.getValueAndArtifactsToBuild` (`CompletionFunction
    /// .getValueAndArtifactsToBuild` with an `AspectCompletionKey`): the `AspectValue` of
    /// `key` as an aspect view. `Ok(None)` when the value is missing.
    // Java: com/google/devtools/build/lib/skyframe/CompletionFunction.java:364 CompletionFunction.getValueAndArtifactsToBuild
    fn get_configured_aspect(
        &self,
        key: &ActionLookupKeyRef,
        env: &mut Environment<'_>,
    ) -> Result<Option<Arc<dyn ConfiguredAspectView>>, ComputeError> {
        let aspect_key = as_aspect_key(key)
            .unwrap_or_else(|| panic!("ClassCastException: {key} is not an AspectKey"));
        let Some(value) = env.get_value(aspect_key.clone())? else {
            return Ok(None);
        };
        Ok(Some(
            Arc::new(SkyframeConfiguredAspectView::new(aspect_key.clone(), value))
                as Arc<dyn ConfiguredAspectView>,
        ))
    }
}

// -------------------------------------------------------------------------------------------
// The configured target view

/// [`ConfiguredTargetView`] over a `ConfiguredTarget` (the value of a `ConfiguredTargetKey`, or
/// an element of `AnalysisResult.getTargetsToBuild()`).
#[derive(Debug)]
pub struct SkyframeConfiguredTargetView {
    configured_target: ConfiguredTarget,
    /// `ConfiguredTargetValue.getTransitivePackages()` (`None` for a view without the value).
    transitive_packages: Option<
        NestedSet<
            blaze_actions::analysis::repo_mapping_manifest_action::RepoMappingPackageMetadata,
        >,
    >,
    configuration_mnemonic: Option<String>,
    /// The target's location (`ConfiguredTargetAndData.getLocation()`).
    location: Option<blaze_events::location::Location>,
}

impl SkyframeConfiguredTargetView {
    pub fn new(configured_target: ConfiguredTarget) -> SkyframeConfiguredTargetView {
        SkyframeConfiguredTargetView {
            configured_target,
            transitive_packages: None,
            configuration_mnemonic: None,
            location: None,
        }
    }

    /// The viewed target.
    pub fn get_configured_target(&self) -> &ConfiguredTarget {
        &self.configured_target
    }
}

/// `TopLevelArtifactHelper.getAllArtifactsToBuild(target, context)` over every output group the
/// target can contribute (`default` first, then the `OutputGroupInfo` groups in their order),
/// without the groups that have no artifacts.
///
/// PORT: Java iterates over the requested output groups (`context.outputGroups()`); the seam
/// returns every group and the callers (`TargetCompletor`, `BuildResultPrinter`) select the
/// requested ones. A group's artifacts are computed as Java computes them for a requested group,
/// and a group without artifacts is left out as Java's loop skips it ("Ignore output groups that
/// have no artifacts").
// Java: com/google/devtools/build/lib/analysis/TopLevelArtifactHelper.java:199 TopLevelArtifactHelper.getAllArtifactsToBuild
pub fn get_all_artifacts_to_build(
    target: &ConfiguredTarget,
) -> IndexMap<String, NestedSet<Artifact>> {
    // Java: com/google/devtools/build/lib/analysis/OutputGroupInfo.java:173 OutputGroupInfo.get
    let output_group_info_value = target.get(&OUTPUT_GROUP_INFO.key());
    let output_group_info: Option<&OutputGroupInfo> = output_group_info_value
        .as_ref()
        .map(|v| downcast_info::<OutputGroupInfo>(v).expect("OutputGroupInfo"));
    let files_to_build = get_files_to_build(target);
    get_all_artifacts_to_build_of(output_group_info, files_to_build.as_ref())
}

/// [`get_all_artifacts_to_build`] of an aspect (`TopLevelArtifactHelper.getAllArtifactsToBuild`
/// over the `AspectValue` as a `ProviderCollection`, as `AspectCompletor` and
/// `TopLevelArtifactHelper.findAllTopLevelArtifacts` call it).
// Java: com/google/devtools/build/lib/analysis/TopLevelArtifactHelper.java:199 TopLevelArtifactHelper.getAllArtifactsToBuild
pub fn get_all_artifacts_to_build_of_aspect(
    aspect: &AspectValue,
) -> IndexMap<String, NestedSet<Artifact>> {
    let providers = aspect.get_providers();
    // Java: com/google/devtools/build/lib/analysis/OutputGroupInfo.java:173 OutputGroupInfo.get
    let output_group_info: Option<&OutputGroupInfo> = providers
        .get(&OUTPUT_GROUP_INFO.key())
        .map(|v| downcast_info::<OutputGroupInfo>(v).expect("OutputGroupInfo"));
    // Java: com/google/devtools/build/lib/analysis/TopLevelArtifactHelper.java:247 TopLevelArtifactHelper.getFilesToBuild
    let files_to_build: Option<NestedSet<Artifact>> = if let Some(file_provider) =
        providers.get_provider::<FileProvider>()
    {
        Some(file_provider.get_files_to_build().clone())
    } else if let Some(default_info_value) = providers.get(&DEFAULT_INFO.key()) {
        let default_info = blaze_analysis::providers::default_info::DefaultInfo::from_value(
            default_info_value.value(),
        )
        .expect("ClassCastException: DefaultInfo");
        default_info.get_files().map(|files| {
            blaze_analysis::rule::depsets::depset_to_artifacts(files, "files").unwrap_or_else(|e| {
                panic!(
                    "IllegalStateException: Error getting 'files' field of \
                             'DefaultInfo': {e}"
                )
            })
        })
    } else {
        None
    };
    get_all_artifacts_to_build_of(output_group_info, files_to_build.as_ref())
}

// Java: com/google/devtools/build/lib/analysis/TopLevelArtifactHelper.java:204 TopLevelArtifactHelper.getAllArtifactsToBuild
fn get_all_artifacts_to_build_of(
    output_group_info: Option<&OutputGroupInfo>,
    files_to_build: Option<&NestedSet<Artifact>>,
) -> IndexMap<String, NestedSet<Artifact>> {
    let mut output_groups: Vec<String> = vec![DEFAULT.to_owned()];
    if let Some(info) = output_group_info {
        for group in info.groups() {
            if group != DEFAULT {
                output_groups.push(group);
            }
        }
    }

    let mut all_output_groups: IndexMap<String, NestedSet<Artifact>> = IndexMap::new();
    for output_group in output_groups {
        let mut results = NestedSetBuilder::stable_order();

        if output_group == DEFAULT
            && let Some(files_to_build) = files_to_build
        {
            results.add_transitive(files_to_build);
        }

        if let Some(info) = output_group_info {
            results.add_transitive(&info.get_output_group(&output_group));
        }

        // Ignore output groups that have no artifacts.
        if results.is_empty() {
            continue;
        }

        // PORT: `isImportantGroup` (`!outputGroup.startsWith(HIDDEN_OUTPUT_GROUP_PREFIX)`) is
        // recomputed from the name by the callers (see `is_important_group`).
        all_output_groups.insert(output_group, results.build());
    }
    all_output_groups
}

/// `ArtifactsInOutputGroup.areImportant()` of a group: whether the user should know about it.
// Java: com/google/devtools/build/lib/analysis/TopLevelArtifactHelper.java:225 TopLevelArtifactHelper.getAllArtifactsToBuild
pub fn is_important_group(output_group: &str) -> bool {
    !output_group.starts_with(HIDDEN_OUTPUT_GROUP_PREFIX)
}

/// Returns files to build directly from `FileProvider` or from `files` under `DefaultInfo`.
// Java: com/google/devtools/build/lib/analysis/TopLevelArtifactHelper.java:247 TopLevelArtifactHelper.getFilesToBuild
fn get_files_to_build(target: &ConfiguredTarget) -> Option<NestedSet<Artifact>> {
    if let Some(file_provider) = target.get_provider::<FileProvider>() {
        return Some(file_provider.get_files_to_build().clone());
    }
    // PORT: every ported configured target class but EnvironmentGroupConfiguredTarget carries a
    // FileProvider; the DefaultInfo branch reads `DefaultInfo.getFiles()`, which the native
    // DefaultInfo port does not expose for that case.
    unported!(
        "TopLevelArtifactHelper.getFilesToBuild",
        "DefaultInfo.getFiles() of {}",
        target.get_label()
    )
}

impl ConfiguredTargetView for SkyframeConfiguredTargetView {
    // Java: com/google/devtools/build/lib/analysis/ConfiguredTarget.java:37 ConfiguredTarget.getLabel
    fn get_label(&self) -> Label {
        self.configured_target.get_label()
    }

    // Java: com/google/devtools/build/lib/skyframe/AbstractConfiguredTargetValue.java:49 AbstractConfiguredTargetValue.getTransitivePackages
    fn get_transitive_packages(
        &self,
    ) -> Option<
        NestedSet<
            blaze_actions::analysis::repo_mapping_manifest_action::RepoMappingPackageMetadata,
        >,
    > {
        self.transitive_packages.clone()
    }

    // Java: com/google/devtools/build/lib/skyframe/BuildDriverFunction.java:600 BuildDriverFunction.requestConfiguredTargetExecution
    fn get_extra_action_artifacts_provider(
        &self,
    ) -> Option<(NestedSet<Artifact>, NestedSet<Artifact>)> {
        use blaze_analysis::configuredtargets::extra_action_artifacts_provider::ExtraActionArtifactsProvider;
        self.configured_target
            .get_provider::<ExtraActionArtifactsProvider>()
            .map(|p| {
                (
                    p.get_extra_action_artifacts().clone(),
                    p.get_transitive_extra_action_artifacts().clone(),
                )
            })
    }

    // Java: com/google/devtools/build/lib/rules/AliasConfiguredTarget.java:176 AliasConfiguredTarget.getOriginalLabel
    fn get_original_label(&self) -> Label {
        self.configured_target.get_original_label()
    }

    // Java: com/google/devtools/build/lib/analysis/configuredtargets/AbstractConfiguredTarget.java:55 AbstractConfiguredTarget.getConfigurationChecksum
    fn get_configuration_checksum(&self) -> Option<String> {
        self.configured_target.get_configuration_checksum()
    }

    /// `ConfiguredTargetKey.fromConfiguredTarget(configuredTarget)`: the lookup key of the
    /// (unmerged) target, which differs from the requested key when the requested key delegated
    /// to another (a top-level rule transition).
    // Java: com/google/devtools/build/lib/skyframe/ConfiguredTargetKey.java:236 ConfiguredTargetKey.fromConfiguredTarget
    fn get_lookup_key(&self) -> ActionLookupKeyRef {
        self.configured_target
            .unwrap_if_merged()
            .get_lookup_key()
            .clone()
    }

    /// The rule class (`getRuleClassString()`) for rules, aliases (`""`, as Java's
    /// `AbstractConfiguredTarget.getRuleClassString`) and merged targets; the target kind of
    /// `InputFileConfiguredTarget` (`source file`) and `OutputFileConfiguredTarget`
    /// (`generated file`), the two classes `TopLevelArtifactHelper.shouldConsiderForDisplay` and
    /// `BuildResultPrinter.filterTargetsToPrint` test with `instanceof`.
    // Java: com/google/devtools/build/lib/analysis/configuredtargets/AbstractConfiguredTarget.java:236 AbstractConfiguredTarget.getRuleClassString
    fn get_rule_class_or_kind(&self) -> String {
        match &self.configured_target {
            ConfiguredTarget::InputFile(_) => InputFile::target_kind().to_owned(),
            ConfiguredTarget::OutputFile(_) => OutputFile::target_kind().to_owned(),
            ct => get_rule_class_string(ct),
        }
    }

    fn get_output_groups(&self) -> IndexMap<String, NestedSet<Artifact>> {
        get_all_artifacts_to_build(&self.configured_target)
    }

    fn get_configuration_mnemonic(&self) -> Option<String> {
        self.configuration_mnemonic.clone()
    }

    // Java: com/google/devtools/build/lib/skyframe/ConfiguredTargetAndData.java ConfiguredTargetAndData.getLocation
    fn get_location(&self) -> blaze_events::location::Location {
        self.location
            .clone()
            .expect("IllegalStateException: configured target view without its target")
    }

    // Java: com/google/devtools/build/lib/analysis/FilesToRunProvider.java:84 FilesToRunProvider.getExecutable
    fn get_executable(&self) -> Option<Artifact> {
        get_files_to_run_provider(&self.configured_target).and_then(|p| p.get_executable().cloned())
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestProvider.java:42 TestProvider.getTestParams
    fn get_test_params(&self) -> Option<TestParams> {
        let provider = self.configured_target.get_provider::<TestProvider>()?;
        let params = provider.get_test_params();
        Some(TestParams {
            runs: params.get_runs(),
            shards: params.get_shards(),
            runs_detects_flakes: params.runs_detects_flakes(),
            timeout: params.get_timeout(),
            test_rule_class: params.get_test_rule_class().to_owned(),
            test_status_artifacts: params.get_test_status_artifacts().to_vec(),
        })
    }
}

/// The views of `AnalysisResult.getTargetsToBuild()` (for `BuildResultPrinter` and the test
/// summary).
pub fn configured_target_views(targets: &[ConfiguredTarget]) -> Vec<Arc<dyn ConfiguredTargetView>> {
    targets
        .iter()
        .map(|ct| {
            Arc::new(SkyframeConfiguredTargetView::new(ct.clone())) as Arc<dyn ConfiguredTargetView>
        })
        .collect()
}

// -------------------------------------------------------------------------------------------
// Top-level aspects

/// [`ConfiguredAspectView`] over an `AspectValue` and its `AspectKey` (an entry of
/// `AnalysisResult.getAspectsMap()` or of a `TopLevelAspectsValue`).
pub struct SkyframeConfiguredAspectView {
    key: AspectKey,
    value: Arc<AspectValue>,
}

impl SkyframeConfiguredAspectView {
    pub fn new(key: AspectKey, value: Arc<AspectValue>) -> SkyframeConfiguredAspectView {
        SkyframeConfiguredAspectView { key, value }
    }

    /// The `AspectKey`.
    pub fn get_key(&self) -> &AspectKey {
        &self.key
    }

    /// The `AspectValue`.
    pub fn get_value(&self) -> &Arc<AspectValue> {
        &self.value
    }

    /// `AspectKey.getAspectDescriptor().getDescription()` (the `AspectConfiguredEvent`'s
    /// description).
    // Java: com/google/devtools/build/lib/packages/AspectDescriptor.java AspectDescriptor.getDescription
    pub fn get_aspect_description(&self) -> String {
        self.key.get_aspect_descriptor().get_description()
    }

    /// `AspectKey.getLabel() + ", aspect " + getAspectClass().getName()`.
    // Java: com/google/devtools/build/lib/skyframe/AspectCompletor.java:88 AspectCompletor.getLocationIdentifier
    pub fn get_location_identifier(&self) -> String {
        format!(
            "{}, aspect {}",
            self.key.get_label(),
            self.key.get_aspect_class().get_name()
        )
    }
}

impl fmt::Debug for SkyframeConfiguredAspectView {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self.key)
    }
}

impl ConfiguredAspectView for SkyframeConfiguredAspectView {
    // Java: com/google/devtools/build/lib/skyframe/AspectKeyCreator.java:171 AspectKeyCreator.AspectKey.getLabel
    fn get_label(&self) -> Label {
        self.key.get_label().clone()
    }

    // Java: com/google/devtools/build/lib/analysis/AspectValue.java:124 AspectValue.getTransitivePackages
    fn get_transitive_packages(
        &self,
    ) -> Option<
        NestedSet<
            blaze_actions::analysis::repo_mapping_manifest_action::RepoMappingPackageMetadata,
        >,
    > {
        self.value.get_transitive_packages().cloned()
    }

    // Java: com/google/devtools/build/lib/skyframe/AspectKeyCreator.java:185 AspectKeyCreator.AspectKey.getAspectClass
    fn get_aspect_class_name(&self) -> String {
        self.key.get_aspect_class().get_name().to_owned()
    }

    // Java: com/google/devtools/build/lib/skyframe/AspectKeyCreator.java:79 AspectKeyCreator.AspectBaseKey.getBaseConfiguredTargetKey
    fn get_base_configured_target_key(&self) -> ActionLookupKeyRef {
        ActionLookupKeyRef::new(self.key.get_base_configured_target_key().clone())
    }

    fn get_output_groups(&self) -> IndexMap<String, NestedSet<Artifact>> {
        get_all_artifacts_to_build_of_aspect(&self.value)
    }

    // Java: com/google/devtools/build/lib/skyframe/BuildDriverFunction.java:330 BuildDriverFunction.compute (`entry.getKey()`)
    fn get_aspect_key(&self) -> ActionLookupKeyRef {
        ActionLookupKeyRef::new(self.key.clone())
    }

    // Java: com/google/devtools/build/lib/analysis/AspectValue.java:105 AspectValue.getWritesOutputToMasterLog
    fn get_writes_output_to_master_log(&self) -> bool {
        self.value.get_writes_output_to_master_log()
    }

    // Java: com/google/devtools/build/lib/skyframe/BuildDriverFunction.java:333 BuildDriverFunction.compute
    fn get_extra_action_artifacts_provider(
        &self,
    ) -> Option<(NestedSet<Artifact>, NestedSet<Artifact>)> {
        use blaze_analysis::configuredtargets::extra_action_artifacts_provider::ExtraActionArtifactsProvider;
        self.value
            .get_providers()
            .get_provider::<ExtraActionArtifactsProvider>()
            .map(|p| {
                (
                    p.get_extra_action_artifacts().clone(),
                    p.get_transitive_extra_action_artifacts().clone(),
                )
            })
    }
}

/// The views of `AnalysisResult.getAspectsMap()` (for `BuildResultPrinter`), keyed by the
/// `AspectKey`s in the map's order.
pub fn configured_aspect_views(
    aspects: &IndexMap<AspectKey, Arc<AspectValue>>,
) -> IndexMap<ActionLookupKeyRef, Arc<dyn ConfiguredAspectView>> {
    aspects
        .iter()
        .map(|(k, v)| {
            (
                ActionLookupKeyRef::new(k.clone()),
                Arc::new(SkyframeConfiguredAspectView::new(k.clone(), v.clone()))
                    as Arc<dyn ConfiguredAspectView>,
            )
        })
        .collect()
}

// -------------------------------------------------------------------------------------------
// Build driver keys

/// `BuildRequest.validationMode()`.
// Java: com/google/devtools/build/lib/buildtool/BuildRequest.java:452 BuildRequest.validationMode
pub fn validation_mode(build_options: &BuildRequestOptions) -> ValidationMode {
    if !build_options.run_validation_actions {
        return ValidationMode::Off;
    }
    if build_options.use_validation_aspect {
        ValidationMode::Aspect
    } else {
        ValidationMode::OutputGroup
    }
}

/// `BuildRequest.getTopLevelArtifactContext().outputGroups()`: the requested output groups, in
/// `ImmutableSortedSet` order. `should_run_tests` is `BuildRequest.shouldRunTests()` (true for
/// `test`).
///
/// PORT: the other `TopLevelArtifactContext` fields are not part of `BuildDriverKey` here:
/// `runTestsExclusively` (`--test_strategy=exclusive`) is read by [`build_driver_keys`] through
/// [`BuildDriverKeyTestContext`], `expandFilesets` and `forRunCommand` only matter to BEP and
/// `run`.
// Java: com/google/devtools/build/lib/buildtool/BuildRequest.java:410 BuildRequest.getTopLevelArtifactContext
pub fn top_level_output_groups(
    build_options: &BuildRequestOptions,
    should_run_tests: bool,
) -> Vec<String> {
    OutputGroupInfo::determine_output_groups(
        &output_group_info::default_groups(),
        &build_options.output_groups,
        validation_mode(build_options),
        should_run_tests,
    )
    .into_iter()
    .collect()
}

/// Returns the set of labels of target patterns that represent single targets, in their order
/// on the command line. `offset` is the relative working directory and `main_repo_mapping` the
/// main repository's mapping.
// Java: com/google/devtools/build/lib/buildtool/AnalysisAndExecutionPhaseRunner.java:246 AnalysisAndExecutionPhaseRunner.getExplicitTargetPatterns
pub fn get_explicit_target_patterns(
    offset: &PathFragment,
    main_repo_mapping: &RepositoryMapping,
    requested_target_patterns: &[String],
) -> IndexSet<Label> {
    let mut explicit_target_patterns: IndexSet<Label> = IndexSet::new();

    let parser = TargetPatternParser::new(offset.clone(), MAIN.clone(), main_repo_mapping.clone());

    for requested_target_pattern in requested_target_patterns {
        if requested_target_pattern.starts_with('-') {
            // Excluded patterns are by definition not explicitly requested so we can move on to
            // the next target pattern.
            continue;
        }

        // Parse the pattern. This should always work because this is at least the second time
        // we're doing it. The previous time is in runAnalysisAndExecutionPhase(). Still, if
        // parsing does fail we propagate the exception up.
        let parsed_pattern = match parser.parse(requested_target_pattern) {
            Ok(p) => p,
            Err(e) => unported!(
                "ViewCreationFailedException",
                "Failed to parse target pattern even though it was previously parsed \
                 successfully: {}",
                e.get_message()
            ),
        };

        if parsed_pattern.get_type() == TargetPatternType::SingleTarget {
            explicit_target_patterns.insert(parsed_pattern.get_single_target_label().clone());
        }
    }

    explicit_target_patterns
}

/// Encapsulates the context required to construct a test BuildDriverKey.
// Java: com/google/devtools/build/lib/skyframe/SkyframeBuildView.java:1553 SkyframeBuildView.BuildDriverKeyTestContext
#[derive(Clone, Debug)]
pub struct BuildDriverKeyTestContext {
    /// `getTestStrategy()`: `ExecutionOptions.testStrategy`.
    pub test_strategy: String,
    /// `forceExclusiveTestsInParallel()`: the `TestActionContext`'s.
    pub force_exclusive_tests_in_parallel: bool,
    /// `forceExclusiveIfLocalTestsInParallel()`: the `TestActionContext`'s.
    pub force_exclusive_if_local_tests_in_parallel: bool,
}

/// `BuildDriverKey.TestType.getMsg()`.
// Java: com/google/devtools/build/lib/skyframe/BuildDriverKey.java:150 BuildDriverKey.TestType
fn test_type_msg(test_type: TestType) -> &'static str {
    match test_type {
        TestType::NOT_TEST => "not-test",
        TestType::PARALLEL => "parallel",
        TestType::EXCLUSIVE => "exclusive",
        TestType::EXCLUSIVE_IF_LOCAL => "exclusive-if-local",
    }
}

/// The test type of the top-level target `label` (the configured target's `getLabel()`).
// Java: com/google/devtools/build/lib/skyframe/SkyframeBuildView.java:1115 SkyframeBuildView.determineTestTypeImpl
pub fn determine_test_type_impl(
    tests_to_run: Option<&IndexSet<Label>>,
    label_target_map: &IndexMap<Label, Target>,
    label: &Label,
    build_driver_key_test_context: &BuildDriverKeyTestContext,
    event_handler: &dyn ExtendedEventHandler,
) -> TestType {
    if !tests_to_run.is_some_and(|t| t.contains(label)) {
        return TestType::NOT_TEST;
    }
    let Some(Target::Rule(rule)) = label_target_map.get(label) else {
        return TestType::NOT_TEST;
    };

    let from_explicit_flag_or_tag = if build_driver_key_test_context.test_strategy == "exclusive"
        || target_utils::is_exclusive_test_rule(rule)
        || (target_utils::is_exclusive_if_local_test_rule(rule)
            && target_utils::is_local_test_rule(rule))
    {
        TestType::EXCLUSIVE
    } else if target_utils::is_exclusive_if_local_test_rule(rule) {
        TestType::EXCLUSIVE_IF_LOCAL
    } else {
        TestType::PARALLEL
    };

    if (from_explicit_flag_or_tag == TestType::EXCLUSIVE
        && build_driver_key_test_context.force_exclusive_tests_in_parallel)
        || (from_explicit_flag_or_tag == TestType::EXCLUSIVE_IF_LOCAL
            && build_driver_key_test_context.force_exclusive_if_local_tests_in_parallel)
    {
        event_handler.handle(&Event::warn(format!(
            "{} is tagged {}, but --test_strategy={} forces parallel test execution.",
            label.get_canonical_form(),
            test_type_msg(from_explicit_flag_or_tag),
            build_driver_key_test_context.test_strategy
        )));
        return TestType::PARALLEL;
    }
    from_explicit_flag_or_tag
}

/// Creates the `BuildDriverKey`s of the top-level configured targets of a build or test request
/// (the `buildDriverCTKeys` of `SkyframeBuildView.analyzeAndExecuteTargets`).
///
/// Inputs:
/// - `label_to_target_map`: `BuildView.constructLabelToTargetMap(loadingResult)` (the top-level
///   labels of `TargetPatternPhaseValue.getTargetLabels()` with their `Target`s, in order).
/// - `top_level_configuration`: `AnalysisResult.getConfiguration()`; the keys are Java's
///   `topLevelCtKeys` (`BuildView.update`): one `ConfiguredTargetKey` per label with this
///   configuration, so a target whose analysis failed still gets its key (its build driver
///   fails) as in Java.
/// - `targets_to_build`: `AnalysisResult.getTargetsToBuild()`, to resolve each key's configured
///   target for the test type (Java resolves it from the key's value in `BuildDriverFunction`).
/// - `tests_to_run`: `TargetPatternPhaseValue.getTestsToRunLabels()` (`None` for `build`).
/// - `explicit_target_patterns`: [`get_explicit_target_patterns`].
/// - `output_groups`: [`top_level_output_groups`].
/// - `skip_incompatible_explicit_targets`: `AnalysisOptions.skipIncompatibleExplicitTargets`;
///   `extra_action_top_level_only`: `AnalysisOptions.extraActionTopLevelOnly`.
/// - `test_context`: `--test_strategy` and the test action context's `forceExclusive*`.
///
/// PORT: Java's key does not carry the test type: `BuildDriverFunction` asks the
/// `TestTypeResolver` (`determineTestTypeImpl` over `configuredTarget.getLabel()`) once the
/// target is analyzed. Analysis runs before execution here, so the type is resolved now, from the
/// analyzed target with the key's label (`getOriginalLabel()`, the label analysis was asked
/// for); the warning of `determineTestTypeImpl` is posted now rather than during execution. A
/// key whose target failed analysis gets `NOT_TEST` (Java never resolves it). Java's
/// `keepGoing` field of the key is not part of the Rust key.
// Java: com/google/devtools/build/lib/skyframe/SkyframeBuildView.java:618 SkyframeBuildView.analyzeAndExecuteTargets
pub fn build_driver_keys(
    label_to_target_map: &IndexMap<Label, Target>,
    top_level_configuration: &BuildConfigurationValue,
    targets_to_build: &[ConfiguredTarget],
    tests_to_run: Option<&IndexSet<Label>>,
    explicit_target_patterns: &IndexSet<Label>,
    output_groups: &[String],
    skip_incompatible_explicit_targets: bool,
    extra_action_top_level_only: bool,
    test_context: &BuildDriverKeyTestContext,
    event_handler: &dyn ExtendedEventHandler,
) -> Vec<BuildDriverKey> {
    // Java: com/google/devtools/build/lib/analysis/BuildView.java:361 BuildView.update
    let configuration_key = top_level_configuration.get_key();
    let top_level_ct_keys: Vec<ConfiguredTargetKey> = label_to_target_map
        .keys()
        .map(|label| {
            ConfiguredTargetKey::builder()
                .set_label(label.clone())
                .set_configuration_key(Some(configuration_key.clone()))
                .build()
        })
        .collect();

    let mut analyzed: IndexMap<Label, &ConfiguredTarget> = IndexMap::new();
    for ct in targets_to_build {
        analyzed.entry(ct.get_original_label()).or_insert(ct);
    }

    let mut build_driver_ct_keys: IndexSet<BuildDriverKey> = IndexSet::new();
    for ct_key in top_level_ct_keys {
        let label = ct_key.get_label().clone();
        // Java: com/google/devtools/build/lib/skyframe/BuildDriverFunction.java:219 BuildDriverFunction.compute
        let test_type = match analyzed.get(&label) {
            Some(ct) => determine_test_type_impl(
                tests_to_run,
                label_to_target_map,
                &ct.get_label(),
                test_context,
                event_handler,
            ),
            None => TestType::NOT_TEST,
        };
        // Java: com/google/devtools/build/lib/skyframe/BuildDriverKey.java:72 BuildDriverKey.ofConfiguredTarget
        build_driver_ct_keys.insert(BuildDriverKey {
            action_lookup_key: ActionLookupKeyRef::new(ct_key),
            output_groups: output_groups.to_vec(),
            explicitly_requested: explicit_target_patterns.contains(&label),
            skip_incompatible_explicit_targets,
            extra_action_top_level_only,
            test_type,
        });
    }
    build_driver_ct_keys.into_iter().collect()
}

/// Whether the build command created the `BuildDriverKey.ofTopLevelAspect` keys of this
/// command's top-level aspects.
///
/// PORT: Java creates the configured-target and the aspect driver keys in one method
/// (`SkyframeBuildView.analyzeAndExecuteTargets`). In Blaze the build command creates them from
/// the analysis result in two calls ([`build_driver_keys`], [`build_driver_aspect_keys`]); a
/// command that analyzed top-level aspects for a build and then starts the execution phase
/// without the second call would build no aspect and still report success. That fails loudly
/// here. The state is per build: [`note_top_level_aspects_analyzed_for_execution`] sets both
/// fields when `BuildView.update` starts the build's execution phase. It lives in the build arena
/// (`blaze_util::build_arena`), which the persistent server also releases after every command.
#[derive(Default)]
struct TopLevelAspectDrivers {
    /// The number of `TopLevelAspectsKey`s `BuildView.update` analyzed with the execution phase.
    analyzed: std::sync::atomic::AtomicUsize,
    /// [`build_driver_aspect_keys`] was called.
    created: std::sync::atomic::AtomicBool,
}

impl blaze_util::build_arena::ArenaSlot for TopLevelAspectDrivers {
    const RELEASE_ORDER: blaze_util::build_arena::ReleaseOrder =
        blaze_util::build_arena::ReleaseOrder::State;
}

fn top_level_aspect_drivers() -> std::sync::Arc<TopLevelAspectDrivers> {
    blaze_util::build_arena::slot::<TopLevelAspectDrivers>()
}

/// `BuildView.update` analyzed `count` `TopLevelAspectsKey`s for a build with an execution
/// phase.
pub(crate) fn note_top_level_aspects_analyzed_for_execution(count: usize) {
    top_level_aspect_drivers()
        .analyzed
        .store(count, Ordering::SeqCst);
    top_level_aspect_drivers()
        .created
        .store(false, Ordering::SeqCst);
}

/// Called when the execution phase first asks for a top-level configured target.
// Java: com/google/devtools/build/lib/skyframe/SkyframeBuildView.java:680 SkyframeBuildView.analyzeAndExecuteTargets
fn check_top_level_aspect_drivers_created() {
    let analyzed = top_level_aspect_drivers().analyzed.load(Ordering::SeqCst);
    if analyzed > 0 && !top_level_aspect_drivers().created.load(Ordering::SeqCst) {
        unported!(
            "SkyframeBuildView.analyzeAndExecuteTargets",
            "the build command did not create the BuildDriverKey.ofTopLevelAspect keys \
             (execution_seam::build_driver_aspect_keys) of {} top-level aspect key(s)",
            analyzed
        );
    }
}

/// Creates the `BuildDriverKey`s of the top-level aspects of a build request (the
/// `buildDriverAspectKeys` of `SkyframeBuildView.analyzeAndExecuteTargets`), one per
/// `TopLevelAspectsKey` (`AnalysisResult::get_top_level_aspects_keys`), in their order. Java's
/// `topLevelKeys` is the union of the configured-target keys ([`build_driver_keys`]) and these.
///
/// The arguments are those of [`build_driver_keys`]: `explicit_target_patterns`
/// ([`get_explicit_target_patterns`]), `output_groups` ([`top_level_output_groups`]),
/// `skip_incompatible_explicit_targets` and `extra_action_top_level_only`.
// Java: com/google/devtools/build/lib/skyframe/SkyframeBuildView.java:680 SkyframeBuildView.analyzeAndExecuteTargets
pub fn build_driver_aspect_keys(
    top_level_aspects_keys: &[TopLevelAspectsKey],
    explicit_target_patterns: &IndexSet<Label>,
    output_groups: &[String],
    skip_incompatible_explicit_targets: bool,
    extra_action_top_level_only: bool,
) -> Vec<BuildDriverKey> {
    top_level_aspect_drivers()
        .created
        .store(true, Ordering::SeqCst);
    let mut build_driver_aspect_keys: IndexSet<BuildDriverKey> = IndexSet::new();
    for k in top_level_aspects_keys {
        build_driver_aspect_keys.insert(BuildDriverKey::of_top_level_aspect(
            ActionLookupKeyRef::new(k.clone()),
            output_groups.to_vec(),
            /* explicitly_requested= */ explicit_target_patterns.contains(k.get_label()),
            skip_incompatible_explicit_targets,
            extra_action_top_level_only,
        ));
    }
    build_driver_aspect_keys.into_iter().collect()
}
