//! Port of `lib/remote/RemoteSpawnCache.java`.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use blaze_events::Event;
use blaze_exec::actions::{ExecKind, Spawn, SpawnMetrics, SpawnResult};
use blaze_exec::exec::SpawnExecutionContext;
use blaze_exec::exec::action_execution_context::{
    self as spawn_cache, CacheHandle, NoResultNoStore, SpawnCache, SpawnCacheError,
};
use blaze_optdefs::misc::remote_options::RemoteOptions;
use parking_lot::Mutex;

use crate::common::bulk_transfer_exception::BulkTransferException;
use crate::common::cache_not_found_exception::CacheNotFoundException;
use crate::common::errors;
use crate::common::remote_cache_client::ActionKey;
use crate::common::remote_execution_capabilities_exception::RemoteExecutionCapabilitiesException;
use crate::merkletree::BlobPolicy;
use crate::remote_action::RemoteAction;
use crate::remote_execution_service::RemoteExecutionService;
use crate::remote_execution_service_download::LocalExecution;
use crate::util::digest_util::DigestUtil;
use crate::util::utils::{
    self, create_exec_exception_from_remote_execution_capabilities_exception, create_spawn_result,
};

/// `ConcurrentHashMap<RemoteCacheClient.ActionKey, LocalExecution> inFlightExecutions`.
///
/// PORT: shared (`Arc`) with the `onClose` callbacks of the `LocalExecution`s, which remove their
/// entry (Java: a lambda capturing `this`).
type InFlightExecutions = Arc<Mutex<HashMap<ActionKey, Arc<LocalExecution>>>>;

/// A remote `SpawnCache` implementation.
// Java: com/google/devtools/build/lib/remote/RemoteSpawnCache.java:53 RemoteSpawnCache
pub struct RemoteSpawnCache {
    options: Arc<RemoteOptions>,
    remote_execution_service: Arc<RemoteExecutionService>,
    digest_util: DigestUtil,
    verbose_failures: bool,
    in_flight_executions: InFlightExecutions,
}

impl std::fmt::Debug for RemoteSpawnCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RemoteSpawnCache")
            .field("verbose_failures", &self.verbose_failures)
            .finish_non_exhaustive()
    }
}

/// `Ints.saturatedCast(duration.toMillis())` (`SpawnMetrics.Builder.toMs`).
// Java: com/google/devtools/build/lib/actions/SpawnMetrics.java:628 SpawnMetrics.Builder.toMs
fn to_ms(duration: Duration) -> i32 {
    i32::try_from(duration.as_millis()).unwrap_or(i32::MAX)
}

