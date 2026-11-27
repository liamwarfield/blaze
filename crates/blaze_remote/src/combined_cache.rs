//! Port of `lib/remote/CombinedCache.java`.

use std::io::{self, Write};
use std::sync::atomic::{AtomicI32, AtomicI64, Ordering};
use std::sync::{Arc, LazyLock};

use blaze_exec::exec::ProgressStatus;
use blaze_exec::util::FileOutErr;
use blaze_optdefs::misc::remote_options::ChunkingFunctionValue;
use blaze_protos::failure_details::{
    FailureDetail, RemoteExecution, failure_detail, remote_execution,
};
use blaze_repo_fetch::downloader::download_progress_event::bytes_count_to_display_string;
use blaze_repo_fetch::fs_utils;
use blaze_vfs::{Path, PathFragment};
use futures::future::BoxFuture;
use indexmap::IndexSet;
use parking_lot::{Condvar, Mutex};
use regex::Regex;

use crate::chunked_blob_downloader::ChunkedBlobDownloader;
use crate::chunked_blob_uploader::{CacheClients, ChunkedBlobUploader};
use crate::chunking::ChunkingConfig;
use crate::common::BlobNotSplittableException;
use crate::common::cache_not_found_exception::CacheNotFoundException;
use crate::common::errors::{self, downcast, downcast_ref};
use crate::common::lazy_file_output_stream::LazyFileOutputStream;
use crate::common::output_digest_mismatch_exception::OutputDigestMismatchException;
use crate::common::progress_status_listener::{ProgressStatusListener, no_action};
use crate::common::remote_action_execution_context::RemoteActionExecutionContext;
use crate::common::remote_cache_client::{
    ActionKey, Blob, BytesBlob, RemoteCacheClient, RemoteCacheClientUploads, dedup_upload,
};
use crate::disk::DiskCacheClient;
use crate::grpc_cache_client::GrpcCacheClient;
use crate::reapi::{
    ActionResult, CacheCapabilities, Digest, ServerCapabilities, chunking_function,
};
use crate::runtime::get_from_future;
use crate::store::Store;
use crate::util::digest_util::DigestUtil;

pub use crate::common::progress_status_listener::SpawnProgressEvent;

// Java: com/google/devtools/build/lib/remote/CombinedCache.java:95 CombinedCache.SPAWN_CHECKING_DISK_CACHE_EVENT
fn spawn_checking_disk_cache_event() -> ProgressStatus {
    ProgressStatus::CheckingCache("disk-cache".to_string())
}

// Java: com/google/devtools/build/lib/remote/CombinedCache.java:97 CombinedCache.SPAWN_CHECKING_REMOTE_CACHE_EVENT
fn spawn_checking_remote_cache_event() -> ProgressStatus {
    ProgressStatus::CheckingCache("remote-cache".to_string())
}

/// Provides unified access to a disk cache, remote cache, or both.
///
/// The cache is reference counted. Initially, the reference count is 1. Use
/// [`CombinedCache::retain`] to increase and [`CombinedCache::release`] to decrease the reference
/// count respectively. Once the reference count is reached to 0, the underlying resources will be
/// released (after pending I/O is finished).
///
/// Use [`CombinedCache::await_termination`] to wait for pending I/O to finish. Use
/// [`CombinedCache::shutdown_now`] to cancel all pending I/O and reject new requests.
///
/// PORT: Java's subclass `RemoteExecutionCache` reads the `protected` fields; they are
/// `pub(crate)` here with public accessors. `virtualThreadExecutor` only runs the chunked
/// transfers, which are not ported, so there is no executor.
// Java: com/google/devtools/build/lib/remote/CombinedCache.java:90 CombinedCache
pub struct CombinedCache {
    /// `closeCountDownLatch` (a `CountDownLatch(1)`): whether `deallocate` ran.
    close_count_down_latch: Mutex<bool>,
    close_count_down_latch_released: Condvar,
    /// `AbstractReferenceCounted.refCnt`.
    ref_cnt: AtomicI32,

    pub(crate) remote_cache_client: Option<Arc<dyn RemoteCacheClient>>,
    pub(crate) disk_cache_client: Option<Arc<DiskCacheClient>>,
    pub(crate) symlink_template: Option<String>,
    pub(crate) digest_util: DigestUtil,
    chunking_function: Option<ChunkingFunctionValue>,
    /// `chunking` (`Chunking`'s `config` once `initialized`): `None` until the first
    /// [`CombinedCache::chunking_supported`] that got the server capabilities, then the
    /// negotiated configuration (itself `None` when the server advertises no chunking function).
    ///
    /// PORT: Java's `Chunking` also holds a `ChunkedBlobDownloader` and a `ChunkedBlobUploader`;
    /// both are stateless apart from the configuration and are made per chunked transfer here
    /// (the downloader borrows the cache, the uploader runs as a task that owns its clients).
    chunking: Mutex<Option<Option<ChunkingConfig>>>,
}

/// The body of `CombinedCache.uploadBlob(context, digest, blob, force)` over the cache's two
/// clients, so that a chunked upload (which owns the clients, see `ChunkedBlobUploader`) uploads
/// its chunks as the cache does.
// Java: com/google/devtools/build/lib/remote/CombinedCache.java:458 CombinedCache.uploadBlob
pub(crate) async fn upload_blob_to_clients(
    remote_cache_client: Option<&Arc<dyn RemoteCacheClient>>,
    disk_cache_client: Option<&Arc<DiskCacheClient>>,
    context: &RemoteActionExecutionContext<'_>,
    digest: &Digest,
    blob: Arc<dyn Blob>,
    force: bool,
) -> io::Result<()> {
    if digest.size_bytes == 0 {
        return Ok(());
    }

    let disk_cache_future = async {
        if let Some(disk_cache_client) = disk_cache_client
            && context.get_write_cache_policy().allow_disk_cache()
        {
            disk_cache_client.upload_blob(digest, &*blob).await
        } else {
            Ok(())
        }
    };

    let remote_cache_future = async {
        if let Some(remote_cache_client) = remote_cache_client
            && context.get_write_cache_policy().allow_remote_cache()
        {
            remote_cache_client
                .upload_blob(context, digest, blob.clone(), force)
                .await
        } else {
            Ok(())
        }
    };

    when_all_succeed(disk_cache_future, remote_cache_future).await
}

impl std::fmt::Debug for CombinedCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CombinedCache")
            .field("has_remote_cache", &self.has_remote_cache())
            .field("has_disk_cache", &self.has_disk_cache())
            .field("symlink_template", &self.symlink_template)
            .field("chunking_function", &self.chunking_function)
            .field("ref_cnt", &self.ref_cnt())
            .finish()
    }
}

