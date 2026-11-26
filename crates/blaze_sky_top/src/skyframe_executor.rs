//! The `SkyframeExecutor` of one command: a single evaluator with the loading, bzlmod,
//! configuration, toolchain and configured-target functions, and the methods of Java's
//! `SkyframeExecutor` the query/cquery/aquery commands and the analysis phase use.
//!
//! PORT: Java's `SkyframeExecutor` lives for the server's lifetime and is re-synced per command
//! (`syncPackageLoading`, `setBaselineConfiguration`, precomputed injection). Blaze runs one
//! command per evaluator: everything Java injects as precomputed values is known when the command
//! starts, so it is set while the evaluator is built. In particular the evaluator's single client
//! is the configuration functions' `ConfigurationEnvironment`, which holds the baseline
//! configuration: the baseline (`setBaselineConfiguration`) is therefore set at construction.

use std::path::PathBuf;
use std::sync::Arc;

use blaze_actions::ActionKeyContext;
use blaze_actions::actions::artifact_factory::ArtifactFactory;
use blaze_analysis::analysis_services::{AnalysisDirectories, AnalysisServices};
use blaze_analysis::aspect_key_creator::{AspectKey, TopLevelAspectsKey};
use blaze_analysis::aspect_value::AspectValue;
use blaze_analysis::configured_target::ConfiguredTarget;
use blaze_analysis::configured_target_key::ConfiguredTargetKey;
use blaze_analysis::configured_target_value::ConfiguredTargetValue;
use blaze_build_config::bazel_rule_class_provider::BazelRuleClassProvider;
use blaze_build_config::build_configuration_key::{
    BuildConfigurationKey, BuildConfigurationKeyValueKey,
};
use blaze_build_config::build_configuration_value::BuildConfigurationValue;
use blaze_build_config::build_options::BuildOptions;
use blaze_build_config::fragment::InvalidConfigurationException;
use blaze_build_config::output_directories::ConfigDirectories;
use blaze_build_config::platform::SharedPlatformValueProvider;
use blaze_build_config::skyframe::{
    ConfigurationEnvironment, ConfigurationError, register_configuration_functions,
};
use blaze_bzlmod::skyframe::register_bzlmod_functions;
use blaze_cmdline::{
    Label, PackageIdentifier, RepositoryMapping, TargetParsingException, TargetPatternsCode,
};
use blaze_events::{Event, EventHandler, ExtendedEventHandler};
use blaze_optdefs::command::common_command_options::CommonCommandOptions;
use blaze_optdefs::command::loading_options::LoadingOptions;
use blaze_optdefs::command::package_options::PackageOptions;
use blaze_optdefs::misc::auth_and_tls_options::AuthAndTLSOptions;
use blaze_optdefs::misc::local_execution_options::LocalExecutionOptions;
use blaze_optdefs::misc::repository_options::RepositoryOptions;
use blaze_options::{Options, OptionsParser};
use blaze_packages::package::Package;
use blaze_packages::target::Target;
use blaze_sky_analysis::producers::errors::ConfiguredTargetFunctionException;
use blaze_sky_loading::directories::LoadingDirectories;
use blaze_sky_loading::package_function::{PackageKey, PackageValue, SkyframePackageLocator};
use blaze_sky_loading::package_manager::{
    RepositoryMappingResolutionOrInterrupted, SkyframePackageManager,
};
use blaze_sky_loading::pkgcache::PathPackageLocator;
use blaze_sky_loading::skyframe_executor::{
    LoadingFunctionsConfig, inject_loading_precomputed_with_package_options,
    register_loading_functions,
};
use blaze_skyframe::{
    EvaluationContext, EvaluationResult, Evaluator, EvaluatorBuilder, Interrupted, Key, SkyKey,
    WalkableGraph,
};
use blaze_starlark::StarlarkSemantics;
use blaze_vfs::{DigestHashFunction, Path, PathFragment};
use indexmap::IndexMap;

use crate::action_conflict_exception::{ActionConflictException, ActionConflicts};
use crate::action_lookup_conflict_finding_function::{
    self, ActionConflictsCell, TopLevelActionLookupConflictFindingKey,
};
use crate::cycles_reporter::{CyclesReporter, SingleCycleReporter, TargetCycleReporter};
use crate::pattern_expanding_error::PatternExpandingError;
use crate::target_pattern_phase::{
    TargetPatternPhaseKey, TargetPatternPhaseValue, TestFilter,
    register_target_pattern_phase_functions,
};
use blaze_repo_rules::external_repos::BzlmodExternalRepos;
use blaze_repo_rules::setup::{CommandInputs, FetchSetup};

/// `SkyframeExecutor.DEFAULT_THREAD_COUNT`.
pub const DEFAULT_THREAD_COUNT: usize = 200;

/// What a command hands the executor (Java: `BlazeDirectories`, the command's options and
/// environment).
pub struct SkyframeExecutorInputs {
    pub workspace: Path,
    pub working_directory: Path,
    pub output_base: Path,
    pub install_base: Path,
    /// `BlazeDirectories.getExecRootBase()` (`<output_base>/execroot`).
    pub exec_root_base: Path,
    /// `bazel`.
    pub product_name: String,
    /// `_main`.
    pub workspace_name: String,
    /// The command's parsed options.
    pub options: Arc<OptionsParser>,
    /// `env.getReporter()`.
    pub reporter: Arc<dyn ExtendedEventHandler>,
    /// `env.getClientEnv()`.
    pub client_env: IndexMap<String, String>,
    /// `env.getRepoEnv()`.
    pub repo_env: IndexMap<String, String>,
    /// `env.getNonstrictRepoEnv()`.
    pub nonstrict_repo_env: IndexMap<String, String>,
    /// The repository precomputed values (`BazelRepositoryModule.getPrecomputedValues`; `fetch`
    /// sets the force-fetch ones).
    pub repo_precomputed: blaze_repo_rules::registration::RepoPrecomputedValues,
    /// `--keep_going`.
    pub keep_going: bool,
    /// The top-level build options of a command that analyzes (Java
    /// `runtime.createBuildOptions(request)`, set as baseline by `BuildView.update`); `None` for
    /// commands that only load (`query`).
    pub top_level_build_options: Option<BuildOptions>,
    /// The execution setup of a command that executes actions (`build`, `test`).
    pub execution: Option<crate::execution_phase::ExecutionSetup>,
    /// The server's kept graph and this command's part in it (`None`: a new graph that is
    /// dropped with the executor, as before; see [`crate::incremental`]).
    pub incremental: Option<crate::incremental::IncrementalCommand>,
}

/// Why the executor could not be created.
#[derive(Debug, Clone, thiserror::Error)]
pub enum SkyframeExecutorCreationError {
    /// The repository setup (`BazelRepositoryModule.beforeCommand`) failed.
    #[error("{0}")]
    ExternalRepos(String),
    /// `BzlmodConfigError` or an `AbruptExitException` of `BazelRepositoryModule.beforeCommand`
    /// (message and `ExternalRepository` code name).
    #[error("{message}")]
    Bzlmod { message: String, code: &'static str },
    /// `setBaselineConfiguration` failed (the exec transition could not be loaded or applied).
    #[error("{}", .0.get_message())]
    InvalidConfiguration(InvalidConfigurationException),
    /// `ExecutionTool.<init>` / `prepareForExecution` failed (an `AbruptExitException`).
    #[error("{0}")]
    Execution(String),
}

/// The configuration-related state (Java: the precomputed baseline values).
struct ConfigurationState {
    client: Arc<ConfigurationEnvironment>,
}

/// The `SkyframeExecutor` of one command.
// Java: com/google/devtools/build/lib/skyframe/SkyframeExecutor.java:369 SkyframeExecutor
pub struct SkyframeExecutor {
    /// `progressReceiver`.
    progress_receiver: Arc<crate::skyframe_progress_receiver::SkyframeProgressReceiver>,
    evaluator: Arc<Evaluator>,
    package_manager: Arc<SkyframePackageManager>,
    pkg_locator: PathPackageLocator,
    starlark_semantics: StarlarkSemantics,
    digest_function: DigestHashFunction,
    reporter: Arc<dyn ExtendedEventHandler>,
    analysis_services: Option<Arc<AnalysisServices>>,
    configuration: Option<ConfigurationState>,
    rule_class_provider: Arc<blaze_packages::native_rules::RuleClassProvider>,
    /// For `BazelLockFileModule.afterCommand`.
    workspace: PathBuf,
    output_base: PathBuf,
    lockfile_mode: blaze_bzlmod_model::LockfileMode,
    execution: Option<crate::execution_phase::ExecutionState>,
    /// `cyclesReporter` (`createCyclesReporter()`).
    cycles_reporter: CyclesReporter,
    /// `ArtifactConflictFinder.ACTION_CONFLICTS`: the conflicts the conflict-finding functions
    /// read (see `ActionConflictsCell`).
    action_conflicts: ActionConflictsCell,
    /// `topLevelArtifactContextForConflictPruning` of the request being analyzed: the requested
    /// output groups (see `skyframe_build_view::configure_targets`).
    top_level_artifact_context_for_conflict_pruning: parking_lot::Mutex<Option<Vec<String>>>,
    /// The repository module's download manager as the registries use it (Java: the
    /// `DownloadManager` `BazelRepositoryModule` hands `VendorCommand.setDownloadManager`).
    registry_fetcher: Arc<dyn blaze_bzlmod_model::registry::RegistryFetcher>,
    /// PORT: Blaze-only (decision D7). The evaluator's values and the services above hold frozen
    /// Starlark values whose heaps the build arena (`blaze_util::build_arena`) keeps: the arena
    /// is not released while this executor exists. Declared last, so it is dropped after the
    /// evaluator and everything else of the executor.
    _arena_lease: blaze_util::build_arena::Lease,
    /// The hold on the server's kept graph: the evaluator goes back when the executor is dropped.
    incremental: Option<crate::incremental::ExecutorHandle>,
    /// The command's options (`AnalysisOptions` for the analysis-cache decision).
    options: Arc<OptionsParser>,
}

impl Drop for SkyframeExecutor {
    fn drop(&mut self) {
        // An executor dropped while unwinding from a panic gives nothing back: its graph may be
        // half updated (Java's server crashes on an uncaught exception).
        if let Some(handle) = self.incremental.take()
            && !std::thread::panicking()
        {
            handle
                .state()
                .clone()
                .put_back(handle, self.evaluator.clone());
        }
    }
}

/// A `Path` as the platform path an API that takes `std::path` wants: the bytes of the internal
/// string (`StringEncoding.internalToPlatform`).
fn to_std_path(p: &Path) -> PathBuf {
    PathBuf::from(
        blaze_util::string_encoding::internal_to_platform(p.get_path_string()).into_owned(),
    )
}

impl SkyframeExecutor {
    /// The download manager of the repository module, as the registries use it (`vendor`
    /// downloads the registry files it vendors through it).
    // Java: com/google/devtools/build/lib/bazel/BazelRepositoryModule.java:287 BazelRepositoryModule.workspaceInit
    pub fn get_registry_fetcher(&self) -> &Arc<dyn blaze_bzlmod_model::registry::RegistryFetcher> {
        &self.registry_fetcher
    }

    /// The command's event handler (Java `SkyframeExecutor.reporter`).
    pub fn get_reporter(&self) -> &Arc<dyn ExtendedEventHandler> {
        &self.reporter
    }

    /// Builds the evaluator of the command (Java: `SkyframeExecutor` construction,
    /// `skyFunctions()`, `syncPackageLoading` / `preparePackageLoading` and, for commands that
    /// analyze, `setBaselineConfiguration`).
    // Java: com/google/devtools/build/lib/skyframe/SkyframeExecutor.java:760 SkyframeExecutor.skyFunctions
    pub fn create(
        inputs: SkyframeExecutorInputs,
    ) -> Result<SkyframeExecutor, SkyframeExecutorCreationError> {
        Self::create_with_output_user_root(inputs, None)
    }

