//! The Skyframe state that outlives a command on a persistent server: the graph kept from one
//! command to the next, and the per-command `sync` that tells it what changed.
//!
//! Java keeps one `SequencedSkyframeExecutor` per workspace for the server's lifetime; its
//! functions read the command's inputs through precomputed values and cells, and
//! `SequencedSkyframeExecutor.sync` / `SkyframeExecutor.handleDiffs` invalidate what changed.
//!
//! PORT: Blaze builds a [`crate::skyframe_executor::SkyframeExecutor`] (its functions and the
//! services they capture) for every command, and more than one in a command (the dispatcher's,
//! then the command's). The graph is what is kept: when an executor is dropped its evaluator goes
//! back to the server's [`SkyframeIncrementalState`], and the next executor builds its evaluator
//! over that graph (`EvaluatorBuilder::build_reusing`). Before it does, this module ports what
//! Java does at the start of a command, and adds one Blaze-only rule, fail safe: an input that a
//! Rust function captures where Java reads a dependency, and whose change this module cannot
//! invalidate precisely, discards the graph (see [`LoadingSignature`]).
//!
//! The mode is read once per server from the server's environment variable `BLAZE_INCREMENTAL`
//! (decision D6 of docs/research/08-server-and-incrementality.md): `none` keeps nothing (each
//! command starts from an empty graph, the behaviour of decision D2); anything else, or unset,
//! keeps the graph. A process that runs one command (batch, in-process mode) never reuses it.

use std::collections::HashSet;
use std::sync::Arc;

use blaze_build_config::build_configuration_key::BuildConfigurationKey;
use blaze_build_config::build_options::BuildOptions;
use blaze_cmdline::{PackageIdentifier, RepositoryMapping};
use blaze_events::{Event, EventHandler, ExtendedEventHandler};
use blaze_optdefs::command::local_diff_awareness::LocalDiffAwarenessOptions;
use blaze_optdefs::command::package_options::PackageOptions;
use blaze_optdefs::misc::repository_options::RepositoryOptions;
use blaze_options::{OptionsClass, OptionsParser, Value};
use blaze_sky_loading::diff_awareness::{
    DiffAwarenessFactory, DiffAwarenessManager, ProcessableModifiedFileSet,
};
use blaze_sky_loading::directories::LoadingDirectories;
use blaze_sky_loading::directory_listing::DirectoryListingStateKey;
use blaze_sky_loading::file_state::FileStateKey;
use blaze_sky_loading::file_state::{
    ExternalFileAction, ExternalFilesHelper, ExternalFilesKnowledgeState, FileType,
};
use blaze_sky_loading::filesystem_value_checker::FileDirtinessChecker;
use blaze_sky_loading::filesystem_value_checker::{
    ExternalDirtinessChecker, FilesystemValueChecker, MissingDiffDirtinessChecker,
    SkyValueDirtinessChecker, UnionDirtinessChecker,
};
use blaze_sky_loading::filesystem_value_checker_inferring_ancestors::{
    InferredDiff, StatFailedException, get_diff_with_inferred_ancestors,
};
use blaze_sky_loading::ignored_subdirectories::{
    IgnoredSubdirectories, IgnoredSubdirectoriesKey, IgnoredSubdirectoriesValue,
};
use blaze_sky_loading::package_lookup::PackageLookupKey;
use blaze_sky_loading::pkgcache::PathPackageLocator;
use blaze_skyframe::{
    EvaluationContext, Evaluator, EvaluatorBuilder, EventFilter, Key, SkyKey, SkyValue,
};
use blaze_starlark::StarlarkSemantics;
use blaze_util::io::timestamp_granularity_monitor::TimestampGranularityMonitor;
use blaze_vfs::{ModifiedFileSet, Path, Root, RootedPath};
use indexmap::IndexSet;
use parking_lot::{Mutex, RwLock};

/// Whether the server keeps its graph between commands (`BLAZE_INCREMENTAL`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeepGraphMode {
    /// `BLAZE_INCREMENTAL=none`: every command starts from an empty graph (decision D2).
    Discard,
    /// The graph is kept and invalidated as Java's `SequencedSkyframeExecutor` does.
    Keep,
}

impl KeepGraphMode {
    /// The environment variable that selects the mode.
    pub const ENV: &'static str = "BLAZE_INCREMENTAL";

    /// The mode of this process's environment (read by the server once, at start).
    pub fn from_process_environment() -> KeepGraphMode {
        match std::env::var(Self::ENV) {
            Ok(v) if v == "none" => KeepGraphMode::Discard,
            _ => KeepGraphMode::Keep,
        }
    }
}

/// The inputs of a command that Java turns into dependencies (precomputed values, cells it
/// invalidates) and that Blaze's functions capture where this module has no precise
/// invalidation. A change discards the whole graph.
///
/// PORT: Blaze-only (see the module documentation). In it:
/// - the Starlark semantics (Java: `PrecomputedValue.STARLARK_SEMANTICS`, which every loading
///   and analysis function reads; the Rust services also capture a copy);
/// - the `RepositoryOptions` that Java turns into precomputed values but Blaze does not
///   ([`REPOSITORY_OPTIONS_IN_SIGNATURE`]).
///
/// The other repository options reach the graph as precomputed values that every executor
/// injects (`BazelRepositoryModule.getPrecomputedValues`: `REGISTRIES`, `MODULE_OVERRIDES`,
/// `REPOSITORY_OVERRIDES` and the rest, with override paths already resolved against the working
/// directory), so a change invalidates their dependents as in Java. The package path is handled
/// as Java does (`SkyframeExecutor.setPackageLocator`, `onPkgLocatorChange`, in `sync`).
/// Cache locations, download sources, retries and timeouts are not in it: Java does not
/// invalidate on them either (module state of `BazelRepositoryModule`).
/// The `RepositoryOptions` of [`LoadingSignature`]: those Java turns into precomputed values
/// (`BazelRepositoryModule.getPrecomputedValues`) and Blaze reads without one (the extension
/// evaluation captures it).
const REPOSITORY_OPTIONS_IN_SIGNATURE: [&str; 1] = ["incompatible_require_repo_extension_metadata"];

#[derive(Clone, PartialEq)]
pub struct LoadingSignature {
    starlark_semantics: StarlarkSemantics,
    repository_options: Vec<(String, Option<Value>)>,
}

impl LoadingSignature {
    /// The signature of an executor's inputs.
    pub fn new(
        options: &OptionsParser,
        starlark_semantics: &StarlarkSemantics,
        _working_directory: &Path,
        _pkg_locator: &PathPackageLocator,
    ) -> LoadingSignature {
        let repository_options: Vec<(String, Option<Value>)> = options
            .get_options_dyn(RepositoryOptions::class())
            .map(|o| {
                o.as_map()
                    .into_iter()
                    .filter(|(name, _)| REPOSITORY_OPTIONS_IN_SIGNATURE.contains(&name.as_str()))
                    .collect()
            })
            .unwrap_or_default();
        LoadingSignature {
            starlark_semantics: starlark_semantics.clone(),
            repository_options,
        }
    }
}

/// Whether the remote fail safe of `build_evaluator` is on.
///
/// PORT: Blaze-only. In a debug build (`debug_assertions`), `BLAZE_INCREMENTAL_REMOTE_FAILSAFE=off`
/// in the server's environment turns it off, for the probes that compare the remote invalidations
/// with the oracle (scripts/incremental/remote-seq); a release build ignores the variable (review
/// round 2, S2-3).
fn remote_fail_safe_enabled() -> bool {
    static ON: std::sync::LazyLock<bool> = std::sync::LazyLock::new(|| {
        !(cfg!(debug_assertions)
            && std::env::var("BLAZE_INCREMENTAL_REMOTE_FAILSAFE").as_deref() == Ok("off"))
    });
    *ON
}

/// Whether the command has a remote cache, a disk cache or remote execution configured
/// (`RemoteOptions.isRemoteCacheEnabled`, which includes the disk cache and remote execution).
// Java: com/google/devtools/build/lib/remote/options/RemoteOptions.java:856 RemoteOptions.isRemoteCacheEnabled
fn remote_configured(options: &OptionsParser) -> bool {
    options
        .get_options::<blaze_optdefs::misc::remote_options::RemoteOptions>()
        .is_some_and(|o| o.is_remote_cache_enabled())
}

/// Why a graph is not reused, on the server's stderr when `BLAZE_INCREMENTAL_DEBUG` is set in the
/// server's environment (Blaze-only diagnostics).
fn debug(message: &str) {
    if debug_enabled() {
        eprintln!("blaze incremental: {message}");
    }
}

fn debug_enabled() -> bool {
    static ON: std::sync::LazyLock<bool> =
        std::sync::LazyLock::new(|| std::env::var_os("BLAZE_INCREMENTAL_DEBUG").is_some());
    *ON
}

/// `SkyframeExecutor.DEFAULT_EVENT_FILTER_WITH_ACTIONS`: store events, and do not propagate
/// events from analysis phase nodes to execution phase nodes.
///
/// Use the fact that analysis of a target must occur before execution of that target, and in a
/// separate Skyframe evaluation, to avoid propagating events from configured target nodes (and
/// more generally action lookup nodes) to action execution nodes. We take advantage of the fact
/// that if a node depends on an action lookup node and is not itself an action lookup node, then
/// it is an execution-phase node: the action lookup nodes are terminal in the analysis phase.
// Java: com/google/devtools/build/lib/skyframe/SkyframeExecutor.java:1150 SkyframeExecutor.DEFAULT_EVENT_FILTER_WITH_ACTIONS
pub struct DefaultEventFilterWithActions;

impl EventFilter for DefaultEventFilterWithActions {
    fn store_events(&self) -> bool {
        true
    }

    fn should_propagate(&self, dep_key: &SkyKey, primary_key: &SkyKey) -> bool {
        // Do not propagate events from analysis phase nodes to execution phase nodes.
        is_analysis_phase_action_lookup_key(primary_key)
            || !is_analysis_phase_action_lookup_key(dep_key)
            // Skymeld only.
            || primary_key.function_name()
                == <blaze_sky_exec::keys::BuildDriverKey as Key>::FUNCTION
    }
}

