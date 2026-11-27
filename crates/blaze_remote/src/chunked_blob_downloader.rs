//! Port of `lib/remote/ChunkedBlobDownloader.java`.

use std::collections::HashMap;
use std::io::{self, Write};

use futures::FutureExt;
use futures::future::BoxFuture;
use futures::stream::{FuturesUnordered, StreamExt};

use crate::chunking::ChunkingConfig;
use crate::combined_cache::CombinedCache;
use crate::common::{BlobNotSplittableException, RemoteActionExecutionContext};
use crate::grpc_cache_client::GrpcCacheClient;
use crate::reapi::{Digest, SplitBlobResponse, chunking_function};
use crate::util::DigestUtil;
use crate::util::utils::verify_blob_contents;

/// Guard against pathological fanout from a single large chunked blob. This is only a per-blob
/// cap; chunk requests still flow through CombinedCache and the shared remote cache transport
/// stack below it, which is what bounds active remote RPC concurrency across blobs.
// Java: com/google/devtools/build/lib/remote/ChunkedBlobDownloader.java:44 ChunkedBlobDownloader.MAX_IN_FLIGHT_CHUNK_DOWNLOADS
const MAX_IN_FLIGHT_CHUNK_DOWNLOADS: usize = 16;

/// The calls the downloader makes on its `GrpcCacheClient`.
///
/// PORT: Java's downloader takes the `GrpcCacheClient` itself (its tests mock it); the trait is
/// that seam, implemented by [`GrpcCacheClient`].
pub trait ChunkSplitter: Send + Sync {
    // Java: com/google/devtools/build/lib/remote/GrpcCacheClient.java:137 GrpcCacheClient.shouldVerifyDownloads
    fn should_verify_downloads(&self) -> bool;

    // Java: com/google/devtools/build/lib/remote/GrpcCacheClient.java:260 GrpcCacheClient.splitBlob
    fn split_blob<'a>(
        &'a self,
        context: &'a RemoteActionExecutionContext<'_>,
        digest: &Digest,
        chunking_function: chunking_function::Value,
    ) -> Option<BoxFuture<'a, io::Result<SplitBlobResponse>>>;
}

impl ChunkSplitter for GrpcCacheClient {
    fn should_verify_downloads(&self) -> bool {
        GrpcCacheClient::should_verify_downloads(self)
    }

    fn split_blob<'a>(
        &'a self,
        context: &'a RemoteActionExecutionContext<'_>,
        digest: &Digest,
        chunking_function: chunking_function::Value,
    ) -> Option<BoxFuture<'a, io::Result<SplitBlobResponse>>> {
        GrpcCacheClient::split_blob(self, context, digest, chunking_function)
    }
}

/// The call the downloader makes on its `CombinedCache` (`downloadBlob(context, digest)`).
///
/// PORT: the seam Java's tests mock, implemented by [`CombinedCache`]. Boxed: a chunk's download
/// goes through the cache's download path again.
pub trait ChunkSource: Send + Sync {
    // Java: com/google/devtools/build/lib/remote/CombinedCache.java:479 CombinedCache.downloadBlob
    fn download_blob<'a>(
        &'a self,
        context: &'a RemoteActionExecutionContext<'_>,
        digest: &'a Digest,
    ) -> BoxFuture<'a, io::Result<Vec<u8>>>;
}

impl ChunkSource for CombinedCache {
    fn download_blob<'a>(
        &'a self,
        context: &'a RemoteActionExecutionContext<'_>,
        digest: &'a Digest,
    ) -> BoxFuture<'a, io::Result<Vec<u8>>> {
        Box::pin(CombinedCache::download_blob(self, context, digest))
    }
}

/// Downloads blobs by fetching chunks through a per-blob sliding window via the SplitBlob API.
///
/// PORT: Java's downloader holds the client and the cache for its lifetime; here it borrows them
/// for the download it is made for (CombinedCache makes one per chunked download).
// Java: com/google/devtools/build/lib/remote/ChunkedBlobDownloader.java:41 ChunkedBlobDownloader
pub struct ChunkedBlobDownloader<'a> {
    grpc_cache_client: &'a dyn ChunkSplitter,
    combined_cache: &'a dyn ChunkSource,
    digest_util: DigestUtil,
    chunking_function: chunking_function::Value,
}

