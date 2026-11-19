//! Port of `lib/remote/RemoteSpawnRunner.java`: a client for the remote execution service.
//!
//! PORT notes for the whole file:
//! - Profiling (`Profiler`, `profileAccounting`, `logProfileTask`) is skipped (PORTING.md:
//!   profiling).
//! - The two `catch (CredentialHelperException e)` clauses have no counterpart: a
//!   `CredentialHelperException` (a `blaze_repo_fetch` `IoException` of kind
//!   `CredentialHelperException`) arrives over gRPC only as the cause of an UNAUTHENTICATED
//!   status, where these catches do not fire in Java either; the paths that throw it directly
//!   (HTTP cache, remote downloader) are not ported.
//!   `utils::create_exec_exception_for_credential_helper_exception` is the place they would
//!   call.
//! - `SpawnRunner::exec` has no `IOException` channel: an `IOException` that Java's `exec` throws
//!   is returned as `abstract_spawn_strategy::exec_io_exception(e)`, which is what
//!   `AbstractSpawnStrategy.exec` makes of it. Inside this file `IOException | ExecException |
//!   InterruptedException` is a `SpawnCacheError` (docs/remote.md, convention 2).
//! - `SpawnExecutionContext.getContext(RemoteLocalFallbackRegistry.class)` is
//!   `SpawnExecutionContext::get_remote_local_fallback_registry` (the command's
//!   `SpawnStrategyRegistry`); a harness whose context has none hands the registry to the runner
//!   (`RemoteSpawnRunner::set_remote_local_fallback_registry`), see [`RemoteLocalFallbackRegistry`].

use std::fmt;
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Instant;

use blaze_events::{Event, ExtendedEventHandler};
use blaze_exec::actions::spawn_strategy::SpawnStrategy;
use blaze_exec::actions::{
    ExecException, ExecKind, POSIX_TIMEOUT_EXIT_CODE, Spawn, SpawnMetrics, SpawnResult,
    SpawnResultBuilder, Status,
};
use blaze_exec::exec::abstract_spawn_strategy::exec_io_exception;
use blaze_exec::exec::action_execution_context::SpawnCacheError;
use blaze_exec::exec::spawn_strategy_registry::SpawnStrategyRegistry;
use blaze_exec::exec::{ProgressStatus, SpawnExecutionContext, SpawnRunner};
use blaze_optdefs::misc::remote_options::RemoteOptions;
use blaze_protos::JavaMessage;
use blaze_protos::failure_details::{self, FailureDetail, failure_detail};
use blaze_protos::google::protobuf::{Any, Timestamp};
use blaze_util::ExitCode;
use blaze_vfs::Path;
use prost::Message as _;

use crate::circuitbreaker::circuit_breaker_factory;
use crate::common::bulk_transfer_exception::BulkTransferException;
use crate::common::errors::{self, IoExceptionWithCause, StatusRuntimeException};
use crate::common::remote_execution_capabilities_exception::RemoteExecutionCapabilitiesException;
use crate::common::remote_execution_client::OperationObserver;
use crate::execute_retrier::ExecuteRetrier;
use crate::execution_status_exception::ExecutionStatusException;
use crate::merkletree::BlobPolicy;
use crate::reapi::longrunning::Operation;
use crate::reapi::rpc::RetryInfo;
use crate::reapi::{
    ExecuteOperationMetadata, ExecuteResponse, ExecutedActionMetadata, execution_stage,
};
use crate::remote_action::RemoteAction;
use crate::remote_execution_service::{RemoteActionResult, RemoteExecutionService};
use crate::remote_retrier::RemoteRetrier;
use crate::remote_retrier_utils;
use crate::retrier::RetryScheduler;
use crate::util::digest_util::DigestUtil;
use crate::util::utils::{
    self, create_exec_exception_from_remote_execution_capabilities_exception, create_spawn_result,
};

/// `RemoteLocalFallbackRegistry`: the action context that provides the strategy used for the
/// local fallback of remote execution.
///
/// Java's `RemoteSpawnRunner.execLocally` reads it with
/// `context.getContext(RemoteLocalFallbackRegistry.class)`
/// (`SpawnExecutionContext::get_remote_local_fallback_registry`); a test harness whose context
/// has no registry gives it to the runner with
/// [`RemoteSpawnRunner::set_remote_local_fallback_registry`]. Java's registry is the
/// `SpawnStrategyRegistry`, which implements this trait.
// Java: com/google/devtools/build/lib/exec/RemoteLocalFallbackRegistry.java:23 RemoteLocalFallbackRegistry
pub trait RemoteLocalFallbackRegistry: Send + Sync {
    /// Returns the remote local fallback strategy (an `AbstractSpawnStrategy`) or `None` if there
    /// is none.
    // Java: com/google/devtools/build/lib/exec/RemoteLocalFallbackRegistry.java:30 RemoteLocalFallbackRegistry.getRemoteLocalFallbackStrategy
    fn get_remote_local_fallback_strategy(
        &self,
        spawn: &dyn Spawn,
    ) -> Option<Arc<dyn SpawnStrategy>>;
}

impl RemoteLocalFallbackRegistry for SpawnStrategyRegistry {
    // Java: com/google/devtools/build/lib/exec/SpawnStrategyRegistry.java:177 SpawnStrategyRegistry.getRemoteLocalFallbackStrategy
    fn get_remote_local_fallback_strategy(
        &self,
        spawn: &dyn Spawn,
    ) -> Option<Arc<dyn SpawnStrategy>> {
        SpawnStrategyRegistry::get_remote_local_fallback_strategy(self, spawn)
    }
}

