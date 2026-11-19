//! Tests of REPOSITORY_MAPPING / REPOSITORY_DIRECTORY over [`OracleRepos`] (Blaze-only; the Java
//! counterpart, RepositoryMappingFunctionTest, needs Bzlmod resolution and is ported with it).

use std::sync::Arc;

use blaze_cmdline::repository_name::{BUILTINS, MAIN};
use blaze_cmdline::{PackageIdentifier, RepositoryName};
use blaze_events::StoredEventHandler;
use blaze_skyframe::SkyKey;
use blaze_starlark::env::NativeRegistry;
use blaze_vfs::{PathFragment, Root};

use crate::build_file_name::BuildFileName;
use crate::io::IOException;
use crate::package_lookup::{PackageLookupKey, PackageLookupValue};
use crate::repos::{
    OracleRepos, RepositoryDirectoryKey, RepositoryDirectoryValue, RepositoryMappingKey,
};
use crate::skyframe_executor::LoadingFunctionsConfig;
use crate::test_util::{FakeRepos, TestSetup, evaluate, value_of};

const JSON: &str = r#"{
 "format": 1,
 "project": "test",
 "workspace": "/ws",
 "main_repo_mapping": [["", ""], ["me", ""], ["rules_cc", "rules_cc+"], ["bazel_tools", "bazel_tools"]],
 "repos": {
  "": {"found": true, "kind": "main", "module_name": "me", "module_version": "1.0",
       "mapping": [["", ""], ["me", ""], ["rules_cc", "rules_cc+"], ["bazel_tools", "bazel_tools"]],
       "directory": "/ws"},
  "bazel_tools": {"found": true, "kind": "module", "module_name": "bazel_tools", "module_version": "",
       "mapping": [["bazel_tools", "bazel_tools"], ["platforms", "platforms"]], "directory": null},
  "_builtins": {"found": true, "kind": "builtins", "module_name": null, "module_version": null,
       "mapping": [["bazel_tools", "bazel_tools"], ["platforms", "platforms"], ["_builtins", "_builtins"], ["", ""]],
       "directory": null},
  "rules_cc+": {"found": true, "kind": "module", "module_name": "rules_cc", "module_version": "0.2.17",
       "mapping": [["rules_cc", "rules_cc+"], ["bazel_tools", "bazel_tools"]], "directory": "@RULES_CC@"},
  "rules_cc++ext+cfg": {"found": true, "kind": "extension", "module_name": "rules_cc", "module_version": "0.2.17",
       "mapping": [["rules_cc", "rules_cc+"], ["cfg", "rules_cc++ext+cfg"]], "directory": null},
  "gone+": {"found": false, "kind": "not_found", "module_name": null, "module_version": null,
       "mapping": null, "directory": null}
 }
}"#;

fn setup_with_oracle_repos() -> TestSetup {
    let mut setup = TestSetup::new(FakeRepos::default());
    let rules_cc = setup.scratch.dir("repo_cache/rules_cc");
    setup.scratch.file(
        "repo_cache/rules_cc/MODULE.bazel",
        "module(name='rules_cc')\n",
    );
    setup.scratch.file("repo_cache/rules_cc/cc/BUILD.bazel", "");
    let json = JSON.replace("@RULES_CC@", rules_cc.get_path_string());
    let repos = OracleRepos::parse(&json, setup.config.directories.clone()).unwrap();
    setup.config = LoadingFunctionsConfig::for_bazel(
        setup.config.directories.clone(),
        Arc::new(repos),
        Arc::new(NativeRegistry::new()),
        crate::test_util::minimal_starlark_environment(),
        None,
    );
    setup
}

#[test]
fn main_repo_mapping_and_module() {
    let setup = setup_with_oracle_repos();
    let evaluator = setup.evaluator();
    let value = value_of(&evaluator, RepositoryMappingKey::key(MAIN.clone()));
    let mapping = value.repository_mapping().unwrap();
    assert_eq!(
        mapping.get("rules_cc"),
        RepositoryName::create_unvalidated("rules_cc+")
    );
    assert_eq!(mapping.get("me"), MAIN.clone());
    assert_eq!(value.associated_module_name(), Some("me"));
    assert_eq!(value.associated_module_version(), Some("1.0"));
    let keys: Vec<&str> = mapping.entries().keys().map(|k| k.as_str()).collect();
    assert_eq!(keys, vec!["", "me", "rules_cc", "bazel_tools"]);
}

