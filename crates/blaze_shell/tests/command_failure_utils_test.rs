//! Port of `src/test/java/com/google/devtools/build/lib/util/CommandFailureUtilsTest.java`.

mod common;

use blaze_cmdline::Label;
use blaze_shell::{CommandDescriptionForm, CommandFailureUtils};

fn target() -> Label {
    Label::parse_canonical_unchecked("//foo:bar")
}

// PORT: PlatformInfo is not ported; the test only uses its label.
fn execution_platform_label() -> Label {
    Label::parse_canonical_unchecked("//platform:exec")
}

fn sh_args() -> Vec<String> {
    common::strings(&[
        "/bin/sh",
        "-c",
        "echo Some errors 1>&2; echo Some output; exit 42",
    ])
}

fn long_args(n: usize) -> Vec<String> {
    let mut args = vec!["some_command".to_string()];
    for i in 1..n {
        args.push(format!("arg{i}"));
    }
    args
}

#[test]
fn describe_command_failure() {
    let env = common::map(&[("FOO", "foo"), ("PATH", "/usr/bin:/bin:/sbin")]);
    let message = CommandFailureUtils::describe_command_failure_full(
        false,
        "Mnemonic",
        &sh_args(),
        Some(&env),
        None,
        Some("cfg12345"),
        Some(&format!("target {}", target())),
        Some(&execution_platform_label()),
        Some("local"),
    );
    assert_eq!(
        message,
        "sh failed: error executing Mnemonic command (from target //foo:bar) \
         /bin/sh -c 'echo Some errors 1>&2; echo Some output; exit 42'"
    );
}

#[test]
fn describe_command_failure_verbose() {
    let env = common::map(&[("FOO", "foo"), ("PATH", "/usr/bin:/bin:/sbin")]);
    let message = CommandFailureUtils::describe_command_failure_full(
        true,
        "Mnemonic",
        &sh_args(),
        Some(&env),
        None,
        Some("cfg12345"),
        Some(&format!("target {}", target())),
        Some(&execution_platform_label()),
        Some("local"),
    );
    assert_eq!(
        message,
        "sh failed: error executing Mnemonic command (from target //foo:bar) \n\
         \x20 (exec env - \\\n\
         \x20   FOO=foo \\\n\
         \x20   PATH=/usr/bin:/bin:/sbin \\\n\
         \x20 /bin/sh -c 'echo Some errors 1>&2; echo Some output; exit 42')\n\
         # Configuration: cfg12345\n\
         # Execution platform: //platform:exec\n\
         # Runner: local"
    );
}

#[test]
fn describe_command_failure_long_message() {
    let mut args = long_args(40);
    args[7] = "with spaces".to_string(); // Test embedded spaces in argument.
    args[9] = "*".to_string(); // Test shell meta characters.
    let env = common::map(&[("FOO", "foo"), ("PATH", "/usr/bin:/bin:/sbin")]);
    let message = CommandFailureUtils::describe_command_failure_full(
        false,
        "Mnemonic",
        &args,
        Some(&env),
        Some("/my/working/directory"),
        Some("cfg12345"),
        Some(&format!("target {}", target())),
        Some(&execution_platform_label()),
        Some("local"),
    );
    assert_eq!(
        message,
        "some_command failed: error executing Mnemonic command (from target //foo:bar) \
         some_command arg1 arg2 arg3 arg4 arg5 arg6 'with spaces' arg8 '*' arg10 \
         arg11 arg12 arg13 arg14 arg15 arg16 arg17 arg18 \
         arg19 arg20 arg21 arg22 arg23 arg24 arg25 arg26 \
         arg27 arg28 arg29 arg30 arg31 \
         ... (remaining 8 arguments skipped)"
    );
}

