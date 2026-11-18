//! Port of `analysis/CachingAnalysisEnvironment.java` (the only implementation of the
//! `AnalysisEnvironment` interface Bazel uses outside tests).

use std::sync::Arc;

use blaze_actions::ArtifactRoot;
use blaze_actions::actions::artifact_factory::ArtifactFactory;
use blaze_actions::{ActionKeyContext, ActionLookupKeyRef, ActionRef, Artifact, ArtifactOwner};
use blaze_cmdline::RepositoryMapping;
use blaze_events::StoredEventHandler;
use blaze_packages::package::package::Package;
use blaze_packages::target::Target;
use blaze_starlark::StarlarkSemantics;
use blaze_util::check_state;
use blaze_vfs::PathFragment;
use indexmap::{IndexMap, IndexSet};
use parking_lot::Mutex;
use starlark::values::OwnedFrozenValue;

/// Java `AnalysisEnvironment` (interface); Blaze has one implementation.
pub type AnalysisEnvironment = CachingAnalysisEnvironment;

/// Thrown in case of a missing build info key (Java `CachingAnalysisEnvironment.MissingDepException`,
/// a RuntimeException).
// Java: com/google/devtools/build/lib/analysis/CachingAnalysisEnvironment.java:383 CachingAnalysisEnvironment.MissingDepException
#[derive(Debug, Clone, thiserror::Error)]
#[error("{0}")]
pub struct MissingDepException(pub String);

/// The workspace status artifacts (Java `WorkspaceStatusValue`), requested by the caller of
/// [`CachingAnalysisEnvironment::new`] before analysis.
#[derive(Clone, Debug)]
pub struct WorkspaceStatusArtifacts {
    pub stable_artifact: Artifact,
    pub volatile_artifact: Artifact,
}

struct State {
    enabled: bool,
    /// Map of artifacts to their origin (Java: to themselves, or to a `Pair<Artifact, String>`
    /// with the creation stack trace under `--experimental_extended_sanity_checks`). The artifact
    /// is stored so that we can deduplicate artifacts created multiple times.
    // PORT: Java's HashMap iteration order is never observable (orphans are sorted).
    artifacts: Option<IndexMap<Artifact, Option<String>>>,
    /// The list of actions registered by the configured target this analysis environment is
    /// responsible for.
    actions: Vec<ActionRef>,
}

/// The implementation of AnalysisEnvironment used for analysis. It tracks metadata for each
/// configured target, such as the errors and warnings emitted by that configured target. It is
/// intended that a separate instance is used for each configured target, so that these don't
/// mix up.
// Java: com/google/devtools/build/lib/analysis/CachingAnalysisEnvironment.java:59 CachingAnalysisEnvironment
pub struct CachingAnalysisEnvironment {
    artifact_factory: Arc<ArtifactFactory>,
    owner: ActionLookupKeyRef,
    extended_sanity_checks: bool,
    allow_analysis_failures: bool,
    action_key_context: Arc<ActionKeyContext>,
    error_event_listener: Arc<StoredEventHandler>,
    /// Java `starlarkBuiltinsValue.starlarkSemantics`.
    starlark_semantics: Arc<StarlarkSemantics>,
    /// Java `starlarkBuiltinsValue.exportedToJava`.
    starlark_defined_builtins: Option<Arc<IndexMap<String, OwnedFrozenValue>>>,
    /// PORT: Java requests `RepositoryMappingValue.key(MAIN)` from the Skyframe environment on
    /// demand; the caller passes the (always already computed) value.
    main_repo_mapping: Option<RepositoryMapping>,
    /// PORT: Java requests `WorkspaceStatusValue.BUILD_INFO_KEY` on demand; the caller passes it.
    workspace_status: Option<WorkspaceStatusArtifacts>,
    /// PORT: Java's `getSkyframeEnv()` gives rule factories the Skyframe environment; the only
    /// use (`ConstraintSetting.validateDefaultConstraintValue`) reads the `PackageValue` of the
    /// current target's package, which is always already computed. The caller passes that
    /// package (see [`CachingAnalysisEnvironment::get_already_loaded_package`]).
    current_package: Option<Arc<Package>>,
    state: Mutex<State>,
}

