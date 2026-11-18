//! Port of `lib/remote/ByteStreamUploader.java`.

use std::fmt;
use std::future::Future;
use std::io;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::task::{Context, Poll};

use tokio::sync::{Mutex, Semaphore, mpsc};
use tokio_stream::wrappers::ReceiverStream;
use uuid::Uuid;

use crate::authandtls::call_credentials_provider::CallCredentialsProvider;
use crate::chunker::Chunker;
use crate::common::RemoteActionExecutionContext;
use crate::common::errors;
use crate::grpc::{CallOptions, ReferenceCountedChannel, methods};
use crate::reapi::bytestream::{
    QueryWriteStatusRequest, QueryWriteStatusResponse, WriteRequest, WriteResponse,
};
use crate::reapi::{Digest, RequestMetadata, digest_function};
use crate::remote_retrier::{ProgressiveBackoff, RemoteRetrier};
use crate::retrier::Retrier;
use crate::util::digest_util::DigestUtil;
use crate::util::utils;

/// The number of `WriteRequest`s that may wait between the [`Writer`] and the transport.
///
/// PORT: Java's `Writer.run()` writes chunks while `requestObserver.isReady()`, i.e. while the
/// transport buffers less than its on-ready threshold. Here the requests of a `Write` call are
/// a bounded queue the transport drains; the writer reads the next chunk from the [`Chunker`]
/// only once the queue has room, so at most this many chunks (plus the one being read) are in
/// memory.
const WRITE_QUEUE_CAPACITY: usize = 1;

/// A client implementing the `Write` method of the `ByteStream` gRPC service.
///
/// The uploader supports reference counting to easily be shared between components with
/// different lifecyles. After instantiation the reference count is `1`.
///
/// See `ReferenceCounted` for more information on reference counting.
// Java: com/google/devtools/build/lib/remote/ByteStreamUploader.java:62 ByteStreamUploader
pub struct ByteStreamUploader {
    /// PORT: `@Nullable String`; the empty string stands for `null` (every use is behind
    /// `Strings.isNullOrEmpty`).
    instance_name: String,
    channel: ReferenceCountedChannel,
    call_credentials_provider: Arc<dyn CallCredentialsProvider>,
    retrier: Arc<RemoteRetrier>,
    digest_function: digest_function::Value,
    query_write_status_implemented: AtomicBool,

    opened_file_permits: Option<Semaphore>,
}

impl ByteStreamUploader {
    /// Creates a new instance.
    ///
    /// * `instance_name`: the instance name to be prepended to resource name of the `Write`
    ///   call. See the `ByteStream` service definition for details
    /// * `channel`: the channel to use for calls
    /// * `call_credentials_provider`: the credentials provider to use for authentication.
    /// * `retrier`: the [`RemoteRetrier`] whose backoff strategy to use for retry timings.
    // Java: com/google/devtools/build/lib/remote/ByteStreamUploader.java:81 ByteStreamUploader.<init>
    pub fn new(
        instance_name: &str,
        channel: ReferenceCountedChannel,
        call_credentials_provider: Arc<dyn CallCredentialsProvider>,
        retrier: Arc<RemoteRetrier>,
        maximum_open_files: i32,
        digest_function: digest_function::Value,
    ) -> ByteStreamUploader {
        let opened_file_permits = if maximum_open_files != -1 {
            if maximum_open_files < 0 {
                blaze_util::unported!(
                    "ByteStreamUploader.<init> (java.util.concurrent.Semaphore with a negative number of permits)"
                );
            }
            Some(Semaphore::new(maximum_open_files as usize))
        } else {
            None
        };
        ByteStreamUploader {
            instance_name: instance_name.to_string(),
            channel,
            call_credentials_provider,
            retrier,
            opened_file_permits,
            digest_function,
            query_write_status_implemented: AtomicBool::new(true),
        }
    }

