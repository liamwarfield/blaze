//! Port of `lib/remote/RemoteExecutionService.java`: the core half.
//!
//! The download half (`downloadFile` ... `shouldDownload`, `prettyPrint`: lines 860 to 1725 of
//! the Java file, with `ActionResultMetadata`, `downloadOutputs`, `LocalExecution`,
//! `waitForAndReuseOutputs`) is a second `impl RemoteExecutionService` block in
//! `remote_execution_service_download.rs`.
//!
//! PORT: `OutputService` and `wasRewound` (action rewinding) are not ported. The struct has no
//! fields for them and the code behaves as with no output service and for an action that was not
//! rewound; each place says so.

use std::cmp::Ordering;
use std::collections::{BTreeSet, HashSet};
use std::io;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering as AtomicOrdering};
use std::sync::{Arc, OnceLock};

use blaze_actions::actions::command_lines::ParamFileActionInput;
use blaze_events::{Event, ExtendedEventHandler};
use blaze_exec::actions::action_adapters::as_blaze_artifact;
use blaze_exec::actions::{
    ActionInput, ActionInputRef, ExecException, FileArtifactValue, FileContentsProxy, PlatformInfo,
    Spawn, SpawnResult, Status as SpawnResultStatus, VirtualActionInput, execution_requirements,
    spawns,
};
use blaze_exec::exec::SpawnExecutionContext;
use blaze_exec::exec::action_execution_context::SpawnCacheError;
use blaze_optdefs::command::execution_options::ExecutionOptions;
use blaze_optdefs::misc::remote_options::{ConcurrentChangesCheckLevel, RemoteOptions};
use blaze_protos::JavaMessage;
use blaze_protos::tools::protos::{CacheSalt, cache_salt};
use blaze_vfs::os_path_policy::{HOST_POLICY, OsPathPolicy};
use blaze_vfs::{Path, PathFragment, Symlinks};
use indexmap::{IndexMap, IndexSet};
use parking_lot::{Condvar, Mutex};

use crate::api_version::ApiVersion;
use crate::client_api_version::{ClientApiVersion, ServerSupportedStatus};
use crate::combined_cache::{CachedActionResult, CombinedCache};
use crate::common::bulk_transfer_exception::BulkTransferException;
use crate::common::errors::{self, io_exception};
use crate::common::remote_action_execution_context::{CachePolicy, RemoteActionExecutionContext};
use crate::common::remote_cache_client::ActionKey;
use crate::common::remote_execution_capabilities_exception::RemoteExecutionCapabilitiesException;
use crate::common::remote_execution_client::{OperationObserver, RemoteExecutionClient};
use crate::common::remote_path_resolver::{self, RemotePathResolver};
use crate::merkletree::{
    BlobPolicy, MerkleTree, MerkleTreeComputer, MerkleTreeUploadable, MerkleTreeUploader,
};
use crate::platform_utils;
use crate::reapi::command::EnvironmentVariable;
use crate::reapi::{
    Action, ActionResult, Command, Digest, ExecuteRequest, ExecuteResponse, ExecutedActionMetadata,
    OutputDirectory, OutputFile, OutputSymlink, Platform, RequestMetadata,
};
use crate::remote_action::RemoteAction;
use crate::remote_execution_cache::RemoteExecutionCache;
use crate::remote_execution_service_download::ActionResultMetadata;
use crate::remote_output_checker::RemoteOutputChecker;
use crate::runtime::get_from_future;
use crate::scrubber::{Scrubber, SpawnScrubber};
use crate::upload_manifest::UploadManifest;
use crate::util::digest_util::DigestUtil;
use crate::util::temp_path_generator::TempPathGenerator;
use crate::util::tracing_metadata_utils;
use crate::util::utils::{
    self, create_exec_exception_from_remote_execution_capabilities_exception,
    get_in_memory_output_path, should_upload_local_results_to_remote_cache,
};

/// `StringEncoding.internalToUnicode(s)` of the internal string that `s` stands for.
///
/// Seam (decisions.md 2026-11-18 "Strings"): while
/// `blaze_util::string_encoding::STRINGS_ARE_INTERNAL` is false Blaze's strings are Unicode
/// already and this is the identity; once it is true it is `internal_to_unicode`.
// PORT: StringEncoding identity (until the strings switch).
// Java: com/google/devtools/build/lib/util/StringEncoding.java:126 StringEncoding.internalToUnicode
fn internal_to_unicode(s: &str) -> String {
    if blaze_util::string_encoding::strings_are_internal() {
        blaze_util::string_encoding::internal_to_unicode(s).into_owned()
    } else {
        s.to_string()
    }
}

/// `StringEncoding.unicodeToInternal(s)`, giving the string that stands for the resulting
/// internal string (the same seam as [`internal_to_unicode`]).
// PORT: StringEncoding identity (until the strings switch).
// Java: com/google/devtools/build/lib/util/StringEncoding.java:137 StringEncoding.unicodeToInternal
fn unicode_to_internal(s: &str) -> String {
    if blaze_util::string_encoding::strings_are_internal() {
        blaze_util::string_encoding::unicode_to_internal(s).into_owned()
    } else {
        s.to_string()
    }
}

/// `PROTO_STRING_COMPARATOR = comparing(StringEncoding::unicodeToInternal)`, for Unicode strings.
///
/// The Java comparator maps each Unicode string to its internal form (a Latin-1 string holding
/// the UTF-8 bytes) and compares those with `String.compareTo`. The UTF-16 code units of an
/// internal string are its bytes, so this is the order of the UTF-8 encodings, which is the
/// order of `str`.
// Java: com/google/devtools/build/lib/remote/RemoteExecutionService.java:175 RemoteExecutionService.PROTO_STRING_COMPARATOR
fn proto_string_comparator(a: &str, b: &str) -> Ordering {
    a.as_bytes().cmp(b.as_bytes())
}

/// A counting semaphore that hands its permits out in the order of the requests
/// (`new Semaphore(permits, /* fair= */ true)`).
// Java: java/util/concurrent/Semaphore.java Semaphore
pub(crate) struct Semaphore {
    state: Mutex<SemaphoreState>,
    changed: Condvar,
}

struct SemaphoreState {
    permits: usize,
    /// The place in the queue the next `acquire` gets, and the place that is served next.
    next_ticket: u64,
    now_serving: u64,
}

impl Semaphore {
    fn new(permits: usize) -> Semaphore {
        Semaphore {
            state: Mutex::new(SemaphoreState {
                permits,
                next_ticket: 0,
                now_serving: 0,
            }),
            changed: Condvar::new(),
        }
    }

    /// `acquire()`.
    ///
    /// PORT: not interruptible (the remote package has no thread interruption, see
    /// `crate::runtime`).
    fn acquire(&self) {
        let mut state = self.state.lock();
        let ticket = state.next_ticket;
        state.next_ticket += 1;
        while !(state.now_serving == ticket && state.permits > 0) {
            self.changed.wait(&mut state);
        }
        state.permits -= 1;
        state.now_serving += 1;
        drop(state);
        self.changed.notify_all();
    }

    /// `release()`.
    fn release(&self) {
        self.state.lock().permits += 1;
        self.changed.notify_all();
    }
}

/// `finally { maybeReleaseRemoteActionBuildingSemaphore(); }`.
struct RemoteActionBuildingSemaphoreRelease<'s>(&'s RemoteExecutionService);

impl Drop for RemoteActionBuildingSemaphoreRelease<'_> {
    fn drop(&mut self) {
        self.0.maybe_release_remote_action_building_semaphore();
    }
}

/// `backgroundTaskExecutor`: an executor that runs each task on a thread of its own
/// (`Executors.newThreadPerTaskExecutor(Thread.ofVirtual().name("remote-execution-bg-", 0)
/// .factory())`).
///
/// PORT: the tasks are [`OutputUpload`]s, the only thing Java submits. They run on platform
/// threads: `UploadManifest::upload` is a blocking method, which must not run on a thread of the
/// remote executor runtime (`crate::runtime::get_from_future`). There is no thread interruption,
/// so `shutdownNow` cannot stop a running task; see [`RemoteExecutionService::shutdown`].
// Java: com/google/devtools/build/lib/remote/RemoteExecutionService.java:201 RemoteExecutionService.backgroundTaskExecutor
pub(crate) struct BackgroundTaskExecutor {
    state: Mutex<BackgroundTaskExecutorState>,
    /// The number of the next thread.
    next_thread: AtomicU64,
}

struct BackgroundTaskExecutorState {
    /// `isShutdown()`.
    shutdown: bool,
    /// The threads of the tasks that may still be running.
    threads: Vec<std::thread::JoinHandle<()>>,
}

impl BackgroundTaskExecutor {
    fn new() -> BackgroundTaskExecutor {
        BackgroundTaskExecutor {
            state: Mutex::new(BackgroundTaskExecutorState {
                shutdown: false,
                threads: Vec::new(),
            }),
            next_thread: AtomicU64::new(0),
        }
    }

    /// `isShutdown()`.
    fn is_shutdown(&self) -> bool {
        self.state.lock().shutdown
    }

    /// `execute(task::run)`. Gives the task back when the executor has been shut down (Java:
    /// throws `RejectedExecutionException`).
    fn execute(&self, task: OutputUpload) -> Result<(), OutputUpload> {
        let mut state = self.state.lock();
        if state.shutdown {
            return Err(task);
        }
        // Forget the threads that have ended, so that the list does not grow with the build.
        state.threads.retain(|thread| !thread.is_finished());
        let name = format!(
            "remote-execution-bg-{}",
            self.next_thread.fetch_add(1, AtomicOrdering::SeqCst)
        );
        let thread = std::thread::Builder::new()
            .name(name)
            .spawn(move || task.run())
            // Java: an `OutOfMemoryError` ("unable to create native thread").
            .unwrap_or_else(|e| panic!("OutOfMemoryError: unable to create native thread: {e}"));
        state.threads.push(thread);
        Ok(())
    }

    /// `shutdown()`: previously submitted tasks are executed, but no new tasks will be accepted.
    fn shutdown(&self) {
        self.state.lock().shutdown = true;
    }

    /// `shutdownNow()`.
    ///
    /// PORT: Java interrupts the running tasks. The port cannot (no thread interruption); the
    /// running uploads end on their own, and sooner once the caller has shut the cache down
    /// (`CombinedCache::shutdown_now` refuses new uploads).
    fn shutdown_now(&self) {
        self.shutdown();
    }

    /// `close()`: initiates an orderly shutdown and waits for all tasks to finish.
    fn close(&self) {
        let threads = {
            let mut state = self.state.lock();
            state.shutdown = true;
            std::mem::take(&mut state.threads)
        };
        for thread in threads {
            // A task that panicked has reported its panic on its own thread.
            let _ = thread.join();
        }
    }
}

/// The parts of the service that reporting an upload error needs. They are shared with the
/// background uploads, which outlive the borrow of the service that started them.
#[derive(Clone)]
struct UploadErrorReporter {
    reporter: Arc<dyn ExtendedEventHandler>,
    reported_errors: Arc<Mutex<HashSet<String>>>,
    build_interrupted: Arc<AtomicBool>,
    verbose_failures: bool,
}

