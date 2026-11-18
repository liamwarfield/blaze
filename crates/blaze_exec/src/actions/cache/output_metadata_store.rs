//! Port of `lib/actions/cache/OutputMetadataStore.java` (seam: the output side of the action's
//! metadata handler, implemented by `blaze_sky_exec`'s `ActionOutputMetadataStore` port).

use std::io;

use blaze_shell::InterruptedException;

use crate::actions::action_input::ActionInputRef;
use crate::actions::file_artifact_value::FileArtifactValue;
use crate::actions::input_metadata_provider::TreeArtifactValue;

/// `throws IOException, InterruptedException`.
#[derive(Debug, thiserror::Error)]
pub enum IoOrInterruptedException {
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error(transparent)]
    Interrupted(#[from] InterruptedException),
}

impl IoOrInterruptedException {
    /// `e instanceof FileNotFoundException`.
    pub fn is_file_not_found(&self) -> bool {
        matches!(self, IoOrInterruptedException::Io(e) if e.kind() == io::ErrorKind::NotFound)
    }
}

/// Handles the metadata of the outputs of the action during its execution.
///
/// Artifacts are passed as [`ActionInputRef`]s whose `as_artifact()` is `Some`; tree artifacts
/// (Java `SpecialArtifact`) are those with `is_tree_artifact()`.
// Java: com/google/devtools/build/lib/actions/cache/OutputMetadataStore.java:25 OutputMetadataStore
pub trait OutputMetadataStore: Send + Sync {
    /// Injects the metadata of a file.
    ///
    /// This can be used to save filesystem operations when the metadata is already known.
    ///
    /// Tree artifacts and their children must not be passed here. Instead, they should be passed
    /// to [`OutputMetadataStore::inject_tree`].
    // Java: com/google/devtools/build/lib/actions/cache/OutputMetadataStore.java:38 OutputMetadataStore.injectFile
    fn inject_file(&self, output: &ActionInputRef, metadata: FileArtifactValue);

    /// Injects the metadata of a tree artifact.
    ///
    /// This can be used to save filesystem operations when the metadata is already known.
    // Java: com/google/devtools/build/lib/actions/cache/OutputMetadataStore.java:48 OutputMetadataStore.injectTree
    fn inject_tree(&self, output: &ActionInputRef, tree: TreeArtifactValue);

    /// Returns a [`FileArtifactValue`] for the given output (`None`: the artifact is not a known
    /// output of the action).
    ///
    /// If the metadata of the given output is not known, it's computed. This may result in a
    /// significant amount of I/O.
    ///
    /// The returned value corresponds to the final target of a symlink and therefore must not
    /// have a type of `FileStateType::Symlink`.
    ///
    /// Freshly created output files (i.e. from an action that just executed) that require a stat
    /// to obtain the metadata will first be set read-only and executable during this call. This
    /// ensures that the returned metadata has an appropriate ctime, which is affected by chmod.
    /// Note that this does not apply to outputs injected via `inject_file` or `inject_tree` since
    /// a stat is not required for them.
    ///
    /// A missing file is reported as an [`io::ErrorKind::NotFound`] error (Java
    /// `FileNotFoundException`).
    // Java: com/google/devtools/build/lib/actions/cache/OutputMetadataStore.java:69 OutputMetadataStore.getOutputMetadata
    fn get_output_metadata(
        &self,
        output: &ActionInputRef,
    ) -> Result<Option<FileArtifactValue>, IoOrInterruptedException>;

    /// Retrieves the metadata for this tree artifact. Data should already be available.
    // Java: com/google/devtools/build/lib/actions/cache/OutputMetadataStore.java:72 OutputMetadataStore.getTreeArtifactValue
    fn get_tree_artifact_value(
        &self,
        tree_artifact: &ActionInputRef,
    ) -> Result<TreeArtifactValue, IoOrInterruptedException>;

    /// Marks an artifact as intentionally omitted.
    ///
    /// This is used as an optimization to not download orphaned artifacts (artifacts that no
    /// action depends on) from a remote system.
    // Java: com/google/devtools/build/lib/actions/cache/OutputMetadataStore.java:81 OutputMetadataStore.markOmitted
    fn mark_omitted(&self, output: &ActionInputRef);

    /// Returns `true` if `mark_omitted` was called on the artifact.
    // Java: com/google/devtools/build/lib/actions/cache/OutputMetadataStore.java:84 OutputMetadataStore.artifactOmitted
    fn artifact_omitted(&self, artifact: &ActionInputRef) -> bool;

    /// Discards any cached metadata for the given outputs.
    ///
    /// May be called if an action can make multiple attempts that are expected to create the
    /// same set of output files.
    // Java: com/google/devtools/build/lib/actions/cache/OutputMetadataStore.java:92 OutputMetadataStore.resetOutputs
    fn reset_outputs(&self, outputs: &[ActionInputRef]);
}