    /// [`SkyframeExecutor::create`] with `ServerDirectories.getOutputUserRoot()`
    /// (`--output_user_root`), under which the default repository cache lives.
    ///
    /// PORT: a separate entry point because `SkyframeExecutorInputs` is built field by field by
    /// its callers. `None` is `CommandInputs::output_user_root == None` of
    /// `blaze_repo_rules::setup`: the default cache location is derived only for the client's
    /// default output base, and the setup fails for any other output base without
    /// `--repository_cache`.
    // Java: com/google/devtools/build/lib/skyframe/SkyframeExecutor.java:760 SkyframeExecutor.skyFunctions
    pub fn create_with_output_user_root(
        inputs: SkyframeExecutorInputs,
        output_user_root: Option<&Path>,
    ) -> Result<SkyframeExecutor, SkyframeExecutorCreationError> {
        Self::create_with_builtin_modules(inputs, output_user_root, None)
    }

    /// [`SkyframeExecutor::create_with_output_user_root`] with `ModuleFileFunction`'s built-in
    /// modules replaced (Java tests: `AnalysisMock.getBuiltinModules`, handed to the
    /// `ModuleFileFunction` of the test's executor). `None` keeps Bazel's (`bazel_tools` only).
    ///
    /// PORT: a test seam; Java constructs the test executor with the mock's SkyFunctions
    /// (`AnalysisMock.getSkyFunctions`) instead.
    // Java: com/google/devtools/build/lib/skyframe/SkyframeExecutor.java:760 SkyframeExecutor.skyFunctions
    pub fn create_with_builtin_modules(
        inputs: SkyframeExecutorInputs,
        output_user_root: Option<&Path>,
        builtin_modules: Option<IndexMap<String, blaze_bzlmod_model::module::NonRegistryOverride>>,
    ) -> Result<SkyframeExecutor, SkyframeExecutorCreationError> {
        Self::create_with_extra_sky_functions(inputs, output_user_root, builtin_modules, None)
    }