impl UploadErrorReporter {
    // Java: com/google/devtools/build/lib/remote/RemoteExecutionService.java:1942 RemoteExecutionService.reportUploadError
    fn report_upload_error(&self, error: &SpawnCacheError) {
        if self.build_interrupted.load(AtomicOrdering::SeqCst) {
            // If build interrupted, ignores all the errors
            return;
        }

        let error_message = format!(
            "Remote Cache: {}",
            grpc_aware_error_message(error, self.verbose_failures)
        );

        report(
            &*self.reporter,
            &self.reported_errors,
            Event::warn(error_message),
        );
    }
}

/// `Utils.grpcAwareErrorMessage(error, verboseFailures)` for the two kinds of `Throwable` an
/// upload reports.
///
/// PORT: `utils::grpc_aware_error_message` takes an `io::Error` (an `IOException`); the branch
/// for the other throwables, `error.getMessage()` or else `error.getClass().getSimpleName()`, is
/// here for `ExecException`.
// Java: com/google/devtools/build/lib/remote/util/Utils.java:385 Utils.grpcAwareErrorMessage
fn grpc_aware_error_message(error: &SpawnCacheError, verbose_failures: bool) -> String {
    match error {
        SpawnCacheError::Io(error) => utils::grpc_aware_error_message(error, verbose_failures),
        SpawnCacheError::Exec(error) => {
            let error_message = match error {
                // `InterruptedException.getMessage()` is null.
                ExecException::Interrupted => String::new(),
                _ => error.get_message(),
            };
            if error_message.is_empty() {
                return match error {
                    ExecException::User(_) => "UserExecException",
                    ExecException::Environmental(_) => "EnvironmentalExecException",
                    ExecException::Spawn(_) => "SpawnExecException",
                    ExecException::Interrupted => "InterruptedException",
                }
                .to_string();
            }
            // PORT: no stack trace is appended on --verbose_failures (as in
            // `utils::grpc_aware_error_message`).
            error_message
        }
    }
}

/// `report(evt)`: reports an event once per message.
// Java: com/google/devtools/build/lib/remote/RemoteExecutionService.java:2158 RemoteExecutionService.report
fn report(
    reporter: &dyn ExtendedEventHandler,
    reported_errors: &Mutex<HashSet<String>>,
    evt: Event,
) {
    // synchronized (this)
    let mut reported_errors = reported_errors.lock();
    let message: String = evt.get_message().into_owned();
    if reported_errors.contains(&message) {
        return;
    }
    reported_errors.insert(message);
    reporter.handle(&evt);
}

/// A layer between spawn execution and remote execution exposing primitive operations for remote
/// cache and execution with spawn specific types.
// Java: com/google/devtools/build/lib/remote/RemoteExecutionService.java:174 RemoteExecutionService
pub struct RemoteExecutionService {
    pub(crate) reporter: Arc<dyn ExtendedEventHandler>,
    pub(crate) verbose_failures: bool,
    pub(crate) exec_root: Path,

    /// Do not use directly, instead use the per-spawn resolver created in
    /// [`RemoteExecutionService::build_remote_action`].
    pub(crate) base_remote_path_resolver: Arc<dyn RemotePathResolver>,

    pub(crate) build_request_id: String,
    pub(crate) command_id: String,
    pub(crate) digest_util: DigestUtil,
    pub(crate) merkle_tree_computer: MerkleTreeComputer,
    pub(crate) remote_options: Arc<RemoteOptions>,
    pub(crate) execution_options: Arc<ExecutionOptions>,
    /// `combinedCache`. When Java's `combinedCache` is a `RemoteExecutionCache`, this is the
    /// `CombinedCache` inside [`RemoteExecutionService::remote_execution_cache`].
    pub(crate) combined_cache: Option<Arc<CombinedCache>>,
    /// `(RemoteExecutionCache) combinedCache`: set if and only if
    /// `combinedCache instanceof RemoteExecutionCache`.
    pub(crate) remote_execution_cache: Option<Arc<RemoteExecutionCache>>,
    pub(crate) remote_executor: Option<Arc<dyn RemoteExecutionClient>>,
    pub(crate) temp_path_generator: Arc<TempPathGenerator>,
    pub(crate) capture_corrupted_outputs_dir: Option<Path>,
    // PORT: shared (`Arc`) with the background uploads, which report their errors through it.
    pub(crate) reported_errors: Arc<Mutex<HashSet<String>>>,

    pub(crate) background_task_executor: BackgroundTaskExecutor,

    pub(crate) shutdown: AtomicBool,
    // PORT: shared (`Arc`) with the background uploads, which read it when they fail.
    pub(crate) build_interrupted: Arc<AtomicBool>,

    pub(crate) remote_output_checker: Option<Arc<RemoteOutputChecker>>,
    // PORT: `outputService` is not ported (no output service).
    pub(crate) scrubber: Option<Arc<Scrubber>>,
    pub(crate) known_missing_cas_digests: Arc<Mutex<HashSet<Digest>>>,
    /// Integration seams (see `crate::hooks`).
    pub(crate) hooks: crate::hooks::RemoteHooks,
    // PORT: `wasRewound` is not ported (no action is ever rewound).
    /// `Boolean useOutputPaths`, initialized on first use.
    pub(crate) use_output_paths: OnceLock<bool>,

    /// Semaphore for limiting the concurrent number of Merkle tree input roots we compute and
    /// keep in memory.
    ///
    /// When --jobs is set to a high value to let the remote execution service runs many actions
    /// in parallel, there is no point in letting the local system compute Merkle trees of input
    /// roots with the same amount of parallelism. Not only does this make Bazel feel sluggish
    /// and slow to respond to being interrupted, it causes it to exhaust memory.
    ///
    /// As there is no point in letting Merkle tree input root computation use a higher
    /// concurrency than the number of CPUs in the system, use a semaphore to limit the
    /// concurrency of buildRemoteAction().
    // Java: com/google/devtools/build/lib/remote/RemoteExecutionService.java:408 RemoteExecutionService.remoteActionBuildingSemaphore
    pub(crate) remote_action_building_semaphore: Semaphore,
}

impl std::fmt::Debug for RemoteExecutionService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RemoteExecutionService")
            .field("exec_root", &self.exec_root)
            .field("build_request_id", &self.build_request_id)
            .field("command_id", &self.command_id)
            .finish_non_exhaustive()
    }
}

/// A simple value class combining a hash of the tool inputs (and their digests) as well as a set
/// of the relative paths of all tool inputs.
// Java: com/google/devtools/build/lib/remote/RemoteExecutionService.java:2176 RemoteExecutionService.ToolSignature
struct ToolSignature {
    key: String,
    tool_inputs: HashSet<PathFragment>,
}

impl ToolSignature {
    // Java: com/google/devtools/build/lib/remote/RemoteExecutionService.java:2180 RemoteExecutionService.ToolSignature.<init>
    #[allow(dead_code)]
    fn new(key: String, tool_inputs: HashSet<PathFragment>) -> ToolSignature {
        ToolSignature { key, tool_inputs }
    }
}

/// A value class representing the result of remotely executed [`RemoteAction`].
// Java: com/google/devtools/build/lib/remote/RemoteExecutionService.java:609 RemoteExecutionService.RemoteActionResult
pub struct RemoteActionResult {
    pub(crate) action_result: ActionResult,
    pub(crate) execute_response: Option<ExecuteResponse>,
    pub(crate) cache_name: Option<String>,
    /// `@Nullable ActionResultMetadata metadata`, parsed on first use.
    pub(crate) metadata: OnceLock<ActionResultMetadata>,
}

impl std::fmt::Debug for RemoteActionResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RemoteActionResult")
            .field("action_result", &self.action_result)
            .field("execute_response", &self.execute_response)
            .field("cache_name", &self.cache_name)
            .finish_non_exhaustive()
    }
}

// PORT: `hashCode` (`Objects.hash(actionResult, executeResponse)`) has no port: the generated
// messages are not `Hash`.
// Java: com/google/devtools/build/lib/remote/RemoteExecutionService.java:753 RemoteExecutionService.RemoteActionResult.equals
impl PartialEq for RemoteActionResult {
    fn eq(&self, that: &RemoteActionResult) -> bool {
        self.action_result == that.action_result && self.execute_response == that.execute_response
    }
}

impl RemoteActionResult {
    /// Creates a new [`RemoteActionResult`] instance from a cached result.
    // Java: com/google/devtools/build/lib/remote/RemoteExecutionService.java:616 RemoteExecutionService.RemoteActionResult.createFromCache
    pub fn create_from_cache(cached_action_result: Option<CachedActionResult>) -> Self {
        let Some(cached_action_result) = cached_action_result else {
            panic!("IllegalArgumentException: cachedActionResult is null");
        };
        RemoteActionResult::new(
            cached_action_result.action_result().clone(),
            None,
            Some(cached_action_result.cache_name().to_string()),
        )
    }

    /// Creates a new [`RemoteActionResult`] instance from a execute response.
    // Java: com/google/devtools/build/lib/remote/RemoteExecutionService.java:623 RemoteExecutionService.RemoteActionResult.createFromResponse
    pub fn create_from_response(response: ExecuteResponse) -> RemoteActionResult {
        blaze_util::check_argument!(response.result.is_some(), "response doesn't have result");
        RemoteActionResult::new(
            response.result.clone().unwrap_or_default(),
            Some(response),
            /* cacheName */ None,
        )
    }

    /// `new RemoteActionResult(actionResult, executeResponse, cacheName)`.
    // Java: com/google/devtools/build/lib/remote/RemoteExecutionService.java:628 RemoteExecutionService.RemoteActionResult.<init>
    pub fn new(
        action_result: ActionResult,
        execute_response: Option<ExecuteResponse>,
        cache_name: Option<String>,
    ) -> RemoteActionResult {
        RemoteActionResult {
            action_result,
            execute_response,
            cache_name,
            metadata: OnceLock::new(),
        }
    }

    /// Returns the exit code of remote executed action.
    // Java: com/google/devtools/build/lib/remote/RemoteExecutionService.java:638 RemoteExecutionService.RemoteActionResult.getExitCode
    pub fn get_exit_code(&self) -> i32 {
        self.action_result.exit_code
    }

    /// `getOutputFiles()`.
    // Java: com/google/devtools/build/lib/remote/RemoteExecutionService.java:642 RemoteExecutionService.RemoteActionResult.getOutputFiles
    pub fn get_output_files(&self) -> &[OutputFile] {
        &self.action_result.output_files
    }

    /// `getOutputFileSymlinks()`.
    // Java: com/google/devtools/build/lib/remote/RemoteExecutionService.java:646 RemoteExecutionService.RemoteActionResult.getOutputFileSymlinks
    pub fn get_output_file_symlinks(&self) -> &[OutputSymlink] {
        &self.action_result.output_file_symlinks
    }

    /// `getOutputDirectories()`.
    // Java: com/google/devtools/build/lib/remote/RemoteExecutionService.java:650 RemoteExecutionService.RemoteActionResult.getOutputDirectories
    pub fn get_output_directories(&self) -> &[OutputDirectory] {
        &self.action_result.output_directories
    }

