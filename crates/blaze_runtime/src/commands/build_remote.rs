//! The `RemoteModule` hooks of `blaze build` / `blaze test` (`beforeCommand`,
//! `registerSpawnStrategies` / `registerActionContexts` / `executorInit`, `afterAnalysis` /
//! `afterTopLevelTargetAnalysis`, the `RemoteExecutionService` subscriptions to
//! `BuildCompleteEvent` / `BuildInterruptedEvent`, and `afterCommand`), on top of
//! `blaze_remote::remote_module`.
//!
//! PORT: Java's `BlazeCommandDispatcher` / `BlazeRuntime` call the module hooks for every
//! command; Blaze calls them from the build command (`crate::commands::build`) for build/test and
//! from the dispatcher ([`dispatcher_before_command`]) for the other commands, which get
//! `beforeCommand` / `afterCommand` only (the commands other than build/test that execute
//! actions refuse the remote options loudly in `crate::unported_options`). A command without
//! any remote option keeps the placeholder provider of `blaze_exec::exec::executor_setup`.

use std::sync::Arc;

use blaze_actions::Artifact;
use blaze_analysis::configured_target::ConfiguredTarget;
use blaze_analysis::providers::file_provider::FilesToRunProvider;
use blaze_analysis::providers::output_group_info::HIDDEN_TOP_LEVEL;
use blaze_analysis::test::test_provider::{TestOutput, TestProvider};
use blaze_bep::buildeventstream::{BuildEventArtifactUploader, LocalFilesArtifactUploader};
use blaze_bep::runtime::{
    BuildEventArtifactUploaderFactory, BuildEventArtifactUploaderFactoryMap,
    BuildEventArtifactUploaderFactoryMapBuilder,
};
use blaze_events::{Event, EventHandler, ExtendedEventHandler, JavaClock};
use blaze_exec::actions::action_adapters::artifact_input;
use blaze_exec::actions::action_output_directory_helper::ActionOutputDirectoryHelper;
use blaze_exec::actions::spawn_strategy::SpawnStrategy;
use blaze_exec::actions::{ActionInputRef, BasicActionInput};
use blaze_exec::exec::action_execution_context::ActionInputPrefetcher;
use blaze_exec::exec::executor_setup::RemoteModuleContributions;
use blaze_exec::exec::spawn_strategy_registry::AbruptExitException;
use blaze_optdefs::command::build_language_options::BuildLanguageOptions;
use blaze_optdefs::command::build_request_options::BuildRequestOptions;
use blaze_optdefs::command::common_command_options::CommonCommandOptions;
use blaze_optdefs::command::execution_options::ExecutionOptions;
use blaze_optdefs::misc::auth_and_tls_options::AuthAndTLSOptions;
use blaze_optdefs::misc::remote_options::RemoteOptions;
use blaze_remote::hooks::RemoteHooks;
use blaze_remote::remote_module::{RemoteModule, RemoteModuleEnvironment};
use blaze_remote::remote_output_checker::{
    AnalysisResultView, RunfilesView, TargetUnderTest, TopLevelTarget,
};
use blaze_sky_top::build_view::AnalysisResult;
use blaze_sky_top::execution_seam::{get_all_artifacts_to_build, is_important_group};
use parking_lot::Mutex;

use crate::command_environment::CommandEnvironment;
use crate::options_result::OptionsParsingResult;
use crate::version_info::BlazeVersionInfo;

/// The command's `RemoteModule`, between `beforeCommand` and `afterCommand`.
pub(crate) struct RemoteSupport {
    module: Mutex<RemoteModule>,
    /// `RemoteModule.executorInit`'s `LeaseService` (a combined cache without
    /// `--rewind_lost_inputs`).
    lease_service: Option<Arc<blaze_remote::lease_service::LeaseService>>,
    /// `env.withMergedAnalysisAndExecutionSourceOfTruth()`, which decides whether
    /// `registerActionContexts` registers the `RemoteImportantOutputHandler`; set by
    /// [`RemoteSupport::set_merged_analysis_and_execution`].
    merged_analysis_and_execution: Arc<std::sync::atomic::AtomicBool>,
}

/// `env.addIdleTask(gcIdleTask)` of `RemoteModule.beforeCommand`, for the task the module
/// created.
// Java: com/google/devtools/build/lib/remote/RemoteModule.java:403 RemoteModule.beforeCommand
fn add_disk_cache_gc_idle_task(env: &CommandEnvironment, module: &mut RemoteModule) {
    if let Some(gc_idle_task) = module.take_disk_cache_gc_idle_task() {
        env.add_idle_task(Arc::new(crate::idle_tasks::DiskCacheGcIdleTask(
            gc_idle_task,
        )));
    }
}

/// Whether any remote functionality is requested: `RemoteModule.setup` returns early (the
/// placeholder provider, which `blaze_exec::exec::executor_setup` ports) otherwise.
// Java: com/google/devtools/build/lib/remote/RemoteModule.java:416 RemoteModule.setup
fn remote_requested(remote_options: &RemoteOptions) -> bool {
    let set = |value: &Option<String>| value.as_ref().is_some_and(|v| !v.is_empty());
    set(&remote_options.remote_executor)
        || set(&remote_options.remote_cache)
        || remote_options
            .disk_cache
            .as_ref()
            .is_some_and(|p| !p.get().is_empty())
        || set(&remote_options.remote_output_service)
        || set(&remote_options.remote_downloader)
}

