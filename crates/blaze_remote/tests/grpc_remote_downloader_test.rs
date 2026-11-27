//! Port of `src/test/java/com/google/devtools/build/lib/remote/downloader/GrpcRemoteDownloaderTest.java`.
//!
//! The fake `Fetch` service runs on the tests' fake gRPC server (`util::grpc_server`); the
//! cache is `InMemoryCacheClient`; the fallback HTTP downloader is a recording stand-in for
//! Java's `mock(Downloader.class)`. Java's `ManualClock` is the `now` handed to
//! `new_fetch_blob_request` (the downloader itself reads the system clock).

mod util;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use blaze_events::ExtendedEventHandler;
use blaze_events::collectors::StoredEventHandler;
use blaze_optdefs::misc::remote_options::RemoteOptions;
use blaze_remote::common::{
    RemoteActionExecutionContext, RemoteCacheClient, RemoteCacheClientUploads,
};
use blaze_remote::downloader::grpc_remote_downloader::GrpcRemoteDownloader;
use blaze_remote::reapi::asset::{FetchBlobRequest, FetchBlobResponse, Qualifier};
use blaze_remote::reapi::{Digest, RequestMetadata};
use blaze_remote::remote_retrier::{ExponentialBackoff, RemoteRetrier};
use blaze_remote::retrier::{ALLOW_ALL_CALLS, Backoff};
use blaze_remote::runtime::get_from_future;
use blaze_remote::util::DigestUtil;
use blaze_repo_fetch::authandtls::{Credentials, Headers, StaticCredentials};
use blaze_repo_fetch::cache::download_cache::KeyType;
use blaze_repo_fetch::downloader::download_progress_event::FetchEvent;
use blaze_repo_fetch::downloader::{Checksum, Downloader};
use blaze_repo_fetch::errors::{IoException, IoExceptionKind};
use blaze_repo_fetch::uri::JavaUri;
use blaze_vfs::{DigestHashFunction, Path};
use util::grpc_server::{FakeFetch, FakeServer};
use util::in_memory_cache_client::InMemoryCacheClient;

fn digest_util() -> DigestUtil {
    DigestUtil::new(DigestHashFunction::Sha256)
}

fn context() -> RemoteActionExecutionContext<'static> {
    RemoteActionExecutionContext::create(RequestMetadata::default())
}

/// `mock(Downloader.class)` with the tests' `doAnswer`: writes `content` to the output when the
/// urls are `urls`, and records each call's urls.
struct FallbackDownloader {
    urls: Vec<JavaUri>,
    content: Vec<u8>,
    calls: Mutex<Vec<Vec<String>>>,
}

impl Downloader for FallbackDownloader {
    fn download(
        &self,
        urls: &[JavaUri],
        _headers: &Headers,
        _credentials: Arc<dyn Credentials>,
        _checksum: Option<&Checksum>,
        _canonical_id: &str,
        output: &Path,
        _event_handler: Arc<dyn ExtendedEventHandler>,
        _client_env: &HashMap<String, String>,
        _type: Option<&str>,
        context: &str,
    ) -> Result<(), IoException> {
        assert_eq!(context, "context");
        self.calls
            .lock()
            .unwrap()
            .push(urls.iter().map(|u| u.as_str().to_string()).collect());
        if urls == self.urls.as_slice() {
            // FileSystemUtils.writeContent(output, content)
            std::fs::write(output.get_path_string(), &self.content).map_err(IoException::from)?;
        }
        Ok(())
    }
}

/// `mock(Downloader.class)` that nothing may call (Mockito's mock returns without effect; the
/// tests that use it never reach it).
struct UnusedDownloader;

impl Downloader for UnusedDownloader {
    fn download(
        &self,
        _urls: &[JavaUri],
        _headers: &Headers,
        _credentials: Arc<dyn Credentials>,
        _checksum: Option<&Checksum>,
        _canonical_id: &str,
        _output: &Path,
        _event_handler: Arc<dyn ExtendedEventHandler>,
        _client_env: &HashMap<String, String>,
        _type: Option<&str>,
        _context: &str,
    ) -> Result<(), IoException> {
        Ok(())
    }
}

struct Fixture {
    server: FakeServer,
    event_handler: Arc<StoredEventHandler>,
    remote_options: RemoteOptions,
    scratch: tempfile::TempDir,
}