#[test]
fn describe_command_failure_long_message_verbose() {
    let mut args = long_args(40);
    args[7] = "with spaces".to_string();
    args[9] = "*".to_string();
    let env = common::map(&[("FOO", "foo"), ("PATH", "/usr/bin:/bin:/sbin")]);
    let message = CommandFailureUtils::describe_command_failure_full(
        true,
        "Mnemonic",
        &args,
        Some(&env),
        Some("/my/working/directory"),
        Some("cfg12345"),
        Some(&format!("target {}", target())),
        Some(&execution_platform_label()),
        Some("local"),
    );
    assert_eq!(
        message,
        "some_command failed: error executing Mnemonic command (from target //foo:bar) \n\
         \x20 (cd /my/working/directory && \\\n\
         \x20 exec env - \\\n\
         \x20   FOO=foo \\\n\
         \x20   PATH=/usr/bin:/bin:/sbin \\\n\
         \x20 some_command arg1 arg2 arg3 arg4 arg5 arg6 'with spaces' arg8 '*' arg10 \
         arg11 arg12 arg13 arg14 arg15 arg16 arg17 arg18 \
         arg19 arg20 arg21 arg22 arg23 arg24 arg25 arg26 \
         arg27 arg28 arg29 arg30 arg31 arg32 arg33 arg34 \
         arg35 arg36 arg37 arg38 arg39)\n\
         # Configuration: cfg12345\n\
         # Execution platform: //platform:exec\n\
         # Runner: local"
    );
}

#[test]
fn describe_command_failure_single_skipped_argument() {
    let args = long_args(35); // Long enough to make us skip 1 argument below.
    let env = common::map(&[]);
    let message = CommandFailureUtils::describe_command_failure_full(
        false,
        "Mnemonic",
        &args,
        Some(&env),
        Some("/my/working/directory"),
        Some("cfg12345"),
        Some(&format!("target {}", target())),
        Some(&execution_platform_label()),
        Some("local"),
    );
    assert_eq!(
        message,
        "some_command failed: error executing Mnemonic command (from target //foo:bar)\
         \x20some_command arg1 arg2 arg3 arg4 arg5 arg6 arg7 arg8 arg9 arg10 arg11 arg12\
         \x20arg13 arg14 arg15 arg16 arg17 arg18 arg19 arg20 arg21 arg22 arg23 arg24 arg25\
         \x20arg26 arg27 arg28 arg29 arg30 arg31 arg32 arg33 ... (remaining 1 argument\
         \x20skipped)"
    );
}

#[test]
fn describe_command_pretty_print_args() {
    let mut args = long_args(6);
    args[3] = "with spaces".to_string(); // Test embedded spaces in argument.
    args[4] = "*".to_string(); // Test shell meta characters.
    let env = common::map(&[("FOO", "foo"), ("PATH", "/usr/bin:/bin:/sbin")]);
    let env_to_clear = common::strings(&["CLEAR", "THIS"]);
    let message = CommandFailureUtils::describe_command(
        CommandDescriptionForm::Complete,
        true,
        &args,
        Some(&env),
        Some(&env_to_clear),
        Some("/my/working/directory"),
        Some("cfg12345"),
        Some(&execution_platform_label()),
        Some("remote"),
    );
    assert_eq!(
        message,
        "(cd /my/working/directory && \\\n\
         \x20 exec env - \\\n\
         \x20   -u CLEAR \\\n\
         \x20   -u THIS \\\n\
         \x20   FOO=foo \\\n\
         \x20   PATH=/usr/bin:/bin:/sbin \\\n\
         \x20 some_command \\\n\
         \x20   arg1 \\\n\
         \x20   arg2 \\\n\
         \x20   'with spaces' \\\n\
         \x20   '*' \\\n\
         \x20   arg5)\n\
         # Configuration: cfg12345\n\
         # Execution platform: //platform:exec\n\
         # Runner: remote"
    );
}

// Byte-strings review 2026-11-20 S1-2. Java CommandFailureUtils.java:105-106 compares
// `message.length() + commandElement.length()` (chars) with 200. The third element is the
// internal string of 70 ASCII chars and 40 `é` (C3 A9): 150 Java chars (230 UTF-8 bytes of the
// Rust string), so with "/bin/bash -c" (12 chars) Java prints the whole command.
#[test]
fn describe_command_failure_counts_chars_of_internal_strings() {
    let e = blaze_util::string_encoding::latin1_decode("é".as_bytes()).into_owned();
    let arg = format!("{}{}", "a".repeat(70), e.repeat(40));
    assert_eq!(arg.chars().count(), 150);
    let args = vec!["/bin/bash".to_string(), "-c".to_string(), arg.clone()];
    let env = common::map(&[]);
    let message = CommandFailureUtils::describe_command_failure_full(
        false,
        "Mnemonic",
        &args,
        Some(&env),
        None,
        None,
        Some(&format!("target {}", target())),
        None,
        None,
    );
    assert_eq!(
        message,
        format!(
            "bash failed: error executing Mnemonic command (from target //foo:bar) \
             /bin/bash -c '{arg}'"
        )
    );
}
