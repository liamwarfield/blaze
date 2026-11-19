//! `ActionInput` and the `Artifact` view the execution layer needs (seam with `blaze_actions`).

use std::any::Any;
use std::cmp::Ordering;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use blaze_vfs::PathFragment;

use super::virtual_action_input::VirtualActionInput;

/// Port of `ActionInput`: anything a spawn may read (artifacts, virtual inputs, plain paths).
///
/// Seam: `blaze_actions`' `Artifact` must be adapted to this trait (and to [`Artifact`]); the
/// execution layer never constructs artifacts itself.
// Java: com/google/devtools/build/lib/actions/ActionInput.java:27 ActionInput
pub trait ActionInput: Send + Sync + fmt::Debug + Any {
    /// `getExecPath()`: the path relative to the execution root.
    // Java: com/google/devtools/build/lib/actions/ActionInput.java:43 ActionInput.getExecPath
    fn get_exec_path(&self) -> &PathFragment;

    /// `getExecPathString()`.
    // Java: com/google/devtools/build/lib/actions/ActionInput.java:36 ActionInput.getExecPathString
    fn get_exec_path_string(&self) -> String {
        self.get_exec_path().get_path_string().to_string()
    }

    /// `isDirectory()`: whether the input is a directory (tree artifacts, directory sources
    /// declared as such).
    // Java: com/google/devtools/build/lib/actions/ActionInput.java:50 ActionInput.isDirectory
    fn is_directory(&self) -> bool;

    /// `isSymlink()`: whether the input is an unresolved symlink.
    // Java: com/google/devtools/build/lib/actions/ActionInput.java:57 ActionInput.isSymlink
    fn is_symlink(&self) -> bool;

    /// `input instanceof Artifact`.
    fn as_artifact(&self) -> Option<&dyn Artifact> {
        None
    }

    /// `input instanceof VirtualActionInput`.
    fn as_virtual_action_input(&self) -> Option<&dyn VirtualActionInput> {
        None
    }

    /// Downcasting for tests and adapters.
    fn as_any(&self) -> &dyn Any;
}

/// The parts of `Artifact` (and its subclasses `SourceArtifact`, `DerivedArtifact`,
/// `SpecialArtifact`, `TreeFileArtifact`, `ArchivedTreeArtifact`) that the execution layer reads.
// Java: com/google/devtools/build/lib/actions/Artifact.java Artifact
pub trait Artifact: ActionInput {
    /// `isSourceArtifact()` (`instanceof SourceArtifact`).
    fn is_source_artifact(&self) -> bool;
    /// `isTreeArtifact()`.
    fn is_tree_artifact(&self) -> bool;
    /// `isRunfilesTree()`.
    fn is_runfiles_tree(&self) -> bool;
    /// `isFileset()`.
    fn is_fileset(&self) -> bool;
    /// `isConstantMetadata()`.
    fn is_constant_metadata(&self) -> bool {
        false
    }
    /// `isChildOfDeclaredDirectory()`: `instanceof TreeFileArtifact`.
    fn is_child_of_declared_directory(&self) -> bool {
        false
    }
    /// `instanceof ArchivedTreeArtifact`.
    fn is_archived_tree_artifact(&self) -> bool {
        false
    }
    /// `TreeFileArtifact.getParent()`.
    fn get_parent(&self) -> Option<ActionInputRef> {
        None
    }
    /// `TreeFileArtifact.getParentRelativePath()`.
    fn get_parent_relative_path(&self) -> Option<PathFragment> {
        None
    }
    /// `getRootRelativePath()`.
    fn get_root_relative_path(&self) -> PathFragment;
    /// `getRoot().getExecPath()`.
    fn get_root_exec_path(&self) -> PathFragment;
    /// `getRoot().isExternal()`. The default fails loudly: only artifacts with an
    /// `ArtifactRoot` (`blaze_actions`) can answer.
    // Java: com/google/devtools/build/lib/actions/ArtifactRoot.java:238 ArtifactRoot.isExternal
    fn is_root_external(&self) -> bool {
        blaze_util::unported!("ArtifactRoot.isExternal (an Artifact view without an ArtifactRoot)")
    }
    /// `getOwner()`: the label of the artifact's owner, `None` where Java returns null. The
    /// default fails loudly: only `blaze_actions` artifacts have an owner.
    // Java: com/google/devtools/build/lib/actions/Artifact.java:437 Artifact.getOwner
    fn get_owner(&self) -> Option<blaze_cmdline::Label> {
        blaze_util::unported!("Artifact.getOwner (an Artifact view without an owner)")
    }
    /// `getRunfilesPath()`. The default fails loudly: only `blaze_actions` artifacts know their
    /// repository.
    // Java: com/google/devtools/build/lib/actions/Artifact.java:636 Artifact.getRunfilesPath
    fn get_runfiles_path(&self) -> PathFragment {
        blaze_util::unported!("Artifact.getRunfilesPath (an Artifact view without a root)")
    }
}