// Java: com/google/devtools/build/lib/skyframe/SkyframeExecutor.java:1168 SkyframeExecutor.isAnalysisPhaseActionLookupKey
fn is_analysis_phase_action_lookup_key(key: &SkyKey) -> bool {
    // key instanceof ActionLookupKey && !(key instanceof ActionTemplateExpansionKey)
    let name = key.function_name();
    name == <blaze_analysis::configured_target_key::ConfiguredTargetKey as Key>::FUNCTION
        || name == <blaze_analysis::aspect_key_creator::AspectKey as Key>::FUNCTION
        || name == <blaze_analysis::aspect_key_creator::TopLevelAspectsKey as Key>::FUNCTION
        || name == <blaze_analysis::analysis_services::WorkspaceStatusKey as Key>::FUNCTION
        || name == <blaze_analysis::coverage_report_value::CoverageReportKey as Key>::FUNCTION
}

/// What one executor of a command hands this module (Java: the arguments of
/// `decideKeepIncrementalState` and `sync`).
#[derive(Clone)]
pub struct IncrementalCommand {
    pub state: Arc<SkyframeIncrementalState>,
    /// The command's id (`BUILD_ID`); executors of one command share it. The user can set it
    /// (`--invocation_id`, `BAZEL_INTERNAL_INVOCATION_ID`), so it never keys the sync.
    pub command_id: String,
    /// A token the server allocates for each command (one `CommandEnvironment`): the per-command
    /// sync runs once per command call, as Java's `SequencedSkyframeExecutor.sync` does, keyed on
    /// this, never on the user's id (review round 2, S1-A).
    pub command_token: u64,
    /// The command's `TimestampGranularityMonitor`.
    pub tsgm: Arc<TimestampGranularityMonitor>,
    /// `--batch` (startup).
    pub batch: bool,
    /// `--track_incremental_state`.
    pub track_incremental_state: bool,
    /// `--discard_analysis_cache`.
    pub discard_analysis_cache: bool,
}

/// What the executor computed that the sync reads.
pub struct SyncInputs<'a> {
    pub signature: LoadingSignature,
    pub pkg_locator: Arc<RwLock<PathPackageLocator>>,
    pub deleted_packages: HashSet<PackageIdentifier>,
    pub directories: LoadingDirectories,
    pub repo_contents_cache_path: Option<Path>,
    pub options: &'a OptionsParser,
    pub client_env: &'a indexmap::IndexMap<String, String>,
    pub repo_env: &'a indexmap::IndexMap<String, String>,
    pub reporter: Arc<dyn ExtendedEventHandler>,
    /// The executor executes actions (`detectModifiedOutputFiles` applies).
    pub executes: bool,
    /// The output service's `OutputChecker` (Java `outputService.getOutputChecker()`).
    pub output_checker: Option<Arc<dyn blaze_exec::actions::action_cache_checker::OutputChecker>>,
    /// The executor analyzes, with these top-level options.
    pub top_level_build_options: Option<BuildOptions>,
}

/// The graph a server keeps between executors.
struct KeptGraph {
    evaluator: Arc<Evaluator>,
    _lease: blaze_util::build_arena::Lease,
    signature: LoadingSignature,
    /// The command that gave the graph back (its token, see `IncrementalCommand`).
    command_token: u64,
    /// Whether that command had a remote cache, a disk cache or remote execution (see
    /// [`remote_configured`]).
    remote: bool,
}

/// The server's state (Java: the fields of `SequencedSkyframeExecutor` and `SkyframeExecutor`
/// that outlive a command, and of `SkyframeBuildView` for the analysis cache).
pub struct SkyframeIncrementalState {
    mode: KeepGraphMode,
    /// Java `externalFilesHelper`'s knowledge (one helper per server in Java).
    external_files_knowledge: Arc<ExternalFilesKnowledgeState>,
    inner: Mutex<Inner>,
    /// Java `diffAwarenessManager` and `ignoredPaths` (`--watchfs`).
    diff_awareness: Mutex<DiffAwarenessSync>,
}

/// The diff awareness of the server's package path entries.
struct DiffAwarenessSync {
    manager: DiffAwarenessManager,
    /// `SkyframeExecutor.ignoredPaths`: the ignored subdirectories of the workspace root as the
    /// last `handleDiffs` computed them (`IgnoredSubdirectories.EMPTY` at first).
    ignored_paths: IgnoredSubdirectories,
}

struct Inner {
    // SequencedSkyframeExecutor.
    /// `trackIncrementalState`.
    track_incremental_state: bool,
    kept: Option<KeptGraph>,
    /// The command the per-command sync ran for.
    synced_command: Option<u64>,
    /// The command whose outputs were checked (`detectModifiedOutputFiles`).
    outputs_checked_command: Option<u64>,
    /// `deletedPackages`.
    deleted_packages: HashSet<PackageIdentifier>,
    /// `previousClientEnvironment`.
    previous_client_environment: IndexSet<String>,
    /// `previousRepositoryEnvironment`.
    previous_repository_environment: IndexSet<String>,
    // SkyframeExecutor.
    /// `lastAnalysisDiscarded`.
    last_analysis_discarded: bool,
    // SkyframeBuildView.
    /// `configuration != null`: an earlier command set a top-level configuration.
    has_configuration: bool,
    /// `originalConfigurationOptions`.
    original_configuration_options: Option<BuildOptions>,
    /// `skyframeAnalysisWasDiscarded`.
    skyframe_analysis_was_discarded: bool,
    /// The top-level options of the last executor that analyzed (Blaze-only: the analysis
    /// functions capture them besides reading `BASELINE_CONFIGURATION`).
    last_top_level_build_options: Option<BuildOptions>,
    /// The main repository mapping the last command computed (Blaze-only: the configured target
    /// and aspect functions read it from a cell, Java from `RepositoryMappingValue`).
    main_repo_mapping: Option<RepositoryMapping>,
    /// Blaze-only: the generation of the graph last handed to an executor. A reset or a newer
    /// hand-out makes the graphs of earlier generations stale: they are dropped when given back,
    /// so a graph is never kept with the state of another one.
    generation: u64,
    /// The command `decideKeepIncrementalState` ran for.
    decided_command: Option<u64>,
    /// `SkyframeExecutor.lastExecutionSalt`.
    last_execution_salt: Option<String>,
    /// Blaze-only: the evaluator of the current generation while an executor holds it (Java's
    /// server has one evaluator; a `delete` during a command reaches the graph the command
    /// evaluates on, review round 2, S2-5).
    handed_out: Option<std::sync::Weak<Evaluator>>,
    /// `SkyframeExecutor.pkgLocator`: the package locator of the last sync.
    last_pkg_locator: Option<PathPackageLocator>,
}

impl ExecutorHandle {
    /// Sets the execution salt and deletes all action execution nodes if it has changed since
    /// the last time it was set.
    ///
    /// PORT: `lastExecutionSalt` is the server's (it describes the kept graph): it is forgotten
    /// with the graph (`resetEvaluator` and the Blaze-only discards), which then has no action
    /// execution node of an earlier salt.
    // Java: com/google/devtools/build/lib/skyframe/SkyframeExecutor.java:2066 SkyframeExecutor.setSaltAndDeleteActionsIfChanged
    pub fn set_salt_and_delete_actions_if_changed(
        &self,
        evaluator: &Evaluator,
        execution_salt: &str,
    ) {
        let _c = blaze_util::profiler::Profiler::instance().profile(
            blaze_util::profiler::ProfilerTask::INFO,
            "setSaltAndDeleteActionsIfChanged",
        );
        let mut inner = self.command.state.inner.lock();
        if let Some(last) = &inner.last_execution_salt
            && last != execution_salt
        {
            debug("delete: execution salt changed");
            evaluator.delete(|k, _| {
                k.function_name() == <blaze_sky_exec::keys::ActionExecutionKey as Key>::FUNCTION
            });
        }
        inner.last_execution_salt = Some(execution_salt.to_string());
    }
}

/// An executor's hold on the kept graph: given back when the executor is dropped.
pub struct ExecutorHandle {
    command: IncrementalCommand,
    signature: LoadingSignature,
    analyzed: bool,
    /// The generation of the graph this executor was given; `put_back` keeps the graph only
    /// while it is the current one.
    generation: u64,
    /// The command has a remote cache, a disk cache or remote execution.
    remote: bool,
}

impl SkyframeIncrementalState {
    /// `diff_awareness_factories`: the workspace's (Java `WorkspaceBuilder.addDiffAwarenessFactory`,
    /// `BazelDiffAwarenessModule.workspaceInit`).
    pub fn new(
        mode: KeepGraphMode,
        diff_awareness_factories: Vec<Arc<dyn DiffAwarenessFactory>>,
    ) -> SkyframeIncrementalState {
        SkyframeIncrementalState {
            mode,
            external_files_knowledge: Arc::default(),
            diff_awareness: Mutex::new(DiffAwarenessSync {
                manager: DiffAwarenessManager::new(diff_awareness_factories),
                ignored_paths: IgnoredSubdirectories::empty(),
            }),
            inner: Mutex::new(Inner {
                track_incremental_state: true,
                kept: None,
                synced_command: None,
                outputs_checked_command: None,
                deleted_packages: HashSet::new(),
                previous_client_environment: IndexSet::new(),
                previous_repository_environment: IndexSet::new(),
                last_analysis_discarded: false,
                has_configuration: false,
                original_configuration_options: None,
                skyframe_analysis_was_discarded: false,
                last_top_level_build_options: None,
                main_repo_mapping: None,
                generation: 0,
                decided_command: None,
                last_execution_salt: None,
                handed_out: None,
                last_pkg_locator: None,
            }),
        }
    }

    pub fn mode(&self) -> KeepGraphMode {
        self.mode
    }