// Java: com/google/devtools/build/lib/remote/RemoteSpawnRunner.java:92 RemoteSpawnRunner.SPAWN_CHECKING_CACHE_EVENT
fn spawn_checking_cache_event() -> ProgressStatus {
    ProgressStatus::CheckingCache("remote".to_string())
}

// Java: com/google/devtools/build/lib/remote/RemoteSpawnRunner.java:95 RemoteSpawnRunner.SPAWN_SCHEDULING_EVENT
fn spawn_scheduling_event() -> ProgressStatus {
    ProgressStatus::Scheduling("remote".to_string())
}

// Java: com/google/devtools/build/lib/remote/RemoteSpawnRunner.java:98 RemoteSpawnRunner.SPAWN_EXECUTING_EVENT
fn spawn_executing_event() -> ProgressStatus {
    ProgressStatus::Executing("remote".to_string())
}

/// An `ExecException` thrown by the callable of `retrier.execute(...)`.
///
/// PORT: `Retrier::execute` takes a callable that fails with `io::Error` only; Java's callable
/// also throws `ExecException`, which the retrier rethrows as it is (`ExecuteRetrier`'s result
/// classifier finds no gRPC status in it, so it is never retried). The exception travels through
/// the retrier as this payload and is taken out again after it.
#[derive(Debug, Clone)]
struct ExecExceptionInRetrier(ExecException);

impl fmt::Display for ExecExceptionInRetrier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0, f)
    }
}

impl std::error::Error for ExecExceptionInRetrier {}

/// The callable's form of a `SpawnCacheError`: `IOException`s stay what they are, an
/// `ExecException` is carried as [`ExecExceptionInRetrier`] (`InterruptedException` is an
/// `io::Error` of kind `Interrupted`, which the retrier rethrows).
fn to_retrier_error(e: SpawnCacheError) -> io::Error {
    match e {
        SpawnCacheError::Io(e) => e,
        SpawnCacheError::Exec(ExecException::Interrupted) => errors::interrupted(),
        SpawnCacheError::Exec(e) => io::Error::other(ExecExceptionInRetrier(e)),
    }
}

/// `Stopwatch` (Guava).
struct Stopwatch {
    start: Instant,
    elapsed_when_stopped: Option<std::time::Duration>,
}

impl Stopwatch {
    /// `Stopwatch.createStarted()`.
    fn create_started() -> Stopwatch {
        Stopwatch {
            start: Instant::now(),
            elapsed_when_stopped: None,
        }
    }

    /// `stop()`.
    fn stop(&mut self) {
        blaze_util::check_state!(
            self.elapsed_when_stopped.is_none(),
            "This stopwatch is already stopped."
        );
        self.elapsed_when_stopped = Some(self.start.elapsed());
    }

    /// `elapsed()`, in nanoseconds (a `java.time.Duration`, which may take part in negative
    /// arithmetic).
    fn elapsed(&self) -> i128 {
        match self.elapsed_when_stopped {
            Some(elapsed) => elapsed.as_nanos() as i128,
            None => self.start.elapsed().as_nanos() as i128,
        }
    }
}

/// `java.time.Duration.toMillis()` of a duration in nanoseconds (truncates towards zero).
fn duration_to_millis(nanos: i128) -> i64 {
    (nanos / 1_000_000) as i64
}

/// `SpawnMetrics.Builder.toMs(duration)`: `Ints.saturatedCast(duration.toMillis())`.
// Java: com/google/devtools/build/lib/actions/SpawnMetrics.java:628 SpawnMetrics.Builder.toMs
fn to_ms(nanos: i128) -> i32 {
    duration_to_millis(nanos).clamp(i32::MIN as i64, i32::MAX as i64) as i32
}

/// `Instant.ofEpochSecond(t.getSeconds(), t.getNanos())` in nanoseconds since the epoch.
fn timestamp_nanos(t: &Timestamp) -> i128 {
    t.seconds as i128 * 1_000_000_000 + t.nanos as i128
}

/// An unset timestamp field reads as the default instance.
fn ts(t: &Option<Timestamp>) -> Timestamp {
    t.unwrap_or_default()
}

/// conversion utility for protobuf Timestamp difference to java.time.Duration
// Java: com/google/devtools/build/lib/remote/RemoteSpawnRunner.java:436 RemoteSpawnRunner.between
fn between(from: &Timestamp, to: &Timestamp) -> i128 {
    // Duration.ofNanos(Durations.toNanos(Timestamps.between(from, to)))
    timestamp_nanos(to) - timestamp_nanos(from)
}

/// `Any.is(clazz)`: whether the type name of the `Any`'s type URL is `full_name`.
// Java: com/google/protobuf/Any.java Any.is
fn any_is(any: &Any, full_name: &str) -> bool {
    // Any.getTypeNameFromTypeUrl
    let type_name = match any.type_url.rfind('/') {
        None => "",
        Some(pos) => &any.type_url[pos + 1..],
    };
    type_name == full_name
}

/// `ExecutingStatusReporter`: reports `SpawnExecutingEvent` once the server says the action is
/// executing.
// Java: com/google/devtools/build/lib/remote/RemoteSpawnRunner.java:139 RemoteSpawnRunner.ExecutingStatusReporter
pub struct ExecutingStatusReporter<'a> {
    reported_executing: AtomicBool,
    context: &'a dyn SpawnExecutionContext,
}

