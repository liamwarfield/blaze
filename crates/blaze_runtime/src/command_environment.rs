//! `CommandEnvironment`: the state of one command invocation.
//!
//! PORT: W1 subset. Skyframe access, the event bus, downloader, repo env, resource manager,
//! output service and timestamps are added with the waves that need them.

use std::sync::Arc;

use blaze_events::{EventHandler, OutErr, Reporter};
use blaze_optdefs::command::common_command_options::CommonCommandOptions;
use blaze_options::invocation_policy::OptionAndRawValue;
use blaze_options::rc::client_options::ClientOptions;
use blaze_protos::blaze::invocation_policy::InvocationPolicy;
use blaze_protos::failure_details::{self, FailureDetail};
use blaze_vfs::{Path, PathFragment};
use indexmap::IndexMap;

use crate::command::CommandAnnotation;
use crate::directories::BlazeDirectories;
use crate::exit_code::DetailedExitCode;
use crate::module::AbruptExitException;
use crate::options_result::OptionsParsingResult;
use crate::runtime::BlazeRuntime;
use crate::workspace::BlazeWorkspace;

/// Encapsulates the state needed for a single command.
// Java: com/google/devtools/build/lib/runtime/CommandEnvironment.java:105 CommandEnvironment
// The getters of Java's CommandEnvironment for the server's SkyframeExecutor and the objects
// built with it: Blaze builds them per command where the command needs them
// (`commands::query::create_skyframe_executor`, the build command's execution setup in
// `commands/build.rs`, the repository setup of `blaze_repo_rules::setup`).
// PORT: skipped, built per command: CommandEnvironment.getSkyframeExecutor, CommandEnvironment.getPackageManager
// PORT: skipped, built per command: CommandEnvironment.getSkyframeBuildView, CommandEnvironment.getPackageLocator
// PORT: skipped, built per command: CommandEnvironment.getRepoContentsCachePath
// PORT: skipped, built per command: CommandEnvironment.getActionTempsDirectory, CommandEnvironment.getOutputService
// PORT: skipped, built per command: CommandEnvironment.getWorkspaceInfoFromDiff, CommandEnvironment.getLocalResourceManager
// PORT: skipped, built per command: CommandEnvironment.recordLastExecutionTime
// PORT: skipped, built per command: CommandEnvironment.determineOutputFileSystem, CommandEnvironment.getOutputDirectoryHelper
// PORT: skipped, built per command: CommandEnvironment.getSyscallCache, CommandEnvironment.getXattrProvider
// PORT: skipped, built per command: CommandEnvironment.getQuiescingExecutors, CommandEnvironment.getBuildResultListener
// PORT: skipped, built per command: CommandEnvironment.getCommandLinePathFactory
// PORT: skipped, built per command: CommandEnvironment.additionalConfigurationChangeEvent
// PORT: skipped, built per command: CommandEnvironment.getAdditionalConfigurationChangeEvent
// PORT: skipped, built per command: CommandEnvironment.getHttpDownloader, CommandEnvironment.getDownloaderDelegate
// PORT: skipped, built per command: CommandEnvironment.getClock, CommandEnvironment.getBuildRequestId
// PORT: skipped, built per command: CommandEnvironment.getVersionGetter, CommandEnvironment.setVersionGetter
// PORT: skipped, built per command: CommandEnvironment.getUseFakeStampData, CommandEnvironment.setUseFakeStampData
// Getters that no Bazel code reads (Google-internal extensions and services):
// PORT: skipped, no reader in Bazel: CommandEnvironment.getIdleTaskResultsFromPreviousIdlePeriod
// PORT: skipped, no reader in Bazel: CommandEnvironment.getStartupOptionsProvider, CommandEnvironment.getFileFromWorkspace
// PORT: skipped, no reader in Bazel: CommandEnvironment.getCommandExtensions, CommandEnvironment.getCommandExtensionReporter
// PORT: skipped, remote analysis caching (out of scope): CommandEnvironment.getRemoteAnalysisCachingEventListener
pub struct CommandEnvironment {
    /// Java `eventBus`.
    event_bus: Arc<CommandEventBus>,
    /// Java `invocationPolicy`: the policy the client sent (not merged with the modules' one).
    invocation_policy: InvocationPolicy,
    /// Java `invocationPolicyFlags`.
    invocation_policy_flags: Vec<OptionAndRawValue>,
    /// Java `responseExtensions`. Shared, so that a handler that outlives a borrow of the
    /// environment (`RemoteRequestedInfoItemHandler`) can add to it.
    response_extensions: Arc<parking_lot::Mutex<Vec<blaze_protos::google::protobuf::Any>>>,
    /// The client's command line (`BlazeCommandDispatcher.execExclusively`'s `args`).
    client_args: Vec<String>,
    /// The client's startup options tagged with the rc file that set them.
    startup_options_tagged_with_bazel_rc: Option<Vec<(String, String)>>,
    /// Java `repoEnv` / `nonstrictRepoEnv`.
    repo_env: IndexMap<String, String>,
    nonstrict_repo_env: IndexMap<String, String>,
    runtime: Arc<BlazeRuntime>,
    workspace: Arc<BlazeWorkspace>,
    directories: BlazeDirectories,
    reporter: Arc<Reporter>,
    command: &'static CommandAnnotation,
    options: Arc<OptionsParsingResult>,
    working_directory: Path,
    relative_working_directory: PathFragment,
    client_env: IndexMap<String, String>,
    command_id: String,
    /// Java: `attemptNumber` (1 for the first attempt; the dispatcher retries a build after a
    /// transient remote cache error).
    attempt_number: i32,
    visible_action_env: std::collections::BTreeSet<String>,
    visible_test_env: std::collections::BTreeSet<String>,
    /// `blazeModuleEnvironment`: the module environment, whose `exit` sets the pending
    /// exception (`pendingException`) and interrupts the command thread.
    blaze_module_environment: BlazeModuleEnvironment,
    has_synced_package_loading: bool,
    /// Java `configFlagDefinitions`: the `--config` definitions of the rc files for this
    /// command.
    config_flag_definitions: Arc<blaze_sky_analysis::flag_set_function::ConfigFlagDefinitions>,
    ui_event_handler: Option<Arc<blaze_events::ui::UiEventHandler>>,
    /// The `@Subscribe` methods of `ui_event_handler`, registered on the event bus with it.
    ui_event_subscriber:
        Option<Arc<blaze_buildtool::runtime::ui_event_subscriber::UiEventSubscriber>>,
    /// The Build Event Protocol of this command, set by `BuildEventServiceModule.beforeCommand`
    /// (`crate::bazel_modules::BazelBuildEventServiceModule`) when the command streams build
    /// events. PORT: Java's commands reach the streamer through the event bus only; the posting
    /// sites that are not subscribers in Blaze (the dispatcher, `run_build`) reach it here.
    bep: Option<Arc<crate::commands::bep::BepCommandSupport>>,
    shutdown_reason: Option<String>,
    /// `SequencedSkyframeExecutor.trackIncrementalState`, set by
    /// [`CommandEnvironment::decide_keep_incremental_state`].
    track_incremental_state: bool,
    /// Java `commandStartTime`, in milliseconds since the epoch.
    command_start_time: i64,
    /// Java `waitTime`: the time this command waited for the command lock plus the client's
    /// `--command_wait_time` (`CommonCommandOptions.waitTime`), in milliseconds.
    wait_time_millis: i64,
    /// Java `idleTasks`: the tasks to run during the idle period following this command.
    /// Behind a lock so that the modules' hooks, which see the environment shared, can add.
    idle_tasks: parking_lot::Mutex<Vec<Arc<dyn blaze_server::IdleTask>>>,
    /// Java `timestampGranularityMonitor`.
    timestamp_granularity_monitor:
        Arc<blaze_util::io::timestamp_granularity_monitor::TimestampGranularityMonitor>,
}