/// Class to keep track of which cache (disk or remote) a given \[cached\] ActionResult comes
/// from.
// Java: com/google/devtools/build/lib/remote/CombinedCache.java:204 CombinedCache.CachedActionResult
#[derive(Debug, Clone, PartialEq)]
pub struct CachedActionResult {
    action_result: ActionResult,
    cache_name: String,
}

impl CachedActionResult {
    /// `new CachedActionResult(actionResult, cacheName)`.
    pub fn new(action_result: ActionResult, cache_name: &str) -> CachedActionResult {
        CachedActionResult {
            action_result,
            cache_name: cache_name.to_string(),
        }
    }

    /// `CachedActionResult.remote(actionResult)`.
    // Java: com/google/devtools/build/lib/remote/CombinedCache.java:206 CachedActionResult.remote
    pub fn remote(action_result: Option<ActionResult>) -> Option<CachedActionResult> {
        let action_result = action_result?;
        Some(CachedActionResult::new(action_result, "remote"))
    }

    /// `CachedActionResult.disk(actionResult)`.
    // Java: com/google/devtools/build/lib/remote/CombinedCache.java:214 CachedActionResult.disk
    pub fn disk(action_result: Option<ActionResult>) -> Option<CachedActionResult> {
        let action_result = action_result?;
        Some(CachedActionResult::new(action_result, "disk"))
    }

    /// `actionResult()`.
    pub fn action_result(&self) -> &ActionResult {
        &self.action_result
    }

    /// `cacheName()`.
    pub fn cache_name(&self) -> &str {
        &self.cache_name
    }
}

impl CombinedCache {
    /// `new CombinedCache(remoteCacheClient, diskCacheClient, symlinkTemplate, digestUtil,
    /// chunkingFunction)`.
    // Java: com/google/devtools/build/lib/remote/CombinedCache.java:167 CombinedCache.<init>
    pub fn new(
        remote_cache_client: Option<Arc<dyn RemoteCacheClient>>,
        disk_cache_client: Option<Arc<DiskCacheClient>>,
        symlink_template: Option<String>,
        digest_util: DigestUtil,
        chunking_function: Option<ChunkingFunctionValue>,
    ) -> CombinedCache {
        blaze_util::check_argument!(
            remote_cache_client.is_some() || disk_cache_client.is_some(),
            "remoteCacheClient and diskCacheClient cannot be null at the same time"
        );
        CombinedCache {
            close_count_down_latch: Mutex::new(false),
            close_count_down_latch_released: Condvar::new(),
            ref_cnt: AtomicI32::new(1),
            remote_cache_client,
            disk_cache_client,
            symlink_template,
            digest_util,
            chunking_function,
            chunking: Mutex::new(None),
        }
    }

    /// `remoteCacheClient` (protected in Java).
    pub fn remote_cache_client(&self) -> Option<&Arc<dyn RemoteCacheClient>> {
        self.remote_cache_client.as_ref()
    }

    /// `diskCacheClient` (protected in Java).
    pub fn disk_cache_client(&self) -> Option<&Arc<DiskCacheClient>> {
        self.disk_cache_client.as_ref()
    }

    /// `symlinkTemplate` (protected in Java).
    pub fn symlink_template(&self) -> Option<&str> {
        self.symlink_template.as_deref()
    }

    /// `digestUtil` (protected in Java).
    pub fn digest_util(&self) -> &DigestUtil {
        &self.digest_util
    }

    /// `Chunking.supported()`: delays the initialization of the chunking support logic until
    /// first use to avoid blocking on a server capabilities check at construction time.
    // Java: com/google/devtools/build/lib/remote/CombinedCache.java:122 CombinedCache.Chunking.supported
    fn chunking_supported(&self) -> io::Result<bool> {
        let Some(chunking_function) = self.chunking_function else {
            return Ok(false);
        };
        if self.grpc_cache_client().is_none() {
            // `!(remoteCacheClient instanceof GrpcCacheClient)`.
            return Ok(false);
        }
        let mut chunking = self.chunking.lock();
        if chunking.is_none() {
            // synchronized (this): the capabilities are fetched once; a failure leaves the
            // state uninitialized, as Java's `initialized` stays false.
            let capabilities = self.get_remote_server_capabilities()?;
            *chunking = Some(match chunking_function {
                ChunkingFunctionValue::AUTO => {
                    ChunkingConfig::from_server_capabilities(&capabilities)
                }
                ChunkingFunctionValue::FAST_CDC_2020 => {
                    ChunkingConfig::from_server_capabilities_for(
                        &capabilities,
                        chunking_function::Value::FastCdc2020,
                    )
                }
                ChunkingFunctionValue::REP_MAX_CDC => ChunkingConfig::from_server_capabilities_for(
                    &capabilities,
                    chunking_function::Value::RepMaxCdc,
                ),
            });
        }
        Ok(chunking.expect("initialized above").is_some())
    }

    /// `Chunking.config()`.
    // Java: com/google/devtools/build/lib/remote/CombinedCache.java:152 CombinedCache.Chunking.config
    fn chunking_config(&self) -> ChunkingConfig {
        self.chunking
            .lock()
            .flatten()
            .expect("must not call config() unless supported() is true")
    }

    /// `remoteCacheClient instanceof GrpcCacheClient grpcClient ? grpcClient : null`.
    fn grpc_cache_client(&self) -> Option<&GrpcCacheClient> {
        self.remote_cache_client
            .as_ref()?
            .as_any()?
            .downcast_ref::<GrpcCacheClient>()
    }

    /// `getRemoteCacheCapabilities()`.
    // Java: com/google/devtools/build/lib/remote/CombinedCache.java:183 CombinedCache.getRemoteCacheCapabilities
    pub fn get_remote_cache_capabilities(&self) -> io::Result<CacheCapabilities> {
        Ok(self
            .get_remote_server_capabilities()?
            .cache_capabilities
            .unwrap_or_default())
    }

    /// `getRemoteAuthority()`.
    // Java: com/google/devtools/build/lib/remote/CombinedCache.java:187 CombinedCache.getRemoteAuthority
    pub async fn get_remote_authority(&self) -> io::Result<String> {
        let Some(remote_cache_client) = &self.remote_cache_client else {
            return Ok(String::new());
        };
        remote_cache_client.get_authority().await
    }

    /// `getRemoteServerCapabilities()`.
    // Java: com/google/devtools/build/lib/remote/CombinedCache.java:194 CombinedCache.getRemoteServerCapabilities
    pub fn get_remote_server_capabilities(&self) -> io::Result<ServerCapabilities> {
        let Some(remote_cache_client) = &self.remote_cache_client else {
            return Ok(ServerCapabilities::default());
        };
        remote_cache_client.get_server_capabilities()
    }

