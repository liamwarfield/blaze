//! Port of src/test/java/com/google/devtools/build/lib/worker/WorkerProcessStatusTest.java
//! (a parameterized test: one Rust test per method, looping over Java's parameter rows).

use blaze_exec::worker::worker_process_status::{Status, WorkerProcessStatus};
use blaze_protos::build_event_stream::build_metrics::worker_metrics::WorkerStatus;

fn create_status_of(status: Status) -> WorkerProcessStatus {
    let worker_process_status = WorkerProcessStatus::new();
    worker_process_status.maybe_update_status(status);
    worker_process_status
}

#[test]
fn test_is_killed() {
    let cases = [
        (Status::NotStarted, false),
        (Status::Alive, false),
        (Status::PendingKillDueToUnknown, false),
        (Status::PendingKillDueToInterruptedException, false),
        (Status::PendingKillDueToUserExecException, false),
        (Status::PendingKillDueToIoException, false),
        (Status::PendingKillDueToMemoryPressure, false),
        (Status::KilledUnknown, true),
        (Status::KilledDueToInterruptedException, true),
        (Status::KilledDueToUserExecException, true),
        (Status::KilledDueToIoException, true),
        (Status::KilledDueToMemoryPressure, true),
    ];
    for (status, is_valid) in cases {
        assert_eq!(create_status_of(status).is_killed(), is_valid, "{status}");
    }
}

#[test]
fn test_is_valid() {
    let cases = [
        (Status::NotStarted, true),
        (Status::Alive, true),
        (Status::PendingKillDueToUnknown, false),
        (Status::PendingKillDueToInterruptedException, false),
        (Status::PendingKillDueToUserExecException, false),
        (Status::PendingKillDueToIoException, false),
        (Status::PendingKillDueToMemoryPressure, false),
        (Status::KilledUnknown, false),
        (Status::KilledDueToInterruptedException, false),
        (Status::KilledDueToUserExecException, false),
        (Status::KilledDueToIoException, false),
        (Status::KilledDueToMemoryPressure, false),
    ];
    for (status, is_valid) in cases {
        assert_eq!(create_status_of(status).is_valid(), is_valid, "{status}");
    }
}

#[test]
fn test_set_killed() {
    let cases = [
        (Status::PendingKillDueToUnknown, Status::KilledUnknown),
        (
            Status::PendingKillDueToMemoryPressure,
            Status::KilledDueToMemoryPressure,
        ),
        (
            Status::KilledDueToInterruptedException,
            Status::KilledDueToInterruptedException,
        ),
        (
            Status::PendingKillDueToIoException,
            Status::KilledDueToIoException,
        ),
        (
            Status::PendingKillDueToUserExecException,
            Status::KilledDueToUserExecException,
        ),
    ];
    for (pending, killed) in cases {
        let status = create_status_of(pending);
        status.set_killed();
        assert_eq!(status.get(), killed);
    }
}

#[test]
fn test_to_worker_status() {
    let cases = [
        (Status::NotStarted, WorkerStatus::NotStarted),
        (Status::Alive, WorkerStatus::Alive),
        (Status::PendingKillDueToUnknown, WorkerStatus::Alive),
        (
            Status::PendingKillDueToInterruptedException,
            WorkerStatus::Alive,
        ),
        (
            Status::PendingKillDueToUserExecException,
            WorkerStatus::Alive,
        ),
        (Status::PendingKillDueToIoException, WorkerStatus::Alive),
        (Status::PendingKillDueToMemoryPressure, WorkerStatus::Alive),
        (Status::KilledUnknown, WorkerStatus::KilledUnknown),
        (
            Status::KilledDueToInterruptedException,
            WorkerStatus::KilledDueToInterruptedException,
        ),
        (
            Status::KilledDueToUserExecException,
            WorkerStatus::KilledDueToUserExecException,
        ),
        (
            Status::KilledDueToIoException,
            WorkerStatus::KilledDueToIoException,
        ),
        (
            Status::KilledDueToMemoryPressure,
            WorkerStatus::KilledDueToMemoryPressure,
        ),
    ];
    for (status, proto) in cases {
        assert_eq!(
            create_status_of(status).to_worker_status(),
            proto,
            "{status}"
        );
    }
}

