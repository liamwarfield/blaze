//! Port of `src/test/java/com/google/devtools/build/lib/shell/CommandUsingLinuxSandboxTest.java`.

mod common;

use blaze_shell::Command;

#[test]
fn test_command_echo() {
    let command = Command::new(
        common::strings(&["echo", "colorless green ideas"]),
        &common::getenv(),
    );
    let command_result = command.execute().unwrap();
    assert!(command_result.termination_status().success());
    assert!(
        command_result
            .stdout_stream()
            .to_string()
            .contains("colorless green ideas")
    );
}

#[test]
#[ignore = "needs LinuxSandboxCommandLineBuilder (lib/sandbox) and the linux-sandbox binary"]
fn test_linux_sandboxed_command_echo() {}

#[test]
#[ignore = "needs LinuxSandboxCommandLineBuilder (lib/sandbox) and the cpu_time_spender test binary"]
fn test_linux_sandboxed_command_with_statistics_spend_user_time() {}

#[test]
#[ignore = "needs LinuxSandboxCommandLineBuilder (lib/sandbox) and the cpu_time_spender test binary"]
fn test_linux_sandboxed_command_with_statistics_spend_system_time() {}

#[test]
#[ignore = "needs LinuxSandboxCommandLineBuilder (lib/sandbox) and the cpu_time_spender test binary"]
fn test_linux_sandboxed_command_with_statistics_spend_user_and_system_time() {}