impl<'a> ExecutingStatusReporter<'a> {
    // Java: com/google/devtools/build/lib/remote/RemoteSpawnRunner.java:143 RemoteSpawnRunner.ExecutingStatusReporter.<init>
    pub fn new(context: &'a dyn SpawnExecutionContext) -> ExecutingStatusReporter<'a> {
        ExecutingStatusReporter {
            reported_executing: AtomicBool::new(false),
            context,
        }
    }

    // Java: com/google/devtools/build/lib/remote/RemoteSpawnRunner.java:164 RemoteSpawnRunner.ExecutingStatusReporter.reportExecuting
    pub fn report_executing(&self) {
        self.context.report(spawn_executing_event());
        self.reported_executing.store(true, Ordering::SeqCst);
    }

    // Java: com/google/devtools/build/lib/remote/RemoteSpawnRunner.java:169 RemoteSpawnRunner.ExecutingStatusReporter.reportExecutingIfNot
    pub fn report_executing_if_not(&self) {
        if !self.reported_executing.load(Ordering::SeqCst) {
            self.report_executing();
        }
    }
}

impl OperationObserver for ExecutingStatusReporter<'_> {
    // Java: com/google/devtools/build/lib/remote/RemoteSpawnRunner.java:148 RemoteSpawnRunner.ExecutingStatusReporter.onNext
    fn on_next(&self, o: &Operation) -> io::Result<()> {
        if !self.reported_executing.load(Ordering::SeqCst) {
            // `o.getMetadata()` is the default instance when the field is unset.
            let metadata_any = o.metadata.clone().unwrap_or_default();
            if any_is(
                &metadata_any,
                "build.bazel.remote.execution.v2.ExecuteOperationMetadata",
            ) {
                // `unpack` throws `InvalidProtocolBufferException` (an `IOException`).
                let metadata = ExecuteOperationMetadata::decode(metadata_any.value.as_slice())
                    .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
                if metadata.stage == execution_stage::Value::Executing as i32 {
                    self.report_executing();
                }
            } else {
                // If the server didn't return metadata, we can't know the accurate execution
                // status, so assuming that the action is accepted by the server and will be
                // executed ASAP.
                self.report_executing();
            }
        }
        Ok(())
    }
}

/// A client for the remote execution service.
// Java: com/google/devtools/build/lib/remote/RemoteSpawnRunner.java:90 RemoteSpawnRunner
pub struct RemoteSpawnRunner {
    remote_options: Arc<RemoteOptions>,
    verbose_failures: bool,
    cmdline_reporter: Option<Arc<dyn ExtendedEventHandler>>,
    retrier: RemoteRetrier,
    log_dir: Path,
    remote_execution_service: Arc<RemoteExecutionService>,
    digest_util: DigestUtil,

    // Used to ensure that a warning is reported only once.
    warning_reported: AtomicBool,

    /// See [`RemoteLocalFallbackRegistry`].
    remote_local_fallback_registry: OnceLock<Arc<dyn RemoteLocalFallbackRegistry>>,
}

impl fmt::Debug for RemoteSpawnRunner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RemoteSpawnRunner")
            .field("log_dir", &self.log_dir)
            .field("remote_execution_service", &self.remote_execution_service)
            .finish_non_exhaustive()
    }
}

impl RemoteSpawnRunner {
    /// `new RemoteSpawnRunner(remoteOptions, verboseFailures, cmdlineReporter, retryService,
    /// logDir, remoteExecutionService, digestUtil)`.
    // Java: com/google/devtools/build/lib/remote/RemoteSpawnRunner.java:112 RemoteSpawnRunner.<init>
    pub fn new(
        remote_options: Arc<RemoteOptions>,
        verbose_failures: bool,
        cmdline_reporter: Option<Arc<dyn ExtendedEventHandler>>,
        retry_service: Arc<dyn RetryScheduler>,
        log_dir: Path,
        remote_execution_service: Arc<RemoteExecutionService>,
        digest_util: DigestUtil,
    ) -> RemoteSpawnRunner {
        let retrier = Self::create_execute_retrier(&remote_options, retry_service);
        RemoteSpawnRunner {
            remote_options,
            verbose_failures,
            cmdline_reporter,
            retrier,
            log_dir,
            remote_execution_service,
            digest_util,
            warning_reported: AtomicBool::new(false),
            remote_local_fallback_registry: OnceLock::new(),
        }
    }

    /// Provides what Java reads with `context.getContext(RemoteLocalFallbackRegistry.class)`
    /// (see [`RemoteLocalFallbackRegistry`]). Set once.
    pub fn set_remote_local_fallback_registry(
        &self,
        registry: Arc<dyn RemoteLocalFallbackRegistry>,
    ) {
        if self.remote_local_fallback_registry.set(registry).is_err() {
            panic!("IllegalStateException: the RemoteLocalFallbackRegistry is already set");
        }
    }

    /// `getRemoteExecutionService()` (`@VisibleForTesting`).
    // Java: com/google/devtools/build/lib/remote/RemoteSpawnRunner.java:130 RemoteSpawnRunner.getRemoteExecutionService
    pub fn get_remote_execution_service(&self) -> &Arc<RemoteExecutionService> {
        &self.remote_execution_service
    }

