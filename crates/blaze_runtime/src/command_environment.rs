//! `CommandEnvironment`: the state of one command invocation.
//!
//! PORT: W1 subset. Skyframe access, the event bus, downloader, repo env, resource manager,
//! output service and timestamps are added with the waves that need them.

use std::sync::Arc;

use blaze_events::{EventHandler, OutErr, Reporter};
use blaze_optdefs::command::common_command_options::CommonCommandOptions;
use blaze_options::rc::client_options::ClientOptions;
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
pub struct CommandEnvironment {
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
    visible_action_env: std::collections::BTreeSet<String>,
    visible_test_env: std::collections::BTreeSet<String>,
    /// `blazeModuleEnvironment.exit(e)`: the first pending abrupt exit, checked in
    /// `beforeCommand` (Java: `BlazeModuleEnvironment.pendingException`).
    pending_exception: Option<AbruptExitException>,
    has_synced_package_loading: bool,
    ui_event_handler: Option<Arc<blaze_events::ui::UiEventHandler>>,
    shutdown_reason: Option<String>,
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
        warnings: &mut Vec<String>,
    ) -> CommandEnvironment {
        let directories = workspace.get_directories().clone();
        // PORT: Java's Reporter is constructed with the command's EventBus; Blaze has no event
        // bus yet.
        let reporter = Reporter::new(None);

        let command_options = options.get_options::<CommonCommandOptions>().expect(
            "CommandEnvironment needs its options provider to have CommonCommandOptions loaded.",
        );
        let mut pending_exception = None;
        let working_directory =
            match Self::compute_working_directory(&directories, &command_options) {
                Ok(wd) => wd,
                Err(e) => {
                    // We'll exit very soon, but set the working directory to something reasonable so
                    // remainder of setup can finish.
                    if pending_exception.is_none() {
                        pending_exception = Some(e);
                    }
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
        let command_id = Self::compute_command_id(&command_options, &client_env, warnings);
        // PORT: buildRequestId, repoEnv and nonstrictRepoEnv are not ported (no repository
        // fetching yet); only the allowlisted action/test env computation is.
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
                    blaze_options::EnvVar::Set { name, .. } => {
                        visible_action_env.remove(name);
                    }
                    blaze_options::EnvVar::Inherit { name } => {
                        visible_action_env.insert(name.clone());
                    }
                    blaze_options::EnvVar::Unset { name } => {
                        visible_action_env.remove(name);
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

        CommandEnvironment {
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
            visible_action_env,
            visible_test_env,
            pending_exception,
            has_synced_package_loading: false,
            ui_event_handler: None,
            shutdown_reason: None,
        }
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
            // FileSystemUtils.getWorkingDirectory: the process's working directory.
            let cwd = std::env::current_dir().expect("getcwd");
            Path::create_from_str(
                cwd.to_str().expect("non-UTF-8 working directory"),
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
    ) -> String {
        // PORT: attemptNumber is always 1 (no remote-cache retries yet), so the retry id
        // derivation is not ported.
        if let Some(id) = &command_options.invocation_id {
            return blaze_options::OptionScalar::java_to_string(id);
        }
        // Try to set the clientId from the client environment.
        let uuid_string = client_env
            .get("BAZEL_INTERNAL_INVOCATION_ID")
            .map(String::as_str)
            .unwrap_or("");
        if !uuid_string.is_empty() {
            match uuid::Uuid::parse_str(uuid_string) {
                Ok(id) => {
                    warnings.push(
                        "BAZEL_INTERNAL_INVOCATION_ID is set. This will soon be deprecated in favor of \
                         --invocation_id. Please switch to using the flag."
                            .to_string(),
                    );
                    id.to_string()
                }
                // String was malformed, so we will resort to generating a random UUID
                Err(_) => uuid::Uuid::new_v4().to_string(),
            }
        } else {
            uuid::Uuid::new_v4().to_string()
        }
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
    pub fn sync_package_loading(
        &mut self,
        _options: &OptionsParsingResult,
    ) -> Result<(), AbruptExitException> {
        blaze_util::unported!("CommandEnvironment.syncPackageLoading")
    }

    // Java: com/google/devtools/build/lib/runtime/CommandEnvironment.java:1192 CommandEnvironment.setUiEventHandler
    pub fn set_ui_event_handler(&mut self, handler: Arc<blaze_events::ui::UiEventHandler>) {
        self.ui_event_handler = Some(handler);
    }

    pub fn get_ui_event_handler(&self) -> Option<&Arc<blaze_events::ui::UiEventHandler>> {
        self.ui_event_handler.as_ref()
    }

    /// The pending abrupt exit recorded by `BlazeModuleEnvironment.exit`.
    pub(crate) fn take_pending_exception(&mut self) -> Option<AbruptExitException> {
        self.pending_exception.take()
    }

    /// `BlazeModuleEnvironment.exit`: the first exception wins.
    // Java: com/google/devtools/build/lib/runtime/CommandEnvironment.java:207 BlazeModuleEnvironment.exit
    pub fn exit(&mut self, exception: AbruptExitException) {
        if self.pending_exception.is_none() {
            self.pending_exception = Some(exception);
        }
    }

    /// `shutdownReasonConsumer.accept(reason)`.
    pub fn set_shutdown_reason(&mut self, reason: String) {
        self.shutdown_reason = Some(reason);
    }

    pub(crate) fn take_shutdown_reason(&mut self) -> Option<String> {
        self.shutdown_reason.take()
    }
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
