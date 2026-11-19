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
use indexmap::IndexSet;
use parking_lot::Mutex;

/// Keeps track of the loading progress.
// Java: com/google/devtools/build/lib/skyframe/PackageProgressReceiver.java:24 PackageProgressReceiver
pub struct PackageProgressReceiver {
    state: Mutex<State>,
}

struct State {
    packages_completed: i32,
    /// `pendingSet`; `None` stands for an empty set (a `const` initializer cannot allocate).
    pending_set: Option<IndexSet<PackageIdentifier>>,
    /// The packages counted in `packages_completed` (see the module comment).
    completed_set: Option<HashSet<PackageIdentifier>>,
}

/// The process's receiver.
pub static PACKAGE_PROGRESS: PackageProgressReceiver = PackageProgressReceiver::new();

impl PackageProgressReceiver {
    pub const fn new() -> PackageProgressReceiver {
        PackageProgressReceiver {
            state: Mutex::new(State {
                packages_completed: 0,
                pending_set: None,
                completed_set: None,
            }),
        }
    }

    /// Register that loading a package has started.
    // Java: com/google/devtools/build/lib/skyframe/PackageProgressReceiver.java:30 PackageProgressReceiver.startReadPackage
    pub fn start_read_package(&self, package_id: &PackageIdentifier) {
        self.state
            .lock()
            .pending_set
            .get_or_insert_with(IndexSet::new)
            .insert(package_id.clone());
    }

    /// Register that loading a package has completed.
    // Java: com/google/devtools/build/lib/skyframe/PackageProgressReceiver.java:35 PackageProgressReceiver.doneReadPackage
    pub fn done_read_package(&self, package_id: &PackageIdentifier) {
        let mut state = self.state.lock();
        if let Some(pending_set) = state.pending_set.as_mut() {
            pending_set.shift_remove(package_id);
        }
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
        state.pending_set = None;
        state.completed_set = None;
    }

    /// Return the ordered pair of a consistent snapshot of the state, consisting of a
    /// human-readable description of the progress achieved so far and a human readable
    /// description of the currently running activities. The later always include the oldest
    /// loading package not finished loading.
    // Java: com/google/devtools/build/lib/skyframe/PackageProgressReceiver.java:55 PackageProgressReceiver.progressState
    pub fn progress_state(&self) -> (String, String) {
        let state = self.state.lock();
        let progress = format!("{} packages loaded", state.packages_completed);
        let mut activity = String::new();
        if let Some(pending_set) = state.pending_set.as_ref()
            && !pending_set.is_empty()
        {
            activity.push_str("currently loading: ");
            activity.push_str(&pending_set.first().expect("non-empty").to_string());
            if pending_set.len() > 1 {
                activity.push_str(&format!(" ... ({} packages)", pending_set.len()));
            }
        }
        (progress, activity)
    }
}

/// This event is fired before the loading phase starts.
///
/// PORT: the receiver is the process's [`PACKAGE_PROGRESS`] (module comment).
// Java: com/google/devtools/build/lib/skyframe/LoadingPhaseStartedEvent.java:21 LoadingPhaseStartedEvent
#[derive(Debug)]
pub struct LoadingPhaseStartedEvent {
    package_progress: &'static PackageProgressReceiver,
}

impl LoadingPhaseStartedEvent {
    // Java: com/google/devtools/build/lib/skyframe/LoadingPhaseStartedEvent.java:30 LoadingPhaseStartedEvent.<init>
    pub fn new(package_progress: &'static PackageProgressReceiver) -> LoadingPhaseStartedEvent {
        LoadingPhaseStartedEvent { package_progress }
    }

    // Java: com/google/devtools/build/lib/skyframe/LoadingPhaseStartedEvent.java:34 LoadingPhaseStartedEvent.getPackageProgressReceiver
    pub fn get_package_progress_receiver(&self) -> &'static PackageProgressReceiver {
        self.package_progress
    }
}

impl blaze_events::Postable for LoadingPhaseStartedEvent {}

impl std::fmt::Debug for PackageProgressReceiver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PackageProgressReceiver")
    }
}

#[cfg(test)]
mod tests {
    use super::PackageProgressReceiver;
    use blaze_cmdline::PackageIdentifier;

    // Java: com/google/devtools/build/lib/skyframe/PackageProgressReceiverTest.java:31 PackageProgressReceiverTest.testPackageVisible
    #[test]
    fn test_package_visible() {
        // If there is only a single package being loaded, it is visible in
        // the activity part of the progress state.
        let id = PackageIdentifier::create_in_main_repo_str("foo/bar/baz");
        let progress = PackageProgressReceiver::new();
        progress.start_read_package(&id);
        let activity = progress.progress_state().1;

        assert!(
            activity.contains(&id.to_string()),
            "Unfinished package '{id}' should be visible in activity: {activity}"
        );
    }

    // Java: com/google/devtools/build/lib/skyframe/PackageProgressReceiverTest.java:45 PackageProgressReceiverTest.testPackageCounted
    #[test]
    fn test_package_counted() {
        // If the loading of a package is completed, it is no longer visible as activity,
        // but counted as one package fully loaded.
        let id = PackageIdentifier::create_in_main_repo_str("foo/bar/baz");
        let progress = PackageProgressReceiver::new();
        progress.start_read_package(&id);
        progress.done_read_package(&id);
        let state = progress.progress_state().0;
        let activity = progress.progress_state().1;

        assert!(
            !activity.contains(&id.to_string()),
            "Finished package '{id}' should not be visible in activity: {activity}"
        );
        assert!(
            state.contains("1 package"),
            "Number of completed packages should be visible in state"
        );
    }

    // Java: com/google/devtools/build/lib/skyframe/PackageProgressReceiverTest.java:65 PackageProgressReceiverTest.testReset
    #[test]
    fn test_reset() {
        // After resetting, messages should be as immediately after creation.
        let progress = PackageProgressReceiver::new();
        let default_state = progress.progress_state().0;
        let default_activity = progress.progress_state().1;
        let id = PackageIdentifier::create_in_main_repo_str("foo/bar/baz");
        progress.start_read_package(&id);
        progress.done_read_package(&id);
        progress.reset();
        assert_eq!(progress.progress_state().0, default_state);
        assert_eq!(progress.progress_state().1, default_activity);
    }
}
