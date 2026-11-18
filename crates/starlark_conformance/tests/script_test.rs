//! Runs Bazel's Starlark script tests (ScriptTest.java testdata) on starlark-rust and prints the
//! per-file table. The numbers are a ratchet: the test fails if they get worse.
//! Run `cargo run -p starlark_conformance -- run -v` for every failure with its category.

use starlark_conformance::script_test;

#[test]
fn bazel_script_tests_on_starlark_rust() {
    let dir = script_test::default_testdata_dir();
    if !dir.exists() {
        eprintln!("skipping: {} not found", dir.display());
        return;
    }
    let results = script_test::run_dir(&dir).unwrap();
    eprintln!("{}", script_test::summary_table(&results));
    assert_eq!(results.len(), 38, "Bazel 9.3.2 has 38 testdata files");
    let chunks_passed: usize = results.iter().map(|r| r.chunks_passed()).sum();
    let chunks: usize = results.iter().map(|r| r.chunks.len()).sum();
    let asserts_passed: usize = results.iter().map(|r| r.asserts_passed()).sum();
    assert_eq!(chunks, 95);
    // Baseline at starlark 0.14.2 (see docs/research/05-starlark-rust.md).
    assert!(
        chunks_passed >= 28,
        "chunks passed regressed: {chunks_passed}"
    );
    assert!(
        asserts_passed >= 2060,
        "asserts passed regressed: {asserts_passed}"
    );
}