    /// `downloadActionResult(context, actionKey, inlineOutErr, inlineOutputFiles)` (blocking).
    // Java: com/google/devtools/build/lib/remote/CombinedCache.java:222 CombinedCache.downloadActionResult
    pub fn download_action_result(
        &self,
        context: &RemoteActionExecutionContext<'_>,
        action_key: &ActionKey,
        inline_out_err: bool,
        inline_output_files: &IndexSet<String>,
    ) -> io::Result<Option<CachedActionResult>> {
        get_from_future(self.download_action_result_async(
            context,
            action_key,
            inline_out_err,
            inline_output_files,
        ))
    }

    /// `downloadActionResultAsync(context, actionKey, inlineOutErr, inlineOutputFiles)`.
    // Java: com/google/devtools/build/lib/remote/CombinedCache.java:232 CombinedCache.downloadActionResultAsync
    pub async fn download_action_result_async(
        &self,
        context: &RemoteActionExecutionContext<'_>,
        action_key: &ActionKey,
        inline_out_err: bool,
        inline_output_files: &IndexSet<String>,
    ) -> io::Result<Option<CachedActionResult>> {
        let spawn_execution_context = context.get_spawn_execution_context();

        let mut result: Option<CachedActionResult> = None;

        if let Some(disk_cache_client) = &self.disk_cache_client
            && context.get_read_cache_policy().allow_disk_cache()
        {
            if let Some(spawn_execution_context) = spawn_execution_context {
                spawn_execution_context.report(spawn_checking_disk_cache_event());
            }
            result = CachedActionResult::disk(
                disk_cache_client.download_action_result(action_key).await?,
            );
        }

        if self.remote_cache_client.is_some()
            && context.get_read_cache_policy().allow_remote_cache()
            && result.is_none()
        {
            if let Some(spawn_execution_context) = spawn_execution_context {
                spawn_execution_context.report(spawn_checking_remote_cache_event());
            }
            result = CachedActionResult::remote(
                self.download_action_result_from_remote(
                    context,
                    action_key,
                    inline_out_err,
                    inline_output_files,
                )
                .await?,
            );
        }

        Ok(result)
    }

    // Java: com/google/devtools/build/lib/remote/CombinedCache.java:276 CombinedCache.downloadActionResultFromRemote
    async fn download_action_result_from_remote(
        &self,
        context: &RemoteActionExecutionContext<'_>,
        action_key: &ActionKey,
        inline_out_err: bool,
        inline_output_files: &IndexSet<String>,
    ) -> io::Result<Option<ActionResult>> {
        blaze_util::check_state!(
            self.remote_cache_client.is_some()
                && context.get_read_cache_policy().allow_remote_cache()
        );
        let remote_cache_client = self.remote_cache_client.as_ref().unwrap();
        let Some(action_result) = remote_cache_client
            .download_action_result(context, action_key, inline_out_err, inline_output_files)
            .await?
        else {
            return Ok(None);
        };

        if let Some(disk_cache_client) = &self.disk_cache_client
            && context.get_write_cache_policy().allow_disk_cache()
        {
            disk_cache_client
                .upload_action_result(action_key, &action_result)
                .await?;
            return Ok(Some(action_result));
        }

        Ok(Some(action_result))
    }

    /// Returns a set of digests that the remote cache does not know about. The returned set is
    /// guaranteed to be a subset of `digests`.
    // Java: com/google/devtools/build/lib/remote/CombinedCache.java:305 CombinedCache.findMissingDigests
    pub async fn find_missing_digests(
        &self,
        context: &RemoteActionExecutionContext<'_>,
        digests: &[Digest],
    ) -> io::Result<IndexSet<Digest>> {
        if digests.is_empty() {
            return Ok(IndexSet::new());
        }

        let disk_query = async {
            if let Some(disk_cache_client) = &self.disk_cache_client
                && context.get_write_cache_policy().allow_disk_cache()
            {
                disk_cache_client.find_missing_digests(digests).await
            } else {
                Ok(IndexSet::new())
            }
        };

        let remote_query = async {
            if let Some(remote_cache_client) = &self.remote_cache_client
                && context.get_write_cache_policy().allow_remote_cache()
            {
                remote_cache_client
                    .find_missing_digests(context, digests)
                    .await
            } else {
                Ok(IndexSet::new())
            }
        };

        // PORT: `Futures.whenAllSucceed(remoteQuery, diskQuery)`. Both queries run to completion
        // (Java does not cancel the other input when one fails); when both fail Java reports the
        // one that failed first, the port the remote one.
        let (disk_query_final, remote_query_final) =
            futures::future::join(disk_query, remote_query).await;
        let mut result = remote_query_final?;
        result.extend(disk_query_final?);
        Ok(result)
    }

    /// Returns whether the remote action cache supports updating action results.
    // Java: com/google/devtools/build/lib/remote/CombinedCache.java:335 CombinedCache.remoteActionCacheSupportsUpdate
    pub fn remote_action_cache_supports_update(&self) -> bool {
        match self.get_remote_cache_capabilities() {
            Ok(capabilities) => {
                capabilities
                    .action_cache_update_capabilities
                    .unwrap_or_default()
                    .update_enabled
            }
            Err(_ignored) => false,
        }
    }

    /// Upload the action result to the remote cache.
    // Java: com/google/devtools/build/lib/remote/CombinedCache.java:344 CombinedCache.uploadActionResult
    pub async fn upload_action_result(
        &self,
        context: &RemoteActionExecutionContext<'_>,
        action_key: &ActionKey,
        action_result: &ActionResult,
    ) -> io::Result<()> {
        let disk_cache_future = async {
            if let Some(disk_cache_client) = &self.disk_cache_client
                && context.get_write_cache_policy().allow_disk_cache()
            {
                disk_cache_client
                    .upload_action_result(action_key, action_result)
                    .await
            } else {
                Ok(())
            }
        };

        let remote_cache_future = async {
            if let Some(remote_cache_client) = &self.remote_cache_client
                && context.get_write_cache_policy().allow_remote_cache()
            {
                remote_cache_client
                    .upload_action_result(context, action_key, action_result)
                    .await
            } else {
                Ok(())
            }
        };

        when_all_succeed(disk_cache_future, remote_cache_future).await
    }

    /// Upload a local file to the remote and/or disk cache.
    // Java: com/google/devtools/build/lib/remote/CombinedCache.java:361 CombinedCache.uploadFile
    pub async fn upload_file(
        &self,
        context: &RemoteActionExecutionContext<'_>,
        digest: &Digest,
        file: &Path,
    ) -> io::Result<()> {
        self.upload_file_with_force(context, digest, file, /* force= */ false)
            .await
    }

