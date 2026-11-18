//! Port of `lib/exec/AbstractSpawnStrategy.java`.

use std::fmt;
use std::io;
use std::sync::Arc;
use std::sync::atomic::{AtomicI32, Ordering};
use std::time::{Duration, UNIX_EPOCH};

use blaze_optdefs::command::execution_options::ExecutionOptions;
use blaze_protos::failure_details::{self, FailureDetail, failure_detail, spawn::Code};
use blaze_protos::tools::protos::Digest;
use blaze_shell::CommandFailureUtils;
use blaze_vfs::{Path, PathFragment};
use indexmap::IndexMap;
use parking_lot::Mutex;

use crate::actions::spawn_strategy::{SandboxedSpawnStrategy, SpawnStrategy, StopConcurrentSpawns};
use crate::actions::{
    EnvironmentalExecException, ExecException, InputMapping, InputMetadataProvider, Spawn,
    SpawnDescribable, SpawnExecException, SpawnResult, Status, spawns,
};
use crate::exec::action_execution_context::{
    ActionContextRegistry, ActionExecutionContext, SpawnCache, SpawnCacheError, SpawnExecutedEvent,
    no_cache,
};
use crate::exec::spawn_input_expander::SpawnInputExpander;
use crate::exec::spawn_runner::{ProgressStatus, SpawnExecutionContext, SpawnRunner};
use crate::util::FileOutErr;

/// Last unique identifier assigned to a spawn by this strategy.
///
/// These identifiers must be unique per strategy within the context of a Bazel server instance to
/// avoid cross-contamination across actions in case we perform asynchronous deletions.
// Java: com/google/devtools/build/lib/exec/AbstractSpawnStrategy.java:78 AbstractSpawnStrategy.execCount
static EXEC_COUNT: AtomicI32 = AtomicI32::new(0);

/// The `EnvironmentalExecException` `AbstractSpawnStrategy.exec` throws when the spawn runner (or
/// the spawn cache) throws an `IOException`: `"Exec failed due to IOException: <cause>"`. An
/// interrupted I/O (`InterruptedIOException`) becomes [`ExecException::Interrupted`].
///
/// Seam: [`SpawnRunner::exec`] has no `IOException` channel, so runners convert their I/O errors
/// with this function, which yields exactly what the strategy would have thrown.
// Java: com/google/devtools/build/lib/exec/AbstractSpawnStrategy.java:169 AbstractSpawnStrategy.exec
pub fn exec_io_exception(e: &io::Error) -> ExecException {
    if e.kind() == io::ErrorKind::Interrupted {
        return ExecException::Interrupted;
    }
    EnvironmentalExecException::with_cause(
        e,
        FailureDetail {
            message: "Exec failed due to IOException".to_string(),
            category: Some(failure_detail::Category::Spawn(failure_details::Spawn {
                code: Code::ExecIoException as i32,
                ..Default::default()
            })),
        },
    )
    .into()
}

/// Abstract common ancestor for spawn strategies implementing the common parts.
///
/// Java subclasses (`StandaloneSpawnStrategy`, `LinuxSandboxedStrategy`, ...) only override
/// `toString()`; here they are this struct with their display name (Rust `Display`).
// Java: com/google/devtools/build/lib/exec/AbstractSpawnStrategy.java:70 AbstractSpawnStrategy
pub struct AbstractSpawnStrategy {
    // PORT: Java's constructor takes no exec root (it reads the action's); kept for the callers
    // that construct strategies with one.
    exec_root: Path,
    spawn_input_expander: SpawnInputExpander,
    spawn_runner: Arc<dyn SpawnRunner>,
    execution_options: Arc<ExecutionOptions>,
    display_name: String,
    simple_class_name: String,
}

impl fmt::Debug for AbstractSpawnStrategy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AbstractSpawnStrategy")
            .field("display_name", &self.display_name)
            .field("spawn_runner", &self.spawn_runner.get_name())
            .finish_non_exhaustive()
    }
}

impl AbstractSpawnStrategy {
    /// `new <Subclass>(spawnRunner, executionOptions)` with the subclass's `toString()`.
    // Java: com/google/devtools/build/lib/exec/AbstractSpawnStrategy.java:84 AbstractSpawnStrategy.<init>
    pub fn new(
        exec_root: Path,
        spawn_runner: Arc<dyn SpawnRunner>,
        execution_options: Arc<ExecutionOptions>,
        display_name: &str,
    ) -> AbstractSpawnStrategy {
        AbstractSpawnStrategy {
            exec_root,
            spawn_input_expander: SpawnInputExpander::new(),
            spawn_runner,
            execution_options,
            display_name: display_name.to_string(),
            simple_class_name: "AbstractSpawnStrategy".to_string(),
        }
    }

