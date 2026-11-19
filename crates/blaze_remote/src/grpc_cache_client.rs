//! Port of `lib/remote/GrpcCacheClient.java`.

use std::collections::HashSet;
use std::io::{self, Write};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use async_trait::async_trait;
use blaze_optdefs::misc::remote_options::RemoteOptions;
use blaze_protos::JavaMessage;
use futures::future::BoxFuture;
use indexmap::IndexSet;
use parking_lot::Mutex;

use crate::authandtls::CallCredentialsProvider;
use crate::byte_stream_uploader::ByteStreamUploader;
use crate::chunker::Chunker;
use crate::common::errors::{self, StatusRuntimeException, io_exception, status_to_io_error};
use crate::common::{
    ActionKey, Blob, CacheNotFoundException, MissingDigestsFinder, OutputDigestMismatchException,
    RemoteActionExecutionContext, RemoteCacheClient,
};
use crate::grpc::channel::io_result;
use crate::grpc::{CallOptions, ReferenceCountedChannel, methods};
use crate::reapi::bytestream::{ReadRequest, ReadResponse};
use crate::reapi::{
    ActionResult, Digest, FindMissingBlobsRequest, FindMissingBlobsResponse,
    GetActionResultRequest, ServerCapabilities, SplitBlobResponse, UpdateActionResultRequest,
    chunking_function, digest_function,
};
use crate::remote_retrier::{ProgressiveBackoff, RemoteRetrier};
use crate::retrier::Retrier;
use crate::util::async_task_cache::AsyncTaskCacheNoResult;
use crate::util::digest_util::{DigestOutputStream, DigestUtil};
use crate::util::utils;
use crate::zstd::ZstdDecompressingOutputStream;

/// Guava's `CountingOutputStream`: an `OutputStream` that counts the number of bytes written.
// Java: com/google/common/io/CountingOutputStream.java CountingOutputStream
struct CountingOutputStream<W: Write> {
    out: W,
    count: i64,
}

impl<W: Write> CountingOutputStream<W> {
    fn new(out: W) -> CountingOutputStream<W> {
        CountingOutputStream { out, count: 0 }
    }

    /// Returns the number of bytes written.
    fn get_count(&self) -> i64 {
        self.count
    }
}

impl<W: Write> Write for CountingOutputStream<W> {
    fn write(&mut self, b: &[u8]) -> io::Result<usize> {
        self.out.write_all(b)?;
        self.count += b.len() as i64;
        Ok(b.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.out.flush()
    }
}

/// The stream `downloadBlob(context, digest, out)` counts the bytes of: `out` itself, or the
/// `DigestOutputStream` around it when downloads are verified.
enum DownloadOut<W: Write> {
    /// `digestUtil.newDigestOutputStream(out)`.
    Digest(DigestOutputStream<W>),
    Plain(W),
}

impl<W: Write> DownloadOut<W> {
    /// `digestSupplier` (`digestOut::digest`): `None` is Java's `null` supplier.
    fn digest(&self) -> Option<Digest> {
        match self {
            DownloadOut::Digest(digest_out) => Some(digest_out.digest()),
            DownloadOut::Plain(_) => None,
        }
    }
}

impl<W: Write> Write for DownloadOut<W> {
    fn write(&mut self, b: &[u8]) -> io::Result<usize> {
        match self {
            DownloadOut::Digest(out) => out.write(b),
            DownloadOut::Plain(out) => {
                out.write_all(b)?;
                Ok(b.len())
            }
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self {
            DownloadOut::Digest(out) => out.flush(),
            DownloadOut::Plain(out) => out.flush(),
        }
    }
}

/// The `CountingOutputStream rawOut` of one download, shared by its read attempts.
///
/// PORT: Java hands the same stream object to every attempt; the attempts of the retrier are
/// futures made by one closure, so the stream sits behind a lock that is taken for single
/// writes only (never across a suspension point).
type RawOut<W> = Mutex<CountingOutputStream<DownloadOut<W>>>;

// Java: com/google/devtools/build/lib/remote/GrpcCacheClient.java:90 GrpcCacheClient.SizeLimitingOutputStream
struct SizeLimitingOutputStream<'a, W: Write> {
    out: &'a RawOut<W>,
    digest: &'a Digest,
}

impl<W: Write> SizeLimitingOutputStream<'_, W> {
    // Java: com/google/devtools/build/lib/remote/GrpcCacheClient.java:116 GrpcCacheClient.SizeLimitingOutputStream.checkSize
    fn check_size(
        out: &CountingOutputStream<DownloadOut<W>>,
        digest: &Digest,
        bytes_to_write: usize,
    ) -> io::Result<()> {
        if bytes_to_write as i64 > digest.size_bytes - out.get_count() {
            return Err(io_exception(format!(
                "Received more bytes than expected for digest '{}/{}'. \
                 Server may have ignored read_offset.",
                digest.hash, digest.size_bytes
            )));
        }
        Ok(())
    }
}

