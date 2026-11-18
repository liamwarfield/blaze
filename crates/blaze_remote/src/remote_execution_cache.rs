//! Port of `lib/remote/RemoteExecutionCache.java`.
//!
//! PORT: Java builds `ensureInputsPresent` from RxJava flows whose subscriptions live on other
//! threads. Per docs/remote.md the port runs everything on the calling task: one
//! `ensure_inputs_present` call owns the futures of its upload tasks and drives them together
//! with its `findMissingDigests` query. The pieces of the Java machinery map as follows:
//!
//! - `findMissingCache` is the same [`AsyncTaskCache`] of "is this digest missing remotely".
//! - `UploadTask.continuation` (the `SingleEmitter<Boolean>` a task gets when the cache makes it
//!   the one that has to supply the answer) is a [`Continuation`]: a `oneshot` sender registered
//!   by the task the cache runs. Java learns who got a continuation while it subscribes the
//!   tasks; the port polls every task once for the same effect.
//! - `UploadTask.completion` is the output of the task's future, and disposing a task
//!   (`uploadTask.disposable`) is dropping that future, which happens when the call returns or
//!   is itself dropped (Java: the calling thread is interrupted).

use std::io::{self, Read};
use std::ops::Deref;
use std::sync::Arc;
use std::task::Poll;

use async_trait::async_trait;
use blaze_exec::actions::VirtualActionInput;
use blaze_optdefs::misc::remote_options::ChunkingFunctionValue;
use blaze_vfs::Path;
use futures::future::BoxFuture;
use futures::stream::{FuturesUnordered, StreamExt};
use indexmap::IndexMap;
use parking_lot::{Mutex, RwLock};
use tokio::sync::oneshot;

use crate::combined_cache::CombinedCache;
use crate::common::bulk_transfer_exception::BulkTransferException;
use crate::common::cache_not_found_exception::CacheNotFoundException;
use crate::common::errors;
use crate::common::remote_action_execution_context::RemoteActionExecutionContext;
use crate::common::remote_cache_client::{Blob, RemoteCacheClient};
use crate::common::remote_path_resolver::RemotePathResolver;
use crate::disk::DiskCacheClient;
use crate::merkletree::{MerkleTreeUploadable, MerkleTreeUploader};
use crate::reapi::Digest;
use crate::runtime::get_from_future;
use crate::util::async_task_cache::AsyncTaskCache;
use crate::util::digest_util::DigestUtil;

/// An interface used to check whether a given `Path` is available without contacting the remote
/// cache, i.e., it is present on the local disk, perhaps after being downloaded from the disk
/// cache.
// Java: com/google/devtools/build/lib/remote/RemoteExecutionCache.java:87 RemoteExecutionCache.RemotePathChecker
#[async_trait]
pub trait RemotePathChecker: Send + Sync {
    /// `isAvailableLocally(context, path)`.
    async fn is_available_locally(
        &self,
        context: &RemoteActionExecutionContext<'_>,
        path: &Path,
    ) -> io::Result<bool>;
}

/// The initial value of `RemoteExecutionCache.remotePathChecker`.
// Java: com/google/devtools/build/lib/remote/RemoteExecutionCache.java:99 RemoteExecutionCache.remotePathChecker
struct DefaultRemotePathChecker;

#[async_trait]
impl RemotePathChecker for DefaultRemotePathChecker {
    // Java: com/google/devtools/build/lib/remote/RemoteExecutionCache.java:102 RemoteExecutionCache.remotePathChecker.isAvailableLocally
    async fn is_available_locally(
        &self,
        _context: &RemoteActionExecutionContext<'_>,
        _path: &Path,
    ) -> io::Result<bool> {
        // PORT: `RemoteActionFileSystem` (the action file system of build without the bytes) is
        // not ported, so no `Path` belongs to one and `fs instanceof RemoteActionFileSystem` is
        // false for every path. The rest of the Java method (download the file from the disk
        // cache if the read policy allows it, then ask the host file system whether it exists)
        // only runs for such a file system.
        Ok(true)
    }
}

/// A `CombinedCache` with additional functionality needed for remote execution.
///
/// PORT: Java's `RemoteExecutionCache extends CombinedCache`; the port owns the
/// [`CombinedCache`] and derefs to it ([`RemoteExecutionCache::combined_cache`]).
// Java: com/google/devtools/build/lib/remote/RemoteExecutionCache.java:78 RemoteExecutionCache
pub struct RemoteExecutionCache {
    combined_cache: Arc<CombinedCache>,

