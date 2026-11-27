//! `InputMetadataProvider` and the input-side values it returns (`TreeArtifactValue`,
//! `RunfilesArtifactValue`, `RunfilesTree`, `FilesetOutputTree`), as far as the execution layer
//! reads them. Seam with `blaze_actions` / `blaze_sky_exec`.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt;
use std::io;
use std::sync::Arc;

use blaze_collect::NestedSet;
use blaze_vfs::{DirentType, Path, PathFragment, Symlinks};

use blaze_util::Fingerprint;

use super::action_input::ActionInputRef;
use super::cache::metadata_digest_utils::{MetadataDigestUtils, add_to};
use super::file_artifact_value::{FileArtifactValue, FileStateType};

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
///
/// `toString()` is the `Debug` form.
// Java: com/google/devtools/build/lib/skyframe/TreeArtifactValue.java TreeArtifactValue
// Java: com/google/devtools/build/lib/skyframe/TreeArtifactValue.java:414 TreeArtifactValue.toString
// PORT: skipped, equality serves only change pruning, and blaze_sky_exec's ActionExecutionValue values_equal treats a value with tree artifacts as never equal (it prunes less, which no output shows): TreeArtifactValue.equals TreeArtifactValue.hashCode
// PORT: skipped, no caller at the pin (TreeArtifactValueCodec, serialization, is not ported): TreeArtifactValue.empty TreeArtifactValue.getChildPaths TreeArtifactValue.childDataBuilder TreeArtifactValue.newMultiBuilder
// PORT: skipped, no caller at the pin: TreeArtifactValue.MultiBuilder.<init> TreeArtifactValue.MultiBuilder.addTree TreeArtifactValue.MultiBuilder.putChild TreeArtifactValue.MultiBuilder.setArchivedRepresentation TreeArtifactValue.MultiBuilder.forEach
#[derive(Debug, Clone, Default)]
pub struct TreeArtifactValue {
    /// `getChildValues()`: the `TreeFileArtifact` children (sorted by exec path, as Java's
    /// `ImmutableSortedMap`) with their metadata.
    // Java: com/google/devtools/build/lib/skyframe/TreeArtifactValue.java:368 TreeArtifactValue.getChildValues
    pub child_values: Vec<(ActionInputRef, FileArtifactValue)>,
    /// `getArchivedRepresentation()`: the archived artifact and its metadata
    /// (`ArchivedRepresentation.create(archivedTreeFileArtifact, archivedFileValue)`).
    // Java: com/google/devtools/build/lib/skyframe/TreeArtifactValue.java:348 TreeArtifactValue.getArchivedRepresentation
    // Java: com/google/devtools/build/lib/skyframe/TreeArtifactValue.java:181 TreeArtifactValue.ArchivedRepresentation.<init>
    // Java: com/google/devtools/build/lib/skyframe/TreeArtifactValue.java:186 TreeArtifactValue.ArchivedRepresentation.create
    pub archived_representation: Option<(ActionInputRef, FileArtifactValue)>,
    /// `getMetadata()`: the aggregate value.
    pub metadata: Option<FileArtifactValue>,
    /// `getResolvedPath()`.
    // Java: com/google/devtools/build/lib/skyframe/TreeArtifactValue.java:357 TreeArtifactValue.getResolvedPath
    pub resolved_path: Option<PathFragment>,
}