    /// Sets `getClass().getSimpleName()` (e.g. `"StandaloneSpawnStrategy"`).
    pub fn with_simple_class_name(mut self, simple_class_name: &str) -> AbstractSpawnStrategy {
        self.simple_class_name = simple_class_name.to_string();
        self
    }

    /// The exec root given at construction.
    pub fn get_exec_root(&self) -> &Path {
        &self.exec_root
    }

    /// Gets the `SpawnRunner` that this strategy uses to actually run spawns.
    ///
    /// This is considered a stop-gap until we refactor the entire SpawnStrategy / SpawnRunner
    /// mechanism to no longer need Spawn strategies.
    // Java: com/google/devtools/build/lib/exec/AbstractSpawnStrategy.java:96 AbstractSpawnStrategy.getSpawnRunner
    pub fn get_spawn_runner(&self) -> &Arc<dyn SpawnRunner> {
        &self.spawn_runner
    }

    /// `canExec(spawn, actionContextRegistry)`.
    // Java: com/google/devtools/build/lib/exec/AbstractSpawnStrategy.java:101 AbstractSpawnStrategy.canExec
    pub fn can_exec_spawn(&self, spawn: &dyn Spawn) -> bool {
        self.spawn_runner.can_exec(spawn)
    }

    /// `exec(spawn, actionExecutionContext, stopConcurrentSpawns)`.
    // Java: com/google/devtools/build/lib/exec/AbstractSpawnStrategy.java:112 AbstractSpawnStrategy.exec
    pub fn exec_spawn(
        &self,
        spawn: &dyn Spawn,
        action_execution_context: &dyn ActionExecutionContext,
        stop_concurrent_spawns: Option<&dyn StopConcurrentSpawns>,
    ) -> Result<Vec<SpawnResult>, ExecException> {
        action_execution_context
            .maybe_report_subcommand(spawn, Some(self.spawn_runner.get_name().as_str()));

        let timeout = spawns::get_timeout(spawn)?;
        let context = SpawnExecutionContextImpl {
            spawn,
            action_execution_context,
            stop_concurrent_spawns,
            timeout,
            id: EXEC_COUNT.fetch_add(1, Ordering::SeqCst) + 1,
            spawn_input_expander: &self.spawn_input_expander,
            lazy_input_mapping: Mutex::new(None),
            digest: Mutex::new(None),
        };

        // Avoid caching for runners which handle caching internally e.g. RemoteSpawnRunner.
        // In production, the getContext method guarantees that we never get null back. However,
        // our integration tests don't set it up correctly, so cache may be null in testing.
        let mut cache: Arc<dyn SpawnCache> = if self.spawn_runner.handles_caching() {
            no_cache()
        } else {
            action_execution_context
                .get_spawn_cache()
                .unwrap_or_else(no_cache)
        };

        // Avoid using the remote cache of a dynamic execution setup for the local runner.
        if context.speculating() && !cache.useful_in_dynamic_execution() {
            cache = no_cache();
        }

        let attempt: Result<SpawnResult, SpawnCacheError> = (|| {
            let cache_handle = cache.lookup(spawn, &context)?;
            let result: Result<SpawnResult, SpawnCacheError> = (|| {
                if cache_handle.has_result() {
                    Ok(cache_handle.get_result())
                } else {
                    let start_time = UNIX_EPOCH
                        + Duration::from_millis(
                            action_execution_context.get_clock().current_time_millis() as u64,
                        );
                    // Actual execution.
                    let spawn_result = self.spawn_runner.exec(spawn, &context)?;

                    let spawn_identifier = spawn_result.get_digest().map(|d| d.hash.clone());
                    action_execution_context.get_event_handler().post(Arc::new(
                        SpawnExecutedEvent::new(
                            spawn,
                            spawn_result.clone(),
                            start_time,
                            spawn_identifier,
                        ),
                    ));
                    if cache_handle.will_store() {
                        cache_handle.store(&spawn_result)?;
                    }
                    Ok(spawn_result)
                }
            })();
            cache_handle.close();
            result
        })();

        let (spawn_result, ex): (SpawnResult, Option<SpawnExecException>) = match attempt {
            Ok(r) => (r, None),
            Err(SpawnCacheError::Io(e)) => return Err(exec_io_exception(&e)),
            Err(SpawnCacheError::Exec(ExecException::Spawn(e))) => {
                // Log the Spawn and re-throw.
                (e.result.clone(), Some(*e))
            }
            Err(SpawnCacheError::Exec(e)) => return Err(e),
        };

        if let Some(spawn_log_context) = action_execution_context.get_spawn_log_context() {
            let file_system = action_execution_context
                .get_action_file_system()
                .unwrap_or_else(|| {
                    let local = action_execution_context
                        .get_exec_root()
                        .get_file_system()
                        .clone();
                    // The RemoteActionFileSystem of a build with a remote output tree (see
                    // RemoteOutputTreeFileSystem).
                    match crate::exec::action_execution_context::RemoteOutputTree::from_context(
                        action_execution_context.get_context_any(std::any::TypeId::of::<
                            crate::exec::action_execution_context::RemoteOutputTree,
                        >()),
                    ) {
                        Some(tree) => crate::exec::action_execution_context::RemoteOutputTreeFileSystem::create(local, tree),
                        None => local,
                    }
                });
            // PORT: Java wraps an IOException of logSpawn in an EnvironmentalExecException
            // ("IOException while logging spawn", SPAWN_LOG_IO_EXCEPTION); the seam's logSpawn
            // reports its errors as ExecException already.
            spawn_log_context.log_spawn(
                spawn,
                action_execution_context.get_input_metadata_provider(),
                &|| {
                    context.get_input_mapping(
                        &PathFragment::empty_fragment(),
                        /* will_access_repeatedly= */ false,
                    )
                },
                &file_system,
                context.get_timeout(),
                &spawn_result,
            )?;
        }
        if let Some(ex) = ex {
            return Err(ex.into());
        }

        if spawn_result.status() != Status::Success {
            let cwd = action_execution_context
                .get_exec_root()
                .get_path_string()
                .to_string();
            let result_message = spawn_result.get_failure_message();
            let message = if !result_message.is_empty() {
                result_message.to_string()
            } else {
                CommandFailureUtils::describe_command_failure(
                    self.execution_options.verbose_failures,
                    Some(cwd.as_str()),
                    &SpawnDescribable(spawn),
                )
            };
            return Err(SpawnExecException::new(
                message,
                spawn_result,
                /* forcibly_run_remotely= */ false,
            )
            .into());
        }
        Ok(vec![spawn_result])
    }
}