    /// The knowledge the loading functions' `ExternalFilesHelper` notes into.
    pub fn external_files_knowledge(&self) -> Arc<ExternalFilesKnowledgeState> {
        self.external_files_knowledge.clone()
    }

    /// Drops the functions (and the services and execution state they hold) of the executor that
    /// gave the kept graph back, keeping only its graph. Called when an executor starts being
    /// created, before it builds its own services (its repository setup, its action cache):
    /// two generations must not be alive at once.
    pub fn strip_kept_graph(&self) {
        let mut inner = self.inner.lock();
        let Some(kept) = inner.kept.take() else {
            return;
        };
        let keep_edges = true;
        match Arc::try_unwrap(kept.evaluator) {
            Ok(evaluator) => {
                let stripped = EvaluatorBuilder::new()
                    .keep_edges(keep_edges)
                    .build_reusing(evaluator);
                inner.kept = Some(KeptGraph {
                    evaluator: Arc::new(stripped),
                    _lease: kept._lease,
                    signature: kept.signature,
                    command_token: kept.command_token,
                    remote: kept.remote,
                });
            }
            // Something still holds the last executor's evaluator: it cannot be reused (fail
            // safe: a new graph).
            Err(_) => debug("kept evaluator still referenced when stripping: graph discarded"),
        }
    }

    /// Drops the functions of the last executor of a command that has completely ended (its
    /// executors are gone), keeping only the graph; the server calls it after `afterCommand`.
    /// Unlike [`Self::strip_kept_graph`], a graph something still references stays kept as it
    /// is (the next executor's strip decides).
    ///
    /// PORT: Blaze only. Java's SkyframeExecutor and its functions live with the server and
    /// hold the state of a command only between `beforeCommand` and `afterCommand` (the modules
    /// clear it there); a Blaze executor's functions capture that state, so dropping them ends
    /// it: the repository services, the download manager, the action cache and output service
    /// handles, the execution thread pools, and the arena lease of the `BzlLoadFunction`.
    pub fn release_command_functions(&self) {
        let mut inner = self.inner.lock();
        let Some(kept) = inner.kept.take() else {
            return;
        };
        let KeptGraph {
            evaluator,
            _lease,
            signature,
            command_token,
            remote,
        } = kept;
        let evaluator = match Arc::try_unwrap(evaluator) {
            Ok(evaluator) => Arc::new(
                EvaluatorBuilder::new()
                    .keep_edges(true)
                    .build_reusing(evaluator),
            ),
            Err(shared) => {
                debug("kept evaluator still referenced after the command: functions kept");
                shared
            }
        };
        inner.kept = Some(KeptGraph {
            evaluator,
            _lease,
            signature,
            command_token,
            remote,
        });
    }

    /// `memoizingEvaluator.delete(deletePredicate)` on the server's graph: the graph the current
    /// executor holds (from its next evaluation on, as Java's pending deletions; the graph keeps
    /// them when it is given back), or else the kept graph (from the next executor's first
    /// evaluation). Nothing to do when there is neither: a new graph has no nodes.
    // Java: com/google/devtools/build/skyframe/InMemoryMemoizingEvaluator.java:212 InMemoryMemoizingEvaluator.delete
    pub fn delete(&self, delete_predicate: &dyn Fn(&SkyKey) -> bool) {
        let inner = self.inner.lock();
        if let Some(evaluator) = inner.handed_out.as_ref().and_then(|w| w.upgrade()) {
            debug("delete: on the graph of the current executor");
            evaluator.delete(|k, _| delete_predicate(k));
        } else if let Some(kept) = &inner.kept {
            debug("delete: on the kept graph");
            kept.evaluator.delete(|k, _| delete_predicate(k));
        }
    }

    /// Records the evaluator built for `handle` (an `Arc` now) as the one a `delete` during the
    /// command applies to (see [`SkyframeIncrementalState::delete`]).
    pub fn attach(&self, handle: &ExecutorHandle, evaluator: &Arc<Evaluator>) {
        let mut inner = self.inner.lock();
        if handle.generation == inner.generation {
            inner.handed_out = Some(Arc::downgrade(evaluator));
        }
    }

    /// `SkyframeExecutor.notifyCommandComplete` for the graph kept between commands:
    /// `memoizingEvaluator.noteEvaluationsAtSameVersionMayBeFinished` posts the evaluation
    /// statistics of the command and resets them. The commands that report them (the build
    /// command's `SkyframeGraphStatsEvent`) took them already from their executor; for the others
    /// they are reset here, so the next command's statistics are its own.
    ///
    /// Returns the `SkyframeGraphStatsEvent` Java posts (`None` when no graph is kept: the
    /// command's executor took its statistics with its graph).
    // Java: com/google/devtools/build/lib/skyframe/SkyframeExecutor.java:1196 SkyframeExecutor.notifyCommandComplete
    pub fn notify_command_complete(
        &self,
    ) -> Option<blaze_skyframe::evaluation_stats::SkyframeGraphStatsEvent> {
        let inner = self.inner.lock();
        inner
            .kept
            .as_ref()
            .map(|kept| kept.evaluator.post_logging_stats())
    }

    /// The kept graph, for a command that reads the server's graph without evaluating on it
    /// (Java `skyframeExecutor.getEvaluator()` in `config` and `dump`). The caller drops it
    /// before the command ends (the server then strips the graph it keeps).
    pub fn kept_evaluator(&self) -> Option<Arc<Evaluator>> {
        self.inner
            .lock()
            .kept
            .as_ref()
            .map(|kept| kept.evaluator.clone())
    }

    /// The strong references to the kept graph's evaluator (`None`: no graph kept).
    ///
    /// PORT: Blaze-only check (root, 2026-11-26). Once a command has ended and its executors
    /// are gone, the kept graph must be the evaluator's only holder: anything else is an `Arc`
    /// cycle through a graph value or a function (the regressions of 2026-11-26: the package
    /// root lookup, the rewound-action synchronizer, the action executor's graph), which leaks
    /// the evaluator and makes the next executor discard the graph. The dispatcher checks it
    /// after every command ([`SkyframeIncrementalState::check_kept_graph_unshared`]).
    pub fn kept_graph_strong_count(&self) -> Option<usize> {
        self.inner
            .lock()
            .kept
            .as_ref()
            .map(|kept| Arc::strong_count(&kept.evaluator))
    }

    /// Checks [`SkyframeIncrementalState::kept_graph_strong_count`] after a command: panics in
    /// debug builds (so that a test that runs a command fails in the crate that adds a holder),
    /// logs to the server log in release builds. Returns whether the graph is unshared.
    pub fn check_kept_graph_unshared(&self, command_name: &str) -> bool {
        match self.kept_graph_strong_count() {
            None | Some(1) => true,
            Some(n) => {
                let message = format!(
                    "the kept Skyframe graph is still referenced after the '{command_name}' \
                     command: {n} strong references to its evaluator, expected 1 (an Arc cycle \
                     through a graph value or a function, or a holder that outlives its command)"
                );
                if cfg!(debug_assertions) {
                    panic!("{message}");
                }
                blaze_util::server_logger::info(
                    "com.google.devtools.build.lib.skyframe.SequencedSkyframeExecutor",
                    "afterCommand",
                    &message,
                );
                false
            }
        }
    }

    /// Whether a graph is kept for the next command.
    pub fn has_kept_graph(&self) -> bool {
        self.inner.lock().kept.is_some()
    }

    /// Whether an executor's graph goes to the next executor (of this command and of later
    /// ones): the mode keeps graphs and the current command tracks incremental state.
    pub fn passes_graph_between_executors(&self) -> bool {
        Self::passes_graph(self, &self.inner.lock())
    }

    fn passes_graph(state: &SkyframeIncrementalState, inner: &Inner) -> bool {
        state.mode == KeepGraphMode::Keep && inner.track_incremental_state
    }

    /// Drops the kept graph (Java `resetEvaluator`: `clean`, `--nokeep_state_after_build`).
    // Java: com/google/devtools/build/lib/skyframe/SkyframeExecutor.java:1176 SkyframeExecutor.resetEvaluator
    pub fn reset_evaluator(&self) {
        // Java: com/google/devtools/build/lib/skyframe/SequencedSkyframeExecutor.java:212 SequencedSkyframeExecutor.resetEvaluator
        self.diff_awareness.lock().manager.reset();
        let mut inner = self.inner.lock();
        inner.kept = None;
        // A graph an executor holds now is dropped when it is given back (a reset during a
        // command, as `invalidate_repo_directories`'s).
        inner.generation += 1;
        inner.handed_out = None;
        // skyframeBuildView.reset(): configuration = null, skyframeAnalysisWasDiscarded = false.
        inner.has_configuration = false;
        inner.original_configuration_options = None;
        inner.skyframe_analysis_was_discarded = false;
        inner.last_analysis_discarded = false;
        inner.last_top_level_build_options = None;
        inner.main_repo_mapping = None;
    }

    /// `RegistryFunction.LAST_INVALIDATION` as `BazelRepositoryModule.getPrecomputedValues`
    /// computes it: the value in the graph (`beforeCommand` reads it back), replaced by `now`
    /// once `INVALIDATION_INTERVAL` (an hour) has passed.
    // Java: com/google/devtools/build/lib/bazel/BazelRepositoryModule.java:744 BazelRepositoryModule.getPrecomputedValues
    pub fn last_registry_invalidation(&self, now: std::time::SystemTime) -> std::time::SystemTime {
        let inner = self.inner.lock();
        let last = inner.kept.as_ref().and_then(|kept| {
            kept.evaluator
                .get_existing_value(&SkyKey::new(
                    blaze_bzlmod::skyframe::precomputed::LAST_INVALIDATION.get_key(),
                ))
                .and_then(|v| {
                    v.downcast_ref::<blaze_skyframe::precomputed::PrecomputedValue>()
                        .and_then(|p| p.get().downcast_ref::<std::time::SystemTime>().copied())
                })
        });
        // lastRegistryInvalidation = Instant.EPOCH initially.
        let last = last.unwrap_or(std::time::UNIX_EPOCH);
        // RegistryFunction.INVALIDATION_INTERVAL = Duration.ofHours(1).
        if now > last + std::time::Duration::from_secs(3600) {
            now
        } else {
            last
        }
    }