impl TreeArtifactValue {
    /// `getChildren()`.
    // Java: com/google/devtools/build/lib/skyframe/TreeArtifactValue.java:339 TreeArtifactValue.getChildren
    pub fn get_children(&self) -> Vec<ActionInputRef> {
        self.child_values.iter().map(|(a, _)| a.clone()).collect()
    }
    /// `getArchivedArtifact()`.
    // Java: com/google/devtools/build/lib/skyframe/TreeArtifactValue.java:361 TreeArtifactValue.getArchivedArtifact
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
    /// DIRECTORY, the digest, size 0, not remote, no resolved path); a remote or resolved-path
    /// tree gets [`FileArtifactValue::TreeArtifactComposite`].
    // Java: com/google/devtools/build/lib/skyframe/TreeArtifactValue.java:686 TreeArtifactValue.Builder.build
    // Java: com/google/devtools/build/lib/skyframe/TreeArtifactValue.java:625 TreeArtifactValue.Builder.<init>
    // Java: com/google/devtools/build/lib/skyframe/TreeArtifactValue.java:641 TreeArtifactValue.Builder.putChild
    // Java: com/google/devtools/build/lib/skyframe/TreeArtifactValue.java:653 TreeArtifactValue.Builder.setArchivedRepresentation
    // Java: com/google/devtools/build/lib/skyframe/TreeArtifactValue.java:676 TreeArtifactValue.Builder.setResolvedPath
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
        // Java: new TreeArtifactCompositeFileArtifactValue(digest, entirelyRemote, resolvedPath)
        let metadata = if entirely_remote || resolved_path.is_some() {
            FileArtifactValue::TreeArtifactComposite {
                digest,
                is_remote: super::file_artifact_value::NotInEquality(entirely_remote),
                resolved_path: resolved_path.clone(),
            }
        } else {
            FileArtifactValue::HashedDirectory { digest }
        };
        TreeArtifactValue {
            child_values,
            archived_representation,
            metadata: Some(metadata),
            resolved_path,
        }
    }

    /// Recursively visits all descendants under a directory.
    ///
    /// `visitor` is invoked for each directory, file, and symlink under the given `parent_dir`,
    /// including `parent_dir` itself, with the path relative to the tree directory root (the
    /// apparent path, without resolving any intervening symlinks), the type, and whether at
    /// least one symlink was traversed on the way to the entry.
    ///
    /// Regular files and directories are reported as [`DirentType::File`] or
    /// [`DirentType::Directory`], respectively. Directories are traversed recursively. Symlinks
    /// that resolve to an existing file or directory are followed and reported as the regular
    /// files or directories they point to, recursively for directories. Symlinks that fail to
    /// resolve to an existing path, special files and files whose type could not be determined
    /// cause an error without invoking the visitor. If the visitor fails, traversal is
    /// immediately halted and the error is propagated.
    ///
    /// PORT: Java visits on a thread pool, in a nondeterministic order, and fails fast with the
    /// first exception of any thread; here the walk is a sequential depth-first traversal in
    /// `readdir` order.
    // Java: com/google/devtools/build/lib/skyframe/TreeArtifactValue.java:612 TreeArtifactValue.visitTree
    // Java: com/google/devtools/build/lib/skyframe/TreeArtifactValue.java:524 TreeArtifactValue.Visitor.<init>
    pub fn visit_tree(
        parent_dir: &Path,
        tree_artifact_visitor: &mut dyn FnMut(&PathFragment, DirentType, bool) -> io::Result<()>,
    ) -> io::Result<()> {
        // Java: com/google/devtools/build/lib/skyframe/TreeArtifactValue.java:535 Visitor.run
        TreeArtifactValue::visit(
            parent_dir,
            &PathFragment::empty_fragment(),
            DirentType::Directory,
            /* traversed_symlink= */ false,
            tree_artifact_visitor,
        )
    }

    // Java: com/google/devtools/build/lib/skyframe/TreeArtifactValue.java:549 Visitor.visit
    // Java: com/google/devtools/build/lib/skyframe/TreeArtifactValue.java:514 TreeArtifactValue.TreeArtifactVisitor.visit
    fn visit(
        parent_dir: &Path,
        parent_relative_path: &PathFragment,
        mut type_: DirentType,
        mut traversed_symlink: bool,
        visitor: &mut dyn FnMut(&PathFragment, DirentType, bool) -> io::Result<()>,
    ) -> io::Result<()> {
        let path = parent_dir.get_relative(parent_relative_path);

        if type_ == DirentType::Symlink {
            traversed_symlink = true;

            let stat_follow = path.stat_if_found(Symlinks::Follow)?;

            let Some(stat_follow) = stat_follow else {
                return Err(io::Error::other(format!(
                    "child {parent_relative_path} is a dangling symbolic link"
                )));
            };

            if stat_follow.is_file() && !stat_follow.is_special_file() {
                type_ = DirentType::File;
            } else if stat_follow.is_directory() {
                type_ = DirentType::Directory;
            } else {
                type_ = DirentType::Unknown;
            }
        }

        if type_ == DirentType::Unknown {
            return Err(io::Error::other(format!(
                "child {parent_relative_path} has an unsupported type"
            )));
        }

        visitor(parent_relative_path, type_, traversed_symlink)?;

        if type_ == DirentType::Directory {
            for dirent in path.readdir(Symlinks::Nofollow)? {
                let child_path = parent_relative_path.get_child(dirent.get_name());
                let child_type = dirent.get_type();
                TreeArtifactValue::visit(
                    parent_dir,
                    &child_path,
                    child_type,
                    traversed_symlink,
                    visitor,
                )?;
            }
        }
        Ok(())
    }

    /// `getDigest()`: the digest of [`TreeArtifactValue::build`] (`None` for values not built
    /// through it).
    // Java: com/google/devtools/build/lib/skyframe/TreeArtifactValue.java:335 TreeArtifactValue.getDigest
    // Java: com/google/devtools/build/lib/skyframe/TreeArtifactValue.java:257 TreeArtifactValue.TreeArtifactCompositeFileArtifactValue.getDigest
    pub fn get_digest(&self) -> Option<&[u8]> {
        self.metadata.as_ref().and_then(|m| m.get_digest())
    }

    /// Returns true if the `TreeFileArtifact`s are only stored remotely: `entirelyRemote` of
    /// [`TreeArtifactValue::build`] (an archived representation and every child remote; false
    /// for `EMPTY`), which the aggregate metadata carries.
    // Java: com/google/devtools/build/lib/skyframe/TreeArtifactValue.java:389 TreeArtifactValue.isEntirelyRemote
    pub fn is_entirely_remote(&self) -> bool {
        matches!(
            self.metadata,
            Some(FileArtifactValue::TreeArtifactComposite {
                is_remote: super::file_artifact_value::NotInEquality(true),
                ..
            })
        )
    }

    /// `getTotalChildBytes()`: the sizes of the regular-file children (`totalChildSize` of
    /// [`TreeArtifactValue::build`]).
    // Java: com/google/devtools/build/lib/skyframe/TreeArtifactValue.java:343 TreeArtifactValue.getTotalChildBytes
    pub fn get_total_child_bytes(&self) -> i64 {
        self.child_values
            .iter()
            .filter(|(_, metadata)| metadata.get_type() == FileStateType::RegularFile)
            .map(|(_, metadata)| metadata.get_size())
            .sum()
    }
}