    /// Deduplicates concurrent `findMissingDigests` queries for the same digest across
    /// overlapping [`RemoteExecutionCache::ensure_inputs_present_with_additional_inputs`]
    /// invocations. Results reported as "present" stay cached until explicitly invalidated, but
    /// a "missing" result is invalidated as soon as the triggered upload attempt terminates.
    // Java: com/google/devtools/build/lib/remote/RemoteExecutionCache.java:97 RemoteExecutionCache.findMissingCache
    find_missing_cache: AsyncTaskCache<Digest, bool>,

    remote_path_checker: RwLock<Arc<dyn RemotePathChecker>>,
}

impl std::fmt::Debug for RemoteExecutionCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RemoteExecutionCache")
            .field("combined_cache", &self.combined_cache)
            .finish_non_exhaustive()
    }
}

impl Deref for RemoteExecutionCache {
    type Target = CombinedCache;

    fn deref(&self) -> &CombinedCache {
        &self.combined_cache
    }
}

/// `UploadTask.continuation` of the upload task of `digest`: the task has to tell
/// `findMissingCache` whether the digest is missing.
// Java: com/google/devtools/build/lib/remote/RemoteExecutionCache.java:314 RemoteExecutionCache.UploadTask
struct Continuation {
    digest: Digest,
    continuation: oneshot::Sender<bool>,
}

/// What happened while waiting for the upload tasks.
enum Step {
    /// The upload task with this index completed (`UploadTask.completion`).
    Completed(usize, io::Result<()>),
    /// An upload task got a continuation after the `findMissingDigests` query was sent.
    GotContinuation,
    /// All upload tasks completed.
    Done,
}

/// `new CancellationException("cancelled")`.
fn cancellation_exception() -> io::Error {
    io::Error::new(
        io::ErrorKind::Interrupted,
        "CancellationException: cancelled",
    )
}

/// `Digest.toString()`: the protobuf text format of the message.
///
/// PORT: `blaze_protos` has no `TextFormat`; this is its output for the two scalar fields of a
/// `Digest`.
// Java: com/google/protobuf/TextFormat.java TextFormat.Printer.printToString
fn digest_to_text_format(digest: &Digest) -> String {
    let mut out = String::new();
    if !digest.hash.is_empty() {
        out.push_str("hash: \"");
        // Java: com/google/protobuf/TextFormatEscaper.java TextFormatEscaper.escapeBytes
        for b in digest.hash.bytes() {
            match b {
                0x07 => out.push_str("\\a"),
                0x08 => out.push_str("\\b"),
                0x0c => out.push_str("\\f"),
                b'\n' => out.push_str("\\n"),
                b'\r' => out.push_str("\\r"),
                b'\t' => out.push_str("\\t"),
                0x0b => out.push_str("\\v"),
                b'\\' => out.push_str("\\\\"),
                b'\'' => out.push_str("\\'"),
                b'"' => out.push_str("\\\""),
                0x20..=0x7e => out.push(b as char),
                _ => {
                    out.push('\\');
                    out.push((b'0' + ((b >> 6) & 3)) as char);
                    out.push((b'0' + ((b >> 3) & 7)) as char);
                    out.push((b'0' + (b & 7)) as char);
                }
            }
        }
        out.push_str("\"\n");
    }
    if digest.size_bytes != 0 {
        out.push_str(&format!("size_bytes: {}\n", digest.size_bytes));
    }
    out
}

/// The blob of a `VirtualActionInput`.
///
/// PORT: Java avoids materializing `VirtualActionInput.getBytes()` during the upload by writing
/// the input into a `PipedOutputStream` on a virtual thread and handing the upload the
/// `PipedInputStream`, which bounds the memory of an upload to the pipe's buffer. The port has
/// no borrowed-data pipe thread: `get` materializes the bytes of the input, for the lifetime of
/// the returned stream only.
// Java: com/google/devtools/build/lib/remote/RemoteExecutionCache.java:246 RemoteExecutionCache.VirtualActionInputBlob
struct VirtualActionInputBlob<'a> {
    virtual_action_input: &'a dyn VirtualActionInput,
}

