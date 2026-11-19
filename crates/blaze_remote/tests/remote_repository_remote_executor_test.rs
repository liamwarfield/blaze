// Options and protos are filled in field by field, as the Java tests set them.
#![allow(clippy::field_reassign_with_default)]
//! Port of `src/test/java/com/google/devtools/build/lib/remote/RemoteRepositoryRemoteExecutorTest.java`.
//!
//! PORT: Java mocks `RemoteExecutionCache` (a class) with Mockito. Here the cache is a real
//! `RemoteExecutionCache` over a mocked `RemoteCacheClient`: `downloadActionResult` answers the
//! stubbed result and records its arguments (the `verify(remoteCache).downloadActionResult(...)`
//! of the tests), and the uploads of `ensureInputsPresent` succeed without effect (Mockito's
//! do-nothing answer of the mocked `ensureInputsPresent`). `RemoteExecutionClient` is an
//! interface and is mocked directly.

use std::collections::BTreeMap;
use std::io::{self, Write};
use std::sync::Arc;

use async_trait::async_trait;
use blaze_remote::common::{
    ActionKey, Blob, MissingDigestsFinder, OperationObserver, RemoteActionExecutionContext,
    RemoteCacheClient, RemoteExecutionClient,
};
use blaze_remote::reapi::{
    ActionResult, Digest, ExecuteRequest, ExecuteResponse, ServerCapabilities,
};
use blaze_remote::remote_execution_cache::RemoteExecutionCache;
use blaze_remote::remote_repository_remote_executor::{
    RemoteRepositoryRemoteExecutor, RepositoryRemoteExecutor,
};
use blaze_remote::util::DigestUtil;
use blaze_remote::util::async_task_cache::{AsyncTaskCache, AsyncTaskCacheNoResult};
use blaze_remote::util::tracing_metadata_utils::set_tool_version;
use blaze_vfs::{DigestHashFunction, Path, PathFragment, UnixFileSystem};
use indexmap::{IndexMap, IndexSet};
use parking_lot::Mutex;

/// `TestConstants.WORKSPACE_NAME`.
const WORKSPACE_NAME: &str = "_main";

/// `DIGEST_UTIL`.
fn digest_util() -> DigestUtil {
    DigestUtil::new(DigestHashFunction::Sha256)
}

/// The arguments of one `downloadActionResult` call.
#[derive(Debug, Clone, PartialEq, Eq)]
struct DownloadActionResultCall {
    inline_out_err: bool,
    inline_output_files: IndexSet<String>,
}

/// `@Mock RemoteExecutionCache remoteCache`, as the client under a real cache.
struct MockRemoteCacheClient {
    cas_upload_cache: AsyncTaskCacheNoResult<Digest>,
    /// `when(remoteCache.downloadActionResult(any(), any(), eq(true), eq(ImmutableSet.of())))
    /// .thenReturn(CachedActionResult.remote(cachedResult))`.
    action_result: Mutex<Option<ActionResult>>,
    download_action_result_calls: Mutex<Vec<DownloadActionResultCall>>,
}

impl MockRemoteCacheClient {
    fn new() -> MockRemoteCacheClient {
        MockRemoteCacheClient {
            cas_upload_cache: AsyncTaskCache::create(),
            action_result: Mutex::new(None),
            download_action_result_calls: Mutex::new(Vec::new()),
        }
    }
}

#[async_trait]
impl MissingDigestsFinder for MockRemoteCacheClient {
    async fn find_missing_digests(
        &self,
        _context: &RemoteActionExecutionContext<'_>,
        _digests: &[Digest],
    ) -> io::Result<IndexSet<Digest>> {
        Ok(IndexSet::new())
    }
}

#[async_trait]
impl RemoteCacheClient for MockRemoteCacheClient {
    fn cas_upload_cache(&self) -> &AsyncTaskCacheNoResult<Digest> {
        &self.cas_upload_cache
    }

    fn get_server_capabilities(&self) -> io::Result<ServerCapabilities> {
        Ok(ServerCapabilities::default())
    }