/// The state behind `pendingException`: `None` if neither [`BlazeModuleEnvironment::exit`] nor
/// [`CommandEnvironment::precomplete_command`] has been called; `Some(Some(e))` if `exit` has
/// been called with `e`; `Some(None)` if `precomplete_command` was called before any call to
/// `exit`.
// Java: com/google/devtools/build/lib/runtime/CommandEnvironment.java:164 CommandEnvironment.pendingException
type PendingException = parking_lot::Mutex<Option<Option<AbruptExitException>>>;

/// The environment modules use to request an abrupt exit of the running command, from any
/// thread. Cloning shares the command's state.
// Java: com/google/devtools/build/lib/runtime/CommandEnvironment.java:195 CommandEnvironment.BlazeModuleEnvironment
#[derive(Clone)]
pub struct BlazeModuleEnvironment {
    pending_exception: Arc<PendingException>,
    /// `commandThread`: the thread that runs the command (the one that created the
    /// environment).
    command_thread: blaze_shell::thread_interrupt::ThreadInterrupt,
}

impl BlazeModuleEnvironment {
    /// Exits Blaze as early as possible by sending an interrupt to the command's main thread.
    /// The first exception wins; only the first call interrupts, and only from another thread.
    // Java: com/google/devtools/build/lib/runtime/CommandEnvironment.java:207 BlazeModuleEnvironment.exit
    pub fn exit(&self, exception: AbruptExitException) {
        // checkNotNull(exception), checkNotNull(exception.getExitCode()): both by type.
        let first = {
            let mut pending = self.pending_exception.lock();
            if pending.is_none() {
                *pending = Some(Some(exception));
                true
            } else {
                false
            }
        };
        if first
            && !blaze_shell::thread_interrupt::ThreadInterrupt::current()
                .same_thread(&self.command_thread)
        {
            // There was no exception, so we're the first one to ask for an exit. Interrupt the
            // command if this exit is coming from a different thread, so that the command
            // terminates promptly.
            self.command_thread.interrupt();
        }
    }

    /// `CommandEnvironment.throwPendingException()`, for the code of a command that holds the
    /// module environment rather than the command environment (the build command's
    /// `ExecutionTool` construction).
    // Java: com/google/devtools/build/lib/runtime/CommandEnvironment.java:835 CommandEnvironment.throwPendingException
    pub(crate) fn throw_pending_exception(&self) -> Result<(), AbruptExitException> {
        let pending = self.pending_exception.lock().clone().flatten();
        if let Some(exception) = pending {
            if blaze_shell::thread_interrupt::ThreadInterrupt::current()
                .same_thread(&self.command_thread)
            {
                // Throwing this exception counts as the requested interruption. Clear the
                // interrupted bit.
                blaze_shell::thread_interrupt::interrupted();
            }
            return Err(exception);
        }
        Ok(())
    }
}

