//! `InputMetadataProvider` and the input-side values it returns (`TreeArtifactValue`,
//! `RunfilesArtifactValue`, `RunfilesTree`, `FilesetOutputTree`), as far as the execution layer
//! reads them. Seam with `blaze_actions` / `blaze_sky_exec`.

use std::collections::{BTreeMap, HashSet};
use std::fmt;
use std::io;
use std::sync::Arc;

use blaze_collect::NestedSet;
use blaze_vfs::PathFragment;

use super::action_input::ActionInputRef;
use super::file_artifact_value::FileArtifactValue;

/// Port of `InputMetadataProvider`: metadata of the inputs of the action being executed.
// Java: com/google/devtools/build/lib/actions/InputMetadataProvider.java:31 InputMetadataProvider
pub trait InputMetadataProvider: Send + Sync {
    /// `getInputMetadata(ActionInput)`: `Ok(None)` when unknown (for example an output of an
    /// earlier spawn of the same action).
    fn get_input_metadata(&self, input: &ActionInputRef) -> io::Result<Option<FileArtifactValue>>;

    /// `getTreeMetadata(ActionInput)`.
    fn get_tree_metadata(&self, input: &ActionInputRef) -> Option<Arc<TreeArtifactValue>>;

    /// `getFileset(ActionInput)`.
    fn get_fileset(&self, input: &ActionInputRef) -> Option<Arc<FilesetOutputTree>>;

    /// `getRunfilesMetadata(ActionInput)`.
    fn get_runfiles_metadata(&self, input: &ActionInputRef) -> Option<Arc<RunfilesArtifactValue>>;

    /// `getInput(String execPath)`.
    fn get_input(&self, exec_path: &PathFragment) -> Option<ActionInputRef>;
}

/// `InputMetadataProvider.expandArtifacts`: expands tree artifacts in `inputs`.
// Java: com/google/devtools/build/lib/actions/InputMetadataProvider.java:128 InputMetadataProvider.expandArtifacts
pub fn expand_artifacts(
    input_metadata_provider: &dyn InputMetadataProvider,
    inputs: &NestedSet<ActionInputRef>,
    keep_empty_tree_artifacts: bool,
    keep_runfiles_trees: bool,
) -> Vec<ActionInputRef> {
    let mut result = Vec::new();
    // Java: TreeSet<Artifact> ordered by exec path.
    let mut empty_tree_artifacts: BTreeMap<PathFragment, ActionInputRef> = BTreeMap::new();
    let mut tree_file_artifact_parents: HashSet<ActionInputRef> = HashSet::new();
    for input in inputs.to_list().iter() {
        let Some(artifact) = input.as_artifact() else {
            result.push(input.clone());
            continue;
        };
        if artifact.is_runfiles_tree() {
            if keep_runfiles_trees {
                result.push(input.clone());
            }
        } else if artifact.is_tree_artifact() {
            match input_metadata_provider.get_tree_metadata(input) {
                Some(tree) if !tree.get_children().is_empty() => {
                    result.extend(tree.get_children().iter().cloned());
                }
                _ => {
                    empty_tree_artifacts.insert(input.get_exec_path().clone(), input.clone());
                }
            }
        } else {
            result.push(input.clone());
            if artifact.is_child_of_declared_directory()
                && let Some(parent) = artifact.get_parent()
            {
                tree_file_artifact_parents.insert(parent);
            }
        }
    }
    if keep_empty_tree_artifacts {
        for (_, a) in empty_tree_artifacts {
            if !tree_file_artifact_parents.contains(&a) {
                result.push(a);
            }
        }
    }
    result
}

/// Port of `TreeArtifactValue` (the parts the execution layer reads).
// Java: com/google/devtools/build/lib/skyframe/TreeArtifactValue.java TreeArtifactValue
#[derive(Debug, Clone, Default)]
pub struct TreeArtifactValue {
    /// `getChildValues()`: the `TreeFileArtifact` children (sorted by exec path, as Java's
    /// `ImmutableSortedMap`) with their metadata.
    pub child_values: Vec<(ActionInputRef, FileArtifactValue)>,
    /// `getArchivedRepresentation()`: the archived artifact and its metadata.
    pub archived_representation: Option<(ActionInputRef, FileArtifactValue)>,
    /// `getMetadata()`: the aggregate value.
    pub metadata: Option<FileArtifactValue>,
    /// `getResolvedPath()`.
    pub resolved_path: Option<PathFragment>,
}

impl TreeArtifactValue {
    /// `getChildren()`.
    pub fn get_children(&self) -> Vec<ActionInputRef> {
        self.child_values.iter().map(|(a, _)| a.clone()).collect()
    }
    /// `getArchivedArtifact()`.
    pub fn get_archived_artifact(&self) -> Option<ActionInputRef> {
        self.archived_representation
            .as_ref()
            .map(|(a, _)| a.clone())
    }
}

/// Port of `RunfilesTree`: a runfiles tree as an input.
// Java: com/google/devtools/build/lib/actions/RunfilesTree.java:27 RunfilesTree
pub trait RunfilesTree: Send + Sync + fmt::Debug {
    /// `getExecPath()`: exec path of the runfiles directory.
    fn get_exec_path(&self) -> PathFragment;
    /// `getMapping()`: runfiles-relative path to artifact (`None`: an empty file).
    fn get_mapping(&self) -> Vec<(PathFragment, Option<ActionInputRef>)>;
    /// `getWorkspaceName()`.
    fn get_workspace_name(&self) -> String;
    /// `isMappingCached()`.
    fn is_mapping_cached(&self) -> bool {
        false
    }
    /// `getArtifactsAtCanonicalLocationsForLogging()`.
    fn get_artifacts_at_canonical_locations_for_logging(&self) -> NestedSet<ActionInputRef>;
    /// `getSymlinksForLogging()`: (runfiles path, artifact).
    fn get_symlinks_for_logging(&self) -> Vec<(String, ActionInputRef)>;
    /// `getRootSymlinksForLogging()`: (runfiles path, artifact).
    fn get_root_symlinks_for_logging(&self) -> Vec<(String, ActionInputRef)>;
    /// `getEmptyFilenamesForLogging()`.
    fn get_empty_filenames_for_logging(&self) -> Vec<PathFragment>;
    /// `getRepoMappingManifestForLogging()`.
    fn get_repo_mapping_manifest_for_logging(&self) -> Option<ActionInputRef>;
}

/// Port of `RunfilesArtifactValue`.
// Java: com/google/devtools/build/lib/actions/RunfilesArtifactValue.java RunfilesArtifactValue
#[derive(Debug, Clone)]
pub struct RunfilesArtifactValue {
    pub runfiles_tree: Arc<dyn RunfilesTree>,
}

impl RunfilesArtifactValue {
    /// `getRunfilesTree()`.
    pub fn get_runfiles_tree(&self) -> &Arc<dyn RunfilesTree> {
        &self.runfiles_tree
    }
}

/// Port of `FilesetOutputSymlink`.
// Java: com/google/devtools/build/lib/actions/FilesetOutputSymlink.java FilesetOutputSymlink
#[derive(Debug, Clone)]
pub struct FilesetOutputSymlink {
    pub name: PathFragment,
    pub target: ActionInputRef,
}

/// Port of `FilesetOutputTree`.
// Java: com/google/devtools/build/lib/actions/FilesetOutputTree.java FilesetOutputTree
#[derive(Debug, Clone, Default)]
pub struct FilesetOutputTree {
    pub symlinks: Vec<FilesetOutputSymlink>,
}
