//! Port of `lib/remote/ChunkedBlobUploader.java`.

use std::collections::HashSet;
use std::io::{self, Read};
use std::sync::Arc;

use blaze_vfs::Path;
use futures::FutureExt;
use futures::future::BoxFuture;
use futures::stream::{FuturesUnordered, StreamExt};
use indexmap::IndexSet;

use crate::chunking::{ChunkingConfig, ContentDefinedChunker};
use crate::combined_cache::upload_blob_to_clients;
use crate::common::{Blob, MissingDigestsFinder, RemoteActionExecutionContext, RemoteCacheClient};
use crate::disk::DiskCacheClient;
use crate::grpc_cache_client::GrpcCacheClient;
use crate::reapi::{Digest, chunking_function};
use crate::util::DigestUtil;

/// Guard against pathological fanout from a single large chunked blob. This is only a per-blob
/// cap; chunk uploads still flow through CombinedCache and the shared remote cache transport
/// stack below it, which is what bounds active remote RPC concurrency across blobs.
// Java: com/google/devtools/build/lib/remote/ChunkedBlobUploader.java:63 ChunkedBlobUploader.MAX_IN_FLIGHT_CHUNK_UPLOADS
const MAX_IN_FLIGHT_CHUNK_UPLOADS: usize = 16;

/// The calls the uploader makes on its `GrpcCacheClient` (`findMissingDigests`, `spliceBlob`)
/// and its `CombinedCache` (`uploadBlob`).
///
/// PORT: the seam Java's tests mock. The real target is [`CacheClients`], which owns the cache's
/// clients because a chunked upload runs as the task of the client's upload deduplication.
pub trait ChunkedUploadTarget: Send + Sync {
    // Java: com/google/devtools/build/lib/remote/GrpcCacheClient.java:314 GrpcCacheClient.findMissingDigests
    fn find_missing_digests<'a>(
        &'a self,
        context: &'a RemoteActionExecutionContext<'_>,
        digests: &'a [Digest],
    ) -> BoxFuture<'a, io::Result<IndexSet<Digest>>>;

    // Java: com/google/devtools/build/lib/remote/GrpcCacheClient.java:219 GrpcCacheClient.spliceBlob
    fn splice_blob<'a>(
        &'a self,
        context: &'a RemoteActionExecutionContext<'_>,
        blob_digest: &Digest,
        chunk_digests: &[Digest],
        chunking_function: chunking_function::Value,
    ) -> Option<BoxFuture<'a, io::Result<()>>>;

    // Java: com/google/devtools/build/lib/remote/CombinedCache.java:458 CombinedCache.uploadBlob
    fn upload_blob<'a>(
        &'a self,
        context: &'a RemoteActionExecutionContext<'_>,
        digest: &'a Digest,
        blob: Arc<dyn Blob>,
        force: bool,
    ) -> BoxFuture<'a, io::Result<()>>;
}

/// The clients of a `CombinedCache`: the remote one (a `GrpcCacheClient`) and the disk one.
pub struct CacheClients {
    pub remote_cache_client: Arc<dyn RemoteCacheClient>,
    pub disk_cache_client: Option<Arc<DiskCacheClient>>,
}

impl CacheClients {
    fn grpc_cache_client(&self) -> &GrpcCacheClient {
        self.remote_cache_client
            .as_any()
            .and_then(|any| any.downcast_ref::<GrpcCacheClient>())
            .expect("ChunkedBlobUploader needs a GrpcCacheClient")
    }
}

impl ChunkedUploadTarget for CacheClients {
    fn find_missing_digests<'a>(
        &'a self,
        context: &'a RemoteActionExecutionContext<'_>,
        digests: &'a [Digest],
    ) -> BoxFuture<'a, io::Result<IndexSet<Digest>>> {
        Box::pin(self.grpc_cache_client().find_missing_digests(context, digests))
    }

    fn splice_blob<'a>(
        &'a self,
        context: &'a RemoteActionExecutionContext<'_>,
        blob_digest: &Digest,
        chunk_digests: &[Digest],
        chunking_function: chunking_function::Value,
    ) -> Option<BoxFuture<'a, io::Result<()>>> {
        self.grpc_cache_client()
            .splice_blob(context, blob_digest, chunk_digests, chunking_function)
    }

    fn upload_blob<'a>(
        &'a self,
        context: &'a RemoteActionExecutionContext<'_>,
        digest: &'a Digest,
        blob: Arc<dyn Blob>,
        force: bool,
    ) -> BoxFuture<'a, io::Result<()>> {
        Box::pin(upload_blob_to_clients(
            Some(&self.remote_cache_client),
            self.disk_cache_client.as_ref(),
            context,
            digest,
            blob,
            force,
        ))
    }
}

