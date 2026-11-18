#![cfg(feature = "wip")]
// Options and protos are filled in field by field, as the Java tests set them.
#![allow(clippy::field_reassign_with_default)]
//! Port of `src/test/java/com/google/devtools/build/lib/remote/CombinedCacheClientFactoryTest.java`.
//!
//! Java runs the tests on an `InMemoryFileSystem`; here they run on the real file system under a
//! temporary directory, so Java's absolute paths (`/etc/something`) are prefixed with that
//! directory.

use std::io;
use std::sync::Arc;

use blaze_optdefs::misc::auth_and_tls_options::AuthAndTLSOptions;
use blaze_optdefs::misc::remote_options::RemoteOptions;
use blaze_options::Foreign;
use blaze_remote::combined_cache_client_factory::{self, CombinedCacheClient};
use blaze_remote::remote_retrier::RemoteRetrier;
use blaze_remote::retrier::{
    ALLOW_ALL_CALLS, Backoff, RETRIES_DISABLED, ResultClassifierResult, TokioRetryScheduler,
};
use blaze_remote::util::DigestUtil;
use blaze_vfs::{DigestHashFunction, Path, PathFragment, UnixFileSystem};

/// The fields of the Java test class.
struct Fixture {
    _tmp: tempfile::TempDir,
    /// The directory standing for `/` of the Java test's file system.
    fs_root: String,
    digest_util: DigestUtil,
    remote_options: RemoteOptions,
    auth_and_tls_options: AuthAndTLSOptions,
    working_directory: Path,
    retrier: Arc<RemoteRetrier>,
}

impl Fixture {
    // Java: CombinedCacheClientFactoryTest.setUp
    fn new() -> Fixture {
        let tmp = tempfile::tempdir().unwrap();
        let fs_root = std::fs::canonicalize(tmp.path())
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();
        let fs = UnixFileSystem::create(DigestHashFunction::Sha256);
        let working_directory = Path::create_from_str(&format!("{fs_root}/etc/something"), &fs);
        // `new RemoteRetrier(() -> RemoteRetrier.RETRIES_DISABLED, (e) -> Result.SUCCESS,
        // retryScheduler, Retrier.ALLOW_ALL_CALLS)`
        let retrier = Arc::new(RemoteRetrier::with_backoff(
            Arc::new(|| -> Arc<dyn Backoff> { RETRIES_DISABLED.clone() }),
            Arc::new(|_e: &io::Error| ResultClassifierResult::Success),
            Arc::new(TokioRetryScheduler),
            ALLOW_ALL_CALLS.clone(),
        ));
        Fixture {
            _tmp: tmp,
            fs_root,
            digest_util: DigestUtil::new(DigestHashFunction::Sha256),
            remote_options: RemoteOptions::default(),
            auth_and_tls_options: AuthAndTLSOptions::default(),
            working_directory,
            retrier,
        }
    }

    /// `PathFragment.create(path)` for an absolute path of the Java test's file system.
    fn fragment(&self, path: &str) -> PathFragment {
        PathFragment::create(&format!("{}{path}", self.fs_root))
    }

    /// `fs.getPath(path)`.
    fn get_path(&self, path: &str) -> Path {
        self.working_directory.get_relative(&self.fragment(path))
    }

    /// `CombinedCacheClientFactory.create(remoteOptions, /* creds= */ null, authAndTlsOptions,
    /// workingDirectory, digestUtil, retrier, /* checkActionResultIntegrity= */ true)`.
    fn create(&self, working_directory: Option<&Path>) -> io::Result<CombinedCacheClient> {
        combined_cache_client_factory::create(
            &self.remote_options,
            /* creds= */ None,
            &self.auth_and_tls_options,
            working_directory,
            &self.digest_util,
            self.retrier.clone(),
            /* check_action_result_integrity= */ true,
        )
    }
}