impl RemoteSpawnCache {
    /// `new RemoteSpawnCache(options, verboseFailures, remoteExecutionService, digestUtil)`.
    // Java: com/google/devtools/build/lib/remote/RemoteSpawnCache.java:62 RemoteSpawnCache.<init>
    pub fn new(
        options: Arc<RemoteOptions>,
        verbose_failures: bool,
        remote_execution_service: Arc<RemoteExecutionService>,
        digest_util: DigestUtil,
    ) -> RemoteSpawnCache {
        RemoteSpawnCache {
            options,
            verbose_failures,
            remote_execution_service,
            digest_util,
            in_flight_executions: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// `getRemoteExecutionService()` (`@VisibleForTesting`).
    // Java: com/google/devtools/build/lib/remote/RemoteSpawnCache.java:74 RemoteSpawnCache.getRemoteExecutionService
    pub fn get_remote_execution_service(&self) -> &Arc<RemoteExecutionService> {
        &self.remote_execution_service
    }

    /// `getInFlightExecutionsSize()` (`@VisibleForTesting`).
    // Java: com/google/devtools/build/lib/remote/RemoteSpawnCache.java:79 RemoteSpawnCache.getInFlightExecutionsSize
    pub fn get_in_flight_executions_size(&self) -> usize {
        self.in_flight_executions.lock().len()
    }

    /// The part of `lookup` inside `if (shouldAcceptCachedResult) { try { ... } finally { ... } }`
    /// up to (not including) the `finally` block. `Some(handle)` is a `return` from inside the
    /// block.
    // Java: com/google/devtools/build/lib/remote/RemoteSpawnCache.java:108 RemoteSpawnCache.lookup
    fn lookup_accepting_cached_result(
        &self,
        spawn: &dyn Spawn,
        action: &RemoteAction<'_>,
        should_upload_local_results: bool,
        spawn_metrics: &mut SpawnMetrics,
        total_time: Instant,
        this_execution: &mut Option<Arc<LocalExecution>>,
        previous_execution: &mut Option<Arc<LocalExecution>>,
    ) -> Result<Option<Box<dyn CacheHandle>>, SpawnCacheError> {
        let in_flight_executions = self.in_flight_executions.clone();
        let action_key = action.get_action_key().clone();
        *this_execution = LocalExecution::create_if_deduplicatable(
            action,
            Box::new(move || {
                in_flight_executions.lock().remove(&action_key);
            }),
        );
        if should_upload_local_results && let Some(this) = this_execution.clone() {
            // `inFlightExecutions.merge(action.getActionKey(), thisExecution, ...)`: the
            // remapping function runs under the map's lock.
            let previous_or_this_execution: Arc<LocalExecution> = {
                let mut in_flight_executions = self.in_flight_executions.lock();
                let existing_execution = in_flight_executions.get(action.get_action_key()).cloned();
                match existing_execution {
                    Some(existing_execution) => {
                        if existing_execution.register_for_output_reuse() {
                            existing_execution
                        } else {
                            // The existing execution has completed and its results may have
                            // already been modified by its action, so we can't deduplicate
                            // against it. Instead, start a new in-flight execution.
                            in_flight_executions
                                .insert(action.get_action_key().clone(), this.clone());
                            this.clone()
                        }
                    }
                    None => {
                        in_flight_executions.insert(action.get_action_key().clone(), this.clone());
                        this.clone()
                    }
                }
            };
            if !Arc::ptr_eq(&previous_or_this_execution, &this) {
                // The current execution is not the first one to be registered for this action
                // key, so we need to wait for the previous one to finish before we can reuse its
                // result.
                *previous_execution = Some(previous_or_this_execution);
                *this_execution = None;
            }
        }

        match self.try_cache_hit(
            spawn,
            action,
            this_execution.as_deref(),
            spawn_metrics,
            total_time,
        ) {
            Ok(Some(spawn_result)) => return Ok(Some(spawn_cache::success(spawn_result))),
            Ok(None) => {}
            // `ExecException` and `InterruptedException` are not caught.
            Err(SpawnCacheError::Exec(e)) => return Err(SpawnCacheError::Exec(e)),
            Err(SpawnCacheError::Io(e)) if errors::is_interrupted(&e) => {
                return Err(SpawnCacheError::Io(e));
            }
            Err(SpawnCacheError::Io(e)) => {
                if errors::downcast_ref::<CacheNotFoundException>(&e).is_some() {
                    // Intentionally left blank
                } else if let Some(e) =
                    errors::downcast_ref::<RemoteExecutionCapabilitiesException>(&e)
                {
                    // PORT: `catch (CredentialHelperException e) { if (thisExecution != null) {
                    // thisExecution.close(); } throw
                    // createExecExceptionForCredentialHelperException(e); }` has no port:
                    // `CredentialHelperException` (lib/authandtls/credentialhelper) is not
                    // ported, so no error is one.
                    let should_local_fallback = self.options.remote_local_fallback_for_remote_cache
                        && self.options.remote_local_fallback;
                    if !should_local_fallback {
                        if let Some(this_execution) = this_execution {
                            this_execution.close();
                        }
                        return Err(
                            create_exec_exception_from_remote_execution_capabilities_exception(e)
                                .into(),
                        );
                    }
                } else if BulkTransferException::all_caused_by_cache_not_found_exception_of(&e) {
                    // Intentionally left blank
                } else {
                    let mut error_message =
                        utils::grpc_aware_error_message(&e, self.verbose_failures);
                    if error_message.is_empty() {
                        // `e.getClass().getSimpleName()`
                        // PORT: `grpc_aware_error_message` already falls back to the simple class
                        // name, so this branch is not reached.
                        error_message = "IOException".to_string();
                    }
                    error_message = format!("Remote Cache: {error_message}");
                    self.remote_execution_service
                        .report(Event::warn(error_message));
                }
            }
        }
        if let Some(previous_execution) = previous_execution.as_ref() {
            let fetch_time = Instant::now();
            let previous_result = self
                .remote_execution_service
                .wait_for_and_reuse_outputs(action, previous_execution)?;
            if let Some(previous_result) = previous_result {
                spawn_metrics.fetch_time_in_ms = to_ms(fetch_time.elapsed());
                spawn_metrics.total_time_in_ms = to_ms(total_time.elapsed());
                spawn_metrics.network_time_in_ms = to_ms(action.get_network_time().get_duration());
                let build_metrics = spawn_metrics.clone();
                self.remote_execution_service
                    .maybe_write_param_files_locally(spawn)?;
                return Ok(Some(spawn_cache::success(deduplicated_spawn_result(
                    previous_result,
                    build_metrics,
                ))));
            }
            // If we reach here, the previous execution was not successful (it encountered an
            // exception or the spawn had an exit code != 0). Since it isn't possible to accurately
            // recreate the failure without rerunning the action, we fall back to running the
            // action locally. This means that we have introduced an unnecessary wait, but that
            // can only happen in the case of a failing build with --keep_going.
        }
        Ok(None)
    }

    /// The inner `try` block of `lookup` (the cache check, Java lines 143-181). `Some(result)` is
    /// the cache hit that Java returns as `SpawnCache.success(spawnResult)`.
    // Java: com/google/devtools/build/lib/remote/RemoteSpawnCache.java:142 RemoteSpawnCache.lookup
    fn try_cache_hit(
        &self,
        spawn: &dyn Spawn,
        action: &RemoteAction<'_>,
        this_execution: Option<&LocalExecution>,
        spawn_metrics: &mut SpawnMetrics,
        total_time: Instant,
    ) -> Result<Option<SpawnResult>, SpawnCacheError> {
        // PORT: profiling (`prof.profile(REMOTE_CACHE_CHECK, "check cache hit")`) is skipped.
        let result = self.remote_execution_service.lookup_cache(action)?;
        // In case the remote cache returned a failed action (exit code != 0) or failed to create
        // a mandatory output, we treat it as a cache miss.
        if let Some(result) = result
            && result.get_exit_code() == 0
            && result.maybe_get_missing_mandatory_output(action).is_none()
        {
            if let Some(this_execution) = this_execution {
                this_execution.close();
            }
            let fetch_time = Instant::now();
            // PORT: profiling (`prof.profile(REMOTE_DOWNLOAD, "download outputs")`) is skipped.
            let in_memory_output = self
                .remote_execution_service
                .download_outputs(action, &result)?;
            let fetch_time_elapsed = fetch_time.elapsed();
            let total_time_elapsed = total_time.elapsed();
            spawn_metrics.fetch_time_in_ms = to_ms(fetch_time_elapsed);
            spawn_metrics.total_time_in_ms = to_ms(total_time_elapsed);
            spawn_metrics.network_time_in_ms = to_ms(action.get_network_time().get_duration());
            let execution_metadata = result.get_execution_metadata();
            let spawn_result = create_spawn_result(
                &self.digest_util,
                action.get_action_key(),
                result.get_exit_code(),
                /* cache_hit= */ true,
                // `runnerName + " cache hit"` prints "null" for a null cache name.
                result.cache_name().unwrap_or("null"),
                in_memory_output,
                execution_metadata
                    .execution_start_timestamp
                    .unwrap_or_default(),
                execution_metadata
                    .execution_completed_timestamp
                    .unwrap_or_default(),
                spawn_metrics.clone(),
                &spawn.get_mnemonic(),
            );
            self.remote_execution_service
                .maybe_write_param_files_locally(spawn)?;
            return Ok(Some(spawn_result));
        }
        Ok(None)
    }
}

/// `new SpawnResult.DelegateSpawnResult(previousResult) { getRunnerName() { return
/// "deduplicated"; } getMetrics() { return buildMetrics; } }`.
// Java: com/google/devtools/build/lib/remote/RemoteSpawnCache.java:225 RemoteSpawnCache.lookup
fn deduplicated_spawn_result(
    previous_result: SpawnResult,
    build_metrics: SpawnMetrics,
) -> SpawnResult {
    previous_result.delegate_with_runner_name_and_metrics("deduplicated", build_metrics)
}

impl SpawnCache for RemoteSpawnCache {
    // Java: com/google/devtools/build/lib/remote/RemoteSpawnCache.java:84 RemoteSpawnCache.lookup
    fn lookup(
        &self,
        spawn: &dyn Spawn,
        context: &dyn SpawnExecutionContext,
    ) -> Result<Box<dyn CacheHandle>, SpawnCacheError> {
        let should_accept_cached_result = self
            .remote_execution_service
            .get_read_cache_policy(spawn)
            .allow_any_cache();
        let should_upload_local_results = self
            .remote_execution_service
            .get_write_cache_policy(spawn)
            .allow_any_cache();
        if !should_accept_cached_result && !should_upload_local_results {
            return Ok(Box::new(NoResultNoStore));
        }

        let total_time = Instant::now();

        let action = self
            .remote_execution_service
            .build_remote_action_with_blob_policy(spawn, context, BlobPolicy::Discard)?;
        // `SpawnMetrics.Builder.forRemoteExec().setInputBytes(..).setInputFiles(..)`
        let mut spawn_metrics = SpawnMetrics {
            input_bytes: action.get_input_bytes(),
            input_files: action.get_input_files(),
            ..SpawnMetrics::for_exec(ExecKind::Remote)
        };

        context.set_digest(self.digest_util.as_spawn_log_proto(action.get_action_key()));

        let mut this_execution: Option<Arc<LocalExecution>> = None;
        if should_accept_cached_result {
            // With path mapping enabled, different Spawns in a single build can have the same
            // ActionKey. When their result isn't in the cache and two of them are scheduled
            // concurrently, neither will result in a cache hit before the other finishes and
            // uploads its result, which results in unnecessary work. To avoid this, we keep track
            // of in-flight executions as long as their results haven't been uploaded to the cache
            // yet and deduplicate all of them against the first one.
            let mut previous_execution: Option<Arc<LocalExecution>> = None;
            let outcome = self.lookup_accepting_cached_result(
                spawn,
                &action,
                should_upload_local_results,
                &mut spawn_metrics,
                total_time,
                &mut this_execution,
                &mut previous_execution,
            );
            // finally
            if let Some(previous_execution) = &previous_execution {
                previous_execution.unregister();
            }
            if let Some(cache_handle) = outcome? {
                return Ok(cache_handle);
            }
        }

        if should_upload_local_results {
            // SAFETY: the handle keeps the `RemoteAction`, which borrows `spawn` and `context`
            // (Java's anonymous `CacheHandle` captures `action`). blaze_exec's
            // `SpawnCache::lookup` returns a `'static` handle, so the borrow is extended to
            // `'static` here. This is sound only because the handle is used and dropped while
            // `spawn` and `context` are alive: `AbstractSpawnStrategy.exec` (the only caller)
            // calls `store` and `close` on it and drops it inside the scope of both. Missing
            // hook: `SpawnCache::lookup<'a>(&self, spawn: &'a dyn Spawn, context: &'a dyn
            // SpawnExecutionContext) -> Result<Box<dyn CacheHandle + 'a>, SpawnCacheError>`.
            let action: RemoteAction<'static> =
                unsafe { std::mem::transmute::<RemoteAction<'_>, RemoteAction<'static>>(action) };
            Ok(Box::new(RemoteCacheHandle {
                options: self.options.clone(),
                remote_execution_service: self.remote_execution_service.clone(),
                action,
                this_execution_final: this_execution,
            }))
        } else {
            Ok(Box::new(NoResultNoStore))
        }
    }

    // Java: com/google/devtools/build/lib/remote/RemoteSpawnCache.java:310 RemoteSpawnCache.usefulInDynamicExecution
    fn useful_in_dynamic_execution(&self) -> bool {
        false
    }
}

/// The anonymous `CacheHandle` that `lookup` returns when local results should be uploaded.
// Java: com/google/devtools/build/lib/remote/RemoteSpawnCache.java:252 RemoteSpawnCache.lookup
struct RemoteCacheHandle {
    options: Arc<RemoteOptions>,
    remote_execution_service: Arc<RemoteExecutionService>,
    /// See the SAFETY note in `RemoteSpawnCache::lookup`: borrows the spawn and its context.
    action: RemoteAction<'static>,
    this_execution_final: Option<Arc<LocalExecution>>,
}

impl CacheHandle for RemoteCacheHandle {
    // Java: com/google/devtools/build/lib/remote/RemoteSpawnCache.java:254 RemoteSpawnCache.lookup.hasResult
    fn has_result(&self) -> bool {
        false
    }

    // Java: com/google/devtools/build/lib/remote/RemoteSpawnCache.java:259 RemoteSpawnCache.lookup.getResult
    fn get_result(&self) -> SpawnResult {
        panic!("NoSuchElementException")
    }

    // Java: com/google/devtools/build/lib/remote/RemoteSpawnCache.java:264 RemoteSpawnCache.lookup.willStore
    fn will_store(&self) -> bool {
        true
    }

    // Java: com/google/devtools/build/lib/remote/RemoteSpawnCache.java:269 RemoteSpawnCache.lookup.store
    fn store(&self, result: &SpawnResult) -> Result<(), SpawnCacheError> {
        if !self
            .remote_execution_service
            .commit_result_and_decide_whether_to_upload(
                result,
                self.this_execution_final.as_deref(),
            )
        {
            return Ok(());
        }

        // As soon as the result is in the cache, actions can get the result from it instead of
        // from the first in-flight execution. Not keeping in-flight executions around
        // indefinitely is important to avoid excessive memory pressure - Spawns can be very
        // large.
        let on_upload_complete: Box<dyn FnOnce() + Send> = match &self.this_execution_final {
            Some(this_execution_final) => Box::new(this_execution_final.delay_close()),
            None => Box::new(|| {}),
        };
        self.remote_execution_service
            .upload_outputs_with_concurrent_changes_check_level(
                &self.action,
                result,
                on_upload_complete,
                self.options.guard_against_concurrent_changes,
            )?;
        if let Some(this_execution_final) = &self.this_execution_final
            && self
                .action
                .get_spawn()
                .get_resource_owner()
                .may_modify_spawn_outputs_after_execution()
        {
            // In this case outputs have been uploaded synchronously and the callback above has
            // run, so no new executions will be deduplicated against this one. We can safely
            // await all existing executions finish the reuse.
            // Note that while this call itself isn't interruptible, all operations it awaits are
            // interruptible.
            // PORT: profiling (`prof.profile(REMOTE_DOWNLOAD, "await output reuse")`) is skipped.
            this_execution_final.await_all_output_reuse();
        }
        Ok(())
    }

    // Java: com/google/devtools/build/lib/remote/RemoteSpawnCache.java:298 RemoteSpawnCache.lookup.close
    fn close(&self) {
        if let Some(this_execution_final) = &self.this_execution_final {
            this_execution_final.close();
        }
    }
}