/// A runfiles symlink: its runfiles path and target artifact (the execution layer's view of
/// `SymlinkEntry`, whose artifact is an [`ActionInputRef`]).
// Java: com/google/devtools/build/lib/analysis/SymlinkEntry.java:35 SymlinkEntry
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct SymlinkEntry {
    /// `getPathString()`.
    pub path: String,
    /// `getArtifact()`.
    pub artifact: ActionInputRef,
}

/// Port of `RunfilesTree`: a runfiles tree as an input.
// Java: com/google/devtools/build/lib/actions/RunfilesTree.java:27 RunfilesTree
pub trait RunfilesTree: Send + Sync + fmt::Debug {
    /// `getExecPath()`: exec path of the runfiles directory.
    fn get_exec_path(&self) -> PathFragment;
    /// `getMapping()`: runfiles-relative path to artifact (`None`: an empty file).
    fn get_mapping(&self) -> Vec<(PathFragment, Option<ActionInputRef>)>;
    /// `getSymlinksMode()`: the `RunfileSymlinksMode` of this runfiles tree.
    ///
    /// PORT: a default so that test and replay trees need not implement it; the trees of a build
    /// (`RunfilesTreeAdapter`) do.
    // Java: com/google/devtools/build/lib/actions/RunfilesTree.java:44 RunfilesTree.getSymlinksMode
    fn get_symlinks_mode(
        &self,
    ) -> blaze_actions::analysis::actions::symlink_tree_action::RunfileSymlinksMode {
        blaze_util::unported!("RunfilesTree.getSymlinksMode");
    }
    /// `isBuildRunfileLinks()`: whether the runfile symlinks should be materialized during the
    /// build.
    ///
    /// PORT: a default, as for `get_symlinks_mode`.
    // Java: com/google/devtools/build/lib/actions/RunfilesTree.java:47 RunfilesTree.isBuildRunfileLinks
    fn is_build_runfile_links(&self) -> bool {
        blaze_util::unported!("RunfilesTree.isBuildRunfileLinks");
    }
    /// `getWorkspaceName()`.
    fn get_workspace_name(&self) -> String;
    /// `isMappingCached()`.
    fn is_mapping_cached(&self) -> bool {
        false
    }
    /// `getArtifactsAtCanonicalLocationsForLogging()`.
    fn get_artifacts_at_canonical_locations_for_logging(&self) -> NestedSet<ActionInputRef>;
    /// `getSymlinksForLogging()`: the set of custom symlink entries.
    // Java: com/google/devtools/build/lib/actions/RunfilesTree.java:68 RunfilesTree.getSymlinksForLogging
    fn get_symlinks_for_logging(&self) -> NestedSet<SymlinkEntry>;
    /// `getRootSymlinksForLogging()`: the set of root symlinks.
    // Java: com/google/devtools/build/lib/actions/RunfilesTree.java:71 RunfilesTree.getRootSymlinksForLogging
    fn get_root_symlinks_for_logging(&self) -> NestedSet<SymlinkEntry>;
    /// `getEmptyFilenamesForLogging()`.
    fn get_empty_filenames_for_logging(&self) -> Vec<PathFragment>;
    /// `getRepoMappingManifestForLogging()`.
    fn get_repo_mapping_manifest_for_logging(&self) -> Option<ActionInputRef>;
}

