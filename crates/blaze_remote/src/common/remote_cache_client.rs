//! Port of `lib/remote/common/RemoteCacheClient.java` and `MissingDigestsFinder.java`.

use std::io::{self, Read, Write};
use std::sync::Arc;

use async_trait::async_trait;
use blaze_vfs::Path;
use indexmap::IndexSet;

use super::remote_action_execution_context::RemoteActionExecutionContext;
use crate::reapi::{ActionResult, Digest, ServerCapabilities};
use crate::util::async_task_cache::AsyncTaskCacheNoResult;

/// A key in the remote action cache. The type wraps around a `Digest` of an `Action`. Action
/// keys are special in that they aren't content-addressable but refer to action results.
// Java: com/google/devtools/build/lib/remote/common/RemoteCacheClient.java:66 RemoteCacheClient.ActionKey
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ActionKey {
    digest: Digest,
}

impl ActionKey {
    /// `new ActionKey(digest)`.
    pub fn new(digest: Digest) -> ActionKey {
        ActionKey { digest }
    }

    /// `digest()`.
    pub fn digest(&self) -> &Digest {
        &self.digest
    }
}

/// A supplier for the data comprising a BLOB.
///
/// As blobs can be large and may need to be kept in memory, consumers should call `get` as late
/// as possible and close the blob as soon as they are done with it.
// Java: com/google/devtools/build/lib/remote/common/RemoteCacheClient.java:125 RemoteCacheClient.Blob
pub trait Blob: Send + Sync {
    /// Get an input stream for the blob's data. Can be called multiple times.
    fn get(&self) -> io::Result<Box<dyn Read + Send + '_>>;

    /// A description of the blob for use in error messages.
    fn description(&self) -> Option<String> {
        None
    }

    /// Whether the stream Java's blob gives can be rewound (`HttpCacheClient.reset(in)`: it
    /// supports `mark`/`reset`, or it is a `FileInputStream`), which decides whether an HTTP
    /// upload is repeated after a credential refresh. A `ByteString`'s stream can; a file's
    /// (`LazyFileInputStream`) and a chunk of a file (`ByteStreams.limit`) cannot.
    // Java: com/google/devtools/build/lib/remote/http/HttpCacheClient.java:734 HttpCacheClient.reset
    fn supports_reset(&self) -> bool {
        false
    }
}

/// The blob of a file (`uploadFile`'s anonymous `Blob` over a `LazyFileInputStream`).
// Java: com/google/devtools/build/lib/remote/common/RemoteCacheClient.java:143 RemoteCacheClient.uploadFile
#[derive(Debug, Clone)]
pub struct FileBlob(pub Path);

impl Blob for FileBlob {
    fn get(&self) -> io::Result<Box<dyn Read + Send + '_>> {
        self.0.get_input_stream()
    }

    fn description(&self) -> Option<String> {
        Some(format!("file {}", self.0.get_path_string()))
    }
}

/// The blob of a `ByteString` (`data::newInput`).
#[derive(Debug, Clone)]
pub struct BytesBlob(pub bytes::Bytes);

impl Blob for BytesBlob {
    fn get(&self) -> io::Result<Box<dyn Read + Send + '_>> {
        Ok(Box::new(io::Cursor::new(self.0.clone())))
    }

    /// `ByteString.newInput()` supports `mark`.
    fn supports_reset(&self) -> bool {
        true
    }
}

/// Supports querying a remote cache whether it contains a list of blobs.
// Java: com/google/devtools/build/lib/remote/common/MissingDigestsFinder.java:22 MissingDigestsFinder
#[async_trait]
pub trait MissingDigestsFinder: Send + Sync {
    /// Returns a set of digests that the remote cache does not know about. The returned set is
    /// guaranteed to be a subset of `digests`.
    async fn find_missing_digests(
        &self,
        context: &RemoteActionExecutionContext<'_>,
        digests: &[Digest],
    ) -> io::Result<IndexSet<Digest>>;
}

/// An interface for a remote caching protocol.
///
/// Implementations must be thread-safe.
///
/// PORT: the Java abstract class owns `casUploadCache`; implementors hold one
/// [`AsyncTaskCacheNoResult`] and return it from [`RemoteCacheClient::cas_upload_cache`], and the
/// final methods of the Java class are the provided methods here.
// Java: com/google/devtools/build/lib/remote/common/RemoteCacheClient.java:47 RemoteCacheClient
#[async_trait]
pub trait RemoteCacheClient: MissingDigestsFinder {
    /// `casUploadCache`.
    fn cas_upload_cache(&self) -> &AsyncTaskCacheNoResult<Digest>;

