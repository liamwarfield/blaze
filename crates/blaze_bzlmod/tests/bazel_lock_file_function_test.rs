//! Behaviour tests of `BazelLockFileFunction` (BAZEL_LOCK_FILE) and
//! `BazelLockFileModule.afterCommand` through the Skyframe functions.
//!
//! Bazel 9.3.2 has no JUnit test for `BazelLockFileFunction` (`BazelLockFileModuleTest` is ported
//! in `blaze_bzlmod_model`); the expected values follow the Java of `BazelLockFileFunction.compute`,
//! `getLockfileValue` and `BazelLockFileModule.afterCommand`.

mod skyframe_common;

use blaze_bzlmod::skyframe::{
    BazelDepGraphKey, BazelLockFileKey, BazelModuleResolutionKey, after_command,
};
use blaze_bzlmod_model::{
    BazelDepGraphValue, BazelLockFileValue, BazelModuleResolutionValue, Checksum, ExternalDepsCode,
    ExternalDepsException, LockfileMode,
};
use blaze_skyframe::{EvaluationResult, EvaluatorBuilder, SkyKey};
use indexmap::IndexMap;
use skyframe_common::*;

fn evaluate_lockfile(h: &Harness, key: BazelLockFileKey) -> EvaluationResult {
    h.evaluate(&[SkyKey::new(key)])
}

fn lockfile_exception(result: &EvaluationResult) -> ExternalDepsException {
    result
        .get_first_error()
        .expect("an error")
        .exception_as::<ExternalDepsException>()
        .expect("an ExternalDepsException")
        .clone()
}

#[test]
fn missing_lockfile_is_empty() {
    let h = Harness::new();
    for key in [BazelLockFileKey::KEY, BazelLockFileKey::HIDDEN_KEY] {
        let result = evaluate_lockfile(&h, key);
        assert_no_error(&result);
        assert_eq!(
            *result.get(&key).unwrap(),
            BazelLockFileValue::empty_lockfile()
        );
    }
}

#[test]
fn lockfile_mode_off_ignores_the_file() {
    let mut h = Harness::new();
    h.lockfile_mode = LockfileMode::Off;
    h.scratch_file("MODULE.bazel.lock", &["{\"lockFileVersion\": 28, garbage"]);
    let result = evaluate_lockfile(&h, BazelLockFileKey::KEY);
    assert_no_error(&result);
    assert_eq!(
        *result.get(&BazelLockFileKey::KEY).unwrap(),
        BazelLockFileValue::empty_lockfile()
    );
}

#[test]
fn old_lockfile_version_is_ignored_in_update_mode() {
    let h = Harness::new();
    h.scratch_file("MODULE.bazel.lock", &["{\"lockFileVersion\": 3}"]);
    let result = evaluate_lockfile(&h, BazelLockFileKey::KEY);
    assert_no_error(&result);
    assert_eq!(
        *result.get(&BazelLockFileKey::KEY).unwrap(),
        BazelLockFileValue::empty_lockfile()
    );
}

#[test]
fn old_lockfile_version_fails_in_error_mode() {
    let mut h = Harness::new();
    h.lockfile_mode = LockfileMode::Error;
    h.scratch_file("MODULE.bazel.lock", &["{\"lockFileVersion\": 3}"]);
    let result = evaluate_lockfile(&h, BazelLockFileKey::KEY);
    assert!(result.has_error());
    let e = lockfile_exception(&result);
    assert_eq!(e.get_code(), ExternalDepsCode::BadLockfile);
    assert_eq!(
        e.get_message(),
        "The version of MODULE.bazel.lock is not supported by this version of Bazel. Please run \
         `bazel mod deps --lockfile_mode=update` to update your lockfile."
    );
}

#[test]
fn unparsable_lockfile_fails() {
    let h = Harness::new();
    h.scratch_file(
        "MODULE.bazel.lock",
        &["{\"lockFileVersion\": 28, \"registryFileHashes\": [}"],
    );
    let result = evaluate_lockfile(&h, BazelLockFileKey::KEY);
    assert!(result.has_error());
    let e = lockfile_exception(&result);
    assert_eq!(e.get_code(), ExternalDepsCode::BadLockfile);
    assert!(
        e.get_message()
            .starts_with("Failed to read and parse the MODULE.bazel.lock file with error: "),
        "{}",
        e.get_message()
    );
    assert!(
        e.get_message()
            .ends_with(". Try deleting it and rerun the build."),
        "{}",
        e.get_message()
    );
}

#[test]
fn unparsable_hidden_lockfile_is_empty() {
    let h = Harness::new();
    write_file(
        &h.output_base.join("MODULE.bazel.lock"),
        "{\"lockFileVersion\": 28, \"registryFileHashes\": [}",
    );
    let result = evaluate_lockfile(&h, BazelLockFileKey::HIDDEN_KEY);
    assert_no_error(&result);
    assert_eq!(
        *result.get(&BazelLockFileKey::HIDDEN_KEY).unwrap(),
        BazelLockFileValue::empty_lockfile()
    );
}