/// Uploads blobs in chunks using Content-Defined Chunking.
///
/// Upload flow for blobs above threshold: chunk the file with the configured chunking function,
/// call findMissingDigests on the chunk digests, upload only the missing chunks, and call
/// SpliceBlob to register the blob as the concatenation of the chunks.
///
/// PORT: Java holds the `GrpcCacheClient` and the `CombinedCache`; its upload runs as the task of
/// the client's upload deduplication, which outlives the caller, so here the uploader owns its
/// target ([`ChunkedUploadTarget`], for a cache its [`CacheClients`]).
// Java: com/google/devtools/build/lib/remote/ChunkedBlobUploader.java:58 ChunkedBlobUploader
pub struct ChunkedBlobUploader {
    target: Arc<dyn ChunkedUploadTarget>,
    chunker: Box<dyn ContentDefinedChunker>,
    chunking_function: chunking_function::Value,
    chunking_threshold: i64,
}

impl ChunkedBlobUploader {
    /// Creates a new uploader with the given chunking configuration.
    // Java: com/google/devtools/build/lib/remote/ChunkedBlobUploader.java:79 ChunkedBlobUploader.<init>
    pub fn new(
        target: Arc<dyn ChunkedUploadTarget>,
        config: &ChunkingConfig,
        digest_util: DigestUtil,
    ) -> ChunkedBlobUploader {
        ChunkedBlobUploader {
            target,
            chunker: config.new_chunker(digest_util),
            chunking_function: config.chunking_function(),
            chunking_threshold: config.chunking_threshold(),
        }
    }

    /// Returns the minimum blob size for chunked upload.
    // Java: com/google/devtools/build/lib/remote/ChunkedBlobUploader.java:92 ChunkedBlobUploader.getChunkingThreshold
    pub fn get_chunking_threshold(&self) -> i64 {
        self.chunking_threshold
    }

    /// Uploads a blob in content-defined chunks. The file is chunked with the configured chunking
    /// function, missing chunks are uploaded, and `SpliceBlob` is called to register the blob as
    /// the concatenation of its chunks.
    ///
    /// * `force`: whether to upload missing chunks even if the cache client has already
    ///   completed uploads of them.
    ///
    /// PORT: the file is chunked on the calling task, as Java chunks it on its virtual thread.
    // Java: com/google/devtools/build/lib/remote/ChunkedBlobUploader.java:105 ChunkedBlobUploader.uploadChunked
    pub async fn upload_chunked(
        &self,
        context: &RemoteActionExecutionContext<'_>,
        blob_digest: &Digest,
        file: &Path,
        force: bool,
    ) -> io::Result<()> {
        let chunk_digests = {
            let mut input = file.get_input_stream()?;
            self.chunker.chunk_to_digests(&mut input)?
        };
        if chunk_digests.is_empty() {
            return Ok(());
        }
        let missing_digests = self
            .target
            .find_missing_digests(context, &chunk_digests)
            .await?;
        self.upload_missing_chunks(context, &missing_digests, &chunk_digests, file, force)
            .await?;
        match self.target.splice_blob(
            context,
            blob_digest,
            &chunk_digests,
            self.chunking_function,
        ) {
            Some(splice) => splice.await,
            // getFromFuture(null): Java's chunking is only negotiated with the option set, which
            // is also what spliceBlob checks.
            None => panic!("NullPointerException: spliceBlob returned null"),
        }
    }

    // Java: com/google/devtools/build/lib/remote/ChunkedBlobUploader.java:122 ChunkedBlobUploader.uploadMissingChunks
    async fn upload_missing_chunks(
        &self,
        context: &RemoteActionExecutionContext<'_>,
        missing_digests: &IndexSet<Digest>,
        chunk_digests: &[Digest],
        file: &Path,
        force: bool,
    ) -> io::Result<()> {
        if missing_digests.is_empty() {
            return Ok(());
        }
        UploadSession {
            uploader: self,
            context,
            missing_digests,
            chunk_digests,
            force,
            in_flight_uploads: FuturesUnordered::new(),
            scheduled_digests: HashSet::new(),
        }
        .run(file)
        .await
    }
}