impl CachingAnalysisEnvironment {
    // Java: com/google/devtools/build/lib/analysis/CachingAnalysisEnvironment.java:87 CachingAnalysisEnvironment.<init>
    pub fn new(
        artifact_factory: Arc<ArtifactFactory>,
        action_key_context: Arc<ActionKeyContext>,
        owner: ActionLookupKeyRef,
        extended_sanity_checks: bool,
        allow_analysis_failures: bool,
        error_event_listener: Arc<StoredEventHandler>,
        starlark_semantics: Arc<StarlarkSemantics>,
        starlark_defined_builtins: Option<Arc<IndexMap<String, OwnedFrozenValue>>>,
        main_repo_mapping: Option<RepositoryMapping>,
        workspace_status: Option<WorkspaceStatusArtifacts>,
        current_package: Option<Arc<Package>>,
    ) -> CachingAnalysisEnvironment {
        CachingAnalysisEnvironment {
            artifact_factory,
            owner,
            extended_sanity_checks,
            allow_analysis_failures,
            action_key_context,
            error_event_listener,
            starlark_semantics,
            starlark_defined_builtins,
            main_repo_mapping,
            workspace_status,
            current_package,
            state: Mutex::new(State {
                enabled: true,
                artifacts: Some(IndexMap::new()),
                actions: Vec::new(),
            }),
        }
    }

    // Java: com/google/devtools/build/lib/analysis/CachingAnalysisEnvironment.java:107 CachingAnalysisEnvironment.disable
    pub fn disable(&self, target: &Target) {
        if !self.has_errors() && !self.allow_analysis_failures {
            self.verify_generated_artifact_have_actions(target);
        }
        let mut state = self.state.lock();
        state.artifacts = None;
        state.enabled = false;
        // PORT: Java also nulls the event listener and the Skyframe environment; Rust keeps the
        // shared handle (the caller replays it).
    }

    // Java: com/google/devtools/build/lib/analysis/CachingAnalysisEnvironment.java:117 CachingAnalysisEnvironment.shortDescription
    fn short_description(action: &ActionRef) -> String {
        // PORT: Java prints `action.getClass().getName()`; the Rust type name stands in.
        format!(
            "{} {}",
            std::any::type_name_of_val(action.as_ref()),
            action.get_mnemonic()
        )
    }

    /// Sanity checks that all generated artifacts have a generating action.
    // Java: com/google/devtools/build/lib/analysis/CachingAnalysisEnvironment.java:132 CachingAnalysisEnvironment.verifyGeneratedArtifactHaveActions
    pub fn verify_generated_artifact_have_actions(&self, target: &Target) {
        let orphan_artifacts: Vec<String> = self
            .get_orphan_artifact_map()
            .into_iter()
            .map(|(_, v)| v)
            .collect();
        if !orphan_artifacts.is_empty() {
            let state = self.state.lock();
            let mut checked_actions = Vec::with_capacity(state.actions.len());
            for action in &state.actions {
                let mut sb = Self::short_description(action);
                for o in action.get_outputs() {
                    sb.push_str("\n    ");
                    sb.push_str(o.get_exec_path_string());
                }
                checked_actions.push(sb);
            }
            panic!(
                "IllegalStateException: {} {} : These artifacts do not have a generating action:\n{}\nThese actions were checked:\n{}\n",
                target.get_target_kind(),
                target.get_label(),
                orphan_artifacts.join("\n"),
                checked_actions.join("\n")
            );
        }
    }

    // Java: com/google/devtools/build/lib/analysis/CachingAnalysisEnvironment.java:157 CachingAnalysisEnvironment.getOrphanArtifacts
    pub fn get_orphan_artifacts(&self) -> IndexSet<Artifact> {
        self.get_orphan_artifact_map()
            .into_iter()
            .map(|(a, _)| a)
            .collect()
    }