impl CommandEnvironment {
    /// Creates a new command environment which can be used for executing commands for the given
    /// runtime in the given workspace.
    // Java: com/google/devtools/build/lib/runtime/CommandEnvironment.java:226 CommandEnvironment.<init>
    pub(crate) fn new(
        runtime: Arc<BlazeRuntime>,
        workspace: Arc<BlazeWorkspace>,
        command: &'static CommandAnnotation,
        options: Arc<OptionsParsingResult>,
        invocation_policy: InvocationPolicy,
        warnings: &mut Vec<String>,
        wait_time_in_ms: i64,
        command_start_time: i64,
        attempt_number: i32,
    ) -> CommandEnvironment {
        blaze_util::check_argument!(attempt_number >= 1);
        let directories = workspace.get_directories().clone();
        // Java: new Reporter(eventBus, ...) with the command's EventBus.
        let event_bus = Arc::new(CommandEventBus::default());
        let reporter = Reporter::new(Some(event_bus.clone() as Arc<dyn blaze_events::EventBus>));

        let command_options = options.get_options::<CommonCommandOptions>().expect(
            "CommandEnvironment needs its options provider to have CommonCommandOptions loaded.",
        );
        // The thread that creates the environment runs the command (Java passes
        // Thread.currentThread() from BlazeWorkspace.initCommand).
        // Java: com/google/devtools/build/lib/runtime/CommandEnvironment.java:266 CommandEnvironment.<init>
        // (runtime.getClock(): BlazeClock).
        let timestamp_granularity_monitor = Arc::new(
            blaze_util::io::timestamp_granularity_monitor::TimestampGranularityMonitor::new(
                Arc::new(crate::profiler_init::ProfilerClock),
            ),
        );
        // Record the command's starting time again, for use by
        // TimestampGranularityMonitor.waitForTimestampGranularity(). This should be done as close
        // as possible to the start of the command's execution.
        timestamp_granularity_monitor.set_command_start_time();
        let blaze_module_environment = BlazeModuleEnvironment {
            pending_exception: Arc::new(parking_lot::Mutex::new(None)),
            command_thread: blaze_shell::thread_interrupt::ThreadInterrupt::current(),
        };
        let working_directory =
            match Self::compute_working_directory(&directories, &command_options) {
                Ok(wd) => wd,
                Err(e) => {
                    blaze_module_environment.exit(e);
                    // We'll exit very soon, but set the working directory to something reasonable so
                    // remainder of setup can finish.
                    directories
                        .get_working_directory()
                        .cloned()
                        .expect("no working directory")
                }
            };
        let relative_working_directory = match directories.get_working_directory() {
            Some(ws) => working_directory.relative_to(ws),
            None => PathFragment::empty_fragment(),
        };

        // Java: com/google/devtools/build/lib/runtime/CommandEnvironment.java:297 CommandEnvironment.<init>
        let wait_time_millis = wait_time_in_ms + command_options.wait_time;
        let command_start_time = command_start_time - command_options.startup_time;

        // PORT: http timeout scaling, downloader setup and the parallel-download check are
        // not ported (no downloader in W1).
        if command_options.http_timeout_scaling <= 0.0 {
            reporter.handle(&blaze_events::Event::warn(
                "Ignoring request to scale http timeouts by a non-positive factor",
            ));
        }

        let client_options = options
            .get_options::<ClientOptions>()
            .expect("CommandEnvironment needs its options provider to have ClientOptions loaded.");
        let client_env = make_map_from_map_entries(&client_options.client_env);
        let command_id =
            Self::compute_command_id(&command_options, &client_env, warnings, attempt_number);
        // PORT: buildRequestId is not ported.
        // Java: ALWAYS_INHERITED_REPO_ENV (Linux: PATH).
        let mut repo_env_builder: std::collections::BTreeMap<String, String> = client_env
            .iter()
            .filter(|(k, _)| !command_options.use_strict_repo_env || k.as_str() == "PATH")
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        let mut nonstrict_repo_env_builder: std::collections::BTreeMap<String, String> = client_env
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        let mut visible_action_env = std::collections::BTreeSet::new();
        let mut visible_test_env = std::collections::BTreeSet::new();
        // TODO: This only needs to check for loads() rather than analyzes() due to the effect of
        // --action_env on the repository env.
        if command.build_phase.loads() || command.name() == "info" {
            // Compute the set of environment variables that are allowlisted on the commandline
            // for inheritance.
            let core_options = options
                .get_options::<blaze_optdefs::fragment::core_options::CoreOptions>()
                .expect("CoreOptions");
            for env_var in &core_options.action_environment {
                match env_var {
                    blaze_options::EnvVar::Set { name, value } => {
                        visible_action_env.remove(name);
                        if !command_options.repo_env_ignores_action_env {
                            repo_env_builder.insert(name.clone(), value.clone());
                            nonstrict_repo_env_builder.insert(name.clone(), value.clone());
                        }
                    }
                    blaze_options::EnvVar::Inherit { name } => {
                        visible_action_env.insert(name.clone());
                    }
                    blaze_options::EnvVar::Unset { name } => {
                        visible_action_env.remove(name);
                        if !command_options.repo_env_ignores_action_env {
                            repo_env_builder.remove(name);
                            nonstrict_repo_env_builder.remove(name);
                        }
                    }
                }
            }
        }
        if command.build_phase.analyzes() || command.name() == "info" {
            let test_options = options
                .get_options::<blaze_optdefs::fragment::test_configuration::TestConfigurationTestOptions>()
                .expect("TestOptions");
            for env_var in &test_options.test_environment {
                if let blaze_options::EnvVar::Inherit { name } = env_var {
                    visible_test_env.insert(name.clone());
                }
            }
        }

        // Java: com/google/devtools/build/lib/runtime/CommandEnvironment.java:384 CommandEnvironment.<init>
        let bazel_workspace = directories
            .get_workspace()
            .map(|w| w.get_path_string().to_string());
        for env_var in &command_options.repository_environment {
            match env_var {
                blaze_options::EnvVar::Set { name, value } => {
                    let value = match &bazel_workspace {
                        Some(w) => value.replace("%bazel_workspace%", w),
                        None => value.clone(),
                    };
                    repo_env_builder.insert(name.clone(), value.clone());
                    nonstrict_repo_env_builder.insert(name.clone(), value);
                }
                blaze_options::EnvVar::Inherit { name } => {
                    if let Some(value) = client_env.get(name) {
                        repo_env_builder.insert(name.clone(), value.clone());
                        nonstrict_repo_env_builder.insert(name.clone(), value.clone());
                    }
                }
                blaze_options::EnvVar::Unset { name } => {
                    repo_env_builder.remove(name);
                    nonstrict_repo_env_builder.remove(name);
                }
            }
        }
        let repo_env: IndexMap<String, String> = repo_env_builder.into_iter().collect();
        let nonstrict_repo_env: IndexMap<String, String> =
            nonstrict_repo_env_builder.into_iter().collect();

        CommandEnvironment {
            event_bus,
            invocation_policy,
            invocation_policy_flags: Vec::new(),
            response_extensions: Arc::default(),
            client_args: Vec::new(),
            startup_options_tagged_with_bazel_rc: None,
            repo_env,
            nonstrict_repo_env,
            runtime,
            workspace,
            directories,
            reporter,
            command,
            options,
            working_directory,
            relative_working_directory,
            client_env,
            command_id,
            attempt_number,
            visible_action_env,
            visible_test_env,
            blaze_module_environment,
            has_synced_package_loading: false,
            config_flag_definitions: Arc::default(),
            ui_event_handler: None,
            ui_event_subscriber: None,
            bep: None,
            shutdown_reason: None,
            // SequencedSkyframeExecutor.trackIncrementalState is true until
            // decideKeepIncrementalState.
            track_incremental_state: true,
            command_start_time,
            wait_time_millis,
            idle_tasks: parking_lot::Mutex::new(Vec::new()),
            timestamp_granularity_monitor,
        }
    }

    /// The command's `TimestampGranularityMonitor`.
    // Java: com/google/devtools/build/lib/runtime/CommandEnvironment.java:700 CommandEnvironment.getTimestampGranularityMonitor
    pub fn get_timestamp_granularity_monitor(
        &self,
    ) -> &Arc<blaze_util::io::timestamp_granularity_monitor::TimestampGranularityMonitor> {
        &self.timestamp_granularity_monitor
    }

    /// Registers a task to be executed during an idle period following this command.
    // Java: com/google/devtools/build/lib/runtime/CommandEnvironment.java:1166 CommandEnvironment.addIdleTask
    pub fn add_idle_task(&self, idle_task: Arc<dyn blaze_server::IdleTask>) {
        self.idle_tasks.lock().push(idle_task);
    }

    /// Returns the list of registered idle tasks.
    // Java: com/google/devtools/build/lib/runtime/CommandEnvironment.java:1171 CommandEnvironment.getIdleTasks
    pub fn get_idle_tasks(&self) -> Vec<Arc<dyn blaze_server::IdleTask>> {
        self.idle_tasks.lock().clone()
    }