/// `RemoteModule.beforeCommand(env)`. `Ok(None)` when no remote option is set (Blaze then keeps
/// its placeholder path); `Err` is the `AbruptExitException` that Java's dispatcher reports as
/// `ERROR: <message>` and exits with.
// Java: com/google/devtools/build/lib/remote/RemoteModule.java:403 RemoteModule.beforeCommand
pub(crate) fn before_command(
    env: &CommandEnvironment,
    options: &OptionsParsingResult,
) -> Result<Option<RemoteSupport>, AbruptExitException> {
    let Some(mut module) = set_up_module(env, options)? else {
        return Ok(None);
    };
    add_disk_cache_gc_idle_task(env, &mut module);
    let build_request_options = options.get_options::<BuildRequestOptions>();
    let lease_extension_requested = options
        .get_options::<RemoteOptions>()
        .is_some_and(|o| o.remote_cache_lease_extension);
    // The digests earlier attempts of this command learnt from LostInputsEvents (see
    // KNOWN_MISSING_CAS_DIGESTS) are in the module's knownMissingCasDigests.
    if let Some(provider) = module.get_action_context_provider() {
        let known_missing = KNOWN_MISSING_CAS_DIGESTS.lock().clone();
        if !known_missing.is_empty() {
            provider
                .get_remote_execution_service()
                .on_lost_inputs(known_missing);
        }
    }
    // RemoteModule.executorInit: with a combined cache and without --rewind_lost_inputs, a
    // LeaseService (with a RemoteLeaseExtension for --experimental_remote_cache_lease_extension).
    // Java: com/google/devtools/build/lib/remote/RemoteModule.java:1241 RemoteModule.executorInit
    let rewind_lost_inputs = build_request_options
        .as_ref()
        .is_some_and(|o| o.rewind_lost_inputs);
    // The --experimental_remote_cache_lease_extension warning (with --rewind_lost_inputs) or
    // RemoteLeaseExtension (without).
    module.executor_init(env.get_reporter().as_ref(), rewind_lost_inputs);
    let lease_service = module
        .get_action_context_provider()
        .and_then(|provider| provider.get_combined_cache())
        .filter(|_| !rewind_lost_inputs)
        .map(|_| {
            Arc::new(blaze_remote::lease_service::LeaseService::new(
                lease_extension_requested,
            ))
        });
    Ok(Some(RemoteSupport {
        module: Mutex::new(module),
        lease_service,
        merged_analysis_and_execution: Arc::new(std::sync::atomic::AtomicBool::new(false)),
    }))
}

/// `memoizingEvaluator.delete(...)` of `LeaseService.handleMissingInputs`: the
/// `ACTION_EXECUTION` values with remote metadata leave the graph (at the start of the next
/// evaluation, which a retried invocation is), so that their actions run again.
// Java: com/google/devtools/build/lib/remote/LeaseService.java:88 LeaseService.handleMissingInputs
fn delete_remote_action_execution_values(memoizing_evaluator: &blaze_skyframe::Evaluator) {
    memoizing_evaluator.delete(|key, value| {
        if key.function_name() == <blaze_sky_exec::keys::ActionExecutionKey as blaze_skyframe::Key>::FUNCTION {
            // Java: memoizingEvaluator.getExistingValue(key) instanceof ActionExecutionValue.
            return value
                .and_then(|v| {
                    v.downcast_ref::<blaze_sky_exec::action_execution_value::ActionExecutionValue>()
                })
                .is_some_and(is_remote);
        }
        false
    });
}

// Java: com/google/devtools/build/lib/remote/LeaseService.java:112 LeaseService.isRemote
fn is_remote(value: &blaze_sky_exec::action_execution_value::ActionExecutionValue) -> bool {
    value
        .get_all_file_values()
        .values()
        .any(|file_artifact_value| file_artifact_value.is_remote())
        || value
            .get_all_tree_artifact_values()
            .values()
            .any(|tree| is_remote_tree(tree))
}

// Java: com/google/devtools/build/lib/remote/LeaseService.java:117 LeaseService.isRemoteTree
fn is_remote_tree(tree_artifact_value: &blaze_exec::actions::TreeArtifactValue) -> bool {
    tree_artifact_value
        .child_values
        .iter()
        .any(|(_, file_artifact_value)| file_artifact_value.is_remote())
        || tree_artifact_value
            .archived_representation
            .as_ref()
            .is_some_and(|(_, archived_file_value)| archived_file_value.is_remote())
}

/// `RemoteModule.beforeCommand(env)` up to the end of `setup` (including
/// `createActionInputFetcher`), for any command: `Ok(None)` when no remote option is set
/// (after `getTempPathGenerator`'s stale-downloads cleanup).
// Java: com/google/devtools/build/lib/remote/RemoteModule.java:403 RemoteModule.beforeCommand
fn set_up_module(
    env: &CommandEnvironment,
    options: &OptionsParsingResult,
) -> Result<Option<RemoteModule>, AbruptExitException> {
    let Some(remote_options) = options.get_options::<RemoteOptions>() else {
        // Quit if no supported command is being used.
        notify_no_cache_available(env);
        return Ok(None);
    };
    if !remote_requested(&remote_options) {
        notify_no_cache_available(env);
        // `setup` still runs `tempPathGenerator = getTempPathGenerator(env)` before it returns
        // with the placeholder provider ("Found stale downloads from previous build,
        // deleting..."); `afterCommand` deletes the directory again (see
        // `placeholder_after_command`).
        // Java: com/google/devtools/build/lib/remote/RemoteModule.java:416 RemoteModule.setup
        let exec_root = env.get_exec_root();
        blaze_remote::remote_module::get_temp_path_generator(
            &env.get_directories().get_action_temps_directory(&exec_root),
            env.get_reporter().as_ref(),
        )?;
        return Ok(None);
    }
    // remoteOptions.scrubber: the converter parsed the file and checked its patterns.
    let scrubber = remote_options.scrubber.as_ref().map(|s| {
        Arc::new(
            blaze_remote::scrubber::Scrubber::new(&s.config)
                .expect("the option converter compiled every pattern"),
        )
    });
    let auth_and_tls_options = options
        .get_options::<AuthAndTLSOptions>()
        .unwrap_or_else(blaze_options::Options::<AuthAndTLSOptions>::get_defaults);
    let execution_options = options
        .get_options::<ExecutionOptions>()
        .unwrap_or_else(blaze_options::Options::<ExecutionOptions>::get_defaults);
    let build_request_options = options.get_options::<BuildRequestOptions>();
    let build_language_options = options
        .get_options::<BuildLanguageOptions>()
        .unwrap_or_else(blaze_options::Options::<BuildLanguageOptions>::get_defaults);
    let writable_outputs = options
        .get_options::<blaze_optdefs::fragment::core_options::CoreOptions>()
        .is_some_and(|o| o.experimental_writable_outputs);

    let exec_root = env.get_exec_root();
    let directories = env.get_directories();
    let build_request_id = build_request_id(options);
    let build_request_id_for_repo_helpers = build_request_id.clone();
    let accept_cached = remote_options.remote_accept_cached;
    let upload_local_results = remote_options.remote_upload_local_results;
    let remote_cache_ttl = remote_options.remote_cache_ttl;
    let remote_instance_name = remote_options.remote_instance_name.clone();
    let verbose_failures = execution_options.verbose_failures;
    let reporter: Arc<dyn ExtendedEventHandler> = env.get_reporter().clone();
    let mut module = RemoteModule::before_command(RemoteModuleEnvironment {
        command_name: env.get_command_name().to_owned(),
        remote_options,
        auth_and_tls_options,
        execution_options: Arc::new(execution_options),
        rewind_lost_inputs: build_request_options.as_ref().map(|o| o.rewind_lost_inputs),
        experimental_sibling_repository_layout: build_language_options
            .experimental_sibling_repository_layout,
        reporter,
        file_system: env.get_output_base().get_file_system().clone(),
        output_base: env.get_output_base().clone(),
        exec_root: exec_root.clone(),
        working_directory: env.get_working_directory().clone(),
        workspace: env.get_workspace().cloned(),
        install_base: Some(directories.get_install_base().clone()),
        workspace_name: env.get_workspace_name().to_owned(),
        action_temps_directory: directories.get_action_temps_directory(&exec_root),
        build_request_id,
        command_id: env.get_command_id().to_owned(),
        product_name: env.get_runtime().get_product_name().to_owned(),
        tool_version: BlazeVersionInfo::instance().get_version(),
        client_env: env.get_client_env().clone(),
        clock: Arc::new(JavaClock),
        scrubber,
        hooks: RemoteHooks::new(),
    })?;
    // RemoteModule.setup ends with `actionInputFetcher = createActionInputFetcher(combinedCache)`.
    let output_permissions = if writable_outputs {
        blaze_exec::actions::cache::OutputPermissions::Writable
    } else {
        blaze_exec::actions::cache::OutputPermissions::Readonly
    };
    module.create_action_input_fetcher(
        env.get_reporter().clone(),
        output_permissions,
        // env.getOutputDirectoryHelper() (the command has BuildRequestOptions).
        build_request_options
            .is_some()
            .then(|| Arc::new(ActionOutputDirectoryHelper::new())),
    );
    // RemoteModule.setup ends both cache paths with initRepoHelpersAndOverlayFs.
    init_repo_helpers_and_overlay_fs(
        env,
        &module,
        &build_request_id_for_repo_helpers,
        &remote_instance_name,
        accept_cached,
        upload_local_results,
        verbose_failures,
        remote_cache_ttl,
    );
    // beforeCommand: `if (!cacheAvailable) remoteFs.notifyNoCacheAvailable(evaluator)`.
    if !module.cache_available() {
        notify_no_cache_available(env);
    }
    Ok(Some(module))
}