    // Java: com/google/devtools/build/lib/analysis/CachingAnalysisEnvironment.java:162 CachingAnalysisEnvironment.getTreeArtifactsConflictingWithFiles
    pub fn get_tree_artifacts_conflicting_with_files(&self) -> IndexSet<Artifact> {
        let state = self.state.lock();
        let artifacts = state.artifacts.as_ref().expect("artifacts");
        let has_tree_artifacts = artifacts.keys().any(|a| a.is_tree_artifact());
        if !has_tree_artifacts {
            return IndexSet::new();
        }
        let mut collect: std::collections::HashSet<PathFragment> = std::collections::HashSet::new();
        for artifact in artifacts.keys() {
            if !artifact.is_source_artifact() && !artifact.is_tree_artifact() {
                collect.insert(artifact.get_exec_path().clone());
            }
        }
        let mut same_exec_path_tree_artifacts = IndexSet::new();
        for artifact in artifacts.keys() {
            if artifact.is_tree_artifact() && collect.contains(artifact.get_exec_path()) {
                same_exec_path_tree_artifacts.insert(artifact.clone());
            }
        }
        same_exec_path_tree_artifacts
    }

    // Java: com/google/devtools/build/lib/analysis/CachingAnalysisEnvironment.java:189 CachingAnalysisEnvironment.getOrphanArtifactMap
    fn get_orphan_artifact_map(&self) -> Vec<(Artifact, String)> {
        let state = self.state.lock();
        // Construct this set to avoid poor performance under large --runs_per_test.
        let mut artifacts_with_actions: std::collections::HashSet<Artifact> =
            std::collections::HashSet::new();
        for action in &state.actions {
            // Don't bother checking that every Artifact only appears once; that test is performed
            // elsewhere (see #testNonUniqueOutputs in ActionListenerIntegrationTest).
            artifacts_with_actions.extend(action.get_outputs());
        }
        // The order of the artifacts.entrySet iteration is unspecified - we use a TreeMap here to
        // guarantee that the return value of this method is deterministic.
        let mut orphan_artifacts: Vec<(Artifact, String)> = Vec::new();
        if let Some(artifacts) = &state.artifacts {
            for (a, origin) in artifacts {
                if !a.is_source_artifact() && !artifacts_with_actions.contains(a) {
                    let value = match origin {
                        None => {
                            "No origin, run with --experimental_extended_sanity_checks".to_owned()
                        }
                        Some(stack) => stack.clone(),
                    };
                    orphan_artifacts.push((
                        a.clone(),
                        format!(
                            "{}\n{}",
                            a.get_exec_path_string(), // uncovered artifact
                            value
                        ), // origin of creation
                    ));
                }
            }
        }
        orphan_artifacts.sort_by(|x, y| Artifact::exec_path_comparator(&x.0, &y.0));
        orphan_artifacts.dedup_by(|x, y| {
            Artifact::exec_path_comparator(&x.0, &y.0) == std::cmp::Ordering::Equal
        });
        orphan_artifacts
    }

    // Java: com/google/devtools/build/lib/analysis/CachingAnalysisEnvironment.java:218 CachingAnalysisEnvironment.getEventHandler
    pub fn get_event_handler(&self) -> &Arc<StoredEventHandler> {
        &self.error_event_listener
    }

    // Java: com/google/devtools/build/lib/analysis/CachingAnalysisEnvironment.java:223 CachingAnalysisEnvironment.getActionKeyContext
    pub fn get_action_key_context(&self) -> &Arc<ActionKeyContext> {
        &self.action_key_context
    }

    // Java: com/google/devtools/build/lib/analysis/CachingAnalysisEnvironment.java:228 CachingAnalysisEnvironment.hasErrors
    pub fn has_errors(&self) -> bool {
        check_state!(self.state.lock().enabled);
        self.error_event_listener.has_errors()
    }

    /// Keeps track of artifacts. We check that all of them have an owner when the environment is
    /// sealed (disable()). For performance reasons we only track the originating stacktrace when
    /// running with --experimental_extended_sanity_checks.
    // Java: com/google/devtools/build/lib/analysis/CachingAnalysisEnvironment.java:240 CachingAnalysisEnvironment.dedupAndTrackArtifactAndOrigin
    fn dedup_and_track_artifact_and_origin(&self, a: Artifact) -> Artifact {
        let mut state = self.state.lock();
        let artifacts = state.artifacts.as_mut().expect("artifacts");
        if let Some((existing, _)) = artifacts.get_key_value(&a) {
            return existing.clone();
        }
        let origin = if self.extended_sanity_checks {
            // PORT: Java records the creation stack trace of a Throwable.
            Some(std::backtrace::Backtrace::force_capture().to_string())
        } else {
            None
        };
        artifacts.insert(a.clone(), origin);
        a
    }

