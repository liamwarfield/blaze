//! Byte-exact round trip of real MODULE.bazel.lock files written by Bazel 9.3.2 (the oracle):
//! parse into the model, serialize again, compare bytes.

use blaze_bzlmod_model::{
    AttributeValue, BazelLockFileFunction, BazelLockFileValue, FactValue, LockfileMode,
    ModuleExtensionEvalFactors, ModuleExtensionId,
};
use blaze_util::string_encoding::{java_new_string_utf8, latin1_decode};

fn round_trip(name: &str) {
    let path = format!("{}/tests/data/{name}", env!("CARGO_MANIFEST_DIR"));
    // BazelLockFileFunction: readContent(path, UTF_8).
    let original = java_new_string_utf8(&std::fs::read(&path).unwrap());
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

// `bytes-B1.MODULE.bazel.lock` was written by the oracle (Bazel 9.3.2) for the scratch workspace
// whose MODULE.bazel and ext.bzl are in crates/blaze_bzlmod/tests/data/bytes-B1: `bazel mod deps
// --lockfile_mode=update`, then `bazel mod show_repo @ext_repo @rule_repo
// --lockfile_mode=update` (which evaluates the extension). The extension's generated repo and
// its facts hold `é` (C3 A9), `🐷` (F0 9F 90 B7) and the byte FF.

#[test]
fn non_ascii_oracle_lockfile_round_trips_byte_exact() {
    round_trip("bytes-B1.MODULE.bazel.lock");
}

fn read_bytes_b1() -> (String, BazelLockFileValue) {
    let path = format!(
        "{}/tests/data/bytes-B1.MODULE.bazel.lock",
        env!("CARGO_MANIFEST_DIR")
    );
    let text = java_new_string_utf8(&std::fs::read(&path).unwrap());
    let value = BazelLockFileValue::from_json(&text).unwrap().unwrap();
    (text, value)
}

fn ext_id() -> ModuleExtensionId {
    ModuleExtensionId::parse_lockfile_form("//:ext.bzl%ext").unwrap()
}

/// The internal strings of the UTF-8 text `s`, as Starlark holds them.
fn internal(s: &str) -> String {
    latin1_decode(s.as_bytes()).into_owned()
}

#[test]
fn non_ascii_lockfile_values_are_read_as_internal_strings() {
    let (_, value) = read_bytes_b1();
    let ext = &value.module_extensions[&ext_id()][&ModuleExtensionEvalFactors::create("", "")];
    let attrs = ext.generated_repo_specs["ext_repo"].attributes();
    let e = internal("é");
    let pig = internal("🐷");
    assert_eq!(
        attrs.get("s"),
        Some(&AttributeValue::string(format!("{e}{pig}")))
    );
    assert_eq!(
        attrs.get("l"),
        Some(&AttributeValue::string_list([
            e.clone(),
            pig.clone(),
            format!("@@not_a_label_{e}")
        ]))
    );
    assert_eq!(
        attrs.get("d"),
        Some(&AttributeValue::string_dict([(e.clone(), pig.clone())]))
    );
    // The oracle wrote the byte FF (not UTF-8) as U+FFFD (internalToUnicode); it reads back as
    // the internal form of U+FFFD.
    assert_eq!(
        attrs.get("x"),
        Some(&AttributeValue::string("\u{ef}\u{bf}\u{bd}"))
    );

    // Facts are written without conversion (double encoded) and read back as they were.
    let facts = value.facts[&ext_id()].value();
    assert_eq!(
        facts,
        &[
            (
                "k".to_string(),
                FactValue::List(vec![
                    FactValue::String(e.clone()),
                    FactValue::String("\u{ff}".to_string())
                ])
            ),
            (e, FactValue::String(pig)),
        ]
    );
}

#[test]
fn non_ascii_lockfile_values_are_written_as_the_oracle_writes_them() {
    let (text, mut value) = read_bytes_b1();
    // What Starlark handed over: the byte FF itself (the oracle wrote it as U+FFFD).
    let factors = ModuleExtensionEvalFactors::create("", "");
    let ext = value
        .module_extensions
        .get_mut(&ext_id())
        .unwrap()
        .get_mut(&factors)
        .unwrap();
    let spec = ext.generated_repo_specs.get_mut("ext_repo").unwrap();
    let attrs: Vec<(String, AttributeValue)> = spec
        .attributes()
        .attributes()
        .iter()
        .map(|(k, v)| {
            if k == "x" {
                (k.clone(), AttributeValue::string("\u{ff}"))
            } else {
                (k.clone(), v.clone())
            }
        })
        .collect();
    *spec = blaze_bzlmod_model::RepoSpec::new(
        spec.repo_rule_id().unwrap().clone(),
        blaze_bzlmod_model::AttributeValues::create(attrs),
    );
    assert_eq!(value.to_lockfile_contents(), text);
    // The real characters, as UTF-8 bytes in the file.
    let bytes = value.to_lockfile_contents().into_bytes();
    let needle = "\"s\": \"é🐷\"".as_bytes();
    assert!(bytes.windows(needle.len()).any(|w| w == needle));
}
