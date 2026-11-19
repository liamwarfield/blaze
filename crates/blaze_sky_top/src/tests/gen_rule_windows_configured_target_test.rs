//! Port of
//! `src/test/java/com/google/devtools/build/lib/bazel/rules/genrule/GenRuleWindowsConfiguredTargetTest.java`:
//! genrule with a Windows host platform (analysis only; runs on any host).

use crate::testing::BuildViewTestCase;
use blaze_actions::actions::action::CommandAction;
use blaze_actions::analysis::actions::spawn_action::SpawnAction;
use blaze_actions::{ActionRef, Artifact};
use blaze_analysis::configured_target::ConfiguredTarget;

/// `Pattern.compile(".*" + quote(prefix) + "\\s+(?<command>.*)").matcher(command).matches()`:
/// the `command` group, or `None` when the whole command does not match. Java's `.` matches
/// anything but a line terminator; the greedy `.*` prefix takes the last occurrence.
fn command_group<'a>(command: &'a str, prefix: &str) -> Option<&'a str> {
    let is_line_terminator = |c: char| c == '\n' || c == '\r';
    let is_java_whitespace = |c: char| matches!(c, ' ' | '\t' | '\n' | '\u{0B}' | '\u{0C}' | '\r');
    for (i, _) in command.rmatch_indices(prefix) {
        if command[..i].contains(is_line_terminator) {
            continue;
        }
        let after = &command[i + prefix.len()..];
        if !after.starts_with(is_java_whitespace) {
            continue;
        }
        let group = after.trim_start_matches(is_java_whitespace);
        if group.contains(is_line_terminator) {
            continue;
        }
        return Some(group);
    }
    None
}

/// `POWERSHELL_COMMAND_PATTERN`: `.*'utf8';\s+(?<command>.*)`.
const POWERSHELL_COMMAND_PREFIX: &str = "'utf8';";
/// `BASH_COMMAND_PATTERN`: `.*/genrule-setup.sh;\s+(?<command>.*)`.
const BASH_COMMAND_PREFIX: &str = "/genrule-setup.sh;";

// Java: GenRuleWindowsConfiguredTargetTest.assertCommandEquals
fn assert_command_equals(expected: &str, command: &str, pattern_prefix: &str) {
    // Ensure the command after the genrule setup is correct.
    let command = command_group(command, pattern_prefix).unwrap_or(command);
    assert_eq!(command, expected);
}

// Java: GenRuleWindowsConfiguredTargetTest.assertPowershellCommandEquals
fn assert_powershell_command_equals(expected: &str, command: &str) {
    assert_command_equals(expected, command, POWERSHELL_COMMAND_PREFIX);
}

// Java: GenRuleWindowsConfiguredTargetTest.assertBashCommandEquals
fn assert_bash_command_equals(expected: &str, command: &str) {
    assert_command_equals(expected, command, BASH_COMMAND_PREFIX);
}

// Java: GenRuleWindowsConfiguredTargetTest.getWindowsPath
fn get_windows_path(artifact: &Artifact) -> String {
    artifact.get_exec_path_string().replace('/', "\\")
}

/// `FileConfiguredTarget.getArtifact()`.
fn file_artifact(ct: &ConfiguredTarget) -> Artifact {
    match ct {
        ConfiguredTarget::InputFile(t) => t.get_artifact().clone(),
        ConfiguredTarget::OutputFile(t) => t.get_artifact().clone(),
        _ => panic!("ClassCastException: not a FileConfiguredTarget"),
    }
}

/// `(SpawnAction) action`.
fn as_spawn_action(action: &ActionRef) -> &SpawnAction {
    action
        .as_any()
        .downcast_ref::<SpawnAction>()
        .expect("ClassCastException: not a SpawnAction")
}

/// `assertThat(outputs).containsExactly(artifact)`.
fn assert_outputs_are(action: &ActionRef, artifact: &Artifact) {
    let outputs = action.get_outputs();
    assert_eq!(outputs, vec![artifact.clone()], "containsExactly");
}