impl<W: Write> Write for SizeLimitingOutputStream<'_, W> {
    // Java: com/google/devtools/build/lib/remote/GrpcCacheClient.java:106 GrpcCacheClient.SizeLimitingOutputStream.write
    fn write(&mut self, b: &[u8]) -> io::Result<usize> {
        let mut out = self.out.lock();
        Self::check_size(&out, self.digest, b.len())?;
        out.write_all(b)?;
        Ok(b.len())
    }

    // Java: com/google/devtools/build/lib/remote/GrpcCacheClient.java:112 GrpcCacheClient.SizeLimitingOutputStream.flush
    fn flush(&mut self) -> io::Result<()> {
        self.out.lock().flush()
    }
}

/// The `OutputStream out` of one read attempt (`requestRead`): the size limited stream, behind
/// a zstd decompressor when the blob is requested compressed.
enum ReadOut<'a, W: Write> {
    Zstd(ZstdDecompressingOutputStream<SizeLimitingOutputStream<'a, W>>),
    Plain(SizeLimitingOutputStream<'a, W>),
}

impl<W: Write> ReadOut<'_, W> {
    /// `data.writeTo(out)`.
    fn write_all(&mut self, data: &[u8]) -> io::Result<()> {
        match self {
            ReadOut::Zstd(out) => out.write_all(data),
            ReadOut::Plain(out) => out.write_all(data),
        }
    }

    /// `out.flush()`.
    fn flush(&mut self) -> io::Result<()> {
        match self {
            ReadOut::Zstd(out) => out.flush(),
            ReadOut::Plain(out) => out.flush(),
        }
    }

    // Java: com/google/devtools/build/lib/remote/GrpcCacheClient.java:615 GrpcCacheClient.requestRead (releaseOut)
    fn release_out(&mut self) {
        if let ReadOut::Zstd(out) = self {
            // Java logs "failed to cleanly close output stream" at WARNING on failure.
            let _ = out.close_shallow();
        }
    }
}

/// A RemoteActionCache implementation that uses gRPC calls to a remote cache server.
// Java: com/google/devtools/build/lib/remote/GrpcCacheClient.java:87 GrpcCacheClient
pub struct GrpcCacheClient {
    /// `RemoteCacheClient.casUploadCache`.
    cas_upload_cache: AsyncTaskCacheNoResult<Digest>,
    call_credentials_provider: Arc<dyn CallCredentialsProvider>,
    channel: ReferenceCountedChannel,
    options: RemoteOptions,
    /// `options.maxOutboundMessageSize`.
    max_outbound_message_size: i32,
    digest_util: DigestUtil,
    retrier: Arc<RemoteRetrier>,
    uploader: ByteStreamUploader,
    max_missing_blobs_digests_per_message: i32,

    closed: AtomicBool,
}

impl GrpcCacheClient {
    // Java: com/google/devtools/build/lib/remote/GrpcCacheClient.java:137 GrpcCacheClient.shouldVerifyDownloads
    pub fn should_verify_downloads(&self) -> bool {
        self.options.remote_verify_downloads
    }

    // Java: com/google/devtools/build/lib/remote/GrpcCacheClient.java:142 GrpcCacheClient.<init>
    pub fn new(
        channel: ReferenceCountedChannel,
        call_credentials_provider: Arc<dyn CallCredentialsProvider>,
        options: &RemoteOptions,
        retrier: Arc<RemoteRetrier>,
        digest_util: DigestUtil,
    ) -> GrpcCacheClient {
        GrpcCacheClient::new_with_max_outbound_message_size(
            channel,
            call_credentials_provider,
            options,
            retrier,
            digest_util,
            RemoteOptions::MAX_OUTBOUND_MESSAGE_SIZE,
        )
    }

