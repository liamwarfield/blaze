//! Port of `SkyframeExecutor.SkyframeProgressReceiver`: the executor's
//! `EvaluationProgressReceiver`, which forwards to the execution progress receiver of the build
//! (`ExecutionProgressReceiver`) once one is set.

use std::sync::Arc;

use blaze_skyframe::{ErrorInfo, EvaluationProgressReceiver, EvaluationState, GroupedDeps};
use blaze_skyframe::{SkyKey, SkyValue};
use parking_lot::RwLock;

/// The executor's progress receiver.
///
/// PORT: Java also forwards `dirtied`, `deleted`, `enqueueing` and `evaluated` to the build
/// view's progress receiver (`SkyframeBuildView.getProgressReceiver()`, which counts configured
/// targets and actions for incremental analysis), tells the `SkyKeyStateReceiver` about
/// computations, drops nodes for `--heuristically_drop_nodes` and interns labels; none of those
/// is ported (Blaze counts configured targets in `ConfiguredTargetFunction`, see
/// `blaze_sky_analysis::analysis_progress_receiver`). `ignoreInvalidations` belongs to
/// `--discard_analysis_cache` on a server and is not ported.
// Java: com/google/devtools/build/lib/skyframe/SkyframeExecutor.java:3493 SkyframeExecutor.SkyframeProgressReceiver
#[derive(Default)]
pub struct SkyframeProgressReceiver {
    /// This receiver is only needed for execution, so it is null otherwise.
    execution_progress_receiver: RwLock<Option<Arc<dyn EvaluationProgressReceiver>>>,
}

impl SkyframeProgressReceiver {
    pub fn new() -> SkyframeProgressReceiver {
        SkyframeProgressReceiver::default()
    }

    /// `executionProgressReceiver = ...` (`SkyframeExecutor.setExecutionProgressReceiver`).
    // Java: com/google/devtools/build/lib/skyframe/SkyframeExecutor.java:1973 SkyframeExecutor.setExecutionProgressReceiver
    pub fn set_execution_progress_receiver(
        &self,
        execution_progress_receiver: Option<Arc<dyn EvaluationProgressReceiver>>,
    ) {
        *self.execution_progress_receiver.write() = execution_progress_receiver;
    }

    fn execution_progress_receiver(&self) -> Option<Arc<dyn EvaluationProgressReceiver>> {
        self.execution_progress_receiver.read().clone()
    }
}

impl EvaluationProgressReceiver for SkyframeProgressReceiver {
    // Java: com/google/devtools/build/lib/skyframe/SkyframeExecutor.java:3521 SkyframeProgressReceiver.enqueueing
    fn enqueueing(&self, sky_key: &SkyKey) {
        if let Some(execution_progress_receiver) = self.execution_progress_receiver() {
            execution_progress_receiver.enqueueing(sky_key);
        }
    }

    // Java: com/google/devtools/build/lib/skyframe/SkyframeExecutor.java:3546 SkyframeProgressReceiver.evaluated
    fn evaluated(
        &self,
        sky_key: &SkyKey,
        state: EvaluationState,
        new_value: Option<&SkyValue>,
        new_error: Option<&ErrorInfo>,
        direct_deps: Option<&GroupedDeps>,
    ) {
        if let Some(execution_progress_receiver) = self.execution_progress_receiver() {
            execution_progress_receiver.evaluated(sky_key, state, new_value, new_error, direct_deps);
        }
    }
}

/// The process's `PackageProgressReceiver` as the UI reads it (`LoadingPhaseStartedEvent`).
///
/// PORT: the receiver lives in `blaze_sky_loading` and the UI's view of it in
/// `blaze_buildtool`; this crate joins the two.
pub struct PackageProgressReceiverView(
    pub &'static blaze_sky_loading::package_progress_receiver::PackageProgressReceiver,
);

impl blaze_buildtool::runtime::ui_state_tracker::PackageProgress for PackageProgressReceiverView {
    // Java: com/google/devtools/build/lib/skyframe/PackageProgressReceiver.java:55 PackageProgressReceiver.progressState
    fn progress_state(&self) -> (String, String) {
        self.0.progress_state()
    }
}

/// The process's `AnalysisProgressReceiver` as the UI reads it
/// (`ConfigurationPhaseStartedEvent`).
pub struct AnalysisProgressReceiverView(
    pub &'static blaze_sky_analysis::analysis_progress_receiver::AnalysisProgressReceiver,
);

impl blaze_buildtool::runtime::ui_state_tracker::AnalysisProgress
    for AnalysisProgressReceiverView
{
    // Java: com/google/devtools/build/lib/skyframe/AnalysisProgressReceiver.java:67 AnalysisProgressReceiver.getProgressString
    fn get_progress_string(&self) -> String {
        self.0.get_progress_string()
    }
}