/// The output base's file system as Java's `instanceof RemoteExternalOverlayFileSystem` sees it.
fn remote_external_overlay_file_system(
    env: &CommandEnvironment,
) -> Option<Arc<blaze_remote::remote_external_overlay_file_system::RemoteExternalOverlayFileSystem>>
{
    blaze_remote::remote_external_overlay_file_system::RemoteExternalOverlayFileSystem::of(
        env.get_output_base().get_file_system(),
    )
}

/// `invalidateRepoDirectories(evaluator, repoNames)` for the overlay file system: Java deletes
/// the `REPOSITORY_DIRECTORY` nodes of the repos (and their reverse dependencies) from the
/// server's graph.
///
/// PORT: the server's graph is the graph `SkyframeIncrementalState` keeps between commands
/// (crates/blaze_sky_top/src/incremental.rs), which offers no targeted `delete`; Blaze drops the
/// whole kept graph instead (`resetEvaluator`). The next command then re-evaluates everything,
/// which re-fetches exactly the repos Java's delete would, but also reloads the other nodes
/// (visible only in the "N packages loaded" counts of the progress messages).
// Java: com/google/devtools/build/lib/remote/RemoteExternalOverlayFileSystem.java:195 RemoteExternalOverlayFileSystem.invalidateRepoDirectories
fn invalidate_repo_directories(
    env: &CommandEnvironment,
) -> Arc<dyn Fn(&std::collections::HashSet<String>) + Send + Sync> {
    let incremental_state = env.get_blaze_workspace().get_incremental_state().clone();
    Arc::new(move |repo_names| {
        if repo_names.is_empty() {
            return;
        }
        incremental_state.reset_evaluator();
    })
}

/// `remoteFs.notifyNoCacheAvailable(env.getSkyframeExecutor().getEvaluator())` when the output
/// base is on the overlay file system.
// Java: com/google/devtools/build/lib/remote/RemoteModule.java:403 RemoteModule.beforeCommand
fn notify_no_cache_available(env: &CommandEnvironment) {
    if let Some(remote_fs) = remote_external_overlay_file_system(env) {
        remote_fs.notify_no_cache_available(&*invalidate_repo_directories(env));
    }
}

/// Initializes the repository remote helpers factory and primes the
/// `RemoteExternalOverlayFileSystem` (when one is in use) with the per-build state it needs.
// Java: com/google/devtools/build/lib/remote/RemoteModule.java:358 RemoteModule.initRepoHelpersAndOverlayFs
fn init_repo_helpers_and_overlay_fs(
    env: &CommandEnvironment,
    module: &RemoteModule,
    build_request_id: &str,
    remote_instance_name: &str,
    accept_cached: bool,
    upload_local_results: bool,
    verbose_failures: bool,
    remote_cache_ttl: std::time::Duration,
) {
    let Some(action_context_provider) = module.get_action_context_provider() else {
        return;
    };
    let Some(combined_cache) = action_context_provider.get_combined_cache() else {
        return;
    };
    let directories = env.get_directories();
    let invocation_id = env.get_command_id().to_owned();
    blaze_repo_rules::repository::remote_helpers::init_repository_remote_helpers_factory(Arc::new(
        blaze_remote::repository_remote_helpers_factory_impl::RepositoryRemoteHelpersFactoryImpl::new(
            blaze_sky_loading::directories::LoadingDirectories {
                workspace: env
                    .get_workspace()
                    .cloned()
                    .unwrap_or_else(|| env.get_working_directory().clone()),
                output_base: env.get_output_base().clone(),
                install_base: directories.get_install_base().clone(),
            },
            combined_cache.clone(),
            // actionContextProvider.getRemoteExecutionClient(); the cache is then a
            // RemoteExecutionCache.
            action_context_provider
                .get_remote_execution_client()
                .zip(action_context_provider.get_remote_execution_cache())
                .map(|(remote_executor, cache)| {
                    blaze_remote::repository_remote_helpers_factory_impl::RemoteExecutionHelpers {
                        cache: cache.clone(),
                        remote_executor: remote_executor.clone(),
                        exec_root: env.get_exec_root(),
                    }
                }),
            build_request_id.to_owned(),
            invocation_id.clone(),
            env.get_workspace_name().to_owned(),
            remote_instance_name.to_owned(),
            accept_cached,
            upload_local_results,
            verbose_failures,
        ),
    ));
    if let Some(remote_fs) = remote_external_overlay_file_system(env) {
        remote_fs.before_command(
            blaze_remote::remote_external_overlay_file_system::CommandState {
                cache: combined_cache.clone(),
                input_prefetcher: module
                    .get_action_input_fetcher()
                    .cloned()
                    .expect("RemoteModule.setup created the action input fetcher"),
                reporter: env.get_reporter().clone(),
                build_request_id: build_request_id.to_owned(),
                command_id: invocation_id,
                invalidate_repo_directories: invalidate_repo_directories(env),
                remote_cache_ttl,
            },
        );
    }
}