    /// Upload a local file to the remote cache.
    ///
    /// Trying to upload the same file multiple times concurrently, results in only one upload
    /// being performed. An upload to the remote cache is also skipped if this instance has already
    /// completed an upload of the same blob, unless `force` is set.
    ///
    /// * `context`: the context for the action.
    /// * `digest`: the digest of the file.
    /// * `file`: the file to upload.
    /// * `force`: whether to upload to the remote cache even if this instance has already
    ///   completed an upload of the same blob, e.g. because it may have been evicted since.
    ///   Concurrent uploads are still deduplicated.
    // Java: com/google/devtools/build/lib/remote/CombinedCache.java:380 CombinedCache.uploadFile
    pub async fn upload_file_with_force(
        &self,
        context: &RemoteActionExecutionContext<'_>,
        digest: &Digest,
        file: &Path,
        force: bool,
    ) -> io::Result<()> {
        if digest.size_bytes == 0 {
            return Ok(());
        }

        let disk_cache_future = async {
            if let Some(disk_cache_client) = &self.disk_cache_client
                && context.get_write_cache_policy().allow_disk_cache()
            {
                disk_cache_client.upload_file(digest, file).await
            } else {
                Ok(())
            }
        };

        // PORT: Java has already started the disk cache upload when this check fails (it is not
        // awaited then); here it has not started yet.
        let chunking_supported = self.chunking_supported()?;

        let remote_cache_future = async {
            if let Some(remote_cache_client) = &self.remote_cache_client
                && context.get_write_cache_policy().allow_remote_cache()
            {
                if chunking_supported
                    && digest.size_bytes > self.chunking_config().chunking_threshold()
                {
                    // remoteCacheClient.dedupUpload(
                    //     digest, () -> uploadChunked(context, digest, file, force), force)
                    dedup_upload(
                        &**remote_cache_client,
                        digest,
                        self.upload_chunked(context, digest, file, force),
                        force,
                    )
                    .await
                } else {
                    remote_cache_client
                        .upload_file(context, digest, file, force)
                        .await
                }
            } else {
                Ok(())
            }
        };

        when_all_succeed(disk_cache_future, remote_cache_future).await
    }

