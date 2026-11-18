//! Port of `src/test/java/com/google/devtools/build/lib/shell/ShellUtilsWithBashTest.java`.

mod common;

use blaze_shell::{Command, ShellUtils};

fn assert_tokenize_is_dual_to_pretty_print(args: &[&str]) {
    let input: Vec<String> = args.iter().map(|s| s.to_string()).collect();
    let shell_command = ShellUtils::pretty_print_argv(&input);

    // Assert that pretty-print is correct, i.e. dual to the actual /bin/sh tokenization. This
    // test assumes no newlines in the input:
    let exec_args = vec![
        "/bin/sh".to_string(),
        "-c".to_string(),
        format!("for i in {shell_command}; do echo \"$i\"; done"), // tokenize, one word per line
    ];
    let mut stdout = match Command::new(exec_args, &common::getenv()).execute() {
        Ok(r) => String::from_utf8(r.get_stdout()).unwrap(),
        Err(e) => panic!("/bin/sh failed:\n{input:?}\n{shell_command}\n{e}"),
    };
    let mut words = Vec::new();
    while let Some(index) = stdout.find('\n') {
        words.push(stdout[..index].to_string());
        stdout = stdout[index + 1..].to_string();
    }
    assert_eq!(words, input);

    // Assert that tokenize is dual to pretty-print:
    let mut out = Vec::new();
    ShellUtils::tokenize(&mut out, &shell_command).unwrap();
    assert_eq!(out, input);
}

#[test]
fn test_tokenize_is_dual_to_pretty_print() {
    assert_tokenize_is_dual_to_pretty_print(&["foo"]);
    assert_tokenize_is_dual_to_pretty_print(&["foo bar"]);
    assert_tokenize_is_dual_to_pretty_print(&["foo bar", "wiz"]);
    assert_tokenize_is_dual_to_pretty_print(&["'foo'"]);
    assert_tokenize_is_dual_to_pretty_print(&["\\'foo\\'"]);
    assert_tokenize_is_dual_to_pretty_print(&["${filename%.c}.o"]);
    assert_tokenize_is_dual_to_pretty_print(&["<html!>"]);

    assert_tokenize_is_dual_to_pretty_print(&[""]);
    assert_tokenize_is_dual_to_pretty_print(&["!@#$%^&*()"]);
    assert_tokenize_is_dual_to_pretty_print(&["x'y\" z"]);
}