/// The part of `RemoteModule.afterCommand` that every command runs:
/// `repositoryRemoteHelpersFactoryDelegate.reset()` and `remoteFs.afterCommand()`.
// Java: com/google/devtools/build/lib/remote/RemoteModule.java:1098 RemoteModule.afterCommand
pub(crate) fn reset_repo_helpers_and_overlay_fs(env: &CommandEnvironment) {
    blaze_repo_rules::repository::remote_helpers::reset_repository_remote_helpers_factory();
    if let Some(remote_fs) = remote_external_overlay_file_system(env) {
        remote_fs.after_command();
    }
}

/// `CommandEnvironment.buildRequestId`: `--build_request_id`, else the dispatcher's
/// `buildRequestIdOverride` (the id of the first attempt, when this is an attempt retried after
/// a remote cache eviction), else a random UUID. The id is recorded for the dispatcher's retry
/// (`RemoteCacheTransientErrorException.getBuildRequestId`).
///
/// PORT: Blaze's `CommandEnvironment` does not hold the build request id (its only reader is
/// the remote module), so it is computed here and the override travels through
/// [`BUILD_REQUEST_ID_OVERRIDE`] / [`LAST_BUILD_REQUEST_ID`] instead of `initCommand`'s
/// parameter.
// Java: com/google/devtools/build/lib/runtime/CommandEnvironment.java:226 CommandEnvironment.<init>
fn build_request_id(options: &OptionsParsingResult) -> String {
    build_request_id_for(
        options
            .get_options::<CommonCommandOptions>()
            .and_then(|o| o.build_request_id.clone()),
    )
}

/// `env.getBuildRequestId()`: the build request id of this attempt of the command, shared by
/// the modules that read it (the Build Event Service module and the remote module).
pub(crate) fn command_build_request_id(options: &OptionsParsingResult) -> String {
    build_request_id(options)
}

/// Whether this attempt of the command is a retry (`buildRequestIdOverride != null`).
pub(crate) fn is_retried_attempt() -> bool {
    BUILD_REQUEST_ID_OVERRIDE.lock().is_some()
}

/// [`build_request_id`] given `--build_request_id`.
///
/// PORT: Java computes `env.getBuildRequestId()` once, when the command's environment is
/// created; here the first module that asks computes it and the later ones get the same value
/// (cleared at the start of each command by [`reset_build_request_id_override`]).
fn build_request_id_for(explicit: Option<String>) -> String {
    if let Some(last) = LAST_BUILD_REQUEST_ID.lock().clone() {
        return last;
    }
    let build_request_id = explicit
        .or_else(|| BUILD_REQUEST_ID_OVERRIDE.lock().clone())
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    *LAST_BUILD_REQUEST_ID.lock() = Some(build_request_id.clone());
    build_request_id
}

/// `BlazeCommandDispatcher.exec`'s `buildRequestIdOverride`: `None` for the first attempt, then
/// the build request id of the attempt that failed with a remote cache eviction ("Use a fixed
/// build request ID across cache eviction retries to tie together the individual invocations,
/// which all have different invocation IDs").
// Java: com/google/devtools/build/lib/runtime/BlazeCommandDispatcher.java:162 BlazeCommandDispatcher.exec
static BUILD_REQUEST_ID_OVERRIDE: Mutex<Option<String>> = Mutex::new(None);

/// The build request id of the last attempt that set the remote module up (`env.getBuildRequestId()`
/// of `new RemoteCacheTransientErrorException(env.getBuildRequestId(), env.getCommandId())`).
static LAST_BUILD_REQUEST_ID: Mutex<Option<String>> = Mutex::new(None);

/// `buildRequestIdOverride = null` at the start of `BlazeCommandDispatcher.exec`.
// Java: com/google/devtools/build/lib/runtime/BlazeCommandDispatcher.java:162 BlazeCommandDispatcher.exec
pub(crate) fn reset_build_request_id_override() {
    *BUILD_REQUEST_ID_OVERRIDE.lock() = None;
    *LAST_BUILD_REQUEST_ID.lock() = None;
}

/// `catch (RemoteCacheTransientErrorException e) { buildRequestIdOverride = e.getBuildRequestId(); }`.
// Java: com/google/devtools/build/lib/runtime/BlazeCommandDispatcher.java:162 BlazeCommandDispatcher.exec
pub(crate) fn retry_with_build_request_id_of_last_attempt() {
    let last = LAST_BUILD_REQUEST_ID.lock().clone();
    *BUILD_REQUEST_ID_OVERRIDE.lock() = last;
}

/// `RemoteModule.afterCommand` for a command that has `RemoteOptions` but enables nothing
/// remote: `afterCommandTask` with the placeholder provider (whose `afterCommand` does nothing)
/// and the `tempPathGenerator` that `setup` created: its directory is deleted, failures are
/// ignored.
// Java: com/google/devtools/build/lib/remote/RemoteModule.java:1140 RemoteModule.afterCommandTask
pub(crate) fn placeholder_after_command(env: &CommandEnvironment, options: &OptionsParsingResult) {
    let Some(remote_options) = options.get_options::<RemoteOptions>() else {
        return;
    };
    if remote_requested(&remote_options) {
        return;
    }
    let exec_root = env.get_exec_root();
    let temp_dir = env
        .get_directories()
        .get_action_temps_directory(&exec_root)
        .get_child("remote");
    // Intentionally ignored.
    let _ = temp_dir.delete_tree();
}

