//! Port of `skyframe/AnalysisProgressReceiver.java`: counts the configured targets and aspects
//! created, for the UI's `(..., N targets configured, M aspect applications)`.
//!
//! PORT: Java's receiver belongs to the server's `SkyframeBuildView` and reaches the UI through
//! `ConfigurationPhaseStartedEvent`. Blaze runs one command per process, so it is a process-wide
//! value ([`ANALYSIS_PROGRESS`]); the build command resets it where Java does.

use std::sync::atomic::{AtomicI32, Ordering};

/// Receiver for the analysis progress.
// Java: com/google/devtools/build/lib/skyframe/AnalysisProgressReceiver.java:24 AnalysisProgressReceiver
pub struct AnalysisProgressReceiver {
    configured_targets_completed: AtomicI32,
    configured_targets_downloaded: AtomicI32,
    configured_aspects_completed: AtomicI32,
    configured_aspects_downloaded: AtomicI32,
}

/// The process's receiver.
pub static ANALYSIS_PROGRESS: AnalysisProgressReceiver = AnalysisProgressReceiver::new();

impl AnalysisProgressReceiver {
    pub const fn new() -> Self {
        AnalysisProgressReceiver {
            configured_targets_completed: AtomicI32::new(0),
            configured_targets_downloaded: AtomicI32::new(0),
            configured_aspects_completed: AtomicI32::new(0),
            configured_aspects_downloaded: AtomicI32::new(0),
        }
    }

    /// Register that a target has been configured.
    // Java: com/google/devtools/build/lib/skyframe/AnalysisProgressReceiver.java:32 AnalysisProgressReceiver.doneConfigureTarget
    pub fn done_configure_target(&self) {
        self.configured_targets_completed
            .fetch_add(1, Ordering::SeqCst);
    }

    /// Register that a configured target has been downloaded from a remote cache.
    // Java: com/google/devtools/build/lib/skyframe/AnalysisProgressReceiver.java:37 AnalysisProgressReceiver.doneDownloadedConfiguredTarget
    pub fn done_downloaded_configured_target(&self) {
        self.configured_targets_completed
            .fetch_add(1, Ordering::SeqCst);
        self.configured_targets_downloaded
            .fetch_add(1, Ordering::SeqCst);
    }

    /// Register that a aspect has been configured.
    // Java: com/google/devtools/build/lib/skyframe/AnalysisProgressReceiver.java:43 AnalysisProgressReceiver.doneConfigureAspect
    pub fn done_configure_aspect(&self) {
        self.configured_aspects_completed
            .fetch_add(1, Ordering::SeqCst);
    }

    /// Register that a configured aspect has been downloaded from a remote cache.
    // Java: com/google/devtools/build/lib/skyframe/AnalysisProgressReceiver.java:48 AnalysisProgressReceiver.doneDownloadedConfiguredAspect
    pub fn done_downloaded_configured_aspect(&self) {
        self.configured_aspects_completed
            .fetch_add(1, Ordering::SeqCst);
        self.configured_aspects_downloaded
            .fetch_add(1, Ordering::SeqCst);
    }

    /// Reset all instance variables of this object to a state equal to that of a newly
    /// constructed object.
    // Java: com/google/devtools/build/lib/skyframe/AnalysisProgressReceiver.java:57 AnalysisProgressReceiver.reset
    pub fn reset(&self) {
        self.configured_targets_completed.store(0, Ordering::SeqCst);
        self.configured_targets_downloaded
            .store(0, Ordering::SeqCst);
        self.configured_aspects_completed.store(0, Ordering::SeqCst);
        self.configured_aspects_downloaded
            .store(0, Ordering::SeqCst);
    }

    /// Return a snapshot of the configuration progress as human-readable description of the
    /// number of targets and aspects configured so far.
    // Java: com/google/devtools/build/lib/skyframe/AnalysisProgressReceiver.java:67 AnalysisProgressReceiver.getProgressString
    pub fn get_progress_string(&self) -> String {
        let mut sb = String::new();

        let targets = self.configured_targets_completed.load(Ordering::SeqCst);
        sb.push_str(&targets.to_string());
        sb.push_str(if targets == 1 { " target" } else { " targets" });
        sb.push_str(" configured");

        let downloaded_targets = self.configured_targets_downloaded.load(Ordering::SeqCst);
        if downloaded_targets > 0 {
            sb.push_str(&format!(" ({} remote cache hits)", downloaded_targets));
        }

        let aspects = self.configured_aspects_completed.load(Ordering::SeqCst);
        if aspects > 0 {
            sb.push_str(", ");
            sb.push_str(&aspects.to_string());
            sb.push_str(if aspects == 1 {
                " aspect application"
            } else {
                " aspect applications"
            });
            let downloaded_aspects = self.configured_aspects_downloaded.load(Ordering::SeqCst);
            if downloaded_aspects > 0 {
                sb.push_str(&format!(" ({} remote cache hits)", downloaded_aspects));
            }
        }

        sb
    }
}