    /// [`SkyframeExecutor::create_with_builtin_modules`] with test SkyFunctions registered after
    /// the executor's own (Java tests: `AnalysisMock.getSkyFunctions`, handed to the test
    /// executor's `setExtraSkyFunctions`).
    // Java: com/google/devtools/build/lib/skyframe/SkyframeExecutor.java:760 SkyframeExecutor.skyFunctions
    pub fn create_with_extra_sky_functions(
        inputs: SkyframeExecutorInputs,
        output_user_root: Option<&Path>,
        builtin_modules: Option<IndexMap<String, blaze_bzlmod_model::module::NonRegistryOverride>>,
        extra_sky_functions: Option<&dyn Fn(EvaluatorBuilder) -> EvaluatorBuilder>,
    ) -> Result<SkyframeExecutor, SkyframeExecutorCreationError> {
        // Taken before anything that can load Starlark (see the field).
        let arena_lease = blaze_util::build_arena::lease();
        let options = &inputs.options;
        let incremental = inputs.incremental.clone();
        if let Some(incremental) = &incremental {
            incremental.state.strip_kept_graph();
        }
        let top_level_build_options_for_sync = inputs.top_level_build_options.clone();
        let executes = inputs.execution.is_some();
        // Java: com/google/devtools/build/lib/skyframe/SkyframeExecutor.java:1722 SkyframeExecutor.sync
        // getEffectiveStarlarkSemantics(buildLanguageOptions): the full
        // BuildLanguageOptions.toStarlarkSemantics.
        let starlark_semantics = blaze_build_config::top_level::starlark_semantics_from(options);
        let digest_function = inputs.output_base.get_file_system().get_digest_function();

        let directories = LoadingDirectories {
            workspace: inputs.workspace.clone(),
            output_base: inputs.output_base.clone(),
            install_base: inputs.install_base.clone(),
        };
        // Bzlmod and repository fetching (Java: BazelRepositoryModule.beforeCommand and the
        // bzlmod / repository SkyFunctions).
        // Java: com/google/devtools/build/lib/bazel/BazelRepositoryModule.java:292 BazelRepositoryModule.beforeCommand
        // (`fetchDisabled = pkgOptions != null && !pkgOptions.fetch`, for every command).
        let mut repo_precomputed = inputs.repo_precomputed.clone();
        repo_precomputed.fetch_disabled = options
            .get_options::<PackageOptions>()
            .is_some_and(|pkg_options| !pkg_options.fetch);
        let repo_options: RepositoryOptions = options
            .get_options::<RepositoryOptions>()
            .unwrap_or_else(Options::<RepositoryOptions>::get_defaults);
        let workspace_std = to_std_path(&inputs.workspace);
        let working_directory_std = to_std_path(&inputs.working_directory);
        let output_base_std = to_std_path(&inputs.output_base);
        let install_base_std = to_std_path(&inputs.install_base);
        // Java: BazelRepositoryModule.beforeCommand reads these from the command environment.
        let output_user_root_std = output_user_root.map(to_std_path);
        let common_command_options = options.get_options::<CommonCommandOptions>();
        let auth_and_tls_options = options.get_options::<AuthAndTLSOptions>();
        // Null for a command without these options (ProcessWrapper.fromCommandEnvironment).
        let local_execution_options = options.get_options::<LocalExecutionOptions>();
        let repository_functions = blaze_repo_rules::setup::prepare_repository_functions_with(
            &FetchSetup {
                workspace: &workspace_std,
                working_directory: &working_directory_std,
                output_base: &output_base_std,
                install_base: &install_base_std,
                repo_options: &repo_options,
                starlark_semantics: starlark_semantics.clone(),
                client_env: inputs.client_env.clone(),
                repo_env: inputs.repo_env.clone(),
                nonstrict_repo_env: inputs.nonstrict_repo_env.clone(),
                precomputed: repo_precomputed.clone(),
                reporter: inputs.reporter.clone(),
            },
            &CommandInputs {
                output_user_root: output_user_root_std.as_deref(),
                common_command_options: common_command_options.as_ref(),
                auth_and_tls_options: auth_and_tls_options.as_ref(),
                local_execution_options: local_execution_options.as_ref(),
            },
            inputs.output_base.get_file_system().clone(),
        )
        .map_err(|e| match e.code {
            // AbruptExitException(detailedExitCode(message, code)).
            Some(code) => SkyframeExecutorCreationError::Bzlmod {
                message: e.message,
                code,
            },
            None => SkyframeExecutorCreationError::ExternalRepos(e.message),
        })?;
        let mut repository_functions = repository_functions;
        if let Some(builtin_modules) = builtin_modules {
            repository_functions.bzlmod_config.builtin_modules = builtin_modules;
        }
        if let Some(incremental) = &incremental {
            // Java: com/google/devtools/build/lib/bazel/BazelRepositoryModule.java:744 BazelRepositoryModule.getPrecomputedValues
            repository_functions
                .bzlmod_config
                .last_registry_invalidation = incremental
                .state
                .last_registry_invalidation(std::time::SystemTime::now());
        }
        let lockfile_mode = repository_functions.bzlmod_config.lockfile_mode;
        let registry_fetcher = repository_functions.bzlmod_config.registry_fetcher.clone();
        let external_repos: Arc<dyn blaze_sky_loading::repos::ExternalRepos> = Arc::new(
            BzlmodExternalRepos::new(blaze_repo_rules::registration::repository_fetch_function(
                repository_functions.services.clone(),
            )),
        );

        // Java: ruleClassProvider (BazelRuleClassProvider) and its BazelStarlarkEnvironment
        // (with BazelRepositoryModule's repository_rule / module_extension / tag_class).
        let rule_class_provider =
            Arc::new(blaze_packages::native_rules::RuleClassProvider::create_bazel());
        let bazel_starlark_environment =
            blaze_rules::bazel_environment::bazel_starlark_environment_with(
                &rule_class_provider,
                &blaze_repo_rules::registration::add_repository_module_toplevels,
            );
        let mut loading_config = LoadingFunctionsConfig::for_bazel(
            directories.clone(),
            external_repos,
            Arc::new(LoadingFunctionsConfig::default_native_registry()),
            bazel_starlark_environment.clone(),
            Some(rule_class_provider.clone()),
        );
        let repo_contents_cache_path = repository_functions.repo_contents_cache_path.clone();
        loading_config.repo_contents_cache_path = repo_contents_cache_path.clone();
        // Java: syncPackageLoading(packageOptions, ...): the package path, the deleted packages
        // and --show_loading_progress, then (below) the precomputed values of
        // preparePackageLoading.
        // Java: com/google/devtools/build/lib/skyframe/SkyframeExecutor.java:3035 SkyframeExecutor.syncPackageLoading
        let package_options: PackageOptions = options
            .get_options::<PackageOptions>()
            .unwrap_or_else(Options::<PackageOptions>::get_defaults);
        loading_config.sync_package_loading(&package_options);
        let pkg_locator = loading_config.pkg_locator.read().clone();
        if let Some(incremental) = &incremental {
            // Java: SkyframeExecutor.preparePackageLoading `this.tsgm.set(tsgm)`; the server's
            // externalFilesHelper.
            loading_config.tsgm = Some(incremental.tsgm.clone());
            loading_config.external_files_knowledge = incremental.state.external_files_knowledge();
        }
        let deleted_packages_for_sync = loading_config.deleted_packages.read().clone();

        let mut builder = register_bzlmod_functions(
            EvaluatorBuilder::new(),
            Arc::new(repository_functions.bzlmod_config),
        );
        builder = register_loading_functions(builder, &loading_config);
        // Java: com/google/devtools/build/lib/skyframe/SkyframeExecutor.java:1693 SkyframeExecutor.preparePackageLoading
        builder = inject_loading_precomputed_with_package_options(
            builder,
            &loading_config,
            starlark_semantics.clone(),
            repository_functions.repository_overrides,
            &package_options,
        );
        // Java: com/google/devtools/build/lib/skyframe/SkyframeExecutor.java:1600 SkyframeExecutor.setCommandId
        // (PrecomputedValue.BUILD_ID.set(injectable(), commandId)).
        // PORT: an executor outside a server command gets an empty id (its graph is its own).
        builder = blaze_sky_loading::precomputed::BUILD_ID.set(
            builder,
            incremental
                .as_ref()
                .map(|i| i.command_id.clone())
                .unwrap_or_default(),
        );
        builder = blaze_repo_rules::registration::register_repo_rules_functions(
            builder,
            repository_functions.services,
        );
        builder =
            blaze_repo_rules::registration::inject_repo_precomputed(builder, &repo_precomputed);
        builder = register_target_pattern_phase_functions(builder);
        // Java: ProjectFilesLookupFunction (SkyFunctions.PROJECT_FILES_LOOKUP) and
        // ProjectFunction (SkyFunctions.PROJECT).
        builder = crate::project::register(builder);

        let mut analysis_services = None;
        let mut configuration = None;
        let mut execution = None;
        let action_conflicts: ActionConflictsCell =
            Arc::new(parking_lot::RwLock::new(Arc::new(ActionConflicts::new())));
        if let Some(build_options) = inputs.top_level_build_options {
            // Java: done by the rule class provider's construction (subrule calls, fragment
            // Starlark values, build info files).
            blaze_rules::register::install_analysis_hooks();
            let config_directories = ConfigDirectories {
                exec_root_base: inputs.exec_root_base.clone(),
                product_name: inputs.product_name.clone(),
            };
            let services = Arc::new(AnalysisServices {
                rule_class_provider: Arc::new(
                    blaze_rules::register::create_bazel_rule_class_provider(),
                ),
                bazel_starlark_environment: bazel_starlark_environment.clone(),
                // Java: new ArtifactFactory(directories.getExecRootBase(),
                // directories.getRelativeOutputPath()).
                artifact_factory: Arc::new(ArtifactFactory::new(
                    inputs.exec_root_base.clone(),
                    &config_directories.get_relative_output_path(),
                )),
                action_key_context: Arc::new(ActionKeyContext::new()),
                starlark_semantics: Arc::new(starlark_semantics.clone()),
                directories: AnalysisDirectories {
                    workspace: inputs.workspace.clone(),
                    output_base: inputs.output_base.clone(),
                    exec_root: inputs.exec_root_base.get_child(&inputs.workspace_name),
                    workspace_name: inputs.workspace_name.clone(),
                    product_name: inputs.product_name.clone(),
                },
                main_repo_mapping: parking_lot::RwLock::new(None),
                keep_going: inputs.keep_going,
                workspace_status: std::sync::OnceLock::new(),
                // Java: StandardSystemProperty.USER_NAME (the server's user.name) and
                // NetUtil.getCachedShortHostName().
                workspace_status_user_name: blaze_exec::exec::test_policy::get_user_name(),
                workspace_status_host_name: short_host_name(),
                extra_action_filter: parking_lot::RwLock::new(None),
                base_target_prerequisites_supplier: std::sync::OnceLock::new(),
            });
            let platforms: SharedPlatformValueProvider = Arc::new(
                blaze_sky_analysis::platform::PlatformFunction::new(services.clone()),
            );
            let baseline = Arc::new(build_options);
            // Java: SkyframeExecutor.getStarlarkExecTransition loads the exec transition's .bzl
            // with BZL_LOAD. A file in @_builtins is evaluated by the bundled loader now; another
            // file is loaded through the evaluator once it is built (below).
            let baseline_exec_configuration =
                blaze_build_config::skyframe::PrecomputedOptions::new(None);
            if !exec_config_needs_bzl_load(&baseline) {
                let baseline_exec = Self::adjust_for_exec(
                    &baseline,
                    &Arc::new(starlark_semantics.clone()),
                    inputs.reporter.as_ref(),
                )
                .map_err(SkyframeExecutorCreationError::InvalidConfiguration)?;
                baseline_exec_configuration.set(Arc::new(baseline_exec));
            }
            let client = Arc::new(ConfigurationEnvironment {
                directories: config_directories,
                rule_class_provider: Arc::new(BazelRuleClassProvider::create()),
                starlark_semantics: Arc::new(starlark_semantics.clone()),
                package_path_roots: pkg_locator
                    .get_path_entries()
                    .iter()
                    .filter_map(|r| r.as_path().cloned())
                    .collect(),
                baseline_configuration: blaze_build_config::skyframe::PrecomputedOptions::new(
                    Some(baseline.clone()),
                ),
                baseline_exec_configuration,
                platforms,
                main_repository_mapping: Arc::new(MainRepositoryMappingFromSkyframe),
                file_lookup: Arc::new(FileValueFromSkyframe),
            });
            builder = register_configuration_functions(builder).client(client.clone());
            // Java: com/google/devtools/build/lib/skyframe/SkyframeExecutor.java:1537 SkyframeExecutor.setBaselineConfiguration
            builder = blaze_build_config::skyframe::BASELINE_CONFIGURATION
                .set(builder, client.baseline_configuration.get());
            if client.baseline_exec_configuration.is_set() {
                builder = blaze_build_config::skyframe::BASELINE_EXEC_CONFIGURATION
                    .set(builder, client.baseline_exec_configuration.get());
            }
            // Java: SkyFunctions.FLAG_SET (FlagSetFunction), requested by Project.applySclConfig.
            builder = blaze_sky_analysis::flag_set_function::register(builder);
            // Java: SkyFunctions.PARSED_FLAGS (ParsedFlagsFunction), requested by
            // PLATFORM_MAPPING and PLATFORM (platform-based flags).
            builder = blaze_sky_analysis::parsed_flags_function::register(
                builder,
                client
                    .rule_class_provider
                    .get_fragment_registry()
                    .get_options_classes()
                    .to_vec(),
            );
            // Java: SkyFunctions.BUILD_OPTIONS_SCOPE (BuildOptionsScopeFunction), requested by
            // BUILD_CONFIGURATION_KEY for Starlark flags whose scope is not known yet.
            builder = blaze_sky_analysis::build_options_scope_function::register(builder);
            builder = blaze_sky_analysis::toolchains::register_toolchain_functions(
                builder,
                services.clone(),
            );
            builder =
                blaze_sky_analysis::configured_target_function::register(builder, services.clone());
            // cquery --output=transitions (CqueryTransitionResolver; see the module).
            builder =
                blaze_sky_analysis::cquery_transition_resolver::register(builder, services.clone());
            // Java: com/google/devtools/build/lib/skyframe/SkyframeExecutor.java:858 SkyframeExecutor.skyFunctions
            // (ASPECT, TOP_LEVEL_ASPECTS, LOAD_ASPECTS, in Java's order).
            builder = blaze_sky_analysis::aspect_function::register(builder, services.clone());
            builder = blaze_sky_analysis::toplevel_starlark_aspect_function::register(
                builder,
                services.clone(),
            );
            builder = blaze_sky_analysis::load_aspects_function::register(builder);
            // Java: GenQueryPackageProviderFactory.GENQUERY_SCOPE (BazelRulesModule's
            // getSkyFunctions).
            builder = blaze_rules::genquery::register_sky_functions(builder);
            // Java: ActionLookupConflictFindingFunction and
            // TopLevelActionLookupConflictFindingFunction.
            builder = action_lookup_conflict_finding_function::register(
                builder,
                action_conflicts.clone(),
            );
            // Java: WorkspaceStatusFunction (SkyFunctions.BUILD_INFO).
            let status_services = services.clone();
            builder = builder
                .typed_function::<blaze_analysis::analysis_services::WorkspaceStatusKey, _>(
                    move |_, env| {
                        // PORT: Java's WorkspaceStatusFunction reads WORKSPACE_STATUS_KEY, which
                        // SkyframeExecutor.injectWorkspaceStatusData injects per build with a new
                        // action; BUILD_ID changes on the same schedule.
                        if blaze_sky_loading::precomputed::BUILD_ID.get(env)?.is_none() {
                            return Ok(None);
                        }
                        Ok(Some(status_services.compute_workspace_status_value()))
                    },
                );
            // Java: CoverageReportFunction (SkyFunctions.COVERAGE_REPORT).
            // Java: com/google/devtools/build/lib/skyframe/SkyframeExecutor.java:932 SkyframeExecutor.skyFunctions
            let coverage_services = services.clone();
            builder = builder
                .typed_function::<blaze_analysis::coverage_report_value::CoverageReportKey, _>(
                    move |_, env| {
                        blaze_analysis::coverage_report_value::compute_coverage_report_value(
                            &coverage_services.action_key_context,
                            env,
                        )
                    },
                );
            if let Some(mut setup) = inputs.execution {
                // Java: CommandEnvironment's package locator path entries and
                // `--repo_contents_cache` (the sandbox mounts those under a hermetic /tmp).
                setup.config.package_path_entries = pkg_locator
                    .get_path_entries()
                    .iter()
                    .filter_map(|r| r.as_path().cloned())
                    .collect();
                setup.config.repo_contents_cache_path = repo_contents_cache_path.clone();
                let (b, state) = crate::execution_phase::prepare_execution(
                    builder,
                    setup,
                    &services,
                    inputs.reporter.clone(),
                )
                .map_err(SkyframeExecutorCreationError::Execution)?;
                builder = b;
                if let Some(incremental) = &incremental {
                    state
                        .prepared
                        .executor
                        .set_timestamp_granularity_monitor(incremental.tsgm.clone());
                }
                execution = Some(state);
            }
            analysis_services = Some(services);
            configuration = Some(ConfigurationState { client });
        }

        if let Some(extra_sky_functions) = extra_sky_functions {
            builder = extra_sky_functions(builder);
        }
        // Java: com/google/devtools/build/lib/skyframe/SkyframeExecutor.java:3493 SkyframeExecutor.SkyframeProgressReceiver
        // PORT: installed only for a command that executes: it forwards only to the execution
        // progress receiver (see SkyframeProgressReceiver), and a receiver makes the evaluator
        // hand every node's direct deps to it.
        let progress_receiver =
            Arc::new(crate::skyframe_progress_receiver::SkyframeProgressReceiver::new());
        // Over a graph kept across commands the analysis counts come from this receiver too
        // (see `skyframe_build_view::EvaluatedCounts`).
        if execution.is_some() || incremental.is_some() {
            builder = builder.progress_receiver(progress_receiver.clone());
        }
        let (evaluator, incremental_handle) = match &incremental {
            Some(incremental) => {
                let (evaluator, handle) =
                    crate::incremental::SkyframeIncrementalState::build_evaluator(
                        incremental,
                        builder,
                        crate::incremental::SyncInputs {
                            signature: crate::incremental::LoadingSignature::new(
                                options,
                                &starlark_semantics,
                                &inputs.working_directory,
                                &pkg_locator,
                            ),
                            pkg_locator: loading_config.pkg_locator.clone(),
                            deleted_packages: deleted_packages_for_sync,
                            directories: directories.clone(),
                            repo_contents_cache_path: repo_contents_cache_path.clone(),
                            options,
                            client_env: &inputs.client_env,
                            repo_env: &inputs.repo_env,
                            reporter: inputs.reporter.clone(),
                            executes,
                            top_level_build_options: top_level_build_options_for_sync,
                        },
                    );
                (Arc::new(evaluator), Some(handle))
            }
            None => (Arc::new(builder.build()), None),
        };
        if let Some(state) = &configuration
            && !state.client.baseline_exec_configuration.is_set()
        {
            // Java: com/google/devtools/build/lib/skyframe/SkyframeExecutor.java:1544 SkyframeExecutor.adjustForExec
            let baseline_exec = adjust_for_exec_with_evaluator(
                &evaluator,
                &state.client.baseline_configuration.get(),
                &state.client.starlark_semantics,
                inputs.reporter.clone(),
            )
            .map_err(SkyframeExecutorCreationError::InvalidConfiguration)?;
            let baseline_exec = Arc::new(baseline_exec);
            blaze_build_config::skyframe::BASELINE_EXEC_CONFIGURATION
                .set_on(&evaluator, baseline_exec.clone());
            state.client.baseline_exec_configuration.set(baseline_exec);
        }
        if let Some(services) = &analysis_services {
            // Java: com/google/devtools/build/lib/skyframe/SkyframeExecutor.java:4598 SkyframeExecutor.BaseTargetPrerequisitesSupplierImpl
            // (`memoizingEvaluator.getExistingValue(key)`). The functions of the evaluator hold
            // the services, so the services hold the evaluator weakly.
            let weak_evaluator = Arc::downgrade(&evaluator);
            let _ = services.base_target_prerequisites_supplier.set(
                blaze_analysis::base_target_prerequisites_supplier::BaseTargetPrerequisitesSupplier::new(
                    Arc::new(move |key| {
                        weak_evaluator
                            .upgrade()
                            .and_then(|evaluator| evaluator.get_existing_value(key))
                    }),
                ),
            );
        }
        let package_manager = Arc::new(SkyframePackageManager::new(
            evaluator.clone(),
            SkyframePackageLocator {
                pkg_locator: loading_config.pkg_locator.clone(),
                deleted_packages: loading_config.deleted_packages.clone(),
            },
        ));
        // Java: com/google/devtools/build/lib/skyframe/SkyframeExecutor.java:3092 SkyframeExecutor.createCyclesReporter
        // PORT: of the seven reporters TargetCycleReporter, RegisteredToolchainsCycleReporter and
        // RegisteredExecutionPlatformsCycleReporter are ported (ActionArtifact, TestExpansion,
        // BzlLoad and BzlmodRepo are not); CyclesReporter fails loudly for a cycle none of them
        // reports (see cycles_reporter.rs). The order is Java's.
        let cycle_reporters: Vec<Box<dyn SingleCycleReporter>> = vec![
            Box::new(TargetCycleReporter::new(package_manager.clone())),
            Box::new(crate::toolchain_cycle_reporters::RegisteredToolchainsCycleReporter),
            Box::new(crate::toolchain_cycle_reporters::RegisteredExecutionPlatformsCycleReporter),
        ];
        let cycles_reporter = CyclesReporter::new(cycle_reporters);
        Ok(SkyframeExecutor {
            progress_receiver,
            cycles_reporter,
            action_conflicts,
            top_level_artifact_context_for_conflict_pruning: parking_lot::Mutex::new(None),
            evaluator,
            package_manager,
            pkg_locator,
            starlark_semantics,
            digest_function,
            reporter: inputs.reporter,
            analysis_services,
            configuration,
            rule_class_provider,
            workspace: workspace_std,
            output_base: output_base_std,
            lockfile_mode,
            execution,
            registry_fetcher,
            _arena_lease: arena_lease,
            incremental: incremental_handle,
            options: inputs.options.clone(),
        })
    }

