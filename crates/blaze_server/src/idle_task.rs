//! `IdleTask` and `IdleTaskException`.

use std::fmt;
use std::time::Duration;

use blaze_shell::thread_interrupt::InterruptedException;

/// The status of an idle task.
// Java: com/google/devtools/build/lib/server/IdleTask.java:22 IdleTask.Status
#[allow(non_camel_case_types)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum IdleTaskStatus {
    /// The task was never started.
    NOT_STARTED,
    /// The task finished successfully.
    SUCCESS,
    /// The task finished with an error.
    FAILURE,
    /// The task was interrupted while running.
    INTERRUPTED,
}

/// The result of running an idle task.
// Java: com/google/devtools/build/lib/server/IdleTask.java:34 IdleTask.Result
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct IdleTaskResult {
    pub name: String,
    pub status: IdleTaskStatus,
    pub running_time: Duration,
}

impl IdleTaskResult {
    pub fn new(name: impl Into<String>, status: IdleTaskStatus, running_time: Duration) -> Self {
        IdleTaskResult {
            name: name.into(),
            status,
            running_time,
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn status(&self) -> IdleTaskStatus {
        self.status
    }

    pub fn running_time(&self) -> Duration {
        self.running_time
    }
}

/// An exception thrown by an [`IdleTask`] when it fails for a reason other than interruption by
/// the [`crate::idle_task_manager::IdleTaskManager`].
// Java: com/google/devtools/build/lib/server/IdleTaskException.java:20 IdleTaskException
#[derive(Debug)]
pub struct IdleTaskException {
    cause: Box<dyn std::error::Error + Send + Sync>,
}

impl IdleTaskException {
    // Java: com/google/devtools/build/lib/server/IdleTaskException.java:21 IdleTaskException.<init>
    pub fn new(cause: impl Into<Box<dyn std::error::Error + Send + Sync>>) -> Self {
        IdleTaskException {
            cause: cause.into(),
        }
    }

    pub fn get_cause(&self) -> &(dyn std::error::Error + Send + Sync) {
        &*self.cause
    }
}

impl fmt::Display for IdleTaskException {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Java: `Exception(Throwable cause)` takes `cause.toString()` as its message.
        write!(f, "{}", self.cause)
    }
}

impl std::error::Error for IdleTaskException {}

/// What [`IdleTask::run`] throws: `IdleTaskException` or `InterruptedException`.
#[derive(Debug)]
pub enum IdleTaskError {
    IdleTask(IdleTaskException),
    Interrupted(InterruptedException),
}

impl From<IdleTaskException> for IdleTaskError {
    fn from(e: IdleTaskException) -> Self {
        IdleTaskError::IdleTask(e)
    }
}

impl From<InterruptedException> for IdleTaskError {
    fn from(e: InterruptedException) -> Self {
        IdleTaskError::Interrupted(e)
    }
}

/// A task to be executed by [`crate::idle_task_manager::IdleTaskManager`] while the server is
/// idle.
// Java: com/google/devtools/build/lib/server/IdleTask.java:19 IdleTask
pub trait IdleTask: Send + Sync {
    /// A display name for the task.
    // Java: com/google/devtools/build/lib/server/IdleTask.java:37 IdleTask.displayName
    fn display_name(&self) -> String;

    /// Returns how long to remain idle before executing the task.
    ///
    /// The `IdleTaskManager` will wait for at least this long, but may delay execution further if
    /// many cleanup tasks were registered.
    // Java: com/google/devtools/build/lib/server/IdleTask.java:45 IdleTask.delay
    fn delay(&self) -> Duration {
        Duration::ZERO
    }

    /// Executes the task.
    ///
    /// Implementations are encouraged to handle interruption promptly by returning
    /// [`IdleTaskError::Interrupted`], but are not required to do so, and may instead opt to
    /// complete normally. Either way, the `IdleTaskManager` will wait for the task to finish
    /// before leaving the idle period, so the task may assume that a subsequent command will not
    /// interfere with it.
    ///
    /// PORT: the task runs on the manager's thread; "interrupted" is that thread's
    /// `blaze_shell::thread_interrupt` status.
    // Java: com/google/devtools/build/lib/server/IdleTask.java:60 IdleTask.run
    fn run(&self) -> Result<(), IdleTaskError>;
}