    async fn get_authority(&self) -> io::Result<String> {
        Ok(String::new())
    }

    async fn download_action_result(
        &self,
        _context: &RemoteActionExecutionContext<'_>,
        _action_key: &ActionKey,
        inline_out_err: bool,
        inline_output_files: &IndexSet<String>,
    ) -> io::Result<Option<ActionResult>> {
        self.download_action_result_calls
            .lock()
            .push(DownloadActionResultCall {
                inline_out_err,
                inline_output_files: inline_output_files.clone(),
            });
        // The stub matches `eq(true)` and `eq(ImmutableSet.of())` only; Mockito answers null
        // otherwise.
        if inline_out_err && inline_output_files.is_empty() {
            Ok(self.action_result.lock().clone())
        } else {
            Ok(None)
        }
    }

    async fn upload_action_result(
        &self,
        _context: &RemoteActionExecutionContext<'_>,
        _action_key: &ActionKey,
        _action_result: &ActionResult,
    ) -> io::Result<()> {
        Ok(())
    }

    async fn download_blob(
        &self,
        _context: &RemoteActionExecutionContext<'_>,
        _digest: &Digest,
        _out: &mut (dyn Write + Send),
    ) -> io::Result<()> {
        panic!("unexpected call: downloadBlob")
    }

    async fn upload_blob_impl(
        &self,
        _context: &RemoteActionExecutionContext<'_>,
        _digest: &Digest,
        _blob: &dyn Blob,
    ) -> io::Result<()> {
        Ok(())
    }

    fn close(&self) {}
}

/// `@Mock RemoteExecutionClient remoteExecutor`.
struct MockRemoteExecutionClient {
    /// `when(remoteExecutor.executeRemotely(any(), any(), any())).thenReturn(response)`.
    response: Mutex<Option<ExecuteResponse>>,
    execute_remotely_calls: Mutex<i32>,
}

impl RemoteExecutionClient for MockRemoteExecutionClient {
    fn get_server_capabilities(&self) -> io::Result<ServerCapabilities> {
        Ok(ServerCapabilities::default())
    }

    fn execute_remotely(
        &self,
        _context: &RemoteActionExecutionContext<'_>,
        _request: ExecuteRequest,
        _observer: &dyn OperationObserver,
    ) -> io::Result<ExecuteResponse> {
        *self.execute_remotely_calls.lock() += 1;
        match self.response.lock().clone() {
            Some(response) => Ok(response),
            None => panic!("NullPointerException: executeRemotely is not stubbed"),
        }
    }

    fn close(&self) {}
}

struct RemoteRepositoryRemoteExecutorTest {
    _tmp: tempfile::TempDir,
    remote_cache_client: Arc<MockRemoteCacheClient>,
    remote_executor: Arc<MockRemoteExecutionClient>,
    repo_executor: RemoteRepositoryRemoteExecutor,
}

impl RemoteRepositoryRemoteExecutorTest {
    // Java: RemoteRepositoryRemoteExecutorTest.setup
    fn setup() -> RemoteRepositoryRemoteExecutorTest {
        // `TracingMetadataUtils.buildMetadata` reads the version of the running binary.
        set_tool_version("test");
        let tmp = tempfile::tempdir().unwrap();
        let fs = UnixFileSystem::create(DigestHashFunction::Sha256);
        let exec_root = Path::create(PathFragment::create(tmp.path().to_str().unwrap()), &fs);

        let remote_cache_client = Arc::new(MockRemoteCacheClient::new());
        let remote_cache = Arc::new(RemoteExecutionCache::new(
            remote_cache_client.clone(),
            /* disk_cache_client= */ None,
            /* symlink_template= */ None,
            digest_util(),
            /* chunking_function= */ None,
        ));
        let remote_executor = Arc::new(MockRemoteExecutionClient {
            response: Mutex::new(None),
            execute_remotely_calls: Mutex::new(0),
        });
        let repo_executor = RemoteRepositoryRemoteExecutor::new(
            remote_cache,
            remote_executor.clone(),
            digest_util(),
            "none",
            "none",
            WORKSPACE_NAME,
            /* remote_instance_name= */ "foo",
            /* accept_cached= */ true,
            exec_root,
        );
        RemoteRepositoryRemoteExecutorTest {
            _tmp: tmp,
            remote_cache_client,
            remote_executor,
            repo_executor,
        }
    }