/// The test case after the `@Before` methods.
// Java: GenRuleWindowsConfiguredTargetTest.assumeBazel
// Java: GenRuleWindowsConfiguredTargetTest.createWindowsPlatform
fn set_up() -> BuildViewTestCase {
    let mut t = BuildViewTestCase::new();
    // The cmd_{bash,bat,ps} attributes don't exist in Blaze.
    // PORT: assumeTrue(analysisMock.isThisBazel()) always holds: the mock is Bazel's.
    t.scratch.file(
        "platforms/BUILD",
        &["platform(name = 'windows', constraint_values = ['@platforms//os:windows'])"],
    );
    t.use_configuration(&["--host_platform=//platforms:windows"]);
    t
}

// Java: GenRuleWindowsConfiguredTargetTest.testCmdBatchIsPreferred
#[test]
fn test_cmd_batch_is_preferred() {
    let mut t = set_up();
    t.scratch.file(
        "genrule1/BUILD",
        &[r#"genrule(
    name = "hello_world",
    outs = ["message.txt"],
    cmd = 'echo "Hello, default cmd." >$(location message.txt)',
    cmd_bash = 'echo "Hello, Bash cmd." >$(location message.txt)',
    cmd_bat = 'echo "Hello, Batch cmd." >$(location message.txt)',
)
"#],
    );

    let message_artifact = file_artifact(&t.get_file_configured_target("//genrule1:message.txt"));
    let shell_action = t
        .get_generating_action(&message_artifact)
        .expect("shellAction is not null");
    let arguments = as_spawn_action(&shell_action).get_arguments().unwrap();

    assert_outputs_are(&shell_action, &message_artifact);

    let expected = format!(
        "echo \"Hello, Batch cmd.\" >{}",
        get_windows_path(&message_artifact)
    );
    assert_eq!(arguments[0], "cmd.exe");
    let last = arguments.len() - 1;
    assert_eq!(arguments[last - 1], "/c");
    assert_eq!(arguments[last], expected);
}

// Java: GenRuleWindowsConfiguredTargetTest.testCmdPsIsPreferred
#[test]
fn test_cmd_ps_is_preferred() {
    let mut t = set_up();
    t.scratch.file(
        "genrule1/BUILD",
        &[r#"genrule(
    name = "hello_world",
    outs = ["message.txt"],
    cmd = 'echo "Hello, default cmd." >$(location message.txt)',
    cmd_bash = 'echo "Hello, Bash cmd." >$(location message.txt)',
    cmd_bat = 'echo "Hello, Batch cmd." >$(location message.txt)',
    cmd_ps = 'echo "Hello, Powershell cmd." >$(location message.txt)',
)
"#],
    );

    let message_artifact = file_artifact(&t.get_file_configured_target("//genrule1:message.txt"));
    let shell_action = t
        .get_generating_action(&message_artifact)
        .expect("shellAction is not null");
    let arguments = as_spawn_action(&shell_action).get_arguments().unwrap();

    assert_outputs_are(&shell_action, &message_artifact);

    let expected = format!(
        "echo \"Hello, Powershell cmd.\" >{}",
        message_artifact.get_exec_path_string()
    );
    assert_eq!(arguments[0], "powershell.exe");
    assert_eq!(arguments[1], "/c");
    assert_powershell_command_equals(&expected, &arguments[2]);
}

// Java: GenRuleWindowsConfiguredTargetTest.testScriptFileIsUsedForBatchCmd
#[test]
fn test_script_file_is_used_for_batch_cmd() {
    let mut t = set_up();
    t.scratch.file(
        "genrule1/BUILD",
        &[
            "genrule(name = 'hello_world',",
            "outs = ['message.txt'],",
            r#"cmd_bat  = ' && '.join(["echo \"Hello, Batch cmd, %s.\" >$(location message.txt)" % i for i in range(1, 1000)]),)"#,
        ],
    );
    t.use_configuration(&[
        "--platforms=//platforms:windows",
        "--host_platform=//platforms:windows",
        "--experimental_platform_in_output_dir",
    ]);

    let message_artifact = file_artifact(&t.get_file_configured_target("//genrule1:message.txt"));
    let shell_action = t
        .get_generating_action(&message_artifact)
        .expect("shellAction is not null");
    let arguments = as_spawn_action(&shell_action).get_arguments().unwrap();

    assert_outputs_are(&shell_action, &message_artifact);

    let expected = "bazel-out\\windows-fastbuild\\bin\\genrule1\\hello_world.genrule_script.bat";
    assert_eq!(arguments[0], "cmd.exe");
    let last = arguments.len() - 1;
    assert_eq!(arguments[last - 1], "/c");
    assert_powershell_command_equals(expected, &arguments[last]);
}