    /// `getOrParseActionResultMetadata(combinedCache, digestUtil, context, remotePathResolver)`.
    // Java: com/google/devtools/build/lib/remote/RemoteExecutionService.java:654 RemoteExecutionService.RemoteActionResult.getOrParseActionResultMetadata
    pub fn get_or_parse_action_result_metadata(
        &self,
        combined_cache: &CombinedCache,
        digest_util: &DigestUtil,
        context: &RemoteActionExecutionContext<'_>,
        remote_path_resolver: &dyn RemotePathResolver,
    ) -> io::Result<&ActionResultMetadata> {
        if let Some(metadata) = self.metadata.get() {
            return Ok(metadata);
        }
        // PORT: skipped, profiling ("Remote.parseActionResultMetadata").
        let metadata = RemoteExecutionService::parse_action_result_metadata(
            combined_cache,
            digest_util,
            context,
            &self.action_result,
            remote_path_resolver,
        )?;
        Ok(self.metadata.get_or_init(|| metadata))
    }

    /// `getOutputDirectorySymlinks()`.
    // Java: com/google/devtools/build/lib/remote/RemoteExecutionService.java:670 RemoteExecutionService.RemoteActionResult.getOutputDirectorySymlinks
    pub fn get_output_directory_symlinks(&self) -> &[OutputSymlink] {
        &self.action_result.output_directory_symlinks
    }

    /// `getOutputSymlinks()`.
    // Java: com/google/devtools/build/lib/remote/RemoteExecutionService.java:674 RemoteExecutionService.RemoteActionResult.getOutputSymlinks
    pub fn get_output_symlinks(&self) -> &[OutputSymlink] {
        &self.action_result.output_symlinks
    }

    /// Returns the freeform informational message with details on the execution of the action
    /// that may be displayed to the user upon failure or when requested explicitly.
    // Java: com/google/devtools/build/lib/remote/RemoteExecutionService.java:682 RemoteExecutionService.RemoteActionResult.getMessage
    pub fn get_message(&self) -> &str {
        match &self.execute_response {
            Some(execute_response) => &execute_response.message,
            None => "",
        }
    }

    /// Returns the details of the execution that originally produced this result.
    // Java: com/google/devtools/build/lib/remote/RemoteExecutionService.java:687 RemoteExecutionService.RemoteActionResult.getExecutionMetadata
    pub fn get_execution_metadata(&self) -> ExecutedActionMetadata {
        // An unset message field reads as the default instance.
        self.action_result
            .execution_metadata
            .clone()
            .unwrap_or_default()
    }

    /// Returns whether the action is executed successfully.
    // Java: com/google/devtools/build/lib/remote/RemoteExecutionService.java:692 RemoteExecutionService.RemoteActionResult.success
    pub fn success(&self) -> bool {
        if let Some(execute_response) = &self.execute_response {
            let code = execute_response.status.as_ref().map_or(0, |s| s.code);
            if code != tonic::Code::Ok as i32 {
                return false;
            }
        }

        self.action_result.exit_code == 0
    }

    /// Returns an output that is mandatory for the given spawn but missing from this result, if
    /// any.
    // Java: com/google/devtools/build/lib/remote/RemoteExecutionService.java:705 RemoteExecutionService.RemoteActionResult.maybeGetMissingMandatoryOutput
    pub fn maybe_get_missing_mandatory_output(
        &self,
        action: &RemoteAction<'_>,
    ) -> Option<ActionInputRef> {
        let output_files = self.get_output_files().iter().map(|f| &f.path);
        let output_dir_paths = self.get_output_directories().iter().map(|d| &d.path);
        let output_symlink_paths = self
            .get_output_symlinks()
            .iter()
            .chain(self.get_output_file_symlinks())
            .chain(self.get_output_directory_symlinks())
            .map(|s| &s.path);
        let all_output_paths: IndexSet<String> = output_files
            .chain(output_dir_paths)
            .chain(output_symlink_paths)
            .map(|path| unicode_to_internal(path))
            .collect();
        // Check that all mandatory outputs are created.
        let spawn = action.get_spawn();
        let remote_path_resolver = action.get_remote_path_resolver();
        spawn
            .get_output_files()
            .iter()
            .filter(|output| spawn.is_mandatory_output(output))
            .find(|output| {
                !all_output_paths
                    .contains(&remote_path_resolver.local_action_input_to_output_path(&***output))
            })
            .cloned()
    }

    /// Returns `true` if this result is from a cache.
    // Java: com/google/devtools/build/lib/remote/RemoteExecutionService.java:729 RemoteExecutionService.RemoteActionResult.cacheHit
    pub fn cache_hit(&self) -> bool {
        let Some(execute_response) = &self.execute_response else {
            return true;
        };

        execute_response.cached_result
    }

    /// Returns cache name (disk/remote) when `cache_hit()` or `None` when not
    // Java: com/google/devtools/build/lib/remote/RemoteExecutionService.java:739 RemoteExecutionService.RemoteActionResult.cacheName
    pub fn cache_name(&self) -> Option<&str> {
        self.cache_name.as_deref()
    }

    /// Returns the underlying `ExecuteResponse` or `None` if this result is from a cache.
    // Java: com/google/devtools/build/lib/remote/RemoteExecutionService.java:748 RemoteExecutionService.RemoteActionResult.getResponse
    pub fn get_response(&self) -> Option<&ExecuteResponse> {
        self.execute_response.as_ref()
    }
}

/// A value classes representing downloaded server logs.
// Java: com/google/devtools/build/lib/remote/RemoteExecutionService.java:2057 RemoteExecutionService.ServerLogs
#[derive(Debug, Clone)]
pub struct ServerLogs {
    pub log_count: i32,
    pub directory: Path,
    pub last_log_path: Option<Path>,
}

/// The background upload of an action's outputs: the `CancellableTask` of an
/// `OutputUploadTask`, with its body (`doUploadOutputs`) and its completion action
/// (`onUploadComplete`).
///
/// PORT: the body starts where `doUploadOutputs` no longer needs the spawn: the
/// `UploadManifest` is already built (see [`OutputUploadTask::start`]) and the context is the
/// detached copy of the action's context.
// Java: com/google/devtools/build/lib/remote/RemoteExecutionService.java:1821 RemoteExecutionService.OutputUploadTask.upload
struct OutputUpload {
    manifest: UploadManifest,
    context: RemoteActionExecutionContext<'static>,
    combined_cache: Arc<CombinedCache>,
    upload_error_reporter: UploadErrorReporter,
    force: bool,
    on_upload_complete: Box<dyn FnOnce() + Send>,
}

impl OutputUpload {
    /// `OutputUploadTask.run()`: `upload.runIfNotCancelled()`, which runs the body and then the
    /// completion action.
    // Java: com/google/devtools/build/lib/remote/RemoteExecutionService.java:1872 RemoteExecutionService.OutputUploadTask.run
    fn run(self) {
        // The rest of `doUploadOutputs(action, spawnResult, force, /* onUploadComplete= */ () -> {})`.
        match self.manifest.upload(
            &self.context,
            &self.combined_cache,
            &*self.upload_error_reporter.reporter,
            self.force,
        ) {
            Ok(_unused) => {}
            Err(error) => report_background_upload_error(&self.upload_error_reporter, error),
        }
        (self.on_upload_complete)();
    }

    /// `upload.requestCancellation()` for an upload that has not started: prevents it from
    /// starting and runs the completion action on its behalf.
    // Java: com/google/devtools/build/lib/concurrent/CancellableTask.java:138 CancellableTask.requestCancellation
    fn request_cancellation(self) {
        (self.on_upload_complete)();
    }
}

/// What `OutputUploadTask` does with an error of `doUploadOutputs`: an `IOException` is reported
/// by `doUploadOutputs` itself, an `ExecException` by the task
/// (`catch (ExecException e) { reportUploadError(e); }`) and an `InterruptedException` is
/// dropped (`// ThreadPerTaskExecutor does not care about interrupt status.`).
// Java: com/google/devtools/build/lib/remote/RemoteExecutionService.java:1833 RemoteExecutionService.OutputUploadTask.<init>
fn report_background_upload_error(
    upload_error_reporter: &UploadErrorReporter,
    error: SpawnCacheError,
) {
    match &error {
        SpawnCacheError::Io(e) if errors::is_interrupted(e) => {}
        SpawnCacheError::Exec(ExecException::Interrupted) => {}
        _ => upload_error_reporter.report_upload_error(&error),
    }
}

/// A cancellable background upload of an action's outputs.
///
/// Registers itself with the `RemoteRewoundActionSynchronizer`, if there is one, before the
/// upload starts and unregisters itself when it is done, so that a rewinding of the action waits
/// for uploads that are still in flight.
///
/// `CancellableTask` ensures that the completion callback runs exactly once and that
/// cancellation only completes once the upload no longer accesses the action's outputs.
///
/// PORT: there is no `RemoteRewoundActionSynchronizer` (no output service, no rewinding), so the
/// task is never registered and nothing requests its cancellation: `requestCancellation` and
/// `awaitCompletion` (`RemoteRewoundActionSynchronizer.Cancellable`) have no caller and no port.
// Java: com/google/devtools/build/lib/remote/RemoteExecutionService.java:1820 RemoteExecutionService.OutputUploadTask
struct OutputUploadTask<'t, 'a> {
    service: &'t RemoteExecutionService,
    action: &'t RemoteAction<'a>,
    spawn_result: &'t SpawnResult,
    force: bool,
    on_upload_complete: Box<dyn FnOnce() + Send>,
}

impl<'t, 'a> OutputUploadTask<'t, 'a> {
    // Java: com/google/devtools/build/lib/remote/RemoteExecutionService.java:1829 RemoteExecutionService.OutputUploadTask.<init>
    fn new(
        service: &'t RemoteExecutionService,
        action: &'t RemoteAction<'a>,
        spawn_result: &'t SpawnResult,
        force: bool,
        on_upload_complete: Box<dyn FnOnce() + Send>,
    ) -> OutputUploadTask<'t, 'a> {
        // PORT: `this.rewoundActionSynchronizer = getRewoundActionSynchronizer()` is null: the
        // output service is not a `RemoteOutputService` (there is none).
        OutputUploadTask {
            service,
            action,
            spawn_result,
            force,
            on_upload_complete,
        }
    }

