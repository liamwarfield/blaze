//! The `RemoteModule` hooks of `blaze build` / `blaze test` (`beforeCommand`,
//! `registerSpawnStrategies` / `registerActionContexts` / `executorInit`, `afterAnalysis` /
//! `afterTopLevelTargetAnalysis`, the `RemoteExecutionService` subscriptions to
//! `BuildCompleteEvent` / `BuildInterruptedEvent`, and `afterCommand`), on top of
//! `blaze_remote::remote_module`.
//!
//! PORT: Java's `BlazeCommandDispatcher` / `BlazeRuntime` call the module hooks for every
//! command; Blaze calls them from the build command (`crate::commands::build`). Other commands
//! that accept the remote options do not set the module up (`crate::unported_options` fails
//! loudly for the ones that execute actions). A command without any remote option keeps the
//! placeholder provider of `blaze_exec::exec::executor_setup` and does not call the module.

use std::sync::Arc;

use blaze_actions::Artifact;
use blaze_analysis::configured_target::ConfiguredTarget;
use blaze_analysis::providers::file_provider::FilesToRunProvider;
use blaze_analysis::providers::output_group_info::HIDDEN_TOP_LEVEL;
use blaze_analysis::test::test_provider::{TestOutput, TestProvider};
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
}

/// Whether any remote functionality is requested: `RemoteModule.setup` returns early (the
/// placeholder provider, which `blaze_exec::exec::executor_setup` ports) otherwise.
// Java: com/google/devtools/build/lib/remote/RemoteModule.java:526 RemoteModule.setup
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
    let Some(remote_options) = options.get_options::<RemoteOptions>() else {
        // Quit if no supported command is being used.
        return Ok(None);
    };
    if !remote_requested(&remote_options) {
        // `setup` still runs `tempPathGenerator = getTempPathGenerator(env)` before it returns
        // with the placeholder provider ("Found stale downloads from previous build,
        // deleting..."); `afterCommand` deletes the directory again (see
        // `placeholder_after_command`).
        // Java: com/google/devtools/build/lib/remote/RemoteModule.java:524 RemoteModule.setup
        let exec_root = env.get_exec_root();
        blaze_remote::remote_module::get_temp_path_generator(
            &env.get_directories().get_action_temps_directory(&exec_root),
            env.get_reporter().as_ref(),
        )?;
        return Ok(None);
    }
    if remote_options.scrubber.is_some() {
        // RemoteOptions.ScrubberConverter parses the file (Scrubber.parse, TextFormat).
        blaze_util::unported!("Scrubber.parse", "--experimental_remote_scrubbing_config");
    }
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
    let reporter: Arc<dyn ExtendedEventHandler> = env.get_reporter().clone();
    let lease_extension_requested = remote_options.remote_cache_lease_extension;
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
        workspace_name: env.get_workspace_name().to_owned(),
        action_temps_directory: directories.get_action_temps_directory(&exec_root),
        build_request_id,
        command_id: env.get_command_id().to_owned(),
        product_name: env.get_runtime().get_product_name().to_owned(),
        tool_version: BlazeVersionInfo::instance().get_version(),
        client_env: env.get_client_env().clone(),
        clock: Arc::new(JavaClock),
        scrubber: None,
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
    // Java: com/google/devtools/build/lib/remote/RemoteModule.java:1259 RemoteModule.executorInit
    let rewind_lost_inputs = build_request_options
        .as_ref()
        .is_some_and(|o| o.rewind_lost_inputs);
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
    }))
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
// Java: com/google/devtools/build/lib/runtime/CommandEnvironment.java:335 CommandEnvironment.<init>
fn build_request_id(options: &OptionsParsingResult) -> String {
    build_request_id_for(
        options
            .get_options::<CommonCommandOptions>()
            .and_then(|o| o.build_request_id.clone()),
    )
}