    /// Builds the evaluator of an executor: over the kept graph when it may be kept, after the
    /// per-command sync; otherwise a new one. Returns the evaluator and the handle that gives it
    /// back when the executor is dropped.
    // Java: com/google/devtools/build/lib/skyframe/SequencedSkyframeExecutor.java:262 SequencedSkyframeExecutor.sync
    pub fn build_evaluator(
        command: &IncrementalCommand,
        builder: EvaluatorBuilder,
        inputs: SyncInputs<'_>,
    ) -> (Evaluator, ExecutorHandle) {
        let state = &command.state;
        let mut inner = state.inner.lock();
        let first_in_command = inner.synced_command != Some(command.command_token);
        if first_in_command {
            inner.synced_command = Some(command.command_token);
            if inner.decided_command != Some(command.command_token) {
                let track = command.track_incremental_state
                    && !(command.batch && command.discard_analysis_cache);
                Self::decide_keep_incremental_state(
                    state,
                    &mut inner,
                    command.command_token,
                    track,
                );
            }
        }
        let analyzed = inputs.top_level_build_options.is_some();
        let remote = remote_configured(inputs.options);
        inner.generation += 1;
        inner.handed_out = None;
        let handle = ExecutorHandle {
            command: command.clone(),
            signature: inputs.signature.clone(),
            analyzed,
            generation: inner.generation,
            remote,
        };
        // Java: com/google/devtools/build/lib/skyframe/SequencedSkyframeExecutor.java:226 SequencedSkyframeExecutor.createEvaluator
        // (trackIncrementalState ? DEFAULT_EVENT_FILTER_WITH_ACTIONS : EventFilter.NO_STORAGE).
        let builder = if Self::passes_graph(state, &inner) {
            builder.event_filter(Arc::new(DefaultEventFilterWithActions))
        } else {
            builder
        };

        let kept = inner.kept.take();
        if kept.is_none() {
            debug("no kept graph");
        }
        let previous = kept.and_then(|kept| {
            if kept.command_token != command.command_token
                && (kept.remote || remote)
                && remote_fail_safe_enabled()
            {
                // PORT: Blaze-only, fail safe until the remote invalidations are ported (review
                // 2026-11-26 S1-1, S1-2): RemoteOutputChecker.maybeInvalidateSkyframeValues with
                // the previous command's checker, the outputs materialized as top-level outputs,
                // and setSaltAndDeleteActionsIfChanged.
                let message = format!(
                    "Skyframe graph of the previous command discarded: {} has a remote cache, a \
                     disk cache or remote execution configured (remote output invalidation is \
                     not ported)",
                    if remote {
                        "this command"
                    } else {
                        "the previous command"
                    }
                );
                debug(&message);
                blaze_util::server_logger::info(
                    "com.google.devtools.build.lib.skyframe.SequencedSkyframeExecutor",
                    "sync",
                    &message,
                );
                return None;
            }
            if kept.signature != inputs.signature {
                // PORT: Blaze-only, fail safe (see LoadingSignature).
                debug("loading signature changed: graph discarded");
                return None;
            }
            // The executors that held the evaluator are gone; nothing else may still hold it.
            let r = Arc::try_unwrap(kept.evaluator).ok();
            if r.is_none() {
                debug("kept evaluator still referenced: graph discarded");
            }
            r
        });
        let Some(previous) = previous else {
            // A new graph: nothing of an earlier command is in it.
            Self::reset_graph_bookkeeping(&mut inner);
            Self::note_new_graph(&mut inner, &inputs, command);
            if first_in_command && state.mode == KeepGraphMode::Keep {
                // Java's handleDiffs runs on every graph, also a new one: it evaluates the
                // ignored subdirectories (new nodes) before it takes the diff awareness's view,
                // and a first view is EVERYTHING_MODIFIED, so the missing-diff check stats the
                // nodes just made after the watches exist (review round 2, S1-C).
                // Java: com/google/devtools/build/lib/skyframe/SkyframeExecutor.java:3775 SkyframeExecutor.handleDiffs
                let fallback = builder.clone();
                let evaluator = builder.build();
                let evaluator =
                    match Self::handle_diffs(state, &mut inner, command, evaluator, &inputs) {
                        Some(evaluator) => evaluator,
                        None => fallback.build(),
                    };
                drop(inner);
                Self::inject_environment_into_new_graph(&evaluator, &inputs);
                return (evaluator, handle);
            }
            drop(inner);
            let evaluator = builder.build();
            if first_in_command {
                Self::inject_environment_into_new_graph(&evaluator, &inputs);
            }
            return (evaluator, handle);
        };

        // The sync runs on the executor's evaluator (its functions and its precomputed values
        // injected, as Java's preparePackageLoading injects them before handleDiffs evaluates);
        // a sync that cannot keep the graph falls back to a new one.
        let fallback = builder.clone();
        let evaluator = builder.build_reusing(previous);
        if debug_enabled() {
            debug(&format!(
                "reusing the kept graph: {} nodes",
                evaluator.get_all_keys().len()
            ));
        }
        let evaluator = match Self::sync(
            state,
            &mut inner,
            command,
            evaluator,
            &inputs,
            first_in_command,
        ) {
            Some(evaluator) => evaluator,
            None => {
                Self::reset_graph_bookkeeping(&mut inner);
                Self::note_new_graph(&mut inner, &inputs, command);
                drop(inner);
                return (fallback.build(), handle);
            }
        };
        drop(inner);
        (evaluator, handle)
    }

    /// Java's handleDiffs ends with handleClientEnvironmentChanges and
    /// handleRepositoryEnvironmentChanges on every graph, also a new one: each variable of the
    /// client and repository environments becomes an injected node (`note_new_graph` records
    /// them as the previous environments).
    // Java: com/google/devtools/build/lib/skyframe/SkyframeExecutor.java:3861 SkyframeExecutor.handleDiffs
    fn inject_environment_into_new_graph(evaluator: &Evaluator, inputs: &SyncInputs<'_>) {
        inject_environment_values(
            evaluator,
            |name| SkyKey::new(blaze_skyframe::precomputed::ClientEnvironmentKey::key(name)),
            inputs.client_env,
            &IndexSet::new(),
        );
        inject_environment_values(
            evaluator,
            |name| {
                SkyKey::new(
                    blaze_repo_rules::repository::repo_environment_function::RepoEnvironmentKey::key(
                        name,
                    ),
                )
            },
            inputs.repo_env,
            &IndexSet::new(),
        );
    }

    /// The analysis-cache bookkeeping that describes the graph (Java resets it with the graph in
    /// `resetEvaluator`): a new graph starts without it, whatever discarded the old one.
    fn reset_graph_bookkeeping(inner: &mut Inner) {
        inner.last_execution_salt = None;
        inner.has_configuration = false;
        inner.original_configuration_options = None;
        inner.skyframe_analysis_was_discarded = false;
        inner.last_analysis_discarded = false;
        inner.last_top_level_build_options = None;
        inner.main_repo_mapping = None;
    }

    /// Records what a new (empty) graph was built from.
    fn note_new_graph(inner: &mut Inner, inputs: &SyncInputs<'_>, command: &IncrementalCommand) {
        inner.deleted_packages = inputs.deleted_packages.clone();
        inner.last_pkg_locator = Some(inputs.pkg_locator.read().clone());
        inner.previous_client_environment = inputs.client_env.keys().cloned().collect();
        inner.previous_repository_environment = inputs.repo_env.keys().cloned().collect();
        inner.last_analysis_discarded = false;
        inner.main_repo_mapping = None;
        if inputs.executes {
            // Nothing of an earlier build is in the new graph: no output to check.
            inner.outputs_checked_command = Some(command.command_token);
        }
        if inputs.top_level_build_options.is_some() {
            inner.last_top_level_build_options = inputs.top_level_build_options.clone();
        }
    }

    /// The bookkeeping part of `decideKeepIncrementalState` (the warnings are printed by
    /// blaze_runtime's `CommandEnvironment.decide_keep_incremental_state`).
    // Java: com/google/devtools/build/lib/skyframe/SequencedSkyframeExecutor.java:399 SequencedSkyframeExecutor.decideKeepIncrementalState
    /// `decideKeepIncrementalState` for a command whose decision (`trackIncrementalState`) the
    /// runtime computed with its warnings (blaze_runtime `CommandEnvironment`); runs for every
    /// command, also one that builds no executor.
    // Java: com/google/devtools/build/lib/runtime/CommandEnvironment.java:896 CommandEnvironment.decideKeepIncrementalState
    pub fn decide_keep_incremental_state_for_command(&self, command_token: u64, track: bool) {
        let mut inner = self.inner.lock();
        if inner.decided_command == Some(command_token) {
            return;
        }
        Self::decide_keep_incremental_state(self, &mut inner, command_token, track);
    }

    fn decide_keep_incremental_state(
        state: &SkyframeIncrementalState,
        inner: &mut Inner,
        command_token: u64,
        track: bool,
    ) {
        inner.decided_command = Some(command_token);
        let old_value_of_track_incremental_state = inner.track_incremental_state;
        // First check if the incrementality state should be kept around during the build
        // (`!explicitlyRequestedNoIncrementalData && !implicitlyRequestedNoIncrementalData`,
        // computed by the caller).
        inner.track_incremental_state = track;
        // Now check if it is necessary to wipe the previous state. We do this if either the
        // previous or current command requires the build to have been isolated.
        let evaluator_needs_reset = old_value_of_track_incremental_state
            != inner.track_incremental_state
            || !inner.track_incremental_state;
        if evaluator_needs_reset {
            // SequencedSkyframeExecutor.sync: resetEvaluator().
            state.diff_awareness.lock().manager.reset();
            inner.kept = None;
            inner.generation += 1;
            inner.handed_out = None;
            inner.has_configuration = false;
            inner.original_configuration_options = None;
            inner.skyframe_analysis_was_discarded = false;
            inner.last_analysis_discarded = false;
            inner.last_top_level_build_options = None;
            inner.main_repo_mapping = None;
        }
    }