    /// The constructor for an `options.maxOutboundMessageSize` other than its initial value.
    ///
    /// PORT: `RemoteOptions.maxOutboundMessageSize` is a plain field that only tests assign; the
    /// Rust options struct has the initial value as a constant, so tests pass theirs here.
    // Java: com/google/devtools/build/lib/remote/GrpcCacheClient.java:142 GrpcCacheClient.<init>
    pub fn new_with_max_outbound_message_size(
        channel: ReferenceCountedChannel,
        call_credentials_provider: Arc<dyn CallCredentialsProvider>,
        options: &RemoteOptions,
        retrier: Arc<RemoteRetrier>,
        digest_util: DigestUtil,
        max_outbound_message_size: i32,
    ) -> GrpcCacheClient {
        let uploader = ByteStreamUploader::new(
            &options.remote_instance_name,
            channel.clone(),
            call_credentials_provider.clone(),
            retrier.clone(),
            options.maximum_open_files,
            digest_util.get_digest_function(),
        );
        let mut client = GrpcCacheClient {
            cas_upload_cache: AsyncTaskCacheNoResult::create(),
            call_credentials_provider,
            channel,
            options: options.clone(),
            max_outbound_message_size,
            digest_util,
            retrier,
            uploader,
            max_missing_blobs_digests_per_message: 0,
            closed: AtomicBool::new(false),
        };
        client.max_missing_blobs_digests_per_message =
            client.compute_max_missing_blobs_digests_per_message();
        blaze_util::check_state!(
            client.max_missing_blobs_digests_per_message > 0,
            "Error: gRPC message size too small."
        );
        client
    }

    // Java: com/google/devtools/build/lib/remote/GrpcCacheClient.java:166 GrpcCacheClient.computeMaxMissingBlobsDigestsPerMessage
    fn compute_max_missing_blobs_digests_per_message(&self) -> i32 {
        let overhead = FindMissingBlobsRequest {
            instance_name: self.options.remote_instance_name.clone(),
            digest_function: self.digest_util.get_digest_function() as i32,
            ..Default::default()
        }
        .java_encoded_len() as i32;
        let tag_size = FindMissingBlobsRequest {
            blob_digests: vec![Digest::default()],
            ..Default::default()
        }
        .java_encoded_len() as i32
            - FindMissingBlobsRequest::default().java_encoded_len() as i32;
        // We assume all non-empty digests have the same size. This is true for fixed-length
        // hashes.
        let digest_size = self.digest_util.compute(&[1]).java_encoded_len() as i32 + tag_size;
        (self.max_outbound_message_size - overhead) / digest_size
    }

    /// The decoration every stub of this client gets: the request metadata of the context, the
    /// network time interceptor and the current call credentials.
    ///
    /// NEEDS(channel): Java sets no deadline on these stubs; the deadline of a call comes from
    /// the channel's default service config (`RemoteGrpcServiceConfig`: `--remote_timeout` per
    /// method), which `ReferenceCountedChannel` does not apply yet (see
    /// `authandtls::ManagedChannel::service_config`).
    fn stub(&self, context: &RemoteActionExecutionContext<'_>) -> CallOptions {
        CallOptions::new()
            .with_request_metadata(context.get_request_metadata().clone())
            .with_network_time(context.get_network_time().clone())
            .with_call_credentials(self.call_credentials_provider.get_call_credentials())
    }

    // Java: com/google/devtools/build/lib/remote/GrpcCacheClient.java:184 GrpcCacheClient.casFutureStub
    fn cas_future_stub(&self, context: &RemoteActionExecutionContext<'_>) -> CallOptions {
        self.stub(context)
    }

    // Java: com/google/devtools/build/lib/remote/GrpcCacheClient.java:193 GrpcCacheClient.bsAsyncStub
    fn bs_async_stub(&self, context: &RemoteActionExecutionContext<'_>) -> CallOptions {
        self.stub(context)
    }

    // Java: com/google/devtools/build/lib/remote/GrpcCacheClient.java:201 GrpcCacheClient.acFutureStub
    fn ac_future_stub(&self, context: &RemoteActionExecutionContext<'_>) -> CallOptions {
        self.stub(context)
    }

