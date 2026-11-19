//! What analysis SkyFunctions read from the `SkyframeExecutor` / `SkyframeBuildView` (Java: the
//! fields of `SkyframeBuildView`, `ConfiguredRuleClassProvider`, `BlazeDirectories` and the
//! precomputed values `ConfiguredTargetFunction` and `ConfiguredTargetFactory` use).
//!
//! `blaze_sky_top` builds one [`AnalysisServices`] per command; the analysis functions capture
//! it in their registration closures (the evaluator's single client is the configuration
//! functions' `ConfigurationEnvironment`).

use std::sync::Arc;

use blaze_actions::ActionKeyContext;
use blaze_actions::actions::artifact_factory::ArtifactFactory;
use blaze_actions::{ActionLookupData, ActionLookupKeyRef, ArtifactOwner, ArtifactRoot, RootType};
use blaze_vfs::PathFragment;

use crate::rule::analysis_environment::WorkspaceStatusArtifacts;
use blaze_cmdline::RepositoryMapping;
use blaze_packages::builtins::BazelStarlarkEnvironment;
use blaze_starlark::StarlarkSemantics;
use blaze_vfs::Path;
use parking_lot::RwLock;

use crate::configured_rule_class_provider::ConfiguredRuleClassProvider;

/// Directory facts analysis needs (Java `BlazeDirectories`).
#[derive(Clone, Debug)]
pub struct AnalysisDirectories {
    pub workspace: Path,
    pub output_base: Path,
    /// `<output_base>/execroot/_main`.
    pub exec_root: Path,
    /// Java `getWorkspaceName()` / `ruleClassProvider.getRunfilesPrefix()` (`_main`).
    pub workspace_name: String,
    /// `bazel` (`BlazeDirectories.getProductName`).
    pub product_name: String,
}

/// Services shared by every analysis SkyFunction of one command.
pub struct AnalysisServices {
    pub rule_class_provider: Arc<ConfiguredRuleClassProvider>,
    /// Java `ruleClassProvider.getBazelStarlarkEnvironment()` (the analysis-capable one).
    pub bazel_starlark_environment: Arc<BazelStarlarkEnvironment>,
    /// Java `SkyframeBuildView.artifactFactory` (shared across configured targets).
    pub artifact_factory: Arc<ArtifactFactory>,
    pub action_key_context: Arc<ActionKeyContext>,
    /// `PrecomputedValue.STARLARK_SEMANTICS`.
    pub starlark_semantics: Arc<StarlarkSemantics>,
    pub directories: AnalysisDirectories,
    /// The main repository's mapping (Java `getMainRepoMapping`), filled once loading synced.
    pub main_repo_mapping: RwLock<Option<RepositoryMapping>>,
    /// Java `--keep_going`.
    pub keep_going: bool,
    /// The workspace status artifacts (Java `WorkspaceStatusValue`, computed once per build by
    /// `WorkspaceStatusFunction`); see [`AnalysisServices::get_workspace_status_artifacts`].
    pub workspace_status: std::sync::OnceLock<WorkspaceStatusArtifacts>,
    /// What `BazelStatusActionFactory` passes to the `BazelWorkspaceStatusAction`: the client's
    /// user name (`StandardSystemProperty.USER_NAME`) and `NetUtil.getCachedShortHostName()`.
    pub workspace_status_user_name: String,
    pub workspace_status_host_name: String,
    /// Java `AspectFunction.baseTargetPrerequisitesSupplier`
    /// (`SkyframeExecutor.BaseTargetPrerequisitesSupplierImpl`): set by `blaze_sky_top` once
    /// its evaluator exists; see [`AnalysisServices::get_base_target_prerequisites_supplier`].
    pub base_target_prerequisites_supplier: std::sync::OnceLock<
        crate::base_target_prerequisites_supplier::BaseTargetPrerequisitesSupplier,
    >,
}

/// `WorkspaceStatusValue.BUILD_INFO_KEY` as a Skyframe key: the node of the workspace status
/// action (`WorkspaceStatusFunction`), requested by analysis when a rule reads the workspace
/// status artifacts (`CachingAnalysisEnvironment.getWorkspaceStatusValue`).
// Java: com/google/devtools/build/lib/skyframe/WorkspaceStatusValue.java:55 WorkspaceStatusValue.BuildInfoKey
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct WorkspaceStatusKey;

impl blaze_skyframe::Key for WorkspaceStatusKey {
    type Value = WorkspaceStatusValue;
    // Java: com/google/devtools/build/lib/skyframe/SkyFunctions.java SkyFunctions.BUILD_INFO
    const FUNCTION: blaze_skyframe::SkyFunctionName = blaze_skyframe::SkyFunctionName(
        blaze_actions::analysis::workspace_status_action::BUILD_INFO_FUNCTION_NAME,
    );
}

