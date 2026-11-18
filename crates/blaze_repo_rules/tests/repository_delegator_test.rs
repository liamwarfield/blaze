//! Port of RepositoryDelegatorTest (src/test/java/com/google/devtools/build/lib/bazel/repository/
//! RepositoryDelegatorTest.java): REPOSITORY_DIRECTORY through `RepositoryFetchFunction`.
//!
//! The Java fixture registers every Bzlmod, loading and repository SkyFunction; the tests that
//! reach REPO_DEFINITION need that setup and are ignored until a shared fixture exists. The
//! visibility tests return before any dependency other than STARLARK_SEMANTICS.

use std::sync::Arc;

use blaze_cmdline::RepositoryName;
use blaze_cmdline::repository_name::MAIN;
use blaze_repo_fetch::cache::local_repo_contents_cache::LocalRepoContentsCache;
use blaze_repo_rules::repository::repository_fetch_function::RepositoryFetchFunction;
use blaze_repo_rules::services::RepoServices;
use blaze_sky_loading::directories::LoadingDirectories;
use blaze_sky_loading::precomputed::STARLARK_SEMANTICS;
use blaze_sky_loading::repos::RepositoryDirectoryKey;
use blaze_sky_loading::repos::RepositoryDirectoryValue;
use blaze_skyframe::EvaluationContext;
use blaze_skyframe::Evaluator;
use blaze_skyframe::EvaluatorBuilder;
use blaze_skyframe::SkyKey;
use blaze_starlark::StarlarkSemantics;
use blaze_vfs::DigestHashFunction;
use blaze_vfs::Path;
use blaze_vfs::UnixFileSystem;
use indexmap::IndexMap;

struct Fixture {
    _tmp: tempfile::TempDir,
    evaluator: Evaluator,
}

// Java: RepositoryDelegatorTest.setupDelegator (the part these tests need).
fn setup_delegator() -> Fixture {
    let tmp = tempfile::tempdir().unwrap();
    let fs = UnixFileSystem::create(DigestHashFunction::Sha256);
    let root = Path::create_from_str(tmp.path().to_str().unwrap(), &fs);
    let directories = LoadingDirectories {
        workspace: root.get_child("workspace"),
        output_base: root.get_child("outputbase"),
        install_base: root.get_child("install"),
    };
    let services = Arc::new(RepoServices {
        directories,
        repo_env: Arc::new(IndexMap::new()),
        nonstrict_repo_env: Arc::new(IndexMap::new()),
        download_manager: None,
        timeout_scaling: 1.0,
        process_wrapper: None,
        process_wrapper_kill_delay: None,
        process_wrapper_graceful_sigterm: false,
        repo_contents_cache: Arc::new(LocalRepoContentsCache::new()),
    });
    let fetch_function = Arc::new(RepositoryFetchFunction::new(services));
    let builder = STARLARK_SEMANTICS.set(EvaluatorBuilder::new(), StarlarkSemantics::default());
    let builder = builder.typed_function::<RepositoryDirectoryKey, _>(move |key, env| {
        fetch_function.compute(&key.0, env)
    });
    Fixture {
        _tmp: tmp,
        evaluator: builder.build(),
    }
}

fn evaluate_directory(fixture: &Fixture, key: RepositoryDirectoryKey) -> RepositoryDirectoryValue {
    let evaluation_context = EvaluationContext {
        keep_going: false,
        parallelism: 8,
        ..Default::default()
    };
    let result = fixture
        .evaluator
        .evaluate(&[SkyKey::new(key.clone())], &evaluation_context)
        .unwrap();
    assert!(!result.has_error());
    (*result.get(&key).unwrap()).clone()
}

#[test]
#[ignore = "needs the Bzlmod SkyFunctions (REPO_DEFINITION requests the root module file)"]
fn test_override() {}

#[test]
#[ignore = "needs the Bzlmod, loading and repository SkyFunctions and a fixture workspace"]
fn test_fetch_repository_exception_event_handled() {}

#[test]
#[ignore = "needs the Bzlmod SkyFunctions (REPO_DEFINITION requests the root module file)"]
fn load_repository_not_defined() {}

#[test]
fn load_invisible_repository() {
    let fixture = setup_delegator();
    let key = RepositoryDirectoryKey::key(
        RepositoryName::create_unvalidated("foo")
            .to_non_visible(&RepositoryName::create_unvalidated("fake_owner_repo")),
    );
    let repository_directory_value = evaluate_directory(&fixture, key);
    let RepositoryDirectoryValue::Failure { error_msg } = repository_directory_value else {
        panic!("expected a Failure, got {repository_directory_value:?}");
    };
    assert!(
        error_msg.contains("No repository visible as '@foo' from repository '@@fake_owner_repo'"),
        "{error_msg}"
    );
}

#[test]
fn load_invisible_repository_from_main() {
    let fixture = setup_delegator();
    let key = RepositoryDirectoryKey::key(
        RepositoryName::create_unvalidated("foo").to_non_visible(&MAIN),
    );
    let repository_directory_value = evaluate_directory(&fixture, key);
    let RepositoryDirectoryValue::Failure { error_msg } = repository_directory_value else {
        panic!("expected a Failure, got {repository_directory_value:?}");
    };
    assert!(
        error_msg.contains("No repository visible as '@foo' from main repository"),
        "{error_msg}"
    );
}