/// A digest as a map key (`Digest` is a protobuf message without `Hash`).
type DigestKey = (String, i64);

fn key(digest: &Digest) -> DigestKey {
    (digest.hash.clone(), digest.size_bytes)
}

impl<'a> ChunkedBlobDownloader<'a> {
    // Java: com/google/devtools/build/lib/remote/ChunkedBlobDownloader.java:52 ChunkedBlobDownloader.<init>
    pub fn new(
        grpc_cache_client: &'a dyn ChunkSplitter,
        combined_cache: &'a dyn ChunkSource,
        chunking_config: &ChunkingConfig,
        digest_util: DigestUtil,
    ) -> ChunkedBlobDownloader<'a> {
        ChunkedBlobDownloader {
            grpc_cache_client,
            combined_cache,
            digest_util,
            chunking_function: chunking_config.chunking_function(),
        }
    }

    /// Downloads a blob using chunked download via the SplitBlob API.
    // Java: com/google/devtools/build/lib/remote/ChunkedBlobDownloader.java:68 ChunkedBlobDownloader.downloadChunked
    pub async fn download_chunked(
        &self,
        context: &RemoteActionExecutionContext<'_>,
        blob_digest: &Digest,
        out: &mut (dyn Write + Send),
    ) -> io::Result<()> {
        if self.grpc_cache_client.should_verify_downloads() {
            let mut digest_out = self.digest_util.new_digest_output_stream(out);
            let chunk_digests = self.get_chunk_digests(context, blob_digest).await?;
            self.download_and_reassemble_chunks(context, &chunk_digests, &mut digest_out)
                .await?;
            return verify_blob_contents(blob_digest, &digest_out.digest());
        }
        let chunk_digests = self.get_chunk_digests(context, blob_digest).await?;
        self.download_and_reassemble_chunks(context, &chunk_digests, out)
            .await
    }

    /// Returns the chunks the blob is composed of; a [`BlobNotSplittableException`] if the server
    /// cannot describe the blob as a sequence of chunks, in which case the caller may fall back
    /// to downloading the whole blob.
    // Java: com/google/devtools/build/lib/remote/ChunkedBlobDownloader.java:90 ChunkedBlobDownloader.getChunkDigests
    async fn get_chunk_digests(
        &self,
        context: &RemoteActionExecutionContext<'_>,
        blob_digest: &Digest,
    ) -> io::Result<Vec<Digest>> {
        if blob_digest.size_bytes == 0 {
            return Ok(Vec::new());
        }
        let Some(split_response_future) =
            self.grpc_cache_client
                .split_blob(context, blob_digest, self.chunking_function)
        else {
            return Err(BlobNotSplittableException::new(blob_digest.clone()).into());
        };
        let chunk_digests = split_response_future.await?.chunk_digests;
        if chunk_digests.is_empty() {
            return Err(BlobNotSplittableException::new(blob_digest.clone()).into());
        }
        Ok(chunk_digests)
    }

    // Java: com/google/devtools/build/lib/remote/ChunkedBlobDownloader.java:135 ChunkedBlobDownloader.downloadAndReassembleChunks
    async fn download_and_reassemble_chunks(
        &self,
        context: &RemoteActionExecutionContext<'_>,
        chunk_digests: &[Digest],
        out: &mut (dyn Write + Send),
    ) -> io::Result<()> {
        DownloadSession {
            combined_cache: self.combined_cache,
            context,
            chunk_digests,
            out,
            downloads: FuturesUnordered::new(),
            active_downloads: HashMap::new(),
            ready_chunks: HashMap::new(),
            next_to_start: 0,
            next_to_write: 0,
        }
        .run()
        .await
    }
}

/// A chunk download in flight: its digest and its result.
type PendingDownload<'b> = BoxFuture<'b, (DigestKey, io::Result<Vec<u8>>)>;

/// One chunked download.
///
/// PORT: Java's completion queue of listener callbacks is a `FuturesUnordered` here; dropping
/// the session (an error, or the caller giving up) drops the downloads still in flight, which is
/// `cancelAllDownloads()` in the `finally` block.
// Java: com/google/devtools/build/lib/remote/ChunkedBlobDownloader.java:141 ChunkedBlobDownloader.DownloadSession
struct DownloadSession<'b, 'c> {
    combined_cache: &'b dyn ChunkSource,
    context: &'b RemoteActionExecutionContext<'c>,
    chunk_digests: &'b [Digest],
    out: &'b mut (dyn Write + Send),
    downloads: FuturesUnordered<PendingDownload<'b>>,
    /// `activeDownloads`: the chunk indices each digest in flight is for.
    active_downloads: HashMap<DigestKey, Vec<usize>>,
    ready_chunks: HashMap<usize, Vec<u8>>,
    next_to_start: usize,
    next_to_write: usize,
}

