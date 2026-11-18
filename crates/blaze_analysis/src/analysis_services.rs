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
}

impl AnalysisServices {
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