/// The `tempPathGenerator` directory `RemoteModule.setup` created for a command other than
/// build/test that requests nothing remote (see [`dispatcher_before_command`]), until
/// `afterCommand` deletes it.
static DISPATCHER_TEMP_DIR: Mutex<Option<blaze_vfs::Path>> = Mutex::new(None);

/// The `RemoteModule` [`dispatcher_before_command`] set up for a command other than build/test
/// that requests remote functionality, until `afterCommand`.
static DISPATCHER_MODULE: Mutex<Option<RemoteModule>> = Mutex::new(None);

/// `RemoteModule.beforeCommand(env)` for the commands other than `build` / `test` (which set the
/// module up in `crate::commands::build`, see [`before_command`]): `RemoteOptions` is a common
/// command option (`RemoteModule.getCommonCommandOptions`), so `beforeCommand` runs for every
/// command. When nothing remote is requested, `setup` returns with the placeholder provider
/// after `tempPathGenerator = getTempPathGenerator(env)` ("Found stale downloads from previous
/// build, deleting..."); otherwise it sets the module up as for a build (the eager capabilities
/// check, the caches, the stale-downloads cleanup). `executorInit` and the build hooks are not
/// called for these commands, as in Java (`crate::unported_options` refuses the remote options
/// for the commands that execute actions other than build/test). `Err` is its
/// `AbruptExitException`.
// Java: com/google/devtools/build/lib/remote/RemoteModule.java:403 RemoteModule.beforeCommand
// Java: com/google/devtools/build/lib/remote/RemoteModule.java:416 RemoteModule.setup
// Java: com/google/devtools/build/lib/remote/RemoteModule.java:1341 RemoteModule.getCommonCommandOptions
pub(crate) fn dispatcher_before_command(
    env: &CommandEnvironment,
) -> Result<(), crate::module::AbruptExitException> {
    *DISPATCHER_TEMP_DIR.lock() = None;
    *DISPATCHER_MODULE.lock() = None;
    if matches!(env.get_command_name(), "build" | "test") {
        return Ok(());
    }
    let Some(remote_options) = env.get_options().get_options::<RemoteOptions>() else {
        // Quit if no supported command is being used.
        return Ok(());
    };
    let to_module_exception = |e: AbruptExitException| {
        crate::module::AbruptExitException::new(
            crate::commands::build::abrupt_exit_detailed_exit_code(e),
        )
    };
    if remote_requested(&remote_options) {
        let mut module = set_up_module(env, env.get_options()).map_err(to_module_exception)?;
        if let Some(module) = &mut module {
            add_disk_cache_gc_idle_task(env, module);
        }
        *DISPATCHER_MODULE.lock() = module;
        return Ok(());
    }
    let exec_root = env.get_exec_root();
    match blaze_remote::remote_module::get_temp_path_generator(
        &env.get_directories().get_action_temps_directory(&exec_root),
        env.get_reporter().as_ref(),
    ) {
        Ok(temp_path_generator) => {
            *DISPATCHER_TEMP_DIR.lock() = Some(temp_path_generator.get_temp_dir().clone());
            Ok(())
        }
        Err(e) => Err(to_module_exception(e)),
    }
}

/// `RemoteModule.afterCommand` for a command [`dispatcher_before_command`] set up: the module's
/// `afterCommand` (its `AbruptExitException`, which the dispatcher reports as it reports the
/// other modules'), or, with the placeholder provider, `afterCommandTask`'s deletion of the
/// `tempPathGenerator` directory, ignoring failures.
// Java: com/google/devtools/build/lib/remote/RemoteModule.java:1098 RemoteModule.afterCommand
// Java: com/google/devtools/build/lib/remote/RemoteModule.java:1113 RemoteModule.afterCommandTask
pub(crate) fn dispatcher_after_command() -> Option<crate::module::AbruptExitException> {
    if let Some(mut module) = DISPATCHER_MODULE.lock().take() {
        return module.after_command().err().map(|e| {
            crate::module::AbruptExitException::new(
                crate::commands::build::abrupt_exit_detailed_exit_code(e),
            )
        });
    }
    if let Some(temp_dir) = DISPATCHER_TEMP_DIR.lock().take() {
        // Intentionally ignored.
        let _ = temp_dir.delete_tree();
    }
    None
}

/// The runtime's `BuildEventArtifactUploaderFactoryMap` (`BlazeRuntime
/// .getBuildEventArtifactUploaderFactoryMap`): Bazel registers one factory, RemoteModule's
/// "remote" (`serverInit`), the delegate whose `create` is
/// `RemoteModule::create_build_event_artifact_uploader` (the ByteStream uploader when `setup`
/// found a gRPC cache, else the local uploader). The build event module selects from it with
/// `--experimental_build_event_upload_strategy` (`select(null)` is "remote" too).
///
/// PORT: Java's map lives as long as the server and the delegate is re-initialised by each
/// command's `RemoteModule.setup`; Blaze creates the module per command, so the map is built per
/// command from the command's [`RemoteSupport`] (`None` when no remote option is set: Java's
/// delegate then has no factory and creates a `LocalFilesArtifactUploader`).
// Java: com/google/devtools/build/lib/remote/RemoteModule.java:228 RemoteModule.serverInit
// Java: com/google/devtools/build/lib/runtime/BlazeRuntime.java BlazeRuntime.getBuildEventArtifactUploaderFactoryMap
pub(crate) fn build_event_artifact_uploader_factory_map(
    remote: Option<&Arc<RemoteSupport>>,
) -> BuildEventArtifactUploaderFactoryMap {
    BuildEventArtifactUploaderFactoryMapBuilder::new()
        .add(
            "remote",
            Arc::new(BuildEventArtifactUploaderFactoryDelegate {
                remote: remote.cloned(),
            }),
        )
        .build()
}

/// `RemoteModule.BuildEventArtifactUploaderFactoryDelegate`.
// Java: com/google/devtools/build/lib/remote/RemoteModule.java:1345 RemoteModule.BuildEventArtifactUploaderFactoryDelegate
struct BuildEventArtifactUploaderFactoryDelegate {
    remote: Option<Arc<RemoteSupport>>,
}

impl BuildEventArtifactUploaderFactory for BuildEventArtifactUploaderFactoryDelegate {
    // Java: com/google/devtools/build/lib/remote/RemoteModule.java:1360 BuildEventArtifactUploaderFactoryDelegate.create
    fn create(&self) -> Arc<dyn BuildEventArtifactUploader> {
        match &self.remote {
            None => Arc::new(LocalFilesArtifactUploader),
            Some(remote) => remote.module.lock().create_build_event_artifact_uploader(),
        }
    }
}