/// One chunked upload.
///
/// PORT: Java's completion queue of listener callbacks is a `FuturesUnordered` here; dropping
/// the session (an error, or the caller giving up) drops the uploads still in flight, which is
/// `cancelAllUploads()` in the `finally` block.
// Java: com/google/devtools/build/lib/remote/ChunkedBlobUploader.java:135 ChunkedBlobUploader.UploadSession
struct UploadSession<'a, 'b> {
    uploader: &'a ChunkedBlobUploader,
    context: &'a RemoteActionExecutionContext<'b>,
    missing_digests: &'a IndexSet<Digest>,
    chunk_digests: &'a [Digest],
    force: bool,
    in_flight_uploads: FuturesUnordered<BoxFuture<'a, io::Result<()>>>,
    scheduled_digests: HashSet<Digest>,
}

impl<'a, 'b> UploadSession<'a, 'b> {
    // Java: com/google/devtools/build/lib/remote/ChunkedBlobUploader.java:159 ChunkedBlobUploader.UploadSession.run
    async fn run(mut self, file: &Path) -> io::Result<()> {
        let mut offset: i64 = 0;
        for chunk_digest in self.chunk_digests {
            self.drain_completed_uploads()?;
            let chunk_offset = offset;
            offset += chunk_digest.size_bytes;
            if !self.should_schedule_upload(chunk_digest) {
                continue;
            }
            if self.in_flight_uploads.len() >= MAX_IN_FLIGHT_CHUNK_UPLOADS {
                self.await_completed_upload().await?;
            }
            self.start_upload(file, chunk_offset, chunk_digest);
        }
        while !self.in_flight_uploads.is_empty() {
            self.await_completed_upload().await?;
        }
        Ok(())
    }

    // Java: com/google/devtools/build/lib/remote/ChunkedBlobUploader.java:182 ChunkedBlobUploader.UploadSession.shouldScheduleUpload
    fn should_schedule_upload(&mut self, chunk_digest: &Digest) -> bool {
        self.missing_digests.contains(chunk_digest)
            && self.scheduled_digests.insert(chunk_digest.clone())
    }

    // Java: com/google/devtools/build/lib/remote/ChunkedBlobUploader.java:186 ChunkedBlobUploader.UploadSession.startUpload
    fn start_upload(&mut self, file: &Path, chunk_offset: i64, chunk_digest: &'a Digest) {
        let uploader = self.uploader;
        let context = self.context;
        let force = self.force;
        let blob: Arc<dyn Blob> = Arc::new(ChunkBlob {
            file: file.clone(),
            offset: chunk_offset,
            digest: chunk_digest.clone(),
        });
        self.in_flight_uploads
            .push(uploader.target.upload_blob(context, chunk_digest, blob, force));
    }

    // Java: com/google/devtools/build/lib/remote/ChunkedBlobUploader.java:194 ChunkedBlobUploader.UploadSession.drainCompletedUploads
    fn drain_completed_uploads(&mut self) -> io::Result<()> {
        loop {
            match self.in_flight_uploads.next().now_or_never() {
                Some(Some(result)) => result?,
                _ => return Ok(()),
            }
        }
    }

    // Java: com/google/devtools/build/lib/remote/ChunkedBlobUploader.java:204 ChunkedBlobUploader.UploadSession.awaitCompletedUpload
    async fn await_completed_upload(&mut self) -> io::Result<()> {
        if let Some(result) = self.in_flight_uploads.next().await {
            result?;
        }
        self.drain_completed_uploads()
    }
}

/// The blob of one chunk of a file.
// Java: com/google/devtools/build/lib/remote/ChunkedBlobUploader.java:221 ChunkedBlobUploader.ChunkBlob
struct ChunkBlob {
    file: Path,
    offset: i64,
    digest: Digest,
}

impl Blob for ChunkBlob {
    // Java: com/google/devtools/build/lib/remote/ChunkedBlobUploader.java:233 ChunkedBlobUploader.ChunkBlob.get
    fn get(&self) -> io::Result<Box<dyn Read + Send + '_>> {
        let mut input = self.file.get_input_stream()?;
        // seekOrSkip(input, offset): an input shorter than the offset is an EOFException.
        let skipped = io::copy(&mut (&mut input).take(self.offset as u64), &mut io::sink())?;
        if skipped < self.offset as u64 {
            return Err(io::Error::other(format!(
                "file was concurrently modified during upload: {}",
                self.file
            )));
        }
        Ok(Box::new(input.take(self.digest.size_bytes as u64)))
    }

    // Java: com/google/devtools/build/lib/remote/ChunkedBlobUploader.java:249 ChunkedBlobUploader.ChunkBlob.description
    fn description(&self) -> Option<String> {
        Some(format!(
            "chunk {} at offset {} of file {}",
            DigestUtil::to_string(&self.digest),
            self.offset,
            self.file
        ))
    }
}
