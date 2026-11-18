//! Port of `skyframe/PackageProgressReceiver.java`: counts the packages whose loading completed,
//! for the UI's `(N packages loaded, ...)`.
//!
//! PORT: Java's receiver is a field of the server's `SkyframeExecutor`, handed to
//! `PackageFunction` and the UI. Blaze runs one command per process, so the receiver is a
//! process-wide value ([`PACKAGE_PROGRESS`]); the build command resets it where Java does.

use std::sync::atomic::{AtomicI32, Ordering};

use blaze_cmdline::PackageIdentifier;

/// Keeps track of the loading progress.
// Java: com/google/devtools/build/lib/skyframe/PackageProgressReceiver.java:24 PackageProgressReceiver
pub struct PackageProgressReceiver {
    packages_completed: AtomicI32,
}

/// The process's receiver.
pub static PACKAGE_PROGRESS: PackageProgressReceiver = PackageProgressReceiver {
    packages_completed: AtomicI32::new(0),
};

impl PackageProgressReceiver {
    /// Register that loading a package has completed.
    // Java: com/google/devtools/build/lib/skyframe/PackageProgressReceiver.java:36 PackageProgressReceiver.doneReadPackage
    pub fn done_read_package(&self, _package_id: &PackageIdentifier) {
        // PORT: the pending set only feeds the "currently loading" progress activity.
        self.packages_completed.fetch_add(1, Ordering::SeqCst);
    }

    /// Reset all instance variables of this object to a state equal to that of a newly
    /// constructed object.
    // Java: com/google/devtools/build/lib/skyframe/PackageProgressReceiver.java:46 PackageProgressReceiver.reset
    pub fn reset(&self) {
        self.packages_completed.store(0, Ordering::SeqCst);
    }

    /// The human-readable progress (`N packages loaded`) of `progressState()`.
    // Java: com/google/devtools/build/lib/skyframe/PackageProgressReceiver.java:55 PackageProgressReceiver.progressState
    pub fn progress_state(&self) -> String {
        format!(
            "{} packages loaded",
            self.packages_completed.load(Ordering::SeqCst)
        )
    }
}
