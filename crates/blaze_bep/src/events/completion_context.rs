//! Port of `CompletionContext`.

use std::collections::HashMap;

use blaze_actions::Artifact;
use blaze_exec::actions::file_artifact_value::FileArtifactValue;
use blaze_util::unported;
use blaze_vfs::Path;

/// `CompletionContext.ArtifactReceiver`.
// Java: com/google/devtools/build/lib/actions/CompletionContext.java:115 CompletionContext.ArtifactReceiver
pub trait ArtifactReceiver {
    // Java: com/google/devtools/build/lib/actions/CompletionContext.java:116 CompletionContext.ArtifactReceiver.accept
    fn accept(&mut self, artifact: &Artifact, metadata: &FileArtifactValue);

    /// `acceptFilesetMapping(Artifact fileset, FilesetOutputSymlink link)`. PORT: filesets are
    /// not ported; a fileset artifact with `expand_filesets` fails loudly before reaching this.
    // Java: com/google/devtools/build/lib/actions/CompletionContext.java:118 CompletionContext.ArtifactReceiver.acceptFilesetMapping
    fn accept_fileset_mapping(&mut self, fileset: &Artifact);
}

/// `ActionInputMap` restricted to what completion reads: the metadata of the target's
/// important artifacts, and the children of its tree artifacts.
// Java: com/google/devtools/build/lib/actions/ActionInputMap.java:51 ActionInputMap
#[derive(Clone, Debug, Default)]
pub struct ImportantInputMap {
    metadata: HashMap<Artifact, FileArtifactValue>,
    /// `TreeArtifactValue.getChildValues()` (sorted by tree file artifact, as Java's
    /// `ImmutableSortedMap`).
    trees: HashMap<Artifact, Vec<(Artifact, FileArtifactValue)>>,
}

impl ImportantInputMap {
    pub fn new() -> Self {
        Self::default()
    }

    // Java: com/google/devtools/build/lib/actions/ActionInputMap.java:99 ActionInputMap.put
    pub fn put(&mut self, artifact: Artifact, metadata: FileArtifactValue) {
        self.metadata.entry(artifact).or_insert(metadata);
    }

    // Java: com/google/devtools/build/lib/actions/ActionInputMap.java:403 ActionInputMap.putTreeArtifact
    pub fn put_tree_artifact(
        &mut self,
        tree: Artifact,
        tree_metadata: FileArtifactValue,
        children: Vec<(Artifact, FileArtifactValue)>,
    ) {
        self.metadata.entry(tree.clone()).or_insert(tree_metadata);
        self.trees.entry(tree).or_insert(children);
    }

    // Java: com/google/devtools/build/lib/actions/ActionInputMap.java:206 ActionInputMap.getInputMetadata
    pub fn get_input_metadata(&self, artifact: &Artifact) -> Option<&FileArtifactValue> {
        self.metadata.get(artifact)
    }

    // Java: com/google/devtools/build/lib/actions/ActionInputMap.java:319 ActionInputMap.getTreeMetadata
    pub fn get_tree_children(
        &self,
        artifact: &Artifact,
    ) -> Option<&[(Artifact, FileArtifactValue)]> {
        self.trees.get(artifact).map(Vec::as_slice)
    }
}

/// `CompletionContext` contains an `ArtifactExpander` and `ArtifactPathResolver` used to
/// resolve output files during a `TargetCompletionFunction` evaluation.
///
/// PORT: the path resolver is always `ArtifactPathResolver.IDENTITY`, so
/// `pathResolver().toPath(artifact)` is `artifact.getPath()` (the host path). Java's
/// `PathResolverFactoryImpl` (`SkyframeExecutor.java:659`) uses the output service's resolver
/// when its action file system is enabled: with the remote output service that is a
/// `RemoteActionFileSystem` over the important inputs (`RemoteOutputService.java:220`), through
/// which an output that exists only remotely is found. Not ported.
// Java: com/google/devtools/build/lib/actions/CompletionContext.java:32 CompletionContext
#[derive(Clone, Debug, Default)]
pub struct CompletionContext {
    important_input_map: ImportantInputMap,
    expand_filesets: bool,
}

impl CompletionContext {
    // Java: com/google/devtools/build/lib/actions/CompletionContext.java:46 CompletionContext.<init>
    pub fn new(important_input_map: ImportantInputMap, expand_filesets: bool) -> Self {
        CompletionContext {
            important_input_map,
            expand_filesets,
        }
    }

    /// `FAILED_COMPLETION_CTX`.
    // Java: com/google/devtools/build/lib/actions/CompletionContext.java:33 CompletionContext.FAILED_COMPLETION_CTX
    pub fn failed_completion_ctx() -> Self {
        CompletionContext::new(ImportantInputMap::new(), false)
    }

    /// `pathResolver().toPath(artifact)` with the identity resolver.
    // Java: com/google/devtools/build/lib/actions/ArtifactPathResolver.java:43 ArtifactPathResolver.IDENTITY
    pub fn to_path(&self, artifact: &Artifact) -> Path {
        artifact.get_path()
    }

    // Java: com/google/devtools/build/lib/actions/CompletionContext.java:67 CompletionContext.getImportantInputMap
    pub fn get_important_input_map(&self) -> &ImportantInputMap {
        &self.important_input_map
    }

    // Java: com/google/devtools/build/lib/actions/CompletionContext.java:72 CompletionContext.getFileArtifactValue
    pub fn get_file_artifact_value(&self, artifact: &Artifact) -> Option<&FileArtifactValue> {
        self.important_input_map.get_input_metadata(artifact)
    }

    /// Visits the given artifacts; tree artifacts are expanded to their children, runfiles
    /// trees skipped.
    // Java: com/google/devtools/build/lib/actions/CompletionContext.java:77 CompletionContext.visitArtifacts
    pub fn visit_artifacts<'a>(
        &self,
        artifacts: impl IntoIterator<Item = &'a Artifact>,
        receiver: &mut dyn ArtifactReceiver,
    ) {
        for artifact in artifacts {
            if artifact.is_runfiles_tree() {
                continue;
            }
            if artifact.is_fileset() {
                if self.expand_filesets {
                    unported!(
                        "CompletionContext.visitArtifacts (fileset expansion) (unreachable in Bazel: only the non-open-sourced fileset rule makes fileset artifacts, packages/OutputFile.java:123)"
                    );
                }
            } else if artifact.is_tree_artifact() {
                let Some(children) = self.important_input_map.get_tree_children(artifact) else {
                    panic!("missing metadata for tree artifact: {artifact}");
                };
                for (child, metadata) in children {
                    receiver.accept(child, metadata);
                }
            } else {
                let Some(metadata) = self.important_input_map.get_input_metadata(artifact) else {
                    panic!("missing metadata for artifact: {artifact}");
                };
                receiver.accept(artifact, metadata);
            }
        }
    }
}
