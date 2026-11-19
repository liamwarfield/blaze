//! Port of `skyframe/PackageProgressReceiver.java`: counts the packages whose loading completed,
//! for the UI's `(N packages loaded, ...)`.
//!
//! PORT: Java's receiver is a field of the server's `SkyframeExecutor`, handed to
//! `PackageFunction` and the UI. Blaze runs one command per process, so the receiver is a
//! process-wide value ([`PACKAGE_PROGRESS`]); its initial state stands for Java's reset in
//! `SkyframeExecutor.preparePackageLoading`, which runs once per command, before
//! `BlazeOptionHandler.parseStarlarkOptions` (`BlazeCommandDispatcher.java:651` and `:768`).
//!
//! PORT: Java loads every package of a command once, in the server's single graph, so its count
//! of completed loads is a count of distinct packages. Blaze loads the build settings of the
//! command line in a separate evaluator (blaze_runtime `dispatcher.rs`, `parse_starlark_options`)
//! and the command's own evaluator loads most of them again; the receiver therefore counts each
//! package identifier once until the next [`PackageProgressReceiver::reset`].

use std::collections::HashSet;

use blaze_cmdline::PackageIdentifier;
use parking_lot::Mutex;

/// Keeps track of the loading progress.
// Java: com/google/devtools/build/lib/skyframe/PackageProgressReceiver.java:24 PackageProgressReceiver
pub struct PackageProgressReceiver {
    state: Mutex<State>,
}

struct State {
    packages_completed: i32,
    /// The packages counted in `packages_completed` (see the module comment).
    completed_set: Option<HashSet<PackageIdentifier>>,
}

/// The process's receiver.
pub static PACKAGE_PROGRESS: PackageProgressReceiver = PackageProgressReceiver {
    state: Mutex::new(State {
        packages_completed: 0,
        completed_set: None,
    }),
};

impl PackageProgressReceiver {
    /// Register that loading a package has completed.
    // Java: com/google/devtools/build/lib/skyframe/PackageProgressReceiver.java:36 PackageProgressReceiver.doneReadPackage
    pub fn done_read_package(&self, package_id: &PackageIdentifier) {
        // PORT: the pending set only feeds the "currently loading" progress activity.
        let mut state = self.state.lock();
        // PORT: a package loaded again by another evaluator of the same command is not counted
        // again (module comment).
        if !state
            .completed_set
            .get_or_insert_with(HashSet::new)
            .insert(package_id.clone())
        {
            return;
        }
        state.packages_completed += 1;
    }

    /// Reset all instance variables of this object to a state equal to that of a newly
    /// constructed object.
    // Java: com/google/devtools/build/lib/skyframe/PackageProgressReceiver.java:46 PackageProgressReceiver.reset
    pub fn reset(&self) {
        let mut state = self.state.lock();
        state.packages_completed = 0;
        state.completed_set = None;
    }

    /// The human-readable progress (`N packages loaded`) of `progressState()`.
    // Java: com/google/devtools/build/lib/skyframe/PackageProgressReceiver.java:55 PackageProgressReceiver.progressState
    pub fn progress_state(&self) -> String {
        format!("{} packages loaded", self.state.lock().packages_completed)
    }
}
