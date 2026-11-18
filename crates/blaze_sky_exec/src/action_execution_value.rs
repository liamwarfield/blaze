//! `ActionExecutionValue`: the metadata of an executed action's outputs.

use std::sync::Arc;

use blaze_actions::Artifact;
use blaze_exec::actions::{FileArtifactValue, RunfilesArtifactValue, TreeArtifactValue};
use indexmap::IndexMap;

/// A value representing an executed action.
// Java: com/google/devtools/build/lib/skyframe/ActionExecutionValue.java:56 ActionExecutionValue
// PORT: Java's four size-specialized subclasses (a memory optimization) are one struct. The
// "rich artifact data" (`RunfilesArtifactValue` of a runfiles tree action, `FilesetOutputTree`)
// is `rich_artifact_data`.
#[derive(Clone, Debug, Default)]
pub struct ActionExecutionValue {
    artifact_data: IndexMap<Artifact, FileArtifactValue>,
    tree_artifact_data: IndexMap<Artifact, Arc<TreeArtifactValue>>,
    rich_artifact_data: Option<Arc<RunfilesArtifactValue>>,
}

impl ActionExecutionValue {
    // Java: com/google/devtools/build/lib/skyframe/ActionExecutionValue.java:93 ActionExecutionValue.create
    pub fn create(
        artifact_data: IndexMap<Artifact, FileArtifactValue>,
        tree_artifact_data: IndexMap<Artifact, Arc<TreeArtifactValue>>,
        rich_artifact_data: Option<Arc<RunfilesArtifactValue>>,
    ) -> ActionExecutionValue {
        ActionExecutionValue {
            artifact_data,
            tree_artifact_data,
            rich_artifact_data,
        }
    }

    /// Returns metadata for a given artifact, if that metadata is cached in this value.
    // Java: com/google/devtools/build/lib/skyframe/ActionExecutionValue.java:162 ActionExecutionValue.getExistingFileArtifactValue
    pub fn get_existing_file_artifact_value(
        &self,
        artifact: &Artifact,
    ) -> Option<&FileArtifactValue> {
        self.artifact_data.get(artifact)
    }

    /// `getTreeArtifactValue(Artifact)`.
    // Java: com/google/devtools/build/lib/skyframe/ActionExecutionValue.java:184 ActionExecutionValue.getTreeArtifactValue
    pub fn get_tree_artifact_value(&self, artifact: &Artifact) -> Option<&Arc<TreeArtifactValue>> {
        self.tree_artifact_data.get(artifact)
    }

    /// `getAllFileValues()`.
    pub fn get_all_file_values(&self) -> &IndexMap<Artifact, FileArtifactValue> {
        &self.artifact_data
    }

    /// `getAllTreeArtifactValues()`.
    pub fn get_all_tree_artifact_values(&self) -> &IndexMap<Artifact, Arc<TreeArtifactValue>> {
        &self.tree_artifact_data
    }

    /// `getRichArtifactData()` (a runfiles tree's `RunfilesArtifactValue`).
    pub fn get_rich_artifact_data(&self) -> Option<&Arc<RunfilesArtifactValue>> {
        self.rich_artifact_data.as_ref()
    }
}