impl Fixture {
    fn new() -> Fixture {
        // BlazeVersionInfo of the test runtime.
        blaze_remote::util::tracing_metadata_utils::set_tool_version("9.3.2");
        Fixture {
            server: FakeServer::new(),
            event_handler: Arc::new(StoredEventHandler::new()),
            remote_options: blaze_options::Options::<RemoteOptions>::get_defaults(),
            scratch: tempfile::tempdir().unwrap(),
        }
    }

    // Java: src/test/java/com/google/devtools/build/lib/remote/downloader/GrpcRemoteDownloaderTest.java:146 GrpcRemoteDownloaderTest.newDownloader
    fn new_downloader(
        &mut self,
        cache_client: Arc<dyn RemoteCacheClient>,
        http_downloader: Arc<dyn Downloader>,
    ) -> GrpcRemoteDownloader {
        self.server.start();
        let options = self.remote_options.clone();
        let retrier = Arc::new(RemoteRetrier::with_backoff(
            Arc::new(move || -> Arc<dyn Backoff> {
                Arc::new(ExponentialBackoff::from_options(&options))
            }),
            blaze_remote::remote_retrier::EXPERIMENTAL_GRPC_RESULT_CLASSIFIER.clone(),
            Arc::new(blaze_remote::retrier::TokioRetryScheduler),
            ALLOW_ALL_CALLS.clone(),
        ));
        let channel = self.server.channel(Vec::new(), None);
        GrpcRemoteDownloader::new(
            "none".to_string(),
            "none".to_string(),
            channel.retain(),
            None,
            retrier,
            cache_client,
            digest_util().get_digest_function(),
            Arc::new(self.remote_options.clone()),
            /* verbose_failures= */ false,
            http_downloader,
            self.remote_options.remote_downloader_local_fallback,
        )
    }

    // Java: src/test/java/com/google/devtools/build/lib/remote/downloader/GrpcRemoteDownloaderTest.java:186 GrpcRemoteDownloaderTest.downloadBlob
    fn download_blob(
        &self,
        downloader: &GrpcRemoteDownloader,
        url: &str,
        checksum: Option<Checksum>,
    ) -> Result<Vec<u8>, IoException> {
        let fs = blaze_vfs::unix_file_system::UnixFileSystem::create(DigestHashFunction::Sha256);
        let destination = Path::create(
            blaze_vfs::PathFragment::create(
                &self
                    .scratch
                    .path()
                    .join("output file path")
                    .display()
                    .to_string(),
            ),
            &fs,
        );
        downloader.download(
            &[JavaUri::create(url).unwrap()],
            &Headers::new(),
            Arc::new(StaticCredentials::empty()),
            checksum.as_ref(),
            "",
            &destination,
            self.event_handler.clone(),
            &HashMap::new(),
            None,
            "context",
        )?;
        Ok(std::fs::read(self.scratch.path().join("output file path")).unwrap())
    }

    fn fetch_events(&self) -> Vec<FetchEvent> {
        self.event_handler
            .get_posts()
            .iter()
            .filter_map(|p| {
                (p.as_ref() as &dyn std::any::Any)
                    .downcast_ref::<FetchEvent>()
                    .cloned()
            })
            .collect()
    }
}

fn upload(cache_client: &Arc<InMemoryCacheClient>, digest: &Digest, content: &[u8]) {
    let client: Arc<dyn RemoteCacheClient> = cache_client.clone();
    get_from_future(client.upload_bytes(
        &context(),
        digest,
        bytes::Bytes::copy_from_slice(content),
        false,
    ))
    .unwrap();
}

/// The `oldestContentAccepted` the request of a download without a checksum carries:
/// `now + 1h`, within the test's run time.
fn assert_oldest_content_accepted_about_an_hour_ahead(request: &FetchBlobRequest) {
    let t = request
        .oldest_content_accepted
        .as_ref()
        .expect("oldest_content_accepted");
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    assert!(
        (t.seconds - (now + 3600)).abs() <= 60,
        "{} vs {}",
        t.seconds,
        now + 3600
    );
}

