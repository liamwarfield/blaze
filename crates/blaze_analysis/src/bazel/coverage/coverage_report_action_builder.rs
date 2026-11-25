//! Port of `bazel/coverage/CoverageReportActionBuilder.java`.
//!
//! A class to create the coverage report generator action.
//!
//! The coverage report action is created after every test shard action is created, at the very
//! end of the analysis phase. There is only one coverage report action per coverage command
//! invocation. It can also be viewed as a single sink node of the action graph.
//!
//! Its inputs are the individual coverage.dat files from the test outputs (each shard produces
//! one) and the baseline coverage artifacts. Note that each ConfiguredTarget among the
//! transitive dependencies of the top level test targets may provide baseline coverage
//! artifacts.
//!
//! The coverage report generation can have two phases, though they both run in the same action.
//! The first phase is merging the individual coverage files into a single report file. The
//! location of this file is reported by Blaze. This phase always happens if the
//! `--combined_report=lcov` or `--combined_report=html`. The second phase is generating an html
//! report. It only happens if `--combined_report=html`.

use std::any::Any;
use std::cmp::Ordering;
use std::sync::Arc;

use blaze_actions::actions::action::{AbstractAction, ComputeKeyError, InputMetadataProvider};
use blaze_actions::actions::artifact_factory::ArtifactFactory;
use blaze_actions::analysis::actions::lazy_write_paths_file_action::LazyWritePathsFileAction;
use blaze_actions::{
    Action, ActionKeyContext, ActionLookupKeyRef, ActionOwner, ActionRef, Artifact, ArtifactOwner,
};
use blaze_collect::{NestedSet, NestedSetBuilder, Order};
use blaze_events::{Event, EventHandler};
use blaze_exec::actions::{
    ActionExecutionException, ActionResult, ExecException, ExecutableAction, ResourceSet,
    ResourceSetOrBuilder, SimpleSpawn,
};
use blaze_exec::exec::action_execution_context::ActionExecutionContext;
use blaze_exec::exec::spawn_strategy_resolver::SpawnStrategyResolver;
use blaze_util::Fingerprint;
use blaze_vfs::PathFragment;
use indexmap::{IndexMap, IndexSet};

use super::coverage_args::CoverageArgs;
use crate::analysis_services::AnalysisDirectories;
use crate::configured_target::ConfiguredTarget;
use crate::info::downcast_info;
use crate::providers::file_provider::FilesToRunProvider;
use crate::test::coverage_report_action_factory::CoverageReportActionsWrapper;
use crate::test::instrumented_files_info::InstrumentedFilesInfo;
use crate::test::test_provider::TestProvider;
use crate::test::test_runner_action::COVERAGE_TMP_ROOT;

// Java: com/google/devtools/build/lib/bazel/coverage/CoverageReportActionBuilder.java:101 CoverageReportActionBuilder.LOCAL_RESOURCES
fn local_resources() -> ResourceSet {
    ResourceSet::create_with_ram_cpu(/* memory_mb= */ 750.0, /* cpu= */ 1.0)
}

/// `ACTION_OWNER_COMPARATOR`: owners with exec properties first, then by label, then by
/// configuration checksum.
// Java: com/google/devtools/build/lib/bazel/coverage/CoverageReportActionBuilder.java:104 CoverageReportActionBuilder.ACTION_OWNER_COMPARATOR
fn action_owner_comparator(a: &ActionOwner, b: &ActionOwner) -> Ordering {
    // comparing(getExecProperties().isEmpty(), falseFirst())
    a.get_exec_properties()
        .is_empty()
        .cmp(&b.get_exec_properties().is_empty())
        .then_with(|| a.get_label().cmp(&b.get_label()))
        .then_with(|| {
            a.get_configuration_checksum()
                .cmp(b.get_configuration_checksum())
        })
}