    /// Registers the task for cancellation and starts the upload in the background.
    ///
    /// PORT: Java hands the whole of `doUploadOutputs` to the background thread, which keeps
    /// using the spawn and its execution context. The port cannot keep those borrows past this
    /// call, so the part of `doUploadOutputs` that needs them runs here, on the calling thread,
    /// before the hand-off:
    ///
    /// - `buildUploadManifest(action, spawnResult)`: the check that the mandatory outputs exist,
    ///   reading and digesting the output files and directories, stdout and stderr, and the
    ///   remote cache capabilities;
    /// - reporting an error of that step (`reportUploadError`) and, in that case, running
    ///   `onUploadComplete`.
    ///
    /// What runs in the background is `manifest.upload(...)` (the `FindMissingBlobs` query, the
    /// uploads of the blobs and of the action result), the reporting of its error and
    /// `onUploadComplete`. It uses `context.detached()`, which has no spawn owner, so the
    /// background upload posts no `ActionUploadStartedEvent` / `ActionUploadFinishedEvent`.
    // Java: com/google/devtools/build/lib/remote/RemoteExecutionService.java:1846 RemoteExecutionService.OutputUploadTask.start
    fn start(self) {
        let service = self.service;
        let upload_error_reporter = service.upload_error_reporter();

        if service.background_task_executor.is_shutdown() {
            // PORT: the `RejectedExecutionException` of `backgroundTaskExecutor.execute`, found
            // before the manifest is built (Java does not build it then).
            //
            // The upload will never run. Cancelling it runs the completion callback if no
            // cancellation got there first.
            (self.on_upload_complete)();
            panic!("RejectedExecutionException");
        }

        let manifest = match service.build_upload_manifest(self.action, self.spawn_result) {
            Ok(manifest) => manifest,
            Err(error) => {
                report_background_upload_error(&upload_error_reporter, error);
                (self.on_upload_complete)();
                return;
            }
        };

        let upload = OutputUpload {
            manifest,
            context: self.action.get_remote_action_execution_context().detached(),
            combined_cache: service.combined_cache_not_null().clone(),
            upload_error_reporter,
            force: self.force,
            on_upload_complete: self.on_upload_complete,
        };
        // Executes rather than submits the upload: the body of a task submitted to an
        // ExecutorService is skipped entirely if its future is cancelled before the body starts,
        // which is why cancellation goes through the task itself instead of its future.
        if let Err(upload) = service.background_task_executor.execute(upload) {
            // The upload will never run. Cancelling it runs the completion callback if no
            // cancellation got there first. A concurrent cancellation may still be running the
            // callback, whose effects are part of the upload's completion contract, so wait for
            // it before propagating the rejection.
            upload.request_cancellation();
            panic!("RejectedExecutionException");
        }
    }
}

/// `prettyPrint(actionInput)`.
///
/// PORT: the download half has the same port as a private function of its module; a private
/// item of the other file cannot be called from here.
// Java: com/google/devtools/build/lib/remote/RemoteExecutionService.java:1718 RemoteExecutionService.prettyPrint
fn pretty_print(action_input: &dyn ActionInput) -> String {
    if let Some(artifact) = action_input.as_artifact() {
        // Java: com/google/devtools/build/lib/actions/Artifact.java:669 Artifact.prettyPrint
        artifact
            .get_root_relative_path()
            .get_path_string()
            .to_string()
    } else {
        action_input.get_exec_path_string()
    }
}

/// `artifact.getRoot().isExternal()` for an input that is an `Artifact`.
// Java: com/google/devtools/build/lib/actions/ArtifactRoot.java:238 ArtifactRoot.isExternal
fn artifact_root_is_external(input: &ActionInputRef) -> bool {
    match as_blaze_artifact(input) {
        Some(artifact) => artifact.get_root().is_external(),
        None => blaze_util::unported!(
            "Artifact.getRoot().isExternal() (missing hook: blaze_exec::actions::Artifact has no root type; only blaze_actions artifacts can be asked)"
        ),
    }
}

/// `metadata.wasModifiedSinceDigest(path)`.
///
/// PORT: missing hook `blaze_exec::actions::FileArtifactValue::was_modified_since_digest`. The
/// method is ported here, over the variants of blaze_exec's value, because the default
/// `--guard_against_concurrent_changes=lite` calls it for every upload; it belongs in blaze_exec.
// Java: com/google/devtools/build/lib/actions/FileArtifactValue.java:228 FileArtifactValue.wasModifiedSinceDigest
fn was_modified_since_digest(metadata: &FileArtifactValue, path: &Path) -> io::Result<bool> {
    match metadata {
        // Java: com/google/devtools/build/lib/actions/FileArtifactValue.java:631 RegularFileArtifactValue.wasModifiedSinceDigest
        FileArtifactValue::RegularFile { digest, proxy, .. } => {
            let Some(proxy) = proxy else {
                return Ok(false);
            };
            let stat = path.stat_if_found(Symlinks::Follow)?;
            let Some(stat) = stat.filter(|stat| stat.is_file()) else {
                // The file no longer exists or changed type, so it certainly has changed.
                return Ok(true);
            };
            let new_proxy = FileContentsProxy::create(&stat);
            if *proxy == new_proxy {
                // If the proxy is the same, then the file certainly hasn't been modified. This is
                // the common case, so we check it first.
                return Ok(false);
            }
            if proxy.is_modified(&new_proxy) {
                // If the non-ctime information in the proxy changed, the file has certainly been
                // modified between the time the digest was computed and now.
                return Ok(true);
            }
            // At this point the ctime changed, so some of the file's metadata has changed since
            // we computed the digest. Returning true here would allow us to cautiously report
            // modification even in complex ABA scenarios (file modified, then modified back with
            // its mtime reset). However, we would also report modification in case a hardlink to
            // the file was created or removed, such as by the hermetic Linux sandbox or certain
            // optimized copy actions.
            // As a compromise, we check whether the current state of the file differs from the
            // previous one, ignoring any inbetween modifications that may have happened.
            //
            // Note that this path is always taken when using the hermetic Linux sandbox, but the
            // associated cost should amortize over the next build as the digest will be cached
            // under the new stat.
            //
            // Java: DigestUtils.getDigestWithManualFallback(path, SyscallCache.NO_CACHE, stat)
            let new_digest = path.get_digest()?;
            Ok(digest.as_deref() != Some(&new_digest[..]))
        }
        // Java: com/google/devtools/build/lib/actions/FileArtifactValue.java:513 DirectoryArtifactValue.wasModifiedSinceDigest
        FileArtifactValue::Directory { .. } => Ok(false),
        // Java: com/google/devtools/build/lib/actions/FileArtifactValue.java:567 HashedDirectoryArtifactValue.wasModifiedSinceDigest
        //
        // TODO(ulfjack): Ideally, we'd attempt to detect intra-build modifications here. I'm
        // consciously deferring work here as this code will most likely change again, and we're
        // already doing better than before by detecting inter-build modifications.
        FileArtifactValue::HashedDirectory { .. } => Ok(false),
        // Java: com/google/devtools/build/lib/actions/FileArtifactValue.java:1213 UnresolvedSymlinkArtifactValue.wasModifiedSinceDigest
        FileArtifactValue::UnresolvedSymlink { .. } => blaze_util::unported!(
            "FileArtifactValue.UnresolvedSymlinkArtifactValue.wasModifiedSinceDigest (missing hook: blaze_exec::actions::FileArtifactValue::create_for_unresolved_symlink)"
        ),
        // Java: com/google/devtools/build/lib/actions/FileArtifactValue.java:801 RemoteFileArtifactValue.wasModifiedSinceDigest
        FileArtifactValue::RemoteFile { .. } => Ok(false),
        // Java: com/google/devtools/build/lib/actions/FileArtifactValue.java:1017 ResolvedSymlinkArtifactValue.wasModifiedSinceDigest
        FileArtifactValue::ResolvedSymlink { delegate, .. } => {
            was_modified_since_digest(delegate, path)
        }
        // Java: com/google/devtools/build/lib/actions/FileArtifactValue.java:706 RunfilesProxyArtifactValue.wasModifiedSinceDigest
        FileArtifactValue::RunfilesProxy { .. } => Ok(false),
        // Java: com/google/devtools/build/lib/actions/FileArtifactValue.java:1461 ConstantMetadataValue.wasModifiedSinceDigest
        FileArtifactValue::ConstantMetadata => panic!(
            "UnsupportedOperationException: ConstantMetadataValue doesn't support wasModifiedSinceDigest {path}"
        ),
        // Java: com/google/devtools/build/lib/actions/FileArtifactValue.java:1377 ProxyFileArtifactValue.wasModifiedSinceDigest
        FileArtifactValue::Proxy { delegate, .. } => was_modified_since_digest(delegate, path),
        // Java: com/google/devtools/build/lib/actions/FileArtifactValue.java:1415 SingletonMarkerValue.wasModifiedSinceDigest
        FileArtifactValue::RunfilesTreeMarker | FileArtifactValue::MissingFileMarker => Ok(false),
    }
}

impl RemoteExecutionService {
    /// `new RemoteExecutionService(reporter, verboseFailures, execRoot, remotePathResolver,
    /// buildRequestId, commandId, workspaceName, digestUtil, remoteOptions, executionOptions,
    /// combinedCache, remoteExecutor, tempPathGenerator, captureCorruptedOutputsDir,
    /// remoteOutputChecker, outputService, knownMissingCasDigests, wasRewound)`.
    ///
    /// PORT:
    /// - Java's `combinedCache` argument is either `combined_cache` (a plain `CombinedCache`) or
    ///   `remote_execution_cache` (a `RemoteExecutionCache`); at most one of them is given.
    /// - `scrubber` is an argument (Java reads `remoteOptions.scrubber`; Blaze's `RemoteOptions`
    ///   has no parsed scrubber).
    /// - `outputService` and `wasRewound` are not ported.
    // Java: com/google/devtools/build/lib/remote/RemoteExecutionService.java:218 RemoteExecutionService.<init>
    pub fn new(
        reporter: Arc<dyn ExtendedEventHandler>,
        verbose_failures: bool,
        exec_root: Path,
        remote_path_resolver: Arc<dyn RemotePathResolver>,
        build_request_id: &str,
        command_id: &str,
        workspace_name: &str,
        digest_util: DigestUtil,
        remote_options: Arc<RemoteOptions>,
        execution_options: Arc<ExecutionOptions>,
        combined_cache: Option<Arc<CombinedCache>>,
        remote_execution_cache: Option<Arc<RemoteExecutionCache>>,
        remote_executor: Option<Arc<dyn RemoteExecutionClient>>,
        temp_path_generator: Arc<TempPathGenerator>,
        capture_corrupted_outputs_dir: Option<Path>,
        remote_output_checker: Option<Arc<RemoteOutputChecker>>,
        known_missing_cas_digests: Arc<Mutex<HashSet<Digest>>>,
        scrubber: Option<Arc<Scrubber>>,
    ) -> RemoteExecutionService {
        blaze_util::check_argument!(
            combined_cache.is_none() || remote_execution_cache.is_none(),
            "combinedCache is either a CombinedCache or a RemoteExecutionCache"
        );
        let combined_cache = match &remote_execution_cache {
            Some(remote_execution_cache) => Some(remote_execution_cache.combined_cache().clone()),
            None => combined_cache,
        };
        let merkle_tree_computer = MerkleTreeComputer::new(
            digest_util.clone(),
            // Merkle trees only need to be uploaded for actions that are executed remotely.
            remote_execution_cache
                .clone()
                .map(|remote_execution_cache| {
                    remote_execution_cache as Arc<dyn MerkleTreeUploader>
                }),
            build_request_id,
            command_id,
            workspace_name,
            // PORT: stands for the `ArtifactPathResolver` of the spawn execution contexts (see
            // `MerkleTreeComputer::new`).
            exec_root.clone(),
        );
        RemoteExecutionService {
            reporter,
            verbose_failures,
            exec_root,
            base_remote_path_resolver: remote_path_resolver,
            build_request_id: build_request_id.to_string(),
            command_id: command_id.to_string(),
            digest_util,
            merkle_tree_computer,
            remote_options,
            execution_options,
            combined_cache,
            remote_execution_cache,
            remote_executor,
            temp_path_generator,
            capture_corrupted_outputs_dir,
            reported_errors: Arc::new(Mutex::new(HashSet::new())),
            background_task_executor: BackgroundTaskExecutor::new(),
            shutdown: AtomicBool::new(false),
            build_interrupted: Arc::new(AtomicBool::new(false)),
            remote_output_checker,
            scrubber,
            known_missing_cas_digests,
            hooks: crate::hooks::RemoteHooks::new(),
            use_output_paths: OnceLock::new(),
            // `Runtime.getRuntime().availableProcessors()`
            remote_action_building_semaphore: Semaphore::new(
                std::thread::available_parallelism().map_or(1, |n| n.get()),
            ),
        }
    }

