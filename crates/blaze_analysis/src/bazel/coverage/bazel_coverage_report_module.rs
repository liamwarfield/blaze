//! Port of `bazel/coverage/BazelCoverageReportModule.java`: the coverage report factory and the
//! Bazel coverage helper. The module's options are transcribed in blaze_optdefs
//! (`misc::bazel_coverage_report_module`); the module itself (`getCommandOptions`) is
//! blaze_runtime's.

use blaze_actions::actions::artifact_factory::ArtifactFactory;
use blaze_actions::{ActionKeyContext, ActionLookupKeyRef, Artifact};
use blaze_events::EventHandler;
use blaze_optdefs::misc::bazel_coverage_report_module::{
    BazelCoverageReportModuleOptions, ReportType,
};

use super::coverage_args::CoverageArgs;
use super::coverage_report_action_builder::{CoverageHelper, CoverageReportActionBuilder};
use crate::analysis_services::AnalysisDirectories;
use crate::configured_target::ConfiguredTarget;
use crate::test::coverage_report_action_factory::{
    CoverageReportActionFactory, CoverageReportActionsWrapper,
};

/// The factory `BazelCoverageReportModule.getCoverageReportFactory` returns.
// Java: com/google/devtools/build/lib/bazel/coverage/BazelCoverageReportModule.java:78 BazelCoverageReportModule.getCoverageReportFactory
pub struct BazelCoverageReportActionFactory {
    /// `commandOptions.getOptions(Options.class)` (null for a command without the options).
    options: Option<BazelCoverageReportModuleOptions>,
}

/// `BazelCoverageReportModule.getCoverageReportFactory(commandOptions)`.
// Java: com/google/devtools/build/lib/bazel/coverage/BazelCoverageReportModule.java:78 BazelCoverageReportModule.getCoverageReportFactory
pub fn get_coverage_report_factory(
    options: Option<BazelCoverageReportModuleOptions>,
) -> BazelCoverageReportActionFactory {
    BazelCoverageReportActionFactory { options }
}

impl CoverageReportActionFactory for BazelCoverageReportActionFactory {
    // Java: com/google/devtools/build/lib/bazel/coverage/BazelCoverageReportModule.java:83 BazelCoverageReportModule.getCoverageReportFactory.createCoverageReportActionsWrapper
    fn create_coverage_report_actions_wrapper(
        &self,
        event_handler: &dyn EventHandler,
        directories: &AnalysisDirectories,
        configured_targets: &[ConfiguredTarget],
        targets_to_test: &[ConfiguredTarget],
        artifact_factory: &ArtifactFactory,
        action_key_context: &ActionKeyContext,
        action_lookup_key: &ActionLookupKeyRef,
        workspace_name: &str,
    ) -> Option<CoverageReportActionsWrapper> {
        let options = self.options.as_ref()?;
        if options.combined_report == ReportType::NONE {
            return None;
        }
        blaze_util::check_argument!(options.combined_report == ReportType::LCOV);
        let builder = CoverageReportActionBuilder::new();
        // PORT: `eventBus.register(new CoverageReportCollector(wrapper))` is the caller's
        // (see `CoverageReportActionFactory`).
        builder.create_coverage_actions_wrapper(
            event_handler,
            directories,
            configured_targets,
            targets_to_test,
            artifact_factory,
            action_key_context,
            action_lookup_key,
            workspace_name,
            &BazelCoverageHelper,
            /* html_report= */ None,
        )
    }
}

// Java: com/google/devtools/build/lib/bazel/coverage/BazelCoverageReportModule.java:120 BazelCoverageReportModule.BazelCoverageHelper
struct BazelCoverageHelper;

impl CoverageHelper for BazelCoverageHelper {
    // Java: com/google/devtools/build/lib/bazel/coverage/BazelCoverageReportModule.java:122 BazelCoverageHelper.getArgs
    fn get_args(&self, args: &CoverageArgs<'_>, lcov_output: &Artifact) -> Vec<String> {
        vec![
            args.report_generator
                .get_executable()
                .expect("NullPointerException: reportGenerator.getExecutable()")
                .get_exec_path_string()
                .to_owned(),
            // A file that contains all the exec paths to the coverage artifacts
            format!(
                "--reports_file={}",
                args.lcov_artifact.get_exec_path_string()
            ),
            format!("--output_file={}", lcov_output.get_exec_path_string()),
        ]
    }

    // Java: com/google/devtools/build/lib/bazel/coverage/BazelCoverageReportModule.java:134 BazelCoverageHelper.getLocationMessage
    fn get_location_message(&self, _args: &CoverageArgs<'_>, lcov_output: &Artifact) -> String {
        format!(
            "LCOV coverage report is located at {}\n and execpath is {}",
            lcov_output.get_path().get_path_string(),
            lcov_output.get_exec_path_string()
        )
    }
}

/// The build tool log entries of `CoverageReportCollector.buildComplete`: the coverage report
/// and the baseline report, as (name, path) of local files.
// Java: com/google/devtools/build/lib/bazel/coverage/BazelCoverageReportModule.java:144 CoverageReportCollector.buildComplete
pub fn coverage_report_build_tool_log_files(
    wrapper: &CoverageReportActionsWrapper,
) -> Vec<(&'static str, blaze_vfs::Path)> {
    vec![
        (
            "coverage_report.lcov",
            wrapper.get_coverage_report_artifact().get_path(),
        ),
        (
            "baseline_report.lcov",
            wrapper.get_baseline_report_artifact().get_path(),
        ),
    ]
}
