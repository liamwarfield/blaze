//! Port of `src/test/java/com/google/devtools/build/lib/util/ShellEscaperTest.java`.

use std::collections::BTreeSet;

use blaze_shell::ShellEscaper;

fn escape_string(s: &str) -> String {
    ShellEscaper::escape_string(s)
}

#[test]
fn shell_escape() {
    assert_eq!(escape_string(""), "''");
    assert_eq!(escape_string("foo"), "foo");
    assert_eq!(escape_string("foo bar"), "'foo bar'");
    assert_eq!(escape_string("'foo'"), "''\\''foo'\\'''");
    assert_eq!(escape_string("\\'foo\\'"), "'\\'\\''foo\\'\\'''");
    assert_eq!(escape_string("${filename%.c}.o"), "'${filename%.c}.o'");
    assert_eq!(escape_string("<html!>"), "'<html!>'");
    assert_eq!(escape_string("~not_home"), "'~not_home'");
    assert_eq!(
        escape_string("external/protobuf+3.19.6/src/goo~gle"),
        "external/protobuf+3.19.6/src/goo~gle"
    );
    assert_eq!(
        escape_string("external/+install_dev_dependencies+foo/pkg"),
        "external/+install_dev_dependencies+foo/pkg"
    );
}

#[test]
fn escape_all() {
    let escaped: BTreeSet<String> = ShellEscaper::escape_all(&["foo", "@bar", "baz'qux"])
        .into_iter()
        .collect();
    let expected: BTreeSet<String> = ["foo", "@bar", "'baz'\\''qux'"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    assert_eq!(escaped, expected);
}

#[test]
fn escape_join_all_into_appendable() {
    let mut appendable = String::from("initial");
    ShellEscaper::escape_join_all_into(&mut appendable, &["foo", "$BAR"]);
    assert_eq!(appendable, "initialfoo '$BAR'");
}

#[test]
fn escape_join_all_into_appendable_with_custom_joiner() {
    let mut appendable = String::from("initial");
    ShellEscaper::escape_join_all_into_with(&mut appendable, &["foo", "$BAR"], "|");
    assert_eq!(appendable, "initialfoo|'$BAR'");
}

const ARGS: [&str; 8] = [
    "foo",
    "@echo:-",
    "100",
    "$US",
    "a b",
    "\"qu'ot'es\"",
    "\"quot\"",
    "\\",
];

#[test]
fn escape_join_all() {
    let actual = ShellEscaper::escape_join_all(&ARGS);
    assert_eq!(
        actual,
        "foo @echo:- 100 '$US' 'a b' '\"qu'\\''ot'\\''es\"' '\"quot\"' '\\'"
    );
}

#[test]
fn escape_join_all_with_custom_joiner() {
    let actual = ShellEscaper::escape_join_all_with(&ARGS, "|");
    assert_eq!(
        actual,
        "foo|@echo:-|100|'$US'|'a b'|'\"qu'\\''ot'\\''es\"'|'\"quot\"'|'\\'"
    );
}