/// Port of `RunfilesArtifactValue`: the artifacts behind a runfiles tree artifact.
///
/// PORT: the parallel lists of filesets and their `FilesetOutputTree`s are absent
/// (`FilesetOutputTree.addTo` is not ported); the code that builds the value fails with
/// `unported!` for a fileset in a runfiles tree.
// Java: com/google/devtools/build/lib/actions/RunfilesArtifactValue.java:37 RunfilesArtifactValue
#[derive(Debug, Clone)]
pub struct RunfilesArtifactValue {
    pub runfiles_tree: Arc<dyn RunfilesTree>,
    metadata: FileArtifactValue,
    // Parallel lists.
    files: Vec<ActionInputRef>,
    file_values: Vec<FileArtifactValue>,
    // Parallel lists.
    trees: Vec<ActionInputRef>,
    tree_values: Vec<Arc<TreeArtifactValue>>,
}

impl RunfilesArtifactValue {
    /// The file and tree artifacts of this runfiles tree.
    ///
    /// PORT: Java also concatenates the filesets; this value holds none (a fileset input is
    /// `unported!` where the input map is built).
    // Java: com/google/devtools/build/lib/actions/RunfilesArtifactValue.java:141 RunfilesArtifactValue.getAllArtifacts
    pub fn get_all_artifacts(&self) -> impl Iterator<Item = &ActionInputRef> {
        self.files.iter().chain(self.trees.iter())
    }

    // Java: com/google/devtools/build/lib/actions/RunfilesArtifactValue.java:60 RunfilesArtifactValue.<init>
    pub fn new(
        runfiles_tree: Arc<dyn RunfilesTree>,
        files: Vec<ActionInputRef>,
        file_values: Vec<FileArtifactValue>,
        trees: Vec<ActionInputRef>,
        tree_values: Vec<Arc<TreeArtifactValue>>,
    ) -> RunfilesArtifactValue {
        blaze_util::check_argument!(
            files.len() == file_values.len() && trees.len() == tree_values.len(),
            "Size mismatch: files={:?}, trees={:?}",
            files,
            trees
        );
        // Compute the digest of this runfiles tree by combining its layout and the digests of
        // every artifact it references.
        let metadata = FileArtifactValue::RunfilesProxy {
            digest: RunfilesArtifactValue::compute_digest(
                &*runfiles_tree,
                &files,
                &file_values,
                &tree_values,
            ),
        };
        RunfilesArtifactValue {
            runfiles_tree,
            metadata,
            files,
            file_values,
            trees,
            tree_values,
        }
    }

