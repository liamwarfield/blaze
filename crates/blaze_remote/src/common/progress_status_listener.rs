//! Port of `lib/remote/common/ProgressStatusListener.java`, with the `SpawnProgressEvent` it
//! receives (`lib/exec/SpawnProgressEvent.java`).

use std::sync::Arc;

use blaze_events::ExtendedEventHandler;
use blaze_exec::actions::ActionExecutionMetadata;

/// The `SpawnRunner` is making some progress.
///
/// PORT: `blaze_exec::exec::ProgressStatus` has no variant for this event and `blaze_exec` has no
/// `SpawnProgressEvent`, so the record lives here with its Java fields.
// Java: com/google/devtools/build/lib/exec/SpawnProgressEvent.java:32 SpawnProgressEvent
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SpawnProgressEvent {
    /// The id that uniquely determines the progress among all progress events for this spawn.
    pub progress_id: String,
    /// Human readable description of the progress.
    pub progress: String,
    /// Whether the progress reported about is finished already.
    pub finished: bool,
}

impl SpawnProgressEvent {
    /// `SpawnProgressEvent.create(resourceId, progress, finished)`.
    // Java: com/google/devtools/build/lib/exec/SpawnProgressEvent.java:39 SpawnProgressEvent.create
    pub fn create(resource_id: &str, progress: &str, finished: bool) -> SpawnProgressEvent {
        SpawnProgressEvent {
            progress_id: resource_id.to_string(),
            progress: progress.to_string(),
            finished,
        }
    }

    /// `progressId()`.
    pub fn progress_id(&self) -> &str {
        &self.progress_id
    }

    /// `progress()`.
    pub fn progress(&self) -> &str {
        &self.progress
    }

    /// `finished()`.
    pub fn finished(&self) -> bool {
        self.finished
    }

    /// `postTo(eventHandler, action)`: posts an `ActionProgressEvent`.
    // Java: com/google/devtools/build/lib/exec/SpawnProgressEvent.java:44 SpawnProgressEvent.postTo
    pub fn post_to(
        &self,
        _event_handler: &dyn ExtendedEventHandler,
        _action: &dyn ActionExecutionMetadata,
    ) {
        blaze_util::unported!(
            "SpawnProgressEvent.postTo (missing hook: ActionProgressEvent in blaze_exec)"
        )
    }
}

/// An interface that is used to receive `ProgressStatus` updates during spawn execution.
///
/// Java's `@FunctionalInterface`: every `Fn(SpawnProgressEvent) + Send + Sync` is a listener.
// Java: com/google/devtools/build/lib/remote/common/ProgressStatusListener.java:21 ProgressStatusListener
pub trait ProgressStatusListener: Send + Sync {
    /// `onProgressStatus(progress)`.
    fn on_progress_status(&self, progress: SpawnProgressEvent);
}

impl<F> ProgressStatusListener for F
where
    F: Fn(SpawnProgressEvent) + Send + Sync,
{
    fn on_progress_status(&self, progress: SpawnProgressEvent) {
        self(progress)
    }
}

/// A `ProgressStatusListener` that does nothing (`ProgressStatusListener.NO_ACTION`).
// Java: com/google/devtools/build/lib/remote/common/ProgressStatusListener.java:26 ProgressStatusListener.NO_ACTION
pub fn no_action() -> Arc<dyn ProgressStatusListener> {
    Arc::new(|_progress: SpawnProgressEvent| {
        // Intentionally left empty
    })
}
