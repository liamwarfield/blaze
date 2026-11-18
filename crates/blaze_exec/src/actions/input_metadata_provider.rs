//! `InputMetadataProvider` and the input-side values it returns (`TreeArtifactValue`,
//! `RunfilesArtifactValue`, `RunfilesTree`, `FilesetOutputTree`), as far as the execution layer
//! reads them. Seam with `blaze_actions` / `blaze_sky_exec`.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt;
use std::io;
use std::sync::Arc;

use blaze_collect::NestedSet;
use blaze_vfs::PathFragment;

use blaze_util::Fingerprint;

use super::action_input::ActionInputRef;
use super::cache::metadata_digest_utils::{MetadataDigestUtils, add_to};
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

    /// `TreeArtifactValue.newBuilder(parent)`, `putChild` for each child,
    /// `setArchivedRepresentation`, `setResolvedPath`, then `build()`: sorts the children and
    /// computes the aggregate metadata (`getMetadata()`) over them.
    ///
    /// PORT: `getMetadata()` is Java's `TreeArtifactCompositeFileArtifactValue(digest,
    /// entirelyRemote, resolvedPath)`. For a local tree without a resolved path it behaves as
    /// `FileArtifactValue::HashedDirectory { digest }` in every method the port reads (type
    /// DIRECTORY, the digest, size 0, not remote, no resolved path); remote or resolved-path
    /// trees are not ported.
    // Java: com/google/devtools/build/lib/skyframe/TreeArtifactValue.java:686 TreeArtifactValue.Builder.build
    pub fn build(
        mut child_values: Vec<(ActionInputRef, FileArtifactValue)>,
        archived_representation: Option<(ActionInputRef, FileArtifactValue)>,
        resolved_path: Option<PathFragment>,
    ) -> TreeArtifactValue {
        // ImmutableSortedMap<TreeFileArtifact, ...>: TreeFileArtifacts compare by exec path.
        child_values.sort_by(|(a, _), (b, _)| a.get_exec_path().cmp(b.get_exec_path()));
        for w in child_values.windows(2) {
            if w[0].0.get_exec_path() == w[1].0.get_exec_path() {
                panic!(
                    "IllegalArgumentException: Multiple entries with same key: {}",
                    w[0].0
                );
            }
        }
        if child_values.is_empty() && archived_representation.is_none() && resolved_path.is_none() {
            // Java: com/google/devtools/build/lib/skyframe/TreeArtifactValue.java:195 TreeArtifactValue.EMPTY
            return TreeArtifactValue {
                child_values,
                archived_representation: None,
                metadata: Some(FileArtifactValue::HashedDirectory {
                    digest: MetadataDigestUtils::from_metadata(&HashMap::new()),
                }),
                resolved_path: None,
            };
        }

        let mut fingerprint = Fingerprint::new();
        let mut entirely_remote = archived_representation
            .as_ref()
            .is_none_or(|(_, archived_file_value)| archived_file_value.is_remote());

        for (child, metadata) in &child_values {
            // Digest will be deterministic because children are sorted.
            let parent_relative_path = child
                .as_artifact()
                .and_then(|a| a.get_parent_relative_path())
                .unwrap_or_else(|| {
                    panic!("tree artifact child {} is not a TreeFileArtifact", child)
                });
            fingerprint.add_string(parent_relative_path.get_path_string());
            add_to(metadata, &mut fingerprint);

            // Tolerate a mix of local and remote children (b/152496153#comment80).
            entirely_remote &= metadata.is_remote();
        }

        if let Some((_, archived_file_value)) = &archived_representation {
            add_to(archived_file_value, &mut fingerprint);
        }

        let digest = fingerprint.digest_and_reset();
        if entirely_remote || resolved_path.is_some() {
            blaze_util::unported!(
                "TreeArtifactValue.getMetadata",
                "TreeArtifactCompositeFileArtifactValue of a remote tree or one with a resolved path"
            );
        }
        TreeArtifactValue {
            child_values,
            archived_representation,
            metadata: Some(FileArtifactValue::HashedDirectory { digest }),
            resolved_path,
        }
    }

    /// `getDigest()`: the digest of [`TreeArtifactValue::build`] (`None` for values not built
    /// through it).
    // Java: com/google/devtools/build/lib/skyframe/TreeArtifactValue.java:335 TreeArtifactValue.getDigest
    pub fn get_digest(&self) -> Option<&[u8]> {
        self.metadata.as_ref().and_then(|m| m.get_digest())
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

/// Port of (the digest part of) `TreeArtifactValueTest`.
#[cfg(test)]
mod tree_artifact_value_test {
    use std::any::Any;

    use blaze_vfs::PathFragment;

    use super::TreeArtifactValue;
    use crate::actions::action_input::{ActionInput, ActionInputRef, Artifact};
    use crate::actions::file_artifact_value::FileArtifactValue;

    /// A `TreeFileArtifact` under `bin/tree`.
    #[derive(Debug)]
    struct TreeFile {
        exec_path: PathFragment,
        parent_relative_path: PathFragment,
    }

    impl ActionInput for TreeFile {
        fn get_exec_path(&self) -> &PathFragment {
            &self.exec_path
        }
        fn is_directory(&self) -> bool {
            false
        }
        fn is_symlink(&self) -> bool {
            false
        }
        fn as_artifact(&self) -> Option<&dyn Artifact> {
            Some(self)
        }
        fn as_any(&self) -> &dyn Any {
            self
        }
    }

    impl Artifact for TreeFile {
        fn is_source_artifact(&self) -> bool {
            false
        }
        fn is_tree_artifact(&self) -> bool {
            false
        }
        fn is_runfiles_tree(&self) -> bool {
            false
        }
        fn is_fileset(&self) -> bool {
            false
        }
        fn is_child_of_declared_directory(&self) -> bool {
            true
        }
        fn get_parent_relative_path(&self) -> Option<PathFragment> {
            Some(self.parent_relative_path.clone())
        }
        fn get_root_relative_path(&self) -> PathFragment {
            PathFragment::create("tree").get_relative(&self.parent_relative_path)
        }
        fn get_root_exec_path(&self) -> PathFragment {
            PathFragment::create("bin")
        }
    }

    /// `TreeFileArtifact.createTreeOutput(parent, relative)` for the parent `bin/tree`.
    fn create_tree_output(relative: &str) -> ActionInputRef {
        ActionInputRef::new(TreeFile {
            exec_path: PathFragment::create("bin/tree")
                .get_relative(&PathFragment::create(relative)),
            parent_relative_path: PathFragment::create(relative),
        })
    }

    /// `metadataWithId(id)`.
    fn metadata_with_id(id: u8) -> FileArtifactValue {
        FileArtifactValue::create_for_normal_file(Some(vec![id]), None, id as i64)
    }

    #[test]
    fn creates_correct_value() {
        let child1 = create_tree_output("child1");
        let child2 = create_tree_output("child2");
        let tree = TreeArtifactValue::build(
            vec![
                (child1.clone(), metadata_with_id(1)),
                (child2.clone(), metadata_with_id(2)),
            ],
            None,
            None,
        );
        assert_eq!(tree.get_children(), vec![child1, child2]);
        assert!(tree.get_digest().is_some());
        assert_eq!(
            tree.metadata.as_ref().unwrap().get_digest(),
            tree.get_digest()
        );
    }

    #[test]
    #[ignore = "needs ArchivedTreeArtifact (not ported)"]
    fn creates_correct_value_with_archived_representation() {}

    #[test]
    #[ignore = "needs TreeArtifactCompositeFileArtifactValue with a resolved path (not ported)"]
    fn creates_correct_value_with_resolved_path() {}

    #[test]
    fn empty() {
        let tree = TreeArtifactValue::build(Vec::new(), None, None);
        assert!(tree.get_children().is_empty());
        assert!(tree.get_digest().is_some());
        assert_eq!(
            tree.metadata.as_ref().unwrap().get_digest(),
            tree.get_digest()
        );
    }

    #[test]
    fn creates_canonical_empty_instance() {
        let a = TreeArtifactValue::build(Vec::new(), None, None);
        let b = TreeArtifactValue::build(Vec::new(), None, None);
        assert_eq!(a.get_digest(), b.get_digest());
    }

    #[test]
    fn order_independence() {
        let child1 = create_tree_output("child1");
        let child2 = create_tree_output("child2");
        let tree1 = TreeArtifactValue::build(
            vec![
                (child1.clone(), metadata_with_id(1)),
                (child2.clone(), metadata_with_id(2)),
            ],
            None,
            None,
        );
        let tree2 = TreeArtifactValue::build(
            vec![(child2, metadata_with_id(2)), (child1, metadata_with_id(1))],
            None,
            None,
        );
        assert_eq!(tree1.get_digest(), tree2.get_digest());
        assert_eq!(tree1.get_children(), tree2.get_children());
    }

    /// Changing a child's contents changes the tree's digest (what the action cache relies on).
    #[test]
    fn child_metadata_change_changes_digest() {
        let child1 = create_tree_output("child1");
        let a = TreeArtifactValue::build(vec![(child1.clone(), metadata_with_id(1))], None, None);
        let b = TreeArtifactValue::build(vec![(child1, metadata_with_id(2))], None, None);
        assert_ne!(a.get_digest(), b.get_digest());
    }
}
