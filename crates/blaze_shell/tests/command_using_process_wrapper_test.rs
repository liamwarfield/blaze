//! Port of `src/test/java/com/google/devtools/build/lib/shell/CommandUsingProcessWrapperTest.java`.

mod common;

use blaze_shell::Command;

#[test]
fn test_command_echo() {
    let command = Command::new(
        common::strings(&["echo", "worker bees can leave"]),
        &common::getenv(),
    );
    let command_result = command.execute().unwrap();
    assert!(command_result.termination_status().success());
    assert!(
        command_result
            .stdout_stream()
            .to_string()
            .contains("worker bees can leave")
    );
}

#[test]
#[ignore = "needs ProcessWrapper (lib/runtime) and the process-wrapper binary"]
fn test_process_wrapped_command_echo() {}

#[test]
#[ignore = "needs ProcessWrapper (lib/runtime) and the cpu_time_spender test binary"]
fn test_process_wrapped_command_with_statistics_spend_user_time() {}

#[test]
#[ignore = "needs ProcessWrapper (lib/runtime) and the cpu_time_spender test binary"]
fn test_process_wrapped_command_with_statistics_spend_system_time() {}

#[test]
#[ignore = "needs ProcessWrapper (lib/runtime) and the cpu_time_spender test binary"]
fn test_process_wrapped_command_with_statistics_spend_user_and_system_time() {}