// Java: src/test/java/com/google/devtools/build/lib/remote/downloader/GrpcRemoteDownloaderTest.java:214 GrpcRemoteDownloaderTest.testDownload
#[test]
fn test_download() {
    let mut f = Fixture::new();
    let content = b"example content".to_vec();
    let content_digest = digest_util().compute(&content);
    let response_digest = content_digest.clone();
    f.server.service_registry.add_fetch(FakeFetch {
        fetch_blob: Some(Box::new(move |request| {
            let request = request.into_inner();
            assert_eq!(
                request.digest_function,
                digest_util().get_digest_function() as i32
            );
            assert_eq!(request.uris, vec!["http://example.com/content.txt"]);
            assert!(request.qualifiers.is_empty());
            assert_oldest_content_accepted_about_an_hour_ahead(&request);
            Ok(FetchBlobResponse {
                blob_digest: Some(response_digest.clone()),
                ..Default::default()
            })
        })),
    });
    let cache_client = Arc::new(InMemoryCacheClient::new());
    let downloader = f.new_downloader(cache_client.clone(), Arc::new(UnusedDownloader));
    upload(&cache_client, &content_digest, &content);

    let downloaded = f
        .download_blob(&downloader, "http://example.com/content.txt", None)
        .unwrap();

    assert_eq!(downloaded, content);
    assert!(f.fetch_events().contains(&FetchEvent {
        url: "http://example.com/content.txt".to_string(),
        downloader: "GRPC",
        success: true,
    }));
}

// Java: src/test/java/com/google/devtools/build/lib/remote/downloader/GrpcRemoteDownloaderTest.java:255 GrpcRemoteDownloaderTest.testDownloadFallback
#[test]
fn test_download_fallback() {
    let mut f = Fixture::new();
    f.remote_options.remote_downloader_local_fallback = true;
    let content = b"example content".to_vec();
    f.server.service_registry.add_fetch(FakeFetch {
        fetch_blob: Some(Box::new(|_request| {
            // responseObserver.onError(new IOException("io error")): UNKNOWN.
            Err(tonic::Status::unknown("io error"))
        })),
    });
    let cache_client = Arc::new(InMemoryCacheClient::new());
    let fallback = Arc::new(FallbackDownloader {
        urls: vec![JavaUri::create("http://example.com/content.txt").unwrap()],
        content: content.clone(),
        calls: Mutex::new(Vec::new()),
    });
    let downloader = f.new_downloader(cache_client, fallback);

    let downloaded = f
        .download_blob(&downloader, "http://example.com/content.txt", None)
        .unwrap();

    assert_eq!(downloaded, content);
    assert_eq!(
        f.fetch_events(),
        vec![FetchEvent {
            url: "http://example.com/content.txt".to_string(),
            downloader: "GRPC",
            success: false,
        }]
    );
}

// Java: src/test/java/com/google/devtools/build/lib/remote/downloader/GrpcRemoteDownloaderTest.java:293 GrpcRemoteDownloaderTest.testFileUrl
#[test]
fn test_file_url() {
    let mut f = Fixture::new();
    let content = b"example content".to_vec();
    f.server.service_registry.add_fetch(FakeFetch {
        fetch_blob: Some(Box::new(|_request| Err(tonic::Status::unknown("io error")))),
    });
    let cache_client = Arc::new(InMemoryCacheClient::new());
    let fallback = Arc::new(FallbackDownloader {
        urls: vec![JavaUri::create("file:///my/local/file").unwrap()],
        content: content.clone(),
        calls: Mutex::new(Vec::new()),
    });
    let downloader = f.new_downloader(cache_client, fallback);

    let downloaded = f
        .download_blob(&downloader, "file:///my/local/file", None)
        .unwrap();

    assert_eq!(downloaded, content);
    assert!(f.fetch_events().is_empty());
}

// Java: src/test/java/com/google/devtools/build/lib/remote/downloader/GrpcRemoteDownloaderTest.java:326 GrpcRemoteDownloaderTest.testStatusHandling
#[test]
fn test_status_handling() {
    let mut f = Fixture::new();
    f.server.service_registry.add_fetch(FakeFetch {
        fetch_blob: Some(Box::new(|request| {
            let request = request.into_inner();
            assert_eq!(request.uris, vec!["http://example.com/content.txt"]);
            assert_oldest_content_accepted_about_an_hour_ahead(&request);
            Ok(FetchBlobResponse {
                status: Some(blaze_remote::reapi::rpc::Status {
                    code: tonic::Code::PermissionDenied as i32,
                    message: "permission denied".to_string(),
                    ..Default::default()
                }),
                uri: "http://example.com/other.txt".to_string(),
                ..Default::default()
            })
        })),
    });
    let cache_client = Arc::new(InMemoryCacheClient::new());
    let downloader = f.new_downloader(cache_client.clone(), Arc::new(UnusedDownloader));
    // Add a cache entry for the empty Digest to verify that the implementation checks the
    // status before fetching the digest.
    upload(&cache_client, &Digest::default(), b"");

    let e = f
        .download_blob(&downloader, "http://example.com/content.txt", None)
        .unwrap_err();
    assert!(e.get_message().contains("permission denied"), "{e:?}");
    assert_eq!(
        f.fetch_events(),
        vec![FetchEvent {
            url: "http://example.com/other.txt".to_string(),
            downloader: "GRPC",
            success: false,
        }]
    );
}