    /// Uploads a BLOB asynchronously to the remote `ByteStream` service. The returned future
    /// completes with the success/failure of the upload.
    ///
    /// Uploads are retried according to the specified [`RemoteRetrier`]. Retrying is
    /// transparent to the user of this API.
    ///
    /// * `digest`: the [`Digest`] of the data to upload.
    /// * `chunker`: the data to upload.
    ///
    /// PORT: Java's callers close the `Chunker` after the upload; here the chunker is moved in
    /// and dropped (closed) when the upload is over. Dropping the returned future cancels the
    /// `Write` call (Java: `uploadResult.isCancelled()` → `requestObserver.cancel("cancelled by
    /// user", null)`).
    ///
    /// PORT: `Futures.catchingAsync(.., StatusRuntimeException.class, sre -> new
    /// IOException(sre))` is the identity: a status already travels as the `io::Error` that
    /// `new IOException(sre)` is (docs/remote.md "Exceptions").
    // Java: com/google/devtools/build/lib/remote/ByteStreamUploader.java:109 ByteStreamUploader.uploadBlobAsync
    pub async fn upload_blob_async(
        &self,
        context: &RemoteActionExecutionContext<'_>,
        digest: &Digest,
        mut chunker: Chunker<'_>,
    ) -> io::Result<()> {
        self.start_async_upload(context, digest, &mut chunker).await
    }

    // Java: com/google/devtools/build/lib/remote/ByteStreamUploader.java:118 ByteStreamUploader.buildUploadResourceName
    fn build_upload_resource_name(
        &self,
        instance_name: &str,
        uuid: Uuid,
        digest: &Digest,
        compressed: bool,
    ) -> String {
        let mut resource_name;

        if DigestUtil::is_old_style_digest_function(self.digest_function) {
            // compressed ? "uploads/%s/compressed-blobs/zstd/%s/%d" : "uploads/%s/blobs/%s/%d"
            resource_name = if compressed {
                format!(
                    "uploads/{}/compressed-blobs/zstd/{}/{}",
                    uuid, digest.hash, digest.size_bytes
                )
            } else {
                format!(
                    "uploads/{}/blobs/{}/{}",
                    uuid, digest.hash, digest.size_bytes
                )
            };
        } else {
            // compressed ? "uploads/%s/compressed-blobs/zstd/%s/%s/%d" : "uploads/%s/blobs/%s/%s/%d"
            let digest_function_name = self.digest_function.as_str_name().to_ascii_lowercase();
            resource_name = if compressed {
                format!(
                    "uploads/{}/compressed-blobs/zstd/{}/{}/{}",
                    uuid, digest_function_name, digest.hash, digest.size_bytes
                )
            } else {
                format!(
                    "uploads/{}/blobs/{}/{}/{}",
                    uuid, digest_function_name, digest.hash, digest.size_bytes
                )
            };
        }
        if !instance_name.is_empty() {
            resource_name = format!("{instance_name}/{resource_name}");
        }
        resource_name
    }

    /// Starts a file upload and returns the result of the upload.
    // Java: com/google/devtools/build/lib/remote/ByteStreamUploader.java:145 ByteStreamUploader.startAsyncUpload
    async fn start_async_upload(
        &self,
        context: &RemoteActionExecutionContext<'_>,
        digest: &Digest,
        chunker: &mut Chunker<'_>,
    ) -> io::Result<()> {
        chunker.reset()?;

        // PORT: Java fails the returned future with this IllegalStateException; an unchecked
        // exception is a panic with the Java text.
        blaze_util::check_state!(
            chunker.get_uncompressed_size() == digest.size_bytes,
            "Expected chunker size of {}, got {}",
            digest.size_bytes,
            chunker.get_uncompressed_size()
        );

        let upload_id = Uuid::new_v4();
        let resource_name = self.build_upload_resource_name(
            &self.instance_name,
            upload_id,
            digest,
            chunker.is_compressed(),
        );
        // PORT: Java blocks the calling thread in `openedFilePermits.acquire()`; here the upload
        // waits for its permit without blocking a thread. The permit is released when it is
        // dropped: when the upload completes, fails or is cancelled (Java: the listener on
        // `currUpload`).
        let _permit = match &self.opened_file_permits {
            Some(opened_file_permits) => match opened_file_permits.acquire().await {
                Ok(permit) => Some(permit),
                Err(e) => {
                    return Err(io::Error::new(
                        io::ErrorKind::Interrupted,
                        format!(
                            "Unexpected interrupt while acquiring open file permit. Original error message: {e}"
                        ),
                    ));
                }
            },
            None => None,
        };
        let new_upload = AsyncUpload::new(
            self,
            context,
            &self.channel,
            &*self.call_credentials_provider,
            &self.retrier,
            resource_name,
            chunker,
        );
        new_upload.start().await
    }

    /// `getOpenedFilePermits()` (`@VisibleForTesting`).
    // Java: com/google/devtools/build/lib/remote/ByteStreamUploader.java:472 ByteStreamUploader.getOpenedFilePermits
    pub fn get_opened_file_permits(&self) -> Option<&Semaphore> {
        self.opened_file_permits.as_ref()
    }
}

