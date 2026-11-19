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

/// `tests/data/interp/KNOWN_DIFFERENCES`: case name -> `text` (printed output and the kind of
/// the first error must still match) or `kind` (may differ entirely).
fn known_differences() -> std::collections::BTreeMap<String, String> {
    std::fs::read_to_string(data_dir().join("interp/KNOWN_DIFFERENCES"))
        .unwrap()
        .lines()
        .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
        .map(|l| {
            let mut it = l.split_whitespace();
            (
                it.next().unwrap().to_owned(),
                it.next().unwrap_or("text").to_owned(),
            )
        })
        .collect()
}

/// `tests/data/interp/NAME.star` against `NAME.latin1.txt`, recorded on Bazel's interpreter by
/// `scripts/starlark/interp-diff record-java`: every line must be equal (printed output, every
/// `SYNTAX` line, the `EVAL` message and Bazel's traceback as `TRACE` lines), except for the
/// cases listed in `KNOWN_DIFFERENCES`, which must still differ (a case that matches must be
/// removed from the list) and, when listed as `text`, match in printed output and the kind of
/// the first error.
#[test]
fn java_recorded_interp() {
    use starlark_conformance::probe::CaseResult;
    let dir = data_dir().join("interp");
    let files = starlark_conformance::probe::case_names(&dir);
    assert!(files.len() >= 500, "{files:?}");
    let known = known_differences();
    let mut bad = Vec::new();
    for f in &files {
        let c = starlark_conformance::probe::compare_case(&dir, f);
        let ok = match (known.get(f).map(String::as_str), &c.result) {
            (None, CaseResult::Match) => true,
            (Some(_), CaseResult::Match) => {
                bad.push(format!(
                    "{f}: matches Bazel now, remove it from KNOWN_DIFFERENCES"
                ));
                continue;
            }
            (Some("kind"), _) => true,
            (Some(_), CaseResult::TextDiffers) => true,
            _ => false,
        };
        if !ok {
            bad.push(format!("{f}:\n  java {:?}\n  rust {:?}", c.java, c.rust));
        }
    }
    for k in known.keys() {
        if !files.contains(k) {
            bad.push(format!(
                "{k}: listed in KNOWN_DIFFERENCES but there is no such case"
            ));
        }
    }
    assert!(
        bad.is_empty(),
        "{} of {} cases differ from Java:\n{}",
        bad.len(),
        files.len(),
        bad.join("\n")
    );
}