fn checksum_fetch(content_digest: Digest) -> FakeFetch {
    FakeFetch {
        fetch_blob: Some(Box::new(move |request| {
            let request = request.into_inner();
            assert_eq!(
                request,
                FetchBlobRequest {
                    digest_function: digest_util().get_digest_function() as i32,
                    uris: vec!["http://example.com/content.txt".to_string()],
                    qualifiers: vec![Qualifier {
                        name: "checksum.sri".to_string(),
                        value: "sha256-ot7ke6YmiSXal3UKt0K69n8C4vtUziPUmftmpbAiKQM=".to_string(),
                    }],
                    ..Default::default()
                }
            );
            Ok(FetchBlobResponse {
                blob_digest: Some(content_digest.clone()),
                ..Default::default()
            })
        })),
    }
}

// Java: src/test/java/com/google/devtools/build/lib/remote/downloader/GrpcRemoteDownloaderTest.java:374 GrpcRemoteDownloaderTest.testPropagateChecksum
#[test]
fn test_propagate_checksum() {
    let mut f = Fixture::new();
    let content = b"example content".to_vec();
    let content_digest = digest_util().compute(&content);
    f.server
        .service_registry
        .add_fetch(checksum_fetch(content_digest.clone()));
    let cache_client = Arc::new(InMemoryCacheClient::new());
    let downloader = f.new_downloader(cache_client.clone(), Arc::new(UnusedDownloader));
    upload(&cache_client, &content_digest, &content);

    let downloaded = f
        .download_blob(
            &downloader,
            "http://example.com/content.txt",
            Some(Checksum::from_string(KeyType::SHA256, &content_digest.hash).unwrap()),
        )
        .unwrap();

    assert_eq!(downloaded, content);
}

// Java: src/test/java/com/google/devtools/build/lib/remote/downloader/GrpcRemoteDownloaderTest.java:415 GrpcRemoteDownloaderTest.testRejectChecksumMismatch
#[test]
fn test_reject_checksum_mismatch() {
    let mut f = Fixture::new();
    let content = b"example content".to_vec();
    let content_digest = digest_util().compute(&content);
    f.server
        .service_registry
        .add_fetch(checksum_fetch(content_digest.clone()));
    let cache_client = Arc::new(InMemoryCacheClient::new());
    let downloader = f.new_downloader(cache_client.clone(), Arc::new(UnusedDownloader));
    upload(&cache_client, &content_digest, b"wrong content");

    let e = f
        .download_blob(
            &downloader,
            "http://example.com/content.txt",
            Some(Checksum::from_string(KeyType::SHA256, &content_digest.hash).unwrap()),
        )
        .unwrap_err();

    assert_eq!(e.kind, IoExceptionKind::UnrecoverableHttpException, "{e:?}");
    assert!(e.get_message().contains(&content_digest.hash), "{e:?}");
    assert!(
        e.get_message()
            .contains(&digest_util().compute_as_utf8("wrong content").hash),
        "{e:?}"
    );
}

fn sri_checksum() -> Checksum {
    Checksum::from_subresource_integrity("sha256-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=")
        .unwrap()
}

fn now() -> SystemTime {
    SystemTime::now()
}

// Java: src/test/java/com/google/devtools/build/lib/remote/downloader/GrpcRemoteDownloaderTest.java:461 GrpcRemoteDownloaderTest.testFetchBlobRequest
#[test]
fn test_fetch_blob_request() {
    let mut headers = Headers::new();
    headers.insert(
        "Authorization".to_string(),
        vec!["Basic Zm9vOmJhcg==".to_string()],
    );
    headers.insert(
        "X-Custom-Token".to_string(),
        vec!["foo".to_string(), "bar".to_string()],
    );
    let request = GrpcRemoteDownloader::new_fetch_blob_request(
        "instance name",
        false,
        &[
            JavaUri::create("http://example.com/a").unwrap(),
            JavaUri::create("http://example.com/b").unwrap(),
            JavaUri::create("file:/not/limited/to/http").unwrap(),
        ],
        Some(&sri_checksum()),
        "canonical ID",
        digest_util().get_digest_function(),
        &headers,
        &StaticCredentials::empty(),
        now(),
    )
    .unwrap();

    let q = |name: &str, value: &str| Qualifier {
        name: name.to_string(),
        value: value.to_string(),
    };
    assert_eq!(
        request,
        FetchBlobRequest {
            instance_name: "instance name".to_string(),
            digest_function: digest_util().get_digest_function() as i32,
            uris: vec![
                "http://example.com/a".to_string(),
                "http://example.com/b".to_string(),
                "file:/not/limited/to/http".to_string(),
            ],
            qualifiers: vec![
                q(
                    "checksum.sri",
                    "sha256-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA="
                ),
                q("bazel.canonical_id", "canonical ID"),
                q("http_header:Authorization", "Basic Zm9vOmJhcg=="),
                q("http_header:X-Custom-Token", "foo,bar"),
            ],
            ..Default::default()
        }
    );
}