    /// `invalidate(SkyFunctionName.functionIsIn(PACKAGE_LOCATOR_DEPENDENT_VALUES))`: the value
    /// types whose builders have direct access to the package locator, rather than accessing it
    /// via an explicit Skyframe dependency.
    // Java: com/google/devtools/build/lib/skyframe/SequencedSkyframeExecutor.java:352 SequencedSkyframeExecutor.PACKAGE_LOCATOR_DEPENDENT_VALUES
    // Java: com/google/devtools/build/lib/skyframe/SequencedSkyframeExecutor.java:362 SequencedSkyframeExecutor.onPkgLocatorChange
    fn on_pkg_locator_change(evaluator: &Evaluator) {
        use blaze_sky_loading::sky_functions;
        let dependent = [
            sky_functions::FILE_STATE,
            sky_functions::FILE,
            sky_functions::DIRECTORY_LISTING_STATE,
            blaze_sky_loading::prepare_deps::PREPARE_DEPS_OF_PATTERN,
            sky_functions::TARGET_PATTERN,
            crate::target_pattern_phase::TARGET_PATTERN_PHASE,
        ];
        let keys: Vec<SkyKey> = evaluator
            .get_all_keys()
            .into_iter()
            .filter(|k| dependent.contains(&k.function_name()))
            .collect();
        evaluator.invalidate(keys);
    }

    /// What Java does to the kept graph before an evaluation of this command.
    fn sync(
        state: &SkyframeIncrementalState,
        inner: &mut Inner,
        command: &IncrementalCommand,
        evaluator: Evaluator,
        inputs: &SyncInputs<'_>,
        first_in_command: bool,
    ) -> Option<Evaluator> {
        if first_in_command {
            // Java: com/google/devtools/build/lib/skyframe/SkyframeExecutor.java:1733 SkyframeExecutor.preparePackageLoading
            evaluator.emitted_event_state().clear();

            // Java: com/google/devtools/build/lib/skyframe/SkyframeExecutor.java:1759 SkyframeExecutor.setPackageLocator
            let pkg_locator = inputs.pkg_locator.read().clone();
            let old_locator = inner.last_pkg_locator.replace(pkg_locator.clone());
            if old_locator.is_some_and(|old| old != pkg_locator) {
                // Java: com/google/devtools/build/lib/skyframe/SequencedSkyframeExecutor.java:362 SequencedSkyframeExecutor.onPkgLocatorChange
                debug("invalidate: package locator changed");
                Self::on_pkg_locator_change(&evaluator);
            }

            // Java: com/google/devtools/build/lib/skyframe/SequencedSkyframeExecutor.java:373 SequencedSkyframeExecutor.setDeletedPackages
            let newly_deleted_or_not_deleted_packages: HashSet<PackageIdentifier> = inner
                .deleted_packages
                .symmetric_difference(&inputs.deleted_packages)
                .cloned()
                .collect();
            if !newly_deleted_or_not_deleted_packages.is_empty() {
                // PackageLookupValue is a HERMETIC node type, so we can't invalidate it.
                debug("delete: deleted packages changed");
                evaluator.delete(|k, _| {
                    k.downcast::<PackageLookupKey>()
                        .is_some_and(|k| newly_deleted_or_not_deleted_packages.contains(&k.0))
                });
            }
            inner.deleted_packages = inputs.deleted_packages.clone();

            // Java: com/google/devtools/build/lib/skyframe/SkyframeExecutor.java:3064 SkyframeExecutor.syncPackageLoading
            evaluator.invalidate_transient_errors();

            // Java: com/google/devtools/build/lib/skyframe/SkyframeExecutor.java:3012 SkyframeExecutor.sync
            if inner.last_analysis_discarded {
                // dropConfiguredTargetsNow(eventHandler): handleAnalysisInvalidatingChange.
                let current = inner.last_top_level_build_options.clone();
                debug("delete: last analysis discarded");
                evaluator.delete(|k, v| {
                    should_delete_on_analysis_invalidating_change(k, v, current.as_ref(), true)
                });
                inner.last_analysis_discarded = false;
            }

            // Java: com/google/devtools/build/lib/skyframe/SkyframeExecutor.java:3774 SkyframeExecutor.handleDiffs
            let evaluator = Self::handle_diffs(state, inner, command, evaluator, inputs)?;

            // handleClientEnvironmentChanges, handleRepositoryEnvironmentChanges.
            inner.previous_client_environment = inject_environment_values(
                &evaluator,
                |name| SkyKey::new(blaze_skyframe::precomputed::ClientEnvironmentKey::key(name)),
                inputs.client_env,
                &inner.previous_client_environment,
            );
            inner.previous_repository_environment = inject_environment_values(
                &evaluator,
                |name| {
                    SkyKey::new(
                        blaze_repo_rules::repository::repo_environment_function::RepoEnvironmentKey::key(
                            name,
                        ),
                    )
                },
                inputs.repo_env,
                &inner.previous_repository_environment,
            );
            return Self::sync_executor(inner, command, evaluator, inputs);
        }
        Self::sync_executor(inner, command, evaluator, inputs)
    }

    /// The checks every executor makes over the kept graph (also the second one of a command).
    fn sync_executor(
        inner: &mut Inner,
        command: &IncrementalCommand,
        evaluator: Evaluator,
        inputs: &SyncInputs<'_>,
    ) -> Option<Evaluator> {
        // The top-level options need nothing here: the functions that read the baseline depend
        // on BASELINE_CONFIGURATION, which every executor injects (Java's setBaselineConfiguration),
        // and the analysis-cache decision is `set_configuration`'s.
        if inputs.top_level_build_options.is_some() {
            inner.last_top_level_build_options = inputs.top_level_build_options.clone();
        }
        if inputs.executes && inner.outputs_checked_command != Some(command.command_token) {
            inner.outputs_checked_command = Some(command.command_token);
            Self::detect_modified_output_files(&evaluator, command, inputs);
        }
        Some(evaluator)
    }

    /// Invalidates the actions whose outputs were modified outside the build.
    // Java: com/google/devtools/build/lib/skyframe/SequencedSkyframeExecutor.java:481 SequencedSkyframeExecutor.detectModifiedOutputFiles
    fn detect_modified_output_files(
        evaluator: &Evaluator,
        command: &IncrementalCommand,
        inputs: &SyncInputs<'_>,
    ) {
        // ExecutionTool.prepareForExecution: `startBuildAndDetermineModifiedOutputFiles`;
        // Java: com/google/devtools/build/lib/buildtool/ExecutionTool.java:493 ExecutionTool.startBuildAndDetermineModifiedOutputFiles
        // without an output service and with --experimental_check_output_files the modified set is
        // EVERYTHING_MODIFIED, without it NOTHING_MODIFIED.
        let check_output_files = inputs
            .options
            .get_options::<PackageOptions>()
            .is_none_or(|o| o.check_output_files);
        let modified_output_files = if check_output_files {
            ModifiedFileSet::EVERYTHING_MODIFIED
        } else {
            ModifiedFileSet::nothing_modified()
        };
        struct Receiver;
        impl blaze_sky_exec::output_checker::ModifiedOutputsReceiver for Receiver {
            // PORT: Java counts the files for its metrics (modifiedFiles, outputDirtyFiles);
            // not ported (logging and ExecutionFinishedEvent metrics).
            fn report_modified_output_file(
                &self,
                _maybe_modified_time: i64,
                _artifact: &blaze_actions::Artifact,
            ) {
            }
        }
        let dirty = blaze_sky_exec::output_checker::get_dirty_action_values(
            &evaluator.get_values(),
            &modified_output_files,
            inputs
                .output_checker
                .as_deref()
                .unwrap_or(&blaze_exec::actions::action_cache_checker::TrustLocalOnly),
            &Receiver,
            Some(&command.tsgm),
            fsvc_threads(inputs.options),
        );
        evaluator.invalidate(dirty);
    }

    /// `handleDiffs`: the diff awareness of each package path entry (`--watchfs`: the files it
    /// names are checked; without it every entry is without diff information and checked
    /// whole), then the external and output files.
    // Java: com/google/devtools/build/lib/skyframe/SkyframeExecutor.java:3775 SkyframeExecutor.handleDiffs
    fn handle_diffs(
        state: &SkyframeIncrementalState,
        _inner: &mut Inner,
        command: &IncrementalCommand,
        evaluator: Evaluator,
        inputs: &SyncInputs<'_>,
    ) -> Option<Evaluator> {
        let ctx = EvaluationContext {
            keep_going: false,
            parallelism: crate::skyframe_executor::DEFAULT_THREAD_COUNT,
            event_handler: inputs.reporter.clone(),
            ..EvaluationContext::default()
        };
        let mut diff_awareness = state.diff_awareness.lock();
        let Some(diffs) = Self::get_diffs(state, &mut diff_awareness, &evaluator, &ctx, inputs)
        else {
            // Interrupted: the graph is not known to be consistent with the file system.
            debug("interrupted while computing the ignored subdirectories: graph discarded");
            return None;
        };
        let mut modified_files_by_path_entry = Vec::new();
        let mut path_entries_without_diff_information = Vec::new();
        for (path_entry, modified_file_set) in diffs {
            if modified_file_set
                .get_modified_file_set()
                .treat_everything_as_modified()
            {
                path_entries_without_diff_information.push((path_entry, modified_file_set));
            } else {
                modified_files_by_path_entry.push((path_entry, modified_file_set));
            }
        }

        // Java: com/google/devtools/build/lib/skyframe/SkyframeExecutor.java:3923 SkyframeExecutor.handleDiffsWithCompleteDiffInformation
        {
            let _c = blaze_util::profiler::Profiler::instance().profile(
                blaze_util::profiler::ProfilerTask::INFO,
                "handleDiffsWithCompleteDiffInformation",
            );
            for (path_entry, processable_modified_file_set) in &modified_files_by_path_entry {
                let modified_file_set = processable_modified_file_set.get_modified_file_set();
                blaze_util::check_state!(
                    !modified_file_set.treat_everything_as_modified(),
                    "{}",
                    path_entry
                );
                let diff = match Self::get_diff(
                    &command.tsgm,
                    modified_file_set,
                    path_entry,
                    &evaluator,
                ) {
                    Ok(diff) => diff,
                    Err(e) => {
                        // PORT: Java throws the AbruptExitException (DIFF_STAT_FAILED) out of the
                        // sync and the command fails; Blaze's sync has no failure path, so the
                        // graph is discarded (fail safe) and the message goes to the log.
                        debug(&format!("{}: graph discarded", e.abrupt_exit_message()));
                        return None;
                    }
                };
                // handleChangedFiles(ImmutableList.of(pathEntry), diff, 0)
                Self::handle_changed_files(
                    &evaluator,
                    diff.values_to_invalidate,
                    diff.values_to_inject,
                );
                diff_awareness
                    .manager
                    .mark_processed(processable_modified_file_set);
            }
        }

        let evaluator = Self::handle_diffs_with_missing_diff_information(
            state,
            command,
            evaluator,
            inputs,
            &path_entries_without_diff_information,
        )?;
        for (_, processable_modified_file_set) in &path_entries_without_diff_information {
            diff_awareness
                .manager
                .mark_processed(processable_modified_file_set);
        }
        Some(evaluator)
    }