    /// `verify(remoteCache).downloadActionResult(any(), any(), <inlineOutErr>, eq(ImmutableSet.of()))`:
    /// exactly one call, with the given `inlineOutErr` when it is not `anyBoolean()`.
    fn verify_download_action_result(&self, inline_out_err: Option<bool>) {
        let calls = self.remote_cache_client.download_action_result_calls.lock();
        let matching = calls
            .iter()
            .filter(|call| {
                inline_out_err.is_none_or(|i| i == call.inline_out_err)
                    && call.inline_output_files.is_empty()
            })
            .count();
        assert_eq!(matching, 1, "downloadActionResult calls: {calls:?}");
    }

    fn execute(
        &self,
        arguments: &[&str],
    ) -> blaze_remote::remote_repository_remote_executor::ExecutionResult {
        self.repo_executor
            .execute(
                &arguments.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
                /* input_files= */ &BTreeMap::new(),
                /* execution_properties= */ &IndexMap::new(),
                /* environment= */ &IndexMap::new(),
                /* working_directory= */ None,
                /* timeout= */ std::time::Duration::ZERO,
            )
            .unwrap()
    }
}

#[test]
fn test_zero_exit_code_from_cache() {
    let t = RemoteRepositoryRemoteExecutorTest::setup();
    // Test that an ActionResult with exit code zero is accepted as cached.

    let cached_result = ActionResult {
        exit_code: 0,
        ..Default::default()
    };
    *t.remote_cache_client.action_result.lock() = Some(cached_result);

    let execution_result = t.execute(&["/bin/bash", "-c", "exit 0"]);

    t.verify_download_action_result(/* anyBoolean() */ None);
    // Don't fallback to execution
    assert_eq!(*t.remote_executor.execute_remotely_calls.lock(), 0);

    assert_eq!(execution_result.exit_code(), 0);
}

#[test]
fn test_none_zero_exit_code_from_cache() {
    let t = RemoteRepositoryRemoteExecutorTest::setup();
    // Test that an ActionResult with a none-zero exit code is not accepted as cached.

    let cached_result = ActionResult {
        exit_code: 1,
        ..Default::default()
    };
    *t.remote_cache_client.action_result.lock() = Some(cached_result.clone());

    let response = ExecuteResponse {
        result: Some(cached_result),
        ..Default::default()
    };
    *t.remote_executor.response.lock() = Some(response);

    let execution_result = t.execute(&["/bin/bash", "-c", "exit 1"]);

    t.verify_download_action_result(/* anyBoolean() */ None);
    // Fallback to execution
    assert_eq!(*t.remote_executor.execute_remotely_calls.lock(), 1);

    assert_eq!(execution_result.exit_code(), 1);
}

#[test]
fn test_inline_stdout_stderr() {
    let t = RemoteRepositoryRemoteExecutorTest::setup();
    // Test that inline stdout/stderr responses are returned in execution results.

    let stdout = "hello".as_bytes().to_vec();
    let stderr = "world".as_bytes().to_vec();
    let cached_result = ActionResult {
        exit_code: 0,
        stdout_raw: stdout.clone(),
        stderr_raw: stderr.clone(),
        ..Default::default()
    };
    *t.remote_cache_client.action_result.lock() = Some(cached_result.clone());

    let response = ExecuteResponse {
        result: Some(cached_result),
        ..Default::default()
    };
    *t.remote_executor.response.lock() = Some(response);

    let execution_result = t.execute(&["/bin/bash", "-c", "echo hello"]);

    t.verify_download_action_result(/* inlineOutErr= */ Some(true));

    assert_eq!(execution_result.exit_code(), 0);
    assert_eq!(execution_result.stdout(), stdout.as_slice());
    assert_eq!(execution_result.stderr(), stderr.as_slice());
}
