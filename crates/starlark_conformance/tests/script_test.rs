//! Runs Bazel's Starlark script tests (ScriptTest.java testdata) on starlark-rust and prints the
//! per-file table, in both of Bazel's modes (`ScriptTest` and `ScriptTest_Latin1`). The numbers
//! are a ratchet: the test fails if they get worse.
//! Run `scripts/cargo-capped run -p starlark_conformance -- run [--latin1] -v` for every failure
//! with its category.

use starlark_conformance::script_test;
use starlark_conformance::script_test::ScriptMode;

/// (chunks passed, asserts passed) of one mode, after printing its table.
fn run(mode: ScriptMode) -> Option<(usize, usize)> {
    let dir = script_test::default_testdata_dir();
    if !dir.exists() {
        eprintln!("skipping: {} not found", dir.display());
        return None;
    }
    let results = script_test::run_dir(&dir, mode).unwrap();
    eprintln!("{mode:?}:\n{}", script_test::summary_table(&results));
    assert_eq!(results.len(), 38, "Bazel 9.3.2 has 38 testdata files");
    let chunks: usize = results.iter().map(|r| r.chunks.len()).sum();
    assert_eq!(chunks, 95);
    let chunks_passed: usize = results.iter().map(|r| r.chunks_passed()).sum();
    let asserts_passed: usize = results.iter().map(|r| r.asserts_passed()).sum();
    Some((chunks_passed, asserts_passed))
}

#[test]
fn bazel_script_tests_on_starlark_rust() {
    let Some((chunks_passed, asserts_passed)) = run(ScriptMode::Unicode) else {
        return;
    };
    // Ratchet: patches through 0036 (PATCHES.md conformance table; the stock starlark 0.14.2
    // baseline was 28 chunks and 2060 assertions).
    assert!(
        chunks_passed >= RATCHET_UNICODE.0,
        "chunks passed regressed: {chunks_passed}"
    );
    assert!(
        asserts_passed >= RATCHET_UNICODE.1,
        "asserts passed regressed: {asserts_passed}"
    );
}

/// Bazel's `ScriptTest_Latin1` target: how Blaze reads every Starlark file.
#[test]
fn bazel_script_tests_on_starlark_rust_latin1() {
    let Some((chunks_passed, asserts_passed)) = run(ScriptMode::Latin1) else {
        return;
    };
    assert!(
        chunks_passed >= RATCHET_LATIN1.0,
        "chunks passed regressed: {chunks_passed}"
    );
    assert!(
        asserts_passed >= RATCHET_LATIN1.1,
        "asserts passed regressed: {asserts_passed}"
    );
}

/// (chunks, asserts) passed in `ScriptMode::Unicode`.
const RATCHET_UNICODE: (usize, usize) = (37, 2494);
/// (chunks, asserts) passed in `ScriptMode::Latin1`.
const RATCHET_LATIN1: (usize, usize) = (38, 2495);