    /// The chunked upload of a file, as the task of the remote client's upload deduplication: it
    /// owns what it uses (a `ChunkedBlobUploader` over this cache's clients, the detached context).
    // Java: com/google/devtools/build/lib/remote/CombinedCache.java:413 CombinedCache.uploadChunked
    fn upload_chunked(
        &self,
        context: &RemoteActionExecutionContext<'_>,
        digest: &Digest,
        file: &Path,
        force: bool,
    ) -> impl std::future::Future<Output = io::Result<()>> + Send + 'static {
        let uploader = ChunkedBlobUploader::new(
            Arc::new(CacheClients {
                remote_cache_client: self
                    .remote_cache_client
                    .clone()
                    .expect("a chunked upload needs the remote cache"),
                disk_cache_client: self.disk_cache_client.clone(),
            }),
            &self.chunking_config(),
            self.digest_util.clone(),
        );
        let context = context.detached();
        let digest = digest.clone();
        let file = file.clone();
        async move {
            uploader
                .upload_chunked(&context, &digest, &file, force)
                .await
        }
    }

    /// Uploads a sequence of bytes to the cache.
    ///
    /// Trying to upload the same BLOB multiple times concurrently, results in only one upload
    /// being performed.
    ///
    /// * `context`: the context for the action.
    /// * `digest`: the digest of the BLOB.
    /// * `data`: the BLOB to upload.
    // Java: com/google/devtools/build/lib/remote/CombinedCache.java:432 CombinedCache.uploadBlob
    pub async fn upload_bytes(
        &self,
        context: &RemoteActionExecutionContext<'_>,
        digest: &Digest,
        data: bytes::Bytes,
    ) -> io::Result<()> {
        self.upload_blob(context, digest, Arc::new(BytesBlob(data)))
            .await
    }

    /// Uploads a blob to the cache from a repeatable stream supplier.
    ///
    /// The supplier may be opened more than once, including concurrently when both disk and
    /// remote cache writes are enabled.
    // Java: com/google/devtools/build/lib/remote/CombinedCache.java:443 CombinedCache.uploadBlob
    pub async fn upload_blob(
        &self,
        context: &RemoteActionExecutionContext<'_>,
        digest: &Digest,
        blob: Arc<dyn Blob>,
    ) -> io::Result<()> {
        self.upload_blob_with_force(context, digest, blob, /* force= */ false)
            .await
    }

    /// Uploads a blob to the cache from a repeatable stream supplier.
    ///
    /// The supplier may be opened more than once, including concurrently when both disk and
    /// remote cache writes are enabled.
    ///
    /// * `force`: whether to upload to the remote cache even if this instance has already
    ///   completed an upload of the same blob, e.g. because it may have been evicted since.
    ///   Concurrent uploads are still deduplicated.
    // Java: com/google/devtools/build/lib/remote/CombinedCache.java:458 CombinedCache.uploadBlob
    pub async fn upload_blob_with_force(
        &self,
        context: &RemoteActionExecutionContext<'_>,
        digest: &Digest,
        blob: Arc<dyn Blob>,
        force: bool,
    ) -> io::Result<()> {
        upload_blob_to_clients(
            self.remote_cache_client.as_ref(),
            self.disk_cache_client.as_ref(),
            context,
            digest,
            blob,
            force,
        )
        .await
    }

    /// `downloadBlob(context, digest)`: only for use by tests and the remote executor
    /// implementation.
    // Java: com/google/devtools/build/lib/remote/CombinedCache.java:479 CombinedCache.downloadBlob
    pub async fn download_blob(
        &self,
        context: &RemoteActionExecutionContext<'_>,
        digest: &Digest,
    ) -> io::Result<Vec<u8>> {
        self.download_blob_named(
            context, /* blobName= */ "", /* execPath= */ None, digest,
        )
        .await
    }

    /// Downloads a blob with content hash `digest` and stores its content in memory.
    ///
    /// Returns a future that completes after the download completes (succeeds / fails). If
    /// successful, the content is stored in the future's `byte[]`.
    // Java: com/google/devtools/build/lib/remote/CombinedCache.java:490 CombinedCache.downloadBlob
    pub async fn download_blob_named(
        &self,
        context: &RemoteActionExecutionContext<'_>,
        blob_name: &str,
        exec_path: Option<&PathFragment>,
        digest: &Digest,
    ) -> io::Result<Vec<u8>> {
        if digest.size_bytes == 0 {
            return Ok(Vec::new());
        }
        let mut b_out = new_byte_array_output_stream(digest.size_bytes);
        self.download_blob_named_to(context, blob_name, exec_path, digest, &mut b_out)
            .await?;
        Ok(b_out)
    }

    /// Downloads a blob with content hash `digest` and stores its content in memory.
    ///
    /// Unlike [`CombinedCache::download_blob_named`], the content is returned as a `ByteString`
    /// without an extra copy of the downloaded bytes.
    ///
    /// Returns a future that completes after the download completes (succeeds / fails). If
    /// successful, the content is stored in the future's `ByteString`.
    // Java: com/google/devtools/build/lib/remote/CombinedCache.java:512 CombinedCache.downloadBlobAsByteString
    pub async fn download_blob_as_byte_string(
        &self,
        context: &RemoteActionExecutionContext<'_>,
        blob_name: &str,
        exec_path: Option<&PathFragment>,
        digest: &Digest,
    ) -> io::Result<bytes::Bytes> {
        if digest.size_bytes == 0 {
            return Ok(bytes::Bytes::new());
        }
        let mut b_out = new_byte_array_output_stream(digest.size_bytes);
        self.download_blob_named_to(context, blob_name, exec_path, digest, &mut b_out)
            .await?;
        Ok(bytes::Bytes::from(b_out))
    }

    // Java: com/google/devtools/build/lib/remote/CombinedCache.java:525 CombinedCache.downloadBlob
    async fn download_blob_named_to(
        &self,
        context: &RemoteActionExecutionContext<'_>,
        blob_name: &str,
        exec_path: Option<&PathFragment>,
        digest: &Digest,
        out: &mut (dyn Write + Send),
    ) -> io::Result<()> {
        if digest.size_bytes == 0 {
            return Ok(());
        }
        match self.download_blob_to(context, digest, out).await {
            Ok(()) => Ok(()),
            Err(e) => match downcast::<CacheNotFoundException>(e) {
                Ok(cache_not_found_exception) => Err(set_exec_path_and_filename(
                    cache_not_found_exception,
                    exec_path,
                    blob_name,
                )),
                Err(e) => Err(e),
            },
        }
    }

    /// `downloadBlob(context, digest, out)` for a stream that is not path-backed: the disk cache
    /// first, then the remote cache.
    // Java: com/google/devtools/build/lib/remote/CombinedCache.java:546 CombinedCache.downloadBlob
    pub async fn download_blob_to(
        &self,
        context: &RemoteActionExecutionContext<'_>,
        digest: &Digest,
        out: &mut (dyn Write + Send),
    ) -> io::Result<()> {
        self.download_blob_maybe_path_backed(context, digest, out, None)
            .await
    }

    /// `downloadBlob(context, digest, out)` for a stream that may be path-backed.
    ///
    /// PORT: `out_path` is `out instanceof MaybePathBacked ? out.maybeGetPath() : null`, which
    /// the disk cache asks of the stream in Java (a `dyn Write` cannot be asked; see
    /// `DiskCacheClient::download_blob_maybe_path_backed`). With a path, the disk cache copies
    /// its entry to that path directly and writes nothing to `out`.
    // Java: com/google/devtools/build/lib/remote/CombinedCache.java:546 CombinedCache.downloadBlob
    pub async fn download_blob_maybe_path_backed(
        &self,
        context: &RemoteActionExecutionContext<'_>,
        digest: &Digest,
        out: &mut (dyn Write + Send),
        out_path: Option<&Path>,
    ) -> io::Result<()> {
        let mut result: io::Result<()> = Err(CacheNotFoundException::new(digest.clone()).into());

        if let Some(disk_cache_client) = &self.disk_cache_client
            && context.get_read_cache_policy().allow_disk_cache()
        {
            result = disk_cache_client
                .download_blob_maybe_path_backed(digest, &mut *out, out_path)
                .await;
        }

        if self.remote_cache_client.is_some()
            && context.get_read_cache_policy().allow_remote_cache()
        {
            let cache_not_found = matches!(
                &result,
                Err(e) if downcast_ref::<CacheNotFoundException>(e).is_some()
            );
            if cache_not_found {
                result = self
                    .download_blob_from_remote(context, digest, out, out_path)
                    .await;
            }
        }

        result
    }

    // Java: com/google/devtools/build/lib/remote/CombinedCache.java:566 CombinedCache.downloadBlobFromRemote
    async fn download_blob_from_remote(
        &self,
        context: &RemoteActionExecutionContext<'_>,
        digest: &Digest,
        out: &mut (dyn Write + Send),
        out_path: Option<&Path>,
    ) -> io::Result<()> {
        blaze_util::check_state!(
            self.remote_cache_client.is_some()
                && context.get_read_cache_policy().allow_remote_cache()
        );

        let chunking_supported = self.chunking_supported()?;

        if chunking_supported && digest.size_bytes > self.chunking_config().chunking_threshold() {
            let downloader = ChunkedBlobDownloader::new(
                self.grpc_cache_client()
                    .expect("chunking is supported only with a GrpcCacheClient"),
                self,
                &self.chunking_config(),
                self.digest_util.clone(),
            );
            // Only a failure to split the blob is recoverable by downloading it as a whole. Once
            // the server has described the blob as a sequence of chunks, the download is
            // committed to that path: chunks are written to `out` as they arrive, so restarting
            // would append the blob to the chunks already written. A missing chunk is reported
            // as a missing blob instead, letting the usual lost input handling regenerate it.
            return match downloader
                .download_chunked(context, digest, &mut *out)
                .await
            {
                Err(e) if BlobNotSplittableException::is(&e) => {
                    self.regular_download_blob_from_remote(context, digest, out, out_path)
                        .await
                }
                result => result,
            };
        }

        self.regular_download_blob_from_remote(context, digest, out, out_path)
            .await
    }

    // Java: com/google/devtools/build/lib/remote/CombinedCache.java:599 CombinedCache.regularDownloadBlobFromRemote
    async fn regular_download_blob_from_remote(
        &self,
        context: &RemoteActionExecutionContext<'_>,
        digest: &Digest,
        out: &mut (dyn Write + Send),
        out_path: Option<&Path>,
    ) -> io::Result<()> {
        blaze_util::check_state!(
            self.remote_cache_client.is_some()
                && context.get_read_cache_policy().allow_remote_cache()
        );
        let remote_cache_client = self.remote_cache_client.as_ref().unwrap();

        if let Some(disk_cache_client) = &self.disk_cache_client
            && context.get_write_cache_policy().allow_disk_cache()
        {
            let temp_path = disk_cache_client.get_temp_path();
            let mut temp_out = LazyFileOutputStream::new(temp_path.clone());
            let f = async {
                remote_cache_client
                    .download_blob(context, digest, &mut temp_out)
                    .await?;
                // Fsync temp before we rename it to avoid data loss in the case of machine
                // crashes (the OS may reorder the writes and the rename).
                temp_out.sync_if_possible()?;
                temp_out.close()?;
                disk_cache_client.capture_file(&temp_path, digest, Store::Cas)?;
                disk_cache_client
                    .download_blob_maybe_path_backed(digest, &mut *out, out_path)
                    .await
            }
            .await;
            return cleanup_temp_file_on_error(f, &temp_path, &mut temp_out);
        }

        remote_cache_client
            .download_blob(context, digest, out)
            .await
    }

    /// `downloadFile(context, outputPath, execPath, localPath, digest, reporter)`.
    // Java: com/google/devtools/build/lib/remote/CombinedCache.java:712 CombinedCache.downloadFile
    pub async fn download_file_with_reporter(
        &self,
        context: &RemoteActionExecutionContext<'_>,
        output_path: &str,
        exec_path: Option<&PathFragment>,
        local_path: &Path,
        digest: &Digest,
        reporter: &DownloadProgressReporter,
    ) -> io::Result<()> {
        let throwable = match self
            .download_file_impl(context, local_path, digest, reporter)
            .await
        {
            Ok(()) => return Ok(()),
            Err(throwable) => throwable,
        };
        let throwable = match downcast::<CacheNotFoundException>(throwable) {
            Ok(cache_not_found_exception) => {
                return Err(set_exec_path_and_filename(
                    cache_not_found_exception,
                    exec_path,
                    output_path,
                ));
            }
            Err(throwable) => throwable,
        };
        match downcast::<OutputDigestMismatchException>(throwable) {
            Ok(mut e) => {
                e.set_output_path(output_path);
                e.set_local_path(local_path.clone());
                Err(e.into())
            }
            Err(throwable) => Err(throwable),
        }
    }

    /// Downloads a file (that is not a directory). The content is fetched from the digest.
    ///
    /// Use [`CombinedCache::download_file_with_reporter`] instead for build outputs as it
    /// provides progress information and correctly handles unexpected cache misses.
    // Java: com/google/devtools/build/lib/remote/CombinedCache.java:744 CombinedCache.downloadFile
    pub async fn download_file(
        &self,
        context: &RemoteActionExecutionContext<'_>,
        path: &Path,
        digest: &Digest,
    ) -> io::Result<()> {
        let reporter = DownloadProgressReporter::new(no_action(), "", 0);
        self.download_file_with_reporter(
            context,
            path.get_path_string(),
            /* execPath= */ None,
            path,
            digest,
            &reporter,
        )
        .await
    }

    /// Downloads a file (that is not a directory). The content is fetched from the digest.
    // Java: com/google/devtools/build/lib/remote/CombinedCache.java:756 CombinedCache.downloadFile
    async fn download_file_impl(
        &self,
        context: &RemoteActionExecutionContext<'_>,
        path: &Path,
        digest: &Digest,
        reporter: &DownloadProgressReporter,
    ) -> io::Result<()> {
        path.get_parent_directory()
            .expect("NullPointerException")
            .create_directory_and_parents()?;
        if digest.size_bytes == 0 {
            // Handle empty file locally.
            fs_utils::write_content(path, &[])?;
            return Ok(());
        }

        if let Some(symlink_template) = &self.symlink_template {
            // Don't actually download files from the CAS. Instead, create a
            // symbolic link that points to a location where CAS objects may
            // be found. This could, for example, be a FUSE file system.
            path.create_symbolic_link(
                &path.get_relative_str(
                    &symlink_template
                        .replace("{hash}", &digest.hash)
                        .replace("{size_bytes}", &digest.size_bytes.to_string()),
                ),
            )?;
            return Ok(());
        }

        reporter.started();
        // PORT: Java's `f.addListener(...)` runs when the download completes or is cancelled;
        // here the guard runs when this future completes or is dropped.
        let mut listener = CloseAndFinish {
            out: ReportingOutputStream::new(LazyFileOutputStream::new(path.clone()), reporter),
            reporter,
        };

        // `out` is a `MaybePathBacked` stream.
        self.download_blob_maybe_path_backed(context, digest, &mut listener.out, Some(path))
            .await
    }

    /// Download the stdout and stderr of an executed action.
    ///
    /// * `context`: the context for the action.
    /// * `result`: the result of the action.
    /// * `out_err`: the `OutErr` that the stdout and stderr will be downloaded to.
    ///
    /// PORT: Java's downloads are already running when this returns; the returned futures start
    /// when they are first polled (hand them to `Utils.waitForBulkTransfer`).
    // Java: com/google/devtools/build/lib/remote/CombinedCache.java:808 CombinedCache.downloadOutErr
    pub fn download_out_err<'a>(
        &'a self,
        context: &'a RemoteActionExecutionContext<'_>,
        result: &'a ActionResult,
        out_err: &'a FileOutErr,
    ) -> Vec<BoxFuture<'a, io::Result<()>>> {
        let mut downloads: Vec<BoxFuture<'a, io::Result<()>>> = Vec::new();
        if !result.stdout_raw.is_empty() {
            // `result.getStdoutRaw().writeTo(outErr.getOutputStream())` and the flush.
            if let Err(e) = out_err.write_out(&result.stdout_raw) {
                downloads.push(Box::pin(async move { Err::<(), io::Error>(e) }));
            }
        } else if let Some(stdout_digest) = &result.stdout_digest {
            downloads.push(Box::pin(async move {
                let mut out = OutErrOutputStream {
                    out_err,
                    error_stream: false,
                };
                self.download_blob_named_to(
                    context,
                    /* blobName= */ "<stdout>",
                    /* execPath= */ None,
                    stdout_digest,
                    &mut out,
                )
                .await
            }));
        }
        if !result.stderr_raw.is_empty() {
            // `result.getStderrRaw().writeTo(outErr.getErrorStream())` and the flush.
            if let Err(e) = out_err.write_err(&result.stderr_raw) {
                downloads.push(Box::pin(async move { Err::<(), io::Error>(e) }));
            }
        } else if let Some(stderr_digest) = &result.stderr_digest {
            downloads.push(Box::pin(async move {
                let mut out = OutErrOutputStream {
                    out_err,
                    error_stream: true,
                };
                self.download_blob_named_to(
                    context,
                    /* blobName= */ "<stderr>",
                    /* execPath= */ None,
                    stderr_digest,
                    &mut out,
                )
                .await
            }));
        }
        downloads
    }

    /// `hasRemoteCache()`.
    // Java: com/google/devtools/build/lib/remote/CombinedCache.java:846 CombinedCache.hasRemoteCache
    pub fn has_remote_cache(&self) -> bool {
        self.remote_cache_client.is_some()
    }

    /// `hasDiskCache()`.
    // Java: com/google/devtools/build/lib/remote/CombinedCache.java:850 CombinedCache.hasDiskCache
    pub fn has_disk_cache(&self) -> bool {
        self.disk_cache_client.is_some()
    }

    // Java: com/google/devtools/build/lib/remote/CombinedCache.java:855 CombinedCache.deallocate
    fn deallocate(&self) {
        if let Some(disk_cache_client) = &self.disk_cache_client {
            disk_cache_client.close();
        }
        // Java: virtualThreadExecutor.shutdown();
        if let Some(remote_cache_client) = &self.remote_cache_client {
            remote_cache_client.shutdown_uploads();
            remote_cache_client.close();
        }

        *self.close_count_down_latch.lock() = true;
        self.close_count_down_latch_released.notify_all();
    }

    /// `touch(o)`.
    // Java: com/google/devtools/build/lib/remote/CombinedCache.java:869 CombinedCache.touch
    pub fn touch(&self) -> &CombinedCache {
        self
    }

    /// `refCnt()`.
    // Java: io/netty/util/AbstractReferenceCounted.java AbstractReferenceCounted.refCnt
    pub fn ref_cnt(&self) -> i32 {
        self.ref_cnt.load(Ordering::SeqCst)
    }

    /// Increases the reference count by 1.
    // Java: com/google/devtools/build/lib/remote/CombinedCache.java:875 CombinedCache.retain
    pub fn retain(&self) -> &CombinedCache {
        // Java: io/netty/util/AbstractReferenceCounted.java AbstractReferenceCounted.retain
        let mut ref_cnt = self.ref_cnt.load(Ordering::SeqCst);
        loop {
            if ref_cnt <= 0 {
                panic!("IllegalReferenceCountException: refCnt: {ref_cnt}, increment: 1");
            }
            match self.ref_cnt.compare_exchange(
                ref_cnt,
                ref_cnt + 1,
                Ordering::SeqCst,
                Ordering::SeqCst,
            ) {
                Ok(_) => return self,
                Err(actual) => ref_cnt = actual,
            }
        }
    }

    /// Decreases the reference count by 1 and deallocates the cache if the reference count
    /// reaches 0. Returns whether the cache was deallocated.
    // Java: io/netty/util/AbstractReferenceCounted.java AbstractReferenceCounted.release
    pub fn release(&self) -> bool {
        let mut ref_cnt = self.ref_cnt.load(Ordering::SeqCst);
        loop {
            if ref_cnt <= 0 {
                panic!("IllegalReferenceCountException: refCnt: {ref_cnt}, decrement: 1");
            }
            match self.ref_cnt.compare_exchange(
                ref_cnt,
                ref_cnt - 1,
                Ordering::SeqCst,
                Ordering::SeqCst,
            ) {
                Ok(_) => break,
                Err(actual) => ref_cnt = actual,
            }
        }
        if ref_cnt == 1 {
            self.deallocate();
            return true;
        }
        false
    }

    /// Waits for active network I/Os to finish.
    ///
    /// An interrupt of the waiting thread (`blaze_shell::thread_interrupt`) ends the wait with an
    /// `InterruptedException`, as Java's waits do.
    // Java: com/google/devtools/build/lib/remote/CombinedCache.java:881 CombinedCache.awaitTermination
    pub fn await_termination(&self) -> io::Result<()> {
        if let Some(remote_cache_client) = &self.remote_cache_client
            && crate::runtime::block_on_interruptibly(
                remote_cache_client.await_upload_termination(),
            )
            .is_none()
        {
            return Err(errors::interrupted());
        }
        // closeCountDownLatch.await()
        let mut closed = self.close_count_down_latch.lock();
        while !*closed {
            if blaze_shell::thread_interrupt::interrupted() {
                return Err(errors::interrupted());
            }
            // PORT: bounded, so that an interrupt (which cannot signal this condition variable)
            // is seen.
            self.close_count_down_latch_released
                .wait_for(&mut closed, std::time::Duration::from_millis(50));
        }
        // Java: virtualThreadExecutor.awaitTermination(Long.MAX_VALUE, TimeUnit.NANOSECONDS);
        Ok(())
    }

    /// Shuts the cache down and cancels active network I/Os.
    // Java: com/google/devtools/build/lib/remote/CombinedCache.java:890 CombinedCache.shutdownNow
    pub fn shutdown_now(&self) {
        if let Some(remote_cache_client) = &self.remote_cache_client {
            remote_cache_client.shutdown_uploads_now();
        }
        // Java: virtualThreadExecutor.shutdownNow();
    }

    /// `createFailureDetail(message, detailedCode)`.
    // Java: com/google/devtools/build/lib/remote/CombinedCache.java:897 CombinedCache.createFailureDetail
    pub fn create_failure_detail(
        message: &str,
        detailed_code: remote_execution::Code,
    ) -> FailureDetail {
        FailureDetail {
            message: message.to_string(),
            category: Some(failure_detail::Category::RemoteExecution(RemoteExecution {
                code: detailed_code as i32,
            })),
        }
    }
}