#[test]
#[ignore = "needs HttpCacheClient (the HTTP remote cache is not ported)"]
fn create_combined_cache_with_existing_working_directory() {
    let mut f = Fixture::new();
    f.remote_options.remote_cache = Some("http://doesnotexist.com".to_string());
    f.remote_options.disk_cache = Some(Foreign(f.fragment("/etc/something/cache/here")));
    f.get_path("/etc/something/cache/here")
        .create_directory_and_parents()
        .unwrap();

    let blob_store = f.create(Some(&f.working_directory)).unwrap();

    // `assertThat(blobStore.remoteCacheClient()).isInstanceOf(HttpCacheClient.class)`
    assert!(blob_store.remote_cache_client().is_some());
    assert!(blob_store.disk_cache_client().is_some());
}

#[test]
#[ignore = "needs HttpCacheClient (the HTTP remote cache is not ported)"]
fn create_combined_cache_with_not_existing_working_directory() {
    let mut f = Fixture::new();
    f.remote_options.remote_cache = Some("http://doesnotexist.com".to_string());
    f.remote_options.disk_cache = Some(Foreign(f.fragment("/etc/something/cache/here")));
    assert!(!f.working_directory.exists());

    let blob_store = f.create(Some(&f.working_directory)).unwrap();

    // `assertThat(blobStore.remoteCacheClient()).isInstanceOf(HttpCacheClient.class)`
    assert!(blob_store.remote_cache_client().is_some());
    assert!(blob_store.disk_cache_client().is_some());
    assert!(f.working_directory.exists());
}

#[test]
#[should_panic(expected = "NullPointerException")]
fn create_combined_cache_with_missing_working_directory_should_throw_exception() {
    let mut f = Fixture::new();
    // interesting case: workingDirectory = null -> NPE.
    f.remote_options.remote_cache = Some("http://doesnotexist.com".to_string());
    f.remote_options.disk_cache = Some(Foreign(f.fragment("/etc/something/cache/here")));

    let _ = f.create(/* working_directory= */ None);
}

#[test]
#[ignore = "needs HttpCacheClient over a Unix domain socket (the HTTP remote cache is not ported)"]
fn create_http_cache_with_proxy() {
    let mut f = Fixture::new();
    f.remote_options.remote_cache = Some("http://doesnotexist.com".to_string());
    f.remote_options.remote_proxy = Some("unix://some-proxy".to_string());

    let blob_store = f.create(Some(&f.working_directory)).unwrap();

    // `assertThat(blobStore.remoteCacheClient()).isInstanceOf(HttpCacheClient.class)`
    assert!(blob_store.remote_cache_client().is_some());
    assert!(blob_store.disk_cache_client().is_none());
}

#[test]
#[should_panic(expected = "Remote cache proxy unsupported: bad-proxy")]
fn create_http_cache_fails_with_unsupported_proxy_protocol() {
    let mut f = Fixture::new();
    f.remote_options.remote_cache = Some("http://doesnotexist.com".to_string());
    f.remote_options.remote_proxy = Some("bad-proxy".to_string());

    // `assertThrows(RuntimeException.class, ...)`: a RuntimeException is a panic.
    let _ = f.create(Some(&f.working_directory));
}

#[test]
#[ignore = "needs HttpCacheClient (the HTTP remote cache is not ported)"]
fn create_http_cache_without_proxy() {
    let mut f = Fixture::new();
    f.remote_options.remote_cache = Some("http://doesnotexist.com".to_string());

    let blob_store = f.create(Some(&f.working_directory)).unwrap();

    // `assertThat(blobStore.remoteCacheClient()).isInstanceOf(HttpCacheClient.class)`
    assert!(blob_store.remote_cache_client().is_some());
    assert!(blob_store.disk_cache_client().is_none());
}

#[test]
fn create_disk_cache() {
    let mut f = Fixture::new();
    f.remote_options.disk_cache = Some(Foreign(f.fragment("/etc/something/cache/here")));

    let blob_store = f.create(Some(&f.working_directory)).unwrap();

    assert!(blob_store.remote_cache_client().is_none());
    assert!(blob_store.disk_cache_client().is_some());
}