    /// The exec baseline: the top-level options through the Starlark exec transition.
    // Java: com/google/devtools/build/lib/skyframe/SkyframeExecutor.java:1541 SkyframeExecutor.adjustForExec
    fn adjust_for_exec(
        build_options: &BuildOptions,
        semantics: &Arc<StarlarkSemantics>,
        event_handler: &dyn EventHandler,
    ) -> Result<BuildOptions, InvalidConfigurationException> {
        let exec_transition =
            blaze_build_config::starlark_transition::load_starlark_exec_transition(
                Some(build_options),
                semantics,
            )
            .map_err(|e| InvalidConfigurationException::new(e.0))?;
        // Get the current target platform and use it as the exec platform.
        // This value isn't actually important as long as it exists and is stable.
        let host_platform = build_options
            .get::<blaze_optdefs::fragment::platform_options::PlatformOptions>()
            .map(|p| p.host_platform.0.clone());
        match exec_transition {
            Some(t) => Ok(
                blaze_build_config::function_transition_util::adjust_for_exec(
                    build_options,
                    &t,
                    host_platform.as_ref(),
                    semantics,
                    event_handler,
                ),
            ),
            None => blaze_util::unported!(
                "ExecutionTransitionFactory.create",
                "no Starlark exec transition (native exec transition is not in Bazel 9)"
            ),
        }
    }

    /// The prepared execution phase of a command that executes actions.
    pub fn get_execution(&self) -> Option<&crate::execution_phase::ExecutionState> {
        self.execution.as_ref()
    }

    /// The source roots of every package loaded by this command (Java
    /// `SkyframeBuildView` / `PackageRoots` collected from the analyzed targets' transitive
    /// packages).
    ///
    /// PORT: Java collects the transitive packages of the top-level targets; Blaze takes every
    /// package loaded in the command's evaluator (a superset: the symlink forest then also links
    /// packages only loaded for target pattern expansion).
    pub fn get_package_roots(&self) -> IndexMap<PackageIdentifier, blaze_vfs::Root> {
        let mut roots: Vec<(PackageIdentifier, blaze_vfs::Root)> = Vec::new();
        for key in self.evaluator.get_all_keys_for_testing() {
            if key.function_name() != PackageKey::FUNCTION {
                continue;
            }
            let Some(value) = self.evaluator.get_existing_value(&key) else {
                continue;
            };
            let Ok(value) = value.downcast::<PackageValue>() else {
                continue;
            };
            let package = value.get_package();
            roots.push((
                package.get_package_identifier().clone(),
                package.get_source_root().clone(),
            ));
        }
        roots.sort_by_key(|a| a.0.to_string());
        roots.into_iter().collect()
    }

    /// Writes the workspace and hidden lockfiles from this command's evaluation (Java
    /// `BazelLockFileModule.afterCommand`, run after every command).
    // Java: com/google/devtools/build/lib/bazel/bzlmod/BazelLockFileModule.java:62 BazelLockFileModule.afterCommand
    pub fn after_command(&self) {
        blaze_bzlmod::skyframe::after_command(
            &self.evaluator,
            &self.workspace,
            &self.output_base,
            self.lockfile_mode,
        );
    }

    // Java: com/google/devtools/build/lib/skyframe/SkyframeExecutor.java:1105 SkyframeExecutor.getEvaluator
    pub fn get_evaluator(&self) -> &Arc<Evaluator> {
        &self.evaluator
    }

    /// The executor's progress receiver, whose execution progress receiver
    /// `setExecutionProgressReceiver` sets.
    pub fn get_progress_receiver(
        &self,
    ) -> &Arc<crate::skyframe_progress_receiver::SkyframeProgressReceiver> {
        &self.progress_receiver
    }

    /// The graph after evaluation (Java `SkyframeExecutorWrappingWalkableGraph.of(this)`).
    pub fn get_walkable_graph(&self) -> Arc<dyn WalkableGraph> {
        self.evaluator.clone()
    }

    // Java: com/google/devtools/build/lib/skyframe/SkyframeExecutor.java:1590 SkyframeExecutor.getPackageManager
    pub fn get_package_manager(&self) -> &Arc<SkyframePackageManager> {
        &self.package_manager
    }

    /// `getPackageManager().getPackagePath()`.
    pub fn get_package_path(&self) -> &PathPackageLocator {
        &self.pkg_locator
    }

    /// `getEffectiveStarlarkSemantics(buildLanguageOptions)`.
    pub fn get_effective_starlark_semantics(&self) -> &StarlarkSemantics {
        &self.starlark_semantics
    }

    /// `getDigestFunction()`.
    pub fn get_digest_function(&self) -> DigestHashFunction {
        self.digest_function
    }

    /// The services the analysis functions share (`None` for commands that do not analyze).
    pub fn get_analysis_services(&self) -> Option<&Arc<AnalysisServices>> {
        self.analysis_services.as_ref()
    }

    /// The loading-phase rule class provider.
    pub fn get_rule_class_provider(&self) -> &Arc<blaze_packages::native_rules::RuleClassProvider> {
        &self.rule_class_provider
    }

    /// The baseline (top-level) build options set at construction.
    pub fn get_baseline_configuration(&self) -> Option<Arc<BuildOptions>> {
        self.configuration
            .as_ref()
            .map(|c| c.client.baseline_configuration.get())
    }

    /// `getActionKeyContext()`.
    pub fn get_action_key_context(&self) -> Option<&Arc<ActionKeyContext>> {
        self.analysis_services
            .as_ref()
            .map(|s| &s.action_key_context)
    }

    // Java: com/google/devtools/build/lib/skyframe/SkyframeExecutor.java:3105 SkyframeExecutor.getCyclesReporter
    pub fn get_cycles_reporter(&self) -> &CyclesReporter {
        &self.cycles_reporter
    }

    /// Sets the `TopLevelArtifactContext` (its output groups) of the request about to be
    /// analyzed, which `SkyframeBuildView.configureTargets` prunes action conflicts with.
    ///
    /// PORT: Java passes `request.getTopLevelArtifactContext()` down through `BuildView.update`
    /// (`topLevelOptions`); see `skyframe_build_view::configure_targets`.
    // Java: com/google/devtools/build/lib/analysis/BuildView.java:216 BuildView.update
    pub fn set_top_level_artifact_context_for_conflict_pruning(&self, output_groups: Vec<String>) {
        *self.top_level_artifact_context_for_conflict_pruning.lock() = Some(output_groups);
    }

    /// The output groups set by
    /// [`SkyframeExecutor::set_top_level_artifact_context_for_conflict_pruning`].
    pub fn get_top_level_artifact_context_for_conflict_pruning(&self) -> Option<Vec<String>> {
        self.top_level_artifact_context_for_conflict_pruning
            .lock()
            .clone()
    }

    /// Checks the given action lookup values for action conflicts. Values satisfying the returned
    /// predicate are known to be transitively error-free from action conflicts or other analysis
    /// failures.
    ///
    /// PORT: skipped, incremental: Java deletes the conflict-finding nodes afterwards
    /// (`memoizingEvaluator.delete`, `resetActionConflictsStoredInSkyframe`) to free memory and
    /// so that a later build recomputes them. A command analyzes once, so they are never read
    /// with other conflicts.
    // Java: com/google/devtools/build/lib/skyframe/SkyframeExecutor.java:2541 SkyframeExecutor.filterActionConflictsForConfiguredTargetsAndAspects
    pub fn filter_action_conflicts_for_configured_targets_and_aspects(
        &self,
        event_handler: Arc<dyn ExtendedEventHandler>,
        keys: &[blaze_actions::ActionLookupKeyRef],
        action_conflicts: &ActionConflicts,
        top_level_artifact_context: &[String],
    ) -> Result<TopLevelActionConflictReport, Interrupted> {
        // ACTION_CONFLICTS.set(injectable(), actionConflicts)
        *self.action_conflicts.write() = Arc::new(action_conflicts.clone());
        let top_level_keys: Vec<SkyKey> =
            action_lookup_conflict_finding_function::keys(keys, top_level_artifact_context)
                .into_iter()
                .map(SkyKey::new)
                .collect();
        // This work is CPU-bound, so use the number of available processors.
        let result = self.evaluate(
            &top_level_keys,
            /* keep_going= */ true,
            /* num_threads= */
            std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(1),
            event_handler,
        )?;
        Ok(TopLevelActionConflictReport {
            result,
            top_level_artifact_context: top_level_artifact_context.to_vec(),
        })
    }

    /// Evaluates `roots` (Java `SkyframeExecutor.evaluate(roots, keepGoing, numThreads,
    /// eventHandler)`).
    // Java: com/google/devtools/build/lib/skyframe/SkyframeExecutor.java:2228 SkyframeExecutor.evaluate
    pub fn evaluate(
        &self,
        roots: &[SkyKey],
        keep_going: bool,
        num_threads: usize,
        event_handler: Arc<dyn ExtendedEventHandler>,
    ) -> Result<EvaluationResult, Interrupted> {
        self.package_manager
            .evaluate(roots, keep_going, num_threads, event_handler)
    }

    /// Returns the main repository's mapping, evaluating it if needed.
    // Java: com/google/devtools/build/lib/skyframe/SkyframeExecutor.java:3420 SkyframeExecutor.getMainRepoMapping
    pub fn get_main_repo_mapping(
        &self,
        keep_going: bool,
        loading_phase_threads: usize,
        event_handler: Arc<dyn ExtendedEventHandler>,
    ) -> Result<RepositoryMapping, RepositoryMappingResolutionOrInterrupted> {
        let mapping = self.package_manager.get_main_repo_mapping(
            keep_going,
            loading_phase_threads,
            event_handler,
        )?;
        if let Some(services) = &self.analysis_services {
            *services.main_repo_mapping.write() = Some(mapping.clone());
            if let Some(handle) = &self.incremental {
                handle
                    .state()
                    .note_main_repo_mapping(&self.evaluator, &mapping);
            }
        }
        Ok(mapping)
    }

    /// Returns flag aliases from `MODULE.bazel` `flag_alias()` definitions (plus the hard-coded
    /// Python aliases), keyed by the native flag name (Java `getFlagAliases`; the bodies are
    /// `blaze_build_config::flag_aliases`). `None` when the dependency graph has errors (Java:
    /// the caller computed the main repository mapping first, which reported them).
    // Java: com/google/devtools/build/lib/skyframe/SkyframeExecutor.java:3325 SkyframeExecutor.getFlagAliases
    pub fn get_flag_aliases(
        &self,
        event_handler: Arc<dyn ExtendedEventHandler>,
        ensure_py_aliases: bool,
        ensure_bazel_py_aliases: bool,
    ) -> Result<Option<IndexMap<String, String>>, Interrupted> {
        let key = blaze_bzlmod::skyframe::keys::BazelDepGraphKey;
        let eval_result = self.evaluate(
            &[SkyKey::new(key)],
            false,
            DEFAULT_THREAD_COUNT,
            event_handler,
        )?;
        let Some(dep_graph_value) = eval_result.get(&key) else {
            return Ok(None);
        };
        Ok(Some(
            blaze_build_config::flag_aliases::get_flag_aliases_from_dep_graph(
                &dep_graph_value,
                ensure_py_aliases,
                ensure_bazel_py_aliases,
            ),
        ))
    }

