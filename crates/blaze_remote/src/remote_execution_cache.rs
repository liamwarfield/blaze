//! Port of `lib/remote/RemoteExecutionCache.java`.
//!
//! PORT: Java builds `ensureInputsPresent` from RxJava flows whose subscriptions live on other
//! threads. Per docs/remote.md the port runs everything on the calling task: one
//! `ensure_inputs_present` call owns the futures of its upload tasks and drives them. The pieces
//! of the Java machinery map as follows:
//!
//! - Work that Java shares between calls through an `AsyncTaskCache` (the answer of
//!   `findMissingCache` for a digest, and the upload of a digest in `casUploadCache`) is a
//!   [`SharedTask`]: a future that every interested call holds a clone of and drives. Like a
//!   Java `Execution` with its subscribers, it keeps running while one call is left and is
//!   cancelled (dropped) when the last one goes away. It borrows nothing: it has the detached
//!   copy of the context of the call that started it.
//! - `UploadTask.continuation` (the `SingleEmitter<Boolean>` a task gets when the cache makes it
//!   the one that has to supply the answer): the call that starts the `SharedTask` of a digest
//!   puts the digest into its `findMissingDigests` query, and the task takes its answer from the
//!   result of that query, which is itself shared by the tasks that wait for it.
//! - `UploadTask.completion` is the output of the task's future, and disposing a task
//!   (`uploadTask.disposable`) is dropping that future, which happens when the call returns or
//!   is itself dropped (Java: the calling thread is interrupted).

use std::collections::HashMap;
use std::io;
use std::ops::Deref;
use std::sync::Arc;
use std::task::Poll;

use async_trait::async_trait;
use blaze_exec::actions::VirtualActionInput;
use blaze_optdefs::misc::remote_options::ChunkingFunctionValue;
use blaze_vfs::Path;
use futures::FutureExt;
use futures::future::{BoxFuture, Shared, WeakShared};
use futures::stream::{FuturesUnordered, StreamExt};
use indexmap::{IndexMap, IndexSet};
use parking_lot::{Mutex, RwLock};

use crate::combined_cache::CombinedCache;
use crate::common::bulk_transfer_exception::BulkTransferException;
use crate::common::cache_not_found_exception::CacheNotFoundException;
use crate::common::errors;
use crate::common::remote_action_execution_context::RemoteActionExecutionContext;
use crate::common::remote_cache_client::RemoteCacheClient;
use crate::common::remote_path_resolver::RemotePathResolver;
use crate::disk::DiskCacheClient;
use crate::merkletree::{MerkleTreeUploadable, MerkleTreeUploader};
use crate::reapi::Digest;
use crate::runtime::get_from_future;
use crate::util::digest_util::DigestUtil;

/// The outcome of a [`SharedTask`]: every call that waits for the task gets the same error
/// (Java hands every observer the same exception object).
type SharedResult<T> = Result<T, Arc<io::Error>>;

/// A task shared between `ensure_inputs_present` calls, with the subscriber semantics of a Java
/// `AsyncTaskCache.Execution`: every call that needs the task holds a clone and drives it, the
/// task runs while at least one clone is left, and dropping the last clone cancels it.
///
/// PORT: `crate::util::async_task_cache::AsyncTaskCache` cannot be used for this. Its tasks are
/// driven by the first caller only, and when that caller goes away the task is cancelled and a
/// waiting caller starts it again; Java keeps the task running for the waiting caller.
// Java: com/google/devtools/build/lib/remote/util/AsyncTaskCache.java:142 AsyncTaskCache.Execution
type SharedTask<T> = Shared<BoxFuture<'static, SharedResult<T>>>;

/// A [`SharedTask`] as its cache knows it: the cache does not keep the task running.
type WeakSharedTask<T> = WeakShared<BoxFuture<'static, SharedResult<T>>>;

/// The `inProgress` map of a Java `AsyncTaskCache`: the tasks that are running, by digest. A
/// task whose calls have all gone away is no longer running.
// Java: com/google/devtools/build/lib/remote/util/AsyncTaskCache.java:83 AsyncTaskCache.inProgress
struct InProgress<T: Clone> {
    /// Identifies a task, so that a task that ends only removes its own entry.
    next_id: u64,
    tasks: HashMap<Digest, (u64, WeakSharedTask<T>)>,
}

