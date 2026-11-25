//! Port of `analysis/test/CoverageReportActionFactory.java`.

use blaze_actions::actions::artifact_factory::ArtifactFactory;
use blaze_actions::{ActionKeyContext, ActionLookupKeyRef, ActionRef, Artifact};
use blaze_events::EventHandler;

use crate::analysis_services::AnalysisDirectories;
use crate::configured_target::ConfiguredTarget;
use crate::coverage_report_value::CoverageReportKey;

/// Wraps the necessary actions to get a coverage report as well as the final output artifacts.
/// The lcovWriteAction creates a file containing a set of lcov files. This file is used as an
/// input artifact for coverageReportAction. We are only interested about the output artifacts
/// from coverageReportAction.
// Java: com/google/devtools/build/lib/analysis/test/CoverageReportActionFactory.java:43 CoverageReportActionFactory.CoverageReportActionsWrapper
#[derive(Debug, Clone)]
pub struct CoverageReportActionsWrapper {
    baseline_report_action: ActionRef,
    coverage_report_action: ActionRef,
    actions: Vec<ActionRef>,
}

impl CoverageReportActionsWrapper {
    // Java: com/google/devtools/build/lib/analysis/test/CoverageReportActionFactory.java:48 CoverageReportActionsWrapper.<init>
    pub fn new(
        baseline_report_action: ActionRef,
        coverage_report_action: ActionRef,
        intermediate_actions: Vec<ActionRef>,
        action_key_context: &ActionKeyContext,
    ) -> CoverageReportActionsWrapper {
        let mut actions = vec![
            baseline_report_action.clone(),
            coverage_report_action.clone(),
        ];
        actions.extend(intermediate_actions);
        if let Err(e) = crate::rule::actions_util::assign_owners_and_throw_if_conflict(
            action_key_context,
            &actions,
            &CoverageReportKey::action_lookup_key(),
        ) {
            panic!("IllegalStateException: {e:?}");
        }
        CoverageReportActionsWrapper {
            baseline_report_action,
            coverage_report_action,
            actions,
        }
    }

    // Java: com/google/devtools/build/lib/analysis/test/CoverageReportActionFactory.java:70 CoverageReportActionsWrapper.getActions
    pub fn get_actions(&self) -> &[ActionRef] {
        &self.actions
    }

    // Java: com/google/devtools/build/lib/analysis/test/CoverageReportActionFactory.java:74 CoverageReportActionsWrapper.getCoverageOutputs
    pub fn get_coverage_outputs(&self) -> Vec<Artifact> {
        let mut outputs = self.baseline_report_action.get_outputs();
        outputs.extend(self.coverage_report_action.get_outputs());
        outputs
    }

    // Java: com/google/devtools/build/lib/analysis/test/CoverageReportActionFactory.java:78 CoverageReportActionsWrapper.getBaselineReportArtifact
    pub fn get_baseline_report_artifact(&self) -> Artifact {
        self.baseline_report_action.get_primary_output()
    }

    // Java: com/google/devtools/build/lib/analysis/test/CoverageReportActionFactory.java:82 CoverageReportActionsWrapper.getCoverageReportArtifact
    pub fn get_coverage_report_artifact(&self) -> Artifact {
        self.coverage_report_action.get_primary_output()
    }
}

/// A factory class to create coverage report actions.
///
/// PORT: Java's `eventBus` parameter is used by the Bazel implementation to register the
/// `CoverageReportCollector` (the build tool log entries of the report); the caller does that
/// with the returned wrapper (blaze_runtime's build command).
// Java: com/google/devtools/build/lib/analysis/test/CoverageReportActionFactory.java:36 CoverageReportActionFactory
pub trait CoverageReportActionFactory: Send + Sync {
    /// Returns a `CoverageReportActionsWrapper`. May return `None` if it's not necessary to
    /// create such Actions based on the input parameters and some other data available to the
    /// factory implementation, such as command line options.
    // Java: com/google/devtools/build/lib/analysis/test/CoverageReportActionFactory.java:93 CoverageReportActionFactory.createCoverageReportActionsWrapper
    #[allow(clippy::too_many_arguments)]
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
    ) -> Option<CoverageReportActionsWrapper>;
}
