//! Execution of `lib/actions/RunfilesTreeAction.java`, and the adapter of
//! `blaze_actions::RunfilesTree` to the execution layer's `RunfilesTree`.

use std::sync::Arc;

use blaze_actions::actions::runfiles_tree::RunfilesTreeRef;
use blaze_actions::actions::runfiles_tree_action::RunfilesTreeAction;
use blaze_collect::NestedSet;
use blaze_vfs::PathFragment;

use super::to_action_input_set;
use crate::actions::action_adapters::artifact_input;
use crate::actions::{
    ActionExecutionException, ActionInputRef, ActionResult, ExecutableAction,
    RunfilesArtifactValue, RunfilesTree,
};
use crate::exec::action_execution_context::ActionExecutionContext;

/// A `blaze_actions` runfiles tree (`RunfilesSupport.RunfilesTreeImpl` & co.) as the execution
/// layer's [`RunfilesTree`]: artifacts become [`ActionInputRef`]s.
///
/// Not a port of a Java class: Java has one `RunfilesTree` interface.
#[derive(Debug, Clone)]
pub struct RunfilesTreeAdapter(pub RunfilesTreeRef);

impl RunfilesTree for RunfilesTreeAdapter {
    // Java: com/google/devtools/build/lib/actions/RunfilesTree.java:30 RunfilesTree.getExecPath
    fn get_exec_path(&self) -> PathFragment {
        self.0.get_exec_path()
    }

    // Java: com/google/devtools/build/lib/actions/RunfilesTree.java:33 RunfilesTree.getMapping
    fn get_mapping(&self) -> Vec<(PathFragment, Option<ActionInputRef>)> {
        self.0
            .get_mapping()
            .iter()
            .map(|(path, artifact)| (path.clone(), artifact.as_ref().map(artifact_input)))
            .collect()
    }

    // Java: com/google/devtools/build/lib/actions/RunfilesTree.java:50 RunfilesTree.getWorkspaceName
    fn get_workspace_name(&self) -> String {
        self.0.get_workspace_name()
    }

    // Java: com/google/devtools/build/lib/actions/RunfilesTree.java:78 RunfilesTree.isMappingCached
    fn is_mapping_cached(&self) -> bool {
        self.0.is_mapping_cached()
    }

    // Java: com/google/devtools/build/lib/actions/RunfilesTree.java:57 RunfilesTree.getArtifactsAtCanonicalLocationsForLogging
    fn get_artifacts_at_canonical_locations_for_logging(&self) -> NestedSet<ActionInputRef> {
        to_action_input_set(&self.0.get_artifacts_at_canonical_locations_for_logging())
    }

    // Java: com/google/devtools/build/lib/actions/RunfilesTree.java:68 RunfilesTree.getSymlinksForLogging
    fn get_symlinks_for_logging(&self) -> Vec<(String, ActionInputRef)> {
        self.0
            .get_symlinks_for_logging()
            .to_list()
            .iter()
            .map(|e| {
                (
                    e.get_path_string().to_string(),
                    artifact_input(e.get_artifact()),
                )
            })
            .collect()
    }

    // Java: com/google/devtools/build/lib/actions/RunfilesTree.java:71 RunfilesTree.getRootSymlinksForLogging
    fn get_root_symlinks_for_logging(&self) -> Vec<(String, ActionInputRef)> {
        self.0
            .get_root_symlinks_for_logging()
            .to_list()
            .iter()
            .map(|e| {
                (
                    e.get_path_string().to_string(),
                    artifact_input(e.get_artifact()),
                )
            })
            .collect()
    }

    // Java: com/google/devtools/build/lib/actions/RunfilesTree.java:65 RunfilesTree.getEmptyFilenamesForLogging
    fn get_empty_filenames_for_logging(&self) -> Vec<PathFragment> {
        self.0.get_empty_filenames_for_logging()
    }

    // Java: com/google/devtools/build/lib/actions/RunfilesTree.java:75 RunfilesTree.getRepoMappingManifestForLogging
    fn get_repo_mapping_manifest_for_logging(&self) -> Option<ActionInputRef> {
        self.0
            .get_repo_mapping_manifest_for_logging()
            .as_ref()
            .map(artifact_input)
    }
}

/// `createRunfilesArtifactValue(inputMetadataProvider)`.
///
/// PORT: Java also collects the metadata of every input (files, trees and filesets, sorted by
/// exec path) into the value; the execution layer's `RunfilesArtifactValue` seam holds only the
/// runfiles tree, and the executor that stores the value owns the input metadata.
// Java: com/google/devtools/build/lib/actions/RunfilesTreeAction.java:62 RunfilesTreeAction.createRunfilesArtifactValue
pub fn create_runfiles_artifact_value(action: &RunfilesTreeAction) -> RunfilesArtifactValue {
    RunfilesArtifactValue {
        runfiles_tree: Arc::new(RunfilesTreeAdapter(action.get_runfiles_tree().clone())),
    }
}

impl ExecutableAction for RunfilesTreeAction {
    // Java: com/google/devtools/build/lib/actions/RunfilesTreeAction.java:122 RunfilesTreeAction.prepare
    fn prepare(&self, _exec_root: &blaze_vfs::Path) -> std::io::Result<()> {
        // Runfiles trees are created as a side effect of building the output manifest, not the
        // runfiles tree artifact. This method is overridden so that depending on the runfiles
        // tree does not delete the runfiles tree that's on the file system that someone decided
        // it must be there.
        Ok(())
    }

    // Java: com/google/devtools/build/lib/actions/RunfilesTreeAction.java:111 RunfilesTreeAction.execute
    fn execute(
        &self,
        ctx: &dyn ActionExecutionContext,
    ) -> Result<ActionResult, ActionExecutionException> {
        let runfiles_artifact_value = create_runfiles_artifact_value(self);
        ctx.set_rich_artifact_data(Arc::new(runfiles_artifact_value));
        Ok(ActionResult::empty())
    }
}