    // Java: com/google/devtools/build/lib/runtime/CommandEnvironment.java:886 CommandEnvironment.getCommandStartTime
    // Java: com/google/devtools/build/lib/runtime/CommandEnvironment.java:544 CommandEnvironment.getInvocationPolicy
    pub fn get_invocation_policy(&self) -> &InvocationPolicy {
        &self.invocation_policy
    }

    // Java: com/google/devtools/build/lib/runtime/CommandEnvironment.java:548 CommandEnvironment.setInvocationPolicyFlags
    pub(crate) fn set_invocation_policy_flags(
        &mut self,
        invocation_policy_flags: Vec<OptionAndRawValue>,
    ) {
        self.invocation_policy_flags = invocation_policy_flags;
    }

    // Java: com/google/devtools/build/lib/runtime/CommandEnvironment.java:1097 CommandEnvironment.getResponseExtensions
    pub fn get_response_extensions(&self) -> Vec<blaze_protos::google::protobuf::Any> {
        self.response_extensions.lock().clone()
    }

    // Java: com/google/devtools/build/lib/runtime/CommandEnvironment.java:1101 CommandEnvironment.addResponseExtensions
    pub fn add_response_extensions(
        &self,
        extensions: impl IntoIterator<Item = blaze_protos::google::protobuf::Any>,
    ) {
        self.response_extensions.lock().extend(extensions);
    }

    /// The list [`CommandEnvironment::add_response_extensions`] adds to.
    pub(crate) fn response_extensions_sink(
        &self,
    ) -> Arc<parking_lot::Mutex<Vec<blaze_protos::google::protobuf::Any>>> {
        self.response_extensions.clone()
    }

    // Java: com/google/devtools/build/lib/runtime/CommandEnvironment.java:552 CommandEnvironment.getInvocationPolicyFlags
    pub fn get_invocation_policy_flags(&self) -> &[OptionAndRawValue] {
        &self.invocation_policy_flags
    }

    // Java: com/google/devtools/build/lib/runtime/CommandEnvironment.java:886 CommandEnvironment.getCommandStartTime
    pub fn get_command_start_time(&self) -> i64 {
        self.command_start_time
    }

    /// The time this command waited for the command lock plus `--command_wait_time`, in
    /// milliseconds.
    // Java: com/google/devtools/build/lib/runtime/CommandEnvironment.java:648 CommandEnvironment.getWaitTime
    pub fn get_wait_time(&self) -> i64 {
        self.wait_time_millis
    }

    /// Calls `SkyframeExecutor.decideKeepIncrementalState` with this command's options.
    ///
    /// Must be called prior to `BlazeModule.beforeCommand` so that modules can use the result of
    /// [`CommandEnvironment::tracks_state_for_incrementality`].
    ///
    /// PORT: Blaze's `SkyframeExecutor` is created by the command (one per command), so the
    /// decision and its warnings live here and the commands read the result from the environment.
    // Java: com/google/devtools/build/lib/runtime/CommandEnvironment.java:896 CommandEnvironment.decideKeepIncrementalState
    pub fn decide_keep_incremental_state(&mut self) {
        let common_options = self
            .options
            .get_options::<CommonCommandOptions>()
            .expect("CommonCommandOptions");
        let analysis_options =
            self.options
                .get_options::<blaze_optdefs::command::analysis_options::AnalysisOptions>();
        let batch = self
            .runtime
            .get_startup_options_provider()
            .get_options::<blaze_optdefs::command::blaze_server_startup_options::BlazeServerStartupOptions>()
            .is_some_and(|o| o.batch);
        self.track_incremental_state = decide_keep_incremental_state(
            batch,
            common_options.keep_state_after_build,
            common_options.track_incremental_state,
            common_options.heuristically_drop_nodes,
            analysis_options.is_some_and(|o| o.discard_analysis_cache),
            self.reporter.as_ref(),
        );
        // The server's state: a change of the decision, or no tracking, resets the kept graph,
        // also for a command that builds no Skyframe executor.
        self.workspace
            .get_incremental_state()
            .decide_keep_incremental_state_for_command(
                &self.command_id,
                self.track_incremental_state,
            );
    }

    /// `getSkyframeExecutor().tracksStateForIncrementality()`.
    // Java: com/google/devtools/build/lib/skyframe/SequencedSkyframeExecutor.java:459 SequencedSkyframeExecutor.tracksStateForIncrementality
    pub fn tracks_state_for_incrementality(&self) -> bool {
        self.track_incremental_state
    }

    // Java: com/google/devtools/build/lib/runtime/CommandEnvironment.java:427 CommandEnvironment.computeWorkingDirectory
    fn compute_working_directory(
        directories: &BlazeDirectories,
        command_options: &CommonCommandOptions,
    ) -> Result<Path, AbruptExitException> {
        let workspace = directories.get_working_directory();
        let working_directory = if directories.in_workspace() {
            let client_cwd: &PathFragment = &command_options.client_cwd.0;
            if client_cwd.contains_uplevel_references() {
                return Err(AbruptExitException::new(DetailedExitCode::of(FailureDetail {
                    message: format!(
                        "Client cwd '{}' contains uplevel references",
                        client_cwd.get_path_string()
                    ),
                    category: Some(
                        failure_details::failure_detail::Category::ClientEnvironment(
                            failure_details::ClientEnvironment {
                                code: failure_details::client_environment::Code::ClientCwdMalformed
                                    as i32,
                            },
                        ),
                    ),
                })));
            }
            let workspace = workspace.unwrap();
            if client_cwd.is_absolute() && !client_cwd.starts_with(workspace.as_fragment()) {
                return Err(AbruptExitException::new(DetailedExitCode::of(FailureDetail {
                    message: format!(
                        "Client cwd '{}' is not inside workspace '{}'",
                        client_cwd.get_path_string(),
                        workspace.get_path_string()
                    ),
                    category: Some(
                        failure_details::failure_detail::Category::ClientEnvironment(
                            failure_details::ClientEnvironment {
                                code: failure_details::client_environment::Code::ClientCwdMalformed
                                    as i32,
                            },
                        ),
                    ),
                })));
            }
            workspace.get_relative(client_cwd)
        } else {
            // FileSystemUtils.getWorkingDirectory: the process's working directory ('user.dir',
            // a platform string, reencoded to the internal representation).
            let cwd = std::env::current_dir().expect("getcwd");
            Path::create_from_str(
                &blaze_util::string_encoding::platform_to_internal(cwd.as_os_str()),
                directories.get_output_base().get_file_system(),
            )
        };
        Ok(working_directory)
    }