    /// Registers a blob as the concatenation of previously uploaded chunks via the SpliceBlob
    /// RPC. All chunks must already be present in the CAS.
    ///
    /// Returns a future that completes when the splice is acknowledged, or `None` if chunking is
    /// not enabled.
    // Java: com/google/devtools/build/lib/remote/GrpcCacheClient.java:219 GrpcCacheClient.spliceBlob
    pub fn splice_blob(
        &self,
        _context: &RemoteActionExecutionContext<'_>,
        _blob_digest: &Digest,
        _chunk_digests: &[Digest],
        _chunking_function: chunking_function::Value,
    ) -> Option<BoxFuture<'static, io::Result<()>>> {
        if !self.options.experimental_remote_cache_chunking {
            return None;
        }
        blaze_util::unported!(
            "GrpcCacheClient.spliceBlob (content-defined chunking: --experimental_remote_cache_chunking)"
        )
    }

    /// Queries the server for chunk information about a blob using the SplitBlob RPC.
    ///
    /// The returned future fails with a `BlobNotSplittableException` if the server does not
    /// implement the RPC or has no chunks for this blob.
    ///
    /// Returns a future with the split blob response, or `None` if chunking is not enabled.
    // Java: com/google/devtools/build/lib/remote/GrpcCacheClient.java:260 GrpcCacheClient.splitBlob
    pub fn split_blob(
        &self,
        _context: &RemoteActionExecutionContext<'_>,
        _digest: &Digest,
        _chunking_function: chunking_function::Value,
    ) -> Option<BoxFuture<'static, io::Result<SplitBlobResponse>>> {
        if !self.options.experimental_remote_cache_chunking {
            return None;
        }
        blaze_util::unported!(
            "GrpcCacheClient.splitBlob (content-defined chunking: --experimental_remote_cache_chunking)"
        )
    }

    /// Returns true if 'options.remoteCache' uses 'grpc' or an empty scheme
    // Java: com/google/devtools/build/lib/remote/GrpcCacheClient.java:304 GrpcCacheClient.isRemoteCacheOptions
    pub fn is_remote_cache_options(options: &RemoteOptions) -> bool {
        let remote_cache = match options.remote_cache.as_deref() {
            None | Some("") => return false,
            Some(remote_cache) => remote_cache,
        };
        // TODO(ishikhman): add proper URI validation/parsing for remote options
        let lower = remote_cache.to_ascii_lowercase();
        !(lower.starts_with("http://") || lower.starts_with("https://"))
    }

    // Java: com/google/devtools/build/lib/remote/GrpcCacheClient.java:366 GrpcCacheClient.getMissingDigests
    async fn get_missing_digests(
        &self,
        context: &RemoteActionExecutionContext<'_>,
        request: FindMissingBlobsRequest,
    ) -> io::Result<FindMissingBlobsResponse> {
        utils::refresh_if_unauthenticated_async(
            || {
                self.retrier.execute_async(|| async {
                    // channel.withChannelFuture(
                    //     channel -> casFutureStub(context, channel).findMissingBlobs(request))
                    io_result(
                        self.channel
                            .unary::<FindMissingBlobsRequest, FindMissingBlobsResponse>(
                                methods::FIND_MISSING_BLOBS,
                                request.clone(),
                                self.cas_future_stub(context),
                            )
                            .await,
                    )
                })
            },
            &*self.call_credentials_provider,
        )
        .await
    }

    /// PORT: Java catches the `StatusRuntimeException` of the stub's future; here the outcome
    /// of the call is converted directly.
    // Java: com/google/devtools/build/lib/remote/GrpcCacheClient.java:377 GrpcCacheClient.handleStatus
    fn handle_status(
        download: Result<ActionResult, tonic::Status>,
    ) -> io::Result<Option<ActionResult>> {
        match download {
            Ok(action_result) => Ok(Some(action_result)),
            Err(sre) => {
                if sre.code() == tonic::Code::NotFound {
                    // Return null to indicate that it was a cache miss.
                    Ok(None)
                } else {
                    Err(status_to_io_error(sre))
                }
            }
        }
    }

    // Java: com/google/devtools/build/lib/remote/GrpcCacheClient.java:468 GrpcCacheClient.downloadBlob
    async fn download_blob_counting<W: Write + Send>(
        &self,
        context: &RemoteActionExecutionContext<'_>,
        digest: &Digest,
        out: CountingOutputStream<DownloadOut<W>>,
    ) -> io::Result<()> {
        let out: RawOut<W> = Mutex::new(out);
        let progressive_backoff = ProgressiveBackoff::new(self.retrier.backoff_supplier());
        let download_future = utils::refresh_if_unauthenticated_async(
            || {
                self.retrier.execute_async_with_backoff(
                    // channel.withChannelFuture(channel -> requestRead(...))
                    || self.request_read(context, &progressive_backoff, digest, &out),
                    &progressive_backoff,
                )
            },
            &*self.call_credentials_provider,
        );

        // PORT: `Futures.catchingAsync(..., StatusRuntimeException.class, e -> new
        // IOException(e))`: the status already travels as the payload of an `io::Error`.
        download_future.await.map(|_bytes_written| ())
    }

    // Java: com/google/devtools/build/lib/remote/GrpcCacheClient.java:498 GrpcCacheClient.getResourceName
    pub fn get_resource_name(
        instance_name: &str,
        digest: &Digest,
        compressed: bool,
        digest_function: digest_function::Value,
    ) -> String {
        let mut resource_name = String::new();
        if !instance_name.is_empty() {
            resource_name += instance_name;
            resource_name += "/";
        }
        resource_name += if compressed {
            "compressed-blobs/zstd/"
        } else {
            "blobs/"
        };
        if !DigestUtil::is_old_style_digest_function(digest_function) {
            resource_name += &digest_function.as_str_name().to_ascii_lowercase();
            resource_name += "/";
        }
        resource_name + &DigestUtil::to_string(digest)
    }

    /// One `ByteStream.Read` call, resuming at the number of bytes already written to `raw_out`.
    ///
    /// PORT: Java's `ClientResponseObserver` callbacks are the arms of the loop over the
    /// response stream: `onNext` for a message, [`GrpcCacheClient::on_completed`] for the end of
    /// the stream, [`GrpcCacheClient::on_error`] for a status. The `SettableFuture` is the
    /// result of this function, and cancelling it (`requestStream.cancel`) is dropping the
    /// future, which drops the response stream and so cancels the call.
    // Java: com/google/devtools/build/lib/remote/GrpcCacheClient.java:511 GrpcCacheClient.requestRead
    async fn request_read<W: Write + Send>(
        &self,
        context: &RemoteActionExecutionContext<'_>,
        progressive_backoff: &ProgressiveBackoff,
        digest: &Digest,
        raw_out: &RawOut<W>,
    ) -> io::Result<i64> {
        let compressed = self.should_compress(digest);
        let resource_name = GrpcCacheClient::get_resource_name(
            &self.options.remote_instance_name,
            digest,
            compressed,
            self.digest_util.get_digest_function(),
        );
        // Prevent misbehaving servers from sending more bytes than expected.
        let size_limited_out = SizeLimitingOutputStream {
            out: raw_out,
            digest,
        };
        let mut out = if compressed {
            ReadOut::Zstd(ZstdDecompressingOutputStream::new(size_limited_out)?)
        } else {
            ReadOut::Plain(size_limited_out)
        };
        let read_offset = raw_out.lock().get_count();
        let request = ReadRequest {
            resource_name,
            read_offset,
            ..Default::default()
        };
        let mut response_stream = match self
            .channel
            .server_streaming::<ReadRequest, ReadResponse>(
                methods::READ,
                request,
                self.bs_async_stub(context),
            )
            .await
        {
            Ok(response_stream) => response_stream,
            Err(t) => return GrpcCacheClient::on_error(t, digest, raw_out, &mut out),
        };
        loop {
            match response_stream.message().await {
                // onNext(readResponse)
                Ok(Some(read_response)) => {
                    let data = read_response.data;
                    // On failure the output stream either refused to accept more bytes than
                    // expected for the digest or was closed due to cancellation (e.g. dynamic
                    // execution choosing the local branch).
                    // requestStream.cancel(e.getMessage(), e): the stream is dropped here.
                    out.write_all(&data)?;
                    // reset the stall backoff because we've made progress or been kept alive
                    progressive_backoff.reset();
                }
                Ok(None) => return GrpcCacheClient::on_completed(digest, raw_out, &mut out),
                Err(t) => return GrpcCacheClient::on_error(t, digest, raw_out, &mut out),
            }
        }
    }

    // Java: com/google/devtools/build/lib/remote/GrpcCacheClient.java:572 GrpcCacheClient.requestRead (onError)
    fn on_error<W: Write + Send>(
        t: tonic::Status,
        digest: &Digest,
        raw_out: &RawOut<W>,
        out: &mut ReadOut<'_, W>,
    ) -> io::Result<i64> {
        let count = raw_out.lock().get_count();
        if count == digest.size_bytes {
            // If the file was fully downloaded, it doesn't matter if there was an
            // error at
            // the end of the stream.
            // Java logs "ignoring error because file was fully received" at INFO.
            return GrpcCacheClient::on_completed(digest, raw_out, out);
        }
        out.release_out();
        // Status status = Status.fromThrowable(t);
        if t.code() == tonic::Code::NotFound {
            Err(CacheNotFoundException::new(digest.clone()).into())
        } else {
            Err(status_to_io_error(t))
        }
    }

    // Java: com/google/devtools/build/lib/remote/GrpcCacheClient.java:592 GrpcCacheClient.requestRead (onCompleted)
    fn on_completed<W: Write + Send>(
        digest: &Digest,
        raw_out: &RawOut<W>,
        out: &mut ReadOut<'_, W>,
    ) -> io::Result<i64> {
        // try { out.flush(); } finally { releaseOut(); }
        let flushed = out.flush();
        out.release_out();
        flushed?;
        let (actual_digest, count) = {
            let raw_out = raw_out.lock();
            (raw_out.out.digest(), raw_out.get_count())
        };
        if let Some(actual_digest) = actual_digest {
            utils::verify_blob_contents(digest, &actual_digest)?;
        } else if count != digest.size_bytes {
            // Verifying the digest would also catch an incomplete download; with
            // verification disabled, at least verify the size.
            return Err(
                OutputDigestMismatchException::with_received_size(digest.clone(), count).into(),
            );
        }
        Ok(count)
    }

    /// Uploads the chunks of `chunker` as the blob `digest`.
    ///
    /// PORT: Java closes the chunker in a listener of the upload future. The uploader takes the
    /// chunker by value here, so it is dropped (which closes its input) when the upload is over,
    /// whatever its outcome.
    // Java: com/google/devtools/build/lib/remote/GrpcCacheClient.java:660 GrpcCacheClient.uploadChunker
    pub async fn upload_chunker(
        &self,
        context: &RemoteActionExecutionContext<'_>,
        digest: &Digest,
        chunker: Chunker<'_>,
    ) -> io::Result<()> {
        self.uploader
            .upload_blob_async(context, digest, chunker)
            .await
    }

    // Java: com/google/devtools/build/lib/remote/GrpcCacheClient.java:676 GrpcCacheClient.getRetrier
    pub fn get_retrier(&self) -> &Retrier {
        &self.retrier
    }

    // Java: com/google/devtools/build/lib/remote/GrpcCacheClient.java:680 GrpcCacheClient.shouldCompress
    fn should_compress(&self, digest: &Digest) -> bool {
        self.options.cache_compression
            && digest.size_bytes >= self.options.cache_compression_threshold as i64
    }

    // Java: com/google/devtools/build/lib/remote/GrpcCacheClient.java:684 GrpcCacheClient.getChannel
    pub fn get_channel(&self) -> &ReferenceCountedChannel {
        &self.channel
    }
}

