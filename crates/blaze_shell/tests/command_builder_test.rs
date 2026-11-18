//! Port of `src/test/java/com/google/devtools/build/lib/util/CommandBuilderTest.java`.

mod common;

use indexmap::IndexMap;

use blaze_shell::{CommandBuilder, OS};

fn linux_builder() -> CommandBuilder {
    let mut b = CommandBuilder::with_os(OS::Linux, &IndexMap::new());
    b.use_temp_dir();
    b
}

fn win_builder() -> CommandBuilder {
    let mut b = CommandBuilder::with_os(OS::Windows, &IndexMap::new());
    b.use_temp_dir();
    b
}

fn assert_argv(builder: &mut CommandBuilder, expected: &[&str]) {
    assert_eq!(builder.build().get_arguments(), common::strings(expected));
}

fn assert_win_cmd_argv(builder: &mut CommandBuilder, expected: &str) {
    assert_argv(
        builder,
        &["CMD.EXE", "/S", "/E:ON", "/V:ON", "/D", "/C", expected],
    );
}

/// Java asserts the exception message; Rust panics with "<ExceptionClass>: <message>".
fn assert_failure(mut builder: CommandBuilder, expected: &str) {
    let msg = common::panic_message(move || builder.build());
    let message = msg.split_once(": ").map(|(_, m)| m).unwrap_or(&msg);
    assert_eq!(message, expected);
}

#[test]
fn linux_builder_test() {
    assert_argv(linux_builder().add_arg("abc"), &["abc"]);
    assert_argv(linux_builder().add_arg("abc def"), &["abc def"]);
    assert_argv(linux_builder().add_args(&["abc", "def"]), &["abc", "def"]);
    assert_argv(
        linux_builder().add_args(&["abc".to_string(), "def".to_string()]),
        &["abc", "def"],
    );
    assert_argv(
        linux_builder().add_arg("abc").use_shell(true),
        &["/bin/sh", "-c", "abc"],
    );
    assert_argv(
        linux_builder().add_arg("abc def").use_shell(true),
        &["/bin/sh", "-c", "abc def"],
    );
    assert_argv(
        linux_builder().add_args(&["abc", "def"]).use_shell(true),
        &["/bin/sh", "-c", "abc def"],
    );
    assert_argv(
        linux_builder()
            .add_args(&["/bin/sh", "-c", "abc"])
            .use_shell(true),
        &["/bin/sh", "-c", "abc"],
    );
    assert_argv(
        linux_builder().add_args(&["/bin/sh", "-c"]),
        &["/bin/sh", "-c"],
    );
    assert_argv(
        linux_builder().add_args(&["/bin/bash", "-c"]),
        &["/bin/bash", "-c"],
    );
    assert_argv(
        linux_builder().add_args(&["/bin/sh", "-c"]).use_shell(true),
        &["/bin/sh", "-c"],
    );
    assert_argv(
        linux_builder()
            .add_args(&["/bin/bash", "-c"])
            .use_shell(true),
        &["/bin/bash", "-c"],
    );
}

#[test]
fn windows_builder_test() {
    assert_argv(win_builder().add_arg("abc.exe"), &["abc.exe"]);
    assert_argv(win_builder().add_arg("abc.exe -o"), &["abc.exe -o"]);
    assert_argv(win_builder().add_arg("ABC.EXE"), &["ABC.EXE"]);
    assert_win_cmd_argv(win_builder().add_arg("abc def.exe"), "abc def.exe");
    assert_argv(
        win_builder().add_args(&["abc.exe", "def"]),
        &["abc.exe", "def"],
    );
    assert_argv(
        win_builder().add_args(&["abc.exe".to_string(), "def".to_string()]),
        &["abc.exe", "def"],
    );
    assert_win_cmd_argv(
        win_builder().add_args(&["abc.exe", "def"]).use_shell(true),
        "abc.exe def",
    );
    assert_win_cmd_argv(win_builder().add_arg("abc"), "abc");
    assert_win_cmd_argv(win_builder().add_args(&["abc", "def"]), "abc def");
    assert_win_cmd_argv(
        win_builder().add_args(&["/bin/sh", "-c", "abc", "def"]),
        "abc def",
    );
    assert_win_cmd_argv(win_builder().add_args(&["/bin/sh", "-c"]), "");
    assert_win_cmd_argv(win_builder().add_args(&["/bin/bash", "-c"]), "");
    assert_win_cmd_argv(
        win_builder().add_args(&["/bin/sh", "-c"]).use_shell(true),
        "",
    );
    assert_win_cmd_argv(
        win_builder().add_args(&["/bin/bash", "-c"]).use_shell(true),
        "",
    );
}

#[test]
fn failure_scenarios() {
    assert_failure(linux_builder(), "At least one argument is expected");
    let mut unknown = CommandBuilder::with_os(OS::Unknown, &IndexMap::new());
    unknown.use_temp_dir().add_arg("a");
    assert_failure(unknown, "Unidentified operating system");
    let mut no_dir = CommandBuilder::with_os(OS::Linux, &IndexMap::new());
    no_dir.add_arg("a");
    assert_failure(no_dir, "Working directory must be set");
}