impl Blob for VirtualActionInputBlob<'_> {
    // Java: com/google/devtools/build/lib/remote/RemoteExecutionCache.java:253 RemoteExecutionCache.VirtualActionInputBlob.get
    fn get(&self) -> io::Result<Box<dyn Read + Send + '_>> {
        Ok(Box::new(io::Cursor::new(
            self.virtual_action_input.get_bytes()?,
        )))
    }
}

impl RemoteExecutionCache {
    /// `new RemoteExecutionCache(remoteCacheClient, diskCacheClient, symlinkTemplate, digestUtil,
    /// chunkingFunction)`.
    // Java: com/google/devtools/build/lib/remote/RemoteExecutionCache.java:129 RemoteExecutionCache.<init>
    pub fn new(
        // `checkNotNull(remoteCacheClient)`: not an `Option`.
        remote_cache_client: Arc<dyn RemoteCacheClient>,
        disk_cache_client: Option<Arc<DiskCacheClient>>,
        symlink_template: Option<String>,
        digest_util: DigestUtil,
        chunking_function: Option<ChunkingFunctionValue>,
    ) -> RemoteExecutionCache {
        RemoteExecutionCache {
            combined_cache: Arc::new(CombinedCache::new(
                Some(remote_cache_client),
                disk_cache_client,
                symlink_template,
                digest_util,
                chunking_function,
            )),
            find_missing_cache: AsyncTaskCache::create(),
            remote_path_checker: RwLock::new(Arc::new(DefaultRemotePathChecker)),
        }
    }

    /// The `CombinedCache` this cache is (Java: the superclass part of the object).
    pub fn combined_cache(&self) -> &Arc<CombinedCache> {
        &self.combined_cache
    }

    /// `remoteCacheClient`, which is never null in a `RemoteExecutionCache`.
    pub fn remote_cache_client(&self) -> &Arc<dyn RemoteCacheClient> {
        match &self.combined_cache.remote_cache_client {
            Some(remote_cache_client) => remote_cache_client,
            None => panic!("NullPointerException: remoteCacheClient"),
        }
    }

    /// `setRemotePathChecker(remotePathChecker)` (`@VisibleForTesting`).
    // Java: com/google/devtools/build/lib/remote/RemoteExecutionCache.java:144 RemoteExecutionCache.setRemotePathChecker
    pub fn set_remote_path_checker(&self, remote_path_checker: Arc<dyn RemotePathChecker>) {
        *self.remote_path_checker.write() = remote_path_checker;
    }

    /// Ensures that the tree structure of the inputs, the input files themselves, and the
    /// command are available in the remote cache, such that the tree can be reassembled and
    /// executed on another machine given the root digest.
    ///
    /// The cache may check whether files or parts of the tree structure are already present, and
    /// do not need to be uploaded again.
    ///
    /// Note that this method is only required for remote execution, not for caching itself.
    /// However, remote execution uses a cache to store input files, and that may be a separate
    /// end-point from the executor itself, so the functionality lives here.
    ///
    /// PORT: `additionalInputs` is Java's `Map<Digest, Message>` with every message already
    /// serialized (`message.toByteString()`, that is `JavaMessage::to_byte_array`); Java
    /// serializes a message when it uploads it. The order of the map only decides the order of
    /// the digests in the `FindMissingBlobs` request.
    // Java: com/google/devtools/build/lib/remote/RemoteExecutionCache.java:160 RemoteExecutionCache.ensureInputsPresent
    pub fn ensure_inputs_present_with_additional_inputs(
        &self,
        context: &RemoteActionExecutionContext<'_>,
        merkle_tree: &MerkleTreeUploadable,
        additional_inputs: &IndexMap<Digest, bytes::Bytes>,
        force: bool,
        remote_path_resolver: Option<&dyn RemotePathResolver>,
    ) -> io::Result<()> {
        // `mergeBulkTransfer(uploads).blockingAwait()`
        get_from_future(self.ensure_inputs_present_with_additional_inputs_async(
            context,
            merkle_tree,
            additional_inputs,
            force,
            remote_path_resolver,
        ))
    }