    // Java: com/google/devtools/build/lib/runtime/CommandEnvironment.java:607 CommandEnvironment.computeCommandId
    fn compute_command_id(
        command_options: &CommonCommandOptions,
        client_env: &IndexMap<String, String>,
        warnings: &mut Vec<String>,
        attempt_number: i32,
    ) -> String {
        let command_id = match &command_options.invocation_id {
            Some(id) => blaze_options::OptionScalar::java_to_string(id),
            None => {
                // Try to set the clientId from the client environment.
                let uuid_string = client_env
                    .get("BAZEL_INTERNAL_INVOCATION_ID")
                    .map(String::as_str)
                    .unwrap_or("");
                if uuid_string.is_empty() {
                    return uuid::Uuid::new_v4().to_string();
                }
                match uuid::Uuid::parse_str(uuid_string) {
                    Ok(id) => {
                        warnings.push(
                            "BAZEL_INTERNAL_INVOCATION_ID is set. This will soon be deprecated in \
                             favor of --invocation_id. Please switch to using the flag."
                                .to_string(),
                        );
                        id.to_string()
                    }
                    // String was malformed, so we will resort to generating a random UUID
                    Err(_) => return uuid::Uuid::new_v4().to_string(),
                }
            }
        };
        // When retrying a command, the retry has to use a different command ID. BES backends can
        // still link the invocations since their build ID will be the same and the attempt number
        // will be increased.
        if attempt_number > 1 {
            return uuid::Uuid::new_v4().to_string();
        }
        command_id
    }

    // Java: com/google/devtools/build/lib/runtime/CommandEnvironment.java:464 CommandEnvironment.getRuntime
    pub fn get_runtime(&self) -> &Arc<BlazeRuntime> {
        &self.runtime
    }

    // Java: com/google/devtools/build/lib/runtime/CommandEnvironment.java:488 CommandEnvironment.getBlazeWorkspace
    pub fn get_blaze_workspace(&self) -> &Arc<BlazeWorkspace> {
        &self.workspace
    }

    // Java: com/google/devtools/build/lib/runtime/CommandEnvironment.java:492 CommandEnvironment.getDirectories
    pub fn get_directories(&self) -> &BlazeDirectories {
        &self.directories
    }

    // Java: com/google/devtools/build/lib/runtime/CommandEnvironment.java:502 CommandEnvironment.getReporter
    pub fn get_reporter(&self) -> &Arc<Reporter> {
        &self.reporter
    }

    // Java: com/google/devtools/build/lib/runtime/CommandEnvironment.java:507 CommandEnvironment.getReporterOutErr
    pub fn get_reporter_out_err(&self) -> &OutErr {
        self.reporter.get_out_err()
    }

    // Java: com/google/devtools/build/lib/runtime/CommandEnvironment.java:523 CommandEnvironment.getClientEnv
    /// The client's command line (Java: `execExclusively`'s `args`, kept for the BEP's
    /// unstructured command line).
    pub fn get_client_args(&self) -> &[String] {
        &self.client_args
    }

    /// Java `CommandEnvironment.setCommandLine...`: the dispatcher records the client's command
    /// line and its rc-tagged startup options.
    pub fn set_client_command_line(
        &mut self,
        args: Vec<String>,
        startup_options_tagged_with_bazel_rc: Option<Vec<(String, String)>>,
    ) {
        self.client_args = args;
        self.startup_options_tagged_with_bazel_rc = startup_options_tagged_with_bazel_rc;
    }

    /// The client's startup options tagged with the rc file that set them.
    pub fn get_startup_options_tagged_with_bazel_rc(&self) -> Option<&[(String, String)]> {
        self.startup_options_tagged_with_bazel_rc.as_deref()
    }

    /// The command's event bus (Java `getEventBus()`): every `Reporter::post` reaches the
    /// subscribers registered on it.
    // Java: com/google/devtools/build/lib/runtime/CommandEnvironment.java CommandEnvironment.getEventBus
    pub fn get_event_bus(&self) -> &Arc<CommandEventBus> {
        &self.event_bus
    }

    // Java: com/google/devtools/build/lib/runtime/CommandEnvironment.java CommandEnvironment.getRepoEnv
    pub fn get_repo_env(&self) -> &IndexMap<String, String> {
        &self.repo_env
    }

    // Java: com/google/devtools/build/lib/runtime/CommandEnvironment.java CommandEnvironment.getNonstrictRepoEnv
    pub fn get_nonstrict_repo_env(&self) -> &IndexMap<String, String> {
        &self.nonstrict_repo_env
    }

    pub fn get_client_env(&self) -> &IndexMap<String, String> {
        &self.client_env
    }

    /// Return an ordered version of the client environment restricted to those variables
    /// allowlisted by the command-line options to be inheritable by actions.
    // Java: com/google/devtools/build/lib/runtime/CommandEnvironment.java:560 CommandEnvironment.getAllowlistedActionEnv
    pub fn get_allowlisted_action_env(&self) -> std::collections::BTreeMap<String, String> {
        self.filter_client_env(&self.visible_action_env)
    }

    /// Return an ordered version of the client environment restricted to those variables
    /// allowlisted by the command-line options to be inheritable by tests.
    // Java: com/google/devtools/build/lib/runtime/CommandEnvironment.java:568 CommandEnvironment.getAllowlistedTestEnv
    pub fn get_allowlisted_test_env(&self) -> std::collections::BTreeMap<String, String> {
        self.filter_client_env(&self.visible_test_env)
    }

    // Java: com/google/devtools/build/lib/runtime/CommandEnvironment.java:587 CommandEnvironment.filterClientEnv
    fn filter_client_env(
        &self,
        vars: &std::collections::BTreeSet<String>,
    ) -> std::collections::BTreeMap<String, String> {
        let mut result = std::collections::BTreeMap::new();
        for var in vars {
            if let Some(value) = self.client_env.get(var) {
                result.insert(var.clone(), value.clone());
            }
        }
        result
    }