    fn owner(&self) -> ArtifactOwner {
        ArtifactOwner::ActionLookupKey(self.owner.clone())
    }

    // Java: com/google/devtools/build/lib/analysis/CachingAnalysisEnvironment.java:262 CachingAnalysisEnvironment.getDerivedArtifact
    pub fn get_derived_artifact(
        &self,
        root_relative_path: &PathFragment,
        root: &ArtifactRoot,
    ) -> Artifact {
        check_state!(self.state.lock().enabled);
        self.dedup_and_track_artifact_and_origin(self.artifact_factory.get_derived_artifact(
            root_relative_path,
            root,
            self.owner(),
        ))
    }

    // Java: com/google/devtools/build/lib/analysis/CachingAnalysisEnvironment.java:271 CachingAnalysisEnvironment.getRunfilesArtifact
    pub fn get_runfiles_artifact(
        &self,
        root_relative_path: &PathFragment,
        root: &ArtifactRoot,
    ) -> Artifact {
        check_state!(self.state.lock().enabled);
        self.dedup_and_track_artifact_and_origin(self.artifact_factory.get_runfiles_artifact(
            root_relative_path,
            root,
            self.owner(),
        ))
    }

    // Java: com/google/devtools/build/lib/analysis/CachingAnalysisEnvironment.java:280 CachingAnalysisEnvironment.getTreeArtifact
    pub fn get_tree_artifact(
        &self,
        root_relative_path: &PathFragment,
        root: &ArtifactRoot,
    ) -> Artifact {
        check_state!(self.state.lock().enabled);
        self.dedup_and_track_artifact_and_origin(self.artifact_factory.get_tree_artifact(
            root_relative_path,
            root,
            self.owner(),
        ))
    }

    // Java: com/google/devtools/build/lib/analysis/CachingAnalysisEnvironment.java:289 CachingAnalysisEnvironment.getSymlinkArtifact
    pub fn get_symlink_artifact(
        &self,
        root_relative_path: &PathFragment,
        root: &ArtifactRoot,
    ) -> Artifact {
        check_state!(self.state.lock().enabled);
        self.dedup_and_track_artifact_and_origin(self.artifact_factory.get_symlink_artifact(
            root_relative_path,
            root,
            self.owner(),
        ))
    }

    // Java: com/google/devtools/build/lib/analysis/CachingAnalysisEnvironment.java:298 CachingAnalysisEnvironment.getFilesetArtifact
    pub fn get_fileset_artifact(
        &self,
        root_relative_path: &PathFragment,
        root: &ArtifactRoot,
    ) -> Artifact {
        check_state!(self.state.lock().enabled);
        self.dedup_and_track_artifact_and_origin(self.artifact_factory.get_fileset_artifact(
            root_relative_path,
            root,
            self.owner(),
        ))
    }

    // Java: com/google/devtools/build/lib/analysis/CachingAnalysisEnvironment.java:307 CachingAnalysisEnvironment.getConstantMetadataArtifact
    pub fn get_constant_metadata_artifact(
        &self,
        root_relative_path: &PathFragment,
        root: &ArtifactRoot,
    ) -> Artifact {
        self.artifact_factory
            .get_constant_metadata_artifact(root_relative_path, root, self.owner())
    }

    // Java: com/google/devtools/build/lib/analysis/CachingAnalysisEnvironment.java:312 CachingAnalysisEnvironment.registerAction
    pub fn register_action(&self, action: ActionRef) {
        let mut state = self.state.lock();
        check_state!(state.enabled);
        state.actions.push(action);
    }