/// `Futures.whenAllSucceed(first, second).call(() -> null, directExecutor())`.
///
/// PORT: both transfers run to completion (Java does not cancel the other input when one fails);
/// when both fail Java reports the one that failed first, the port the first argument's.
async fn when_all_succeed(
    first: impl Future<Output = io::Result<()>>,
    second: impl Future<Output = io::Result<()>>,
) -> io::Result<()> {
    let (first, second) = futures::future::join(first, second).await;
    first?;
    second?;
    Ok(())
}

/// `new ByteArrayOutputStream((int) digest.getSizeBytes())`.
fn new_byte_array_output_stream(size_bytes: i64) -> Vec<u8> {
    let size = size_bytes as i32;
    if size < 0 {
        panic!("IllegalArgumentException: Negative initial size: {size}");
    }
    Vec::with_capacity(size as usize)
}

/// `cacheNotFoundException.setExecPath(execPath); cacheNotFoundException.setFilename(filename)`.
///
/// PORT: the exception is rebuilt rather than mutated (the `io::Error` may be shared between
/// the waiters of one transfer); a null `execPath` clears the field as in Java.
fn set_exec_path_and_filename(
    cache_not_found_exception: CacheNotFoundException,
    exec_path: Option<&PathFragment>,
    filename: &str,
) -> io::Error {
    let missing_digest = cache_not_found_exception.get_missing_digest().clone();
    match exec_path {
        Some(exec_path) => {
            let mut e = CacheNotFoundException::with_exec_path(missing_digest, exec_path.clone());
            e.set_filename(filename);
            e.into()
        }
        None => CacheNotFoundException::with_filename(missing_digest, filename).into(),
    }
}

