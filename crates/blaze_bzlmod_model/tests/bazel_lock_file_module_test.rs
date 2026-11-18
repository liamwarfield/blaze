//! Port of `BazelLockFileModuleTest`, plus lockfile serialization and reading checks.

use blaze_bzlmod_model::facts::starlark_float_repr;
use blaze_bzlmod_model::{
    AttributeValue, AttributeValues, BazelLockFileFunction, BazelLockFileModule,
    BazelLockFileValue, ExternalDepsCode, FactValue, Facts, LockFileModuleExtension,
    LockFileModuleExtensionWithFactors, LockfileMode, LockfileModuleExtensionMetadata,
    ModuleExtensionEvalFactors, ModuleExtensionId, RepoSpec, UseAllRepos,
};
use blaze_cmdline::Label;
use indexmap::IndexMap;

struct Fixture {
    extension_id: ModuleExtensionId,
    non_reproducible_result: LockFileModuleExtension,
    reproducible_result: LockFileModuleExtension,
    eval_factors: ModuleExtensionEvalFactors,
    other_eval_factors: ModuleExtensionEvalFactors,
}

fn set_up() -> Fixture {
    let non_reproducible_result = LockFileModuleExtension {
        bzl_transitive_digest: vec![1, 2, 3],
        usages_digest: vec![4, 5, 6],
        recorded_inputs: Vec::new(),
        generated_repo_specs: IndexMap::new(),
        module_extension_metadata: None,
    };
    let mut reproducible_result = non_reproducible_result.clone();
    // LockfileModuleExtensionMetadata.of(ModuleExtensionMetadata.create(NONE, NONE, true, {}))
    reproducible_result.module_extension_metadata = Some(LockfileModuleExtensionMetadata {
        explicit_root_module_direct_deps: None,
        explicit_root_module_direct_dev_deps: None,
        use_all_repos: UseAllRepos::No,
        reproducible: true,
    });
    Fixture {
        extension_id: ModuleExtensionId::create(
            Label::parse_canonical_unchecked("//:ext.bzl"),
            "ext",
            None,
        ),
        non_reproducible_result,
        reproducible_result,
        eval_factors: ModuleExtensionEvalFactors::create("linux", "x86_64"),
        other_eval_factors: ModuleExtensionEvalFactors::create("linux", "aarch64"),
    }
}

#[test]
fn combine_module_extensions_reproducible_factor_added() {
    let f = set_up();
    let old = IndexMap::from([(
        f.extension_id.clone(),
        IndexMap::from([(f.eval_factors.clone(), f.non_reproducible_result.clone())]),
    )]);
    let new = IndexMap::from([(
        f.extension_id.clone(),
        LockFileModuleExtensionWithFactors {
            extension_factors: f.other_eval_factors.clone(),
            module_extension: f.reproducible_result.clone(),
        },
    )]);
    assert_eq!(
        BazelLockFileModule::combine_module_extensions(&old, &new, |_| true, false),
        old
    );
}

#[test]
fn combine_module_extensions_factor_becomes_reproducible() {
    let f = set_up();
    let old = IndexMap::from([(
        f.extension_id.clone(),
        IndexMap::from([(f.eval_factors.clone(), f.non_reproducible_result.clone())]),
    )]);
    let new = IndexMap::from([(
        f.extension_id.clone(),
        LockFileModuleExtensionWithFactors {
            extension_factors: f.eval_factors.clone(),
            module_extension: f.reproducible_result.clone(),
        },
    )]);
    assert!(BazelLockFileModule::combine_module_extensions(&old, &new, |_| true, false).is_empty());
}

#[test]
fn combine_module_extensions_factor_becomes_non_reproducible() {
    let f = set_up();
    let old = IndexMap::from([(
        f.extension_id.clone(),
        IndexMap::from([(f.eval_factors.clone(), f.reproducible_result.clone())]),
    )]);
    let new = IndexMap::from([(
        f.extension_id.clone(),
        LockFileModuleExtensionWithFactors {
            extension_factors: f.eval_factors.clone(),
            module_extension: f.non_reproducible_result.clone(),
        },
    )]);
    assert_eq!(
        BazelLockFileModule::combine_module_extensions(&old, &new, |_| true, false),
        IndexMap::from([(
            f.extension_id.clone(),
            IndexMap::from([(f.eval_factors.clone(), f.non_reproducible_result.clone())])
        )])
    );
}

