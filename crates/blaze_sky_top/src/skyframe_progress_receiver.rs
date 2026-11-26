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
    /// The counts of `SkyframeBuildView`'s `ActionLookupValueProgressReceiver` (and of the
    /// packages loaded), read when the evaluator works over a graph kept across commands.
    evaluated_counts: parking_lot::Mutex<crate::skyframe_build_view::EvaluatedCounts>,
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

    /// The counts of the nodes evaluated so far (`SkyframeBuildView.getEvaluatedCounts`).
    pub fn evaluated_counts(&self) -> crate::skyframe_build_view::EvaluatedCounts {
        self.evaluated_counts.lock().clone()
    }

    /// `SkyframeBuildView.resetProgressReceiver`.
    // Java: com/google/devtools/build/lib/skyframe/SkyframeBuildView.java:193 SkyframeBuildView.resetProgressReceiver
    pub fn reset_evaluated_counts(&self) {
        self.evaluated_counts.lock().reset();
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
        // skyframeBuildView.getProgressReceiver().evaluated(...)
        // Java: com/google/devtools/build/lib/skyframe/SkyframeBuildView.java:1481 ActionLookupValueProgressReceiver.evaluated
        if state.version_changed()
            && state.succeeded()
            && let Some(new_value) = new_value
        {
            self.evaluated_counts.lock().accumulate(sky_key, new_value);
        }
        if let Some(execution_progress_receiver) = self.execution_progress_receiver() {
            execution_progress_receiver.evaluated(
                sky_key,
                state,
                new_value,
                new_error,
                direct_deps,
            );
        }
    }
}

/// The process's `AnalysisProgressReceiver` as the UI reads it
/// (`ConfigurationPhaseStartedEvent`).
pub struct AnalysisProgressReceiverView(
    pub &'static blaze_sky_analysis::analysis_progress_receiver::AnalysisProgressReceiver,
);

impl blaze_buildtool::runtime::ui_state_tracker::AnalysisProgress for AnalysisProgressReceiverView {
    // Java: com/google/devtools/build/lib/skyframe/AnalysisProgressReceiver.java:67 AnalysisProgressReceiver.getProgressString
    fn get_progress_string(&self) -> String {
        self.0.get_progress_string()
    }
}