    /// The flow that [`RemoteExecutionCache::ensure_inputs_present_with_additional_inputs`]
    /// blocks on. Dropping the returned future is Java's interruption of the blocked thread: the
    /// upload tasks of the call are disposed.
    // Java: com/google/devtools/build/lib/remote/RemoteExecutionCache.java:160 RemoteExecutionCache.ensureInputsPresent
    pub async fn ensure_inputs_present_with_additional_inputs_async(
        &self,
        context: &RemoteActionExecutionContext<'_>,
        merkle_tree: &MerkleTreeUploadable,
        additional_inputs: &IndexMap<Digest, bytes::Bytes>,
        force: bool,
        remote_path_resolver: Option<&dyn RemotePathResolver>,
    ) -> io::Result<()> {
        // Java: com/google/devtools/build/lib/remote/RemoteExecutionCache.java:321 RemoteExecutionCache.createUploadTasks
        let all_digests: Vec<Digest> = merkle_tree
            .all_digests()
            .into_iter()
            .chain(additional_inputs.keys().cloned())
            .collect();

        // The upload tasks that got a continuation and have not been answered yet.
        let continuations: Mutex<Vec<Continuation>> = Mutex::new(Vec::new());
        // The upload tasks that have not completed, and the completions of those that have.
        let mut upload_tasks: FuturesUnordered<BoxFuture<'_, (usize, io::Result<()>)>> =
            FuturesUnordered::new();
        let mut completions: Vec<(usize, io::Result<()>)> = Vec::with_capacity(all_digests.len());

        // PORT: skipped, profiling ("collect digests").
        for (index, digest) in all_digests.iter().enumerate() {
            let mut upload_task = self.maybe_create_upload_task(
                index,
                digest,
                context,
                merkle_tree,
                additional_inputs,
                force,
                remote_path_resolver,
                &continuations,
            );
            // Java subscribes the task to `findMissingCache` while it creates it, which is when
            // the cache decides whether the task gets a continuation.
            match futures::poll!(upload_task.as_mut()) {
                Poll::Ready(completion) => completions.push(completion),
                Poll::Pending => upload_tasks.push(upload_task),
            }
        }

        loop {
            // Java: com/google/devtools/build/lib/remote/RemoteExecutionCache.java:417 RemoteExecutionCache.findMissingBlobs
            // PORT: skipped, profiling ("findMissingDigests").
            //
            // PORT: Java sends one query, for the tasks that have a continuation once all tasks
            // are created, and keeps that query running when the call is disposed (the
            // `AsyncSubject`), because the tasks of other calls may wait for its answer. Here a
            // disposed call takes its query with it; `AsyncTaskCache` then lets a waiting task of
            // another call run the cache's task itself, so that task gets its continuation late.
            // Hence the loop: continuations that show up after the first query are answered by a
            // further query. Without a disposed call there is exactly one query, as in Java.
            let tasks_with_continuation: Vec<Continuation> =
                std::mem::take(&mut *continuations.lock());
            if !tasks_with_continuation.is_empty() {
                let digests_to_query: Vec<Digest> = tasks_with_continuation
                    .iter()
                    .map(|upload_task| upload_task.digest.clone())
                    .collect();
                let mut find_missing_digests = self
                    .remote_cache_client()
                    .find_missing_digests(context, &digests_to_query);
                let missing_digests = futures::future::poll_fn(|cx| {
                    // The tasks that did not need the answer (`findMissingCache` had it) are
                    // uploading already; keep them moving.
                    while let Poll::Ready(Some(completion)) = upload_tasks.poll_next_unpin(cx) {
                        completions.push(completion);
                    }
                    find_missing_digests.as_mut().poll(cx)
                })
                .await?;
                for upload_task in tasks_with_continuation {
                    // `uploadTask.continuation.onSuccess(missingDigests.contains(uploadTask.digest))`;
                    // the receiver is gone when the task was disposed.
                    let _ = upload_task
                        .continuation
                        .send(missing_digests.contains(&upload_task.digest));
                }
            }

            // Java: com/google/devtools/build/lib/remote/RemoteExecutionCache.java:455 RemoteExecutionCache.waitForUploadTasks
            // PORT: skipped, profiling ("upload").
            let step = futures::future::poll_fn(|cx| match upload_tasks.poll_next_unpin(cx) {
                Poll::Ready(Some((index, completion))) => {
                    Poll::Ready(Step::Completed(index, completion))
                }
                Poll::Ready(None) => Poll::Ready(Step::Done),
                Poll::Pending => {
                    if continuations.lock().is_empty() {
                        Poll::Pending
                    } else {
                        Poll::Ready(Step::GotContinuation)
                    }
                }
            })
            .await;
            match step {
                Step::Completed(index, completion) => completions.push((index, completion)),
                Step::GotContinuation => {}
                Step::Done => break,
            }
        }
        // The upload tasks are disposed here (Java: the disposer of `Flowable.using`); all of
        // them have completed. On the early returns above, and when this future is dropped,
        // dropping `upload_tasks` disposes the ones that are still running.
        drop(upload_tasks);

        // Java: com/google/devtools/build/lib/remote/util/RxUtils.java:133 RxUtils.mergeBulkTransfer
        //
        // PORT: Java collects the results in the order in which the uploads complete; the port
        // takes them in the order of the digests, so that the suppressed exceptions of the
        // `BulkTransferException` have a stable order.
        completions.sort_by_key(|(index, _)| *index);
        let mut bulk_transfer_exception: Option<BulkTransferException> = None;
        let mut interrupted = false;
        for (_, completion) in completions {
            // Java: com/google/devtools/build/lib/remote/util/RxUtils.java:82 RxUtils.toTransferResult
            match completion {
                Ok(()) => {}
                // A transfer is only ever canceled as part of tearing down the operation it
                // belongs to, so this is reported as an interruption rather than propagated
                // downstream.
                Err(error) if errors::is_interrupted(&error) => interrupted = true,
                Err(error) => bulk_transfer_exception
                    .get_or_insert_with(BulkTransferException::new)
                    .add(error),
            }
        }

        if interrupted {
            return Err(errors::interrupted());
        }

        if let Some(bulk_transfer_exception) = bulk_transfer_exception {
            return Err(bulk_transfer_exception.into());
        }

        Ok(())
    }