    // Java: com/google/devtools/build/lib/actions/RunfilesArtifactValue.java:87 RunfilesArtifactValue.computeDigest
    fn compute_digest(
        runfiles_tree: &dyn RunfilesTree,
        files: &[ActionInputRef],
        file_values: &[FileArtifactValue],
        tree_values: &[Arc<TreeArtifactValue>],
    ) -> Vec<u8> {
        let mut result = Fingerprint::new();

        let mapping = runfiles_tree.get_mapping();
        result.add_int(mapping.len() as i32);
        for (path, artifact) in &mapping {
            result.add_string(path.get_path_string());
            result.add_boolean(artifact.is_some());
            if let Some(artifact) = artifact {
                result.add_string(artifact.get_exec_path().get_path_string());
            }
        }

        result.add_int(files.len() as i32);
        for (file, file_value) in files.iter().zip(file_values) {
            let is_constant_metadata = file.as_artifact().is_some_and(|a| a.is_constant_metadata());
            if is_constant_metadata {
                add_to(&FileArtifactValue::ConstantMetadata, &mut result);
            } else {
                add_to(file_value, &mut result);
            }
        }

        result.add_int(tree_values.len() as i32);
        for tree_value in tree_values {
            result.add_bytes(
                tree_value.get_digest().unwrap_or_else(|| {
                    panic!("NullPointerException: TreeArtifactValue.getDigest()")
                }),
            );
        }

        // No filesets (see the type's comment).
        result.digest_and_reset()
    }

    /// `withOverriddenRunfilesTree(overrideTree)`.
    // Java: com/google/devtools/build/lib/actions/RunfilesArtifactValue.java:119 RunfilesArtifactValue.withOverriddenRunfilesTree
    pub fn with_overridden_runfiles_tree(
        &self,
        override_tree: Arc<dyn RunfilesTree>,
    ) -> RunfilesArtifactValue {
        RunfilesArtifactValue::new(
            override_tree,
            self.files.clone(),
            self.file_values.clone(),
            self.trees.clone(),
            self.tree_values.clone(),
        )
    }

    /// Returns the data of the artifact for this value, as computed by the action cache checker.
    // Java: com/google/devtools/build/lib/actions/RunfilesArtifactValue.java:125 RunfilesArtifactValue.getMetadata
    pub fn get_metadata(&self) -> &FileArtifactValue {
        &self.metadata
    }

    /// `getRunfilesTree()`.
    // Java: com/google/devtools/build/lib/actions/RunfilesArtifactValue.java:130 RunfilesArtifactValue.getRunfilesTree
    pub fn get_runfiles_tree(&self) -> &Arc<dyn RunfilesTree> {
        &self.runfiles_tree
    }

    /// Visits the file artifacts that this runfiles artifact expands to, together with their
    /// data.
    // Java: com/google/devtools/build/lib/actions/RunfilesArtifactValue.java:146 RunfilesArtifactValue.forEachFile
    pub fn for_each_file(&self, mut consumer: impl FnMut(&ActionInputRef, &FileArtifactValue)) {
        for (file, file_value) in self.files.iter().zip(&self.file_values) {
            consumer(file, file_value);
        }
    }