impl<'b, 'c> DownloadSession<'b, 'c> {
    // Java: com/google/devtools/build/lib/remote/ChunkedBlobDownloader.java:160 ChunkedBlobDownloader.DownloadSession.run
    async fn run(mut self) -> io::Result<()> {
        self.fill_window();
        while self.next_to_write < self.chunk_digests.len() {
            self.drain_completed_downloads().await?;
            self.drain_ready_chunks()?;
            self.fill_window();
        }
        Ok(())
    }

    // Java: com/google/devtools/build/lib/remote/ChunkedBlobDownloader.java:173 ChunkedBlobDownloader.DownloadSession.fillWindow
    fn fill_window(&mut self) {
        while self.next_to_start < self.chunk_digests.len() {
            if self.next_to_start - self.next_to_write >= MAX_IN_FLIGHT_CHUNK_DOWNLOADS {
                return;
            }
            let chunk_digest = &self.chunk_digests[self.next_to_start];
            if let Some(existing) = self.active_downloads.get_mut(&key(chunk_digest)) {
                existing.push(self.next_to_start);
                self.next_to_start += 1;
                continue;
            }
            self.start_download(self.next_to_start);
            self.next_to_start += 1;
        }
    }

    // Java: com/google/devtools/build/lib/remote/ChunkedBlobDownloader.java:190 ChunkedBlobDownloader.DownloadSession.startDownload
    fn start_download(&mut self, chunk_index: usize) {
        let chunk_digest: &'b Digest = &self.chunk_digests[chunk_index];
        let combined_cache = self.combined_cache;
        let context = self.context;
        let digest_key = key(chunk_digest);
        self.active_downloads
            .insert(digest_key.clone(), vec![chunk_index]);
        self.downloads.push(Box::pin(async move {
            // The download of a chunk goes through CombinedCache again; a chunk is never above
            // the chunking threshold, so it is not chunked itself.
            (
                digest_key,
                combined_cache.download_blob(context, chunk_digest).await,
            )
        }));
    }

    // Java: com/google/devtools/build/lib/remote/ChunkedBlobDownloader.java:198 ChunkedBlobDownloader.DownloadSession.drainCompletedDownloads
    async fn drain_completed_downloads(&mut self) -> io::Result<()> {
        // completedDownloads.take()
        let Some(mut download) = self.downloads.next().await else {
            unreachable!("a chunk to write is always being downloaded");
        };
        loop {
            self.process_completed_download(download)?;
            // completedDownloads.poll()
            match self.downloads.next().now_or_never() {
                Some(Some(next)) => download = next,
                _ => return Ok(()),
            }
        }
    }

    // Java: com/google/devtools/build/lib/remote/ChunkedBlobDownloader.java:206 ChunkedBlobDownloader.DownloadSession.processCompletedDownload
    fn process_completed_download(
        &mut self,
        (digest_key, result): (DigestKey, io::Result<Vec<u8>>),
    ) -> io::Result<()> {
        let chunk_indices = self
            .active_downloads
            .remove(&digest_key)
            .unwrap_or_default();
        let chunk_data = result?;
        for chunk_index in chunk_indices {
            if chunk_index == self.next_to_write {
                self.out.write_all(&chunk_data)?;
                self.next_to_write += 1;
            } else {
                self.ready_chunks.insert(chunk_index, chunk_data.clone());
            }
        }
        Ok(())
    }

    // Java: com/google/devtools/build/lib/remote/ChunkedBlobDownloader.java:220 ChunkedBlobDownloader.DownloadSession.drainReadyChunks
    fn drain_ready_chunks(&mut self) -> io::Result<()> {
        while let Some(chunk) = self.ready_chunks.remove(&self.next_to_write) {
            self.out.write_all(&chunk)?;
            self.next_to_write += 1;
        }
        Ok(())
    }
}