    // Java: com/google/devtools/build/lib/analysis/CachingAnalysisEnvironment.java:318 CachingAnalysisEnvironment.getLocalGeneratingAction
    pub fn get_local_generating_action(&self, artifact: &Artifact) -> Option<ActionRef> {
        let state = self.state.lock();
        state
            .actions
            .iter()
            .find(|action| action.get_outputs().contains(artifact))
            .cloned()
    }

    // Java: com/google/devtools/build/lib/analysis/CachingAnalysisEnvironment.java:329 CachingAnalysisEnvironment.getRegisteredActions
    pub fn get_registered_actions(&self) -> Vec<ActionRef> {
        self.state.lock().actions.clone()
    }

    // Java: com/google/devtools/build/lib/analysis/CachingAnalysisEnvironment.java:339 CachingAnalysisEnvironment.getStarlarkSemantics
    pub fn get_starlark_semantics(&self) -> &Arc<StarlarkSemantics> {
        &self.starlark_semantics
    }

    /// Returns the `@_builtins`-defined `exported_to_java` symbols.
    // Java: com/google/devtools/build/lib/analysis/CachingAnalysisEnvironment.java:344 CachingAnalysisEnvironment.getStarlarkDefinedBuiltins
    pub fn get_starlark_defined_builtins(&self) -> &IndexMap<String, OwnedFrozenValue> {
        match &self.starlark_defined_builtins {
            Some(b) => b,
            None => blaze_util::unported!("CachingAnalysisEnvironment.getStarlarkDefinedBuiltins"),
        }
    }

    // Java: com/google/devtools/build/lib/analysis/CachingAnalysisEnvironment.java:349 CachingAnalysisEnvironment.getStableWorkspaceStatusArtifact
    pub fn get_stable_workspace_status_artifact(&self) -> Result<Artifact, MissingDepException> {
        Ok(self.get_workspace_status_value()?.stable_artifact.clone())
    }

    // Java: com/google/devtools/build/lib/analysis/CachingAnalysisEnvironment.java:354 CachingAnalysisEnvironment.getVolatileWorkspaceStatusArtifact
    pub fn get_volatile_workspace_status_artifact(&self) -> Result<Artifact, MissingDepException> {
        Ok(self.get_workspace_status_value()?.volatile_artifact.clone())
    }

    // Java: com/google/devtools/build/lib/analysis/CachingAnalysisEnvironment.java:359 CachingAnalysisEnvironment.getWorkspaceStatusValue
    fn get_workspace_status_value(&self) -> Result<&WorkspaceStatusArtifacts, MissingDepException> {
        self.workspace_status
            .as_ref()
            .ok_or_else(|| MissingDepException("Restart due to missing build info".to_owned()))
    }

    // Java: com/google/devtools/build/lib/analysis/CachingAnalysisEnvironment.java:368 CachingAnalysisEnvironment.getMainRepoMapping
    pub fn get_main_repo_mapping(&self) -> Result<&RepositoryMapping, MissingDepException> {
        // This isn't expected to happen since the main repository mapping is computed before the
        // analysis phase.
        self.main_repo_mapping.as_ref().ok_or_else(|| {
            MissingDepException("Restart due to missing main repository mapping".to_owned())
        })
    }

    // Java: com/google/devtools/build/lib/analysis/CachingAnalysisEnvironment.java:379 CachingAnalysisEnvironment.getOwner
    /// Java `getSkyframeEnv().getValue(packageIdentifier)` for the current target's own package
    /// (already loaded, so never missing).
    // Java: com/google/devtools/build/lib/analysis/CachingAnalysisEnvironment.java:334 CachingAnalysisEnvironment.getSkyframeEnv
    pub fn get_already_loaded_package(
        &self,
        package_identifier: &blaze_cmdline::PackageIdentifier,
    ) -> &Arc<Package> {
        match &self.current_package {
            Some(package) if package.get_package_identifier() == package_identifier => package,
            _ => blaze_util::unported!(
                "CachingAnalysisEnvironment.getSkyframeEnv",
                "package {} is not the current target's package",
                package_identifier
            ),
        }
    }

    pub fn get_owner(&self) -> &ActionLookupKeyRef {
        &self.owner
    }
}