    /// The part of `handleDiffs` that asks the diff awareness of each package path entry for its
    /// changes: the evaluation of the ignored subdirectories (`.bazelignore`), then
    /// `diffAwarenessManager.getDiff` per entry. `None` when the evaluation was interrupted.
    // Java: com/google/devtools/build/lib/skyframe/SkyframeExecutor.java:3775 SkyframeExecutor.handleDiffs
    fn get_diffs(
        state: &SkyframeIncrementalState,
        diff_awareness: &mut DiffAwarenessSync,
        evaluator: &Evaluator,
        ctx: &EvaluationContext,
        inputs: &SyncInputs<'_>,
    ) -> Option<Vec<(Root, ProcessableModifiedFileSet)>> {
        let pkg_roots: Vec<Root> = inputs.pkg_locator.read().get_path_entries().to_vec();
        let workspace_path = Some(inputs.directories.get_workspace().clone());
        // evaluateSkyKeys(eventHandler, ImmutableList.of(IgnoredSubdirectoriesValue.key()))
        let ignored_key = IgnoredSubdirectoriesKey::key_main();
        let evaluation_result = evaluator
            .evaluate(&[SkyKey::new(ignored_key.clone())], ctx)
            .ok()?;
        let ignored_subdirectories_value: Option<Arc<IgnoredSubdirectoriesValue>> =
            evaluation_result.get(&ignored_key);
        let _ = state;
        let options = inputs
            .options
            .get_options::<LocalDiffAwarenessOptions>()
            .unwrap_or_else(blaze_options::Options::<LocalDiffAwarenessOptions>::get_defaults);
        let mut diffs = Vec::new();
        for path_entry in pkg_roots {
            // Ignored subdirectories are specified relative to the workspace root by definition
            // of .bazelignore. So, we only use ignored paths when the package root is equal to
            // the workspace path.
            if let (Some(workspace_path), Some(entry_path)) =
                (&workspace_path, path_entry.as_path())
                && workspace_path == entry_path
                && let Some(ignored) = &ignored_subdirectories_value
            {
                diff_awareness.ignored_paths = ignored
                    .as_ignored_subdirectories()
                    .with_prefix(&entry_path.as_fragment().to_relative());
            }
            let ignored_paths = diff_awareness.ignored_paths.clone();
            let modified_file_set = diff_awareness.manager.get_diff(
                inputs.reporter.as_ref() as &dyn EventHandler,
                &path_entry,
                &ignored_paths,
                &options,
            );
            // PORT: with one package path entry Java hands the diff's workspace info to its
            // WorkspaceInfoFromDiffReceiver; Bazel's ignores it.
            diffs.push((path_entry, modified_file_set));
        }
        Some(diffs)
    }

    /// The invalidations and injections of the files a complete diff names.
    // Java: com/google/devtools/build/lib/skyframe/SkyframeExecutor.java:1640 SkyframeExecutor.getDiff
    fn get_diff(
        tsgm: &Arc<TimestampGranularityMonitor>,
        modified_file_set: &ModifiedFileSet,
        path_entry: &Root,
        evaluator: &Evaluator,
    ) -> Result<InferredDiff, StatFailedException> {
        if modified_file_set.modified_source_files().is_empty() {
            return Ok(InferredDiff::default());
        }
        let dirty_file_state_sky_keys: Vec<FileStateKey> = modified_file_set
            .modified_source_files()
            .iter()
            .map(|path_fragment| {
                blaze_util::check_state!(
                    !path_fragment.is_absolute(),
                    "found absolute PathFragment: {}",
                    path_fragment
                );
                FileStateKey::key(RootedPath::to_rooted_path(
                    path_entry.clone(),
                    path_fragment,
                ))
            })
            .collect();
        get_diff_with_inferred_ancestors(
            Some(tsgm),
            evaluator,
            dirty_file_state_sky_keys,
            &FileDirtinessChecker,
        )
    }

    // Java: com/google/devtools/build/lib/skyframe/SkyframeExecutor.java:4121 SkyframeExecutor.handleChangedFiles
    fn handle_changed_files(
        evaluator: &Evaluator,
        keys_to_be_changed_later_in_this_build: impl IntoIterator<Item = SkyKey>,
        changed_keys_with_new_values: impl IntoIterator<Item = (SkyKey, SkyValue)>,
    ) {
        // PORT: logDiffInfo (logging) and the modified-files counter are not ported.
        evaluator.invalidate(keys_to_be_changed_later_in_this_build);
        for (key, value) in changed_keys_with_new_values {
            evaluator.inject_dyn(key, value);
        }
    }