    /// Returns the already loaded package, if any.
    // Java: com/google/devtools/build/lib/skyframe/SkyframeExecutor.java:2864 SkyframeExecutor.getExistingPackage
    pub fn get_existing_package(&self, id: &PackageIdentifier) -> Option<Arc<Package>> {
        self.evaluator
            .get_existing_value(&SkyKey::new(PackageKey::key(id.clone())))
            .map(|v| {
                v.downcast::<PackageValue>()
                    .unwrap_or_else(|_| panic!("PACKAGE value of {}", id))
                    .get_package()
                    .clone()
            })
    }

    /// Evaluates the target patterns of a build (Java `loadTargetPatternsWithFilters`).
    // Java: com/google/devtools/build/lib/skyframe/SkyframeExecutor.java:3179 SkyframeExecutor.loadTargetPatternsWithFilters
    pub fn load_target_patterns_with_filters(
        &self,
        event_handler: Arc<dyn ExtendedEventHandler>,
        target_patterns: &[String],
        relative_working_directory: &PathFragment,
        options: &LoadingOptions,
        thread_count: usize,
        keep_going: bool,
        determine_tests: bool,
    ) -> Result<TargetPatternPhaseValue, TargetPatternPhaseError> {
        self.load_target_patterns_with_filters_detailed(
            event_handler,
            target_patterns,
            relative_working_directory,
            options,
            thread_count,
            keep_going,
            determine_tests,
        )
        .map_err(TargetPatternPhaseError::from)
    }

    /// [`SkyframeExecutor::load_target_patterns_with_filters`] whose error keeps
    /// `TargetParsingException.getDetailedExitCode()`.
    // Java: com/google/devtools/build/lib/skyframe/SkyframeExecutor.java:3179 SkyframeExecutor.loadTargetPatternsWithFilters
    pub fn load_target_patterns_with_filters_detailed(
        &self,
        event_handler: Arc<dyn ExtendedEventHandler>,
        target_patterns: &[String],
        relative_working_directory: &PathFragment,
        options: &LoadingOptions,
        thread_count: usize,
        keep_going: bool,
        determine_tests: bool,
    ) -> Result<TargetPatternPhaseValue, TargetPatternPhaseDetailedError> {
        let key = TargetPatternPhaseKey::key(
            target_patterns.to_vec(),
            relative_working_directory.clone(),
            options.compile_one_dependency,
            options.build_tests_only,
            determine_tests,
            options.build_tag_filter_list.clone(),
            options.build_manual_tests,
            options.expand_test_suites,
            Some(TestFilter::for_options(options)),
        );
        self.get_target_pattern_phase_value(
            event_handler,
            target_patterns,
            thread_count,
            keep_going,
            key,
        )
    }

    /// Loads the given target patterns without applying any filters (such as removing non-test
    /// targets if `--build_tests_only` is set).
    // Java: com/google/devtools/build/lib/skyframe/SkyframeExecutor.java:3156 SkyframeExecutor.loadTargetPatternsWithoutFilters
    pub fn load_target_patterns_without_filters(
        &self,
        event_handler: Arc<dyn ExtendedEventHandler>,
        target_patterns: &[String],
        relative_working_directory: &PathFragment,
        thread_count: usize,
        keep_going: bool,
    ) -> Result<TargetPatternPhaseValue, TargetPatternPhaseError> {
        // Java: com/google/devtools/build/lib/skyframe/TargetPatternPhaseValue.java:172 TargetPatternPhaseValue.keyWithoutFilters
        let key = TargetPatternPhaseKey::key(
            target_patterns.to_vec(),
            relative_working_directory.clone(),
            false,
            false,
            false,
            Vec::new(),
            false,
            false,
            None,
        );
        self.get_target_pattern_phase_value(
            event_handler,
            target_patterns,
            thread_count,
            keep_going,
            key,
        )
        .map_err(TargetPatternPhaseError::from)
    }

    // Java: com/google/devtools/build/lib/skyframe/SkyframeExecutor.java:3201 SkyframeExecutor.getTargetPatternPhaseValue
    fn get_target_pattern_phase_value(
        &self,
        event_handler: Arc<dyn ExtendedEventHandler>,
        target_patterns: &[String],
        thread_count: usize,
        keep_going: bool,
        key: TargetPatternPhaseKey,
    ) -> Result<TargetPatternPhaseValue, TargetPatternPhaseDetailedError> {
        // Java: com/google/devtools/build/lib/skyframe/SkyframeExecutor.java:3210 SkyframeExecutor.getTargetPatternPhaseValue
        event_handler.post(Arc::new(
            blaze_sky_loading::package_progress_receiver::LoadingPhaseStartedEvent::new(
                &blaze_sky_loading::package_progress_receiver::PACKAGE_PROGRESS,
            ),
        ));
        // PORT: TargetParsingPhaseTimeEvent feeds the build metrics; the command measures the
        // phase itself (blaze_runtime).
        let sky_key = SkyKey::new(key.clone());
        let eval_result = self
            .evaluate(
                std::slice::from_ref(&sky_key),
                keep_going,
                thread_count,
                event_handler.clone(),
            )
            .map_err(|_| TargetPatternPhaseDetailedError::Interrupted)?;
        self.try_throw_target_parsing_exception(
            &event_handler,
            target_patterns,
            &sky_key,
            &eval_result,
        )?;
        Ok(eval_result
            .get(&key)
            .expect("TARGET_PATTERN_PHASE value without error")
            .as_ref()
            .clone())
    }

    // Java: com/google/devtools/build/lib/skyframe/SkyframeExecutor.java:3217 SkyframeExecutor.tryThrowTargetParsingException
    fn try_throw_target_parsing_exception(
        &self,
        event_handler: &Arc<dyn ExtendedEventHandler>,
        target_patterns: &[String],
        key: &SkyKey,
        eval_result: &EvaluationResult,
    ) -> Result<(), TargetPatternPhaseDetailedError> {
        if eval_result.has_error() {
            let error_info = eval_result
                .get_error(key)
                .unwrap_or_else(|| panic!("NullPointerException: no error for {:?}", key));
            let exc = if !error_info.get_cycle_info().is_empty() {
                let exc = TargetParsingException::new(
                    "cycles detected during target parsing",
                    TargetPatternsCode::Cycle,
                );
                self.cycles_reporter
                    .report_cycles(error_info.get_cycle_info(), key, event_handler);
                // Fallback: we don't know which patterns failed, specifically, so we report the
                // entire set as being in error.
                event_handler.post(Arc::new(PatternExpandingError::failed(
                    target_patterns.to_vec(),
                    exc.get_message(),
                )));
                let detailed_exit_code = target_parsing_detailed_exit_code(&exc);
                TargetPatternPhaseDetailedError::TargetParsing {
                    exception: exc,
                    detailed_exit_code,
                }
            } else {
                construct_no_cycle_target_parsing_exception(
                    event_handler.as_ref(),
                    target_patterns,
                    error_info,
                )
            };
            return Err(exc);
        }
        Ok(())
    }

    // -----------------------------------------------------------------------------------------
    // Configurations.

    fn configuration_state(&self) -> &ConfigurationState {
        self.configuration
            .as_ref()
            .expect("IllegalStateException: the command does not configure targets")
    }

    /// The shared body of `createBuildConfigurationKey` and `getConfiguration`.
    /// `wrap_any_exception`: `createBuildConfigurationKey` wraps every exception that is not an
    /// `InvalidConfigurationException` ("so wrap it and throw"); `getConfiguration` treats one
    /// as an unknown error.
    // Java: com/google/devtools/build/lib/skyframe/SkyframeExecutor.java:2204 SkyframeExecutor.createBuildConfigurationKey
    // Java: com/google/devtools/build/lib/skyframe/SkyframeExecutor.java:2107 SkyframeExecutor.getConfiguration
    fn evaluate_configuration<K: Key + Clone>(
        &self,
        key: K,
        keep_going: bool,
        wrap_any_exception: bool,
        event_handler: Arc<dyn ExtendedEventHandler>,
    ) -> Result<Arc<K::Value>, InvalidConfigurationException> {
        let sky_key = SkyKey::new(key.clone());
        let result = self
            .evaluate(
                std::slice::from_ref(&sky_key),
                keep_going,
                DEFAULT_THREAD_COUNT,
                event_handler.clone(),
            )
            .map_err(|_| InvalidConfigurationException::new("interrupted"))?;
        if let Some(v) = result.get(&key) {
            return Ok(v);
        }
        let (first_error_key, error) = &result.error_map()[0];
        if let Some(e) = error.exception_as::<ConfigurationError>() {
            return Err(match e {
                ConfigurationError::InvalidConfiguration(e) => e.clone(),
                other => InvalidConfigurationException::with_code(
                    other.to_string(),
                    "PLATFORM_MAPPING_EVALUATION_FAILURE",
                ),
            });
        }
        // Java: case DetailedException detailedException ->
        //     throw new InvalidConfigurationException(detailedException.getDetailedExitCode(), e);
        // PORT: the DetailedExceptions that reach the configuration functions are
        // ToolchainExceptions (a bad --host_platform / --platforms target); the wrapper carries
        // the Toolchain code as `Toolchain.<CODE>` (see invalid_configuration_detailed_exit_code).
        if !wrap_any_exception
            && let Some(e) = error.exception_as::<blaze_analysis::toolchains::ToolchainException>()
        {
            return Err(InvalidConfigurationException::with_code(
                e.get_message(),
                toolchain_code_of_invalid_configuration(e.get_detailed_code()),
            ));
        }
        if wrap_any_exception {
            // Wrap exceptions related to loading.
            let no_such_thing = error
                .exception_as::<blaze_packages::exceptions::NoSuchThingException>()
                .map(|e| (e.get_message(), e.get_detailed_exit_code()))
                .or_else(|| {
                    error
                        .exception_as::<blaze_packages::exceptions::NoSuchPackageException>()
                        .map(|e| (e.get_message(), e.get_detailed_exit_code()))
                })
                .or_else(|| {
                    error
                        .exception_as::<blaze_packages::exceptions::NoSuchTargetException>()
                        .map(|e| (e.to_string(), e.get_detailed_exit_code()))
                });
            if let Some((message, detailed_exit_code)) = no_such_thing {
                return Err(match detailed_exit_code.code {
                    blaze_packages::exceptions::FailureCode::PackageLoading(code) => {
                        InvalidConfigurationException::with_package_loading_code(
                            message,
                            code as i32,
                        )
                    }
                    blaze_packages::exceptions::FailureCode::Other(other) => {
                        InvalidConfigurationException::with_wrapped_failure_code(message, other)
                    }
                });
            }
        }
        if wrap_any_exception && let Some(e) = error.get_exception() {
            // If we get here, e is non-null but not an InvalidConfigurationException, so wrap it
            // and throw (a toolchain or platform lookup error, ...).
            return Err(InvalidConfigurationException::with_code(
                e.to_string(),
                "PLATFORM_MAPPING_EVALUATION_FAILURE",
            ));
        }
        if error.get_exception().is_none() && !error.get_cycle_info().is_empty() {
            self.cycles_reporter.report_cycles(
                error.get_cycle_info(),
                first_error_key,
                &event_handler,
            );
            return Err(InvalidConfigurationException::with_code(
                "cannot load build configuration because of this cycle",
                "CYCLE",
            ));
        }
        panic!(
            "IllegalStateException: Unknown error during configuration creation evaluation: {:?}",
            error.get_exception()
        );
    }

