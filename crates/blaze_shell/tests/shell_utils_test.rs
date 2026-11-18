//! Port of `src/test/java/com/google/devtools/build/lib/shell/ShellUtilsTest.java`.

use blaze_shell::ShellUtils;

fn shell_escape(s: &str) -> String {
    ShellUtils::shell_escape(s)
}

#[test]
fn test_shell_escape() {
    assert_eq!(shell_escape(""), "''");
    assert_eq!(shell_escape("foo"), "foo");
    assert_eq!(shell_escape("foo bar"), "'foo bar'");
    assert_eq!(shell_escape("'foo'"), "''\\''foo'\\'''");
    assert_eq!(shell_escape("\\'foo\\'"), "'\\'\\''foo\\'\\'''");
    assert_eq!(shell_escape("${filename%.c}.o"), "'${filename%.c}.o'");
    assert_eq!(shell_escape("<html!>"), "'<html!>'");
}

#[test]
fn test_pretty_print_argv() {
    assert_eq!(
        ShellUtils::pretty_print_argv(&["echo", "$US", "100"]),
        "echo '$US' 100"
    );
}

fn assert_tokenize(copts: &str, expected_tokens: &[&str]) {
    let mut actual_tokens = Vec::new();
    ShellUtils::tokenize(&mut actual_tokens, copts).unwrap();
    assert_eq!(actual_tokens, expected_tokens);
}

#[test]
fn test_tokenize() {
    assert_tokenize("-DASMV", &["-DASMV"]);
    assert_tokenize("-DNO_UNDERLINE", &["-DNO_UNDERLINE"]);
    assert_tokenize("-DASMV -DNO_UNDERLINE", &["-DASMV", "-DNO_UNDERLINE"]);
    assert_tokenize(
        "-DDES_LONG=\"unsigned int\" -wd310",
        &["-DDES_LONG=unsigned int", "-wd310"],
    );
    assert_tokenize(
        "-Wno-write-strings -Wno-pointer-sign -Wno-unused-variable -Wno-pointer-to-int-cast",
        &[
            "-Wno-write-strings",
            "-Wno-pointer-sign",
            "-Wno-unused-variable",
            "-Wno-pointer-to-int-cast",
        ],
    );
}

#[test]
fn test_tokenize_on_nested_quotation() {
    assert_tokenize("-Dfoo='foo\"bar' -Dwiz", &["-Dfoo=foo\"bar", "-Dwiz"]);
    assert_tokenize("-Dfoo=\"foo'bar\" -Dwiz", &["-Dfoo=foo'bar", "-Dwiz"]);
}

#[test]
fn test_tokenize_on_backslash_escapes() {
    assert_tokenize("-Dfoo=\\'foo -Dbar", &["-Dfoo='foo", "-Dbar"]);
    assert_tokenize("-Dfoo=\\\"foo -Dbar", &["-Dfoo=\"foo", "-Dbar"]);
    assert_tokenize("-Dfoo=\\\\foo -Dbar", &["-Dfoo=\\foo", "-Dbar"]);

    assert_tokenize("-Dfoo='\\'foo -Dbar", &["-Dfoo=\\foo", "-Dbar"]);
    assert_tokenize("-Dfoo='\\\"foo' -Dbar", &["-Dfoo=\\\"foo", "-Dbar"]);
    assert_tokenize("-Dfoo='\\\\foo' -Dbar", &["-Dfoo=\\\\foo", "-Dbar"]);

    assert_tokenize("-Dfoo=\"\\'foo\" -Dbar", &["-Dfoo=\\'foo", "-Dbar"]);
    assert_tokenize("-Dfoo=\"\\\"foo\" -Dbar", &["-Dfoo=\"foo", "-Dbar"]);
    assert_tokenize("-Dfoo=\"\\\\foo\" -Dbar", &["-Dfoo=\\foo", "-Dbar"]);
}

fn assert_tokenize_fails(copts: &str, expected_error: &str) {
    let e = ShellUtils::tokenize(&mut Vec::new(), copts).unwrap_err();
    assert_eq!(e.get_message(), expected_error);
}

#[test]
fn test_tokenize_empty_string() {
    assert_tokenize("", &[]);
}

#[test]
fn test_tokenize_fails_on_unterminated_quotation() {
    assert_tokenize_fails("-Dfoo=\"bar", "unterminated quotation");
    assert_tokenize_fails("-Dfoo='bar", "unterminated quotation");
    assert_tokenize_fails("-Dfoo=\"b'ar", "unterminated quotation");
}