    /// Creates the upload task of `digest`. Its output is `UploadTask.completion` (with the
    /// index of the task).
    // Java: com/google/devtools/build/lib/remote/RemoteExecutionCache.java:348 RemoteExecutionCache.maybeCreateUploadTask
    fn maybe_create_upload_task<'a>(
        &'a self,
        index: usize,
        digest: &'a Digest,
        context: &'a RemoteActionExecutionContext<'a>,
        merkle_tree: &'a MerkleTreeUploadable,
        additional_inputs: &'a IndexMap<Digest, bytes::Bytes>,
        force: bool,
        remote_path_resolver: Option<&'a dyn RemotePathResolver>,
        continuations: &'a Mutex<Vec<Continuation>>,
    ) -> BoxFuture<'a, (usize, io::Result<()>)> {
        Box::pin(async move {
            let upload = async {
                let should_upload = self
                    .find_missing_cache
                    .execute(
                        digest.clone(),
                        // The task of the cache: runs only if the cache has no answer and no
                        // other upload task is supplying one (Java: `onAlreadyRunning` and
                        // `onAlreadyFinished` are the other two cases).
                        async {
                            let (continuation, should_upload) = oneshot::channel();
                            // `uploadTask.continuation = continuation;`
                            continuations.lock().push(Continuation {
                                digest: digest.clone(),
                                continuation,
                            });
                            should_upload.await.map_err(|_| cancellation_exception())
                        },
                        force,
                    )
                    .await?;
                if !should_upload {
                    return Ok(());
                }
                match self
                    .upload_blob_of_merkle_tree(
                        context,
                        digest,
                        merkle_tree,
                        additional_inputs,
                        remote_path_resolver,
                        force,
                    )
                    .await
                {
                    // On success, the digest is now present remotely: replace the cached
                    // "missing" answer with "present" so late callers (or the next
                    // ensureInputsPresent invocation) skip both findMissingDigests and
                    // the upload path. On failure, invalidate so a subsequent caller
                    // (e.g., after action rewinding) re-queries the remote.
                    Ok(()) => {
                        self.find_missing_cache.put(digest.clone(), false);
                        Ok(())
                    }
                    Err(t) => {
                        self.find_missing_cache.invalidate(digest);
                        Err(t)
                    }
                }
            };
            (index, upload.await)
        })
    }

    /// `uploadBlob(context, digest, merkleTree, additionalInputs, remotePathResolver, force)`.
    // Java: com/google/devtools/build/lib/remote/RemoteExecutionCache.java:290 RemoteExecutionCache.uploadBlob
    async fn upload_blob_of_merkle_tree(
        &self,
        context: &RemoteActionExecutionContext<'_>,
        digest: &Digest,
        merkle_tree: &MerkleTreeUploadable,
        additional_inputs: &IndexMap<Digest, bytes::Bytes>,
        remote_path_resolver: Option<&dyn RemotePathResolver>,
        force: bool,
    ) -> io::Result<()> {
        let upload = merkle_tree.upload(self, context, remote_path_resolver, digest, force);
        if let Some(upload) = upload {
            return upload.await;
        }

        let message = additional_inputs.get(digest);
        if let Some(message) = message {
            return self
                .remote_cache_client()
                .upload_bytes(context, digest, message.clone(), force)
                .await;
        }

        Err(errors::io_exception(format!(
            "findMissingDigests returned a missing digest that has not been requested: {}",
            digest_to_text_format(digest)
        )))
    }
}

