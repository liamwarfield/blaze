//! Execution of `lib/actions/RunfilesTreeAction.java`, and the adapter of
//! `blaze_actions::RunfilesTree` to the execution layer's `RunfilesTree`.

use std::collections::HashMap;
use std::sync::Arc;

use blaze_actions::actions::runfiles_tree::RunfilesTreeRef;
use blaze_actions::actions::runfiles_tree_action::RunfilesTreeAction;
use blaze_actions::analysis::actions::symlink_tree_action::RunfileSymlinksMode;
use blaze_actions::analysis::symlink_entry::SymlinkEntry as BlazeSymlinkEntry;
use blaze_collect::{NestedSet, NestedSetBuilder, NestedSetNode};
use blaze_vfs::PathFragment;

use super::to_action_input_set;
use crate::actions::action_adapters::artifact_input;
use blaze_actions::actions::action::Action;

use crate::actions::{
    ActionExecutionException, ActionInputRef, ActionResult, ExecutableAction, FileArtifactValue,
    InputMetadataProvider, RunfilesArtifactValue, RunfilesTree, SymlinkEntry, TreeArtifactValue,
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

    // Java: com/google/devtools/build/lib/actions/RunfilesTree.java:44 RunfilesTree.getSymlinksMode
    fn get_symlinks_mode(&self) -> RunfileSymlinksMode {
        self.0.get_symlinks_mode()
    }

    // Java: com/google/devtools/build/lib/actions/RunfilesTree.java:47 RunfilesTree.isBuildRunfileLinks
    fn is_build_runfile_links(&self) -> bool {
        self.0.is_build_runfile_links()
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
    fn get_symlinks_for_logging(&self) -> NestedSet<SymlinkEntry> {
        to_symlink_entry_set(&self.0.get_symlinks_for_logging())
    }

    // Java: com/google/devtools/build/lib/actions/RunfilesTree.java:71 RunfilesTree.getRootSymlinksForLogging
    fn get_root_symlinks_for_logging(&self) -> NestedSet<SymlinkEntry> {
        to_symlink_entry_set(&self.0.get_root_symlinks_for_logging())
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

/// Converts a `NestedSet<SymlinkEntry>` of `blaze_actions` into the execution layer's, keeping the
/// node structure (and sharing of sub-sets within the set) so that the compact execution log
/// writes one `SymlinkEntrySet` per node, as Java does (Java passes the same object).
///
/// PORT: the nodes of two conversions differ, so a sub-set shared by two runfiles trees is logged
/// once per tree (Java: once), as for [`to_action_input_set`].
// `NestedSetNode` hashes and compares by node identity (Java `Node.equals`).
#[allow(clippy::mutable_key_type)]
fn to_symlink_entry_set(set: &NestedSet<BlazeSymlinkEntry>) -> NestedSet<SymlinkEntry> {
    let mut memo: HashMap<NestedSetNode<BlazeSymlinkEntry>, NestedSet<SymlinkEntry>> =
        HashMap::new();
    convert_symlink_entry_set(set, &mut memo)
}

#[allow(clippy::mutable_key_type)]
fn convert_symlink_entry_set(
    set: &NestedSet<BlazeSymlinkEntry>,
    memo: &mut HashMap<NestedSetNode<BlazeSymlinkEntry>, NestedSet<SymlinkEntry>>,
) -> NestedSet<SymlinkEntry> {
    let convert = |e: &BlazeSymlinkEntry| SymlinkEntry {
        path: e.get_path_string().to_string(),
        artifact: artifact_input(e.get_artifact()),
    };
    let node = set.to_node();
    if let Some(s) = memo.get(&node) {
        return s.clone();
    }
    let order = set.get_order();
    let converted = if set.is_empty() {
        NestedSetBuilder::empty_set(order)
    } else if set.is_singleton() {
        NestedSetBuilder::create(order, [convert(set.get_singleton())])
    } else {
        // See `convert_set` in `analysis/actions/mod.rs` for the layout of a rebuilt node.
        let mut b = NestedSetBuilder::new_builder(order);
        for child in set.get_non_leaves() {
            let c = convert_symlink_entry_set(&child, memo);
            b.add_transitive(&c);
        }
        for leaf in set.get_leaves() {
            b.add(convert(&leaf));
        }
        b.build()
    };
    memo.insert(node, converted.clone());
    converted
}

/// `createRunfilesArtifactValue(inputMetadataProvider)`: the runfiles tree with the metadata of
/// every input of the action (files and trees, sorted by exec path).
// Java: com/google/devtools/build/lib/actions/RunfilesTreeAction.java:62 RunfilesTreeAction.createRunfilesArtifactValue
pub fn create_runfiles_artifact_value(
    action: &RunfilesTreeAction,
    input_metadata_provider: &dyn InputMetadataProvider,
) -> std::io::Result<RunfilesArtifactValue> {
    let inputs = action.get_inputs().to_list();
    let mut files: Vec<ActionInputRef> = Vec::new();
    let mut file_values: Vec<FileArtifactValue> = Vec::new();
    let mut trees: Vec<ActionInputRef> = Vec::new();
    let mut tree_values: Vec<Arc<TreeArtifactValue>> = Vec::new();

    // Sort for better equality in RunfilesArtifactValue.
    let mut sorted_inputs: Vec<blaze_actions::Artifact> = inputs.to_vec();
    sorted_inputs.sort_by(blaze_actions::Artifact::exec_path_comparator);
    for input in &sorted_inputs {
        let input_ref = artifact_input(input);
        if input.is_fileset() {
            blaze_util::unported!(
                "RunfilesTreeAction.createRunfilesArtifactValue (unreachable in Bazel: only the non-open-sourced fileset rule makes fileset artifacts, packages/OutputFile.java:123)",
                "fileset {} in a runfiles tree (FilesetOutputTree)",
                input
            );
        } else if input.is_tree_artifact() {
            // ImmutableList.Builder.add: a null element is a NullPointerException.
            let tree_value = input_metadata_provider
                .get_tree_metadata(&input_ref)
                .unwrap_or_else(|| panic!("NullPointerException: tree metadata of {input}"));
            trees.push(input_ref);
            tree_values.push(tree_value);
        } else {
            let file_value = input_metadata_provider
                .get_input_metadata(&input_ref)?
                .unwrap_or_else(|| panic!("NullPointerException: metadata of {input}"));
            files.push(input_ref);
            file_values.push(file_value);
        }
    }

    Ok(RunfilesArtifactValue::new(
        Arc::new(RunfilesTreeAdapter(action.get_runfiles_tree().clone())),
        files,
        file_values,
        trees,
        tree_values,
    ))
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
        match create_runfiles_artifact_value(self, ctx.get_input_metadata_provider()) {
            Ok(runfiles_artifact_value) => {
                ctx.set_rich_artifact_data(Arc::new(runfiles_artifact_value));
            }
            Err(e) => panic!("IllegalStateException: {e}"),
        }
        Ok(ActionResult::empty())
    }
}
