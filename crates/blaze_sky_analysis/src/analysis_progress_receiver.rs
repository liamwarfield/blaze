//! Port of `skyframe/AnalysisProgressReceiver.java`: counts the configured targets created, for
//! the UI's `(..., N targets configured)`.
//!
//! PORT: Java's receiver belongs to the server's `SkyframeBuildView` and reaches the UI through
//! `ConfigurationPhaseStartedEvent`. Blaze runs one command per process, so it is a process-wide
//! value ([`ANALYSIS_PROGRESS`]); the build command resets it where Java does.

use std::sync::atomic::{AtomicI32, Ordering};

/// Receiver for the analysis progress.
// Java: com/google/devtools/build/lib/skyframe/AnalysisProgressReceiver.java:24 AnalysisProgressReceiver
pub struct AnalysisProgressReceiver {
    configured_targets_completed: AtomicI32,
}

/// The process's receiver.
pub static ANALYSIS_PROGRESS: AnalysisProgressReceiver = AnalysisProgressReceiver {
    configured_targets_completed: AtomicI32::new(0),
};

impl AnalysisProgressReceiver {
    // Java: com/google/devtools/build/lib/skyframe/AnalysisProgressReceiver.java:32 AnalysisProgressReceiver.doneConfigureTarget
    pub fn done_configure_target(&self) {
        self.configured_targets_completed
            .fetch_add(1, Ordering::SeqCst);
    }

    // Java: com/google/devtools/build/lib/skyframe/AnalysisProgressReceiver.java:57 AnalysisProgressReceiver.reset
    pub fn reset(&self) {
        self.configured_targets_completed.store(0, Ordering::SeqCst);
    }

    /// PORT: aspects and remote-cache downloads are not counted (no top-level aspects or remote
    /// analysis caching are ported), so their parts of the string never appear.
    // Java: com/google/devtools/build/lib/skyframe/AnalysisProgressReceiver.java:67 AnalysisProgressReceiver.getProgressString
    pub fn get_progress_string(&self) -> String {
        let targets = self.configured_targets_completed.load(Ordering::SeqCst);
        format!(
            "{}{} configured",
            targets,
            if targets == 1 { " target" } else { " targets" }
        )
    }
}