impl<T: Clone> InProgress<T> {
    fn new() -> InProgress<T> {
        InProgress {
            next_id: 0,
            tasks: HashMap::new(),
        }
    }

    /// `inProgress.get(key)`.
    fn get(&self, key: &Digest) -> Option<SharedTask<T>> {
        self.tasks.get(key).and_then(|(_, task)| task.upgrade())
    }

    fn next_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    /// `inProgress.put(key, execution)`.
    fn put(&mut self, key: Digest, id: u64, task: &SharedTask<T>) {
        let task = task
            .downgrade()
            .expect("a task that was just created has not completed");
        self.tasks.insert(key, (id, task));
    }

    /// `inProgress.remove(key)`, by the task `id` when it ends.
    fn remove(&mut self, key: &Digest, id: u64) {
        if self.tasks.get(key).is_some_and(|(task, _)| *task == id) {
            self.tasks.remove(key);
        }
    }
}

/// `findMissingCache`: an `AsyncTaskCache<Digest, Boolean>` of "is this digest missing from the
/// remote cache".
// Java: com/google/devtools/build/lib/remote/RemoteExecutionCache.java:97 RemoteExecutionCache.findMissingCache
struct FindMissingCache {
    /// `AsyncTaskCache.finished`.
    finished: HashMap<Digest, bool>,
    /// `AsyncTaskCache.inProgress`.
    in_progress: InProgress<bool>,
}

