//! Port of RepoFileFunctionTest.
//!
//! Java: src/test/java/com/google/devtools/build/lib/skyframe/RepoFileFunctionTest.java. The Java
//! tests check the effect of REPO.bazel on configured targets, which needs packages and analysis;
//! they are ignored until those exist. The `repo_file_level_*` tests check the same REPO.bazel
//! contents at the REPO_FILE / REPO_PACKAGE_ARGS level.

use std::sync::Arc;

use blaze_cmdline::RepositoryName;
use blaze_cmdline::repository_name::MAIN;
use blaze_events::StoredEventHandler;
use blaze_skyframe::SkyKey;
use starlark::values::dict::DictRef;

use crate::repo_file::{BadRepoFileException, RepoFileKey, RepoPackageArgsKey};
use crate::test_util::{FakeRepos, TestSetup, evaluate, value_of};

#[test]
#[ignore = "needs PACKAGE and analysis (getConfiguredTarget, rule attributes)"]
fn repo_file_in_the_main_repo() {}

#[test]
#[ignore = "needs Bzlmod (MODULE.bazel registry), PACKAGE and analysis"]
fn repo_file_in_an_external_repo() {}

#[test]
#[ignore = "needs PACKAGE (assertTargetError)"]
fn cant_call_repo_twice() {}

#[test]
#[ignore = "needs PACKAGE and analysis (feature merging)"]
fn feature_merger() {}

#[test]
#[ignore = "needs PACKAGE (assertTargetError)"]
fn restricted_syntax() {}

fn repo_file_error(setup: &TestSetup, repo: RepositoryName) -> (String, Vec<String>) {
    let evaluator = setup.evaluator();
    let events = Arc::new(StoredEventHandler::new());
    let key = RepoFileKey::key(repo);
    let result = evaluate(
        &evaluator,
        vec![SkyKey::new(key.clone())],
        false,
        events.clone(),
    );
    let error = result
        .get_error(&SkyKey::new(key))
        .expect("REPO_FILE should fail");
    let e = error
        .exception_as::<BadRepoFileException>()
        .expect("a BadRepoFileException");
    (
        e.get_message().to_owned(),
        events
            .get_events()
            .iter()
            .map(|e| e.get_message().into_owned())
            .collect(),
    )
}

#[test]
fn repo_file_level_repo_file_in_the_main_repo() {
    let setup = TestSetup::new(FakeRepos::default());
    setup.scratch.file(
        &setup.ws("REPO.bazel"),
        "repo(default_deprecation='EVERYTHING IS DEPRECATED')",
    );
    let evaluator = setup.evaluator();
    let value = value_of(&evaluator, RepoFileKey::key(MAIN.clone()));
    let args = value.package_args_map().expect("repo() was called");
    let dict = DictRef::from_value(args.value()).expect("a dict");
    let entries: Vec<(String, String)> = dict
        .iter()
        .map(|(k, v)| (k.unpack_str().unwrap().to_owned(), v.to_str()))
        .collect();
    assert_eq!(
        entries,
        vec![(
            "default_deprecation".to_owned(),
            "EVERYTHING IS DEPRECATED".to_owned()
        )]
    );
    assert!(value.ignored_directories().is_empty());

    let package_args = value_of(&evaluator, RepoPackageArgsKey::key(MAIN.clone()));
    assert_eq!(
        package_args.get_package_args().default_deprecation(),
        Some("EVERYTHING IS DEPRECATED")
    );
}

#[test]
fn repo_file_level_no_repo_file() {
    let setup = TestSetup::new(FakeRepos::default());
    let evaluator = setup.evaluator();
    let value = value_of(&evaluator, RepoFileKey::key(MAIN.clone()));
    assert!(value.package_args_map().is_none());
    assert!(value.ignored_directories().is_empty());
}

#[test]
fn repo_file_level_cant_call_repo_twice() {
    let setup = TestSetup::new(FakeRepos::default());
    setup.scratch.file(
        &setup.ws("REPO.bazel"),
        "repo(default_deprecation='EVERYTHING IS DEPRECATED')\nrepo(features=['abc'])\n",
    );
    let (message, events) = repo_file_error(&setup, MAIN.clone());
    assert_eq!(
        message,
        "error evaluating REPO.bazel file for the main repo"
    );
    assert!(
        events
            .iter()
            .any(|e| e.contains("'repo' can only be called once")),
        "{:?}",
        events
    );
}