/// `RemoteModule.knownMissingCasDigests` (filled by `RemoteExecutionService.onLostInputs`,
/// cleared by `onBuildComplete` after a successful build), as far as it outlives one command:
/// the digests (`hash/size`) of the blobs a build found lost, so that the attempt
/// `BlazeCommandDispatcher` retries does not accept the stale action results that refer to
/// them.
///
/// PORT: Java's `RemoteModule` lives as long as the server and keeps the set across commands;
/// Blaze creates a `RemoteModule` for each command and runs one command per process, so the
/// set is kept here for the process (the dispatcher's retries run in it) and given to the
/// `RemoteExecutionService` of each new module.
// Java: com/google/devtools/build/lib/remote/RemoteModule.java:146 RemoteModule.knownMissingCasDigests
static KNOWN_MISSING_CAS_DIGESTS: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// `RemoteModule.executorInit` when no cache and no remote execution is enabled: `setup` left
/// the placeholder action context provider (`RemoteActionContextProvider.createForPlaceholder`),
/// so `executorInit` does not return early and still sets the action execution salt,
/// `computeActionExecutionSalt(remoteOptions)` (a fingerprint of `false` and the default exec
/// properties), for every command that has `RemoteOptions`. Only a command without
/// `RemoteOptions` (`remoteOptions == null`, `actionContextProvider == null`) keeps the
/// executor's empty salt.
// Java: com/google/devtools/build/lib/remote/RemoteModule.java:416 RemoteModule.setup
// Java: com/google/devtools/build/lib/remote/RemoteModule.java:1241 RemoteModule.executorInit
// Java: com/google/devtools/build/lib/remote/RemoteModule.java:1241 RemoteModule.executorInit
pub(crate) fn placeholder_contributions(
    remote_options: Option<&RemoteOptions>,
) -> RemoteModuleContributions {
    RemoteModuleContributions {
        action_execution_salt: remote_options
            .map(|o| RemoteModule::compute_action_execution_salt(Some(o))),
        ..RemoteModuleContributions::default()
    }
}

impl RemoteSupport {
    /// The build's Skymeld decision (`SkymeldModule.beforeCommand`), which Java makes before
    /// `registerActionContexts` and Blaze after it (see
    /// `ImportantOutputHandler::is_registered`).
    pub(crate) fn set_merged_analysis_and_execution(&self, merged: bool) {
        self.merged_analysis_and_execution
            .store(merged, std::sync::atomic::Ordering::SeqCst);
    }

    /// `RemoteModule.registerSpawnStrategies` (`registerRemoteSpawnStrategy`),
    /// `registerActionContexts` (`registerSpawnCache`) and `executorInit`
    /// (`setActionInputPrefetcher`, `setActionExecutionSalt`), as the executor's contributions.
    // Java: com/google/devtools/build/lib/remote/RemoteModule.java:1176 RemoteModule.registerSpawnStrategies
    // Java: com/google/devtools/build/lib/remote/RemoteModule.java:1190 RemoteModule.registerActionContexts
    // Java: com/google/devtools/build/lib/remote/RemoteModule.java:1241 RemoteModule.executorInit
    pub(crate) fn contributions(
        &self,
        remote_options: Option<&RemoteOptions>,
        finalize_actions: bool,
        rewind_lost_inputs: bool,
        remote_retry_on_transient_cache_error: i32,
    ) -> RemoteModuleContributions {
        let module = self.module.lock();
        let Some(provider) = module.get_action_context_provider() else {
            return placeholder_contributions(remote_options);
        };
        let (strategy, _runner) = provider.get_remote_spawn_strategy();
        let remote_spawn_strategy: Arc<dyn SpawnStrategy> = Arc::new(strategy);
        let action_input_prefetcher = module
            .get_action_input_fetcher()
            .map(|f| f.clone() as Arc<dyn ActionInputPrefetcher>);
        RemoteModuleContributions {
            remote_spawn_strategy: Some(remote_spawn_strategy),
            spawn_cache: Some(provider.get_spawn_cache()),
            action_input_prefetcher,
            remote_output_tree: module.get_remote_output_tree().cloned(),
            action_execution_salt: Some(RemoteModule::compute_action_execution_salt(
                remote_options,
            )),
            // RemoteModule.setup created a RemoteOutputService (no --experimental_remote_output_
            // service: that one is unported) and executorInit gave it the RemoteOutputChecker.
            // Java: com/google/devtools/build/lib/remote/RemoteModule.java:1241 RemoteModule.executorInit
            output_checker: module
                .get_remote_output_checker()
                .map(|c| c.clone() as Arc<dyn blaze_exec::actions::OutputChecker>),
            // SkyframeActionExecutor: `if (finalizeActions) outputService.finalizeAction(..)`;
            // RemoteOutputService.finalizeAction calls the action input fetcher's.
            // Java: com/google/devtools/build/lib/remote/RemoteOutputService.java:170 RemoteOutputService.finalizeAction
            // SkyframeActionExecutor.prepareForExecution: rewindingEnabled and
            // invocationRetriesEnabled; LostInputsEvent goes to RemoteExecutionService
            // .onLostInputs (and to the attempts the dispatcher retries).
            // Java: com/google/devtools/build/lib/skyframe/SkyframeActionExecutor.java:331 SkyframeActionExecutor.prepareForExecution
            // Java: com/google/devtools/build/lib/remote/RemoteExecutionService.java:2105 RemoteExecutionService.onLostInputs
            lost_inputs_policy: Some({
                let service = provider.get_remote_execution_service();
                let lease_service = self.lease_service.clone();
                blaze_exec::exec::executor_setup::LostInputsPolicy {
                    rewinding_enabled: rewind_lost_inputs,
                    invocation_retries_enabled: remote_retry_on_transient_cache_error > 0,
                    on_lost_inputs: Arc::new(move |missing_digests: &[String]| {
                        service.on_lost_inputs(missing_digests);
                        if let Some(lease_service) = &lease_service {
                            lease_service.on_lost_inputs();
                        }
                        KNOWN_MISSING_CAS_DIGESTS
                            .lock()
                            .extend(missing_digests.iter().cloned());
                    }),
                }
            }),
            // For skymeld, a non-toplevel target might become a toplevel after it has been
            // executed. This is the last chance to download the missing toplevel outputs in this
            // case before sending out TargetCompleteEvent. See
            // https://github.com/bazelbuild/bazel/issues/20737.
            // Java: com/google/devtools/build/lib/remote/RemoteModule.java:1202 RemoteModule.registerActionContexts
            important_output_handler: module
                .get_action_input_fetcher()
                .zip(module.get_remote_output_checker())
                .map(|(fetcher, checker)| {
                    Arc::new(
                        blaze_remote::remote_important_output_handler::RemoteImportantOutputHandler::new(
                            checker.clone(),
                            fetcher.clone(),
                            self.merged_analysis_and_execution.clone(),
                        ),
                    )
                        as Arc<
                            dyn blaze_exec::exec::action_execution_context::ImportantOutputHandler,
                        >
                }),
            action_finalizer: if finalize_actions {
                module.get_action_input_fetcher().map(|f| {
                    f.clone()
                        as Arc<dyn blaze_exec::exec::action_execution_context::ActionFinalizer>
                })
            } else {
                None
            },
        }
    }