    // Java: com/google/devtools/build/lib/runtime/CommandEnvironment.java:527 CommandEnvironment.getCommand
    pub fn get_command(&self) -> &'static CommandAnnotation {
        self.command
    }

    // Java: com/google/devtools/build/lib/runtime/CommandEnvironment.java:531 CommandEnvironment.getCommandName
    pub fn get_command_name(&self) -> &'static str {
        self.command.name()
    }

    // Java: com/google/devtools/build/lib/runtime/CommandEnvironment.java:535 CommandEnvironment.getOptions
    pub fn get_options(&self) -> &Arc<OptionsParsingResult> {
        &self.options
    }

    // Java: com/google/devtools/build/lib/runtime/CommandEnvironment.java:668 CommandEnvironment.getCommandId
    pub fn get_command_id(&self) -> &str {
        &self.command_id
    }

    // Java: com/google/devtools/build/lib/runtime/CommandEnvironment.java:1144 CommandEnvironment.getAttemptNumber
    pub fn get_attempt_number(&self) -> i32 {
        self.attempt_number
    }

    /// The working directory of the server (the workspace, or null outside one).
    // Java: com/google/devtools/build/lib/runtime/CommandEnvironment.java:706 CommandEnvironment.getWorkspace
    pub fn get_workspace(&self) -> Option<&Path> {
        self.directories.get_working_directory()
    }

    /// The name of the main repository's exec root directory.
    /// PORT: Java returns `runtime.getRuleClassProvider().getRunfilesPrefix()`, which Bazel sets
    /// to `LabelConstants.DEFAULT_REPOSITORY_DIRECTORY` ("_main") in
    /// `BazelRuleClassProvider.setup`. Read from `blaze_cmdline` once the rule class provider
    /// exists.
    // Java: com/google/devtools/build/lib/runtime/CommandEnvironment.java:710 CommandEnvironment.getWorkspaceName
    pub fn get_workspace_name(&self) -> &'static str {
        crate::runtime::RUNFILES_PREFIX
    }

    // Java: com/google/devtools/build/lib/runtime/CommandEnvironment.java:718 CommandEnvironment.getOutputBase
    pub fn get_output_base(&self) -> &Path {
        self.directories.get_output_base()
    }

    // Java: com/google/devtools/build/lib/runtime/CommandEnvironment.java:726 CommandEnvironment.getExecRoot
    pub fn get_exec_root(&self) -> Path {
        self.directories.get_exec_root(self.get_workspace_name())
    }

    // Java: com/google/devtools/build/lib/runtime/CommandEnvironment.java:745 CommandEnvironment.getWorkingDirectory
    pub fn get_working_directory(&self) -> &Path {
        &self.working_directory
    }

    // Java: com/google/devtools/build/lib/runtime/CommandEnvironment.java:540 CommandEnvironment.getConfigFlagDefinitions
    pub fn get_config_flag_definitions(
        &self,
    ) -> &Arc<blaze_sky_analysis::flag_set_function::ConfigFlagDefinitions> {
        &self.config_flag_definitions
    }

    /// Java: the `configFlagDefinitions` constructor argument (BlazeCommandDispatcher passes
    /// what `parseOptionsAndGetConfigDefinitions` returned).
    pub fn set_config_flag_definitions(
        &mut self,
        definitions: blaze_sky_analysis::flag_set_function::ConfigFlagDefinitions,
    ) {
        self.config_flag_definitions = Arc::new(definitions);
    }

    // Java: com/google/devtools/build/lib/runtime/CommandEnvironment.java:644 CommandEnvironment.getRelativeWorkingDirectory
    pub fn get_relative_working_directory(&self) -> &PathFragment {
        &self.relative_working_directory
    }

    // Java: com/google/devtools/build/lib/runtime/CommandEnvironment.java:878 CommandEnvironment.hasSyncedPackageLoading
    pub fn has_synced_package_loading(&self) -> bool {
        self.has_synced_package_loading
    }

    /// Initializes the package cache (the package path, default visibility, ...).
    // Java: com/google/devtools/build/lib/runtime/CommandEnvironment.java:851 CommandEnvironment.syncPackageLoading
    ///
    /// PORT: Blaze's commands create their `SkyframeExecutor` from the command's options when
    /// they need one (`commands::query::create_skyframe_executor`), so `getSkyframeExecutor()
    /// .sync(...)` reduces to the per-command resets of `preparePackageLoading` whose state is
    /// process-wide (`commands::query::prepare_package_loading`).
    pub fn sync_package_loading(
        &mut self,
        _options: &OptionsParsingResult,
    ) -> Result<(), AbruptExitException> {
        // We want to ensure that we're never calling #syncPackageLoading twice in the same build
        // because it does the very expensive work of diffing the cache between incremental
        // builds.
        if self.has_synced_package_loading {
            panic!(
                "IllegalStateException: We should never call this method more than once over the \
                 course of a single command"
            );
        }
        self.has_synced_package_loading = true;
        crate::commands::query::prepare_package_loading();
        Ok(())
    }

    /// `setUiEventHandler(uiEventHandler)`: also registers the handler's `@Subscribe` methods
    /// (`subscriber`) on the event bus.
    ///
    /// PORT: `reporter.addHandler(uiEventHandler)` is the caller's (the dispatcher registers the
    /// handler behind a filter, see `ReportedOnceFilter`).
    // Java: com/google/devtools/build/lib/runtime/CommandEnvironment.java:1192 CommandEnvironment.setUiEventHandler
    pub fn set_ui_event_handler(
        &mut self,
        handler: Arc<blaze_events::ui::UiEventHandler>,
        subscriber: Arc<blaze_buildtool::runtime::ui_event_subscriber::UiEventSubscriber>,
    ) {
        blaze_util::check_state!(
            self.ui_event_handler.is_none(),
            "UiEventHandler already set"
        );
        self.ui_event_handler = Some(handler);
        self.event_bus.register(subscriber.clone());
        self.ui_event_subscriber = Some(subscriber);
    }

    /// The handler's subscriber on the event bus (see [`CommandEnvironment::set_ui_event_handler`]).
    pub fn get_ui_event_subscriber(
        &self,
    ) -> Option<&Arc<blaze_buildtool::runtime::ui_event_subscriber::UiEventSubscriber>> {
        self.ui_event_subscriber.as_ref()
    }

    // Java: com/google/devtools/build/lib/runtime/CommandEnvironment.java:1199 CommandEnvironment.getUiEventHandler
    pub fn get_ui_event_handler(&self) -> Option<&Arc<blaze_events::ui::UiEventHandler>> {
        self.ui_event_handler.as_ref()
    }

    /// The command's Build Event Protocol support, if the BES module set one up.
    pub(crate) fn set_bep(&mut self, bep: Option<Arc<crate::commands::bep::BepCommandSupport>>) {
        self.bep = bep;
    }

    /// The command's Build Event Protocol support (`None` when no build event file or service
    /// was requested, or the command does not stream build events).
    pub(crate) fn get_bep(&self) -> Option<&Arc<crate::commands::bep::BepCommandSupport>> {
        self.bep.as_ref()
    }

    /// Returns the module environment, which modules use to request an abrupt exit of this
    /// command from any thread.
    // Java: com/google/devtools/build/lib/runtime/CommandEnvironment.java:556 CommandEnvironment.getBlazeModuleEnvironment
    pub fn get_blaze_module_environment(&self) -> &BlazeModuleEnvironment {
        &self.blaze_module_environment
    }

    /// `getBlazeModuleEnvironment().exit(exception)`.
    // Java: com/google/devtools/build/lib/runtime/CommandEnvironment.java:207 BlazeModuleEnvironment.exit
    pub fn exit(&self, exception: AbruptExitException) {
        self.blaze_module_environment.exit(exception);
    }

    /// Returns additional listeners to the console output stream: those the modules return
    /// from `getOutputListener`, in module order.
    // Java: com/google/devtools/build/lib/runtime/CommandEnvironment.java:652 CommandEnvironment.getOutputListeners
    pub(crate) fn get_output_listeners(&self) -> Vec<OutErr> {
        let mut result = Vec::new();
        for module in self.runtime.get_blaze_modules() {
            if let Some(listener) = module.get_output_listener(self) {
                result.push(listener);
            }
        }
        result
    }

    /// Prevents any further interruption of this command by modules, and returns the final
    /// exit code from modules, or `None` if no modules requested an abrupt exit.
    ///
    /// Always returns the same value on subsequent calls.
    // Java: com/google/devtools/build/lib/runtime/CommandEnvironment.java:781 CommandEnvironment.finalizeDetailedExitCode
    fn finalize_detailed_exit_code(&self) -> Option<DetailedExitCode> {
        {
            // Set the pending exception so that further calls to exit(AbruptExitException) don't
            // lead to unwanted thread interrupts.
            let mut pending = self.blaze_module_environment.pending_exception.lock();
            if pending.is_none() {
                *pending = Some(None);
                return None;
            }
        }
        if blaze_shell::thread_interrupt::ThreadInterrupt::current()
            .same_thread(&self.blaze_module_environment.command_thread)
        {
            // We may have interrupted the thread in the process, so clear the interrupted bit.
            // Whether the command was interrupted or not, it's about to be over, so don't
            // interrupt later things happening on this thread.
            blaze_shell::thread_interrupt::interrupted();
        }
        // Extract the exit code (it can be null if someone has already called
        // finalizeExitCode()).
        self.get_pending_detailed_exit_code()
    }

    /// Hook method called by the BlazeCommandDispatcher right before the dispatch of each
    /// command ends (while its outcome can still be modified).
    ///
    // PORT: skipped, a direct call to its one subscriber (commands/bep.rs): CommandPrecompleteEvent.getExitCode
    /// PORT: the deprecated `CommandPrecompleteEvent(ExitCode)` posted here has no subscriber
    /// in Blaze; the metrics collector's `CommandPrecompleteEvent` is posted by `afterCommand`
    /// (see `BlazeCommandDispatcher::after_command`).
    // Java: com/google/devtools/build/lib/runtime/CommandEnvironment.java:802 CommandEnvironment.precompleteCommand
    pub(crate) fn precomplete_command(
        &self,
        _original_exit: &DetailedExitCode,
    ) -> Option<DetailedExitCode> {
        self.finalize_detailed_exit_code()
    }

    /// Returns the current exit code requested by modules, or `None` if no exit has been
    /// requested.
    // Java: com/google/devtools/build/lib/runtime/CommandEnvironment.java:810 CommandEnvironment.getPendingDetailedExitCode
    fn get_pending_detailed_exit_code(&self) -> Option<DetailedExitCode> {
        self.get_pending_exception()
            .map(|e| e.get_detailed_exit_code().clone())
    }

    /// Retrieves the exception currently queued by a Blaze module.
    // Java: com/google/devtools/build/lib/runtime/CommandEnvironment.java:823 CommandEnvironment.getPendingException
    pub fn get_pending_exception(&self) -> Option<AbruptExitException> {
        self.blaze_module_environment
            .pending_exception
            .lock()
            .clone()
            .flatten()
    }

    /// Throws the exception currently queued by a Blaze module.
    ///
    /// This should be called as often as is practical so that errors are reported as soon as
    /// possible. Ideally, we'd not need this, but the event bus swallows exceptions so we raise
    /// the exception this way.
    // Java: com/google/devtools/build/lib/runtime/CommandEnvironment.java:835 CommandEnvironment.throwPendingException
    pub fn throw_pending_exception(&self) -> Result<(), AbruptExitException> {
        if let Some(exception) = self.get_pending_exception() {
            if blaze_shell::thread_interrupt::ThreadInterrupt::current()
                .same_thread(&self.blaze_module_environment.command_thread)
            {
                // Throwing this exception counts as the requested interruption. Clear the
                // interrupted bit.
                blaze_shell::thread_interrupt::interrupted();
            }
            return Err(exception);
        }
        Ok(())
    }

    /// `shutdownReasonConsumer.accept(reason)`.
    pub fn set_shutdown_reason(&mut self, reason: String) {
        self.shutdown_reason = Some(reason);
    }

    pub(crate) fn take_shutdown_reason(&mut self) -> Option<String> {
        self.shutdown_reason.take()
    }
}