    /// `exec(spawn, context)` with Java's exceptions (`IOException`, `ExecException`,
    /// `InterruptedException`).
    // Java: com/google/devtools/build/lib/remote/RemoteSpawnRunner.java:177 RemoteSpawnRunner.exec
    fn exec_impl(
        &self,
        spawn: &dyn Spawn,
        context: &dyn SpawnExecutionContext,
    ) -> Result<SpawnResult, SpawnCacheError> {
        blaze_util::check_argument!(
            self.remote_execution_service
                .may_be_executed_remotely(spawn),
            "Spawn can't be executed remotely. This is a bug."
        );

        let mut total_time = Stopwatch::create_started();
        let mut accept_cached_result = self
            .remote_execution_service
            .get_read_cache_policy(spawn)
            .allow_any_cache();
        let upload_local_results = self
            .remote_execution_service
            .get_write_cache_policy(spawn)
            .allow_any_cache();

        let action: RemoteAction<'_> = self
            .remote_execution_service
            .build_remote_action_with_blob_policy(
                spawn,
                context,
                if self.remote_options.remote_discard_merkle_trees {
                    BlobPolicy::Discard
                } else {
                    BlobPolicy::Keep
                },
            )?;

        context.set_digest(self.digest_util.as_spawn_log_proto(action.get_action_key()));

        let mut spawn_metrics = SpawnMetrics::for_exec(ExecKind::Remote);
        spawn_metrics.input_bytes = action.get_input_bytes();
        spawn_metrics.input_files = action.get_input_files();

        self.remote_execution_service
            .maybe_write_param_files_locally(spawn)?;

        // setParseTime(totalTime.elapsed())
        spawn_metrics.parse_time_in_ms = to_ms(total_time.elapsed());

        // PORT: skipped, profiling (`Profiler prof = Profiler.instance()`).
        let lookup: Result<Option<SpawnResult>, SpawnCacheError> = 'try_: {
            context.report(spawn_checking_cache_event());

            // Try to lookup the action in the action cache.
            let cached_result: Option<RemoteActionResult> = if accept_cached_result {
                match self.remote_execution_service.lookup_cache(&action) {
                    Ok(cached_result) => cached_result,
                    Err(e) => break 'try_ Err(e.into()),
                }
            } else {
                None
            };

            if let Some(cached_result) = cached_result {
                if cached_result.get_exit_code() != 0
                    || cached_result
                        .maybe_get_missing_mandatory_output(&action)
                        .is_some()
                {
                    // Failed actions are treated as a cache miss mostly in order to avoid
                    // caching flaky actions (tests).
                    // Set acceptCachedResult to false in order to force the action re-execution
                    accept_cached_result = false;
                } else {
                    // `cachedResult.cacheName()`; Java's string concatenation prints a null name
                    // as "null".
                    let cache_name = cached_result.cache_name().unwrap_or("null").to_string();
                    match self.download_and_finalize_spawn_result(
                        &action,
                        &cached_result,
                        /* cache_hit= */ true,
                        &cache_name,
                        spawn,
                        &mut total_time,
                        &|| action.get_network_time().get_duration(),
                        &mut spawn_metrics,
                    ) {
                        Ok(result) => break 'try_ Ok(Some(result)),
                        Err(SpawnCacheError::Io(e)) => {
                            match errors::downcast_ref::<BulkTransferException>(&e) {
                                Some(bulk) if bulk.all_caused_by_cache_not_found_exception() => {
                                    // No cache hit, so we fall through to local or remote
                                    // execution. We set acceptCachedResult to false in order to
                                    // force the action re-execution.
                                    accept_cached_result = false;
                                }
                                // `if (!e.allCausedByCacheNotFoundException()) throw e;` and the
                                // other `IOException`s.
                                _ => break 'try_ Err(SpawnCacheError::Io(e)),
                            }
                        }
                        Err(e) => break 'try_ Err(e),
                    }
                }
            }
            Ok(None)
        };
        match lookup {
            Ok(Some(result)) => return Ok(result),
            Ok(None) => {}
            // PORT: `catch (CredentialHelperException e)` has no port (see the module comment).
            Err(SpawnCacheError::Io(e)) => {
                // `InterruptedException` is not an `IOException`: it propagates.
                if errors::is_interrupted(&e) {
                    return Err(e.into());
                }
                return self.exec_locally_and_upload_or_fail(
                    &action,
                    spawn,
                    context,
                    upload_local_results,
                    e,
                );
            }
            Err(e) => return Err(e),
        }

        if self.remote_options.remote_require_cached {
            let mut builder = SpawnResultBuilder::new();
            builder
                .set_status(Status::ExecutionDenied)
                .set_exit_code(1)
                .set_failure_message(
                    "Action must be cached due to --experimental_remote_require_cached but it is not",
                )
                .set_failure_detail(FailureDetail {
                    message: String::new(),
                    category: Some(failure_detail::Category::Spawn(failure_details::Spawn {
                        code: failure_details::spawn::Code::ExecutionDenied as i32,
                        ..Default::default()
                    })),
                })
                .set_runner_name("remote");
            return Ok(builder.build());
        }