    /// `afterAnalysis(env, request, buildOptions, analysisResult)` (`merged` false) or, on the
    /// Skymeld path, the `afterTopLevelTargetAnalysis` / `afterTestAnalyzedEvent` calls of each
    /// analyzed top-level target (`merged` true). `output_groups` is the top-level artifact
    /// context's `outputGroups()`.
    // Java: com/google/devtools/build/lib/remote/RemoteModule.java:1069 RemoteModule.afterAnalysis
    // Java: com/google/devtools/build/lib/remote/RemoteModule.java:1035 RemoteModule.afterTopLevelTargetAnalysis
    pub(crate) fn after_analysis(
        &self,
        analysis_result: &AnalysisResult,
        output_groups: &[String],
        merged: bool,
        // `analysisResult.getArtifactsToBuild()` as read here: the coverage report artifacts
        // `BuildView.update` added (the other artifacts to build are not coverage outputs).
        coverage_artifacts_to_build: &[Artifact],
    ) {
        let module = self.module.lock();
        let Some(checker) = module.get_remote_output_checker() else {
            return;
        };
        let targets: Vec<(Option<String>, ConfiguredTargetView<'_>)> = analysis_result
            .get_targets_to_build()
            .iter()
            .map(|target| {
                let rule_class = target
                    .is_rule_configured_target()
                    .then(|| blaze_analysis::configuredtargets::get_rule_class_string(target));
                (
                    rule_class,
                    ConfiguredTargetView {
                        target,
                        output_groups,
                    },
                )
            })
            .collect();
        let tests: Option<Vec<TestTargetView<'_>>> =
            analysis_result.get_targets_to_test().map(|tests| {
                tests
                    .iter()
                    .map(|target| TestTargetView {
                        test_provider: target
                            .get_provider::<TestProvider>()
                            .expect("NullPointerException: TestProvider"),
                    })
                    .collect()
            });
        // PORT: top-level aspects are not ported (AnalysisResult.getAspectsMap is empty).
        let is_owned_by_coverage_report = is_owned_by_coverage_report;
        if merged {
            for (rule_class, view) in &targets {
                checker.after_top_level_target_analysis(rule_class.as_deref(), view);
            }
            if let Some(tests) = &tests {
                for test in tests {
                    checker.after_test_analyzed_event(test);
                }
            }
            return;
        }
        let artifacts_to_build: Vec<ActionInputRef> = coverage_artifacts_to_build
            .iter()
            .map(artifact_input)
            .collect();
        checker.after_analysis(
            &AnalysisResultView {
                targets_to_build: targets
                    .iter()
                    .map(|(rule_class, view)| (rule_class.as_deref(), view as &dyn TopLevelTarget))
                    .collect(),
                aspects: Vec::new(),
                targets_to_test: tests
                    .as_ref()
                    .map(|tests| tests.iter().map(|t| t as &dyn TargetUnderTest).collect()),
                // Only read for coverage (maybeAddCoverageArtifacts).
                artifacts_to_build,
            },
            &is_owned_by_coverage_report,
        );
    }

    /// `RemoteModule.coverageArtifactsKnown(coverageArtifacts)` (the `CoverageArtifactsKnownEvent`
    /// of the merged analysis and execution path).
    // Java: com/google/devtools/build/lib/remote/RemoteModule.java:1062 RemoteModule.coverageArtifactsKnown
    pub(crate) fn coverage_artifacts_known(&self, coverage_artifacts: &[Artifact]) {
        let module = self.module.lock();
        let Some(checker) = module.get_remote_output_checker() else {
            return;
        };
        let inputs: Vec<ActionInputRef> = coverage_artifacts.iter().map(artifact_input).collect();
        checker.coverage_artifacts_known(&inputs, &is_owned_by_coverage_report);
    }

    /// `env.getSkyframeExecutor()::wasActionRewound`, the `wasRewound` predicate
    /// `RemoteActionContextProvider` gives the `RemoteExecutionService`, once the command's
    /// Skyframe executor exists.
    // Java: com/google/devtools/build/lib/remote/RemoteActionContextProvider.java:156 RemoteActionContextProvider.getRemoteExecutionService
    pub(crate) fn set_was_rewound(
        &self,
        executor: Arc<blaze_sky_exec::skyframe_action_executor::SkyframeActionExecutor>,
    ) {
        if let Some(provider) = self.module.lock().get_action_context_provider() {
            provider
                .get_remote_execution_service()
                .set_was_rewound(Arc::new(move |owner| {
                    executor.was_rewound_resource_owner(owner)
                }));
        }
    }

    /// `RemoteExecutionService.onBuildInterrupted(BuildInterruptedEvent)`.
    // Java: com/google/devtools/build/lib/remote/RemoteExecutionService.java:2091 RemoteExecutionService.onBuildInterrupted
    pub(crate) fn build_interrupted(&self) {
        if let Some(provider) = self.module.lock().get_action_context_provider() {
            provider
                .get_remote_execution_service()
                .on_build_interrupted();
        }
    }

    /// `RemoteExecutionService.onBuildComplete(BuildCompleteEvent)`.
    // Java: com/google/devtools/build/lib/remote/RemoteExecutionService.java:2096 RemoteExecutionService.onBuildComplete
    /// `RemoteOutputService.onExecutionPhaseCompleteEvent`: `leaseService.finalizeExecution()`
    /// (see `LeaseService::finalize_execution` for why it runs before the action cache is
    /// saved).
    // Java: com/google/devtools/build/lib/remote/RemoteOutputService.java:163 RemoteOutputService.onExecutionPhaseCompleteEvent
    pub(crate) fn finalize_execution(
        &self,
        action_cache: Option<&dyn blaze_exec::actions::cache::ActionCache>,
        memoizing_evaluator: &blaze_skyframe::Evaluator,
    ) {
        if let Some(lease_service) = &self.lease_service {
            lease_service.finalize_execution(action_cache, &|| {
                delete_remote_action_execution_values(memoizing_evaluator)
            });
        }
    }

    pub(crate) fn build_complete(&self, success: bool) {
        if success {
            // RemoteExecutionService.onBuildComplete clears knownMissingCasDigests.
            KNOWN_MISSING_CAS_DIGESTS.lock().clear();
        }
        if let Some(provider) = self.module.lock().get_action_context_provider() {
            provider
                .get_remote_execution_service()
                .on_build_complete(success);
        }
    }

    /// `RemoteModule.afterCommand()` (waits for the background uploads, closes
    /// `--remote_grpc_log`, deletes the downloaded inputs). An error is reported as Java's
    /// `BlazeRuntime.afterCommand` does: `ERROR: <message>`, and returned.
    // Java: com/google/devtools/build/lib/remote/RemoteModule.java:1098 RemoteModule.afterCommand
    // Java: com/google/devtools/build/lib/runtime/BlazeRuntime.java:750 BlazeRuntime.afterCommand
    pub(crate) fn after_command(&self, reporter: &dyn EventHandler) -> Option<AbruptExitException> {
        match self.module.lock().after_command() {
            Ok(()) => None,
            Err(e) => {
                reporter.handle(&Event::error(e.get_message().to_owned()));
                Some(e)
            }
        }
    }
}

/// A top-level configured target as `RemoteOutputChecker.addTopLevelTarget` reads it, with the
/// build's `TopLevelArtifactContext`.
struct ConfiguredTargetView<'a> {
    target: &'a ConfiguredTarget,
    output_groups: &'a [String],
}