/// A shared `ActionInput`, with Java's `equals`/`hashCode` (inputs are equal when their exec
/// paths and kinds are equal), so it can live in `NestedSet`s, maps and sets.
// PORT: Java compares artifacts by exec path, root and owner; inputs of one spawn never share an
// exec path with different roots or owners, so exec path plus kind is an equivalent identity here.
#[derive(Clone)]
pub struct ActionInputRef(pub Arc<dyn ActionInput>);

impl ActionInputRef {
    /// Wraps an input.
    pub fn new<T: ActionInput>(input: T) -> ActionInputRef {
        ActionInputRef(Arc::new(input))
    }

    /// The exec path, or `None` for `VirtualActionInput.EMPTY_MARKER` (which has none).
    pub fn exec_path_or_none(&self) -> Option<&PathFragment> {
        match self.0.as_virtual_action_input() {
            Some(v) if v.is_empty_action_input() => None,
            _ => Some(self.0.get_exec_path()),
        }
    }

    fn kind_tag(&self) -> u8 {
        if self.0.as_virtual_action_input().is_some() {
            1
        } else if let Some(a) = self.0.as_artifact() {
            if a.is_tree_artifact() {
                2
            } else if a.is_runfiles_tree() {
                3
            } else {
                4
            }
        } else {
            0
        }
    }
}

impl std::ops::Deref for ActionInputRef {
    type Target = dyn ActionInput;
    fn deref(&self) -> &Self::Target {
        &*self.0
    }
}

impl fmt::Debug for ActionInputRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&*self.0, f)
    }
}

impl fmt::Display for ActionInputRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.exec_path_or_none() {
            Some(p) => f.write_str(p.get_path_string()),
            None => f.write_str("EmptyActionInput"),
        }
    }
}

impl PartialEq for ActionInputRef {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
            || (self.kind_tag() == other.kind_tag()
                && self.exec_path_or_none() == other.exec_path_or_none())
    }
}

impl Eq for ActionInputRef {}

impl Hash for ActionInputRef {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.exec_path_or_none().hash(state);
    }
}

/// Port of `ActionInputHelper.BasicActionInput` (`ActionInputHelper.fromPath`): a plain input
/// identified only by its exec path.
// Java: com/google/devtools/build/lib/actions/ActionInputHelper.java:40 ActionInputHelper.BasicActionInput
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct BasicActionInput {
    exec_path: PathFragment,
}

impl BasicActionInput {
    /// `ActionInputHelper.fromPath(String)`.
    // Java: com/google/devtools/build/lib/actions/ActionInputHelper.java:95 ActionInputHelper.fromPath
    pub fn from_path(path: &str) -> BasicActionInput {
        BasicActionInput {
            exec_path: PathFragment::create(path),
        }
    }

    /// `ActionInputHelper.fromPath(PathFragment)`.
    // Java: com/google/devtools/build/lib/actions/ActionInputHelper.java:103 ActionInputHelper.fromPath
    pub fn from_path_fragment(exec_path: PathFragment) -> BasicActionInput {
        BasicActionInput { exec_path }
    }
}

impl ActionInput for BasicActionInput {
    fn get_exec_path(&self) -> &PathFragment {
        &self.exec_path
    }
    fn is_directory(&self) -> bool {
        false
    }
    fn is_symlink(&self) -> bool {
        false
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// A `PathFragment` ordered by `PathFragment.HIERARCHICAL_COMPARATOR` (children directly after
/// their parent), the key order of spawn input mappings.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct HierarchicalPath(pub PathFragment);

impl Ord for HierarchicalPath {
    fn cmp(&self, other: &Self) -> Ordering {
        PathFragment::hierarchical_compare(&self.0, &other.0)
    }
}

impl PartialOrd for HierarchicalPath {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// `SortedMap<PathFragment, ActionInput>` as returned by
/// `SpawnExecutionContext.getInputMapping` (sorted by `HIERARCHICAL_COMPARATOR`).
pub type InputMapping = std::collections::BTreeMap<HierarchicalPath, ActionInputRef>;
