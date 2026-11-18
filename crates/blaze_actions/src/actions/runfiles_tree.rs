//! Port of `lib/actions/RunfilesTree.java`.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

use blaze_collect::NestedSet;
use blaze_util::Fingerprint;
use blaze_vfs::PathFragment;

use crate::actions::action_key_context::ActionKeyContext;
use crate::actions::artifact::Artifact;
use crate::analysis::actions::symlink_tree_action::RunfileSymlinksMode;
use crate::analysis::symlink_entry::SymlinkEntry;

/// The mapping of a runfiles tree: location in the tree to the artifact there (`None` for empty
/// files). Java `SortedMap<PathFragment, Artifact>`.
pub type RunfilesMapping = BTreeMap<PathFragment, Option<Artifact>>;

/// Lazy wrapper for a single runfiles tree.
// Java: com/google/devtools/build/lib/actions/RunfilesTree.java:28 RunfilesTree
pub trait RunfilesTree: Send + Sync + fmt::Debug {
    /// Returns the exec path of the root directory of the runfiles tree.
    // Java: com/google/devtools/build/lib/actions/RunfilesTree.java:30 RunfilesTree.getExecPath
    fn get_exec_path(&self) -> PathFragment;

    /// Returns the mapping from the location in the runfiles tree to the artifact that's there.
    // Java: com/google/devtools/build/lib/actions/RunfilesTree.java:33 RunfilesTree.getMapping
    fn get_mapping(&self) -> Arc<RunfilesMapping>;

    /// Returns artifacts the runfiles tree contain symlinks to.
    ///
    /// This includes artifacts that the symlinks and root symlinks point to, not just artifacts
    /// at their canonical location.
    // Java: com/google/devtools/build/lib/actions/RunfilesTree.java:41 RunfilesTree.getArtifacts
    fn get_artifacts(&self) -> NestedSet<Artifact>;

    /// Returns the [`RunfileSymlinksMode`] for this runfiles tree.
    // Java: com/google/devtools/build/lib/actions/RunfilesTree.java:44 RunfilesTree.getSymlinksMode
    fn get_symlinks_mode(&self) -> RunfileSymlinksMode;

    /// Returns whether the runfile symlinks should be materialized during the build.
    // Java: com/google/devtools/build/lib/actions/RunfilesTree.java:47 RunfilesTree.isBuildRunfileLinks
    fn is_build_runfile_links(&self) -> bool;

    /// Returns the name of the workspace that the build is occurring in.
    // Java: com/google/devtools/build/lib/actions/RunfilesTree.java:50 RunfilesTree.getWorkspaceName
    fn get_workspace_name(&self) -> String;

    /// Returns artifacts the runfiles tree contain symlinks to at their canonical locations.
    ///
    /// This does **not** include artifacts that only the symlinks and root symlinks point to.
    // Java: com/google/devtools/build/lib/actions/RunfilesTree.java:57 RunfilesTree.getArtifactsAtCanonicalLocationsForLogging
    fn get_artifacts_at_canonical_locations_for_logging(&self) -> NestedSet<Artifact>;

    /// Returns the set of names of implicit empty files to materialize.
    ///
    /// If this runfiles tree does not implicitly add empty files, implementations should have a
    /// dedicated fast path that returns an empty set without traversing the tree.
    // Java: com/google/devtools/build/lib/actions/RunfilesTree.java:65 RunfilesTree.getEmptyFilenamesForLogging
    fn get_empty_filenames_for_logging(&self) -> Vec<PathFragment>;

    /// Returns the set of custom symlink entries.
    // Java: com/google/devtools/build/lib/actions/RunfilesTree.java:68 RunfilesTree.getSymlinksForLogging
    fn get_symlinks_for_logging(&self) -> NestedSet<SymlinkEntry>;

    /// Returns the set of root symlinks.
    // Java: com/google/devtools/build/lib/actions/RunfilesTree.java:71 RunfilesTree.getRootSymlinksForLogging
    fn get_root_symlinks_for_logging(&self) -> NestedSet<SymlinkEntry>;

    /// Returns the repo mapping manifest if it exists.
    // Java: com/google/devtools/build/lib/actions/RunfilesTree.java:75 RunfilesTree.getRepoMappingManifestForLogging
    fn get_repo_mapping_manifest_for_logging(&self) -> Option<Artifact>;

    /// Whether [`RunfilesTree::get_mapping`] is cached due to potential reuse within a single
    /// build.
    // Java: com/google/devtools/build/lib/actions/RunfilesTree.java:78 RunfilesTree.isMappingCached
    fn is_mapping_cached(&self) -> bool;

    /// Fingerprints this runfiles tree.
    // Java: com/google/devtools/build/lib/actions/RunfilesTree.java:81 RunfilesTree.fingerprint
    fn fingerprint(
        &self,
        action_key_context: &ActionKeyContext,
        fp: &mut Fingerprint,
        digest_absolute_paths: bool,
    );

    /// Whether the runfiles tree contains any `CONSTANT_METADATA` artifacts.
    // Java: com/google/devtools/build/lib/actions/RunfilesTree.java:87 RunfilesTree.containsConstantMetadata
    fn contains_constant_metadata(&self) -> bool {
        self.get_artifacts()
            .to_list()
            .iter()
            .any(|a| a.is_constant_metadata())
    }
}

/// Shared handle to a [`RunfilesTree`].
pub type RunfilesTreeRef = Arc<dyn RunfilesTree>;