fn assert_windows_escape_arg(arg: &str, expected: &str) {
    assert_eq!(ShellUtils::windows_escape_arg(arg), expected, "arg: {arg}");
}

#[test]
fn test_escape_create_process_arg() {
    assert_windows_escape_arg("", "\"\"");
    assert_windows_escape_arg(" ", "\" \"");
    assert_windows_escape_arg("\"", "\"\\\"\"");
    assert_windows_escape_arg("\"\\", "\"\\\"\\\\\"");
    assert_windows_escape_arg("\\", "\\");
    assert_windows_escape_arg("\\\"", "\"\\\\\\\"\"");
    assert_windows_escape_arg("with space", "\"with space\"");
    assert_windows_escape_arg("with^caret", "with^caret");
    assert_windows_escape_arg("space ^caret", "\"space ^caret\"");
    assert_windows_escape_arg("caret^ space", "\"caret^ space\"");
    assert_windows_escape_arg("with\"quote", "\"with\\\"quote\"");
    assert_windows_escape_arg("with\\backslash", "with\\backslash");
    assert_windows_escape_arg(
        "one\\ backslash and \\space",
        "\"one\\ backslash and \\space\"",
    );
    assert_windows_escape_arg("two\\\\backslashes", "two\\\\backslashes");
    assert_windows_escape_arg(
        "two\\\\ backslashes \\\\and space",
        "\"two\\\\ backslashes \\\\and space\"",
    );
    assert_windows_escape_arg("one\\\"x", "\"one\\\\\\\"x\"");
    assert_windows_escape_arg("two\\\\\"x", "\"two\\\\\\\\\\\"x\"");
    assert_windows_escape_arg("a \\ b", "\"a \\ b\"");
    assert_windows_escape_arg("a \\\" b", "\"a \\\\\\\" b\"");
    assert_windows_escape_arg("A", "A");
    assert_windows_escape_arg("\"a\"", "\"\\\"a\\\"\"");
    assert_windows_escape_arg("B C", "\"B C\"");
    assert_windows_escape_arg("\"b c\"", "\"\\\"b c\\\"\"");
    assert_windows_escape_arg("D\"E", "\"D\\\"E\"");
    assert_windows_escape_arg("\"d\"e\"", "\"\\\"d\\\"e\\\"\"");
    assert_windows_escape_arg("C:\\F G", "\"C:\\F G\"");
    assert_windows_escape_arg("\"C:\\f g\"", "\"\\\"C:\\f g\\\"\"");
    assert_windows_escape_arg("C:\\H\"I", "\"C:\\H\\\"I\"");
    assert_windows_escape_arg("\"C:\\h\"i\"", "\"\\\"C:\\h\\\"i\\\"\"");
    assert_windows_escape_arg("C:\\J\\\"K", "\"C:\\J\\\\\\\"K\"");
    assert_windows_escape_arg("\"C:\\j\\\"k\"", "\"\\\"C:\\j\\\\\\\"k\\\"\"");
    assert_windows_escape_arg("C:\\L M ", "\"C:\\L M \"");
    assert_windows_escape_arg("\"C:\\l m \"", "\"\\\"C:\\l m \\\"\"");
    assert_windows_escape_arg("C:\\N O\\", "\"C:\\N O\\\\\"");
    assert_windows_escape_arg("\"C:\\n o\\\"", "\"\\\"C:\\n o\\\\\\\"\"");
    assert_windows_escape_arg("C:\\P Q\\ ", "\"C:\\P Q\\ \"");
    assert_windows_escape_arg("\"C:\\p q\\ \"", "\"\\\"C:\\p q\\ \\\"\"");
    assert_windows_escape_arg("C:\\R\\S\\", "C:\\R\\S\\");
    assert_windows_escape_arg("C:\\R x\\S\\", "\"C:\\R x\\S\\\\\"");
    assert_windows_escape_arg("\"C:\\r\\s\\\"", "\"\\\"C:\\r\\s\\\\\\\"\"");
    assert_windows_escape_arg("\"C:\\r x\\s\\\"", "\"\\\"C:\\r x\\s\\\\\\\"\"");
    assert_windows_escape_arg("C:\\T U\\W\\", "\"C:\\T U\\W\\\\\"");
    assert_windows_escape_arg("\"C:\\t u\\w\\\"", "\"\\\"C:\\t u\\w\\\\\\\"\"");
    assert_windows_escape_arg("\"a", "\"\\\"a\"");
}