    /// Provides the integration seams of this build (see `crate::hooks`).
    pub fn with_hooks(mut self, hooks: crate::hooks::RemoteHooks) -> RemoteExecutionService {
        self.hooks = hooks;
        self
    }

    /// `combinedCache`, where Java dereferences it without a null check.
    fn combined_cache_not_null(&self) -> &Arc<CombinedCache> {
        match &self.combined_cache {
            Some(combined_cache) => combined_cache,
            None => panic!("NullPointerException: combinedCache"),
        }
    }

    fn upload_error_reporter(&self) -> UploadErrorReporter {
        UploadErrorReporter {
            reporter: self.reporter.clone(),
            reported_errors: self.reported_errors.clone(),
            build_interrupted: self.build_interrupted.clone(),
            verbose_failures: self.verbose_failures,
        }
    }

    // Java: com/google/devtools/build/lib/remote/RemoteExecutionService.java:270 RemoteExecutionService.buildCommand
    fn build_command(
        &self,
        use_output_paths: bool,
        outputs: &[ActionInputRef],
        arguments: &[String],
        env: &IndexMap<String, String>,
        platform: Option<Platform>,
        remote_path_resolver: &dyn RemotePathResolver,
        spawn_scrubber: Option<&SpawnScrubber>,
        execution_platform: Option<&PlatformInfo>,
    ) -> Command {
        let mut command = Command::default();
        if use_output_paths {
            let mut output_paths: Vec<String> = Vec::new();
            for output in outputs {
                let path_string = internal_to_unicode(
                    &remote_path_resolver.local_action_input_to_output_path(&**output),
                );
                output_paths.push(path_string);
            }
            output_paths.sort_by(|a, b| proto_string_comparator(a, b));
            command.output_paths.extend(output_paths);
        } else {
            let mut output_files: Vec<String> = Vec::new();
            let mut output_directories: Vec<String> = Vec::new();
            for output in outputs {
                let path_string = internal_to_unicode(
                    &remote_path_resolver.local_action_input_to_output_path(&**output),
                );
                if output.is_directory() {
                    output_directories.push(path_string);
                } else {
                    output_files.push(path_string);
                }
            }
            output_files.sort_by(|a, b| proto_string_comparator(a, b));
            output_directories.sort_by(|a, b| proto_string_comparator(a, b));
            command.output_files.extend(output_files);
            command.output_directories.extend(output_directories);
        }

        if let Some(platform) = platform {
            command.platform = Some(platform);
        }
        let mut first = true;
        for arg in arguments {
            let mut arg = arg.clone();
            if let Some(spawn_scrubber) = spawn_scrubber {
                arg = spawn_scrubber.transform_argument(&arg);
            }
            if first && execution_platform.is_some() {
                first = false;
                // PORT: skipped, Windows only. Java:
                //   OS executionOs = getOsFromConstraintsOrHost(executionPlatform);
                //   arg = OsPathPolicy.of(executionOs).postProcessPathStringForExecution(arg);
                // `OsPathPolicy.of` is the Unix policy for every OS but Windows. blaze_exec's
                // `PlatformInfo` has no constraints to read the OS from, and blaze_vfs has no
                // Windows policy, so the Unix policy (the identity) is applied.
                arg = HOST_POLICY.post_process_path_string_for_execution(&arg);
            }
            command.arguments.push(internal_to_unicode(&arg));
        }
        // Sorting the environment pairs by variable name.
        //
        // PORT: `new TreeSet<>(env.keySet())` orders the names, which are internal strings, by
        // `String.compareTo`: by their bytes. That is the order of `str` whether Blaze's strings
        // are Unicode or internal.
        let variables: BTreeSet<&String> = env.keys().collect();
        for var in variables {
            command.environment_variables.push(EnvironmentVariable {
                name: internal_to_unicode(var),
                value: internal_to_unicode(&env[var]),
            });
        }

        command.working_directory = internal_to_unicode(
            remote_path_resolver
                .get_working_directory()
                .get_path_string(),
        );
        command
    }

    // Java: com/google/devtools/build/lib/remote/RemoteExecutionService.java:334 RemoteExecutionService.useRemoteCache
    fn use_remote_cache(&self) -> bool {
        self.combined_cache
            .as_ref()
            .is_some_and(|combined_cache| combined_cache.has_remote_cache())
    }

    // Java: com/google/devtools/build/lib/remote/RemoteExecutionService.java:338 RemoteExecutionService.useDiskCache
    fn use_disk_cache(&self) -> bool {
        self.combined_cache
            .as_ref()
            .is_some_and(|combined_cache| combined_cache.has_disk_cache())
    }

    /// `getReadCachePolicy(spawn)`.
    // Java: com/google/devtools/build/lib/remote/RemoteExecutionService.java:342 RemoteExecutionService.getReadCachePolicy
    pub fn get_read_cache_policy(&self, spawn: &dyn Spawn) -> CachePolicy {
        if self.combined_cache.is_none() {
            return CachePolicy::NoCache;
        }

        let allow_remote_cache = self.use_remote_cache()
            && self.remote_options.remote_accept_cached
            && spawns::may_be_cached_remotely(spawn);
        let allow_disk_cache = self.use_disk_cache() && spawns::may_be_cached(spawn);

        CachePolicy::create(allow_remote_cache, allow_disk_cache)
    }

    /// `getWriteCachePolicy(spawn)`.
    // Java: com/google/devtools/build/lib/remote/RemoteExecutionService.java:354 RemoteExecutionService.getWriteCachePolicy
    pub fn get_write_cache_policy(&self, spawn: &dyn Spawn) -> CachePolicy {
        let Some(combined_cache) = &self.combined_cache else {
            return CachePolicy::NoCache;
        };

        let allow_remote_cache = self.use_remote_cache()
            && should_upload_local_results_to_remote_cache(
                &self.remote_options,
                spawn.get_execution_info(),
            )
            && combined_cache.remote_action_cache_supports_update();
        let allow_disk_cache = self.use_disk_cache() && spawns::may_be_cached(spawn);

        CachePolicy::create(allow_remote_cache, allow_disk_cache)
    }

    /// Returns `true` if the spawn may be executed remotely.
    // Java: com/google/devtools/build/lib/remote/RemoteExecutionService.java:369 RemoteExecutionService.mayBeExecutedRemotely
    pub fn may_be_executed_remotely(&self, spawn: &dyn Spawn) -> bool {
        self.remote_execution_cache.is_some()
            && self.remote_executor.is_some()
            && spawns::may_be_executed_remotely(spawn)
            && !Self::is_scrubbed_spawn(spawn, self.scrubber.as_deref())
    }

    // Java: com/google/devtools/build/lib/remote/RemoteExecutionService.java:377 RemoteExecutionService.buildSalt
    fn build_salt(spawn: &dyn Spawn, spawn_scrubber: Option<&SpawnScrubber>) -> Option<Vec<u8>> {
        let mut salt_builder = CacheSalt {
            may_be_executed_remotely: spawns::may_be_executed_remotely(spawn),
            ..Default::default()
        };

        let workspace = spawn
            .get_execution_info()
            .get(execution_requirements::DIFFERENTIATE_WORKSPACE_CACHE);
        if let Some(workspace) = workspace {
            // PORT: Java stores the internal string as it is (no `internalToUnicode`); the
            // message then holds the UTF-8 encoding of the internal string's chars.
            salt_builder.workspace =
                blaze_util::string_encoding::to_internal(workspace).into_owned();
        }

        if let Some(spawn_scrubber) = spawn_scrubber {
            salt_builder.scrub_salt = Some(cache_salt::ScrubSalt {
                salt: spawn_scrubber.get_salt().to_string(),
            });
        }

        Some(salt_builder.to_byte_array())
    }

    // Java: com/google/devtools/build/lib/remote/RemoteExecutionService.java:412 RemoteExecutionService.getToolSignature
    fn get_tool_signature(
        &self,
        spawn: &dyn Spawn,
        context: &dyn SpawnExecutionContext,
    ) -> Result<Option<ToolSignature>, SpawnCacheError> {
        if self.remote_options.mark_tool_inputs
            && spawns::supports_workers(spawn)
            && !spawn.get_tool_files().is_empty()
        {
            Ok(Some(
                self.compute_persistent_worker_signature(spawn, context)?,
            ))
        } else {
            Ok(None)
        }
    }

    // Java: com/google/devtools/build/lib/remote/RemoteExecutionService.java:421 RemoteExecutionService.maybeAcquireRemoteActionBuildingSemaphore
    fn maybe_acquire_remote_action_building_semaphore(&self) {
        if !self.remote_options.throttle_remote_action_building {
            return;
        }

        // PORT: skipped, profiling ("acquiring semaphore").
        self.remote_action_building_semaphore.acquire();
    }

    // Java: com/google/devtools/build/lib/remote/RemoteExecutionService.java:432 RemoteExecutionService.maybeReleaseRemoteActionBuildingSemaphore
    fn maybe_release_remote_action_building_semaphore(&self) {
        if !self.remote_options.throttle_remote_action_building {
            return;
        }

        self.remote_action_building_semaphore.release();
    }

    // Java: com/google/devtools/build/lib/remote/RemoteExecutionService.java:440 RemoteExecutionService.useOutputPaths
    pub(crate) fn use_output_paths(&self) -> bool {
        // PORT: `synchronized initUseOutputPaths()` with its "already initialized" check is the
        // `OnceLock`.
        *self
            .use_output_paths
            .get_or_init(|| self.init_use_output_paths())
    }