impl fmt::Display for AbstractSpawnStrategy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.display_name)
    }
}

impl SpawnStrategy for AbstractSpawnStrategy {
    // Java: com/google/devtools/build/lib/exec/AbstractSpawnStrategy.java:106 AbstractSpawnStrategy.exec
    fn exec(
        &self,
        spawn: &dyn Spawn,
        action_execution_context: &dyn ActionExecutionContext,
    ) -> Result<Vec<SpawnResult>, ExecException> {
        self.exec_spawn(spawn, action_execution_context, None)
    }

    // Java: com/google/devtools/build/lib/exec/AbstractSpawnStrategy.java:101 AbstractSpawnStrategy.canExec
    fn can_exec(
        &self,
        spawn: &dyn Spawn,
        _action_context_registry: &dyn ActionContextRegistry,
    ) -> bool {
        self.can_exec_spawn(spawn)
    }

    fn get_simple_class_name(&self) -> String {
        self.simple_class_name.clone()
    }

    fn as_sandboxed_spawn_strategy(&self) -> Option<&dyn SandboxedSpawnStrategy> {
        Some(self)
    }

    fn as_abstract_spawn_strategy(&self) -> Option<&AbstractSpawnStrategy> {
        Some(self)
    }
}

impl SandboxedSpawnStrategy for AbstractSpawnStrategy {
    fn exec_with_stop_concurrent_spawns(
        &self,
        spawn: &dyn Spawn,
        action_execution_context: &dyn ActionExecutionContext,
        stop_concurrent_spawns: Option<&dyn StopConcurrentSpawns>,
    ) -> Result<Vec<SpawnResult>, ExecException> {
        self.exec_spawn(spawn, action_execution_context, stop_concurrent_spawns)
    }
}

/// `AbstractSpawnStrategy.SpawnExecutionContextImpl`.
// Java: com/google/devtools/build/lib/exec/AbstractSpawnStrategy.java:224 AbstractSpawnStrategy.SpawnExecutionContextImpl
struct SpawnExecutionContextImpl<'a> {
    spawn: &'a dyn Spawn,
    action_execution_context: &'a dyn ActionExecutionContext,
    stop_concurrent_spawns: Option<&'a dyn StopConcurrentSpawns>,
    timeout: Duration,
    id: i32,
    spawn_input_expander: &'a SpawnInputExpander,
    // Memoize the input mapping so that prefetchInputs can reuse it instead of recomputing it.
    // TODO(ulfjack): Guard against client modification of this map.
    lazy_input_mapping: Mutex<Option<(PathFragment, Arc<InputMapping>)>>,
    digest: Mutex<Option<Digest>>,
}