/// `mock(Credentials.class)` answering `{CredKey: [CredValue]}` for the url.
struct CredKeyCredentials;

impl Credentials for CredKeyCredentials {
    fn get_request_metadata(&self, _uri: &JavaUri) -> Result<Headers, IoException> {
        let mut headers = Headers::new();
        headers.insert("CredKey".to_string(), vec!["CredValue".to_string()]);
        Ok(headers)
    }
}

// Java: src/test/java/com/google/devtools/build/lib/remote/downloader/GrpcRemoteDownloaderTest.java:506 GrpcRemoteDownloaderTest.testFetchBlobRequest_withCredentialsPropagation
#[test]
fn test_fetch_blob_request_with_credentials_propagation() {
    let request = GrpcRemoteDownloader::new_fetch_blob_request(
        "instance name",
        true,
        &[JavaUri::create("http://example.com/a").unwrap()],
        Some(&sri_checksum()),
        "canonical ID",
        digest_util().get_digest_function(),
        &Headers::new(),
        &CredKeyCredentials,
        now(),
    )
    .unwrap();
    let names: Vec<(&str, &str)> = request
        .qualifiers
        .iter()
        .map(|q| (q.name.as_str(), q.value.as_str()))
        .collect();
    assert_eq!(
        names,
        vec![
            ("http_header_url:0:CredKey", "CredValue"),
            (
                "checksum.sri",
                "sha256-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA="
            ),
            ("bazel.canonical_id", "canonical ID"),
        ]
    );
}

// Java: src/test/java/com/google/devtools/build/lib/remote/downloader/GrpcRemoteDownloaderTest.java:549 GrpcRemoteDownloaderTest.testFetchBlobRequest_withoutCredentialsPropagation
#[test]
fn test_fetch_blob_request_without_credentials_propagation() {
    let request = GrpcRemoteDownloader::new_fetch_blob_request(
        "instance name",
        false,
        &[JavaUri::create("http://example.com/a").unwrap()],
        Some(&sri_checksum()),
        "canonical ID",
        digest_util().get_digest_function(),
        &Headers::new(),
        &CredKeyCredentials,
        now(),
    )
    .unwrap();
    let names: Vec<&str> = request.qualifiers.iter().map(|q| q.name.as_str()).collect();
    assert_eq!(names, vec!["checksum.sri", "bazel.canonical_id"]);
}

// Java: src/test/java/com/google/devtools/build/lib/remote/downloader/GrpcRemoteDownloaderTest.java:588 GrpcRemoteDownloaderTest.testFetchBlobRequest_withoutChecksum
#[test]
fn test_fetch_blob_request_without_checksum() {
    // ManualClock: a fixed instant.
    let clock = UNIX_EPOCH + Duration::from_millis(1_700_000_000_123);
    let request = GrpcRemoteDownloader::new_fetch_blob_request(
        "instance name",
        false,
        &[JavaUri::create("http://example.com/").unwrap()],
        None,
        "canonical ID",
        digest_util().get_digest_function(),
        &Headers::new(),
        &StaticCredentials::empty(),
        clock,
    )
    .unwrap();
    let expected_millis: i64 = 1_700_000_000_123 + 3_600_000;
    assert_eq!(
        request,
        FetchBlobRequest {
            instance_name: "instance name".to_string(),
            digest_function: digest_util().get_digest_function() as i32,
            uris: vec!["http://example.com/".to_string()],
            oldest_content_accepted: Some(blaze_protos::google::protobuf::Timestamp {
                seconds: expected_millis / 1000,
                nanos: ((expected_millis % 1000) * 1_000_000) as i32,
            }),
            qualifiers: vec![Qualifier {
                name: "bazel.canonical_id".to_string(),
                value: "canonical ID".to_string(),
            }],
            ..Default::default()
        }
    );
}