    // Java: com/google/devtools/build/lib/skyframe/SkyframeExecutor.java:2113 SkyframeExecutor.createBuildConfigurationKey
    pub fn create_build_configuration_key(
        &self,
        event_handler: Arc<dyn ExtendedEventHandler>,
        build_options: Arc<BuildOptions>,
    ) -> Result<BuildConfigurationKey, InvalidConfigurationException> {
        Ok(self
            .evaluate_configuration(
                BuildConfigurationKeyValueKey::create(build_options),
                false,
                true,
                event_handler,
            )?
            .as_ref()
            .clone())
    }

    /// Returns the configuration corresponding to the given set of build options.
    // Java: com/google/devtools/build/lib/skyframe/SkyframeExecutor.java:1903 SkyframeExecutor.getConfiguration
    pub fn get_configuration(
        &self,
        event_handler: Arc<dyn ExtendedEventHandler>,
        build_options: Arc<BuildOptions>,
        keep_going: bool,
    ) -> Result<Arc<BuildConfigurationValue>, InvalidConfigurationException> {
        self.configuration_state();
        // Prepare the Skyframe inputs.
        let build_configuration_key =
            self.create_build_configuration_key(event_handler.clone(), build_options)?;
        // Skyframe-evaluate the configurations and throw errors if any.
        self.evaluate_configuration(build_configuration_key, keep_going, false, event_handler)
    }

    /// Returns the configuration of an already mapped key (Java `getConfiguration(eventHandler,
    /// configurationKey)`).
    // Java: com/google/devtools/build/lib/skyframe/SkyframeExecutor.java:1930 SkyframeExecutor.getConfiguration
    pub fn get_configuration_for_key(
        &self,
        event_handler: Arc<dyn ExtendedEventHandler>,
        key: &BuildConfigurationKey,
    ) -> Option<Arc<BuildConfigurationValue>> {
        if let Some(v) = self.evaluator.get_existing_value(&SkyKey::new(key.clone())) {
            return Some(
                v.downcast::<BuildConfigurationValue>()
                    .unwrap_or_else(|_| panic!("BUILD_CONFIGURATION value")),
            );
        }
        self.evaluate_configuration(key.clone(), true, false, event_handler)
            .ok()
    }

    /// Asks the Skyframe evaluator to build a `BuildConfigurationValue`.
    // Java: com/google/devtools/build/lib/skyframe/SkyframeExecutor.java:1894 SkyframeExecutor.createConfiguration
    pub fn create_configuration(
        &self,
        event_handler: Arc<dyn ExtendedEventHandler>,
        build_options: Arc<BuildOptions>,
        keep_going: bool,
    ) -> Result<Arc<BuildConfigurationValue>, InvalidConfigurationException> {
        let top_level_target_config =
            self.get_configuration(event_handler.clone(), build_options, keep_going)?;
        // ErrorSensingEventHandler
        let sensing = ErrorSensing {
            delegate: event_handler.as_ref(),
            has_errors: std::sync::atomic::AtomicBool::new(false),
        };
        top_level_target_config.report_invalid_options(&sensing);
        if sensing.has_errors.load(std::sync::atomic::Ordering::SeqCst) {
            return Err(InvalidConfigurationException::with_code(
                "Build options are invalid",
                "INVALID_BUILD_OPTIONS",
            ));
        }
        Ok(top_level_target_config)
    }

    /// `setBaselineConfiguration(targetOptions, eventHandler)`.
    ///
    /// Injects the precomputed values, which invalidates the configurations computed from the
    /// previous baseline (the functions that read it depend on them), and sets the client's
    /// copies (read outside Skyframe).
    // Java: com/google/devtools/build/lib/skyframe/SkyframeExecutor.java:1520 SkyframeExecutor.setBaselineConfiguration
    pub fn set_baseline_configuration(
        &self,
        target_options: &BuildOptions,
        event_handler: Arc<dyn ExtendedEventHandler>,
    ) -> Result<(), InvalidConfigurationException> {
        let state = self.configuration_state();
        if *state.client.baseline_configuration.get() == *target_options {
            return Ok(());
        }
        let baseline = Arc::new(target_options.clone());
        // Java: com/google/devtools/build/lib/skyframe/SkyframeExecutor.java:1544 SkyframeExecutor.adjustForExec
        let baseline_exec = adjust_for_exec_with_evaluator(
            &self.evaluator,
            &baseline,
            &state.client.starlark_semantics,
            event_handler,
        )?;
        let baseline_exec = Arc::new(baseline_exec);
        blaze_build_config::skyframe::BASELINE_CONFIGURATION
            .set_on(&self.evaluator, baseline.clone());
        blaze_build_config::skyframe::BASELINE_EXEC_CONFIGURATION
            .set_on(&self.evaluator, baseline_exec.clone());
        state.client.baseline_configuration.set(baseline);
        state.client.baseline_exec_configuration.set(baseline_exec);
        Ok(())
    }

    /// The analysis-cache decision of `BuildView.update` for a server that keeps its graph:
    /// `SkyframeBuildView.shouldDiscardAnalysisCache` (Java's message) and the discard of
    /// `setConfiguration` (see [`crate::incremental::SkyframeIncrementalState::set_configuration`]).
    /// Nothing for an executor whose graph is its own.
    // Java: com/google/devtools/build/lib/analysis/BuildView.java:273 BuildView.update
    pub fn should_discard_analysis_cache(
        &self,
        target_options: &BuildOptions,
        event_handler: &dyn EventHandler,
    ) -> Result<(), InvalidConfigurationException> {
        let Some(handle) = &self.incremental else {
            return Ok(());
        };
        let analysis_options = self
            .options
            .get_options::<blaze_optdefs::command::analysis_options::AnalysisOptions>()
            .unwrap_or_else(
                Options::<blaze_optdefs::command::analysis_options::AnalysisOptions>::get_defaults,
            );
        handle
            .state()
            .set_configuration(
                &self.evaluator,
                event_handler,
                target_options,
                analysis_options.max_config_changes_to_show,
                analysis_options.allow_analysis_cache_discards,
            )
            .map_err(|message| {
                InvalidConfigurationException::with_code(
                    message,
                    "CONFIGURATION_DISCARDED_ANALYSIS_CACHE",
                )
            })
    }

    /// `SkyframeBuildView.getEvaluatedCounts` and the action counts: from the progress receiver
    /// when the graph may hold nodes of earlier commands, otherwise from a sweep over the graph.
    // Java: com/google/devtools/build/lib/skyframe/SkyframeBuildView.java:197 SkyframeBuildView.getEvaluatedCounts
    pub fn get_evaluated_counts(
        &self,
        time_in_ms: i64,
    ) -> crate::build_view::AnalysisPhaseCompleteStats {
        if self.incremental.is_some() {
            self.progress_receiver
                .evaluated_counts()
                .to_stats(time_in_ms)
        } else {
            crate::skyframe_build_view::get_evaluated_counts(&self.evaluator, time_in_ms)
        }
    }

    /// `SkyframeBuildView.resetProgressReceiver` (`BuildView.update`).
    pub fn reset_progress_receiver(&self) {
        self.progress_receiver.reset_evaluated_counts();
    }

    /// The keys of every configuration computed so far.
    // Java: com/google/devtools/build/lib/skyframe/SkyframeExecutor.java:2153 SkyframeExecutor.getTransitiveConfigurationKeys
    pub fn get_transitive_configuration_keys(&self) -> Vec<SkyKey> {
        self.evaluator
            .get_all_keys_for_testing()
            .into_iter()
            .filter(|key| key.function_name() == BuildConfigurationKey::FUNCTION)
            .filter(|key| self.evaluator.get_existing_value(key).is_some())
            .collect()
    }

    // -----------------------------------------------------------------------------------------
    // Analysis.

    /// Configures a given set of configured targets.
    // Java: com/google/devtools/build/lib/skyframe/SkyframeExecutor.java:2317 SkyframeExecutor.configureTargets
    pub fn configure_targets(
        &self,
        event_handler: Arc<dyn ExtendedEventHandler>,
        label_to_target_map: &IndexMap<Label, Target>,
        configured_target_keys: &[ConfiguredTargetKey],
        top_level_aspect_keys: &[TopLevelAspectsKey],
        keep_going: bool,
    ) -> Result<ConfigureTargetsResult, Interrupted> {
        // PORT: ConfigurationPhaseStartedEvent(configuredTargetProgress) hands the UI the
        // progress receiver, which is process-wide here
        // (blaze_sky_analysis::analysis_progress_receiver::ANALYSIS_PROGRESS).
        // Iterables.concat(configuredTargetKeys, topLevelAspectKeys)
        let keys: Vec<SkyKey> = configured_target_keys
            .iter()
            .cloned()
            .map(SkyKey::new)
            .chain(top_level_aspect_keys.iter().cloned().map(SkyKey::new))
            .collect();
        let result = self.evaluate(
            &keys,
            keep_going,
            DEFAULT_THREAD_COUNT,
            event_handler.clone(),
        )?;

        let mut targets_with_configuration: Vec<TargetAndConfiguration> = Vec::new();
        let mut configured_targets: Vec<ConfiguredTarget> = Vec::new();
        let graph = self.get_walkable_graph();
        for key in configured_target_keys {
            let Some(value) = result.get(key) else {
                continue;
            };
            let configured_target = value.get_configured_target().clone();
            // ImmutableSet.Builder: configured targets are deduplicated.
            if !configured_targets.iter().any(|ct| {
                ct.get_lookup_key()
                    .0
                    .key_equals(&*configured_target.get_lookup_key().0)
            }) {
                configured_targets.push(configured_target.clone());
            }

            let target = label_to_target_map
                .get(key.get_label())
                .cloned()
                .unwrap_or_else(|| panic!("NullPointerException: {}", key.get_label()));
            let configuration = get_configuration_from_graph(
                &*graph,
                configured_target.get_configuration_key().as_ref(),
            );
            // PORT: TargetConfiguredEvent is built by the command for the build event stream
            // from `targets_with_configuration` (blaze_runtime commands/bep.rs).
            targets_with_configuration.push(TargetAndConfiguration::new(target, configuration));
        }

        // ImmutableMap.Builder<AspectKey, ConfiguredAspect>
        let mut aspects: IndexMap<AspectKey, Arc<AspectValue>> = IndexMap::new();
        for key in top_level_aspect_keys {
            let Some(value) = result.get(key) else {
                continue; // Skip aspects that couldn't be applied to targets.
            };
            // The ConfiguredTargetKey in the AspectKey will vary from the TopLevelAspectKey's
            // ConfiguredTargetKey due to rule transitions. See the implementation in
            // ToplevelStarlarkAspectFunction#getConfiguredTargetKey().
            // Keep this logic in-sync with BuildDriverFunction#announceTopLevelAspectAnalyzed(),
            // which is the corresponding skymeld (merged analysis+execution) codepath.
            let Some(first_aspect_key) = value.get_top_level_aspects_map().keys().next() else {
                continue;
            };
            let transitioned_key = first_aspect_key.get_base_configured_target_key().clone();
            let aspect_count = value.get_top_level_aspects_map().len() as i32;
            event_handler.post(Arc::new(ToplevelAspectsIdentifiedEvent {
                configured_target_key: transitioned_key,
                aspect_count,
            }));
            for (aspect_key, aspect_value) in value.get_top_level_aspects_map() {
                // ImmutableMap.Builder.buildOrThrow: a key may appear only once.
                if aspects
                    .insert(aspect_key.clone(), aspect_value.clone())
                    .is_some()
                {
                    panic!(
                        "IllegalArgumentException: Multiple entries with same key: {:?}",
                        aspect_key
                    );
                }
                let configuration =
                    get_configuration_from_graph(&*graph, aspect_key.get_configuration_key());
                event_handler.post(Arc::new(AspectConfiguredEvent::new(
                    aspect_key.get_label().clone(),
                    /* aspect_class_name= */ aspect_key.get_aspect_class().get_name(),
                    /* aspect_description= */
                    &aspect_key.get_aspect_descriptor().get_description(),
                    configuration,
                )));
            }
        }

        Ok(ConfigureTargetsResult {
            evaluation_result: result,
            configured_targets,
            aspects,
            targets_with_configuration,
        })
    }
}

/// Returns the configuration of `key` in the graph.
// Java: com/google/devtools/build/lib/skyframe/SkyframeExecutor.java:2402 SkyframeExecutor.getConfigurationFromGraph
pub fn get_configuration_from_graph(
    graph: &dyn WalkableGraph,
    key: Option<&BuildConfigurationKey>,
) -> Option<Arc<BuildConfigurationValue>> {
    let key = key?;
    graph.get_value(&SkyKey::new(key.clone())).map(|v| {
        v.downcast::<BuildConfigurationValue>()
            .unwrap_or_else(|_| panic!("BUILD_CONFIGURATION value"))
    })
}

/// Returns the value of a configured target key in the graph.
pub fn get_configured_target_value(
    graph: &dyn WalkableGraph,
    key: &SkyKey,
) -> Option<Arc<ConfiguredTargetValue>> {
    graph.get_value(key).map(|v| {
        v.downcast::<ConfiguredTargetValue>()
            .unwrap_or_else(|_| panic!("CONFIGURED_TARGET value of {:?}", key))
    })
}

/// Encapsulation of the result of `filterActionConflictsForConfiguredTargetsAndAspects()`
/// allowing callers to determine which top-level keys did not have analysis errors and retrieve
/// the ActionConflictException for those that keys that specifically have conflicts.
// Java: com/google/devtools/build/lib/skyframe/SkyframeExecutor.java:2570 SkyframeExecutor.TopLevelActionConflictReport
pub struct TopLevelActionConflictReport {
    result: EvaluationResult,
    /// `topLevelArtifactContext.outputGroups()`.
    top_level_artifact_context: Vec<String>,
}

impl TopLevelActionConflictReport {
    fn key(&self, k: &blaze_actions::ActionLookupKeyRef) -> TopLevelActionLookupConflictFindingKey {
        TopLevelActionLookupConflictFindingKey::create(k.clone(), &self.top_level_artifact_context)
    }