impl SpawnExecutionContext for SpawnExecutionContextImpl<'_> {
    fn get_id(&self) -> i32 {
        self.id
    }

    // Java: com/google/devtools/build/lib/exec/AbstractSpawnStrategy.java:255 SpawnExecutionContextImpl.setDigest
    fn set_digest(&self, digest: Digest) {
        let mut current = self.digest.lock();
        if let Some(existing) = current.as_ref() {
            blaze_util::check_argument!(
                *existing == digest,
                "setDigest was called more than once with different digests: {:?} vs {:?}",
                existing,
                digest
            );
        }
        *current = Some(digest);
    }

    fn get_digest(&self) -> Option<Digest> {
        self.digest.lock().clone()
    }

    // Java: com/google/devtools/build/lib/exec/AbstractSpawnStrategy.java:273 SpawnExecutionContextImpl.prefetchInputs
    fn prefetch_inputs_and_wait(&self) -> Result<(), ExecException> {
        if spawns::should_prefetch_inputs_for_local_execution(self.spawn) {
            let inputs: Vec<_> = self
                .get_input_mapping(
                    &PathFragment::empty_fragment(),
                    /* will_access_repeatedly= */ true,
                )
                .values()
                .cloned()
                .collect();
            // PORT: the BulkTransferException handling (lost remote inputs) belongs to the
            // remote prefetcher, which is not ported.
            return self.action_execution_context.prefetch_files(
                self.spawn.get_resource_owner(),
                &inputs,
                self.get_input_metadata_provider(),
            );
        }
        Ok(())
    }

    fn get_input_metadata_provider(&self) -> &dyn InputMetadataProvider {
        self.action_execution_context.get_input_metadata_provider()
    }

    // Java: com/google/devtools/build/lib/exec/AbstractSpawnStrategy.java:319 SpawnExecutionContextImpl.lockOutputFiles
    fn lock_output_files(
        &self,
        exit_code: i32,
        error_message: &str,
        out_err: &FileOutErr,
    ) -> Result<(), ExecException> {
        if let Some(stop) = self.stop_concurrent_spawns {
            stop.stop(exit_code, error_message, out_err)?;
        }
        Ok(())
    }

    fn speculating(&self) -> bool {
        self.stop_concurrent_spawns.is_some()
    }

    fn get_timeout(&self) -> Duration {
        self.timeout
    }

    fn get_file_out_err(&self) -> &FileOutErr {
        self.action_execution_context.get_file_out_err()
    }

    // Java: com/google/devtools/build/lib/exec/AbstractSpawnStrategy.java:342 SpawnExecutionContextImpl.getInputMapping
    fn get_input_mapping(
        &self,
        base_directory: &PathFragment,
        will_access_repeatedly: bool,
    ) -> Arc<InputMapping> {
        // Return previously computed copy if present.
        if let Some((dir, mapping)) = self.lazy_input_mapping.lock().as_ref()
            && dir == base_directory
        {
            return mapping.clone();
        }

        let input_mapping = Arc::new(self.spawn_input_expander.get_input_mapping(
            self.spawn,
            self.action_execution_context.get_input_metadata_provider(),
            base_directory,
        ));

        // Don't cache the input mapping if it is unlikely that it is used again.
        // This reduces memory usage in the case where remote caching/execution is
        // used, and the expected cache hit rate is high.
        if will_access_repeatedly {
            *self.lazy_input_mapping.lock() = Some((base_directory.clone(), input_mapping.clone()));
        }
        input_mapping
    }

    // Java: com/google/devtools/build/lib/exec/AbstractSpawnStrategy.java:368 SpawnExecutionContextImpl.report
    fn report(&self, progress: ProgressStatus) {
        // PORT: Java returns early when the owner or the primary output is null; neither is
        // nullable in the seam.
        let action = self.spawn.get_resource_owner();
        self.action_execution_context
            .post_progress(action, &progress);
    }

    fn is_rewinding_enabled(&self) -> bool {
        self.action_execution_context.is_rewinding_enabled()
    }

    fn check_for_lost_inputs(&self) -> Result<(), ExecException> {
        self.action_execution_context.check_for_lost_inputs()
    }

    fn get_client_env(&self) -> &IndexMap<String, String> {
        self.action_execution_context.get_client_env()
    }

    // Java: com/google/devtools/build/lib/exec/AbstractSpawnStrategy.java:309 SpawnExecutionContextImpl.getContext
    fn get_remote_local_fallback_registry(
        &self,
    ) -> Option<Arc<crate::exec::spawn_strategy_registry::SpawnStrategyRegistry>> {
        self.action_execution_context.get_spawn_strategy_registry()
    }
}