// Java: com/google/devtools/build/lib/remote/CombinedCache.java:630 CombinedCache.cleanupTempFileOnError
fn cleanup_temp_file_on_error(
    f: io::Result<()>,
    temp_path: &Path,
    temp_out: &mut LazyFileOutputStream,
) -> io::Result<()> {
    match f {
        Ok(()) => Ok(()),
        Err(root_cause) => {
            // PORT: Java adds the failures of the cleanup to the root cause as suppressed
            // exceptions; an `io::Error` has no suppressed list.
            let _ = temp_out.close();
            let _ = temp_path.delete();
            Err(root_cause)
        }
    }
}

// Java: com/google/devtools/build/lib/remote/CombinedCache.java:653 DownloadProgressReporter.PATTERN
static PATTERN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new("^bazel-out/[^/]+/[^/]+/").expect("valid pattern"));

/// A reporter that reports download progresses.
// Java: com/google/devtools/build/lib/remote/CombinedCache.java:652 CombinedCache.DownloadProgressReporter
pub struct DownloadProgressReporter {
    include_file: bool,
    listener: Arc<dyn ProgressStatusListener>,
    id: String,
    file: String,
    total_size: String,
    downloaded_bytes: AtomicI64,
}

impl DownloadProgressReporter {
    /// `new DownloadProgressReporter(listener, file, totalSize)`.
    // Java: com/google/devtools/build/lib/remote/CombinedCache.java:661 DownloadProgressReporter.<init>
    pub fn new(
        listener: Arc<dyn ProgressStatusListener>,
        file: &str,
        total_size: i64,
    ) -> DownloadProgressReporter {
        DownloadProgressReporter::new_with_include_file(
            /* includeFile= */ true, listener, file, total_size,
        )
    }

