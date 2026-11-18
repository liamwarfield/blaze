//! Port of `src/test/java/com/google/devtools/build/lib/util/CommandUtilsTest.java`.

mod common;

use std::collections::BTreeMap;

use indexmap::IndexMap;

use blaze_shell::{Command, CommandExecutionError, CommandFailureUtils, CommandUtils, JavaFile};

/// `Maps.newTreeMap()` filled with PATH and FOO: iteration is sorted.
fn tree_env() -> IndexMap<String, String> {
    let mut env = BTreeMap::new();
    env.insert("PATH".to_string(), "/usr/bin:/bin:/sbin".to_string());
    env.insert("FOO".to_string(), "foo".to_string());
    env.into_iter().collect()
}

fn build_long_command() -> Command {
    let mut args = vec!["this_command_will_not_be_found".to_string()];
    for i in 1..40 {
        args.push(format!("arg{i}"));
    }
    Command::with_env(
        args,
        Some(tree_env()),
        Some(JavaFile::new("/tmp")),
        &common::getenv(),
    )
}

#[test]
fn long_command() {
    let command = build_long_command();
    let message = CommandFailureUtils::describe_command_failure(
        false,
        CommandUtils::cwd(&command).as_deref(),
        &command,
    );
    assert_eq!(
        message,
        "this_command_will_not_be_found failed: \
         error executing <shell command> command this_command_will_not_be_found arg1 \
         arg2 arg3 arg4 arg5 arg6 arg7 arg8 arg9 arg10 \
         arg11 arg12 arg13 arg14 arg15 arg16 arg17 arg18 \
         arg19 arg20 arg21 arg22 arg23 arg24 arg25 arg26 \
         arg27 arg28 arg29 arg30 \
         ... (remaining 9 arguments skipped)"
    );
}

#[test]
fn long_command_verbose() {
    let command = build_long_command();
    let verbose_message = CommandFailureUtils::describe_command_failure(
        true,
        CommandUtils::cwd(&command).as_deref(),
        &command,
    );
    assert_eq!(
        verbose_message,
        "this_command_will_not_be_found failed: error executing <shell command> command \n  \
         (cd /tmp && \\\n  \
         exec env - \\\n    \
         FOO=foo \\\n    \
         PATH=/usr/bin:/bin:/sbin \\\n  \
         this_command_will_not_be_found arg1 \
         arg2 arg3 arg4 arg5 arg6 arg7 arg8 arg9 arg10 \
         arg11 arg12 arg13 arg14 arg15 arg16 arg17 arg18 \
         arg19 arg20 arg21 arg22 arg23 arg24 arg25 arg26 \
         arg27 arg28 arg29 arg30 arg31 arg32 arg33 arg34 \
         arg35 arg36 arg37 arg38 arg39)"
    );
}

#[test]
fn failing_command() {
    let args = common::strings(&[
        "/bin/sh",
        "-c",
        "echo Some errors 1>&2; echo Some output; exit 42",
    ]);
    let exception =
        match Command::with_env(args, Some(tree_env()), None, &common::getenv()).execute() {
            Err(CommandExecutionError::Command(e)) => e,
            other => panic!("expected CommandException, got {:?}", other.map(|_| ())),
        };
    let message = CommandUtils::describe_command_failure(false, &exception);
    let verbose_message = CommandUtils::describe_command_failure(true, &exception);
    assert_eq!(
        message,
        "sh failed: error executing <shell command> command \
         /bin/sh -c 'echo Some errors 1>&2; echo Some output; exit 42': \
         Process exited with status 42\n\
         Some output\n\
         Some errors\n"
    );
    assert_eq!(
        verbose_message,
        "sh failed: error executing <shell command> command \n  \
         (exec env - \\\n    \
         FOO=foo \\\n    \
         PATH=/usr/bin:/bin:/sbin \\\n  \
         /bin/sh -c 'echo Some errors 1>&2; echo Some output; exit 42'): \
         Process exited with status 42\n\
         Some output\n\
         Some errors\n"
    );
}