#[async_trait]
impl MissingDigestsFinder for GrpcCacheClient {
    // Java: com/google/devtools/build/lib/remote/GrpcCacheClient.java:314 GrpcCacheClient.findMissingDigests
    async fn find_missing_digests(
        &self,
        context: &RemoteActionExecutionContext<'_>,
        digests: &[Digest],
    ) -> io::Result<IndexSet<Digest>> {
        if digests.is_empty() {
            return Ok(IndexSet::new());
        }
        // Need to potentially split the digests into multiple requests.
        let mut request_builder = FindMissingBlobsRequest {
            instance_name: self.options.remote_instance_name.clone(),
            digest_function: self.digest_util.get_digest_function() as i32,
            ..Default::default()
        };
        let mut get_missing_digest_calls = Vec::new();
        for digest in digests {
            request_builder.blob_digests.push(digest.clone());
            if request_builder.blob_digests.len() as i32
                == self.max_missing_blobs_digests_per_message
            {
                get_missing_digest_calls
                    .push(self.get_missing_digests(context, request_builder.clone()));
                request_builder.blob_digests.clear();
            }
        }

        if !request_builder.blob_digests.is_empty() {
            get_missing_digest_calls
                .push(self.get_missing_digests(context, request_builder.clone()));
        }

        // Futures.whenAllSucceed(getMissingDigestCalls).call(...): fails as soon as one of the
        // calls fails.
        let success = futures::future::try_join_all(get_missing_digest_calls)
            .await
            .map(|responses| {
                let mut result: IndexSet<Digest> = IndexSet::new();
                for response in responses {
                    result.extend(response.missing_blob_digests);
                }
                result
            });

        let request_metadata = context.get_request_metadata();
        match success {
            Ok(result) => Ok(result),
            Err(e) => {
                // catchingAsync(success, RuntimeException.class, ...): the failure of a call is
                // the `StatusRuntimeException` of its stub (an unchecked exception); every other
                // failure of `getMissingDigests` is an `IOException` and passes through.
                let message = errors::downcast_ref::<StatusRuntimeException>(&e)
                    .map(|sre| StatusRuntimeException::format_throwable_message(&sre.status));
                match message {
                    // NEEDS(errors): `crate::common::errors::io_exception_with_cause(message,
                    // cause)` is `new IOException(message, cause)`: an `io::Error` that displays
                    // `message` and that `errors::downcast_ref` (so `status_from_throwable`)
                    // looks through to `cause`, as `Status.fromThrowable` walks the cause chain.
                    Some(message) => Err(errors::io_exception_with_cause(
                        format!(
                            "findMissingBlobs({}) for {}: {}",
                            request_builder.blob_digests.len(),
                            request_metadata.action_id,
                            message
                        ),
                        e,
                    )),
                    None => Err(e),
                }
            }
        }
    }
}