    /// Finds and invalidates changed files under path entries whose corresponding diff awareness
    /// said all files may have been modified, and the external and output files.
    // Java: com/google/devtools/build/lib/skyframe/SkyframeExecutor.java:3950 SkyframeExecutor.handleDiffsWithMissingDiffInformation
    fn handle_diffs_with_missing_diff_information(
        state: &SkyframeIncrementalState,
        command: &IncrementalCommand,
        evaluator: Evaluator,
        inputs: &SyncInputs<'_>,
        path_entries_without_diff_information: &[(Root, ProcessableModifiedFileSet)],
    ) -> Option<Evaluator> {
        let package_options = inputs.options.get_options::<PackageOptions>();
        let check_output_files = package_options
            .as_ref()
            .is_none_or(|o| o.check_output_files);
        let check_external_other_files = package_options
            .as_ref()
            .is_none_or(|o| o.check_external_other_files);
        let check_external_repository_files = inputs
            .options
            .get_options::<RepositoryOptions>()
            .is_some_and(|o| o.check_external_repository_files);
        // SkyframeExecutor.allowExternalRepositories: true for Bazel
        // (BazelRepositoryModule / SequencedSkyframeExecutorFactory).
        let allow_external_repositories = true;

        let external_files_helper = Arc::new(ExternalFilesHelper::create_with_knowledge(
            inputs.pkg_locator.clone(),
            ExternalFileAction::DependOnExternalPkgForExternalRepoPaths,
            inputs.directories.clone(),
            inputs.repo_contents_cache_path.clone(),
            state.external_files_knowledge.clone(),
        ));
        let external_files_knowledge = external_files_helper.get_external_files_knowledge();
        if !path_entries_without_diff_information.is_empty()
            || (check_output_files && external_files_knowledge.any_output_files_seen)
            || (check_external_repository_files && allow_external_repositories)
            || (check_external_repository_files
                && external_files_knowledge.any_files_in_external_repos_seen)
            || (check_external_other_files
                && external_files_knowledge.too_many_external_other_files_seen)
        {
            // We freshly compute knowledge of the presence of external files in the skyframe
            // graph. We use a fresh ExternalFilesHelper instance and only set the real instance's
            // knowledge *after* we are done with the graph scan, lest an interrupt during the
            // graph scan causes us to incorrectly think there are no longer any external files.
            let tmp_external_files_helper =
                Arc::new(external_files_helper.clone_with_fresh_external_files_knowledge());

            // Before running the FilesystemValueChecker ensure that all values marked for
            // invalidation have actually been invalidated, because checking those is a waste of
            // time.
            // Java: com/google/devtools/build/lib/skyframe/SkyframeExecutor.java:4098 SkyframeExecutor.applyInvalidation
            let ctx = EvaluationContext {
                keep_going: false,
                parallelism: crate::skyframe_executor::DEFAULT_THREAD_COUNT,
                event_handler: inputs.reporter.clone(),
                ..EvaluationContext::default()
            };
            if evaluator.evaluate(&[], &ctx).is_err() {
                // Interrupted: the graph is not known to be consistent with the file system.
                debug("interrupted while applying invalidations: graph discarded");
                return None;
            }

            let fsvc = FilesystemValueChecker::new(
                Some(command.tsgm.clone()),
                fsvc_threads(inputs.options),
            );
            // getDiffPackageRootsUnderWhichToCheck
            let diff_package_roots_under_which_to_check: HashSet<Root> =
                path_entries_without_diff_information
                    .iter()
                    .map(|(root, _)| root.clone())
                    .collect();
            let mut dirtiness_checkers: Vec<Arc<dyn SkyValueDirtinessChecker>> = Vec::new();
            if !diff_package_roots_under_which_to_check.is_empty() {
                dirtiness_checkers.push(Arc::new(MissingDiffDirtinessChecker::new(
                    diff_package_roots_under_which_to_check,
                )));
            }
            let mut file_types_to_check: Vec<FileType> = Vec::new();
            if check_external_repository_files {
                file_types_to_check = vec![FileType::ExternalRepo];
            }
            if check_external_other_files
                && (external_files_knowledge.too_many_external_other_files_seen
                    || !external_files_knowledge
                        .external_other_files_seen
                        .is_empty())
            {
                file_types_to_check.push(FileType::ExternalOther);
            }
            // See the comment for FileType.OUTPUT for why we need to consider output files here.
            if check_output_files {
                file_types_to_check.push(FileType::Output);
            }
            // Iterables.toString(fileTypesToCheck), for the log line below.
            let file_types_text = file_type_set_to_string(&file_types_to_check);
            let external_dirtiness_checker = (!file_types_to_check.is_empty()).then(|| {
                // FileType.REPO_CONTENTS_CACHE_DIRS is intentionally never checked here.
                Arc::new(ExternalDirtinessChecker::new(
                    tmp_external_files_helper.clone(),
                    file_types_to_check,
                ))
            });
            if let Some(checker) = &external_dirtiness_checker {
                dirtiness_checkers.push(checker.clone());
            }
            blaze_util::check_argument!(!dirtiness_checkers.is_empty());

            // Java: com/google/devtools/build/lib/skyframe/SkyframeExecutor.java:4022 SkyframeExecutor.handleDiffsWithMissingDiffInformation
            blaze_util::server_logger::info(
                "com.google.devtools.build.lib.skyframe.SkyframeExecutor",
                "handleDiffsWithMissingDiffInformation",
                &format!(
                    "About to scan skyframe graph checking for filesystem nodes of types {}",
                    file_types_text
                ),
            );
            let batch_dirty_result = fsvc.get_dirty_keys(
                &evaluator.get_done_values(),
                &UnionDirtinessChecker::new(dirtiness_checkers),
            );
            if let Some(checker) = &external_dirtiness_checker {
                evaluator.invalidate(external_files_helper.get_extra_keys_to_invalidate(
                    &checker.get_dirty_external_repos(),
                    inputs.reporter.as_ref() as &dyn EventHandler,
                ));
            }
            Self::handle_changed_files(
                &evaluator,
                batch_dirty_result
                    .changed_keys_without_new_values()
                    .iter()
                    .cloned(),
                batch_dirty_result
                    .changed_keys_with_new_values()
                    .iter()
                    .map(|(k, v)| (k.clone(), v.clone())),
            );
            // We use the knowledge gained during the graph scan that just completed. Otherwise,
            // naively, once an external file gets into the Skyframe graph, we'll
            // overly-conservatively always think the graph needs to be scanned.
            external_files_helper.set_external_files_knowledge(
                tmp_external_files_helper.get_external_files_knowledge(),
            );
        } else if check_external_other_files
            && !external_files_knowledge
                .external_other_files_seen
                .is_empty()
        {
            // Java: com/google/devtools/build/lib/skyframe/SkyframeExecutor.java:4048 SkyframeExecutor.handleDiffsWithMissingDiffInformation
            blaze_util::server_logger::info(
                "com.google.devtools.build.lib.skyframe.SkyframeExecutor",
                "handleDiffsWithMissingDiffInformation",
                &format!(
                    "About to scan {} external files",
                    external_files_knowledge.external_other_files_seen.len()
                ),
            );
            let fsvc = FilesystemValueChecker::new(
                Some(command.tsgm.clone()),
                fsvc_threads(inputs.options),
            );
            let mut external_dirty_nodes = std::collections::HashMap::new();
            for path in &external_files_knowledge.external_other_files_seen {
                let key = SkyKey::new(FileStateKey::key(path.clone()));
                if let Some(value) = evaluator.get_existing_value(&key) {
                    external_dirty_nodes.insert(key, value);
                }
                let key = SkyKey::new(DirectoryListingStateKey::key(path.clone()));
                if let Some(value) = evaluator.get_existing_value(&key) {
                    external_dirty_nodes.insert(key, value);
                }
            }
            let batch_dirty_result = fsvc.get_dirty_keys(
                &external_dirty_nodes,
                &ExternalDirtinessChecker::new(
                    external_files_helper.clone(),
                    vec![FileType::ExternalOther],
                ),
            );
            Self::handle_changed_files(
                &evaluator,
                batch_dirty_result
                    .changed_keys_without_new_values()
                    .iter()
                    .cloned(),
                batch_dirty_result
                    .changed_keys_with_new_values()
                    .iter()
                    .map(|(k, v)| (k.clone(), v.clone())),
            );
        }
        Some(evaluator)
    }

    /// Gives an executor's evaluator back (the executor is being dropped).
    ///
    /// The graph of a command with `--nokeep_state_after_build` is dropped at the end of the
    /// command by [`Self::reset_evaluator`] (Java `BlazeRuntime.afterCommand`), not here: the
    /// command's later executors still use it, as Java's one executor does.
    pub fn put_back(&self, handle: ExecutorHandle, evaluator: Arc<Evaluator>) {
        // Blaze-only diagnostics: `BLAZE_INCREMENTAL_DEBUG_KEYS=<file>` in the server's
        // environment writes the keys of every graph given back (one per line, sorted).
        if let Some(path) = std::env::var_os("BLAZE_INCREMENTAL_DEBUG_KEYS") {
            let mut keys: Vec<String> = evaluator
                .get_all_keys()
                .iter()
                .map(|k| format!("{k:?}"))
                .collect();
            keys.sort();
            let _ = std::fs::write(path, keys.join("\n") + "\n");
        }
        if debug_enabled() {
            debug(&format!(
                "put_back: {} nodes ({})",
                evaluator.get_all_keys().len(),
                handle.command.command_id
            ));
        }
        let mut inner = self.inner.lock();
        if handle.generation != inner.generation {
            // A reset, or a newer graph handed out meanwhile: this graph is not the current one.
            debug("put_back: stale graph not kept");
            return;
        }
        if !Self::passes_graph(self, &inner)
            || inner.synced_command != Some(handle.command.command_token)
        {
            debug("put_back: graph not kept");
            inner.kept = None;
            return;
        }
        if handle.analyzed && handle.command.discard_analysis_cache {
            // SkyframeBuildView.clearAnalysisCache: skyframeAnalysisWasDiscarded = true;
            // SkyframeExecutor.discardPreExecutionCache: lastAnalysisDiscarded = true.
            // PORT: Java clears the analysis values now (keeping the nodes for execution) and
            // deletes them at the next command; Blaze keeps them until the next command, which
            // deletes them (the same nodes: every cleared value is deleted by
            // `shouldDeleteOnAnalysisInvalidatingChange`).
            inner.skyframe_analysis_was_discarded = true;
            inner.last_analysis_discarded = true;
        }
        inner.kept = Some(KeptGraph {
            evaluator,
            _lease: blaze_util::build_arena::lease(),
            signature: handle.signature,
            command_token: handle.command.command_token,
            remote: handle.remote,
        });
    }

    /// `SkyframeBuildView.shouldDiscardAnalysisCache` and `setConfiguration`: prints Java's
    /// message when the analysis cache is discarded, and deletes the analysis nodes.
    ///
    /// Returns the `InvalidConfigurationException` message when discards are not allowed.
    // Java: com/google/devtools/build/lib/skyframe/SkyframeBuildView.java:301 SkyframeBuildView.shouldDiscardAnalysisCache
    // Java: com/google/devtools/build/lib/skyframe/SkyframeBuildView.java:350 SkyframeBuildView.setConfiguration
    pub fn set_configuration(
        &self,
        evaluator: &Evaluator,
        event_handler: &dyn EventHandler,
        new_options: &BuildOptions,
        max_differences_to_show: i32,
        allow_analysis_cache_discards: bool,
    ) -> Result<(), String> {
        let mut inner = self.inner.lock();
        let should_discard = if !inner.has_configuration {
            false
        } else if inner.skyframe_analysis_was_discarded {
            event_handler.handle(&Event::warn(
                "--discard_analysis_cache was used in the previous build, discarding analysis cache.",
            ));
            true
        } else {
            let diff = describe_configuration_difference(
                inner
                    .original_configuration_options
                    .as_ref()
                    .expect("configuration set"),
                new_options,
                max_differences_to_show,
            );
            match diff {
                Some(diff) => {
                    if !allow_analysis_cache_discards {
                        return Err(format!("{diff}, analysis cache would have been discarded."));
                    }
                    event_handler.handle(&Event::warn(format!(
                        "{diff}, discarding analysis cache (this can be expensive, see \
                         https://bazel.build/advanced/performance/iteration-speed)."
                    )));
                    true
                }
                None => false,
            }
        };
        if should_discard {
            // Note that clearing the analysis cache is currently required for correctness.
            inner.original_configuration_options = Some(new_options.clone());
            // skyframeExecutor.handleAnalysisInvalidatingChange()
            let current = Some(new_options.clone());
            debug("delete: analysis-invalidating configuration change");
            evaluator.delete(|k, v| {
                should_delete_on_analysis_invalidating_change(k, v, current.as_ref(), false)
            });
        } else if !inner.has_configuration {
            inner.original_configuration_options = Some(new_options.clone());
        }
        inner.has_configuration = true;
        inner.skyframe_analysis_was_discarded = false;
        Ok(())
    }

    /// The main repository mapping of this command, computed outside the analysis functions that
    /// read it from a cell (Blaze-only: Java's functions request `RepositoryMappingValue`). A
    /// change deletes the analysis nodes, fail safe.
    pub fn note_main_repo_mapping(&self, evaluator: &Evaluator, mapping: &RepositoryMapping) {
        let mut inner = self.inner.lock();
        if inner
            .main_repo_mapping
            .as_ref()
            .is_some_and(|previous| previous != mapping)
        {
            debug("delete: main repository mapping changed");
            evaluator.delete(|k, _| is_analysis_key(k));
        }
        inner.main_repo_mapping = Some(mapping.clone());
    }
}

impl ExecutorHandle {
    /// The server state this handle belongs to.
    pub fn state(&self) -> &Arc<SkyframeIncrementalState> {
        &self.command.state
    }
}