/// Signal that the blob already exists on the server, so upload should complete early but
/// successfully.
///
/// PORT: travels as the payload of an `io::Error` (through the retrier, as the Java exception
/// does) until [`AsyncUpload::start`] catches it.
// Java: com/google/devtools/build/lib/remote/ByteStreamUploader.java:191 ByteStreamUploader.AlreadyExists
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct AlreadyExists;

impl AlreadyExists {
    // Java: com/google/devtools/build/lib/remote/ByteStreamUploader.java:192 ByteStreamUploader.AlreadyExists.<init>
    fn new() -> AlreadyExists {
        AlreadyExists
    }
}

impl fmt::Display for AlreadyExists {
    /// `Throwable.toString()` of an exception without a message.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("com.google.devtools.build.lib.remote.ByteStreamUploader$AlreadyExists")
    }
}

impl std::error::Error for AlreadyExists {}

impl From<AlreadyExists> for io::Error {
    fn from(e: AlreadyExists) -> io::Error {
        io::Error::other(e)
    }
}

/// The fields of [`AsyncUpload`] that an attempt (`call()`) mutates.
struct AsyncUploadState<'a, 'c> {
    chunker: &'a mut Chunker<'c>,
    last_committed_offset: i64,
}

// Java: com/google/devtools/build/lib/remote/ByteStreamUploader.java:197 ByteStreamUploader.AsyncUpload
struct AsyncUpload<'a, 'c> {
    /// PORT: `ByteStreamUploader.this` of the Java inner class (`queryWriteStatusImplemented`).
    uploader: &'a ByteStreamUploader,
    /// PORT: Java keeps the `RemoteActionExecutionContext` and only reads
    /// `context.getRequestMetadata()` from it.
    request_metadata: RequestMetadata,
    channel: &'a ReferenceCountedChannel,
    call_credentials_provider: &'a dyn CallCredentialsProvider,
    retrier: &'a Retrier,
    resource_name: String,
    progressive_backoff: ProgressiveBackoff,

    /// PORT: `chunker` and `lastCommittedOffset`. The retrier invokes `call()` through a shared
    /// reference, one attempt at a time; the attempt that runs holds this lock.
    state: Mutex<AsyncUploadState<'a, 'c>>,
}

impl<'a, 'c> AsyncUpload<'a, 'c> {
    // Java: com/google/devtools/build/lib/remote/ByteStreamUploader.java:208 ByteStreamUploader.AsyncUpload.<init>
    fn new(
        uploader: &'a ByteStreamUploader,
        context: &RemoteActionExecutionContext<'_>,
        channel: &'a ReferenceCountedChannel,
        call_credentials_provider: &'a dyn CallCredentialsProvider,
        retrier: &'a Retrier,
        resource_name: String,
        chunker: &'a mut Chunker<'c>,
    ) -> AsyncUpload<'a, 'c> {
        AsyncUpload {
            uploader,
            request_metadata: context.get_request_metadata().clone(),
            channel,
            call_credentials_provider,
            retrier,
            // new ProgressiveBackoff(retrier::newBackoff)
            progressive_backoff: ProgressiveBackoff::new(retrier.backoff_supplier()),
            resource_name,
            state: Mutex::new(AsyncUploadState {
                chunker,
                last_committed_offset: -1,
            }),
        }
    }

    // Java: com/google/devtools/build/lib/remote/ByteStreamUploader.java:224 ByteStreamUploader.AsyncUpload.start
    async fn start(&self) -> io::Result<()> {
        let this = self;
        let committed_size = utils::refresh_if_unauthenticated_async(
            move || {
                this.retrier
                    .execute_async_with_backoff(move || this.call(), &this.progressive_backoff)
            },
            this.call_credentials_provider,
        )
        .await;
        let result = match committed_size {
            Ok(committed_size) => self.check_committed_size(committed_size).await,
            Err(e) => Err(e),
        };
        match result {
            // Futures.catching(.., AlreadyExists.class, ae -> null, ..)
            Err(e) if errors::downcast_ref::<AlreadyExists>(&e).is_some() => Ok(()),
            result => result,
        }
    }