#[test]
fn repo_file_level_restricted_syntax() {
    let setup = TestSetup::new(FakeRepos::default());
    setup.scratch.file(
        &setup.ws("REPO.bazel"),
        "if 3+5>7: repo(default_deprecation='EVERYTHING IS DEPRECATED')",
    );
    let (message, events) = repo_file_error(&setup, MAIN.clone());
    assert_eq!(message, "error parsing REPO.bazel file for the main repo");
    assert!(
        events.iter().any(|e| e.contains(
            "`if` statements are not allowed in REPO.bazel files. You may use an `if` expression \
             for simple cases."
        )),
        "{:?}",
        events
    );
}

#[test]
fn repo_file_level_parse_error() {
    let setup = TestSetup::new(FakeRepos::default());
    let path = setup.scratch.file(&setup.ws("REPO.bazel"), "repo(\n");
    let (message, _events) = repo_file_error(&setup, MAIN.clone());
    assert_eq!(
        message,
        format!("error parsing REPO.bazel file at {}", path)
    );
}

#[test]
fn repo_file_level_ignore_directories_after_repo() {
    let setup = TestSetup::new(FakeRepos::default());
    setup.scratch.file(
        &setup.ws("REPO.bazel"),
        "repo(features=['a'])\nignore_directories(['x', 'y/**'])\n",
    );
    let evaluator = setup.evaluator();
    let value = value_of(&evaluator, RepoFileKey::key(MAIN.clone()));
    assert_eq!(
        value.ignored_directories(),
        &["x".to_owned(), "y/**".to_owned()]
    );
}

#[test]
fn repo_file_level_repo_after_ignore_directories() {
    let setup = TestSetup::new(FakeRepos::default());
    setup.scratch.file(
        &setup.ws("REPO.bazel"),
        "ignore_directories(['x'])\nrepo(features=['a'])\n",
    );
    let (message, events) = repo_file_error(&setup, MAIN.clone());
    assert_eq!(
        message,
        "error evaluating REPO.bazel file for the main repo"
    );
    assert!(
        events.iter().any(
            |e| e.contains("if repo() is called, it must be called before any other functions")
        ),
        "{:?}",
        events
    );
}

#[test]
fn repo_file_level_external_repo_display_name() {
    let scratch_dir = crate::test_util::Scratch::new();
    let repo_dir = scratch_dir.dir("foo_repo");
    scratch_dir.file("foo_repo/REPO.bazel", "repo()\n");
    let mut repos = FakeRepos::default();
    repos.dirs.insert("foo+".to_owned(), repo_dir);
    let setup = TestSetup::new(repos);
    let (message, events) = repo_file_error(&setup, RepositoryName::create_unvalidated("foo+"));
    assert_eq!(message, "error evaluating REPO.bazel file for @@foo+");
    assert!(
        events
            .iter()
            .any(|e| e.contains("at least one argument must be given to the 'repo' function")),
        "{:?}",
        events
    );
}

// Not a Java test: REPO_PACKAGE_ARGS converts the repo() arguments with PackageArgs.processParam.
#[test]
fn repo_package_args_bad_argument() {
    let setup = TestSetup::new(FakeRepos::default());
    setup
        .scratch
        .file(&setup.ws("REPO.bazel"), "repo(default_testonly='yes')\n");
    let evaluator = setup.evaluator();
    let events = Arc::new(StoredEventHandler::new());
    let key = RepoPackageArgsKey::key(MAIN.clone());
    let result = evaluate(
        &evaluator,
        vec![SkyKey::new(key.clone())],
        false,
        events.clone(),
    );
    let error = result
        .get_error(&SkyKey::new(key))
        .expect("REPO_PACKAGE_ARGS should fail");
    let e = error
        .exception_as::<crate::repo_file::BadPackageArgsException>()
        .expect("a BadPackageArgsException");
    assert_eq!(
        e.get_message(),
        "error evaluating REPO.bazel file for the main repo"
    );
    assert!(
        events.get_events().iter().any(|e| e
            .get_message()
            .contains("repo() argument 'default_testonly'")),
        "{:?}",
        events.get_events()
    );
}