/// Port of `skyframe/AnalysisProgressReceiverTest.java`.
#[cfg(test)]
mod tests {
    use super::AnalysisProgressReceiver;

    // Java: com/google/devtools/build/lib/skyframe/AnalysisProgressReceiverTest.java:28 AnalysisProgressReceiverTest.testTargetCounted
    #[test]
    fn test_target_counted() {
        // If the configuration of a target is completed it is counted as fully configured
        // target.
        let progress = AnalysisProgressReceiver::new();
        progress.done_configure_target();
        let progress_string1 = progress.get_progress_string();
        assert!(
            progress_string1.contains("1 target configured"),
            "One configured target should be visible in progress."
        );

        progress.done_configure_target();
        let progress_string2 = progress.get_progress_string();
        assert!(
            progress_string2.contains("2 targets configured"),
            "Two configured targets should be visible in progress."
        );
    }

    // Java: com/google/devtools/build/lib/skyframe/AnalysisProgressReceiverTest.java:47 AnalysisProgressReceiverTest.testDownloadedTargetCounted
    #[test]
    fn test_downloaded_target_counted() {
        let progress = AnalysisProgressReceiver::new();
        progress.done_downloaded_configured_target();
        assert!(
            progress
                .get_progress_string()
                .contains("1 target configured (1 remote cache hits)")
        );

        progress.done_configure_target();
        assert!(
            progress
                .get_progress_string()
                .contains("2 targets configured (1 remote cache hits)")
        );
    }

    // Java: com/google/devtools/build/lib/skyframe/AnalysisProgressReceiverTest.java:61 AnalysisProgressReceiverTest.testAspectCounted
    #[test]
    fn test_aspect_counted() {
        let progress = AnalysisProgressReceiver::new();
        progress.done_configure_aspect();
        assert!(
            progress
                .get_progress_string()
                .contains("0 targets configured, 1 aspect application"),
            "One configured aspect should be visible in progress."
        );

        progress.done_configure_aspect();
        assert!(
            progress
                .get_progress_string()
                .contains("0 targets configured, 2 aspect applications"),
            "Two configured aspects should be visible in progress."
        );
    }

    // Java: com/google/devtools/build/lib/skyframe/AnalysisProgressReceiverTest.java:79 AnalysisProgressReceiverTest.testDownloadedAspectCounted
    #[test]
    fn test_downloaded_aspect_counted() {
        let progress = AnalysisProgressReceiver::new();
        progress.done_downloaded_configured_aspect();
        assert!(
            progress
                .get_progress_string()
                .contains("0 targets configured, 1 aspect application (1 remote cache hits)")
        );

        progress.done_configure_aspect();
        assert!(
            progress
                .get_progress_string()
                .contains("0 targets configured, 2 aspect applications (1 remote cache hits)")
        );
    }

    // Java: com/google/devtools/build/lib/skyframe/AnalysisProgressReceiverTest.java:95 AnalysisProgressReceiverTest.testTargetAndAspectCounted
    #[test]
    fn test_target_and_aspect_counted() {
        let progress = AnalysisProgressReceiver::new();
        assert!(
            progress
                .get_progress_string()
                .contains("0 targets configured")
        );

        progress.done_configure_target();
        assert!(
            progress
                .get_progress_string()
                .contains("1 target configured")
        );

        progress.done_configure_aspect();
        assert!(
            progress
                .get_progress_string()
                .contains("1 target configured, 1 aspect application")
        );
    }

    // Java: com/google/devtools/build/lib/skyframe/AnalysisProgressReceiverTest.java:112 AnalysisProgressReceiverTest.testReset
    #[test]
    fn test_reset() {
        // After resetting, messages should be as immediately after creation.
        let progress = AnalysisProgressReceiver::new();
        let default_progress = progress.get_progress_string();
        progress.done_configure_target();
        assert_ne!(progress.get_progress_string(), default_progress);
        progress.reset();
        assert_eq!(progress.get_progress_string(), default_progress);
    }
}