    /// Check the committed_size the server returned makes sense after a successful full upload.
    // Java: com/google/devtools/build/lib/remote/ByteStreamUploader.java:244 ByteStreamUploader.AsyncUpload.checkCommittedSize
    async fn check_committed_size(&self, committed_size: i64) -> io::Result<()> {
        let state = self.state.lock().await;
        let chunker = &*state.chunker;
        let expected = chunker.get_offset();

        if committed_size == expected {
            // Both compressed and uncompressed uploads can succeed with this result.
            return Ok(());
        }

        if chunker.is_compressed() {
            if committed_size == -1 {
                // Returned early, blob already available.
                return Ok(());
            }

            return Err(errors::io_exception(format!(
                "compressed write incomplete: committed_size {} is neither -1 nor total {} - {}",
                committed_size, expected, self.resource_name
            )));
        }

        // Uncompressed upload failed.
        Err(errors::io_exception(format!(
            "write incomplete: committed_size {} for {} total - {}",
            committed_size, expected, self.resource_name
        )))
    }

    /// Make one attempt to upload. If this is the first attempt, uploading starts from the
    /// beginning of the blob. On later attempts, the server is queried to see at which offset
    /// upload should resume. The final committed size from the server is returned on success.
    // Java: com/google/devtools/build/lib/remote/ByteStreamUploader.java:277 ByteStreamUploader.AsyncUpload.call
    async fn call(&self) -> io::Result<i64> {
        let mut state = self.state.lock().await;
        let first_attempt = state.last_committed_offset == -1;
        let committed_size = if first_attempt {
            0
        } else {
            self.query().await?
        };
        if !first_attempt && committed_size > state.last_committed_offset {
            // We have made progress on this upload in the last request. Reset the backoff so
            // that this request has a full deck of retries
            self.progressive_backoff.reset();
        }
        state.last_committed_offset = committed_size;
        self.upload(&mut *state.chunker, committed_size).await
    }

    /// The decoration of both ByteStream stubs: the request metadata and the call credentials.
    // Java: com/google/devtools/build/lib/remote/ByteStreamUploader.java:295 ByteStreamUploader.AsyncUpload.bsFutureStub
    // Java: com/google/devtools/build/lib/remote/ByteStreamUploader.java:302 ByteStreamUploader.AsyncUpload.bsAsyncStub
    fn bs_stub(&self) -> CallOptions {
        CallOptions::new()
            .with_request_metadata(self.request_metadata.clone())
            .with_call_credentials(self.call_credentials_provider.get_call_credentials())
    }

    // Java: com/google/devtools/build/lib/remote/ByteStreamUploader.java:309 ByteStreamUploader.AsyncUpload.query
    async fn query(&self) -> io::Result<i64> {
        if !self
            .uploader
            .query_write_status_implemented
            .load(Ordering::SeqCst)
        {
            // Without server support for QueryWriteStatus, we have no choice but to restart the
            // entire upload.
            return Ok(0);
        }
        let committed_size_future = self
            .channel
            .unary::<QueryWriteStatusRequest, QueryWriteStatusResponse>(
                methods::QUERY_WRITE_STATUS,
                QueryWriteStatusRequest {
                    resource_name: self.resource_name.clone(),
                },
                self.bs_stub(),
            )
            .await;
        match committed_size_future {
            Ok(r) => {
                if r.complete {
                    Err(AlreadyExists::new().into())
                } else {
                    Ok(r.committed_size)
                }
            }
            Err(status) => {
                if status.code() == tonic::Code::Unimplemented {
                    self.uploader
                        .query_write_status_implemented
                        .store(false, Ordering::SeqCst);
                    return Ok(0);
                }
                Err(errors::status_to_io_error(status))
            }
        }
    }

    // Java: com/google/devtools/build/lib/remote/ByteStreamUploader.java:343 ByteStreamUploader.AsyncUpload.upload
    async fn upload(&self, chunker: &mut Chunker<'_>, pos: i64) -> io::Result<i64> {
        Writer::new(&self.resource_name, chunker, pos)
            .write(self.channel, self.bs_stub())
            .await
    }
}

/// The observer of one `Write` call.
///
/// PORT: Java's `Writer` is driven by gRPC callbacks (`beforeStart`, the on-ready handler
/// `run()`, `onNext`, `onCompleted`, `onError`) and completes the `uploadResult` future;
/// [`Writer::write`] is that event loop and its return value is `uploadResult`.
// Java: com/google/devtools/build/lib/remote/ByteStreamUploader.java:353 ByteStreamUploader.Writer
struct Writer<'w, 'a, 'c> {
    chunker: &'a mut Chunker<'c>,
    pos: i64,
    resource_name: &'w str,
    committed_size: i64,
    first: bool,
    finished_writing: bool,
}

