//! Byte-exact round trip of real MODULE.bazel.lock files written by Bazel 9.3.2 (the oracle):
//! parse into the model, serialize again, compare bytes.

use blaze_bzlmod_model::{BazelLockFileFunction, BazelLockFileValue, LockfileMode};

fn round_trip(name: &str) {
    let path = format!("{}/tests/data/{name}", env!("CARGO_MANIFEST_DIR"));
    let original = std::fs::read_to_string(&path).unwrap();
    let value = BazelLockFileValue::from_json(&original)
        .unwrap_or_else(|e| panic!("{name}: {e}"))
        .unwrap();
    let written = value.to_lockfile_contents();
    if written != original {
        let first_diff = written
            .bytes()
            .zip(original.bytes())
            .position(|(a, b)| a != b)
            .unwrap_or(written.len().min(original.len()));
        let lo = first_diff.saturating_sub(200);
        panic!(
            "{name}: not byte-identical (lengths {} vs {}), first difference at byte {first_diff}:\n--- written\n{}\n--- original\n{}",
            written.len(),
            original.len(),
            &written[lo..(first_diff + 200).min(written.len())],
            &original[lo..(first_diff + 200).min(original.len())]
        );
    }
    // Through the function, too.
    let via_function =
        BazelLockFileFunction::compute(Ok(Some(original.clone())), false, LockfileMode::Error)
            .unwrap();
    assert_eq!(via_function, value);
}

#[test]
fn brotli_lockfile_round_trips_byte_exact() {
    round_trip("brotli.MODULE.bazel.lock");
}

#[test]
fn re2_oracle_lockfile_round_trips_byte_exact() {
    round_trip("re2.MODULE.bazel.lock");
}

#[test]
fn bazel_source_lockfile_with_facts_round_trips_byte_exact() {
    round_trip("bazel-9.3.2.MODULE.bazel.lock");
}

#[test]
fn hidden_lockfile_with_metadata_round_trips_byte_exact() {
    round_trip("brotli-hidden.MODULE.bazel.lock");
}