    // Java: com/google/devtools/build/lib/skyframe/SkyframeExecutor.java:2582 SkyframeExecutor.TopLevelActionConflictReport.isErrorFree
    pub fn is_error_free(&self, k: &blaze_actions::ActionLookupKeyRef) -> bool {
        self.result.get(&self.key(k)).is_some()
    }

    /// Get the ActionConflictException produced for the given ActionLookupKey. Will throw if the
    /// given key is error-free.
    // Java: com/google/devtools/build/lib/skyframe/SkyframeExecutor.java:2592 SkyframeExecutor.TopLevelActionConflictReport.getConflictException
    pub fn get_conflict_exception(
        &self,
        k: &blaze_actions::ActionLookupKeyRef,
    ) -> Option<ActionConflictException> {
        let error_info = self
            .result
            .get_error(&SkyKey::new(self.key(k)))
            .unwrap_or_else(|| panic!("NullPointerException: no error for {:?}", k));
        // `e instanceof ActionConflictException`. A conflict inside one target reaches the
        // top-level key as the cause of ConfiguredTargetFunction's UnreportedException.
        error_info
            .get_exception()
            .and_then(|e| {
                e.downcast_ref::<ActionConflictException>().or_else(|| {
                    e.downcast_ref::<ConfiguredTargetFunctionException>()
                        .and_then(|e| e.as_action_conflict())
                })
            })
            .cloned()
    }
}

/// What target pattern evaluation throws.
#[derive(Debug, Clone, thiserror::Error)]
pub enum TargetPatternPhaseError {
    #[error("{}", .0.get_message())]
    TargetParsing(TargetParsingException),
    #[error("interrupted")]
    Interrupted,
}

/// What target pattern evaluation throws, with `TargetParsingException.getDetailedExitCode()`.
///
/// PORT: the ported `TargetParsingException` (blaze_cmdline) holds a `TargetPatterns` code only;
/// Java's holds the `DetailedExitCode` of its cause when it has one (a `PackageLoading` code for
/// a missing package, for example). The exit code travels next to the exception here.
#[derive(Debug, Clone, thiserror::Error)]
pub enum TargetPatternPhaseDetailedError {
    #[error("{}", .exception.get_message())]
    TargetParsing {
        exception: TargetParsingException,
        /// `exception.getDetailedExitCode()`.
        detailed_exit_code: blaze_query_engine::DetailedExitCode,
    },
    #[error("interrupted")]
    Interrupted,
}

impl From<TargetPatternPhaseDetailedError> for TargetPatternPhaseError {
    fn from(e: TargetPatternPhaseDetailedError) -> Self {
        match e {
            TargetPatternPhaseDetailedError::TargetParsing { exception, .. } => {
                TargetPatternPhaseError::TargetParsing(exception)
            }
            TargetPatternPhaseDetailedError::Interrupted => TargetPatternPhaseError::Interrupted,
        }
    }
}

/// `TargetParsingException.getDetailedExitCode()` of an exception created from a
/// `TargetPatterns.Code` (`DetailedExitCode.of(FailureDetail{message, TargetPatterns{code}})`).
// Java: com/google/devtools/build/lib/cmdline/TargetParsingException.java:76 TargetParsingException.getDetailedExitCode
pub fn target_parsing_detailed_exit_code(
    e: &TargetParsingException,
) -> blaze_query_engine::DetailedExitCode {
    // The exception's own DetailedExitCode: the cause's when it was built from one (a
    // NoSuchPackageException's PackageLoading code), else the one of its TargetPatterns code.
    blaze_query_engine::DetailedExitCode::of(
        e.get_detailed_exit_code()
            .get_failure_detail()
            .cloned()
            .expect("a TargetParsingException's DetailedExitCode has a FailureDetail"),
    )
}

/// `Toolchain.<CODE>` for the `FailureDetails.Toolchain.Code` name `code`, as the `code` of an
/// `InvalidConfigurationException` that wraps a `ToolchainException`.
fn toolchain_code_of_invalid_configuration(code: &'static str) -> &'static str {
    match code {
        "INVALID_PLATFORM_VALUE" => "Toolchain.INVALID_PLATFORM_VALUE",
        "INVALID_CONSTRAINT_VALUE" => "Toolchain.INVALID_CONSTRAINT_VALUE",
        "INVALID_TOOLCHAIN_TYPE" => "Toolchain.INVALID_TOOLCHAIN_TYPE",
        "INVALID_TOOLCHAIN" => "Toolchain.INVALID_TOOLCHAIN",
        "NO_MATCHING_EXECUTION_PLATFORM" => "Toolchain.NO_MATCHING_EXECUTION_PLATFORM",
        "NO_MATCHING_TOOLCHAIN" => "Toolchain.NO_MATCHING_TOOLCHAIN",
        "MISSING_PROVIDER" => "Toolchain.MISSING_PROVIDER",
        other => blaze_util::unported!(
            "SkyframeExecutor.getConfiguration",
            "ToolchainException with code {other}"
        ),
    }
}

/// `SkyframeExecutor.EXCEPTION_TRAVERSAL_LIMIT`.
// Java: com/google/devtools/build/lib/skyframe/SkyframeExecutor.java:389 SkyframeExecutor.EXCEPTION_TRAVERSAL_LIMIT
const EXCEPTION_TRAVERSAL_LIMIT: usize = 10;

// Java: com/google/devtools/build/lib/skyframe/SkyframeExecutor.java:3240 SkyframeExecutor.constructNoCycleTargetParsingException
fn construct_no_cycle_target_parsing_exception(
    event_handler: &dyn ExtendedEventHandler,
    target_patterns: &[String],
    error_info: &blaze_skyframe::ErrorInfo,
) -> TargetPatternPhaseDetailedError {
    let e = error_info
        .get_exception()
        .unwrap_or_else(|| panic!("NullPointerException"));
    let detailed_exit_code = traverse_exception_chain(e);
    let target_parsing_exception = e.downcast_ref::<TargetParsingException>();
    if target_parsing_exception.is_none() {
        // If it's a TargetParsingException, then the TargetPatternPhaseFunction has already
        // reported the error, so we don't need to report it again.
        event_handler.post(Arc::new(PatternExpandingError::failed(
            target_patterns.to_vec(),
            e.to_string(),
        )));
    }

    // Following SkyframeTargetPatternEvaluator, we create with a new TargetParsingException
    // either with an existing DetailedExitCode, or with a FailureDetail Code.
    let exception = match target_parsing_exception {
        Some(tpe) => tpe.clone(),
        None => TargetParsingException::new(
            e.to_string(),
            TargetPatternsCode::TargetPatternParseFailure,
        ),
    };
    let detailed_exit_code =
        detailed_exit_code.unwrap_or_else(|| target_parsing_detailed_exit_code(&exception));
    TargetPatternPhaseDetailedError::TargetParsing {
        exception,
        detailed_exit_code,
    }
}

/// `DetailedException.getDetailedExitCode(exception)` of the first exception of the cause chain
/// that has one.
///
/// PORT: `DetailedException` is a Java interface; the exception classes recognised are those of
/// `skyframe_error_processor::detailed_exception_failure_detail` and `TargetParsingException`.
/// The ported `TargetParsingException` does not hold its cause's exit code (see
/// `TargetPatternPhaseDetailedError`): its own `TargetPatterns` code is returned.
// Java: com/google/devtools/build/lib/skyframe/SkyframeExecutor.java:3260 SkyframeExecutor.traverseExceptionChain
fn traverse_exception_chain(
    top_level_exception: &blaze_skyframe::SkyError,
) -> Option<blaze_query_engine::DetailedExitCode> {
    if let Some(tpe) = top_level_exception.downcast_ref::<TargetParsingException>() {
        return Some(target_parsing_detailed_exit_code(tpe));
    }
    if let Some(failure_detail) =
        crate::skyframe_error_processor::detailed_exception_failure_detail(top_level_exception)
    {
        return Some(blaze_query_engine::DetailedExitCode::of(failure_detail));
    }
    let mut traverse_exception: &(dyn std::error::Error + 'static) = &**top_level_exception;
    let mut traverse_level = 1;
    while traverse_level < EXCEPTION_TRAVERSAL_LIMIT {
        traverse_level += 1;
        let Some(cause) = traverse_exception.source() else {
            break;
        };
        traverse_exception = cause;
        if let Some(tpe) = traverse_exception.downcast_ref::<TargetParsingException>() {
            return Some(target_parsing_detailed_exit_code(tpe));
        }
        if let Some(failure_detail) =
            crate::skyframe_error_processor::detailed_exception_failure_detail_of(
                traverse_exception,
            )
        {
            return Some(blaze_query_engine::DetailedExitCode::of(failure_detail));
        }
    }
    None
}

/// Signals that the top-level aspects of a target were identified: the (transitioned) key of the
/// target and the number of aspects applied to it.
// Java: com/google/devtools/build/lib/skyframe/ToplevelAspectsIdentifiedEvent.java:23 ToplevelAspectsIdentifiedEvent
#[derive(Clone, Debug)]
pub struct ToplevelAspectsIdentifiedEvent {
    /// `configuredTargetKey()`.
    pub configured_target_key: ConfiguredTargetKey,
    /// `aspectCount()`.
    pub aspect_count: i32,
}

impl blaze_events::Postable for ToplevelAspectsIdentifiedEvent {}

/// Event when an aspect is configured.
///
/// PORT: the build event stream conversion (`getEventId`, `asStreamProto`, `getConfigurations`)
/// belongs to the command's BEP writer, which reads the fields.
// Java: com/google/devtools/build/lib/analysis/AspectConfiguredEvent.java:32 AspectConfiguredEvent
#[derive(Clone, Debug)]
pub struct AspectConfiguredEvent {
    target: Label,
    aspect_class_name: String,
    aspect_description: String,
    configuration: Option<Arc<BuildConfigurationValue>>,
}

impl AspectConfiguredEvent {
    // Java: com/google/devtools/build/lib/analysis/AspectConfiguredEvent.java:38 AspectConfiguredEvent.<init>
    pub fn new(
        target: Label,
        aspect_class_name: &str,
        aspect_description: &str,
        configuration: Option<Arc<BuildConfigurationValue>>,
    ) -> Self {
        AspectConfiguredEvent {
            target,
            aspect_class_name: aspect_class_name.to_owned(),
            aspect_description: aspect_description.to_owned(),
            configuration,
        }
    }

    // Java: com/google/devtools/build/lib/analysis/AspectConfiguredEvent.java:66 AspectConfiguredEvent.getAspectClassName
    pub fn get_aspect_class_name(&self) -> &str {
        &self.aspect_class_name
    }

    // Java: com/google/devtools/build/lib/analysis/AspectConfiguredEvent.java:70 AspectConfiguredEvent.getTarget
    pub fn get_target(&self) -> &Label {
        &self.target
    }

    /// The aspect's description (`AspectDescriptor.getDescription()`; the BEP id's `aspect`).
    pub fn get_aspect_description(&self) -> &str {
        &self.aspect_description
    }

    /// The configuration of the aspect, `None` for targets without one.
    pub fn get_configuration(&self) -> Option<&Arc<BuildConfigurationValue>> {
        self.configuration.as_ref()
    }
}

impl blaze_events::Postable for AspectConfiguredEvent {}

/// A target and the configuration of its configured target.
// Java: com/google/devtools/build/lib/analysis/TargetAndConfiguration.java:30 TargetAndConfiguration
#[derive(Clone, Debug)]
pub struct TargetAndConfiguration {
    target: Target,
    configuration: Option<Arc<BuildConfigurationValue>>,
}

impl TargetAndConfiguration {
    pub fn new(target: Target, configuration: Option<Arc<BuildConfigurationValue>>) -> Self {
        TargetAndConfiguration {
            target,
            configuration,
        }
    }

    // Java: com/google/devtools/build/lib/analysis/TargetAndConfiguration.java:58 TargetAndConfiguration.getLabel
    pub fn get_label(&self) -> &Label {
        self.target.get_label()
    }

    // Java: com/google/devtools/build/lib/analysis/TargetAndConfiguration.java:62 TargetAndConfiguration.getTarget
    pub fn get_target(&self) -> &Target {
        &self.target
    }

    // Java: com/google/devtools/build/lib/analysis/TargetAndConfiguration.java:67 TargetAndConfiguration.getConfiguration
    pub fn get_configuration(&self) -> Option<&Arc<BuildConfigurationValue>> {
        self.configuration.as_ref()
    }
}

/// Result of a call to `configureTargets`.
// Java: com/google/devtools/build/lib/skyframe/SkyframeExecutor.java:2408 SkyframeExecutor.ConfigureTargetsResult
pub struct ConfigureTargetsResult {
    pub evaluation_result: EvaluationResult,
    pub configured_targets: Vec<ConfiguredTarget>,
    /// `aspects()`: the top-level aspects, in the order of their `TopLevelAspectsKey`s.
    pub aspects: IndexMap<AspectKey, Arc<AspectValue>>,
    pub targets_with_configuration: Vec<TargetAndConfiguration>,
    // PORT: package roots (collectPackageRoots) only matter to execution (the symlink forest);
    // see `SkyframeExecutor::get_package_roots`.
}

struct ErrorSensing<'a> {
    delegate: &'a dyn ExtendedEventHandler,
    has_errors: std::sync::atomic::AtomicBool,
}

impl EventHandler for ErrorSensing<'_> {
    // Java: com/google/devtools/build/lib/events/ErrorSensingEventHandler.java:44 ErrorSensingEventHandler.handle
    fn handle(&self, event: &Event) {
        if event.get_kind() == blaze_events::EventKind::Error {
            self.has_errors
                .store(true, std::sync::atomic::Ordering::SeqCst);
        }
        self.delegate.handle(event);
    }
}

/// `EvaluationContext` for an analysis evaluation.
pub fn analysis_evaluation_context(
    keep_going: bool,
    event_handler: Arc<dyn ExtendedEventHandler>,
) -> EvaluationContext {
    EvaluationContext {
        keep_going,
        event_handler,
        ..EvaluationContext::default()
    }
}

/// `NetUtil.getCachedShortHostName()`: `InetAddress.getLocalHost().getHostName()` (the whole
/// name, not cut at a dot), an internal string (`platformToInternal`), or "unknown".
// Java: com/google/devtools/build/lib/util/NetUtil.java:39 NetUtil.getCachedShortHostName
pub fn short_host_name() -> String {
    blaze_exec::exec::local::local_spawn_runner::get_cached_short_host_name()
}

/// `env.getValue(RepositoryMappingValue.key(RepositoryName.MAIN))` for PlatformMappingFunction
/// (blaze_build_config cannot name the key).
struct MainRepositoryMappingFromSkyframe;

impl blaze_build_config::skyframe::MainRepositoryMappingLookup
    for MainRepositoryMappingFromSkyframe
{
    fn main_repository_mapping(
        &self,
        env: &mut blaze_skyframe::Environment<'_>,
    ) -> Result<Option<blaze_cmdline::RepositoryMapping>, blaze_skyframe::ComputeError> {
        let key = blaze_sky_loading::repos::RepositoryMappingKey::key(
            blaze_cmdline::repository_name::MAIN.clone(),
        );
        Ok(env.get_value(key)?.map(|value| {
            value
                .repository_mapping()
                .expect("NullPointerException: repositoryMapping()")
                .clone()
        }))
    }
}

/// `env.getValue(FileValue.key(rootedPath))` for the configuration functions (see
/// `blaze_build_config::skyframe::FileValueLookup`).
struct FileValueFromSkyframe;

impl blaze_build_config::skyframe::FileValueLookup for FileValueFromSkyframe {
    fn file_value(
        &self,
        env: &mut blaze_skyframe::Environment<'_>,
        rooted_path: &blaze_vfs::RootedPath,
    ) -> Result<Option<blaze_build_config::skyframe::FileValueFacts>, blaze_skyframe::ComputeError>
    {
        let key = blaze_sky_loading::file::FileKey::key(rooted_path.clone());
        Ok(env
            .get_value(key)?
            .map(|file_value| blaze_build_config::skyframe::FileValueFacts {
                exists: file_value.exists(),
                is_directory: file_value.is_directory(),
                real_path: file_value.real_rooted_path(rooted_path).as_path(),
            }))
    }
}

/// Whether `--experimental_exec_config` names a .bzl outside `@_builtins` (loaded through
/// BZL_LOAD rather than the bundled loader).
fn exec_config_needs_bzl_load(options: &BuildOptions) -> bool {
    let Some(core) = options.get::<blaze_optdefs::fragment::core_options::CoreOptions>() else {
        return false;
    };
    // TransitionReference.create: `<label>%<symbol>`; the label's repository decides the key.
    let Some((bzl, _)) = core.starlark_exec_config.split_once('%') else {
        return false;
    };
    match blaze_cmdline::Label::parse_canonical(bzl) {
        Ok(label) => label.get_repository() != &*blaze_cmdline::repository_name::BUILTINS,
        // A malformed reference fails in StarlarkExecTransitionLoader with its own message.
        Err(_) => false,
    }
}

/// `SkyframeExecutor.adjustForExec` with the exec transition's .bzl loaded through the evaluator
/// (`getStarlarkExecTransition`'s BzlFileLoader).
// Java: com/google/devtools/build/lib/skyframe/SkyframeExecutor.java:2168 SkyframeExecutor.getStarlarkExecTransition
fn adjust_for_exec_with_evaluator(
    evaluator: &Arc<blaze_skyframe::Evaluator>,
    build_options: &BuildOptions,
    semantics: &Arc<StarlarkSemantics>,
    event_handler: Arc<dyn ExtendedEventHandler>,
) -> Result<BuildOptions, InvalidConfigurationException> {
    struct EvaluatorLoader<'a> {
        evaluator: &'a Arc<blaze_skyframe::Evaluator>,
        semantics: &'a Arc<StarlarkSemantics>,
        event_handler: Arc<dyn ExtendedEventHandler>,
    }
    impl blaze_build_config::starlark_transition::BzlFileLoader for EvaluatorLoader<'_> {
        fn convert_transition(
            &self,
            value: &starlark::values::OwnedFrozenValue,
        ) -> Option<Arc<blaze_build_config::starlark_transition::StarlarkDefinedConfigTransition>>
        {
            let frozen = value.value().unpack_frozen()?;
            blaze_config::transitions::StarlarkDefinedConfigTransition::from_value(
                frozen.to_value(),
            )?;
            // The module's heap stays alive with the BZL_LOAD value (PORT: keep-alive), as
            // StarlarkCallable requires.
            Some(
                blaze_analysis::config::transitions::starlark_defined_config_transition_of(
                    &blaze_packages::rule_class::StarlarkCallable::new(frozen),
                ),
            )
        }
        fn get_value(
            &self,
            bzl_file: &blaze_cmdline::Label,
            for_builtins: bool,
        ) -> Result<Option<starlark::environment::FrozenModule>, String> {
            if for_builtins {
                return blaze_build_config::starlark_transition::load_builtins_bzl(
                    bzl_file,
                    self.semantics,
                )
                .map(Some);
            }
            let key = blaze_sky_loading::bzl_load::BzlLoadKey::key_for_build(bzl_file.clone());
            let ctx = blaze_skyframe::EvaluationContext {
                keep_going: false,
                event_handler: self.event_handler.clone(),
                ..blaze_skyframe::EvaluationContext::default()
            };
            let result = self
                .evaluator
                .evaluate(&[SkyKey::new(key.clone())], &ctx)
                .unwrap_or_else(|_| {
                    blaze_util::unported!(
                        "SkyframeExecutor.getStarlarkExecTransition",
                        "interrupted while loading {}",
                        bzl_file
                    )
                });
            if let Some(value) = result.get(&key) {
                return Ok(Some(value.get_module().clone()));
            }
            let (_, error) = &result.error_map()[0];
            match error.get_exception() {
                // If it's a BzlLoadFailedException, rethrow it directly; otherwise, wrap it.
                Some(e) => Err(e.to_string()),
                None => blaze_util::unported!(
                    "SkyframeExecutor.getStarlarkExecTransition",
                    "cycle while loading {} (CyclesReporter)",
                    bzl_file
                ),
            }
        }
    }
    let loader = EvaluatorLoader {
        evaluator,
        semantics,
        event_handler: event_handler.clone(),
    };
    let exec_transition =
        blaze_build_config::starlark_transition::load_starlark_exec_transition_with_loader(
            Some(build_options),
            &loader,
        )
        .map_err(|e| InvalidConfigurationException::new(e.0))?
        .expect("evaluated, not missing");
    let host_platform = build_options
        .get::<blaze_optdefs::fragment::platform_options::PlatformOptions>()
        .map(|p| p.host_platform.0.clone());
    match exec_transition {
        Some(t) => Ok(
            blaze_build_config::function_transition_util::adjust_for_exec(
                build_options,
                &t,
                host_platform.as_ref(),
                semantics,
                event_handler.as_ref(),
            ),
        ),
        None => blaze_util::unported!(
            "ExecutionTransitionFactory.create",
            "no Starlark exec transition (native exec transition is not in Bazel 9)"
        ),
    }
}