        let use_cached_result = AtomicBool::new(accept_cached_result);
        let force_upload_input = AtomicBool::new(false);
        // PORT: whether the last attempt of the callable failed with the unchecked
        // `StatusRuntimeException` it throws itself (see below).
        let threw_status_runtime_exception = AtomicBool::new(false);
        let retried: io::Result<SpawnResult> =
            self.retrier.execute(|| -> io::Result<SpawnResult> {
                threw_status_runtime_exception.store(false, Ordering::SeqCst);
                // Upload the command and all the inputs into the remote cache.
                // PORT: skipped, profiling (`prof.profile(UPLOAD_TIME, "upload missing inputs")`).
                {
                    let network_time_start = action.get_network_time().get_duration();
                    let upload_time = Stopwatch::create_started();
                    // Upon retry, we force upload inputs
                    match self.remote_execution_service.upload_inputs_if_not_present(
                        &action,
                        force_upload_input.swap(true, Ordering::SeqCst),
                    ) {
                        Ok(()) => {}
                        Err(SpawnCacheError::Io(e)) => {
                            if let Some(bulk) = errors::downcast_ref::<BulkTransferException>(&e) {
                                // An input that is missing from the remote cache and not available
                                // locally can only be regenerated reliably by action rewinding or an
                                // invocation retry. Unwrap the LostInputsExecException, which is not
                                // classified as retryable.
                                bulk.get_lost_artifacts(&|exec_path| {
                                    context.get_input_metadata_provider().get_input(exec_path)
                                })
                                .throw_if_not_empty();
                            }
                            return Err(e);
                        }
                        Err(e) => return Err(to_retrier_error(e)),
                    }

                    // subtract network time consumed here to ensure wall clock during upload is not
                    // double
                    // counted, and metrics time computation does not exceed total time
                    let network_time_now = action.get_network_time().get_duration();
                    spawn_metrics.upload_time_in_ms = to_ms(
                        upload_time.elapsed()
                            - (network_time_now.as_nanos() as i128
                                - network_time_start.as_nanos() as i128),
                    );
                }

                context.report(spawn_scheduling_event());

                let reporter = ExecutingStatusReporter::new(context);
                // PORT: skipped, profiling (`clampTimeNanos`, `prof.profile(REMOTE_EXECUTION,
                // "execute remotely")`).
                let result: RemoteActionResult = self.remote_execution_service.execute_remotely(
                    &action,
                    use_cached_result.load(Ordering::SeqCst),
                    &reporter,
                )?;
                // In case of replies from server contains metadata, but none of them has EXECUTING
                // status.
                // It's already late at this stage, but we should at least report once.
                reporter.report_executing_if_not();

                if result.cache_hit()
                    && (!result.success()
                        || result.maybe_get_missing_mandatory_output(&action).is_some())
                {
                    // Instead of failing in downloadAndFinalizeSpawnResult, retry with forced
                    // execution.
                    use_cached_result.store(false, Ordering::SeqCst);
                    let status = crate::reapi::rpc::Status {
                        code: tonic::Code::NotFound as i32,
                        message: String::new(),
                        details: vec![Any {
                            type_url: "type.googleapis.com/google.rpc.RetryInfo".to_string(),
                            value: RetryInfo::default().encode_to_vec(),
                        }],
                    };
                    // StatusProto.toStatusRuntimeException(status): the status proto travels in the
                    // trailers (tonic: the details).
                    threw_status_runtime_exception.store(true, Ordering::SeqCst);
                    return Err(io::Error::other(StatusRuntimeException::new(
                        tonic::Status::with_details(
                            tonic::Code::NotFound,
                            String::new(),
                            bytes::Bytes::from(status.to_byte_array()),
                        ),
                    )));
                }

                self.maybe_print_execution_messages(
                    context,
                    result.get_message(),
                    result.success(),
                );

                // PORT: skipped, profiling (`profileAccounting(clampTimeNanos,
                // result.getExecutionMetadata())`).
                Self::spawn_metrics_accounting(
                    &mut spawn_metrics,
                    &result.get_execution_metadata(),
                );

                // PORT: skipped, profiling (`prof.profile(REMOTE_DOWNLOAD, "download server logs")`).
                self.maybe_download_server_logs(
                    &action,
                    result
                        .get_response()
                        .expect("the result of executeRemotely has a response"),
                )?;

                match self.download_and_finalize_spawn_result(
                    &action,
                    &result,
                    result.cache_hit(),
                    &self.get_name(),
                    spawn,
                    &mut total_time,
                    &|| action.get_network_time().get_duration(),
                    &mut spawn_metrics,
                ) {
                    Ok(spawn_result) => Ok(spawn_result),
                    Err(SpawnCacheError::Io(e)) => {
                        if let Some(bulk) = errors::downcast_ref::<BulkTransferException>(&e)
                            && bulk.all_caused_by_cache_not_found_exception()
                        {
                            // No cache hit, so if we retry this execution, we must no longer accept
                            // cached results, it must be reexecuted
                            use_cached_result.store(false, Ordering::SeqCst);
                        }
                        Err(e)
                    }
                    Err(e) => Err(to_retrier_error(e)),
                }
            });
        match retried {
            Ok(result) => Ok(result),
            Err(e) => {
                if let Some(ExecExceptionInRetrier(exec_exception)) =
                    errors::downcast_ref::<ExecExceptionInRetrier>(&e)
                {
                    return Err(exec_exception.clone().into());
                }
                if threw_status_runtime_exception.load(Ordering::SeqCst) {
                    // PORT: Java's callable throws the unchecked `StatusRuntimeException`, which
                    // no `catch` of `exec` handles once the retrier gives up: it escapes `exec`
                    // (a crash).
                    panic!("{}", e);
                }
                // `InterruptedException` is not an `IOException`: it propagates.
                if errors::is_interrupted(&e) {
                    return Err(e.into());
                }
                // PORT: `catch (CredentialHelperException e)` has no port (see the module
                // comment).
                self.exec_locally_and_upload_or_fail(
                    &action,
                    spawn,
                    context,
                    upload_local_results,
                    e,
                )
            }
        }
    }

    /// `spawnMetricsAccounting(spawnMetrics, executionMetadata)` (`@VisibleForTesting`).
    ///
    /// PORT: `SpawnMetrics.Builder` is the `SpawnMetrics` struct itself (its fields are the
    /// builder's fields).
    // Java: com/google/devtools/build/lib/remote/RemoteSpawnRunner.java:441 RemoteSpawnRunner.spawnMetricsAccounting
    pub fn spawn_metrics_accounting(
        spawn_metrics: &mut SpawnMetrics,
        execution_metadata: &ExecutedActionMetadata,
    ) {
        // Expect that a non-empty worker indicates that all fields are populated.
        // If the bounded sides of these checkpoints are default timestamps, i.e. unset,
        // the phase durations can be extremely large. Unset pairs, or a fully unset
        // collection of timestamps, will result in zeroed durations, and no metrics
        // contributions for a phase or phases.
        if !execution_metadata.worker.is_empty() {
            // Accumulate queueTime from any previous attempts
            let remote_queue_time = between(
                &ts(&execution_metadata.queued_timestamp),
                &ts(&execution_metadata.worker_start_timestamp),
            );
            // addQueueTime(remoteQueueTime)
            spawn_metrics.queue_time_in_ms = spawn_metrics
                .queue_time_in_ms
                .wrapping_add(to_ms(remote_queue_time));
            // setup time does not include failed attempts
            let setup_time = between(
                &ts(&execution_metadata.worker_start_timestamp),
                &ts(&execution_metadata.execution_start_timestamp),
            );
            spawn_metrics.setup_time_in_ms = to_ms(setup_time);
            // execution time is unspecified for failures
            let execution_wall_time = between(
                &ts(&execution_metadata.execution_start_timestamp),
                &ts(&execution_metadata.execution_completed_timestamp),
            );
            spawn_metrics.execution_wall_time_in_ms = to_ms(execution_wall_time);
            // remoteProcessOutputs time is unspecified for failures
            let remote_process_outputs_time = between(
                &ts(&execution_metadata.output_upload_start_timestamp),
                &ts(&execution_metadata.output_upload_completed_timestamp),
            );
            spawn_metrics.process_outputs_time_in_ms = to_ms(remote_process_outputs_time);
        }
    }

    // Java: com/google/devtools/build/lib/remote/RemoteSpawnRunner.java:475 RemoteSpawnRunner.downloadAndFinalizeSpawnResult
    fn download_and_finalize_spawn_result(
        &self,
        action: &RemoteAction<'_>,
        result: &RemoteActionResult,
        cache_hit: bool,
        cache_name: &str,
        spawn: &dyn Spawn,
        total_time: &mut Stopwatch,
        network_time: &dyn Fn() -> std::time::Duration,
        spawn_metrics: &mut SpawnMetrics,
    ) -> Result<SpawnResult, SpawnCacheError> {
        let network_time_start = network_time();
        let mut fetch_time = Stopwatch::create_started();

        // PORT: skipped, profiling (`profile(REMOTE_DOWNLOAD, "download outputs")`).
        let in_memory_output = self
            .remote_execution_service
            .download_outputs(action, result)?;

        fetch_time.stop();
        total_time.stop();
        let network_time_end = network_time();
        // subtract network time consumed here to ensure wall clock during fetch is not double
        // counted, and metrics time computation does not exceed total time
        let execution_metadata = result.get_execution_metadata();
        // `(int) duration.toMillis()`: a narrowing cast.
        spawn_metrics.fetch_time_in_ms = duration_to_millis(
            fetch_time.elapsed()
                - (network_time_end.as_nanos() as i128 - network_time_start.as_nanos() as i128),
        ) as i32;
        spawn_metrics.total_time_in_ms = duration_to_millis(total_time.elapsed()) as i32;
        spawn_metrics.network_time_in_ms =
            duration_to_millis(network_time_end.as_nanos() as i128) as i32;
        Ok(create_spawn_result(
            &self.digest_util,
            action.get_action_key(),
            result.get_exit_code(),
            cache_hit,
            cache_name,
            in_memory_output,
            ts(&execution_metadata.execution_start_timestamp),
            ts(&execution_metadata.execution_completed_timestamp),
            spawn_metrics.clone(),
            &spawn.get_mnemonic(),
        ))
    }

    // Java: com/google/devtools/build/lib/remote/RemoteSpawnRunner.java:526 RemoteSpawnRunner.maybePrintExecutionMessages
    fn maybe_print_execution_messages(
        &self,
        context: &dyn SpawnExecutionContext,
        message: &str,
        success: bool,
    ) {
        let out_err = context.get_file_out_err();
        let print_message = self
            .remote_options
            .remote_print_execution_messages
            .should_print_messages(success)
            && !message.is_empty();
        if print_message {
            // `outErr.printErr(...)`: a `PrintWriter`, which swallows I/O errors.
            let _ = out_err
                .write_err(format!("Remote server execution message: {message}\n").as_bytes());
        }
    }

    /// `maybeDownloadServerLogs(action, resp)`; fails only with an `InterruptedException`.
    // Java: com/google/devtools/build/lib/remote/RemoteSpawnRunner.java:537 RemoteSpawnRunner.maybeDownloadServerLogs
    fn maybe_download_server_logs(
        &self,
        action: &RemoteAction<'_>,
        resp: &ExecuteResponse,
    ) -> io::Result<()> {
        match self
            .remote_execution_service
            .maybe_download_server_logs(action, resp, &self.log_dir)
        {
            Ok(server_logs) => {
                if server_logs.log_count > 0 && self.verbose_failures {
                    let path = if server_logs.log_count > 1 {
                        server_logs.directory.to_string()
                    } else {
                        match &server_logs.last_log_path {
                            Some(last_log_path) => last_log_path.to_string(),
                            None => "null".to_string(),
                        }
                    };
                    self.report(Event::info(format!(
                        "Remote server log of failing action:\n   {path}"
                    )));
                }
                Ok(())
            }
            Err(e) if errors::is_interrupted(&e) => Err(e),
            Err(_e) => {
                self.report_once(Event::warn(
                    "Failed downloading server logs from the remote cache.",
                ));
                Ok(())
            }
        }
    }

    // Java: com/google/devtools/build/lib/remote/RemoteSpawnRunner.java:552 RemoteSpawnRunner.execLocally
    fn exec_locally(
        &self,
        spawn: &dyn Spawn,
        context: &dyn SpawnExecutionContext,
    ) -> Result<SpawnResult, SpawnCacheError> {
        let local_fallback_registry: Arc<dyn RemoteLocalFallbackRegistry> = match context
            .get_remote_local_fallback_registry()
        {
            Some(registry) => registry,
            None => match self.remote_local_fallback_registry.get() {
                Some(registry) => registry.clone(),
                None => panic!(
                    "NullPointerException: Expected a RemoteLocalFallbackRegistry to be registered"
                ),
            },
        };
        let remote_local_fallback_strategy = local_fallback_registry
            .get_remote_local_fallback_strategy(spawn)
            .unwrap_or_else(|| {
                panic!(
                    "NullPointerException: A remote local fallback strategy must be set if using remote fallback."
                )
            });
        let Some(remote_local_fallback_strategy) =
            remote_local_fallback_strategy.as_abstract_spawn_strategy()
        else {
            panic!(
                "ClassCastException: the remote local fallback strategy is not an AbstractSpawnStrategy"
            );
        };
        Ok(remote_local_fallback_strategy
            .get_spawn_runner()
            .exec(spawn, context)?)
    }

    // Java: com/google/devtools/build/lib/remote/RemoteSpawnRunner.java:565 RemoteSpawnRunner.execLocallyAndUploadOrFail
    fn exec_locally_and_upload_or_fail(
        &self,
        action: &RemoteAction<'_>,
        spawn: &dyn Spawn,
        context: &dyn SpawnExecutionContext,
        upload_local_results: bool,
        cause: io::Error,
    ) -> Result<SpawnResult, SpawnCacheError> {
        // Regardless of cause, if we are interrupted, we should stop without displaying a
        // user-visible failure/stack trace.
        if blaze_shell::thread_interrupt::interrupted() {
            return Err(ExecException::Interrupted.into());
        }
        // If the failure is caused by eviction of inputs to the current action that are only
        // available remotely, try to regenerate the lost inputs.
        if let Some(e) = errors::downcast_ref::<BulkTransferException>(&cause) {
            e.get_lost_artifacts(&|exec_path| {
                context.get_input_metadata_provider().get_input(exec_path)
            })
            .throw_if_not_empty();
        }
        if self.remote_options.remote_local_fallback
            && !remote_retrier_utils::caused_by_exec_timeout(&cause)
        {
            return self.exec_locally_and_upload(action, spawn, context, upload_local_results);
        }
        self.handle_error(action, cause, context)
    }

    /// `exception.getCause() instanceof ExecutionStatusException`: the exception of
    /// `new IOException(executionStatusException)` or `new IOException(message,
    /// executionStatusException)`.
    fn get_cause_execution_status_exception(
        exception: &io::Error,
    ) -> Option<&ExecutionStatusException> {
        if let Some(e) = errors::downcast_ref::<ExecutionStatusException>(exception) {
            return Some(e);
        }
        errors::downcast_ref::<IoExceptionWithCause>(exception).and_then(|with_cause| {
            errors::downcast_ref::<ExecutionStatusException>(with_cause.get_cause())
        })
    }

    // Java: com/google/devtools/build/lib/remote/RemoteSpawnRunner.java:588 RemoteSpawnRunner.handleError
    fn handle_error(
        &self,
        action: &RemoteAction<'_>,
        exception: io::Error,
        context: &dyn SpawnExecutionContext,
    ) -> Result<SpawnResult, SpawnCacheError> {
        if let Some(e) = errors::downcast_ref::<RemoteExecutionCapabilitiesException>(&exception) {
            return Err(
                create_exec_exception_from_remote_execution_capabilities_exception(e).into(),
            );
        }
        if let Some(e) = Self::get_cause_execution_status_exception(&exception) {
            let mut result: Option<RemoteActionResult> = None;
            if let Some(resp) = e.get_response() {
                self.maybe_download_server_logs(action, resp)?;
                if resp.result.is_some() {
                    let created = RemoteActionResult::create_from_response(resp.clone());
                    match self
                        .remote_execution_service
                        .download_outputs(action, &created)
                    {
                        Ok(_) => {}
                        Err(SpawnCacheError::Io(bulk_transfer_ex))
                            if errors::downcast_ref::<BulkTransferException>(&bulk_transfer_ex)
                                .is_some() =>
                        {
                            // `exception.addSuppressed(bulkTransferEx)`.
                            // PORT: `io::Error` has no suppressed exceptions; they only show in
                            // stack traces, which Blaze does not print.
                        }
                        Err(other) => return Err(other),
                    }
                    result = Some(created);
                }
            }
            if e.is_execution_timeout() {
                self.maybe_print_execution_messages(
                    context,
                    &e.get_response()
                        .expect("an execution timeout has a response")
                        .message,
                    /* success= */ false,
                );
                let mut result_builder = SpawnResultBuilder::new();
                result_builder
                    .set_runner_name(&self.get_name())
                    .set_status(Status::Timeout)
                    .set_exit_code(POSIX_TIMEOUT_EXIT_CODE)
                    .set_failure_detail(FailureDetail {
                        message: "remote spawn timed out".to_string(),
                        category: Some(failure_detail::Category::Spawn(failure_details::Spawn {
                            code: failure_details::spawn::Code::Timeout as i32,
                            ..Default::default()
                        })),
                    });
                if let Some(result) = &result {
                    let execution_metadata = result.get_execution_metadata();
                    let start = ts(&execution_metadata.execution_start_timestamp);
                    let completed = ts(&execution_metadata.execution_completed_timestamp);
                    result_builder
                        .set_wall_time_in_ms(
                            // `(int) Duration.between(start, completed).toMillis()`
                            duration_to_millis(
                                timestamp_nanos(&completed) - timestamp_nanos(&start),
                            ) as i32,
                        )
                        .set_start_time(utils::timestamp_to_instant(start));
                }
                return Ok(result_builder.build());
            }
        }
        let status: Status;
        let detailed_code: failure_details::spawn::Code;
        let catastrophe: bool;
        if remote_retrier_utils::caused_by_status(&exception, tonic::Code::Unavailable) {
            status = Status::ExecutionFailedCatastrophically;
            detailed_code = failure_details::spawn::Code::ExecutionFailed;
            catastrophe = true;
        } else if BulkTransferException::all_caused_by_cache_not_found_exception_of(&exception) {
            // At this point, cache evictions that affect uploaded inputs have already been
            // handled. Cache evictions that affect the outputs of the current actions have also
            // been retried with a request that disallows reusing cached results. This means that
            // there is no point in retrying the entire build.
            status = Status::RemoteCacheFailed;
            detailed_code = failure_details::spawn::Code::RemoteCacheFailed;
            catastrophe = false;
        } else {
            status = Status::ExecutionFailed;
            detailed_code = failure_details::spawn::Code::ExecutionFailed;
            catastrophe = false;
        }

        let mut error_message = utils::grpc_aware_error_message(&exception, self.verbose_failures);

        if let Some(e) = Self::get_cause_execution_status_exception(&exception)
            && let Some(response) = e.get_response()
            && !response.message.is_empty()
        {
            error_message.push('\n');
            error_message.push_str(&response.message);
        }

        let mut builder = SpawnResultBuilder::new();
        builder
            .set_runner_name(&self.get_name())
            .set_status(status)
            .set_exit_code(ExitCode::REMOTE_ERROR.get_numeric_exit_code())
            .set_failure_message(&error_message)
            .set_failure_detail(FailureDetail {
                message: format!("remote spawn failed: {error_message}"),
                category: Some(failure_detail::Category::Spawn(failure_details::Spawn {
                    code: detailed_code as i32,
                    catastrophic: catastrophe,
                    ..Default::default()
                })),
            });
        Ok(builder.build())
    }

    /// `execLocallyAndUpload(action, spawn, context, uploadLocalResults)`
    /// (`@VisibleForTesting`).
    // Java: com/google/devtools/build/lib/remote/RemoteSpawnRunner.java:687 RemoteSpawnRunner.execLocallyAndUpload
    pub fn exec_locally_and_upload(
        &self,
        action: &RemoteAction<'_>,
        spawn: &dyn Spawn,
        context: &dyn SpawnExecutionContext,
        upload_local_results: bool,
    ) -> Result<SpawnResult, SpawnCacheError> {
        let result = self.exec_locally(spawn, context)?;
        if upload_local_results && Status::Success == result.status() && result.exit_code() == 0 {
            // `uploadOutputs(action, result, () -> {}, remoteOptions.guardAgainstConcurrentChanges)`
            self.remote_execution_service
                .upload_outputs_with_concurrent_changes_check_level(
                    action,
                    &result,
                    Box::new(|| {}),
                    self.remote_options.guard_against_concurrent_changes,
                )?;
        }
        Ok(result)
    }

    // Java: com/google/devtools/build/lib/remote/RemoteSpawnRunner.java:698 RemoteSpawnRunner.reportOnce
    fn report_once(&self, evt: Event) {
        if self
            .warning_reported
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
        {
            self.report(evt);
        }
    }

    // Java: com/google/devtools/build/lib/remote/RemoteSpawnRunner.java:704 RemoteSpawnRunner.report
    fn report(&self, evt: Event) {
        if let Some(cmdline_reporter) = &self.cmdline_reporter {
            cmdline_reporter.handle(&evt);
        }
    }

    // Java: com/google/devtools/build/lib/remote/RemoteSpawnRunner.java:710 RemoteSpawnRunner.createExecuteRetrier
    fn create_execute_retrier(
        options: &RemoteOptions,
        retry_service: Arc<dyn RetryScheduler>,
    ) -> RemoteRetrier {
        ExecuteRetrier::new(
            options.remote_max_retry_attempts,
            retry_service,
            circuit_breaker_factory::create_circuit_breaker(options),
        )
        .into_remote_retrier()
    }
}