// Java: GenRuleWindowsConfiguredTargetTest.testScriptFileIsUsedForPowershellCmd
#[test]
fn test_script_file_is_used_for_powershell_cmd() {
    let mut t = set_up();
    t.scratch.file(
        "genrule1/BUILD",
        &[
            "genrule(name = 'hello_world',",
            "outs = ['message.txt'],",
            r#"cmd_ps  = '; '.join(["echo \"Hello, Powershell cmd, %s.\" >$(location message.txt)" % i for i in range(1, 1000)]),)"#,
        ],
    );
    t.use_configuration(&[
        "--platforms=//platforms:windows",
        "--host_platform=//platforms:windows",
        "--experimental_platform_in_output_dir",
    ]);

    let message_artifact = file_artifact(&t.get_file_configured_target("//genrule1:message.txt"));
    let shell_action = t
        .get_generating_action(&message_artifact)
        .expect("shellAction is not null");
    let arguments = as_spawn_action(&shell_action).get_arguments().unwrap();

    assert_outputs_are(&shell_action, &message_artifact);

    let expected = ".\\bazel-out\\windows-fastbuild\\bin\\genrule1\\hello_world.genrule_script.ps1";
    assert_eq!(arguments[0], "powershell.exe");
    assert_eq!(arguments[1], "/c");
    assert_powershell_command_equals(expected, &arguments[2]);
}

// Java: GenRuleWindowsConfiguredTargetTest.testCmdBashIsPreferred
#[test]
fn test_cmd_bash_is_preferred() {
    let mut t = set_up();
    t.scratch.file(
        "genrule1/BUILD",
        &[r#"genrule(
    name = "hello_world",
    outs = ["message.txt"],
    cmd = 'echo "Hello, default cmd." >$(location message.txt)',
    cmd_bash = 'echo "Hello, Bash cmd." >$(location message.txt)',
)
"#],
    );

    let message_artifact = file_artifact(&t.get_file_configured_target("//genrule1:message.txt"));
    let shell_action = t
        .get_generating_action(&message_artifact)
        .expect("shellAction is not null");
    let arguments = as_spawn_action(&shell_action).get_arguments().unwrap();

    assert_outputs_are(&shell_action, &message_artifact);

    let expected = format!(
        "echo \"Hello, Bash cmd.\" >{}",
        message_artifact.get_exec_path_string()
    );
    assert_eq!(arguments[0], "c:/msys64/usr/bin/bash.exe");
    assert_eq!(arguments[1], "-c");
    assert_bash_command_equals(&expected, &arguments[2]);
}

// Java: GenRuleWindowsConfiguredTargetTest.testMissingCmdAttributeError
#[test]
fn test_missing_cmd_attribute_error() {
    let mut t = set_up();
    t.check_error(
        "foo",
        "bar",
        "missing value for `cmd` attribute, you can also set `cmd_ps` or `cmd_bat` on \
         Windows and `cmd_bash` on other platforms.",
        &["genrule(name='bar',      srcs = [],      outs=['out'])"],
    );
}

// Java: GenRuleWindowsConfiguredTargetTest.testMissingCmdAttributeErrorOnNonWindowsPlatform
#[test]
fn test_missing_cmd_attribute_error_on_non_windows_platform() {
    let mut t = set_up();
    t.scratch.file(
        "newplatforms/BUILD",
        &["platform(name = 'nonwindows', constraint_values = ['@platforms//os:linux'])"],
    );
    t.use_configuration(&["--host_platform=//newplatforms:nonwindows"]);

    t.check_error(
        "foo",
        "bar",
        "missing value for `cmd` attribute, you can also set `cmd_ps` or `cmd_bat` on \
         Windows and `cmd_bash` on other platforms.",
        &["genrule(name='bar',      srcs = [],      outs=['out'],      cmd_bat='echo hello > $(@)')"],
    );
}