/// `Iterables.toString(fileTypesToCheck)` of Java's `EnumSet<FileType>`: the constant names in
/// declaration order.
fn file_type_set_to_string(file_types: &[FileType]) -> String {
    let mut sorted: Vec<FileType> = file_types.to_vec();
    sorted.sort_by_key(|t| *t as u8);
    sorted.dedup();
    let names: Vec<&str> = sorted
        .iter()
        .map(|t| match t {
            FileType::Bundled => "BUNDLED",
            FileType::Internal => "INTERNAL",
            FileType::Output => "OUTPUT",
            FileType::ExternalRepo => "EXTERNAL_REPO",
            FileType::RepoContentsCacheDirs => "REPO_CONTENTS_CACHE_DIRS",
            FileType::ExternalOther => "EXTERNAL_OTHER",
        })
        .collect();
    format!("[{}]", names.join(", "))
}

/// `--experimental_fsvc_threads` (Java `buildRequestOptions == null ? 200 : fsvcThreads`).
fn fsvc_threads(options: &OptionsParser) -> usize {
    options
        .get_options::<blaze_optdefs::command::build_request_options::BuildRequestOptions>()
        .map(|o| o.fsvc_threads.max(1) as usize)
        .unwrap_or(200)
}

/// Injects the current value of each environment variable as a separate Skyframe node and
/// invalidates the nodes for any variables that disappeared since the previous call. Returns the
/// new set of variable names to remember as the previous environment.
// Java: com/google/devtools/build/lib/skyframe/SkyframeExecutor.java:3897 SkyframeExecutor.injectEnvironmentValues
fn inject_environment_values(
    evaluator: &Evaluator,
    key_fn: impl Fn(&str) -> SkyKey,
    environment: &indexmap::IndexMap<String, String>,
    previous_environment: &IndexSet<String>,
) -> IndexSet<String> {
    // Remove deleted environment variables.
    let deleted_keys: Vec<SkyKey> = previous_environment
        .iter()
        .filter(|k| !environment.contains_key(*k))
        .map(|k| key_fn(k))
        .collect();
    evaluator.invalidate(deleted_keys);
    // Inject current environmental values. We can inject unconditionally without fearing
    // over-invalidation; skyframe will not invalidate an injected key if the key's new value is
    // the same as the old value.
    for (name, value) in environment {
        evaluator.inject_dyn(
            key_fn(name),
            Arc::new(blaze_skyframe::precomputed::EnvironmentVariableValue(Some(
                value.clone(),
            ))),
        );
    }
    environment.keys().cloned().collect()
}

/// The keys of analysis nodes: configured targets, aspects, configurations and the execution
/// nodes above them.
fn is_analysis_key(k: &SkyKey) -> bool {
    let name = k.function_name();
    is_analysis_phase_action_lookup_key(k)
        || name == <BuildConfigurationKey as Key>::FUNCTION
        || name == <blaze_sky_exec::keys::ActionExecutionKey as Key>::FUNCTION
        || name == <blaze_sky_exec::keys::ArtifactNestedSetKey as Key>::FUNCTION
        || name
            == <blaze_sky_exec::action_template_expansion_value::ActionTemplateExpansionKey as Key>::FUNCTION
}

/// Whether a configuration key is `EMPTY_OPTIONS`'s.
// Java: com/google/devtools/build/lib/skyframe/SkyframeExecutor.java:1363 SkyframeExecutor.isEmptyOptionsKey
fn is_empty_options_key(key: Option<&BuildConfigurationKey>) -> bool {
    key.is_some_and(|key| {
        key.get_options_checksum() == blaze_build_config::transitions::EMPTY_OPTIONS.checksum()
    })
}

// Java: com/google/devtools/build/lib/skyframe/SequencedSkyframeExecutor.java:711 SequencedSkyframeExecutor.isExecConfig
fn is_exec_config(bck: Option<&BuildConfigurationKey>) -> bool {
    bck.is_some_and(|bck| {
        bck.get_options()
            .get::<blaze_optdefs::fragment::core_options::CoreOptions>()
            .is_some_and(|core| core.is_exec)
    })
}

/// Which nodes an analysis-invalidating change deletes.
///
/// PORT: `v.isCleared()` (values cleared by `--discard_analysis_cache`) is `cleared`: Blaze does
/// not clear values; after such a build every configured target and aspect not in the empty
/// options configuration is deleted, which is the set Java clears. An
/// `ActionTemplateExpansionKey` is deleted unconditionally (Java keeps it when its owner is kept;
/// it is re-evaluated, which only costs time).
// Java: com/google/devtools/build/lib/skyframe/SequencedSkyframeExecutor.java:672 SequencedSkyframeExecutor.shouldDeleteOnAnalysisInvalidatingChange
fn should_delete_on_analysis_invalidating_change(
    k: &SkyKey,
    _v: Option<&SkyValue>,
    current_options: Option<&BuildOptions>,
    cleared: bool,
) -> bool {
    use blaze_analysis::aspect_key_creator::{AspectKey, TopLevelAspectsKey};
    use blaze_analysis::configured_target_key::ConfiguredTargetKey;
    let name = k.function_name();
    // Remove ActionLookupData since all such nodes depend on ActionLookupKey nodes and deleting
    // en masse is cheaper than deleting via graph traversal.
    if name == <blaze_sky_exec::keys::ArtifactNestedSetKey as Key>::FUNCTION
        || name == <blaze_sky_exec::keys::ActionExecutionKey as Key>::FUNCTION
        || name
            == <blaze_sky_exec::action_template_expansion_value::ActionTemplateExpansionKey as Key>::FUNCTION
    {
        return true;
    }
    // Remove BuildConfigurationKeys except for the currently active key and the key for
    // EMPTY_OPTIONS, which is a constant and will be re-used frequently.
    if let Some(bck) = k.downcast::<BuildConfigurationKey>() {
        if is_empty_options_key(Some(bck)) {
            return false;
        }
        if current_options.is_some_and(|o| **bck.get_options() == *o) {
            return false;
        }
        if is_exec_config(Some(bck)) {
            return false;
        }
        return true;
    }
    // Remove ActionLookupKeys unless they are for the empty options config, in which case they
    // will be re-used frequently and we can avoid re-creating them. They are dependencies of the
    // empty configuration key and will never change.
    let configuration_key: Option<Option<&BuildConfigurationKey>> =
        if let Some(ct) = k.downcast::<ConfiguredTargetKey>() {
            Some(ct.get_configuration_key())
        } else if let Some(a) = k.downcast::<AspectKey>() {
            Some(a.get_configuration_key())
        } else if let Some(a) = k.downcast::<TopLevelAspectsKey>() {
            Some(a.get_configuration_key())
        } else {
            None
        };
    if let Some(configuration_key) = configuration_key {
        if is_empty_options_key(configuration_key) {
            return false;
        }
        if cleared {
            return true;
        }
        if is_exec_config(configuration_key) {
            return false;
        }
        if configuration_key.is_none() {
            return false;
        }
        return true;
    }
    false
}

/// Returns a description of the analysis-cache affecting changes between the current
/// configuration and the incoming one, or `None` if the configuration has not changed in a way
/// that requires the analysis cache to be invalidated.
// Java: com/google/devtools/build/lib/skyframe/SkyframeBuildView.java:230 SkyframeBuildView.describeConfigurationDifference
fn describe_configuration_difference(
    old_options: &BuildOptions,
    new_options: &BuildOptions,
    max_differences_to_show: i32,
) -> Option<String> {
    let diff = blaze_build_config::options_diff::OptionsDiff::diff(old_options, new_options);

    // getNativeCacheInvalidatingDifferences(configuration, diff): the configuration's options are
    // the new options.
    let mut native_cache_invalidating_differences: Vec<&'static str> = Vec::new();
    for (class, definition) in diff.get_differing_options() {
        let old = diff.get_first().get(definition).cloned().flatten();
        let new = diff
            .get_second()
            .get(definition)
            .and_then(|values| values.first().cloned())
            .flatten();
        if blaze_build_config::fragments::test_configuration::should_invalidate_for_option_diff(
            new_options,
            class,
            *definition,
            &old,
            &new,
        ) && !native_cache_invalidating_differences.contains(&definition.get_option_name())
        {
            native_cache_invalidating_differences.push(definition.get_option_name());
        }
    }
    let changed_starlark_options = diff.get_changed_starlark_options();
    if native_cache_invalidating_differences.is_empty() && changed_starlark_options.is_empty() {
        // The configuration may have changed, but none of the changes required a cache reset.
        // For example, test trimming was turned on and a test option changed. In this case,
        // nothing needs to be done.
        return None;
    }

    if max_differences_to_show == 0 {
        return Some("Build options have changed".to_owned());
    }

    let mut relevant_differences: Vec<String> = changed_starlark_options
        .iter()
        .map(|l| l.get_canonical_form())
        .chain(
            native_cache_invalidating_differences
                .iter()
                .map(|s| (*s).to_owned()),
        )
        .map(|s| format!("--{s}"))
        .collect();
    // Sorting the list to ensure that (if truncated through maxDifferencesToShow) the options in
    // the message remain stable.
    relevant_differences.sort();

    let n = relevant_differences.len();
    if max_differences_to_show > 0 && n > max_differences_to_show as usize {
        let shown = max_differences_to_show as usize;
        Some(format!(
            "Build options {}{} and {} more have changed",
            relevant_differences[..shown].join(", "),
            if max_differences_to_show == 1 {
                ""
            } else {
                ","
            },
            n - shown
        ))
    } else if n == 1 {
        Some(format!(
            "Build option {} has changed",
            relevant_differences[0]
        ))
    } else if n == 2 {
        Some(format!(
            "Build options {} have changed",
            relevant_differences.join(" and ")
        ))
    } else {
        Some(format!(
            "Build options {}, and {} have changed",
            relevant_differences[..n - 1].join(", "),
            relevant_differences[n - 1]
        ))
    }
}