impl SpawnRunner for RemoteSpawnRunner {
    // Java: com/google/devtools/build/lib/remote/RemoteSpawnRunner.java:177 RemoteSpawnRunner.exec
    fn exec(
        &self,
        spawn: &dyn Spawn,
        context: &dyn SpawnExecutionContext,
    ) -> Result<SpawnResult, ExecException> {
        match self.exec_impl(spawn, context) {
            Ok(result) => Ok(result),
            Err(SpawnCacheError::Exec(e)) => Err(e),
            // The `IOException` that `AbstractSpawnStrategy.exec` turns into an
            // `EnvironmentalExecException` (see the module comment).
            Err(SpawnCacheError::Io(e)) => Err(exec_io_exception(&e)),
        }
    }

    // Java: com/google/devtools/build/lib/remote/RemoteSpawnRunner.java:517 RemoteSpawnRunner.canExec
    fn can_exec(&self, spawn: &dyn Spawn) -> bool {
        self.remote_execution_service
            .may_be_executed_remotely(spawn)
    }

    // Java: com/google/devtools/build/lib/remote/RemoteSpawnRunner.java:522 RemoteSpawnRunner.handlesCaching
    fn handles_caching(&self) -> bool {
        true
    }

    // Java: com/google/devtools/build/lib/remote/RemoteSpawnRunner.java:135 RemoteSpawnRunner.getName
    fn get_name(&self) -> String {
        "remote".to_string()
    }
}
