//! Port of `lib/worker/WorkerProcessStatus.java`.

use std::fmt;

use blaze_protos::build_event_stream::build_metrics::worker_metrics::WorkerStatus;
use blaze_protos::failure_details::worker::Code;
use parking_lot::Mutex;

/// The status of the worker process.
// Java: com/google/devtools/build/lib/worker/WorkerProcessStatus.java:30 WorkerProcessStatus.Status
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Status {
    /// Used as a starting value, before the worker process has been created by Bazel.
    ///
    /// This state is not logged in the BEP as `WorkerSpawnRunner#initializeMetrics` is only
    /// called after `#prepareExecution` (which sets the status to ALIVE).
    NotStarted,
    /// Worker process has been created by Bazel (the process might not be immediately
    /// measurable).
    Alive,
    /// Intermediate states: Bazel has marked this worker process to be killed.
    PendingKillDueToUnknown,
    PendingKillDueToInterruptedException,
    PendingKillDueToUserExecException,
    PendingKillDueToIoException,
    PendingKillDueToMemoryPressure,
    /// Semi-terminal status: Bazel has determined that worker process has already been killed due
    /// to some unknown reason; if a more specific reason (below) comes along, we can transition to
    /// those statuses.
    KilledUnknown,
    // Terminal statuses: The worker process has been killed and Bazel is aware of the reason.
    /// Bazel killed the worker due to an InterruptedException, can happen when the remote branch
    /// interrupts the local branch after winning the race in dynamic execution.
    KilledDueToInterruptedException,
    /// Bazel killed the worker due to a UserExecException.
    KilledDueToUserExecException,
    /// Bazel killed the worker due to an IOException.
    KilledDueToIoException,
    /// This can be a result of Bazel forcibly killing the worker process, which might result in
    /// other exceptions caught in the execution threads (as enumerated by the other killed
    /// statuses above). Thus, this has the highest priority so that we can override that and set
    /// the correct reason why the worker was killed.
    KilledDueToMemoryPressure,
}

impl Status {
    /// The priority of a status determines whether another status can be used to override and
    /// update it.
    fn priority(self) -> i32 {
        match self {
            Status::NotStarted => 0,
            Status::Alive => 1,
            Status::PendingKillDueToUnknown => 2,
            Status::PendingKillDueToInterruptedException
            | Status::PendingKillDueToUserExecException
            | Status::PendingKillDueToIoException => 3,
            Status::PendingKillDueToMemoryPressure => 4,
            Status::KilledUnknown => 5,
            Status::KilledDueToInterruptedException
            | Status::KilledDueToUserExecException
            | Status::KilledDueToIoException => 6,
            Status::KilledDueToMemoryPressure => 7,
        }
    }

    // Java: com/google/devtools/build/lib/worker/WorkerProcessStatus.java:98 WorkerProcessStatus.Status.killedStatus
    pub fn killed_status(self) -> Status {
        match self {
            Status::PendingKillDueToUnknown => Status::KilledUnknown,
            Status::PendingKillDueToInterruptedException => Status::KilledDueToInterruptedException,
            Status::PendingKillDueToUserExecException => Status::KilledDueToUserExecException,
            Status::PendingKillDueToIoException => Status::KilledDueToIoException,
            Status::PendingKillDueToMemoryPressure => Status::KilledDueToMemoryPressure,
            other => other,
        }
    }

    // Java: com/google/devtools/build/lib/worker/WorkerProcessStatus.java:105 WorkerProcessStatus.Status.toWorkerStatus
    pub fn to_worker_status(self) -> WorkerStatus {
        match self {
            Status::NotStarted => WorkerStatus::NotStarted,
            Status::Alive
            | Status::PendingKillDueToUnknown
            | Status::PendingKillDueToInterruptedException
            | Status::PendingKillDueToUserExecException
            | Status::PendingKillDueToIoException
            | Status::PendingKillDueToMemoryPressure => WorkerStatus::Alive,
            Status::KilledUnknown => WorkerStatus::KilledUnknown,
            Status::KilledDueToInterruptedException => {
                WorkerStatus::KilledDueToInterruptedException
            }
            Status::KilledDueToUserExecException => WorkerStatus::KilledDueToUserExecException,
            Status::KilledDueToIoException => WorkerStatus::KilledDueToIoException,
            Status::KilledDueToMemoryPressure => WorkerStatus::KilledDueToMemoryPressure,
        }
    }
}

impl fmt::Display for Status {
    /// `toString()`: the enum constant's name.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Status::NotStarted => "NOT_STARTED",
            Status::Alive => "ALIVE",
            Status::PendingKillDueToUnknown => "PENDING_KILL_DUE_TO_UNKNOWN",
            Status::PendingKillDueToInterruptedException => {
                "PENDING_KILL_DUE_TO_INTERRUPTED_EXCEPTION"
            }
            Status::PendingKillDueToUserExecException => "PENDING_KILL_DUE_TO_USER_EXEC_EXCEPTION",
            Status::PendingKillDueToIoException => "PENDING_KILL_DUE_TO_IO_EXCEPTION",
            Status::PendingKillDueToMemoryPressure => "PENDING_KILL_DUE_TO_MEMORY_PRESSURE",
            Status::KilledUnknown => "KILLED_UNKNOWN",
            Status::KilledDueToInterruptedException => "KILLED_DUE_TO_INTERRUPTED_EXCEPTION",
            Status::KilledDueToUserExecException => "KILLED_DUE_TO_USER_EXEC_EXCEPTION",
            Status::KilledDueToIoException => "KILLED_DUE_TO_IO_EXCEPTION",
            Status::KilledDueToMemoryPressure => "KILLED_DUE_TO_MEMORY_PRESSURE",
        })
    }
}