/// What `findMissingCache.execute` gives the upload task of a digest.
enum ShouldUpload {
    /// The cache has the answer (`onAlreadyFinished`).
    Finished(bool),
    /// The answer is being found out, by this call (the task has `UploadTask.continuation`) or
    /// by another one (`onAlreadyRunning`).
    Running(SharedTask<bool>),
}

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
    find_missing_cache: Arc<Mutex<FindMissingCache>>,

    /// The uploads that the calls of this cache are waiting for.
    ///
    /// PORT: stands for the subscribers of `remoteCacheClient.casUploadCache`: an upload that
    /// several `ensure_inputs_present` calls wait for is one [`SharedTask`], which is what calls
    /// the client. (The client's own cache still deduplicates it against other uploads.)
    // Java: com/google/devtools/build/lib/remote/common/RemoteCacheClient.java:57 RemoteCacheClient.casUploadCache
    uploads: Arc<Mutex<InProgress<()>>>,

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
            find_missing_cache: Arc::new(Mutex::new(FindMissingCache {
                finished: HashMap::new(),
                in_progress: InProgress::new(),
            })),
            uploads: Arc::new(Mutex::new(InProgress::new())),
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

        // The digests of the upload tasks that have a continuation, and the `findMissingDigests`
        // query for them (`findMissingBlobs`), which starts when it is first awaited.
        //
        // Use AsyncSubject so that if downstream is disposed, the findMissingDigests call is not
        // cancelled (because it may be needed by other threads).
        //
        // PORT: the query is shared by the tasks of the digests it asks for, so it runs while a
        // call waits for one of them, also after this call is gone. (Java never cancels it.)
        // Java: com/google/devtools/build/lib/remote/RemoteExecutionCache.java:417 RemoteExecutionCache.findMissingBlobs
        let digests_to_query: Arc<Mutex<Vec<Digest>>> = Arc::new(Mutex::new(Vec::new()));
        let mut missing_digests: SharedTask<Arc<IndexSet<Digest>>> = {
            let remote_cache_client = self.remote_cache_client().clone();
            let context = context.detached();
            let digests_to_query = digests_to_query.clone();
            async move {
                let digests_to_query: Vec<Digest> = std::mem::take(&mut *digests_to_query.lock());
                remote_cache_client
                    .find_missing_digests(&context, &digests_to_query)
                    .await
                    .map(Arc::new)
                    .map_err(Arc::new)
            }
            .boxed()
            .shared()
        };

        // The upload tasks that have not completed, and the completions of those that have.
        let mut upload_tasks: FuturesUnordered<BoxFuture<'_, (usize, io::Result<()>)>> =
            FuturesUnordered::new();
        let mut completions: Vec<(usize, io::Result<()>)> = Vec::with_capacity(all_digests.len());

        // PORT: skipped, profiling ("collect digests").
        {
            // Java subscribes each task to `findMissingCache` (under the lock of the cache) while
            // it creates it, which is when the cache decides whether the task gets a
            // continuation.
            let mut find_missing_cache = self.find_missing_cache.lock();
            for (index, digest) in all_digests.iter().enumerate() {
                let should_upload =
                    self.execute_find_missing(&mut find_missing_cache, digest, force, || {
                        // `uploadTask.continuation = continuation;`
                        digests_to_query.lock().push(digest.clone());
                        missing_digests.clone()
                    });
                upload_tasks.push(self.maybe_create_upload_task(
                    index,
                    digest,
                    should_upload,
                    context,
                    merkle_tree,
                    additional_inputs,
                    force,
                    remote_path_resolver,
                ));
            }
        }

        // Java: com/google/devtools/build/lib/remote/RemoteExecutionCache.java:417 RemoteExecutionCache.findMissingBlobs
        // PORT: skipped, profiling ("findMissingDigests").
        let has_digests_to_query = !digests_to_query.lock().is_empty();
        if has_digests_to_query {
            futures::future::poll_fn(|cx| {
                // The tasks that do not need the answer (`findMissingCache` had it) are uploading
                // already; keep them moving.
                while let Poll::Ready(Some(completion)) = upload_tasks.poll_next_unpin(cx) {
                    completions.push(completion);
                }
                missing_digests.poll_unpin(cx)
            })
            .await
            .map_err(|e| errors::shared(&e))?;
        }
        drop(missing_digests);

        // Java: com/google/devtools/build/lib/remote/RemoteExecutionCache.java:455 RemoteExecutionCache.waitForUploadTasks
        // PORT: skipped, profiling ("upload").
        while let Some(completion) = upload_tasks.next().await {
            completions.push(completion);
        }
        // The upload tasks are disposed here (Java: the disposer of `Flowable.using`); all of
        // them have completed. On the early return above, and when this future is dropped,
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

    /// `findMissingCache.execute(digest, task, onAlreadyRunning, onAlreadyFinished, force)`, as
    /// far as it runs when the upload task subscribes: looks the digest up and, if the cache has
    /// no answer and no task is finding it out, starts one. `continuation` is called in that
    /// case and gives the query whose result has the answer.
    // Java: com/google/devtools/build/lib/remote/util/AsyncTaskCache.java:307 AsyncTaskCache.execute
    fn execute_find_missing(
        &self,
        find_missing_cache: &mut FindMissingCache,
        digest: &Digest,
        force: bool,
        continuation: impl FnOnce() -> SharedTask<Arc<IndexSet<Digest>>>,
    ) -> ShouldUpload {
        if !force {
            if let Some(cached) = find_missing_cache.finished.get(digest) {
                // onAlreadyFinished
                return ShouldUpload::Finished(*cached);
            }
        } else {
            find_missing_cache.finished.remove(digest);
        }

        if let Some(execution) = find_missing_cache.in_progress.get(digest) {
            // onAlreadyRunning
            return ShouldUpload::Running(execution);
        }

        let id = find_missing_cache.in_progress.next_id();
        let execution: SharedTask<bool> = {
            let missing_digests = continuation();
            let find_missing_cache = self.find_missing_cache.clone();
            let digest = digest.clone();
            async move {
                // `uploadTask.continuation.onSuccess(missingDigests.contains(uploadTask.digest))`
                let should_upload = missing_digests
                    .await
                    .map(|missing_digests| missing_digests.contains(&digest));
                // Java: com/google/devtools/build/lib/remote/util/AsyncTaskCache.java:197 AsyncTaskCache.Execution.onSuccess
                // Java: com/google/devtools/build/lib/remote/util/AsyncTaskCache.java:217 AsyncTaskCache.Execution.onError
                let mut find_missing_cache = find_missing_cache.lock();
                find_missing_cache.in_progress.remove(&digest, id);
                if let Ok(should_upload) = &should_upload {
                    find_missing_cache.finished.insert(digest, *should_upload);
                }
                should_upload
            }
            .boxed()
            .shared()
        };
        find_missing_cache
            .in_progress
            .put(digest.clone(), id, &execution);
        ShouldUpload::Running(execution)
    }

    /// Creates the upload task of `digest`. Its output is `UploadTask.completion` (with the
    /// index of the task).
    // Java: com/google/devtools/build/lib/remote/RemoteExecutionCache.java:348 RemoteExecutionCache.maybeCreateUploadTask
    fn maybe_create_upload_task<'a>(
        &'a self,
        index: usize,
        digest: &'a Digest,
        should_upload: ShouldUpload,
        context: &'a RemoteActionExecutionContext<'a>,
        merkle_tree: &'a MerkleTreeUploadable,
        additional_inputs: &'a IndexMap<Digest, bytes::Bytes>,
        force: bool,
        remote_path_resolver: Option<&'a dyn RemotePathResolver>,
    ) -> BoxFuture<'a, (usize, io::Result<()>)> {
        Box::pin(async move {
            let upload = async {
                let should_upload = match should_upload {
                    ShouldUpload::Finished(should_upload) => should_upload,
                    ShouldUpload::Running(execution) => {
                        execution.await.map_err(|e| errors::shared(&e))?
                    }
                };
                if !should_upload {
                    return Ok::<(), io::Error>(());
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
                        // `findMissingCache.put(digest, false)`
                        self.find_missing_cache
                            .lock()
                            .finished
                            .insert(digest.clone(), false);
                        Ok(())
                    }
                    Err(t) => {
                        // `findMissingCache.invalidate(digest)`
                        self.find_missing_cache.lock().finished.remove(digest);
                        Err(t)
                    }
                }
            };
            (index, upload.await)
        })
    }

    /// Runs an upload of `digest` as a task shared with the other calls that wait for the
    /// upload of that digest (see the `uploads` field). `start` makes the upload when none is
    /// running.
    // Java: com/google/devtools/build/lib/remote/common/RemoteCacheClient.java:225 RemoteCacheClient.dedupUpload
    async fn shared_upload(
        &self,
        context: &RemoteActionExecutionContext<'_>,
        digest: &Digest,
        start: impl FnOnce(
            Arc<dyn RemoteCacheClient>,
            RemoteActionExecutionContext<'static>,
            Digest,
        ) -> BoxFuture<'static, io::Result<()>>,
    ) -> io::Result<()> {
        let running = self.uploads.lock().get(digest);
        let upload: SharedTask<()> = match running {
            Some(upload) => upload,
            None => {
                let upload = start(
                    self.remote_cache_client().clone(),
                    context.detached(),
                    digest.clone(),
                );
                let mut uploads = self.uploads.lock();
                // Another call may have started the upload since the first look.
                match uploads.get(digest) {
                    Some(upload) => upload,
                    None => {
                        let id = uploads.next_id();
                        let upload: SharedTask<()> = {
                            let uploads = self.uploads.clone();
                            let digest = digest.clone();
                            async move {
                                let result = upload.await.map_err(Arc::new);
                                uploads.lock().remove(&digest, id);
                                result
                            }
                            .boxed()
                            .shared()
                        };
                        uploads.put(digest.clone(), id, &upload);
                        upload
                    }
                }
            }
        };
        upload.await.map_err(|e| errors::shared(&e))
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
            // `remoteCacheClient.uploadBlob(context, digest, message.toByteString(), force)`
            return MerkleTreeUploader::upload_blob(self, context, digest, message.clone(), force)
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
        self.shared_upload(context, digest, |remote_cache_client, context, digest| {
            async move {
                remote_cache_client
                    .upload_bytes(&context, &digest, data, force)
                    .await
            }
            .boxed()
        })
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
        let path = path.clone();
        self.shared_upload(context, digest, |remote_cache_client, context, digest| {
            async move {
                remote_cache_client
                    .upload_file(&context, &digest, &path, force)
                    .await
            }
            .boxed()
        })
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
        // PORT: Java uploads a `VirtualActionInputBlob`, which avoids materializing
        // `VirtualActionInput.getBytes()` during the upload: it writes the input into a
        // `PipedOutputStream` on a virtual thread and hands the upload the `PipedInputStream`, so
        // that an upload needs no more memory than the buffer of the pipe. The upload here is a
        // task shared between calls, which cannot borrow the input of one of them, so the call
        // that starts it materializes the bytes of the input, for the time of the upload.
        // Java: com/google/devtools/build/lib/remote/RemoteExecutionCache.java:246 RemoteExecutionCache.VirtualActionInputBlob
        self.shared_upload(context, digest, |remote_cache_client, context, digest| {
            let data = virtual_action_input.get_bytes();
            async move {
                remote_cache_client
                    .upload_bytes(&context, &digest, bytes::Bytes::from(data?), force)
                    .await
            }
            .boxed()
        })
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