    /// Visits the tree artifacts that this runfiles artifact expands to, together with their
    /// data.
    // Java: com/google/devtools/build/lib/actions/RunfilesArtifactValue.java:153 RunfilesArtifactValue.forEachTree
    pub fn for_each_tree(
        &self,
        mut consumer: impl FnMut(&ActionInputRef, &Arc<TreeArtifactValue>),
    ) {
        for (tree, tree_value) in self.trees.iter().zip(&self.tree_values) {
            consumer(tree, tree_value);
        }
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
// PORT: skipped, only the non-open-sourced fileset rule creates Fileset artifacts (RuleClass.setOutputFileKind), so Bazel has no fileset output trees: FilesetOutputTree.forward FilesetOutputTree.create FilesetOutputTree.symlinks FilesetOutputTree.isForwarded FilesetOutputTree.size FilesetOutputTree.isEmpty FilesetOutputTree.hashCode FilesetOutputTree.equals FilesetOutputTree.toString FilesetOutputTree.addTo
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

/// Port of the `visitTree_*` tests of `TreeArtifactValueTest`.
///
/// PORT: Java uses an `InMemoryFileSystem` (`Scratch`); these tests use the real file system
/// under a fresh temporary directory. Java's visitor is called in a nondeterministic order
/// (`containsExactly`), so both sides are compared sorted.
#[cfg(test)]
mod visit_tree_test {
    use std::ffi::CString;
    use std::io;
    use std::os::unix::fs::symlink;

    use blaze_vfs::{DigestHashFunction, DirentType, FileSystemExt, UnixFileSystem};

    use super::TreeArtifactValue;

    /// `scratch.dir("tree")` in a fresh directory.
    fn scratch() -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let tree_dir = dir.path().join("tree");
        std::fs::create_dir(&tree_dir).unwrap();
        (dir, tree_dir)
    }

    /// `scratch.file(path)`.
    fn file(path: std::path::PathBuf) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, b"").unwrap();
    }

    fn mkfifo(path: &std::path::Path) {
        let c_path = CString::new(path.to_str().unwrap()).unwrap();
        // SAFETY: `c_path` is a valid NUL-terminated string.
        let rc = unsafe { libc::mkfifo(c_path.as_ptr(), 0o644) };
        assert_eq!(rc, 0, "mkfifo {}", path.display());
    }

    /// `VisitTreeArgs`: what the visitor was called with, sorted.
    fn visit_tree(tree_dir: &std::path::Path) -> io::Result<Vec<(String, DirentType, bool)>> {
        let fs = UnixFileSystem::create(DigestHashFunction::Sha256);
        let tree_dir = fs.get_path(tree_dir.to_str().unwrap());
        let mut children = Vec::new();
        TreeArtifactValue::visit_tree(&tree_dir, &mut |child, type_, traversed_symlink| {
            children.push((
                child.get_path_string().to_string(),
                type_,
                traversed_symlink,
            ));
            Ok(())
        })?;
        children.sort();
        Ok(children)
    }

    fn sorted(mut args: Vec<(&str, DirentType, bool)>) -> Vec<(String, DirentType, bool)> {
        args.sort();
        args.into_iter()
            .map(|(child, type_, traversed_symlink)| (child.to_string(), type_, traversed_symlink))
            .collect()
    }

    #[test]
    fn visit_tree_visits_each_child() {
        let (_dir, tree) = scratch();
        file(tree.join("file1"));
        file(tree.join("a/file2"));
        file(tree.join("a/b/file3"));
        symlink("file1", tree.join("file_link")).unwrap();
        symlink("b", tree.join("a/dir_link")).unwrap();

        assert_eq!(
            visit_tree(&tree).unwrap(),
            sorted(vec![
                ("", DirentType::Directory, false),
                ("a", DirentType::Directory, false),
                ("a/b", DirentType::Directory, false),
                ("file1", DirentType::File, false),
                ("a/file2", DirentType::File, false),
                ("a/b/file3", DirentType::File, false),
                ("file_link", DirentType::File, true),
                ("a/dir_link", DirentType::Directory, true),
                ("a/dir_link/file3", DirentType::File, true),
            ])
        );
    }

    #[test]
    fn visit_tree_throws_on_dangling_symlink() {
        let (_dir, tree) = scratch();
        symlink("/does_not_exist", tree.join("symlink")).unwrap();

        let e = visit_tree(&tree).unwrap_err();
        assert!(
            e.to_string()
                .contains("child symlink is a dangling symbolic link"),
            "{e}"
        );
    }

    #[test]
    fn visit_tree_throws_on_symlink_loop() {
        let (_dir, tree) = scratch();
        symlink(&tree, tree.join("symlink")).unwrap();

        let e = visit_tree(&tree).unwrap_err();
        assert!(e.to_string().contains("tree/symlink"), "{e}");
        assert!(
            e.to_string().contains("Too many levels of symbolic links"),
            "{e}"
        );
    }

    // PORT: Java overrides `readdir` of the file system to return a dirent of type UNKNOWN; here
    // the entry is a FIFO, which `readdir` reports as UNKNOWN.
    #[test]
    fn visit_tree_throws_on_unknown_dirent_type() {
        let (_dir, tree) = scratch();
        mkfifo(&tree.join("unknown"));

        let e = visit_tree(&tree).unwrap_err();
        assert!(
            e.to_string()
                .contains("child unknown has an unsupported type"),
            "{e}"
        );
    }

    // PORT: Java overrides `statIfFound` of the file system to return a special file for the
    // symlink; here the symlink points to a FIFO.
    #[test]
    fn visit_tree_throws_on_symlink_to_special_file() {
        let (dir, tree) = scratch();
        let special = dir.path().join("special");
        mkfifo(&special);
        symlink(&special, tree.join("sym")).unwrap();

        let e = visit_tree(&tree).unwrap_err();
        assert!(
            e.to_string().contains("child sym has an unsupported type"),
            "{e}"
        );
    }

    #[test]
    fn visit_tree_propagates_io_exception_from_visitor() {
        let (_dir, tree) = scratch();
        let fs = UnixFileSystem::create(DigestHashFunction::Sha256);
        let tree_dir = fs.get_path(tree.to_str().unwrap());

        let thrown = TreeArtifactValue::visit_tree(&tree_dir, &mut |_, _, _| {
            Err(io::Error::other("From visitor"))
        })
        .unwrap_err();
        assert_eq!(thrown.to_string(), "From visitor");
    }

    #[test]
    fn visit_tree_permits_up_level_symlink_inside_tree() {
        let (_dir, tree) = scratch();
        file(tree.join("file"));
        std::fs::create_dir(tree.join("a")).unwrap();
        symlink("../file", tree.join("a/up_link")).unwrap();

        assert_eq!(
            visit_tree(&tree).unwrap(),
            sorted(vec![
                ("", DirentType::Directory, false),
                ("file", DirentType::File, false),
                ("a", DirentType::Directory, false),
                ("a/up_link", DirentType::File, true),
            ])
        );
    }

    #[test]
    fn visit_tree_permits_up_level_symlink_outside_tree() {
        let (dir, tree) = scratch();
        file(tree.join("file"));
        std::fs::create_dir(tree.join("a")).unwrap();
        file(dir.path().join("other_tree/file"));
        symlink("../../other_tree/file", tree.join("a/uplink")).unwrap();

        assert_eq!(
            visit_tree(&tree).unwrap(),
            sorted(vec![
                ("", DirentType::Directory, false),
                ("file", DirentType::File, false),
                ("a", DirentType::Directory, false),
                ("a/uplink", DirentType::File, true),
            ])
        );
    }

    #[test]
    fn visit_tree_permits_absolute_symlink() {
        let (dir, tree) = scratch();
        let target_file = dir.path().join("target_file");
        file(target_file.clone());
        let target_dir = dir.path().join("target_dir");
        std::fs::create_dir(&target_dir).unwrap();
        symlink(&target_file, tree.join("absolute_file_link")).unwrap();
        symlink(&target_dir, tree.join("absolute_dir_link")).unwrap();

        assert_eq!(
            visit_tree(&tree).unwrap(),
            sorted(vec![
                ("", DirentType::Directory, false),
                ("absolute_file_link", DirentType::File, true),
                ("absolute_dir_link", DirentType::Directory, true),
            ])
        );
    }

    #[test]
    fn visit_tree_permits_uplevel_symlink_traversing_outside_then_back_inside_tree() {
        let (_dir, tree) = scratch();
        file(tree.join("file"));
        symlink("../tree/file", tree.join("link")).unwrap();

        assert_eq!(
            visit_tree(&tree).unwrap(),
            sorted(vec![
                ("", DirentType::Directory, false),
                ("file", DirentType::File, false),
                ("link", DirentType::File, true),
            ])
        );
    }
}