#[test]
fn lockfile_round_trips_through_the_function() {
    let h = Harness::new();
    let mut value = BazelLockFileValue::empty_lockfile();
    value.registry_file_hashes.insert(
        "https://bcr.bazel.build/bazel_registry.json".to_string(),
        Some(Checksum::sha256_of(b"{}")),
    );
    value.registry_file_hashes.insert(
        "https://bcr.bazel.build/modules/missing/1.0/MODULE.bazel".to_string(),
        None,
    );
    write_file(
        &h.root_directory.join("MODULE.bazel.lock"),
        &value.to_lockfile_contents(),
    );
    let result = evaluate_lockfile(&h, BazelLockFileKey::KEY);
    assert_no_error(&result);
    assert_eq!(*result.get(&BazelLockFileKey::KEY).unwrap(), value);
}

fn resolution_value() -> BazelModuleResolutionValue {
    let dep_graph = BazelDepGraphValue::create_empty_dep_graph();
    BazelModuleResolutionValue {
        resolved_dep_graph: dep_graph.dep_graph.clone(),
        unpruned_dep_graph: IndexMap::new(),
        registry_file_hashes: IndexMap::from([
            (
                "https://b.example/modules/b/1.0/MODULE.bazel".to_string(),
                Some(Checksum::sha256_of(b"b")),
            ),
            (
                "file:///local/registry/modules/x/1.0/MODULE.bazel".to_string(),
                Some(Checksum::sha256_of(b"x")),
            ),
            (
                "https://a.example/modules/a/1.0/MODULE.bazel".to_string(),
                None,
            ),
        ]),
        selected_yanked_versions: IndexMap::from([(
            create_module_key("a", "1.0"),
            "bad".to_string(),
        )]),
    }
}

fn evaluator_with_values(old_lockfile: Option<BazelLockFileValue>) -> blaze_skyframe::Evaluator {
    let mut builder = EvaluatorBuilder::new()
        .inject(BazelModuleResolutionKey, resolution_value())
        .inject(
            BazelDepGraphKey,
            BazelDepGraphValue::create_empty_dep_graph(),
        );
    if let Some(old) = old_lockfile {
        builder = builder.inject(BazelLockFileKey::KEY, old);
    }
    builder.build()
}

/// The lockfile `afterCommand` writes for [`resolution_value`]: local (`file:`) registry hashes
/// dropped, the rest sorted by URL, the selected yanked versions kept.
fn expected_lockfile() -> BazelLockFileValue {
    let mut expected = BazelLockFileValue::empty_lockfile();
    expected.registry_file_hashes = IndexMap::from([
        (
            "https://a.example/modules/a/1.0/MODULE.bazel".to_string(),
            None,
        ),
        (
            "https://b.example/modules/b/1.0/MODULE.bazel".to_string(),
            Some(Checksum::sha256_of(b"b")),
        ),
    ]);
    expected.selected_yanked_versions =
        IndexMap::from([(create_module_key("a", "1.0"), "bad".to_string())]);
    expected
}

#[test]
fn after_command_writes_the_lockfile() {
    let h = Harness::new();
    let evaluator = evaluator_with_values(Some(BazelLockFileValue::empty_lockfile()));
    after_command(
        &evaluator,
        &h.root_directory,
        &h.output_base,
        LockfileMode::Update,
    );
    let written = std::fs::read_to_string(h.root_directory.join("MODULE.bazel.lock")).unwrap();
    let parsed = BazelLockFileValue::from_json(&written).unwrap().unwrap();
    assert_eq!(parsed, expected_lockfile());
    // Keys in sorted order (ImmutableSortedMap).
    assert_eq!(
        parsed.registry_file_hashes.keys().collect::<Vec<_>>(),
        expected_lockfile()
            .registry_file_hashes
            .keys()
            .collect::<Vec<_>>()
    );
    assert_eq!(written, expected_lockfile().to_lockfile_contents());
    // The hidden lockfile has nothing new (no extensions), so it is not written.
    assert!(!h.output_base.join("MODULE.bazel.lock").exists());
}

#[test]
fn after_command_skips_unchanged_lockfile() {
    let h = Harness::new();
    let evaluator = evaluator_with_values(Some(expected_lockfile()));
    after_command(
        &evaluator,
        &h.root_directory,
        &h.output_base,
        LockfileMode::Refresh,
    );
    assert!(!h.root_directory.join("MODULE.bazel.lock").exists());
}

#[test]
fn after_command_only_in_update_and_refresh_modes() {
    for mode in [LockfileMode::Off, LockfileMode::Error] {
        let h = Harness::new();
        let evaluator = evaluator_with_values(Some(BazelLockFileValue::empty_lockfile()));
        after_command(&evaluator, &h.root_directory, &h.output_base, mode);
        assert!(!h.root_directory.join("MODULE.bazel.lock").exists());
    }
}

#[test]
fn after_command_without_values_writes_nothing() {
    let h = Harness::new();
    // No lockfile value: an error must have occurred and been reported already.
    let evaluator = evaluator_with_values(None);
    after_command(
        &evaluator,
        &h.root_directory,
        &h.output_base,
        LockfileMode::Update,
    );
    assert!(!h.root_directory.join("MODULE.bazel.lock").exists());
    assert!(!h.output_base.join("MODULE.bazel.lock").exists());
}