impl<'w, 'a, 'c> Writer<'w, 'a, 'c> {
    // Java: com/google/devtools/build/lib/remote/ByteStreamUploader.java:364 ByteStreamUploader.Writer.<init>
    fn new(resource_name: &'w str, chunker: &'a mut Chunker<'c>, pos: i64) -> Writer<'w, 'a, 'c> {
        Writer {
            resource_name,
            chunker,
            pos,
            committed_size: -1,
            first: true,
            finished_writing: false,
        }
    }

    /// Starts the `Write` call and runs it to its end: `bsAsyncStub(channel).write(writer)`
    /// with the body of `run()` as the loop that feeds the request stream.
    ///
    /// PORT: the request stream handed to the channel must be `'static`, while the chunks come
    /// from a borrowed [`Chunker`]. The stream is the receiving end of a bounded queue
    /// ([`WRITE_QUEUE_CAPACITY`]) of owned `WriteRequest`s; this task reads a chunk and queues
    /// it only when the queue has room (`requestObserver.isReady()`), and it looks at the call
    /// for a response or an error before each chunk is queued, as gRPC delivers `onCompleted` /
    /// `onError` between two runs of the on-ready handler. Closing the queue is
    /// `requestObserver.onCompleted()`; dropping the [`WriteCall`] is
    /// `requestObserver.cancel(..)`.
    // Java: com/google/devtools/build/lib/remote/ByteStreamUploader.java:373 ByteStreamUploader.Writer.beforeStart
    // Java: com/google/devtools/build/lib/remote/ByteStreamUploader.java:386 ByteStreamUploader.Writer.run
    async fn write(
        mut self,
        channel: &ReferenceCountedChannel,
        call_options: CallOptions,
    ) -> io::Result<i64> {
        let (request_observer, requests) = mpsc::channel::<WriteRequest>(WRITE_QUEUE_CAPACITY);
        let mut call = WriteCall::start(channel.clone(), requests, call_options);

        // while (requestObserver.isReady())
        loop {
            let permit = tokio::select! {
                biased;
                result = &mut call => return self.on_close(result),
                permit = request_observer.reserve() => permit,
            };
            let Ok(permit) = permit else {
                // The transport gave up the request stream: the call is about to close.
                break;
            };
            let mut request = WriteRequest::default();
            if self.first {
                self.first = false;
                // A failure here is `uploadResult.setException(e)` followed by
                // `requestObserver.cancel("failed to seek chunk", e)`: the call is dropped.
                self.seek_chunker()?;
                // Resource name only needs to be set on the first write for each file.
                request.resource_name = self.resource_name.to_string();
            }
            // PORT: `chunker.next()` throws the unchecked NoSuchElementException when the server
            // asked to resume at the end of the blob; gRPC catches what an on-ready handler
            // throws and cancels the call with this status, which `onError` then reports.
            if !self.chunker.has_next() {
                return Err(self.on_error(tonic::Status::cancelled("Failed to call onReady.")));
            }
            let chunk = match self.chunker.next() {
                Ok(chunk) => chunk,
                Err(e) => {
                    // requestObserver.cancel("Failed to read next chunk.", e), which comes back
                    // as `onError(CANCELLED)`.
                    // NEEDS(channel): optional. A way to cancel a call with a description and a
                    // cause (`ClientCall.cancel(message, cause)`), so that the channel-level
                    // listeners (the gRPC log) see this status instead of the "Cancelled by
                    // client" of a dropped call.
                    let mut status = tonic::Status::cancelled("Failed to read next chunk.");
                    status.set_source(Arc::new(e));
                    return Err(self.on_error(status));
                }
            };
            let is_last_chunk = !self.chunker.has_next();
            request.data = chunk.get_data().to_vec();
            request.write_offset = chunk.get_offset();
            request.finish_write = is_last_chunk;
            // The server may have answered while the chunk was read.
            if let Poll::Ready(result) = futures::poll!(&mut call) {
                return self.on_close(result);
            }
            // requestObserver.onNext(request)
            permit.send(request);
            if is_last_chunk {
                // requestObserver.onCompleted() is the drop of `request_observer` below.
                self.finished_writing = true;
                break;
            }
        }
        drop(request_observer);

        let result = call.await;
        self.on_close(result)
    }

    /// The end of the call: `onNext(response)` and `onCompleted()`, or `onError(t)`.
    fn on_close(&mut self, result: Result<WriteResponse, tonic::Status>) -> io::Result<i64> {
        match result {
            Ok(response) => {
                self.on_next(response);
                self.on_completed()
            }
            Err(status) => Err(self.on_error(status)),
        }
    }

    /// PORT: `Err` is `uploadResult.setException(e)` plus the cancellation of the call (the
    /// caller drops it).
    // Java: com/google/devtools/build/lib/remote/ByteStreamUploader.java:419 ByteStreamUploader.Writer.seekChunker
    fn seek_chunker(&mut self) -> io::Result<()> {
        if let Err(mut e) = self.chunker.seek(self.pos) {
            // PORT: a failure of this reset is `e.addSuppressed(resetException)`, which an
            // `io::Error` cannot carry.
            let _ = self.chunker.reset();
            let too_many_open_files_error = "Too many open files";
            // PORT: `e.getMessage()` is the `Display` of the `io::Error`.
            let message = e.to_string();
            if message
                .to_ascii_lowercase()
                .contains(&too_many_open_files_error.to_ascii_lowercase())
            {
                let new_message = format!(
                    "An IOException was thrown because the process opened too many files. We recommend \
                     setting --bep_maximum_open_remote_upload_files flag to a number lower than \
                     your system default (run 'ulimit -a' for *nix-based operating systems). \
                     Original error message: {message}"
                );
                // PORT: `new IOException(newMessage, e)`; the cause is not kept.
                e = errors::io_exception(new_message);
            }
            return Err(e);
        }
        Ok(())
    }

    // Java: com/google/devtools/build/lib/remote/ByteStreamUploader.java:446 ByteStreamUploader.Writer.onNext
    fn on_next(&mut self, response: WriteResponse) {
        self.committed_size = response.committed_size;
    }

    // Java: com/google/devtools/build/lib/remote/ByteStreamUploader.java:451 ByteStreamUploader.Writer.onCompleted
    fn on_completed(&mut self) -> io::Result<i64> {
        if self.finished_writing {
            Ok(self.committed_size)
        } else {
            // Server completed succesfully before we finished writing all the data, meaning the
            // blob already exists. The server is supposed to set committed_size to the size of
            // the blob (for uncompressed uploads) or -1 (for compressed uploads), but we do not
            // verify this.
            // requestObserver.cancel("server has returned early", null): the call is over.
            Err(AlreadyExists::new().into())
        }
    }

    // Java: com/google/devtools/build/lib/remote/ByteStreamUploader.java:464 ByteStreamUploader.Writer.onError
    fn on_error(&mut self, t: tonic::Status) -> io::Error {
        // requestObserver.cancel("failed", t): the call is over.
        if t.code() == tonic::Code::AlreadyExists {
            AlreadyExists::new().into()
        } else {
            errors::status_to_io_error(t)
        }
    }
}

/// One `ByteStream.Write` call, running on the remote executor.
///
/// PORT: a gRPC call makes progress on the channel's event loop whatever the thread that feeds
/// it does (Java reads the chunks on a gRPC executor thread). The call is therefore its own
/// task, so that a response or an error arrives while [`Writer::write`] reads from the
/// [`Chunker`]; nothing in it borrows. Dropping this handle aborts the task, which cancels the
/// call (`ClientCall.cancel`).
struct WriteCall {
    handle: tokio::task::JoinHandle<Result<WriteResponse, tonic::Status>>,
}

impl WriteCall {
    /// `bsAsyncStub(channel).write(writer)`.
    // Java: com/google/devtools/build/lib/remote/ByteStreamUploader.java:347 ByteStreamUploader.AsyncUpload.upload
    fn start(
        channel: ReferenceCountedChannel,
        requests: mpsc::Receiver<WriteRequest>,
        call_options: CallOptions,
    ) -> WriteCall {
        WriteCall {
            handle: crate::runtime::runtime().spawn(async move {
                channel
                    .client_streaming::<WriteRequest, WriteResponse, _>(
                        methods::WRITE,
                        ReceiverStream::new(requests),
                        call_options,
                    )
                    .await
            }),
        }
    }
}

impl Future for WriteCall {
    type Output = Result<WriteResponse, tonic::Status>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        match Pin::new(&mut self.handle).poll(cx) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(Ok(result)) => Poll::Ready(result),
            Poll::Ready(Err(e)) => {
                if e.is_panic() {
                    std::panic::resume_unwind(e.into_panic());
                }
                // The task was aborted, which only the drop of this handle does.
                Poll::Ready(Err(tonic::Status::cancelled("Cancelled by client")))
            }
        }
    }
}

impl Drop for WriteCall {
    fn drop(&mut self) {
        self.handle.abort();
    }
}