fn inputs_of<'a>(artifacts: impl IntoIterator<Item = &'a Artifact>) -> Vec<ActionInputRef> {
    artifacts.into_iter().map(artifact_input).collect()
}

impl TopLevelTarget for ConfiguredTargetView<'_> {
    // Java: com/google/devtools/build/lib/analysis/TopLevelArtifactHelper.java:97 ArtifactsToBuild.getImportantArtifacts
    fn get_important_artifacts_to_build(&self) -> Vec<ActionInputRef> {
        let all = get_all_artifacts_to_build(self.target);
        let mut important = Vec::new();
        for output_group in self.output_groups {
            if let Some(artifacts) = all.get(output_group)
                && is_important_group(output_group)
            {
                important.extend(inputs_of(artifacts.to_list().iter()));
            }
        }
        important
    }

    fn output_groups_contain_hidden_top_level(&self) -> bool {
        self.output_groups.iter().any(|g| g == HIDDEN_TOP_LEVEL)
    }

    fn get_runfiles(&self) -> Option<RunfilesView> {
        let provider = self.target.get_provider::<FilesToRunProvider>()?;
        let runfiles_support = provider.get_runfiles_support()?;
        let runfiles = runfiles_support.get_runfiles();
        Some(RunfilesView {
            artifacts: inputs_of(runfiles.get_artifacts().to_list().iter()),
            symlink_artifacts: runfiles
                .get_symlinks()
                .to_list()
                .iter()
                .map(|s| artifact_input(s.get_artifact()))
                .collect(),
            root_symlink_artifacts: runfiles
                .get_root_symlinks()
                .to_list()
                .iter()
                .map(|s| artifact_input(s.get_artifact()))
                .collect(),
        })
    }

    fn get_extra_action_artifacts(&self) -> Option<Vec<ActionInputRef>> {
        // PORT: ExtraActionArtifactsProvider is not ported; its artifacts come from action
        // listeners (`--experimental_action_listener`), which Blaze does not run, so the
        // provider would be empty.
        None
    }
}

/// A test target as `RemoteOutputChecker.addTargetUnderTest` reads it.
struct TestTargetView<'a> {
    test_provider: &'a TestProvider,
}

impl TargetUnderTest for TestTargetView<'_> {
    fn get_test_outputs(&self) -> Vec<ActionInputRef> {
        self.test_provider
            .get_test_params()
            .get_outputs()
            .iter()
            .map(|output| match output {
                TestOutput::Artifact(artifact) => artifact_input(artifact),
                // ActionInputHelper.fromPath(path)
                TestOutput::Path(path) => {
                    ActionInputRef::new(BasicActionInput::from_path(path.get_path_string()))
                }
            })
            .collect()
    }

    fn get_coverage_artifacts(&self) -> Vec<ActionInputRef> {
        inputs_of(
            self.test_provider
                .get_test_params()
                .get_coverage_artifacts()
                .iter(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Review 2026-11-19 R6: an attempt retried after a remote cache eviction keeps the build
    /// request id of the attempt it retries; `--build_request_id` wins; a new command starts
    /// without the override.
    // Java: com/google/devtools/build/lib/runtime/BlazeCommandDispatcher.java:162 BlazeCommandDispatcher.exec
    #[test]
    fn retried_attempt_keeps_the_build_request_id() {
        reset_build_request_id_override();
        let first = build_request_id_for(None);
        retry_with_build_request_id_of_last_attempt();
        assert_eq!(build_request_id_for(None), first);
        retry_with_build_request_id_of_last_attempt();
        assert_eq!(build_request_id_for(None), first);
        assert_eq!(
            build_request_id_for(Some("explicit".to_string())),
            "explicit"
        );
        reset_build_request_id_override();
        assert_ne!(build_request_id_for(None), first);
    }
}

/// `artifact.getArtifactOwner().equals(CoverageReportValue.COVERAGE_REPORT_KEY)`.
// Java: com/google/devtools/build/lib/remote/RemoteOutputChecker.java:141 RemoteOutputChecker.maybeAddCoverageArtifacts
fn is_owned_by_coverage_report(input: &ActionInputRef) -> bool {
    blaze_exec::actions::action_adapters::as_blaze_artifact(input).is_some_and(|artifact| {
        matches!(
            artifact.get_artifact_owner(),
            blaze_actions::actions::action_lookup::ArtifactOwner::ActionLookupKey(key)
                if key == blaze_analysis::coverage_report_value::CoverageReportKey::action_lookup_key()
        )
    })
}