    /// Returns the capabilities of the remote cache server.
    fn get_server_capabilities(&self) -> io::Result<ServerCapabilities>;

    /// The client as `Any`, for the tests' `(GrpcCacheClient) client` casts (`None` for clients
    /// that do not support it).
    fn as_any(&self) -> Option<&dyn std::any::Any> {
        None
    }

    /// `getAuthority()`.
    async fn get_authority(&self) -> io::Result<String>;

    /// Downloads an action result for the `actionKey`.
    ///
    /// Returns `None` if the action result is not found (Java: a future completing with null).
    ///
    /// * `inline_out_err`: a hint to the server to inline the stdout and stderr in the
    ///   `ActionResult` message.
    /// * `inline_output_files`: a hint to the server to inline the specified output files in the
    ///   `ActionResult` message.
    async fn download_action_result(
        &self,
        context: &RemoteActionExecutionContext<'_>,
        action_key: &ActionKey,
        inline_out_err: bool,
        inline_output_files: &IndexSet<String>,
    ) -> io::Result<Option<ActionResult>>;

    /// Uploads an action result for the `actionKey`.
    async fn upload_action_result(
        &self,
        context: &RemoteActionExecutionContext<'_>,
        action_key: &ActionKey,
        action_result: &ActionResult,
    ) -> io::Result<()>;

    /// Downloads a BLOB for the given `digest` and writes it to `out`.
    ///
    /// It's the callers responsibility to close `out`.
    ///
    /// Fails with a `CacheNotFoundException` if the `digest` does not exist.
    async fn download_blob(
        &self,
        context: &RemoteActionExecutionContext<'_>,
        digest: &Digest,
        out: &mut (dyn Write + Send),
    ) -> io::Result<()>;

    /// Uploads a BLOB to the CAS, without deduplication (`@VisibleForTesting uploadBlobImpl`).
    async fn upload_blob_impl(
        &self,
        context: &RemoteActionExecutionContext<'_>,
        digest: &Digest,
        blob: &dyn Blob,
    ) -> io::Result<()>;

    /// `uploadBlobImpl` as the task of `uploadBlob`, which owns the client, the context and the
    /// blob. A client whose transfers block a thread (the HTTP cache) runs it as a task of its
    /// own, so that the uploads a caller waits for together proceed at once, as each has its
    /// own Netty pipeline in Java; the default awaits `upload_blob_impl` in place.
    fn upload_blob_owned(
        self: Arc<Self>,
        context: RemoteActionExecutionContext<'static>,
        digest: Digest,
        blob: Arc<dyn Blob>,
    ) -> futures::future::BoxFuture<'static, io::Result<()>>
    where
        Self: 'static,
    {
        Box::pin(async move { self.upload_blob_impl(&context, &digest, &*blob).await })
    }

    /// Close resources associated with the remote cache.
    fn close(&self);

    /// Returns the digests of the uploads that are in progress.
    // Java: com/google/devtools/build/lib/remote/common/RemoteCacheClient.java:232 RemoteCacheClient.getInProgressUploads
    fn get_in_progress_uploads(&self) -> Vec<Digest> {
        self.cas_upload_cache().get_in_progress_tasks()
    }

    /// Returns the digests of the uploads that have finished.
    // Java: com/google/devtools/build/lib/remote/common/RemoteCacheClient.java:237 RemoteCacheClient.getFinishedUploads
    fn get_finished_uploads(&self) -> Vec<Digest> {
        self.cas_upload_cache().get_finished_tasks()
    }

    /// Returns the number of subscribers of the upload of `digest`.
    // Java: com/google/devtools/build/lib/remote/common/RemoteCacheClient.java:242 RemoteCacheClient.getUploadSubscriberCount
    fn get_upload_subscriber_count(&self, digest: &Digest) -> usize {
        self.cas_upload_cache().get_subscriber_count(digest)
    }

    /// `shutdownUploads()`.
    // Java: com/google/devtools/build/lib/remote/common/RemoteCacheClient.java:246 RemoteCacheClient.shutdownUploads
    fn shutdown_uploads(&self) {
        self.cas_upload_cache().shutdown();
    }

    /// `awaitUploadTermination()`.
    // Java: com/google/devtools/build/lib/remote/common/RemoteCacheClient.java:251 RemoteCacheClient.awaitUploadTermination
    async fn await_upload_termination(&self) {
        self.cas_upload_cache().await_termination().await;
    }

