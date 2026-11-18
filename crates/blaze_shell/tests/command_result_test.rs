//! Port of `src/test/java/com/google/devtools/build/lib/shell/CommandResultTest.java`.

mod common;

use std::time::Duration;

use blaze_shell::{CommandResult, TerminationStatus};

#[test]
fn test_builder_with_no_stderr() {
    let msg = common::panic_message(|| {
        CommandResult::builder()
            .set_stdout_stream(CommandResult::EMPTY_OUTPUT)
            .set_termination_status(TerminationStatus::new(0, false))
            .build()
    });
    assert!(msg.starts_with("IllegalStateException"), "{msg}");
    assert!(msg.contains("stderrStream"), "{msg}");
}

#[test]
fn test_builder_with_no_stdout() {
    let msg = common::panic_message(|| {
        CommandResult::builder()
            .set_stderr_stream(CommandResult::EMPTY_OUTPUT)
            .set_termination_status(TerminationStatus::new(0, false))
            .build()
    });
    assert!(msg.starts_with("IllegalStateException"), "{msg}");
    assert!(msg.contains("stdoutStream"), "{msg}");
}

#[test]
fn test_builder_with_no_termination_status() {
    let msg = common::panic_message(|| {
        CommandResult::builder()
            .set_stdout_stream(CommandResult::EMPTY_OUTPUT)
            .set_stderr_stream(CommandResult::EMPTY_OUTPUT)
            .build()
    });
    assert!(msg.starts_with("IllegalStateException"), "{msg}");
    assert!(msg.contains("terminationStatus"), "{msg}");
}

#[test]
fn test_builder_with_no_execution_time() {
    let command_result = CommandResult::builder()
        .set_stdout_stream(CommandResult::EMPTY_OUTPUT)
        .set_stderr_stream(CommandResult::EMPTY_OUTPUT)
        .set_termination_status(TerminationStatus::new(0, false))
        .build();
    assert_eq!(command_result.wall_execution_time(), None);
    assert_eq!(command_result.user_execution_time(), None);
    assert_eq!(command_result.system_execution_time(), None);
}

#[test]
fn test_builder_with_execution_time() {
    let command_result = CommandResult::builder()
        .set_stdout_stream(CommandResult::EMPTY_OUTPUT)
        .set_stderr_stream(CommandResult::EMPTY_OUTPUT)
        .set_termination_status(TerminationStatus::new(0, false))
        .set_wall_execution_time(Duration::from_millis(1929))
        .set_user_execution_time(Duration::from_millis(1492))
        .set_system_execution_time(Duration::from_millis(1787))
        .build();
    assert_eq!(
        command_result.wall_execution_time(),
        Some(Duration::from_millis(1929))
    );
    assert_eq!(
        command_result.user_execution_time(),
        Some(Duration::from_millis(1492))
    );
    assert_eq!(
        command_result.system_execution_time(),
        Some(Duration::from_millis(1787))
    );
}