    // Java: com/google/devtools/build/lib/remote/RemoteExecutionService.java:447 RemoteExecutionService.initUseOutputPaths
    fn init_use_output_paths(&self) -> bool {
        let compute_server_highest_version = || -> io::Result<Option<ApiVersion>> {
            let mut server_highest_version: Option<ApiVersion> = None;
            // If both Remote Executor and Remote Cache are configured,
            // use the highest version supported by both.

            let mut executor_support_status: Option<ServerSupportedStatus> = None;
            if let Some(remote_executor) = &self.remote_executor {
                // PORT: never null here (Java checks `serverCapabilities != null`).
                let server_capabilities = remote_executor.get_server_capabilities()?;
                executor_support_status = Some(
                    ClientApiVersion::CURRENT.check_server_supported_versions(&server_capabilities),
                );
            }

            let mut cache_support_status: Option<ServerSupportedStatus> = None;
            if let Some(combined_cache) = &self.combined_cache {
                // PORT: never null here (Java checks `serverCapabilities != null`).
                let server_capabilities = combined_cache.get_remote_server_capabilities()?;
                cache_support_status = Some(
                    ClientApiVersion::CURRENT.check_server_supported_versions(&server_capabilities),
                );
            }

            let mut executor_highest_version: Option<ApiVersion> = None;
            if let Some(executor_support_status) = &executor_support_status
                && executor_support_status.is_supported()
            {
                executor_highest_version = executor_support_status
                    .get_highest_supported_version()
                    .cloned();
            }

            let mut cache_highest_version: Option<ApiVersion> = None;
            if let Some(cache_support_status) = &cache_support_status
                && cache_support_status.is_supported()
            {
                cache_highest_version = cache_support_status
                    .get_highest_supported_version()
                    .cloned();
            }

            if let (Some(executor_highest_version), Some(cache_highest_version)) =
                (&executor_highest_version, &cache_highest_version)
            {
                // `min(ImmutableList.of(executorHighestVersion, cacheHighestVersion))`: the first
                // of the two when they compare as equal.
                server_highest_version = Some(
                    if cache_highest_version.compare_to(executor_highest_version) == Ordering::Less
                    {
                        cache_highest_version.clone()
                    } else {
                        executor_highest_version.clone()
                    },
                );
            } else if executor_highest_version.is_some() {
                server_highest_version = executor_highest_version;
            } else if cache_highest_version.is_some() {
                server_highest_version = cache_highest_version;
            }
            Ok(server_highest_version)
        };
        let server_highest_version: Option<ApiVersion> = match compute_server_highest_version() {
            Ok(server_highest_version) => server_highest_version,
            Err(_e) => {
                // Intentionally ignored.
                None
            }
        };
        match server_highest_version {
            None => true,
            Some(server_highest_version) => {
                server_highest_version.compare_to(&ApiVersion::TWO_POINT_ONE) != Ordering::Less
            }
        }
    }