    /// `new DownloadProgressReporter(includeFile, listener, file, totalSize)`.
    // Java: com/google/devtools/build/lib/remote/CombinedCache.java:665 DownloadProgressReporter.<init>
    pub fn new_with_include_file(
        include_file: bool,
        listener: Arc<dyn ProgressStatusListener>,
        file: &str,
        total_size: i64,
    ) -> DownloadProgressReporter {
        DownloadProgressReporter {
            include_file,
            listener,
            id: file.to_string(),
            total_size: bytes_count_to_display_string(total_size),
            file: PATTERN.replace(file, "").into_owned(),
            downloaded_bytes: AtomicI64::new(0),
        }
    }

    /// `started()`.
    // Java: com/google/devtools/build/lib/remote/CombinedCache.java:676 DownloadProgressReporter.started
    pub fn started(&self) {
        self.report_progress(false, false);
    }

    /// `downloadedBytes(count)`.
    // Java: com/google/devtools/build/lib/remote/CombinedCache.java:680 DownloadProgressReporter.downloadedBytes
    pub fn downloaded_bytes(&self, count: i32) {
        self.downloaded_bytes
            .fetch_add(i64::from(count), Ordering::SeqCst);
        self.report_progress(true, false);
    }

    /// `finished()`.
    // Java: com/google/devtools/build/lib/remote/CombinedCache.java:685 DownloadProgressReporter.finished
    pub fn finished(&self) {
        self.report_progress(true, true);
    }

    // Java: com/google/devtools/build/lib/remote/CombinedCache.java:689 DownloadProgressReporter.reportProgress
    fn report_progress(&self, include_bytes: bool, finished: bool) {
        let progress = if include_bytes {
            if self.include_file {
                format!(
                    "Downloading {}, {} / {}",
                    self.file,
                    bytes_count_to_display_string(self.downloaded_bytes.load(Ordering::SeqCst)),
                    self.total_size
                )
            } else {
                format!(
                    "{} / {}",
                    bytes_count_to_display_string(self.downloaded_bytes.load(Ordering::SeqCst)),
                    self.total_size
                )
            }
        } else if self.include_file {
            format!("Downloading {}", self.file)
        } else {
            String::new()
        };
        self.listener
            .on_progress_status(SpawnProgressEvent::create(&self.id, &progress, finished));
    }
}

/// An `OutputStream` that reports all the write operations with [`DownloadProgressReporter`].
///
/// PORT: Java wraps any `OutputStream`; the only one it is built with is a
/// `LazyFileOutputStream`.
// Java: com/google/devtools/build/lib/remote/CombinedCache.java:908 CombinedCache.ReportingOutputStream
struct ReportingOutputStream<'r> {
    out: LazyFileOutputStream,
    reporter: &'r DownloadProgressReporter,
}

impl<'r> ReportingOutputStream<'r> {
    fn new(
        out: LazyFileOutputStream,
        reporter: &'r DownloadProgressReporter,
    ) -> ReportingOutputStream<'r> {
        ReportingOutputStream { out, reporter }
    }

    // Java: com/google/devtools/build/lib/remote/CombinedCache.java:942 ReportingOutputStream.close
    fn close(&mut self) -> io::Result<()> {
        self.out.close()
    }

    /// `maybeGetPath()` (`MaybePathBacked`).
    // Java: com/google/devtools/build/lib/remote/CombinedCache.java:948 ReportingOutputStream.maybeGetPath
    #[allow(dead_code)]
    fn maybe_get_path(&self) -> Option<&Path> {
        self.out.maybe_get_path()
    }
}

impl Write for ReportingOutputStream<'_> {
    // Java: com/google/devtools/build/lib/remote/CombinedCache.java:925 ReportingOutputStream.write
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.out.write_all(buf)?;
        self.reporter.downloaded_bytes(buf.len() as i32);
        Ok(buf.len())
    }

    // Java: com/google/devtools/build/lib/remote/CombinedCache.java:937 ReportingOutputStream.flush
    fn flush(&mut self) -> io::Result<()> {
        self.out.flush()
    }
}

/// The listener `downloadFile` adds to its download: closes the output stream and reports the
/// end of the download.
// Java: com/google/devtools/build/lib/remote/CombinedCache.java:785 CombinedCache.downloadFile
struct CloseAndFinish<'r> {
    out: ReportingOutputStream<'r>,
    reporter: &'r DownloadProgressReporter,
}

impl Drop for CloseAndFinish<'_> {
    fn drop(&mut self) {
        // Java logs "Unexpected exception closing output stream after downloading %s/%d to %s"
        // at warning level when the close fails.
        let _ = self.out.close();
        self.reporter.finished();
    }
}

/// `outErr.getOutputStream()` / `outErr.getErrorStream()`.
///
/// PORT: `blaze_exec::util::FileOutErr` exposes its streams as `write_out` / `write_err`.
struct OutErrOutputStream<'a> {
    out_err: &'a FileOutErr,
    error_stream: bool,
}

impl Write for OutErrOutputStream<'_> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if self.error_stream {
            self.out_err.write_err(buf)?;
        } else {
            self.out_err.write_out(buf)?;
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