    /// `shutdownUploadsNow()`.
    // Java: com/google/devtools/build/lib/remote/common/RemoteCacheClient.java:256 RemoteCacheClient.shutdownUploadsNow
    fn shutdown_uploads_now(&self) {
        self.cas_upload_cache().shutdown_now();
    }
}

/// The upload methods of `RemoteCacheClient` (`uploadFile`, `uploadBlob`, `dedupUpload`), which
/// run the upload as a task of `casUploadCache`.
///
/// PORT: the task of an `AsyncTaskCache` is shared by every caller that waits for it and must
/// outlive the caller that started it, so it may not borrow from it: these methods are on the
/// shared handle of the client (`Arc`), and the task owns a clone of it, the detached context,
/// the digest and the blob.
#[async_trait]
pub trait RemoteCacheClientUploads {
    /// Uploads a `file` BLOB to the CAS.
    // Java: com/google/devtools/build/lib/remote/common/RemoteCacheClient.java:143 RemoteCacheClient.uploadFile
    async fn upload_file(
        &self,
        context: &RemoteActionExecutionContext<'_>,
        digest: &Digest,
        file: &Path,
        force: bool,
    ) -> io::Result<()>;

    /// Uploads a BLOB to the CAS.
    // Java: com/google/devtools/build/lib/remote/common/RemoteCacheClient.java:170 RemoteCacheClient.uploadBlob
    async fn upload_blob(
        &self,
        context: &RemoteActionExecutionContext<'_>,
        digest: &Digest,
        blob: Arc<dyn Blob>,
        force: bool,
    ) -> io::Result<()>;

    /// Uploads an in-memory BLOB to the CAS (`uploadBlob(context, digest, ByteString, force)`).
    // Java: com/google/devtools/build/lib/remote/common/RemoteCacheClient.java:187 RemoteCacheClient.uploadBlob
    async fn upload_bytes(
        &self,
        context: &RemoteActionExecutionContext<'_>,
        digest: &Digest,
        data: bytes::Bytes,
        force: bool,
    ) -> io::Result<()>;
}

#[async_trait]
impl<T: RemoteCacheClient + ?Sized + 'static> RemoteCacheClientUploads for Arc<T> {
    // Java: com/google/devtools/build/lib/remote/common/RemoteCacheClient.java:143 RemoteCacheClient.uploadFile
    async fn upload_file(
        &self,
        context: &RemoteActionExecutionContext<'_>,
        digest: &Digest,
        file: &Path,
        force: bool,
    ) -> io::Result<()> {
        self.upload_blob(context, digest, Arc::new(FileBlob(file.clone())), force)
            .await
    }

    // Java: com/google/devtools/build/lib/remote/common/RemoteCacheClient.java:170 RemoteCacheClient.uploadBlob
    async fn upload_blob(
        &self,
        context: &RemoteActionExecutionContext<'_>,
        digest: &Digest,
        blob: Arc<dyn Blob>,
        force: bool,
    ) -> io::Result<()> {
        let client = self.clone();
        let context = context.detached();
        let task_digest = digest.clone();
        // `Completable.defer(() -> uploadBlobImpl(context, digest, blob))`
        let upload = async move {
            client
                .upload_blob_owned(context, task_digest, blob)
                .await
        };
        self.cas_upload_cache()
            .execute(digest.clone(), upload, force)
            .await
    }

    // Java: com/google/devtools/build/lib/remote/common/RemoteCacheClient.java:187 RemoteCacheClient.uploadBlob
    async fn upload_bytes(
        &self,
        context: &RemoteActionExecutionContext<'_>,
        digest: &Digest,
        data: bytes::Bytes,
        force: bool,
    ) -> io::Result<()> {
        self.upload_blob(context, digest, Arc::new(BytesBlob(data)), force)
            .await
    }
}

/// `RemoteCacheClient.dedupUpload(digest, upload, force)`: runs `upload` through the client's
/// upload deduplication (a free function because it is generic over the future). The upload is
/// shared with every caller that waits for the same digest, so it owns what it needs.
// Java: com/google/devtools/build/lib/remote/common/RemoteCacheClient.java:225 RemoteCacheClient.dedupUpload
pub async fn dedup_upload<F>(
    client: &dyn RemoteCacheClient,
    digest: &Digest,
    upload: F,
    force: bool,
) -> io::Result<()>
where
    F: std::future::Future<Output = io::Result<()>> + Send + 'static,
{
    client
        .cas_upload_cache()
        .execute(digest.clone(), upload, force)
        .await
}