    /// `buildRemoteAction(spawn, context)` (`@VisibleForTesting`).
    // Java: com/google/devtools/build/lib/remote/RemoteExecutionService.java:500 RemoteExecutionService.buildRemoteAction
    pub fn build_remote_action<'a>(
        &self,
        spawn: &'a dyn Spawn,
        context: &'a dyn SpawnExecutionContext,
    ) -> Result<RemoteAction<'a>, SpawnCacheError> {
        self.build_remote_action_with_blob_policy(
            spawn,
            context,
            if self.remote_options.remote_discard_merkle_trees {
                BlobPolicy::Discard
            } else {
                BlobPolicy::KeepAndReupload
            },
        )
    }

    /// Creates a new [`RemoteAction`] instance from spawn.
    // Java: com/google/devtools/build/lib/remote/RemoteExecutionService.java:511 RemoteExecutionService.buildRemoteAction
    pub fn build_remote_action_with_blob_policy<'a>(
        &self,
        spawn: &'a dyn Spawn,
        context: &'a dyn SpawnExecutionContext,
        blob_policy: BlobPolicy,
    ) -> Result<RemoteAction<'a>, SpawnCacheError> {
        self.maybe_acquire_remote_action_building_semaphore();
        let _finally = RemoteActionBuildingSemaphoreRelease(self);

        // Create a remote path resolver that is aware of the spawn's path mapper, which rewrites
        // the paths of the inputs and outputs as well as paths appearing in the command line for
        // execution. This is necessary to ensure that artifacts are correctly emitted into and
        // staged from the unmapped location locally.
        let remote_path_resolver: Arc<dyn RemotePathResolver> = remote_path_resolver::create_mapped(
            self.base_remote_path_resolver.clone(),
            &self.exec_root,
            spawn.get_path_mapper(),
        );
        let tool_signature = self.get_tool_signature(spawn, context)?;
        let no_tool_inputs: HashSet<PathFragment> = HashSet::new();
        let merkle_tree: MerkleTree = match self.merkle_tree_computer.build_for_spawn(
            spawn,
            match &tool_signature {
                Some(tool_signature) => &tool_signature.tool_inputs,
                None => &no_tool_inputs,
            },
            self.scrubber.as_deref(),
            context,
            &*remote_path_resolver,
            blob_policy,
        ) {
            Ok(merkle_tree) => merkle_tree,
            Err(SpawnCacheError::Io(e)) => {
                // PORT: `catch (CredentialHelperException e) { throw
                // createExecExceptionForCredentialHelperException(e); }` has no port:
                // `CredentialHelperException` (lib/authandtls/credentialhelper) is not ported, so
                // no error is one.
                if let Some(e) = errors::downcast_ref::<RemoteExecutionCapabilitiesException>(&e) {
                    return Err(
                        create_exec_exception_from_remote_execution_capabilities_exception(e)
                            .into(),
                    );
                }
                return Err(e.into());
            }
            Err(e) => return Err(e),
        };

        // Get the remote platform properties.
        let platform: Option<Platform> = if let Some(tool_signature) = &tool_signature {
            let mut additional_properties: IndexMap<String, String> = IndexMap::new();
            additional_properties.insert(
                "persistentWorkerKey".to_string(),
                tool_signature.key.clone(),
            );
            platform_utils::get_platform_proto_with_additional_properties(
                spawn,
                Some(&self.remote_options),
                &additional_properties,
            )?
        } else {
            platform_utils::get_platform_proto(spawn, Some(&self.remote_options))?
        };

        let spawn_scrubber: Option<&SpawnScrubber> = match &self.scrubber {
            Some(scrubber) => scrubber.for_spawn(spawn),
            None => None,
        };
        let command = self.build_command(
            self.use_output_paths(),
            spawn.get_output_files(),
            spawn.get_arguments(),
            spawn.get_environment(),
            platform.clone(),
            &*remote_path_resolver,
            spawn_scrubber,
            spawn.get_execution_platform(),
        );
        let command_hash: Digest = self.digest_util.compute_message(&command);
        let action: Action = utils::build_action(
            command_hash.clone(),
            merkle_tree.digest().clone(),
            platform,
            context.get_timeout(),
            spawns::may_be_cached_remotely(spawn),
            Self::build_salt(spawn, spawn_scrubber),
        );

        let action_key: ActionKey = self.digest_util.compute_action_key(&action);

        let metadata: RequestMetadata = tracing_metadata_utils::build_metadata(
            &self.build_request_id,
            &self.command_id,
            &action_key.digest().hash,
            Some(spawn.get_resource_owner()),
        );
        let remote_action_execution_context = RemoteActionExecutionContext::create_with_policies(
            spawn,
            context,
            metadata,
            self.get_write_cache_policy(spawn),
            self.get_read_cache_policy(spawn),
        );
        Ok(RemoteAction::new(
            spawn,
            context,
            remote_action_execution_context,
            remote_path_resolver,
            merkle_tree,
            command_hash,
            command,
            action,
            action_key,
        ))
    }

    // Java: com/google/devtools/build/lib/remote/RemoteExecutionService.java:593 RemoteExecutionService.computePersistentWorkerSignature
    fn compute_persistent_worker_signature(
        &self,
        _spawn: &dyn Spawn,
        _context: &dyn SpawnExecutionContext,
    ) -> Result<ToolSignature, SpawnCacheError> {
        // Java:
        //   WorkerParser workerParser =
        //       new WorkerParser(
        //           execRoot, Options.getDefaults(WorkerOptions.class), LocalEnvProvider.NOOP, null);
        //   WorkerKey workerKey = workerParser.compute(spawn, context).getWorkerKey();
        //   Fingerprint fingerprint = new Fingerprint();
        //   // getWorkerFilesCombinedHash always uses SHA-256, so the hash is always 32 bytes.
        //   fingerprint.addBytes(workerKey.getWorkerFilesCombinedHash().asBytes());
        //   fingerprint.addStrings(workerKey.getArgs());
        //   fingerprint.addStringMap(workerKey.getEnv());
        //   return new ToolSignature(
        //       fingerprint.hexDigestAndReset(), workerKey.getWorkerFilesWithDigests().keySet());
        blaze_util::unported!(
            "RemoteExecutionService.computePersistentWorkerSignature (WorkerParser)"
        )
    }

    /// Lookup the remote cache for the given [`RemoteAction`]. `None` if not found.
    // Java: com/google/devtools/build/lib/remote/RemoteExecutionService.java:770 RemoteExecutionService.lookupCache
    pub fn lookup_cache(
        &self,
        action: &RemoteAction<'_>,
    ) -> io::Result<Option<RemoteActionResult>> {
        blaze_util::check_state!(
            action
                .get_remote_action_execution_context()
                .get_read_cache_policy()
                .allow_any_cache(),
            "spawn doesn't accept cached result"
        );

        // A rewound action must regenerate its lost outputs even if the cache still serves its
        // stale action result. Treat this as a cache miss.
        // TODO(https://github.com/bazelbuild/remote-apis/pull/386): Allow cache lookup for rewound
        // actions when CacheCapabilities.verifies_action_results is advertised, while still
        // bypassing unverified cache layers.
        //
        // PORT: `wasRewound.test(action.getSpawn().getResourceOwner())` is false (rewinding is
        // not ported).

        let mut inline_output_files: HashSet<String> = HashSet::new();
        let in_memory_output_path = get_in_memory_output_path(action.get_spawn());
        if let Some(in_memory_output_path) = &in_memory_output_path {
            // PORT: Java puts the internal string into the request as it is (no
            // `internalToUnicode`, unlike `executeRemotely`).
            inline_output_files.insert(
                blaze_util::string_encoding::to_internal(
                    &action
                        .get_remote_path_resolver()
                        .local_exec_path_to_output_path(in_memory_output_path),
                )
                .into_owned(),
            );
        }

        let combined_cache = self.combined_cache_not_null();
        let cached_action_result: Option<CachedActionResult> = combined_cache
            .download_action_result(
                action.get_remote_action_execution_context(),
                action.get_action_key(),
                /* inlineOutErr= */ false,
                &inline_output_files,
            )?;

        if cached_action_result.is_none() {
            return Ok(None);
        }

        let result = RemoteActionResult::create_from_cache(cached_action_result);

        // The legacy whole-invocation retry path uses knownMissingCasDigests to prevent a retried
        // invocation from accepting the same stale action result. Action rewinding bypasses cache
        // lookup above without populating this set.
        let known_missing_cas_digests_is_empty = self.known_missing_cas_digests.lock().is_empty();
        if !known_missing_cas_digests_is_empty {
            let metadata: &ActionResultMetadata = match result.get_or_parse_action_result_metadata(
                combined_cache,
                &self.digest_util,
                action.get_remote_action_execution_context(),
                &**action.get_remote_path_resolver(),
            ) {
                Ok(metadata) => metadata,
                Err(e) => {
                    match errors::downcast_ref::<BulkTransferException>(&e) {
                        Some(bulk_transfer_exception)
                            if bulk_transfer_exception
                                .all_caused_by_cache_not_found_exception() =>
                        {
                            // Handle dangling reference to lost Tree message as AC miss.
                            return Ok(None);
                        }
                        _ => return Err(e),
                    }
                }
            };

            // If we already know digests referenced by this AC is missing from remote cache,
            // ignore it so that we can fall back to execution. This could happen when the remote
            // cache is an HTTP cache, or doesn't implement AC integrity check.
            //
            // See https://github.com/bazelbuild/bazel/issues/18696.
            if Self::update_known_missing_cas_digests(&self.known_missing_cas_digests, metadata) {
                return Ok(None);
            }
        }

        Ok(Some(result))
    }

    /// Removes digests referenced by `metadata` from `known_missing_cas_digests` and returns
    /// whether any were removed
    // Java: com/google/devtools/build/lib/remote/RemoteExecutionService.java:841 RemoteExecutionService.updateKnownMissingCasDigests
    pub(crate) fn update_known_missing_cas_digests(
        known_missing_cas_digests: &Mutex<HashSet<Digest>>,
        metadata: &ActionResultMetadata,
    ) -> bool {
        // Using `remove` below because we assume the missing blob will be uploaded afterwards.
        let mut result = false;
        for file in metadata.files() {
            if known_missing_cas_digests.lock().remove(file.digest()) {
                result = true;
            }
        }
        for entry in metadata.directories() {
            for file in entry.1.files() {
                if known_missing_cas_digests.lock().remove(file.digest()) {
                    result = true;
                }
            }
        }
        result
    }

    /// `buildUploadManifest(action, spawnResult)` (`@VisibleForTesting`).
    // Java: com/google/devtools/build/lib/remote/RemoteExecutionService.java:1727 RemoteExecutionService.buildUploadManifest
    pub fn build_upload_manifest(
        &self,
        action: &RemoteAction<'_>,
        spawn_result: &SpawnResult,
    ) -> Result<UploadManifest, SpawnCacheError> {
        // PORT: skipped, profiling ("build upload manifest").
        let mut output_files: Vec<Path> = Vec::new();
        // Check that all mandatory outputs are created.
        for output_file in action.get_spawn().get_output_files() {
            let follow_symlinks = if output_file.is_symlink() {
                Symlinks::Nofollow
            } else {
                Symlinks::Follow
            };
            let local_path = self.exec_root.get_relative(output_file.get_exec_path());
            if action.get_spawn().is_mandatory_output(output_file)
                && !local_path.exists_with(follow_symlinks)
            {
                return Err(io_exception(format!(
                    "Expected output {} was not created locally.",
                    pretty_print(&**output_file)
                ))
                .into());
            }
            output_files.push(local_path);
        }

        UploadManifest::create(
            &self
                .combined_cache_not_null()
                .get_remote_cache_capabilities()?,
            self.digest_util.clone(),
            action.get_remote_path_resolver().clone(),
            action.get_action_key().clone(),
            action.get_action(),
            action.get_command(),
            &output_files,
            Some(action.get_spawn_execution_context().get_file_out_err()),
            spawn_result.exit_code(),
            spawn_result.get_start_time(),
            spawn_result.get_wall_time_in_ms(),
            /* preserveExecutableBit= */ false,
        )
    }

    /// Upload outputs of a remote action which was executed locally to remote cache.
    ///
    /// PORT: both Java callers (`RemoteSpawnCache`, `RemoteSpawnRunner`) pass
    /// `remoteOptions.guardAgainstConcurrentChanges` as the `concurrentChangesCheckLevel`
    /// argument; this form takes it from the service's options.
    // Java: com/google/devtools/build/lib/remote/RemoteExecutionService.java:1759 RemoteExecutionService.uploadOutputs
    pub fn upload_outputs(
        &self,
        action: &RemoteAction<'_>,
        spawn_result: &SpawnResult,
        on_upload_complete: Box<dyn FnOnce() + Send>,
    ) -> Result<(), SpawnCacheError> {
        self.upload_outputs_with_concurrent_changes_check_level(
            action,
            spawn_result,
            on_upload_complete,
            self.remote_options.guard_against_concurrent_changes,
        )
    }

    /// Upload outputs of a remote action which was executed locally to remote cache.
    // Java: com/google/devtools/build/lib/remote/RemoteExecutionService.java:1759 RemoteExecutionService.uploadOutputs
    pub fn upload_outputs_with_concurrent_changes_check_level(
        &self,
        action: &RemoteAction<'_>,
        spawn_result: &SpawnResult,
        on_upload_complete: Box<dyn FnOnce() + Send>,
        concurrent_changes_check_level: ConcurrentChangesCheckLevel,
    ) -> Result<(), SpawnCacheError> {
        blaze_util::check_state!(!self.shutdown.load(AtomicOrdering::SeqCst), "shutdown");
        blaze_util::check_state!(
            action
                .get_remote_action_execution_context()
                .get_write_cache_policy()
                .allow_any_cache(),
            "spawn shouldn't upload local result"
        );
        blaze_util::check_state!(
            SpawnResultStatus::Success == spawn_result.status() && spawn_result.exit_code() == 0,
            "shouldn't upload outputs of failed local action"
        );

        // PORT: skipped, profiling ("checkForConcurrentModifications").
        if let Err(e) =
            self.check_for_concurrent_modifications(action, concurrent_changes_check_level)
        {
            self.report(Event::warn(format!(
                "{}: Skipping uploading outputs because of concurrent modifications with --guard_against_concurrent_changes enabled: {}",
                match action.get_spawn().get_target_label() {
                    Some(target_label) => target_label.to_string(),
                    None => "null".to_string(),
                },
                e
            )));
            on_upload_complete();
            return Ok(());
        }

        // The output blobs of a rewound action may have been evicted after this invocation
        // uploaded them, so bypass the deduplication of completed uploads. Otherwise, the
        // refreshed action result could still reference missing blobs. Evaluate this here rather
        // than in the upload task, which may run after the build's rewinding state has been
        // discarded.
        //
        // PORT: `wasRewound.test(action.getSpawn().getResourceOwner())` is false (rewinding is
        // not ported).
        let force = false;
        if self.remote_options.remote_cache_async
            && !self
                .hooks
                .may_modify_spawn_outputs_after_execution(action.get_spawn().get_resource_owner())
        {
            OutputUploadTask::new(self, action, spawn_result, force, on_upload_complete).start();
            Ok(())
        } else {
            self.do_upload_outputs(action, spawn_result, force, on_upload_complete)
        }
    }

    // PORT: `getRewoundActionSynchronizer()` (RemoteExecutionService.java:1800) has no port: it
    // returns null unless the output service is a `RemoteOutputService`.

    // Java: com/google/devtools/build/lib/remote/RemoteExecutionService.java:1893 RemoteExecutionService.doUploadOutputs
    fn do_upload_outputs(
        &self,
        action: &RemoteAction<'_>,
        spawn_result: &SpawnResult,
        force: bool,
        on_upload_complete: Box<dyn FnOnce() + Send>,
    ) -> Result<(), SpawnCacheError> {
        // PORT: skipped, profiling ("upload outputs").
        let upload = || -> Result<(), SpawnCacheError> {
            let manifest: UploadManifest = self.build_upload_manifest(action, spawn_result)?;
            let _unused = manifest.upload(
                action.get_remote_action_execution_context(),
                self.combined_cache_not_null(),
                &*self.reporter,
                force,
            )?;
            Ok(())
        };
        let result = match upload() {
            // catch (IOException e)
            Err(SpawnCacheError::Io(e)) if !errors::is_interrupted(&e) => {
                self.report_upload_error(&SpawnCacheError::Io(e));
                Ok(())
            }
            result => result,
        };
        // finally
        on_upload_complete();
        result
    }

    // Java: com/google/devtools/build/lib/remote/RemoteExecutionService.java:1908 RemoteExecutionService.checkForConcurrentModifications
    fn check_for_concurrent_modifications(
        &self,
        action: &RemoteAction<'_>,
        level: ConcurrentChangesCheckLevel,
    ) -> io::Result<()> {
        if level == ConcurrentChangesCheckLevel::OFF {
            return Ok(());
        }

        // As this check runs after the action has been executed, we can reuse the input map if it
        // has already been created with willAccessRepeatedly = true, but do not need to force its
        // retention.
        for input in action
            .get_input_map(/* willAccessRepeatedly= */ false)
            .values()
        {
            // In lite mode, only check source artifacts in the main repository for modifications.
            // Non-source artifacts are made read-only after execution, and external repositories
            // are rarely modified, with local_repository being the notable exception.
            // TODO: Find a way to include repositories that are symlinks to source directories.
            // On Bazel itself, this reduces the number of wasModifiedSinceDigest calls by 99%
            // compared to the full check. By not checking output files, this mode also avoids
            // spurious false positives (see https://github.com/bazelbuild/bazel/issues/3360).
            if level == ConcurrentChangesCheckLevel::LITE
                && !(input
                    .as_artifact()
                    .is_some_and(|artifact| artifact.is_source_artifact())
                    && !artifact_root_is_external(input))
            {
                continue;
            } else if input.as_virtual_action_input().is_some() {
                continue;
            }
            let metadata: Option<FileArtifactValue> = action
                .get_spawn_execution_context()
                .get_input_metadata_provider()
                .get_input_metadata(input)?;
            let path = self.exec_root.get_relative(input.get_exec_path());
            let Some(metadata) = metadata else {
                panic!("NullPointerException: metadata of {input} is null");
            };
            if was_modified_since_digest(&metadata, &path)? {
                return Err(io_exception(format!(
                    "{path} was modified during execution"
                )));
            }
        }
        Ok(())
    }

    // Java: com/google/devtools/build/lib/remote/RemoteExecutionService.java:1942 RemoteExecutionService.reportUploadError
    fn report_upload_error(&self, error: &SpawnCacheError) {
        self.upload_error_reporter().report_upload_error(error);
    }

    /// Upload inputs of a remote action to remote cache if they are not presented already.
    ///
    /// Must be called before calling [`RemoteExecutionService::execute_remotely`].
    // Java: com/google/devtools/build/lib/remote/RemoteExecutionService.java:1958 RemoteExecutionService.uploadInputsIfNotPresent
    pub fn upload_inputs_if_not_present(
        &self,
        action: &RemoteAction<'_>,
        force: bool,
    ) -> Result<(), SpawnCacheError> {
        blaze_util::check_state!(!self.shutdown.load(AtomicOrdering::SeqCst), "shutdown");
        blaze_util::check_state!(
            self.may_be_executed_remotely(action.get_spawn()),
            "spawn can't be executed remotely"
        );

        let Some(remote_execution_cache) = &self.remote_execution_cache else {
            panic!("ClassCastException: combinedCache is not a RemoteExecutionCache");
        };
        // Upload the command and all the inputs into the remote cache.
        //
        // PORT: the messages are serialized here (see
        // `RemoteExecutionCache::ensure_inputs_present_with_additional_inputs`). Java's map is a
        // `HashMap`, whose order only decides the order of the two digests in the
        // `FindMissingBlobs` request.
        let mut additional_inputs: IndexMap<Digest, bytes::Bytes> = IndexMap::with_capacity(2);
        additional_inputs.insert(
            action.get_action_key().digest().clone(),
            bytes::Bytes::from(action.get_action().to_byte_array()),
        );
        additional_inputs.insert(
            action.get_command_hash().clone(),
            bytes::Bytes::from(action.get_command().to_byte_array()),
        );

        // As uploading depends on having the full input root in memory, limit
        // concurrency. This prevents memory exhaustion. We assume that
        // ensureInputsPresent() provides enough parallelism to saturate the
        // network connection.
        self.maybe_acquire_remote_action_building_semaphore();
        let _finally = RemoteActionBuildingSemaphoreRelease(self);

        let recomputed_merkle_tree: MerkleTreeUploadable;
        let merkle_tree: &MerkleTreeUploadable = match action.get_merkle_tree() {
            MerkleTree::Uploadable(uploadable) if !force => uploadable,
            _ => {
                // --experimental_remote_discard_merkle_trees was provided or the remote lost a
                // shared subtree uploaded previously. Recompute the tree - including all subtrees
                // in the latter case.
                let spawn = action.get_spawn();
                let context = action.get_spawn_execution_context();
                let tool_signature = self.get_tool_signature(spawn, context)?;
                let no_tool_inputs: HashSet<PathFragment> = HashSet::new();
                recomputed_merkle_tree = match self.merkle_tree_computer.build_for_spawn(
                    spawn,
                    match &tool_signature {
                        Some(tool_signature) => &tool_signature.tool_inputs,
                        None => &no_tool_inputs,
                    },
                    self.scrubber.as_deref(),
                    context,
                    &**action.get_remote_path_resolver(),
                    if force {
                        BlobPolicy::KeepAndReupload
                    } else {
                        BlobPolicy::Keep
                    },
                )? {
                    MerkleTree::Uploadable(uploadable) => uploadable,
                    MerkleTree::RootOnly(_) => panic!(
                        "ClassCastException: MerkleTree.RootOnly cannot be cast to MerkleTree.Uploadable"
                    ),
                };
                &recomputed_merkle_tree
            }
        };

        remote_execution_cache.ensure_inputs_present_with_additional_inputs(
            &action
                .get_remote_action_execution_context()
                .with_write_cache_policy(CachePolicy::RemoteCacheOnly), // Only upload to remote cache
            merkle_tree,
            &additional_inputs,
            force,
            Some(&**action.get_remote_path_resolver()),
        )?;
        Ok(())
    }

    /// Executes the remote action remotely and returns the result.
    ///
    /// * `accept_cached_result`: tells remote execution server whether it should used cached
    ///   result.
    /// * `observer`: receives status updates during the execution.
    // Java: com/google/devtools/build/lib/remote/RemoteExecutionService.java:2017 RemoteExecutionService.executeRemotely
    pub fn execute_remotely(
        &self,
        action: &RemoteAction<'_>,
        accept_cached_result: bool,
        observer: &dyn OperationObserver,
    ) -> io::Result<RemoteActionResult> {
        blaze_util::check_state!(!self.shutdown.load(AtomicOrdering::SeqCst), "shutdown");
        blaze_util::check_state!(
            self.may_be_executed_remotely(action.get_spawn()),
            "spawn can't be executed remotely"
        );

        let mut request_builder = ExecuteRequest {
            instance_name: self.remote_options.remote_instance_name.clone(),
            digest_function: self.digest_util.get_digest_function() as i32,
            action_digest: Some(action.get_action_key().digest().clone()),
            // TODO(https://github.com/bazelbuild/remote-apis/pull/386): Allow cached execution
            // results for rewound actions when ExecutionCapabilities.verifies_action_results is
            // advertised.
            //
            // PORT: `|| wasRewound.test(action.getSpawn().getResourceOwner())` is false
            // (rewinding is not ported).
            skip_cache_lookup: !accept_cached_result,
            ..Default::default()
        };
        if self.remote_options.remote_result_cache_priority != 0 {
            request_builder
                .results_cache_policy
                .get_or_insert_with(Default::default)
                .priority = self.remote_options.remote_result_cache_priority;
        }
        if self.remote_options.remote_execution_priority != 0 {
            request_builder
                .execution_policy
                .get_or_insert_with(Default::default)
                .priority = self.remote_options.remote_execution_priority;
        }
        let in_memory_output_path = get_in_memory_output_path(action.get_spawn());
        if let Some(in_memory_output_path) = &in_memory_output_path {
            request_builder
                .inline_output_files
                .push(internal_to_unicode(
                    &action
                        .get_remote_path_resolver()
                        .local_exec_path_to_output_path(in_memory_output_path),
                ));
        }

        let request = request_builder;

        let Some(remote_executor) = &self.remote_executor else {
            panic!("NullPointerException: remoteExecutor");
        };
        let reply: ExecuteResponse = remote_executor.execute_remotely(
            action.get_remote_action_execution_context(),
            request,
            observer,
        )?;

        Ok(RemoteActionResult::create_from_response(reply))
    }

    /// Downloads server logs from a remotely executed action if any.
    // Java: com/google/devtools/build/lib/remote/RemoteExecutionService.java:2064 RemoteExecutionService.maybeDownloadServerLogs
    pub fn maybe_download_server_logs(
        &self,
        action: &RemoteAction<'_>,
        resp: &ExecuteResponse,
        log_dir: &Path,
    ) -> io::Result<ServerLogs> {
        blaze_util::check_state!(!self.shutdown.load(AtomicOrdering::SeqCst), "shutdown");
        let Some(combined_cache) = &self.combined_cache else {
            panic!("NullPointerException: combinedCache can't be null");
        };
        let mut server_logs = ServerLogs {
            log_count: 0,
            directory: log_dir.get_relative_str(action.get_action_id()),
            last_log_path: None,
        };

        let exit_code = resp.result.as_ref().map_or(0, |result| result.exit_code);
        let status_code = resp.status.as_ref().map_or(0, |status| status.code);
        if !resp.server_logs.is_empty() && (exit_code != 0 || status_code != tonic::Code::Ok as i32)
        {
            // PORT: `resp.getServerLogsMap().entrySet()`: the generated map field is the list of
            // its entries, in the order of the message.
            for e in &resp.server_logs {
                let value = e.value.clone().unwrap_or_default();
                if value.human_readable {
                    let last_log_path = server_logs.directory.get_relative_str(&e.key);
                    server_logs.last_log_path = Some(last_log_path.clone());
                    server_logs.log_count += 1;
                    get_from_future(combined_cache.download_file(
                        action.get_remote_action_execution_context(),
                        &last_log_path,
                        &value.digest.clone().unwrap_or_default(),
                    ))?;
                }
            }
        }

        Ok(server_logs)
    }

    /// `onBuildInterrupted(BuildInterruptedEvent event)`.
    // PORT: @Subscribe. The owner of the service calls this when the build is interrupted.
    // Java: com/google/devtools/build/lib/remote/RemoteExecutionService.java:2091 RemoteExecutionService.onBuildInterrupted
    pub fn on_build_interrupted(&self) {
        self.build_interrupted.store(true, AtomicOrdering::SeqCst);
    }

    /// `onBuildComplete(BuildCompleteEvent event)`; `success` is
    /// `event.getResult().getSuccess()`.
    // PORT: @Subscribe. The owner of the service calls this when the build completes.
    // Java: com/google/devtools/build/lib/remote/RemoteExecutionService.java:2096 RemoteExecutionService.onBuildComplete
    pub fn on_build_complete(&self, success: bool) {
        if success {
            // If build succeeded, clear knownMissingCasDigests in case there are missing digests
            // from other targets from previous builds which are not relevant anymore.
            self.known_missing_cas_digests.lock().clear();
        }
    }

    /// `onLostInputs(LostInputsEvent event)`; `missing_digests` is `event.missingDigests()`.
    // PORT: @Subscribe. The owner of the service calls this for a `LostInputsEvent`.
    // Java: com/google/devtools/build/lib/remote/RemoteExecutionService.java:2105 RemoteExecutionService.onLostInputs
    pub fn on_lost_inputs<S: AsRef<str>>(&self, missing_digests: impl IntoIterator<Item = S>) {
        for digest in missing_digests {
            self.known_missing_cas_digests
                .lock()
                .insert(DigestUtil::from_string(digest.as_ref()));
        }
    }

    /// Shuts the service down.
    // Java: com/google/devtools/build/lib/remote/RemoteExecutionService.java:2112 RemoteExecutionService.shutdown
    pub fn shutdown(&self) {
        if self
            .shutdown
            .compare_exchange(false, true, AtomicOrdering::SeqCst, AtomicOrdering::SeqCst)
            .is_err()
        {
            return;
        }

        if self.build_interrupted.load(AtomicOrdering::SeqCst) {
            self.background_task_executor.shutdown_now();
            if let Some(combined_cache) = &self.combined_cache {
                combined_cache.shutdown_now();
            }
            // `Thread.currentThread().interrupt();`
            blaze_shell::thread_interrupt::interrupt_current();
        }

        // Waits for all background tasks to finish and interrupts them if there is another
        // interrupt.
        //
        // PORT: the background tasks cannot be interrupted (see
        // `BackgroundTaskExecutor::shutdown_now`); this waits for them to end.
        self.background_task_executor.close();

        // Release the cache only after background tasks are done as they might be using it.
        if let Some(combined_cache) = &self.combined_cache {
            combined_cache.release();
        }

        if let Some(remote_executor) = &self.remote_executor {
            remote_executor.close();
        }
    }

    /// Shuts down the background task executor so that new task submissions are rejected.
    // Java: com/google/devtools/build/lib/remote/RemoteExecutionService.java:2140 RemoteExecutionService.shutdownBackgroundTaskExecutorForTesting
    pub fn shutdown_background_task_executor_for_testing(&self) {
        self.background_task_executor.shutdown();
    }

    /// Whether parameter files should be written locally, even when using remote execution or
    /// caching.
    // Java: com/google/devtools/build/lib/remote/RemoteExecutionService.java:2147 RemoteExecutionService.maybeWriteParamFilesLocally
    pub fn maybe_write_param_files_locally(&self, spawn: &dyn Spawn) -> io::Result<()> {
        if !self.execution_options.should_materialize_param_files() {
            return Ok(());
        }
        for action_input in spawn.get_input_files().to_list().iter() {
            if let Some(param_file_action_input) =
                action_input.as_any().downcast_ref::<ParamFileActionInput>()
            {
                VirtualActionInput::atomically_write_relative_to(
                    param_file_action_input,
                    &self.exec_root,
                )?;
            }
        }
        Ok(())
    }

    /// `report(evt)`.
    // Java: com/google/devtools/build/lib/remote/RemoteExecutionService.java:2158 RemoteExecutionService.report
    pub(crate) fn report(&self, evt: Event) {
        report(&*self.reporter, &self.reported_errors, evt);
    }

    // Java: com/google/devtools/build/lib/remote/RemoteExecutionService.java:2168 RemoteExecutionService.isScrubbedSpawn
    fn is_scrubbed_spawn(spawn: &dyn Spawn, scrubber: Option<&Scrubber>) -> bool {
        scrubber.is_some_and(|scrubber| scrubber.for_spawn(spawn).is_some())
    }
}