#[test]
fn builtins_mapping_is_computed_from_bazel_tools() {
    let setup = setup_with_oracle_repos();
    let evaluator = setup.evaluator();
    let value = value_of(&evaluator, RepositoryMappingKey::key(BUILTINS.clone()));
    let keys: Vec<&str> = value
        .repository_mapping()
        .unwrap()
        .entries()
        .keys()
        .map(|k| k.as_str())
        .collect();
    // Java: DEFAULT_VALUE_FOR_BUILTINS_REPO.withAdditionalMappings(bazel_tools mapping): the
    // additional (bazel_tools) entries come first. This is also what the oracle printed.
    assert_eq!(keys, vec!["bazel_tools", "platforms", "_builtins", ""]);
    assert_eq!(value.associated_module_name(), None);
}

#[test]
fn not_found_repos() {
    let setup = setup_with_oracle_repos();
    let evaluator = setup.evaluator();
    for name in ["gone+", "never_recorded+"] {
        let value = value_of(
            &evaluator,
            RepositoryMappingKey::key(RepositoryName::create_unvalidated(name)),
        );
        assert!(value.is_not_found(), "{}", name);
    }
    let value = value_of(
        &evaluator,
        RepositoryDirectoryKey::key(RepositoryName::create_unvalidated("gone+")),
    );
    assert_eq!(
        *value,
        RepositoryDirectoryValue::Failure {
            error_msg: "Repository '@@gone+' is not defined".to_owned()
        }
    );
}

#[test]
fn repository_directory_symlinks_into_external() {
    let setup = setup_with_oracle_repos();
    let evaluator = setup.evaluator();
    let value = value_of(
        &evaluator,
        RepositoryDirectoryKey::key(RepositoryName::create_unvalidated("rules_cc+")),
    );
    let external = setup
        .config
        .directories
        .get_external_repository_directory()
        .get_relative_str("rules_cc+");
    assert_eq!(
        *value,
        RepositoryDirectoryValue::Success {
            root: Root::from_path(external.clone()),
            exclude_from_vendoring: true
        }
    );
    assert!(external.is_symbolic_link());

    let pkg = value_of(
        &evaluator,
        PackageLookupKey::key(PackageIdentifier::create(
            RepositoryName::create_unvalidated("rules_cc+"),
            PathFragment::create("cc"),
        )),
    );
    assert_eq!(
        *pkg,
        PackageLookupValue::success(Root::from_path(external), BuildFileName::BuildDotBazel)
    );
}

#[test]
fn unfetched_repository_fails_loudly() {
    let setup = setup_with_oracle_repos();
    let evaluator = setup.evaluator();
    let key = RepositoryDirectoryKey::key(RepositoryName::create_unvalidated("rules_cc++ext+cfg"));
    let result = evaluate(
        &evaluator,
        vec![SkyKey::new(key.clone())],
        true,
        Arc::new(StoredEventHandler::new()),
    );
    let error = result.get_error(&SkyKey::new(key)).unwrap();
    let e = error.exception_as::<IOException>().unwrap();
    assert!(
        e.get_message().contains("never fetched by the oracle"),
        "{}",
        e
    );
}

#[test]
#[ignore = "reads ~/.cache/blaze/w3/brotli.repos.json (scripts/w3/oracle-repos); run with --ignored"]
fn brotli_recording_loads() {
    let home = std::env::var_os("HOME").unwrap();
    let path = std::path::PathBuf::from(home).join(".cache/blaze/w3/brotli.repos.json");
    let setup = TestSetup::new(FakeRepos::default());
    let repos = OracleRepos::load(&path, setup.config.directories.clone()).unwrap();
    let mut setup = setup;
    setup.config = LoadingFunctionsConfig::for_bazel(
        setup.config.directories.clone(),
        Arc::new(repos),
        Arc::new(NativeRegistry::new()),
        crate::test_util::minimal_starlark_environment(),
        None,
    );
    let evaluator = setup.evaluator();
    let main = value_of(&evaluator, RepositoryMappingKey::key(MAIN.clone()));
    assert_eq!(
        main.repository_mapping().unwrap().get("rules_cc"),
        RepositoryName::create_unvalidated("rules_cc+")
    );
    assert_eq!(main.associated_module_name(), Some("brotli"));
    let rules_cc = value_of(
        &evaluator,
        RepositoryDirectoryKey::key(RepositoryName::create_unvalidated("rules_cc+")),
    );
    assert!(matches!(
        *rules_cc,
        RepositoryDirectoryValue::Success { .. }
    ));
}