fn temp_dir(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("blaze_bzlmod_model_{name}_{}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn update_lockfile_writes_json_with_trailing_newline() {
    let dir = temp_dir("update_lockfile");
    BazelLockFileModule::update_lockfile(&dir, &BazelLockFileValue::empty_lockfile()).unwrap();
    let content = std::fs::read_to_string(dir.join("MODULE.bazel.lock")).unwrap();
    assert_eq!(
        content,
        BazelLockFileValue::empty_lockfile().to_json() + "\n"
    );
    // The bytes Bazel 9.3.2 writes for an empty lockfile.
    assert_eq!(
        content,
        "{\n  \"lockFileVersion\": 28,\n  \"registryFileHashes\": {},\n  \
         \"selectedYankedVersions\": {},\n  \"moduleExtensions\": {},\n  \"facts\": {},\n  \
         \"factsVersions\": {}\n}\n"
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn update_lockfile_with_null_repo_rule_id_does_not_throw() {
    let f = set_up();
    let dir = temp_dir("null_repo_rule_id");
    let mut ext = f.non_reproducible_result.clone();
    ext.generated_repo_specs.insert(
        "repo".into(),
        RepoSpec::new_nullable(None, AttributeValues::default()),
    );
    let mut lockfile = BazelLockFileValue::empty_lockfile();
    lockfile.module_extensions.insert(
        f.extension_id.clone(),
        IndexMap::from([(f.eval_factors.clone(), ext)]),
    );
    BazelLockFileModule::update_lockfile(&dir, &lockfile).unwrap();
    assert!(dir.join("MODULE.bazel.lock").exists());
    std::fs::remove_dir_all(&dir).unwrap();
}

// The remaining tests are not in the Java test class; they pin Gson's output rules.

#[test]
fn attribute_values_serialization() {
    let f = set_up();
    let mut ext = f.non_reproducible_result.clone();
    ext.generated_repo_specs.insert(
        "r".into(),
        RepoSpec::new(
            blaze_bzlmod_model::repo_spec::HTTP_ARCHIVE.clone(),
            AttributeValues::create([
                ("none", AttributeValue::None),
                (
                    "label",
                    AttributeValue::Label(Label::parse_canonical_unchecked("@@foo+//:bar")),
                ),
                ("at_string", AttributeValue::string("@@not_a_label")),
                ("quoted", AttributeValue::string("'q'")),
                ("ctl", AttributeValue::string("a\u{1}b\t\"<>&='\u{2028}")),
                (
                    "list",
                    AttributeValue::List(vec![AttributeValue::None, AttributeValue::Int(-3)]),
                ),
                (
                    "dict",
                    AttributeValue::Dict(vec![(AttributeValue::string("k"), AttributeValue::None)]),
                ),
            ]),
        ),
    );
    let mut lockfile = BazelLockFileValue::empty_lockfile();
    lockfile.module_extensions.insert(
        f.extension_id.clone(),
        IndexMap::from([(f.eval_factors.clone(), ext)]),
    );
    let json = lockfile.to_json();
    let expected = r#"  "moduleExtensions": {
    "//:ext.bzl%ext": {
      "os:linux,arch:x86_64": {
        "bzlTransitiveDigest": "AQID",
        "usagesDigest": "BAUG",
        "recordedInputs": [],
        "generatedRepoSpecs": {
          "r": {
            "repoRuleId": "@@bazel_tools//tools/build_defs/repo:http.bzl%http_archive",
            "attributes": {
              "label": "@@foo+//:bar",
              "at_string": "'@@not_a_label'",
              "quoted": "''q''",
              "ctl": "a\u0001b\t\"<>&='\u2028",
              "list": [
                null,
                -3
              ],
              "dict": {}
            }
          }
        }
      }
    }
  },"#;
    assert!(json.contains(expected), "{json}");
    // Reading back gives the same value (None attributes are dropped on write).
    let back = BazelLockFileValue::from_json(&json).unwrap().unwrap();
    let spec = &back.module_extensions[&f.extension_id][&f.eval_factors].generated_repo_specs["r"];
    assert_eq!(
        spec.attributes().get("at_string"),
        Some(&AttributeValue::string("@@not_a_label"))
    );
    assert_eq!(
        spec.attributes().get("quoted"),
        Some(&AttributeValue::string("'q'"))
    );
    assert!(spec.attributes().get("none").is_none());
    assert_eq!(back.to_json(), json);
}

#[test]
fn facts_serialization() {
    let f = set_up();
    let facts = Facts::validate_and_create(FactValue::Dict(vec![
        (
            "b".into(),
            FactValue::List(vec![FactValue::Float(1.0), FactValue::None]),
        ),
        (
            "a".into(),
            FactValue::Dict(vec![
                ("z".into(), FactValue::None),
                ("y".into(), FactValue::Int(7.into())),
            ]),
        ),
    ]))
    .unwrap();
    let mut lockfile = BazelLockFileValue::empty_lockfile();
    lockfile.facts.insert(f.extension_id.clone(), facts);
    lockfile.facts_versions.insert(f.extension_id.clone(), 2);
    let json = lockfile.to_json();
    assert!(
        json.ends_with(
            "  \"facts\": {\n    \"//:ext.bzl%ext\": {\n      \"a\": {\n        \"y\": 7\n      },\n      \"b\": [\n        1.0,\n        null\n      ]\n    }\n  },\n  \"factsVersions\": {\n    \"//:ext.bzl%ext\": 2\n  }\n}"
        ),
        "{json}"
    );
    let back = BazelLockFileValue::from_json(&json).unwrap().unwrap();
    assert_eq!(back.to_json(), json);
}

#[test]
fn starlark_float_formatting() {
    assert_eq!(starlark_float_repr(1.0), "1.0");
    assert_eq!(starlark_float_repr(0.1), "0.1");
    assert_eq!(starlark_float_repr(-2.5), "-2.5");
    assert_eq!(starlark_float_repr(1e16), "10000000000000000.0");
    assert_eq!(starlark_float_repr(1e17), "1e+17");
    assert_eq!(starlark_float_repr(1.5e-5), "1.5e-05");
    assert_eq!(starlark_float_repr(0.0001), "0.0001");
    assert_eq!(starlark_float_repr(123.456), "123.456");
}

#[test]
fn facts_too_deep() {
    let mut v = FactValue::Int(1.into());
    for _ in 0..8 {
        v = FactValue::List(vec![v]);
    }
    let e = Facts::validate_and_create(FactValue::Dict(vec![("k".into(), v)])).unwrap_err();
    assert_eq!(e, "Facts cannot be nested more than 7 levels deep");
}

/// Not a JUnit test. `BazelLockFileFunction.getLockfileValue` reads the version with
/// `Integer.parseInt(matcher.group(1))` (BazelLockFileFunction.java:124); digits beyond the int
/// range throw NumberFormatException, an IllegalArgumentException that `compute` (:99-117) turns
/// into BAD_LOCKFILE, or into the empty lockfile for the hidden lockfile.
#[test]
fn lock_file_version_beyond_int_range_is_a_bad_lockfile() {
    for (digits, json) in [
        (
            "99999999999",
            "{\n  \"lockFileVersion\": 99999999999,\n  \"moduleExtensions\": {}\n}",
        ),
        ("2147483648", "{\"lockFileVersion\":2147483648}"),
    ] {
        for mode in [
            LockfileMode::Update,
            LockfileMode::Refresh,
            LockfileMode::Error,
        ] {
            let e = BazelLockFileFunction::compute(Ok(Some(json.into())), false, mode).unwrap_err();
            assert_eq!(e.get_code(), ExternalDepsCode::BadLockfile);
            assert_eq!(
                e.get_message(),
                format!(
                    "Failed to read and parse the MODULE.bazel.lock file with error: For input \
                     string: \"{digits}\". Try deleting it and rerun the build."
                )
            );
        }
        // The hidden lockfile never fails.
        assert_eq!(
            BazelLockFileFunction::compute(Ok(Some(json.into())), true, LockfileMode::Update)
                .unwrap(),
            BazelLockFileValue::empty_lockfile()
        );
        // Off mode ignores the file.
        assert_eq!(
            BazelLockFileFunction::compute(Ok(Some(json.into())), false, LockfileMode::Off)
                .unwrap(),
            BazelLockFileValue::empty_lockfile()
        );
    }
    // Integer.MAX_VALUE parses: it is an unsupported version, not a malformed file.
    let max = "{\"lockFileVersion\": 2147483647}";
    assert_eq!(
        BazelLockFileFunction::compute(Ok(Some(max.into())), false, LockfileMode::Update).unwrap(),
        BazelLockFileValue::empty_lockfile()
    );
}

#[test]
fn read_errors() {
    // Old versions are ignored (or rejected in error mode).
    let old = "{\"lockFileVersion\": 26}";
    assert_eq!(
        BazelLockFileFunction::compute(Ok(Some(old.into())), false, LockfileMode::Update).unwrap(),
        BazelLockFileValue::empty_lockfile()
    );
    let e = BazelLockFileFunction::compute(Ok(Some(old.into())), false, LockfileMode::Error)
        .unwrap_err();
    assert_eq!(e.get_code(), ExternalDepsCode::BadLockfile);
    assert_eq!(
        e.get_message(),
        "The version of MODULE.bazel.lock is not supported by this version of Bazel. Please run \
         `bazel mod deps --lockfile_mode=update` to update your lockfile."
    );
    // Missing file.
    assert_eq!(
        BazelLockFileFunction::compute(Ok(None), false, LockfileMode::Error).unwrap(),
        BazelLockFileValue::empty_lockfile()
    );
    // A merge conflict.
    let conflict = "{\n  \"lockFileVersion\": 28,\n<<<<<<< HEAD\n  \"registryFileHashes\": {}\n}";
    let e = BazelLockFileFunction::compute(Ok(Some(conflict.into())), false, LockfileMode::Update)
        .unwrap_err();
    assert!(
        e.get_message()
            .starts_with("Failed to read and parse the MODULE.bazel.lock file with error: "),
        "{}",
        e.get_message()
    );
    assert!(
        e.get_message().ends_with(
            " This looks like a merge conflict. See \
             https://bazel.build/external/lockfile#merge-conflicts for advice."
        ),
        "{}",
        e.get_message()
    );
    // The hidden lockfile never fails.
    assert_eq!(
        BazelLockFileFunction::compute(Ok(Some(conflict.into())), true, LockfileMode::Error)
            .unwrap(),
        BazelLockFileValue::empty_lockfile()
    );
    // Off mode ignores the file.
    assert_eq!(
        BazelLockFileFunction::compute(Ok(Some(conflict.into())), false, LockfileMode::Off)
            .unwrap(),
        BazelLockFileValue::empty_lockfile()
    );
    // A repo spec without a rule id makes the lockfile invalid.
    let invalid = r#"{"lockFileVersion": 28, "moduleExtensions": {"//:ext.bzl%ext": {"general": {"bzlTransitiveDigest": "", "usagesDigest": "", "recordedInputs": [], "generatedRepoSpecs": {"r": {"attributes": {}}}}}}}"#;
    assert_eq!(
        BazelLockFileFunction::compute(Ok(Some(invalid.into())), false, LockfileMode::Update)
            .unwrap(),
        BazelLockFileValue::empty_lockfile()
    );
    assert!(
        BazelLockFileFunction::compute(Ok(Some(invalid.into())), false, LockfileMode::Error)
            .is_err()
    );
}

#[test]
fn eval_factors() {
    let f = ModuleExtensionEvalFactors::parse("os:linux,arch:x86_64");
    assert_eq!(f, ModuleExtensionEvalFactors::create("linux", "x86_64"));
    assert_eq!(f.to_string(), "os:linux,arch:x86_64");
    assert_eq!(
        ModuleExtensionEvalFactors::parse("general").to_string(),
        "general"
    );
    assert_eq!(
        ModuleExtensionEvalFactors::parse("arch:arm").to_string(),
        "arch:arm"
    );
    assert!(
        ModuleExtensionEvalFactors::create("", "x")
            .has_same_dependencies_as(&ModuleExtensionEvalFactors::create("", "y"))
    );
}