/// Value that stores the workspace status artifacts and their generating action. There should be
/// only one of these values in the graph at any time.
// Java: com/google/devtools/build/lib/skyframe/WorkspaceStatusValue.java:33 WorkspaceStatusValue
#[derive(Debug)]
pub struct WorkspaceStatusValue {
    /// `BasicActionLookupValue.actions`: the single `WorkspaceStatusAction`.
    pub actions: Vec<blaze_actions::ActionRef>,
    pub stable_artifact: blaze_actions::Artifact,
    pub volatile_artifact: blaze_actions::Artifact,
}

impl AnalysisServices {
    /// The supplier `AspectFunction` passes to `DependencyResolver.computeDependencies` and to
    /// the `UnloadedToolchainContextsProducer` of the base target.
    // Java: com/google/devtools/build/lib/skyframe/AspectFunction.java:168 AspectFunction.baseTargetPrerequisitesSupplier
    pub fn get_base_target_prerequisites_supplier(
        &self,
    ) -> &crate::base_target_prerequisites_supplier::BaseTargetPrerequisitesSupplier {
        self.base_target_prerequisites_supplier
            .get()
            .expect("NullPointerException: baseTargetPrerequisitesSupplier (set by blaze_sky_top)")
    }

    /// `WorkspaceStatusFunction.compute`: the workspace status action over the artifacts of
    /// [`AnalysisServices::get_workspace_status_artifacts`] (which carry its generating action key).
    // Java: com/google/devtools/build/lib/skyframe/WorkspaceStatusFunction.java:38 WorkspaceStatusFunction.compute
    pub fn compute_workspace_status_value(&self) -> WorkspaceStatusValue {
        let artifacts = self.get_workspace_status_artifacts();
        // Java: com/google/devtools/build/lib/bazel/BazelWorkspaceStatusModule.java:278 BazelStatusActionFactory.createWorkspaceStatusAction
        let action: blaze_actions::ActionRef = Arc::new(
            blaze_actions::bazel::bazel_workspace_status_module::BazelWorkspaceStatusAction::new(
                artifacts.stable_artifact.clone(),
                artifacts.volatile_artifact.clone(),
                self.workspace_status_user_name.clone(),
                self.workspace_status_host_name.clone(),
            ),
        );
        WorkspaceStatusValue {
            actions: vec![action],
            stable_artifact: artifacts.stable_artifact.clone(),
            volatile_artifact: artifacts.volatile_artifact.clone(),
        }
    }

    /// Java `env.getValue(WorkspaceStatusValue.BUILD_INFO_KEY)`: the stable and volatile
    /// workspace status artifacts.
    ///
    /// PORT: Java's `WorkspaceStatusFunction` asks `SkyframeExecutor.createWorkspaceStatusAction`
    /// for the action (`BazelStatusActionFactory`, which names the files) and keeps its outputs.
    /// Analysis reads only the two artifacts, so they are made here exactly as the action
    /// factory's environment makes them; the action (it needs the client's user and host) is
    /// the build command's.
    // Java: com/google/devtools/build/lib/skyframe/WorkspaceStatusFunction.java:38 WorkspaceStatusFunction.compute
    pub fn get_workspace_status_artifacts(&self) -> &WorkspaceStatusArtifacts {
        self.workspace_status.get_or_init(|| {
            // Java: com/google/devtools/build/lib/skyframe/SkyframeExecutor.java:1491 SkyframeExecutor.createWorkspaceStatusAction
            // (directories.getBuildDataDirectory(ruleClassProvider.getRunfilesPrefix())).
            let root = ArtifactRoot::as_derived_root(
                &self.directories.exec_root,
                RootType::OUTPUT,
                &[&format!("{}-out", self.directories.product_name)],
            );
            let owner = || {
                ArtifactOwner::ActionLookupKey(ActionLookupKeyRef::new(
                    blaze_actions::analysis::workspace_status_action::BUILD_INFO_KEY,
                ))
            };
            // Java: com/google/devtools/build/lib/bazel/BazelWorkspaceStatusModule.java:278 BazelStatusActionFactory.createWorkspaceStatusAction
            let stable_artifact = self.artifact_factory.get_derived_artifact(
                &PathFragment::create("stable-status.txt"),
                &root,
                owner(),
            );
            let volatile_artifact = self.artifact_factory.get_constant_metadata_artifact(
                &PathFragment::create("volatile-status.txt"),
                &root,
                owner(),
            );
            let generating_action_key = ActionLookupData::create(
                ActionLookupKeyRef::new(
                    blaze_actions::analysis::workspace_status_action::BUILD_INFO_KEY,
                ),
                0,
            );
            for output in [&stable_artifact, &volatile_artifact] {
                output.set_generating_action_key(generating_action_key.clone());
            }
            WorkspaceStatusArtifacts {
                stable_artifact,
                volatile_artifact,
            }
        })
    }
}