/// `ResourceSetOrBuilder.ignoringOverrides(resources)`: a declaration of `resources` that the
/// owning target's "resources:*" entries do not override.
// Java: com/google/devtools/build/lib/actions/ResourceSetOrBuilder.java:52 ResourceSetOrBuilder.ignoringOverrides
#[derive(Debug)]
struct IgnoringOverrides(ResourceSet);

impl ResourceSetOrBuilder for IgnoringOverrides {
    fn build_resource_set(&self, _inputs_size: i32) -> ResourceSet {
        self.0.clone()
    }

    fn build_local_resources(
        &self,
        _inputs_size: i32,
        _execution_info: &IndexMap<String, String>,
        _exec_properties: &IndexMap<String, String>,
    ) -> Result<ResourceSet, ExecException> {
        Ok(self.0.clone())
    }
}

/// The coverage report action.
///
/// SpawnActions can't be used because they need the AnalysisEnvironment and this action is
/// created specially at the very end of the analysis phase when we don't have it anymore.
// Java: com/google/devtools/build/lib/bazel/coverage/CoverageReportActionBuilder.java:113 CoverageReportActionBuilder.CoverageReportAction
#[derive(Debug)]
pub struct CoverageReportAction {
    base: AbstractAction,
    command: Vec<String>,
    location_message: String,
}

impl CoverageReportAction {
    // Java: com/google/devtools/build/lib/bazel/coverage/CoverageReportActionBuilder.java:118 CoverageReportAction.<init>
    fn new(
        owner: ActionOwner,
        inputs: NestedSet<Artifact>,
        outputs: IndexSet<Artifact>,
        command: Vec<String>,
        location_message: String,
    ) -> CoverageReportAction {
        CoverageReportAction {
            base: AbstractAction::new(owner, inputs, outputs),
            command,
            location_message,
        }
    }

    /// The command line of the report generator.
    pub fn get_command(&self) -> &[String] {
        &self.command
    }
}

impl Action for CoverageReportAction {
    fn abstract_action(&self) -> &AbstractAction {
        &self.base
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    // Java: com/google/devtools/build/lib/bazel/coverage/CoverageReportActionBuilder.java:177 CoverageReportAction.getMnemonic
    fn get_mnemonic(&self) -> String {
        "CoverageReport".to_owned()
    }

    // Java: com/google/devtools/build/lib/bazel/coverage/CoverageReportActionBuilder.java:182 CoverageReportAction.getRawProgressMessage
    fn get_raw_progress_message(&self) -> Option<String> {
        Some("Coverage report generation".to_owned())
    }

    // Java: com/google/devtools/build/lib/bazel/coverage/CoverageReportActionBuilder.java:187 CoverageReportAction.computeKey
    fn compute_key(
        &self,
        _action_key_context: &ActionKeyContext,
        _input_metadata_provider: Option<&dyn InputMetadataProvider>,
        fp: &mut Fingerprint,
    ) -> Result<(), ComputeKeyError> {
        fp.add_strings(&self.command);
        Ok(())
    }
}

impl ExecutableAction for CoverageReportAction {
    // Java: com/google/devtools/build/lib/bazel/coverage/CoverageReportActionBuilder.java:130 CoverageReportAction.execute
    fn execute(
        &self,
        ctx: &dyn ActionExecutionContext,
    ) -> Result<ActionResult, ActionExecutionException> {
        // The resources are fixed because this action borrows an arbitrary tested target's
        // ActionOwner (see ACTION_OWNER_COMPARATOR): its exec properties describe that test
        // rather than this report merge, so charging their `resources:` entries would tie the
        // report's scheduling to whichever target happened to sort largest.
        // new BaseSpawn(command, ImmutableMap.of(), ImmutableMap.of(), this, ...): BaseSpawn
        // takes its inputs, tools and outputs from the action.
        let spawn = SimpleSpawn::new(
            blaze_exec::actions::action_adapters::ActionMetadataSnapshot::of(self),
            self.command.clone(),
            IndexMap::new(),
            IndexMap::new(),
            blaze_exec::analysis::actions::to_action_input_set(&self.get_inputs()),
            blaze_exec::analysis::actions::to_action_input_set(&self.get_tools()),
            self.get_outputs()
                .iter()
                .map(blaze_exec::actions::action_adapters::artifact_input)
                .collect(),
            None,
            Arc::new(IgnoringOverrides(local_resources())),
            blaze_exec::actions::PathMapper::NOOP,
        );
        let resolver = blaze_exec::exec::action_execution_context::get_context::<
            SpawnStrategyResolver,
        >(ctx)
        .unwrap_or_else(|| {
            panic!("NullPointerException: getContext(SpawnStrategyResolver.class) returned null")
        });
        let spawn_results = resolver.exec(&spawn, ctx).map_err(|e| match &e {
            ExecException::Spawn(spawn_exec_exception) => {
                blaze_exec::analysis::actions::spawn_action::to_action_execution_exception(
                    spawn_exec_exception,
                    self,
                )
            }
            _ => ActionExecutionException::from_exec_exception(&e, None, self),
        })?;
        // informImportantOutputHandler(ctx)
        // PORT: ImportantOutputHandler is not ported (only the remote module registers one);
        // without the context Java returns at once.
        ctx.get_event_handler()
            .handle(&Event::info(self.location_message.clone()));
        Ok(ActionResult::create(spawn_results))
    }