#[async_trait]
impl RemoteCacheClient for GrpcCacheClient {
    fn cas_upload_cache(&self) -> &AsyncTaskCacheNoResult<Digest> {
        &self.cas_upload_cache
    }

    // Java: com/google/devtools/build/lib/remote/GrpcCacheClient.java:390 GrpcCacheClient.getServerCapabilities
    fn get_server_capabilities(&self) -> io::Result<ServerCapabilities> {
        self.channel.get_server_capabilities()
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    // Java: com/google/devtools/build/lib/remote/GrpcCacheClient.java:395 GrpcCacheClient.getAuthority
    async fn get_authority(&self) -> io::Result<String> {
        // channel.withChannelFuture(ch -> Futures.immediateFuture(ch.authority()))
        self.channel.get_authority_async().await
    }

    // Java: com/google/devtools/build/lib/remote/GrpcCacheClient.java:400 GrpcCacheClient.downloadActionResult
    async fn download_action_result(
        &self,
        context: &RemoteActionExecutionContext<'_>,
        action_key: &ActionKey,
        inline_out_err: bool,
        inline_output_files: &HashSet<String>,
    ) -> io::Result<Option<ActionResult>> {
        // PORT: java-hash-order. `addAllInlineOutputFiles(inlineOutputFiles)` adds the names in
        // the iteration order of the caller's set; the Rust set has no stable order, so the
        // names are sent sorted (the order of this hint does not change its meaning).
        let mut inline_output_files: Vec<String> = inline_output_files.iter().cloned().collect();
        inline_output_files.sort();
        let request = GetActionResultRequest {
            instance_name: self.options.remote_instance_name.clone(),
            digest_function: self.digest_util.get_digest_function() as i32,
            action_digest: Some(action_key.digest().clone()),
            inline_stderr: inline_out_err,
            inline_stdout: inline_out_err,
            inline_output_files,
        };
        utils::refresh_if_unauthenticated_async(
            || {
                self.retrier.execute_async(|| async {
                    GrpcCacheClient::handle_status(
                        // channel.withChannelFuture(
                        //     channel -> acFutureStub(context, channel).getActionResult(request))
                        self.channel
                            .unary::<GetActionResultRequest, ActionResult>(
                                methods::GET_ACTION_RESULT,
                                request.clone(),
                                self.ac_future_stub(context),
                            )
                            .await,
                    )
                })
            },
            &*self.call_credentials_provider,
        )
        .await
    }

    // Java: com/google/devtools/build/lib/remote/GrpcCacheClient.java:425 GrpcCacheClient.uploadActionResult
    async fn upload_action_result(
        &self,
        context: &RemoteActionExecutionContext<'_>,
        action_key: &ActionKey,
        action_result: &ActionResult,
    ) -> io::Result<()> {
        let upload = utils::refresh_if_unauthenticated_async(
            || {
                self.retrier.execute_async(|| async {
                    // Futures.catchingAsync(..., StatusRuntimeException.class,
                    //     (sre) -> Futures.immediateFailedFuture(new IOException(sre)))
                    io_result(
                        self.channel
                            .unary::<UpdateActionResultRequest, ActionResult>(
                                methods::UPDATE_ACTION_RESULT,
                                UpdateActionResultRequest {
                                    instance_name: self.options.remote_instance_name.clone(),
                                    digest_function: self.digest_util.get_digest_function() as i32,
                                    action_digest: Some(action_key.digest().clone()),
                                    action_result: Some(action_result.clone()),
                                    ..Default::default()
                                },
                                self.ac_future_stub(context),
                            )
                            .await,
                    )
                })
            },
            &*self.call_credentials_provider,
        );

        upload.await.map(|_ac| ())
    }

    // Java: com/google/devtools/build/lib/remote/GrpcCacheClient.java:452 GrpcCacheClient.downloadBlob
    async fn download_blob(
        &self,
        context: &RemoteActionExecutionContext<'_>,
        digest: &Digest,
        out: &mut (dyn Write + Send),
    ) -> io::Result<()> {
        if digest.size_bytes == 0 {
            return Ok(());
        }

        let out = if self.options.remote_verify_downloads {
            DownloadOut::Digest(self.digest_util.new_digest_output_stream(out))
        } else {
            DownloadOut::Plain(out)
        };

        self.download_blob_counting(context, digest, CountingOutputStream::new(out))
            .await
    }

    // Java: com/google/devtools/build/lib/remote/GrpcCacheClient.java:629 GrpcCacheClient.uploadBlobImpl
    async fn upload_blob_impl(
        &self,
        context: &RemoteActionExecutionContext<'_>,
        digest: &Digest,
        blob: &dyn Blob,
    ) -> io::Result<()> {
        let upload = self
            .upload_chunker(
                context,
                digest,
                Chunker::builder()
                    .set_input(digest.size_bytes, blob)
                    .set_compressed(self.should_compress(digest))
                    .build(),
            )
            .await;
        let e = match upload {
            Ok(()) => return Ok(()),
            Err(e) => e,
        };
        // var cause = e.getCause(); if (!(cause instanceof StatusRuntimeException sre)) ...
        let Some(code) =
            errors::downcast_ref::<StatusRuntimeException>(&e).map(|sre| sre.status.code())
        else {
            return Err(e);
        };
        let blob_description = blob.description();
        // INVALID_ARGUMENT is returned in case of a digest mismatch, which can hint at concurrent
        // modifications to the blob's source. Print it to help the user debug such issues.
        // https://github.com/bazelbuild/bazel/blob/ec36eacc31678ecf4b5c25f9ab7ab166330aff28/third_party/remoteapis/build/bazel/remote/execution/v2/remote_execution.proto#L283-L286
        if code == tonic::Code::InvalidArgument
            && let Some(blob_description) = blob_description
        {
            // NEEDS(errors): see `find_missing_digests`.
            return Err(errors::io_exception_with_cause(
                format!("while uploading {blob_description}: {e}"),
                e,
            ));
        }
        Err(e)
    }

    // Java: com/google/devtools/build/lib/remote/GrpcCacheClient.java:296 GrpcCacheClient.close
    fn close(&self) {
        if self.closed.swap(true, Ordering::SeqCst) {
            return;
        }
        self.channel.release();
    }
}