#[async_trait]
impl MerkleTreeUploader for RemoteExecutionCache {
    // Java: com/google/devtools/build/lib/remote/RemoteExecutionCache.java:284 RemoteExecutionCache.uploadBlob
    async fn upload_blob(
        &self,
        context: &RemoteActionExecutionContext<'_>,
        digest: &Digest,
        data: bytes::Bytes,
        force: bool,
    ) -> io::Result<()> {
        self.remote_cache_client()
            .upload_bytes(context, digest, data, force)
            .await
    }

    // Java: com/google/devtools/build/lib/remote/RemoteExecutionCache.java:208 RemoteExecutionCache.uploadFile
    async fn upload_file(
        &self,
        context: &RemoteActionExecutionContext<'_>,
        remote_path_resolver: Option<&dyn RemotePathResolver>,
        digest: &Digest,
        path: &Path,
        force: bool,
    ) -> io::Result<()> {
        let remote_path_checker = self.remote_path_checker.read().clone();
        let is_available_locally = remote_path_checker
            .is_available_locally(context, path)
            .await?;
        if !is_available_locally {
            // If we get here, the remote input was determined to exist in the remote or disk
            // cache at some point before action execution, but reported to be missing when
            // querying the remote for missing action inputs; possibly because it was evicted in
            // the interim.
            if let Some(remote_path_resolver) = remote_path_resolver {
                return Err(CacheNotFoundException::with_exec_path(
                    digest.clone(),
                    remote_path_resolver.local_path_to_exec_path(path.as_fragment()),
                )
                .into());
            } else {
                // This path should only be taken for RemoteRepositoryRemoteExecutor, which has no
                // way to handle lost inputs.
                return Err(CacheNotFoundException::with_filename(
                    digest.clone(),
                    path.get_path_string(),
                )
                .into());
            }
        }
        self.remote_cache_client()
            .upload_file(context, digest, path, force)
            .await
    }

    // Java: com/google/devtools/build/lib/remote/RemoteExecutionCache.java:237 RemoteExecutionCache.uploadVirtualActionInput
    async fn upload_virtual_action_input(
        &self,
        context: &RemoteActionExecutionContext<'_>,
        digest: &Digest,
        virtual_action_input: &dyn VirtualActionInput,
        force: bool,
    ) -> io::Result<()> {
        let blob = VirtualActionInputBlob {
            virtual_action_input,
        };
        self.remote_cache_client()
            .upload_blob(context, digest, &blob, force)
            .await
    }

    // Java: com/google/devtools/build/lib/remote/RemoteExecutionCache.java:198 RemoteExecutionCache.ensureInputsPresent
    async fn ensure_inputs_present(
        &self,
        context: &RemoteActionExecutionContext<'_>,
        merkle_tree: &MerkleTreeUploadable,
        force: bool,
        remote_path_resolver: Option<&dyn RemotePathResolver>,
    ) -> io::Result<()> {
        let additional_inputs: IndexMap<Digest, bytes::Bytes> = IndexMap::new();
        self.ensure_inputs_present_with_additional_inputs_async(
            context,
            merkle_tree,
            &additional_inputs,
            force,
            remote_path_resolver,
        )
        .await
    }
}
