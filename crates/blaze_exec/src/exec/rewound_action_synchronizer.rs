//! Port of `OutputService.RewoundActionSynchronizer` (`lib/vfs/OutputService.java`): how the
//! action executor serialises rewound actions (which delete and recreate their outputs) against
//! the actions that read those outputs.

use crate::actions::InputMetadataProvider;
use crate::actions::action_cache_checker::InterruptedException;
use blaze_actions::actions::action::Action;

/// Released when dropped (`SilentCloseable.close()`).
// Java: com/google/devtools/build/lib/profiler/SilentCloseable.java:19 SilentCloseable
pub struct SilentCloseable(Option<Box<dyn FnOnce() + Send>>);

impl SilentCloseable {
    /// `() -> {}`.
    pub fn noop() -> SilentCloseable {
        SilentCloseable(None)
    }

    /// Runs `close` when dropped.
    pub fn new(close: impl FnOnce() + Send + 'static) -> SilentCloseable {
        SilentCloseable(Some(Box::new(close)))
    }
}

impl Drop for SilentCloseable {
    fn drop(&mut self) {
        if let Some(close) = self.0.take() {
            close();
        }
    }
}

/// Coordinates the execution of rewound actions with the actions that consume their outputs.
/// Depending on the particular output service and action filesystem implementation, this may lead
/// to races, which this interface aims to prevent.
///
/// The executor finds it among the action contexts (`Arc<dyn RewoundActionSynchronizer>`); without
/// one it is [`NOOP`] (`OutputService.getRewoundActionSynchronizer()`'s default).
// Java: com/google/devtools/build/lib/vfs/OutputService.java:283 OutputService.RewoundActionSynchronizer
pub trait RewoundActionSynchronizer: Send + Sync {
    /// Guards an action from the beginning of its `Action#prepare` preparation until the end of
    /// its `Action#execute` execution.
    // Java: com/google/devtools/build/lib/vfs/OutputService.java:288 RewoundActionSynchronizer.enterActionPreparation
    fn enter_action_preparation(
        &self,
        action: &dyn Action,
        was_rewound: bool,
    ) -> Result<SilentCloseable, InterruptedException>;

    /// Guards an action from the beginning to the end of its `Action#execute` execution.
    // Java: com/google/devtools/build/lib/vfs/OutputService.java:292 RewoundActionSynchronizer.enterActionExecution
    fn enter_action_execution(
        &self,
        action: &dyn Action,
        was_rewound: bool,
        metadata_provider: &dyn InputMetadataProvider,
    ) -> Result<SilentCloseable, InterruptedException>;
}

/// A no-op implementation of [`RewoundActionSynchronizer`], suitable for action filesystems that
/// support racy access to action outputs.
// Java: com/google/devtools/build/lib/vfs/OutputService.java:300 RewoundActionSynchronizer.NOOP
pub struct NoopRewoundActionSynchronizer;

impl RewoundActionSynchronizer for NoopRewoundActionSynchronizer {
    fn enter_action_preparation(
        &self,
        _action: &dyn Action,
        _was_rewound: bool,
    ) -> Result<SilentCloseable, InterruptedException> {
        Ok(SilentCloseable::noop())
    }

    fn enter_action_execution(
        &self,
        _action: &dyn Action,
        _was_rewound: bool,
        _metadata_provider: &dyn InputMetadataProvider,
    ) -> Result<SilentCloseable, InterruptedException> {
        Ok(SilentCloseable::noop())
    }
}

/// `RewoundActionSynchronizer.NOOP`.
pub static NOOP: NoopRewoundActionSynchronizer = NoopRewoundActionSynchronizer;