#[test]
fn is_remote_cache_options_http_cache_enabled() {
    let mut f = Fixture::new();
    f.remote_options.remote_cache = Some("http://doesnotexist:90".to_string());
    assert!(combined_cache_client_factory::is_remote_cache_options(
        &f.remote_options
    ));
}

#[test]
fn is_remote_cache_options_http_cache_enabled_in_upper_case() {
    let mut f = Fixture::new();
    f.remote_options.remote_cache = Some("HTTP://doesnotexist:90".to_string());
    assert!(combined_cache_client_factory::is_remote_cache_options(
        &f.remote_options
    ));
}

#[test]
fn is_remote_cache_options_https_cache_enabled() {
    let mut f = Fixture::new();
    f.remote_options.remote_cache = Some("https://doesnotexist:90".to_string());
    assert!(combined_cache_client_factory::is_remote_cache_options(
        &f.remote_options
    ));
}

#[test]
fn is_remote_cache_options_bad_protocol_starts_with_http() {
    let mut f = Fixture::new();
    f.remote_options.remote_cache = Some("httplolol://doesnotexist:90".to_string());
    assert!(!combined_cache_client_factory::is_remote_cache_options(
        &f.remote_options
    ));
}

#[test]
fn is_remote_cache_options_disk_cache_enabled() {
    let mut f = Fixture::new();
    f.remote_options.disk_cache = Some(Foreign(f.fragment("/etc/something/cache/here")));
    assert!(combined_cache_client_factory::is_remote_cache_options(
        &f.remote_options
    ));
}

#[test]
fn is_remote_cache_options_http_and_disk_cache_enabled() {
    let mut f = Fixture::new();
    f.remote_options.remote_cache = Some("http://doesnotexist:90".to_string());
    f.remote_options.disk_cache = Some(Foreign(f.fragment("/etc/something/cache/here")));

    assert!(combined_cache_client_factory::is_remote_cache_options(
        &f.remote_options
    ));
}

#[test]
fn is_remote_cache_options_https_and_disk_cache_enabled() {
    let mut f = Fixture::new();
    f.remote_options.remote_cache = Some("https://doesnotexist:90".to_string());
    f.remote_options.disk_cache = Some(Foreign(f.fragment("/etc/something/cache/here")));

    assert!(combined_cache_client_factory::is_remote_cache_options(
        &f.remote_options
    ));
}

#[test]
fn is_remote_cache_options_http_cache_disabled_when_grpc_enabled() {
    let mut f = Fixture::new();
    f.remote_options.remote_cache = Some("grpc://doesnotexist:90".to_string());

    assert!(!combined_cache_client_factory::is_remote_cache_options(
        &f.remote_options
    ));
}

#[test]
fn is_remote_cache_options_http_cache_disabled_when_no_protocol() {
    let mut f = Fixture::new();
    f.remote_options.remote_cache = Some("doesnotexist:90".to_string());

    assert!(!combined_cache_client_factory::is_remote_cache_options(
        &f.remote_options
    ));
}

#[test]
fn is_remote_cache_options_disk_cache_option_empty() {
    let mut f = Fixture::new();
    f.remote_options.disk_cache = Some(Foreign(PathFragment::empty_fragment()));
    assert!(!combined_cache_client_factory::is_remote_cache_options(
        &f.remote_options
    ));
}

#[test]
fn is_remote_cache_options_remote_http_cache_option_empty() {
    let mut f = Fixture::new();
    f.remote_options.remote_cache = Some(String::new());
    assert!(!combined_cache_client_factory::is_remote_cache_options(
        &f.remote_options
    ));
}

#[test]
fn is_remote_cache_options_default_options() {
    let f = Fixture::new();
    assert!(!combined_cache_client_factory::is_remote_cache_options(
        &f.remote_options
    ));
}