/// `Code.toString()` (the proto enum constant's name).
pub fn worker_code_name(code: Code) -> &'static str {
    code.as_str_name()
}

/// This is a state machine instance that encapsulates the status of the worker process, and
/// provides a mechanism to signal to Bazel when to kill a worker.
// Java: com/google/devtools/build/lib/worker/WorkerProcessStatus.java:27 WorkerProcessStatus
#[derive(Debug)]
pub struct WorkerProcessStatus {
    state: Mutex<(Status, Option<Code>)>,
}

impl WorkerProcessStatus {
    // Java: com/google/devtools/build/lib/worker/WorkerProcessStatus.java:116 WorkerProcessStatus.<init>
    pub fn new() -> WorkerProcessStatus {
        WorkerProcessStatus {
            state: Mutex::new((Status::NotStarted, None)),
        }
    }

    // Java: com/google/devtools/build/lib/worker/WorkerProcessStatus.java:120 WorkerProcessStatus.get
    pub fn get(&self) -> Status {
        self.state.lock().0
    }

    /// A status is invalid if it is killed or marked to be killed later.
    // Java: com/google/devtools/build/lib/worker/WorkerProcessStatus.java:125 WorkerProcessStatus.isValid
    pub fn is_valid(&self) -> bool {
        let status = self.get();
        status == Status::NotStarted || status == Status::Alive
    }

    // Java: com/google/devtools/build/lib/worker/WorkerProcessStatus.java:129 WorkerProcessStatus.isPendingEviction
    pub fn is_pending_eviction(&self) -> bool {
        self.get() == Status::PendingKillDueToMemoryPressure
    }

    // Java: com/google/devtools/build/lib/worker/WorkerProcessStatus.java:133 WorkerProcessStatus.getWorkerCode
    pub fn get_worker_code(&self) -> Option<Code> {
        self.state.lock().1
    }

    // Java: com/google/devtools/build/lib/worker/WorkerProcessStatus.java:145 WorkerProcessStatus.isPendingKill
    fn is_pending_kill(status: Status) -> bool {
        matches!(
            status,
            Status::PendingKillDueToUnknown
                | Status::PendingKillDueToIoException
                | Status::PendingKillDueToInterruptedException
                | Status::PendingKillDueToUserExecException
                | Status::PendingKillDueToMemoryPressure
        )
    }

    // Java: com/google/devtools/build/lib/worker/WorkerProcessStatus.java:157 WorkerProcessStatus.isKilled
    pub fn is_killed(&self) -> bool {
        matches!(
            self.get(),
            Status::KilledUnknown
                | Status::KilledDueToIoException
                | Status::KilledDueToInterruptedException
                | Status::KilledDueToUserExecException
                | Status::KilledDueToMemoryPressure
        )
    }

    /// Attempts to update the status to its corresponding kill status. Should be called **after**
    /// the process is destroyed by Bazel.
    // Java: com/google/devtools/build/lib/worker/WorkerProcessStatus.java:166 WorkerProcessStatus.setKilled
    pub fn set_killed(&self) -> bool {
        let mut st = self.state.lock();
        if WorkerProcessStatus::is_pending_kill(st.0) {
            st.0 = st.0.killed_status();
            return true;
        }
        false
    }

    /// Returns whether the WorkerStatus was successfully updated after attempting to update it to
    /// a given Status.
    // Java: com/google/devtools/build/lib/worker/WorkerProcessStatus.java:179 WorkerProcessStatus.maybeUpdateStatus
    pub fn maybe_update_status(&self, to_status: Status) -> bool {
        let mut st = self.state.lock();
        if st.0.priority() < to_status.priority() {
            st.0 = to_status;
            return true;
        }
        false
    }

    // Java: com/google/devtools/build/lib/worker/WorkerProcessStatus.java:188 WorkerProcessStatus.maybeUpdateStatus
    pub fn maybe_update_status_with_code(&self, to_status: Status, worker_code: Code) -> bool {
        self.state.lock().1 = Some(worker_code);
        self.maybe_update_status(to_status)
    }

    // Java: com/google/devtools/build/lib/worker/WorkerProcessStatus.java:200 WorkerProcessStatus.toWorkerStatus
    pub fn to_worker_status(&self) -> WorkerStatus {
        self.get().to_worker_status()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Java tests: src/test/java/com/google/devtools/build/lib/worker/WorkerProcessStatusTest.java

    #[test]
    fn test_status_transitions_by_priority() {
        let status = WorkerProcessStatus::new();
        assert_eq!(status.get(), Status::NotStarted);
        assert!(status.maybe_update_status(Status::Alive));
        assert!(status.maybe_update_status(Status::PendingKillDueToUserExecException));
        // Same priority: no update.
        assert!(!status.maybe_update_status(Status::PendingKillDueToIoException));
        assert!(status.set_killed());
        assert_eq!(status.get(), Status::KilledDueToUserExecException);
        assert!(status.is_killed());
        assert!(!status.set_killed());
        assert!(status.maybe_update_status(Status::KilledDueToMemoryPressure));
    }
}