    // Java: com/google/devtools/build/lib/bazel/coverage/CoverageReportActionBuilder.java:195 CoverageReportAction.actionCacheHit
    fn action_cache_hit(&self, event_handler: &dyn EventHandler) -> Option<bool> {
        event_handler.handle(&Event::info(self.location_message.clone()));
        Some(true)
    }
}

/// A helper interface for product-specific coverage support.
// Java: com/google/devtools/build/lib/bazel/coverage/CoverageReportActionBuilder.java:328 CoverageReportActionBuilder.CoverageHelper
pub trait CoverageHelper {
    /// Returns the arguments for the coverage report generator action.
    fn get_args(&self, args: &CoverageArgs<'_>, lcov_output: &Artifact) -> Vec<String>;

    /// Returns a message describing the location of the coverage report.
    fn get_location_message(&self, args: &CoverageArgs<'_>, lcov_output: &Artifact) -> String;

    /// Returns additional inputs for the coverage report action.
    fn get_instrumented_files(
        &self,
        _args: &CoverageArgs<'_>,
        _configured_targets: &[ConfiguredTarget],
    ) -> NestedSet<Artifact> {
        NestedSetBuilder::empty_set(Order::STABLE_ORDER)
    }
}

/// A class to create the coverage report generator action.
// Java: com/google/devtools/build/lib/bazel/coverage/CoverageReportActionBuilder.java:99 CoverageReportActionBuilder
#[derive(Default)]
pub struct CoverageReportActionBuilder;

impl CoverageReportActionBuilder {
    pub fn new() -> CoverageReportActionBuilder {
        CoverageReportActionBuilder
    }

    /// Returns the coverage report action. May return `None` in case of an error.
    // Java: com/google/devtools/build/lib/bazel/coverage/CoverageReportActionBuilder.java:205 CoverageReportActionBuilder.createCoverageActionsWrapper
    #[allow(clippy::too_many_arguments)]
    pub fn create_coverage_actions_wrapper(
        &self,
        reporter: &dyn EventHandler,
        directories: &AnalysisDirectories,
        configured_targets: &[ConfiguredTarget],
        targets_to_test: &[ConfiguredTarget],
        factory: &ArtifactFactory,
        action_key_context: &ActionKeyContext,
        artifact_owner: &ActionLookupKeyRef,
        workspace_name: &str,
        coverage_helper: &dyn CoverageHelper,
        html_report: Option<Artifact>,
    ) -> Option<CoverageReportActionsWrapper> {
        if targets_to_test.is_empty() {
            return None;
        }
        let mut builder: NestedSetBuilder<Artifact> = NestedSetBuilder::stable_order();
        let mut report_generator: Option<Arc<FilesToRunProvider>> = None;
        let mut action_owner: Option<ActionOwner> = None;
        for target in targets_to_test {
            // Skip incompatible tests.
            if target
                .get(
                    &crate::config::incompatible_platform_provider::INCOMPATIBLE_PLATFORM_PROVIDER
                        .key(),
                )
                .is_some()
            {
                continue;
            }
            let test_params = crate::configuredtargets::get_provider::<TestProvider>(target)
                .expect("NullPointerException: TestProvider")
                .get_test_params()
                .clone();
            builder.add_all(test_params.get_coverage_artifacts());
            // targetsToTest has non-deterministic order, so we ensure that we pick the same
            // action owner and matching report generator each time by picking the owner that's
            // lexicographically largest. We prefer an owner with exec properties set in case the
            // action is run remotely.
            // Without --collect_code_coverage the test has no CoverageParams: the generator and
            // the owner are null.
            let owner = test_params.get_action_owner_for_coverage();
            if report_generator.is_none()
                || action_owner_comparator(
                    owner.expect("NullPointerException: getActionOwnerForCoverage()"),
                    action_owner
                        .as_ref()
                        .expect("NullPointerException: actionOwner"),
                ) == Ordering::Greater
            {
                report_generator = test_params.get_coverage_report_generator().cloned();
                action_owner = owner.cloned();
            }
        }
        // If all tests are incompatible, there's nothing to do.
        let report_generator = report_generator?;
        let action_owner = action_owner.expect("NullPointerException: actionOwner");
        let baseline_coverage_artifacts = Self::get_baseline_coverage_artifacts(configured_targets);
        builder.add_transitive(&baseline_coverage_artifacts);
        let coverage_artifacts = builder.build();
        if !coverage_artifacts.is_empty() {
            let owner = ArtifactOwner::ActionLookupKey(artifact_owner.clone());
            let baseline_lcov_artifact = factory.get_derived_artifact(
                &PathFragment::create(COVERAGE_TMP_ROOT)
                    .get_relative(&PathFragment::create("baseline_lcov_files.tmp")),
                &directories.get_build_data_directory(workspace_name),
                owner.clone(),
            );
            let baseline_lcov_file_action: ActionRef =
                Arc::new(Self::generate_lcov_file_write_action(
                    baseline_lcov_artifact.clone(),
                    baseline_coverage_artifacts.clone(),
                    action_owner.clone(),
                ));
            let baseline_report_action: ActionRef =
                Arc::new(Self::generate_coverage_report_action(
                    &CoverageArgs {
                        directories,
                        coverage_artifacts: baseline_coverage_artifacts,
                        lcov_artifact: baseline_lcov_artifact,
                        factory,
                        artifact_owner: owner.clone(),
                        report_generator: report_generator.clone(),
                        workspace_name: workspace_name.to_owned(),
                        html_report: None,
                        action_owner: action_owner.clone(),
                    },
                    coverage_helper,
                    configured_targets,
                    "_baseline_report.dat",
                ));
            let coverage_lcov_artifact = factory.get_derived_artifact(
                &PathFragment::create(COVERAGE_TMP_ROOT)
                    .get_relative(&PathFragment::create("coverage_lcov_files.tmp")),
                &directories.get_build_data_directory(workspace_name),
                owner.clone(),
            );
            let coverage_lcov_file_action: ActionRef =
                Arc::new(Self::generate_lcov_file_write_action(
                    coverage_lcov_artifact.clone(),
                    coverage_artifacts.clone(),
                    action_owner.clone(),
                ));
            let coverage_report_action: ActionRef =
                Arc::new(Self::generate_coverage_report_action(
                    &CoverageArgs {
                        directories,
                        coverage_artifacts,
                        lcov_artifact: coverage_lcov_artifact,
                        factory,
                        artifact_owner: owner,
                        report_generator,
                        workspace_name: workspace_name.to_owned(),
                        html_report,
                        action_owner,
                    },
                    coverage_helper,
                    configured_targets,
                    "_coverage_report.dat",
                ));
            Some(CoverageReportActionsWrapper::new(
                baseline_report_action,
                coverage_report_action,
                vec![baseline_lcov_file_action, coverage_lcov_file_action],
                action_key_context,
            ))
        } else {
            reporter.handle(&Event::error(
                "Cannot generate coverage report - no coverage information was collected",
            ));
            None
        }
    }

    // Java: com/google/devtools/build/lib/bazel/coverage/CoverageReportActionBuilder.java:305 CoverageReportActionBuilder.getBaselineCoverageArtifacts
    fn get_baseline_coverage_artifacts(
        configured_targets: &[ConfiguredTarget],
    ) -> NestedSet<Artifact> {
        let mut baseline_coverage_artifacts = NestedSetBuilder::stable_order();
        for target in configured_targets {
            if let Some(info) = crate::configuredtargets::get_info(
                target,
                &blaze_packages::starlark_defs::builtin_providers::INSTRUMENTED_FILES_INFO.key(),
            ) {
                let provider =
                    downcast_info::<InstrumentedFilesInfo>(&info).expect("InstrumentedFilesInfo");
                baseline_coverage_artifacts
                    .add_transitive(provider.get_baseline_coverage_artifacts());
            }
        }
        baseline_coverage_artifacts.build()
    }

    // Java: com/google/devtools/build/lib/bazel/coverage/CoverageReportActionBuilder.java:317 CoverageReportActionBuilder.generateLcovFileWriteAction
    fn generate_lcov_file_write_action(
        lcov_artifact: Artifact,
        coverage_artifacts: NestedSet<Artifact>,
        action_owner: ActionOwner,
    ) -> LazyWritePathsFileAction {
        LazyWritePathsFileAction::new(
            action_owner,
            lcov_artifact,
            coverage_artifacts,
            /* files_to_ignore= */ IndexSet::new(),
            /* include_derived_artifacts= */ true,
        )
    }

    // Java: com/google/devtools/build/lib/bazel/coverage/CoverageReportActionBuilder.java:342 CoverageReportActionBuilder.generateCoverageReportAction
    fn generate_coverage_report_action(
        args: &CoverageArgs<'_>,
        coverage_helper: &dyn CoverageHelper,
        configured_targets: &[ConfiguredTarget],
        basename: &str,
    ) -> CoverageReportAction {
        let root = args
            .directories
            .get_build_data_directory(&args.workspace_name);
        let lcov_output = args.factory.get_derived_artifact(
            &PathFragment::create(COVERAGE_TMP_ROOT).get_relative(&PathFragment::create(basename)),
            &root,
            args.artifact_owner.clone(),
        );
        let report_generator_exec = args
            .report_generator
            .get_executable()
            .expect("NullPointerException: reportGenerator.getExecutable()")
            .clone();
        let runfiles_tree = args
            .report_generator
            .get_runfiles_support()
            .map(|runfiles_support| runfiles_support.get_runfiles_tree_artifact().clone());
        let action_args = coverage_helper.get_args(args, &lcov_output);

        let mut inputs_builder: NestedSetBuilder<Artifact> = NestedSetBuilder::stable_order();
        inputs_builder.add_transitive(&args.coverage_artifacts);
        inputs_builder.add(report_generator_exec);
        inputs_builder.add(args.lcov_artifact.clone());
        if let Some(runfiles_tree) = runfiles_tree {
            inputs_builder.add(runfiles_tree);
        }

        let mut outputs_builder: IndexSet<Artifact> = IndexSet::new();
        outputs_builder.insert(lcov_output.clone());
        if let Some(html_report) = &args.html_report {
            outputs_builder.insert(html_report.clone());
            inputs_builder
                .add_transitive(&coverage_helper.get_instrumented_files(args, configured_targets));
        }
        CoverageReportAction::new(
            args.action_owner.clone(),
            inputs_builder.build(),
            outputs_builder,
            action_args,
            coverage_helper.get_location_message(args, &lcov_output),
        )
    }
}