#[test]
fn test_maybe_update_status_successful_update() {
    let cases = [
        (Status::NotStarted, Status::Alive),
        (Status::Alive, Status::PendingKillDueToUnknown),
        (Status::Alive, Status::PendingKillDueToInterruptedException),
        (Status::Alive, Status::PendingKillDueToUserExecException),
        (Status::Alive, Status::PendingKillDueToIoException),
        (Status::Alive, Status::PendingKillDueToMemoryPressure),
        (Status::Alive, Status::KilledUnknown),
        (Status::Alive, Status::KilledDueToIoException),
        (Status::Alive, Status::KilledDueToInterruptedException),
        (Status::Alive, Status::KilledDueToUserExecException),
        (Status::Alive, Status::KilledDueToMemoryPressure),
        (
            Status::PendingKillDueToUnknown,
            Status::PendingKillDueToIoException,
        ),
        (
            Status::PendingKillDueToUnknown,
            Status::PendingKillDueToInterruptedException,
        ),
        (
            Status::PendingKillDueToUnknown,
            Status::PendingKillDueToUserExecException,
        ),
        (
            Status::PendingKillDueToUnknown,
            Status::PendingKillDueToMemoryPressure,
        ),
        (
            Status::PendingKillDueToIoException,
            Status::PendingKillDueToMemoryPressure,
        ),
        (
            Status::PendingKillDueToInterruptedException,
            Status::PendingKillDueToMemoryPressure,
        ),
        (
            Status::PendingKillDueToUserExecException,
            Status::PendingKillDueToMemoryPressure,
        ),
        (Status::PendingKillDueToUnknown, Status::KilledUnknown),
        (
            Status::PendingKillDueToInterruptedException,
            Status::KilledDueToInterruptedException,
        ),
        (
            Status::PendingKillDueToUserExecException,
            Status::KilledDueToUserExecException,
        ),
        (
            Status::PendingKillDueToIoException,
            Status::KilledDueToIoException,
        ),
        (
            Status::PendingKillDueToMemoryPressure,
            Status::KilledDueToMemoryPressure,
        ),
        (
            Status::KilledUnknown,
            Status::KilledDueToInterruptedException,
        ),
        (Status::KilledUnknown, Status::KilledDueToUserExecException),
        (Status::KilledUnknown, Status::KilledDueToIoException),
        (Status::KilledUnknown, Status::KilledDueToMemoryPressure),
        (
            Status::KilledDueToInterruptedException,
            Status::KilledDueToMemoryPressure,
        ),
        (
            Status::KilledDueToUserExecException,
            Status::KilledDueToMemoryPressure,
        ),
        (
            Status::KilledDueToIoException,
            Status::KilledDueToMemoryPressure,
        ),
    ];
    for (from, to) in cases {
        let from_status = create_status_of(from);
        assert_eq!(from_status.get(), from);
        assert_eq!(from_status.maybe_update_status(to), true, "{from} -> {to}");
        assert_eq!(from_status.get(), to);
    }
}

#[test]
fn test_maybe_update_status_no_update() {
    let cases = [
        (Status::Alive, Status::NotStarted),
        (Status::PendingKillDueToUnknown, Status::Alive),
        (Status::PendingKillDueToInterruptedException, Status::Alive),
        (Status::PendingKillDueToUserExecException, Status::Alive),
        (Status::PendingKillDueToIoException, Status::Alive),
        (Status::PendingKillDueToMemoryPressure, Status::Alive),
        (
            Status::PendingKillDueToInterruptedException,
            Status::PendingKillDueToUnknown,
        ),
        (
            Status::PendingKillDueToUserExecException,
            Status::PendingKillDueToUnknown,
        ),
        (
            Status::PendingKillDueToIoException,
            Status::PendingKillDueToUnknown,
        ),
        (
            Status::PendingKillDueToMemoryPressure,
            Status::PendingKillDueToUnknown,
        ),
        (Status::KilledUnknown, Status::PendingKillDueToUnknown),
        (
            Status::KilledDueToInterruptedException,
            Status::PendingKillDueToInterruptedException,
        ),
        (
            Status::KilledDueToUserExecException,
            Status::PendingKillDueToUserExecException,
        ),
        (
            Status::KilledDueToIoException,
            Status::PendingKillDueToIoException,
        ),
        (
            Status::KilledDueToMemoryPressure,
            Status::PendingKillDueToMemoryPressure,
        ),
        (Status::KilledDueToMemoryPressure, Status::KilledUnknown),
        (
            Status::KilledDueToMemoryPressure,
            Status::KilledDueToInterruptedException,
        ),
        (
            Status::KilledDueToMemoryPressure,
            Status::KilledDueToUserExecException,
        ),
        (
            Status::KilledDueToMemoryPressure,
            Status::KilledDueToIoException,
        ),
    ];
    for (from, to) in cases {
        let from_status = create_status_of(from);
        assert_eq!(from_status.get(), from);
        assert_eq!(from_status.maybe_update_status(to), false, "{from} -> {to}");
        assert_eq!(from_status.get(), from);
    }
}