/// Decides if graph edges should be stored during this evaluation and returns
/// `trackIncrementalState`.
///
/// Necessary conditions to not store graph edges are either
///
/// 1. batch (since incremental builds are not possible) and discard_analysis_cache (since
///    otherwise user isn't concerned about saving memory this way).
/// 2. track_incremental_state set to false.
///
/// PORT: skipped, incremental: the rest of the method (`heuristicallyDropNodes`,
/// `evaluatorNeedsReset`, `keepStateAfterBuild` bookkeeping) configures the evaluator that
/// outlives the command; Blaze's evaluator lives for one command.
// Java: com/google/devtools/build/lib/skyframe/SequencedSkyframeExecutor.java:399 SequencedSkyframeExecutor.decideKeepIncrementalState
pub fn decide_keep_incremental_state(
    batch: bool,
    keep_state_after_build: bool,
    should_track_incremental_state: bool,
    heuristically_drop_nodes: bool,
    discard_analysis_cache: bool,
    event_handler: &dyn EventHandler,
) -> bool {
    // First check if the incrementality state should be kept around during the build.
    let explicitly_requested_no_incremental_data = !should_track_incremental_state;
    let implicitly_requested_no_incremental_data = batch && discard_analysis_cache;
    let track_incremental_state =
        !explicitly_requested_no_incremental_data && !implicitly_requested_no_incremental_data;
    if explicitly_requested_no_incremental_data != implicitly_requested_no_incremental_data {
        if !explicitly_requested_no_incremental_data {
            event_handler.handle(&blaze_events::Event::warn(
                "--batch and --discard_analysis_cache specified, but --notrack_incremental_state \
                 not specified: incrementality data is implicitly discarded, but you may need \
                 to specify --notrack_incremental_state in the future if you want to \
                 maximize memory savings.",
            ));
        }
        if !batch && keep_state_after_build {
            event_handler.handle(&blaze_events::Event::warn(
                "--notrack_incremental_state was specified, but without \
                 --nokeep_state_after_build. Inmemory state from this build will not be \
                 reusable, but it will not get fully wiped until the beginning of the next \
                 build. Use --nokeep_state_after_build to clean up eagerly.",
            ));
        }
    }

    if track_incremental_state && heuristically_drop_nodes {
        event_handler.handle(&blaze_events::Event::warn(
            "--heuristically_drop_nodes was specified with track incremental state also being \
             true. The flag is ignored and no node is heuristically dropped in the track \
             incremental mode.",
        ));
    }
    track_incremental_state
}

