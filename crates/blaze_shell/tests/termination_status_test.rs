//! Port of `src/test/java/com/google/devtools/build/lib/shell/TerminationStatusTest.java`.

mod common;

use std::time::Duration;

use blaze_shell::TerminationStatus;

#[test]
fn test_crashed_exit_codes_return_false() {
    assert!(!TerminationStatus::crashed(0));
    assert!(!TerminationStatus::crashed(1));
    assert!(!TerminationStatus::crashed(127));
}

#[test]
fn test_crashed_termination_signals_return_false() {
    assert!(!TerminationStatus::crashed(TerminationStatus::SIGNAL_1));
    assert!(!TerminationStatus::crashed(TerminationStatus::SIGNAL_63));
    assert!(!TerminationStatus::crashed(
        TerminationStatus::SIGNAL_SIGKILL
    ));
    assert!(!TerminationStatus::crashed(
        TerminationStatus::SIGNAL_SIGTERM
    ));
}

#[test]
fn test_crashed_abrupt_signals_return_true() {
    assert!(TerminationStatus::crashed(
        TerminationStatus::SIGNAL_SIGABRT
    ));
    assert!(TerminationStatus::crashed(TerminationStatus::SIGNAL_SIGBUS));
}

#[test]
fn test_builder_with_no_wait_response() {
    let msg = common::panic_message(|| TerminationStatus::builder().set_timed_out(false).build());
    assert!(msg.starts_with("IllegalStateException"), "{msg}");
}

#[test]
fn test_builder_with_no_timed_out() {
    let msg = common::panic_message(|| TerminationStatus::builder().set_wait_response(0).build());
    assert!(msg.starts_with("IllegalStateException"), "{msg}");
}

#[test]
fn test_builder_with_no_execution_time() {
    let termination_status = TerminationStatus::builder()
        .set_wait_response(0)
        .set_timed_out(false)
        .build();
    assert_eq!(termination_status.get_wall_execution_time(), None);
    assert_eq!(termination_status.get_user_execution_time(), None);
    assert_eq!(termination_status.get_system_execution_time(), None);
}

#[test]
fn test_builder_with_execution_time() {
    let termination_status = TerminationStatus::builder()
        .set_wait_response(0)
        .set_timed_out(false)
        .set_wall_execution_time(Duration::from_millis(1929))
        .set_user_execution_time(Duration::from_millis(1492))
        .set_system_execution_time(Duration::from_millis(1787))
        .build();
    assert_eq!(
        termination_status.get_wall_execution_time(),
        Some(Duration::from_millis(1929))
    );
    assert_eq!(
        termination_status.get_user_execution_time(),
        Some(Duration::from_millis(1492))
    );
    assert_eq!(
        termination_status.get_system_execution_time(),
        Some(Duration::from_millis(1787))
    );
}
