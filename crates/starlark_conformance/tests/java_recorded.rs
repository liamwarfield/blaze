//! Runs the Starlark files in `tests/data/*.star` on starlark-rust and compares what they print
//! with what Bazel 9.3.2's own interpreter printed (`<name>.latin1.txt`, `<name>.unicode.txt`,
//! recorded with `java/StarlarkProbe.java` on the oracle's A-server.jar; see its header).
//!
//! `latin1` is how Bazel and Blaze read every Starlark file (one char per byte, the byte-strings
//! semantics flag on); `unicode` (the flag off) is only compared for `latin1_chars.star`, whose
//! source is ASCII, so its strings are the same chars in both interpreters.

use std::path::Path;
use std::path::PathBuf;

use starlark_conformance::script_test::ScriptMode;

fn data_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/data")
}

/// Runs `name` (a file in tests/data) as the probe does; returns the printed lines.
fn run(name: &str, mode: ScriptMode) -> Vec<String> {
    let bytes = std::fs::read(data_dir().join(name)).unwrap();
    starlark_conformance::probe::run(name, &bytes, mode)
}

fn expected(name: &str, mode: ScriptMode) -> Vec<String> {
    let stem = name.strip_suffix(".star").unwrap();
    let suffix = match mode {
        ScriptMode::Latin1 => "latin1",
        ScriptMode::Unicode => "unicode",
    };
    std::fs::read_to_string(data_dir().join(format!("{stem}.{suffix}.txt")))
        .unwrap()
        .lines()
        .map(str::to_owned)
        .collect()
}

/// Every differing line, as `file:line: java | rust`.
fn compare(name: &str, mode: ScriptMode) -> Vec<String> {
    let want = expected(name, mode);
    let got = run(name, mode);
    let mut bad = Vec::new();
    for i in 0..want.len().max(got.len()) {
        let w = want.get(i).map(String::as_str).unwrap_or("<none>");
        let g = got.get(i).map(String::as_str).unwrap_or("<none>");
        if w != g {
            bad.push(format!(
                "{name}:{}: java {w}\n{:>width$} rust {g}",
                i + 1,
                "",
                width = name.len()
            ));
        }
    }
    bad
}

fn star_files() -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(data_dir())
        .unwrap()
        .filter_map(|e| {
            let n = e.unwrap().file_name().into_string().unwrap();
            n.ends_with(".star").then_some(n)
        })
        .collect();
    v.sort();
    v
}

#[test]
fn java_recorded_latin1() {
    let files = star_files();
    assert!(files.len() >= 13, "{files:?}");
    let bad: Vec<String> = files
        .iter()
        .flat_map(|f| compare(f, ScriptMode::Latin1))
        .collect();
    assert!(bad.is_empty(), "differences from Java:\n{}", bad.join("\n"));
}

#[test]
fn java_recorded_unicode_latin1_chars() {
    let bad = compare("latin1_chars.star", ScriptMode::Unicode);
    assert!(bad.is_empty(), "differences from Java:\n{}", bad.join("\n"));
    assert!(Path::new(&data_dir().join("latin1_chars.unicode.txt")).exists());
}

/// Cases of `tests/data/interp` whose error text is Bazel's in Blaze too (texts Blaze ports:
/// `fail`, `int()` parsing, the lexer's tab and number-literal errors). Elsewhere the error text
/// is starlark-rust's (accepted, docs/decisions.md) and only its kind is compared.
const INTERP_TEXT_COMPARED: &[&str] = &[
    "lex_tab_",
    "lex_binary_literal",
    "lex_float_too_large",
    "lex_int_00",
    "str_fail_",
    "str_int_",
    "more_i_base0_dec",
    "more_i_base_neg",
    "more_i_big_base0_octal",
    "more_i_double_sign",
    "more_i_float_str",
    "more_i_only_prefix",
    "more_i_sign_after_prefix",
    "more_i_space_inner",
];

/// `tests/data/interp/NAME.star` against `NAME.latin1.txt`, recorded on Bazel's interpreter by
/// `scripts/starlark/interp-diff record-java`. Printed lines must be equal; then Java's first
/// error line (Java reports every syntax error of a file, starlark-rust the first) must have the
/// same kind (`SYNTAX`, a lexer/parser/resolver error, or `EVAL`), and for the cases of
/// [`INTERP_TEXT_COMPARED`] the same text.
fn compare_interp(name: &str) -> Vec<String> {
    let dir = data_dir().join("interp");
    let bytes = std::fs::read(dir.join(name)).unwrap();
    let got = starlark_conformance::probe::run(name, &bytes, ScriptMode::Latin1);
    let stem = name.strip_suffix(".star").unwrap();
    let want: Vec<String> = std::fs::read_to_string(dir.join(format!("{stem}.latin1.txt")))
        .unwrap()
        .lines()
        .map(str::to_owned)
        .collect();
    let is_error = |l: &String| l.starts_with("SYNTAX ") || l.starts_with("EVAL ");
    let split = |v: &[String]| {
        let n = v.iter().position(is_error).unwrap_or(v.len());
        (v[..n].to_vec(), v.get(n).cloned())
    };
    let (want_out, want_err) = split(&want);
    let (got_out, got_err) = split(&got);
    let text = INTERP_TEXT_COMPARED.iter().any(|p| stem.starts_with(p));
    let same_err = match (&want_err, &got_err) {
        (None, None) => true,
        (Some(w), Some(g)) if text => w == g,
        (Some(w), Some(g)) => w.split(' ').next() == g.split(' ').next(),
        _ => false,
    };
    if want_out == got_out && same_err {
        Vec::new()
    } else {
        vec![format!("{name}:\n  java {want:?}\n  rust {got:?}")]
    }
}

#[test]
fn java_recorded_interp() {
    let mut files: Vec<String> = std::fs::read_dir(data_dir().join("interp"))
        .unwrap()
        .filter_map(|e| {
            let n = e.unwrap().file_name().into_string().unwrap();
            n.ends_with(".star").then_some(n)
        })
        .collect();
    files.sort();
    assert!(files.len() >= 140, "{files:?}");
    let bad: Vec<String> = files.iter().flat_map(|f| compare_interp(f)).collect();
    assert!(
        bad.is_empty(),
        "{} of {} cases differ from Java:\n{}",
        bad.len(),
        files.len(),
        bad.join("\n")
    );
}