/// Converts a list of `(name, value)` entries to a map sorted by name, later entries winning
/// (Java collects into a TreeMap).
// Java: com/google/devtools/build/lib/runtime/CommandEnvironment.java:598 CommandEnvironment.makeMapFromMapEntries
fn make_map_from_map_entries(entries: &[(String, String)]) -> IndexMap<String, String> {
    let mut result = std::collections::BTreeMap::new();
    for (k, v) in entries {
        result.insert(k.clone(), v.clone());
    }
    result.into_iter().collect()
}

/// The command's event bus (Java: Guava's `EventBus` of the command). Subscribers receive every
/// posted event in registration order; each ignores the events it does not subscribe to.
#[derive(Default)]
pub struct CommandEventBus {
    subscribers: parking_lot::RwLock<Vec<Arc<dyn blaze_events::EventBus>>>,
}

impl CommandEventBus {
    /// `eventBus.register(subscriber)`.
    pub fn register(&self, subscriber: Arc<dyn blaze_events::EventBus>) {
        self.subscribers.write().push(subscriber);
    }

    /// Drops every subscriber: the end of the command's event bus in `BlazeRuntime.afterCommand`
    /// (`BlazeWorkspace.clearEventBus`, "this is how we release memory associated with
    /// handlers", and `Reporter.clearEventBus`). Events posted later reach no subscriber.
    ///
    /// PORT: Java leaves the bus to the collector once the workspace and the reporter no longer
    /// reference it. A Blaze subscriber may hold the bus (`ExecutionProgressReceiverSetup`, the
    /// test result aggregator), and the bus holds its subscribers, so neither would be freed.
    // Java: com/google/devtools/build/lib/runtime/BlazeWorkspace.java:357 BlazeWorkspace.clearEventBus
    pub fn clear_subscribers(&self) {
        let subscribers = std::mem::take(&mut *self.subscribers.write());
        // Dropped outside the lock: a subscriber's drop may post or register.
        drop(subscribers);
    }
}

impl blaze_events::EventBus for CommandEventBus {
    fn post(&self, event: Arc<dyn blaze_events::Postable>) {
        let subscribers = self.subscribers.read().clone();
        for subscriber in subscribers {
            subscriber.post(event.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use blaze_events::{EventKind, StoredEventHandler};

    use super::*;

    fn decide(
        batch: bool,
        keep_state_after_build: bool,
        should_track_incremental_state: bool,
        heuristically_drop_nodes: bool,
        discard_analysis_cache: bool,
    ) -> (bool, Vec<String>) {
        let handler = StoredEventHandler::new();
        let track = decide_keep_incremental_state(
            batch,
            keep_state_after_build,
            should_track_incremental_state,
            heuristically_drop_nodes,
            discard_analysis_cache,
            &handler,
        );
        let warnings = handler
            .get_events()
            .iter()
            .map(|e| {
                assert_eq!(e.get_kind(), EventKind::Warning);
                e.get_message().into_owned()
            })
            .collect();
        (track, warnings)
    }

    // The defaults of every command: incremental state is tracked, nothing is reported.
    #[test]
    fn default_options_track_incremental_state() {
        assert_eq!(decide(false, true, true, false, false), (true, vec![]));
    }

    // Oracle (scripts/w4/scenarios/query-errors step 23, ui-build-results step 45):
    // `--notrack_incremental_state` without `--nokeep_state_after_build`.
    #[test]
    fn notrack_incremental_state_warns_about_keep_state_after_build() {
        assert_eq!(
            decide(false, true, false, false, false),
            (
                false,
                vec![
                    "--notrack_incremental_state was specified, but without \
                     --nokeep_state_after_build. Inmemory state from this build will not be \
                     reusable, but it will not get fully wiped until the beginning of the next \
                     build. Use --nokeep_state_after_build to clean up eagerly."
                        .to_owned()
                ]
            )
        );
    }

    // Oracle (ui-build-results step 46): with `--nokeep_state_after_build` there is no warning.
    #[test]
    fn notrack_incremental_state_with_nokeep_state_after_build_is_silent() {
        assert_eq!(decide(false, false, false, false, false), (false, vec![]));
    }

    // SequencedSkyframeExecutor.decideKeepIncrementalState: --batch with
    // --discard_analysis_cache discards the state implicitly (message from the Java source).
    #[test]
    fn batch_with_discard_analysis_cache_discards_implicitly() {
        assert_eq!(
            decide(true, true, true, false, true),
            (
                false,
                vec![
                    "--batch and --discard_analysis_cache specified, but \
                     --notrack_incremental_state not specified: incrementality data is \
                     implicitly discarded, but you may need to specify \
                     --notrack_incremental_state in the future if you want to maximize memory \
                     savings."
                        .to_owned()
                ]
            )
        );
        // Both the explicit and the implicit request: nothing to warn about.
        assert_eq!(decide(true, true, false, false, true), (false, vec![]));
    }

    // Oracle (ui-build-results step 47): `--heuristically_drop_nodes` while tracking state.
    #[test]
    fn heuristically_drop_nodes_is_ignored_when_tracking() {
        assert_eq!(
            decide(false, true, true, true, false),
            (
                true,
                vec![
                    "--heuristically_drop_nodes was specified with track incremental state also \
                     being true. The flag is ignored and no node is heuristically dropped in the \
                     track incremental mode."
                        .to_owned()
                ]
            )
        );
        assert_eq!(decide(false, false, false, true, false), (false, vec![]));
    }
}