/// [`build_request_id`] given `--build_request_id`.
fn build_request_id_for(explicit: Option<String>) -> String {
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
// Java: com/google/devtools/build/lib/runtime/BlazeCommandDispatcher.java:292 BlazeCommandDispatcher.exec
static BUILD_REQUEST_ID_OVERRIDE: Mutex<Option<String>> = Mutex::new(None);

/// The build request id of the last attempt that set the remote module up (`env.getBuildRequestId()`
/// of `new RemoteCacheTransientErrorException(env.getBuildRequestId(), env.getCommandId())`).
static LAST_BUILD_REQUEST_ID: Mutex<Option<String>> = Mutex::new(None);

/// `buildRequestIdOverride = null` at the start of `BlazeCommandDispatcher.exec`.
// Java: com/google/devtools/build/lib/runtime/BlazeCommandDispatcher.java:292 BlazeCommandDispatcher.exec
pub(crate) fn reset_build_request_id_override() {
    *BUILD_REQUEST_ID_OVERRIDE.lock() = None;
    *LAST_BUILD_REQUEST_ID.lock() = None;
}

/// `catch (RemoteCacheTransientErrorException e) { buildRequestIdOverride = e.getBuildRequestId(); }`.
// Java: com/google/devtools/build/lib/runtime/BlazeCommandDispatcher.java:319 BlazeCommandDispatcher.exec
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

/// `BuildEventServiceModule.createBepTransports`'s `uploaderSupplier.get()` for each build
/// event file transport (text, binary, JSON) whose `--build_event_*_file_path_conversion` is on:
/// the uploader factory that `--experimental_build_event_upload_strategy` selects. Bazel
/// registers one factory, RemoteModule's "remote" (`serverInit`), which `select(null)` also
/// picks; any other name is the local uploader. The remote factory's `create` is
/// `RemoteModule::create_build_event_artifact_uploader` (the ByteStream uploader with a gRPC
/// cache, which fails loudly: Bazel writes `bytestream://` URIs there, Blaze's BEP only has the
/// local uploader). Without a gRPC cache it is the local uploader the transports already have.
///
/// PORT: Java creates the uploader while it creates the transports; `blaze_bep` takes no
/// uploader from outside, so this runs right after the transports are created. The
/// connectivity check (`ConnectivityStatus`, "local" when not OK) is always OK in Bazel.
// Java: com/google/devtools/build/lib/buildeventservice/BuildEventServiceModule.java:366 BuildEventServiceModule.beforeCommand
// Java: com/google/devtools/build/lib/buildeventservice/BuildEventServiceModule.java:827 BuildEventServiceModule.createBepTransports
// Java: com/google/devtools/build/lib/runtime/BuildEventArtifactUploaderFactoryMap.java:33 BuildEventArtifactUploaderFactoryMap.select
pub(crate) fn create_bep_file_transport_uploaders(
    remote: &RemoteSupport,
    options: &OptionsParsingResult,
) {
    use blaze_optdefs::misc::build_event_protocol_options::BuildEventProtocolOptions;
    use blaze_optdefs::misc::build_event_stream_options::BuildEventStreamOptions;
    let Some(bep_options) = options.get_options::<BuildEventProtocolOptions>() else {
        return;
    };
    let Some(bes_stream_options) = options.get_options::<BuildEventStreamOptions>() else {
        return;
    };
    let remote_factory_selected = match bep_options.build_event_upload_strategy.as_deref() {
        None => true,
        Some(name) => name == "remote",
    };
    if !remote_factory_selected {
        return;
    }
    let uses_uploader = |file: &str, path_conversion: bool| !file.is_empty() && path_conversion;
    if uses_uploader(
        &bes_stream_options.build_event_text_file,
        bes_stream_options.build_event_text_file_path_conversion,
    ) || uses_uploader(
        &bes_stream_options.build_event_binary_file,
        bes_stream_options.build_event_binary_file_path_conversion,
    ) || uses_uploader(
        &bes_stream_options.build_event_json_file,
        bes_stream_options.build_event_json_file_path_conversion,
    ) {
        let _local_uploader: Option<std::convert::Infallible> =
            remote.module.lock().create_build_event_artifact_uploader();
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
// Java: com/google/devtools/build/lib/remote/RemoteModule.java:526 RemoteModule.setup
// Java: com/google/devtools/build/lib/remote/RemoteModule.java:1241 RemoteModule.executorInit
// Java: com/google/devtools/build/lib/remote/RemoteModule.java:1308 RemoteModule.executorInit
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
            // Java: com/google/devtools/build/lib/remote/RemoteModule.java:1296 RemoteModule.executorInit
            output_checker: module
                .get_remote_output_checker()
                .map(|c| c.clone() as Arc<dyn blaze_exec::actions::OutputChecker>),
            // SkyframeActionExecutor: `if (finalizeActions) outputService.finalizeAction(..)`;
            // RemoteOutputService.finalizeAction calls the action input fetcher's.
            // Java: com/google/devtools/build/lib/remote/RemoteOutputService.java:170 RemoteOutputService.finalizeAction
            // SkyframeActionExecutor.prepareForExecution: rewindingEnabled and
            // invocationRetriesEnabled; LostInputsEvent goes to RemoteExecutionService
            // .onLostInputs (and to the attempts the dispatcher retries).
            // Java: com/google/devtools/build/lib/skyframe/SkyframeActionExecutor.java:355 SkyframeActionExecutor.prepareForExecution
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
        // PORT: top-level aspects are not ported (AnalysisResult.getAspectsMap is empty) and
        // coverage (`--collect_code_coverage`) fails before the build; `CoverageReportValue
        // .COVERAGE_REPORT_KEY` owns no artifact.
        let is_owned_by_coverage_report = |_: &ActionInputRef| false;
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
        let artifacts_to_build: Vec<ActionInputRef> = Vec::new();
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

    /// `RemoteExecutionService.onBuildInterrupted(BuildInterruptedEvent)`.
    // Java: com/google/devtools/build/lib/remote/RemoteExecutionService.java:2090 RemoteExecutionService.onBuildInterrupted
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
    // Java: com/google/devtools/build/lib/remote/RemoteOutputService.java:162 RemoteOutputService.onExecutionPhaseCompleteEvent
    pub(crate) fn finalize_execution(
        &self,
        action_cache: Option<&dyn blaze_exec::actions::cache::ActionCache>,
    ) {
        if let Some(lease_service) = &self.lease_service {
            lease_service.finalize_execution(action_cache);
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
    // Java: com/google/devtools/build/lib/remote/RemoteModule.java:1097 RemoteModule.afterCommand
    // Java: com/google/devtools/build/lib/runtime/BlazeRuntime.java:775 BlazeRuntime.afterCommand
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
    // Java: com/google/devtools/build/lib/runtime/BlazeCommandDispatcher.java:316 BlazeCommandDispatcher.exec
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
