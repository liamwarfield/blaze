//! Port of `lib/exec/SpawnInputExpander.java`.

use std::sync::Arc;

use blaze_collect::NestedSet;
use blaze_vfs::PathFragment;
use indexmap::IndexMap;

use crate::actions::{
    ActionInputRef, FilesetOutputTree, HierarchicalPath, InputMapping, InputMetadataProvider,
    PathMapper, RunfilesTree, Spawn, empty_marker, expand_artifacts,
};

/// A helper class for spawn strategies to turn runfiles suppliers into input mappings. This class
/// performs no I/O operations, but only rearranges the files according to how the runfiles should
/// be laid out.
// Java: com/google/devtools/build/lib/exec/SpawnInputExpander.java:44 SpawnInputExpander
#[derive(Debug, Clone)]
pub struct SpawnInputExpander {
    expand_archived_tree_artifacts: bool,
}

impl Default for SpawnInputExpander {
    fn default() -> Self {
        SpawnInputExpander::new()
    }
}

// Java: com/google/devtools/build/lib/exec/SpawnInputExpander.java:56 SpawnInputExpander.addMapping
fn add_mapping(
    input_map: &mut InputMapping,
    target_location: &PathFragment,
    input: ActionInputRef,
    base_directory: &PathFragment,
) {
    blaze_util::check_argument!(!target_location.is_absolute(), "{}", target_location);
    input_map.insert(
        HierarchicalPath(base_directory.get_relative(target_location)),
        input,
    );
}

impl SpawnInputExpander {
    /// `new SpawnInputExpander()`.
    // Java: com/google/devtools/build/lib/exec/SpawnInputExpander.java:48 SpawnInputExpander.<init>
    pub fn new() -> SpawnInputExpander {
        SpawnInputExpander::with_expand_archived_tree_artifacts(true)
    }

    /// `new SpawnInputExpander(expandArchivedTreeArtifacts)`.
    // Java: com/google/devtools/build/lib/exec/SpawnInputExpander.java:52 SpawnInputExpander.<init>
    pub fn with_expand_archived_tree_artifacts(
        expand_archived_tree_artifacts: bool,
    ) -> SpawnInputExpander {
        SpawnInputExpander {
            expand_archived_tree_artifacts,
        }
    }

    /// `addSingleRunfilesTreeToInputs(RunfilesTree, inputMap, inputMetadataProvider, pathMapper,
    /// baseDirectory)`.
    // Java: com/google/devtools/build/lib/exec/SpawnInputExpander.java:66 SpawnInputExpander.addSingleRunfilesTreeToInputs
    pub fn add_single_runfiles_tree_to_inputs(
        &self,
        runfiles_tree: &dyn RunfilesTree,
        input_map: &mut InputMapping,
        input_metadata_provider: &dyn InputMetadataProvider,
        path_mapper: &PathMapper,
        base_directory: &PathFragment,
    ) {
        self.add_single_runfiles_tree_mappings_to_inputs(
            input_map,
            &runfiles_tree.get_exec_path(),
            &runfiles_tree.get_mapping(),
            input_metadata_provider,
            path_mapper,
            base_directory,
        );
    }

    /// Gathers the mapping for a single runfiles tree into `input_map`.
    ///
    /// This should not be a public interface, it's only there to support legacy code until we
    /// figure out how not to call this method (or else how to make this method more palatable).
    // Java: com/google/devtools/build/lib/exec/SpawnInputExpander.java:87 SpawnInputExpander.addSingleRunfilesTreeToInputs
    pub fn add_single_runfiles_tree_mappings_to_inputs(
        &self,
        input_map: &mut InputMapping,
        root: &PathFragment,
        mappings: &[(PathFragment, Option<ActionInputRef>)],
        input_metadata_provider: &dyn InputMetadataProvider,
        path_mapper: &PathMapper,
        base_directory: &PathFragment,
    ) {
        blaze_util::check_argument!(!root.is_absolute(), "{}", root);
        for (key, value) in mappings {
            let location = root.get_relative(key);
            let Some(artifact_ref) = value else {
                add_mapping(
                    input_map,
                    &map_for_runfiles(path_mapper, root, &location),
                    empty_marker(),
                    base_directory,
                );
                continue;
            };
            let artifact = artifact_ref
                .as_artifact()
                .expect("runfiles mappings hold artifacts");
            blaze_util::check_argument!(!artifact.is_runfiles_tree(), "{:?}", artifact_ref);
            if artifact.is_tree_artifact() {
                let tree_artifact_value = input_metadata_provider
                    .get_tree_metadata(artifact_ref)
                    .unwrap_or_else(|| {
                        panic!("NullPointerException: no tree metadata for {artifact_ref}")
                    });
                let archived_tree_artifact = if self.expand_archived_tree_artifacts {
                    None
                } else {
                    tree_artifact_value.get_archived_artifact()
                };
                if let Some(archived) = archived_tree_artifact {
                    // TODO(bazel-team): Add path mapping support for archived tree artifacts.
                    add_mapping(input_map, &location, archived, base_directory);
                } else if tree_artifact_value.get_children().is_empty() {
                    add_mapping(
                        input_map,
                        &map_for_runfiles(path_mapper, root, &location),
                        artifact_ref.clone(),
                        base_directory,
                    );
                } else {
                    for input in tree_artifact_value.get_children() {
                        let parent_relative_path = input
                            .as_artifact()
                            .and_then(|a| a.get_parent_relative_path())
                            .expect("tree artifact children are TreeFileArtifacts");
                        add_mapping(
                            input_map,
                            &map_for_runfiles(path_mapper, root, &location)
                                .get_relative(&parent_relative_path),
                            input,
                            base_directory,
                        );
                    }
                }
            } else if artifact.is_fileset() {
                // TODO(bazel-team): Add path mapping support for filesets.
                let fileset_output = input_metadata_provider
                    .get_fileset(artifact_ref)
                    .unwrap_or_else(|| {
                        panic!("NullPointerException: no fileset metadata for {artifact_ref}")
                    });
                add_fileset_manifest(
                    &location,
                    artifact_ref,
                    &fileset_output,
                    input_map,
                    base_directory,
                );
            } else {
                add_mapping(
                    input_map,
                    &map_for_runfiles(path_mapper, root, &location),
                    artifact_ref.clone(),
                    base_directory,
                );
            }
        }
    }

    /// `addFilesetManifests(filesetMappings, inputMap, baseDirectory)`.
    // Java: com/google/devtools/build/lib/exec/SpawnInputExpander.java:137 SpawnInputExpander.addFilesetManifests
    pub fn add_fileset_manifests(
        fileset_mappings: &IndexMap<ActionInputRef, Arc<FilesetOutputTree>>,
        input_map: &mut InputMapping,
        base_directory: &PathFragment,
    ) {
        for (fileset, tree) in fileset_mappings {
            add_fileset_manifest(
                fileset.get_exec_path(),
                fileset,
                tree,
                input_map,
                base_directory,
            );
        }
    }

    // Java: com/google/devtools/build/lib/exec/SpawnInputExpander.java:159 SpawnInputExpander.addInputs
    fn add_inputs(
        &self,
        input_map: &mut InputMapping,
        input_files: &NestedSet<ActionInputRef>,
        input_metadata_provider: &dyn InputMetadataProvider,
        path_mapper: &PathMapper,
        base_directory: &PathFragment,
    ) {
        // Actions that accept TreeArtifacts as inputs generally expect the directory corresponding
        // to the artifact to be created, even if it is empty. We explicitly keep empty
        // TreeArtifacts here to signal consumers that they should create the directory.
        let inputs = expand_artifacts(
            input_metadata_provider,
            input_files,
            /* keep_empty_tree_artifacts= */ true,
            /* keep_runfiles_trees= */ true,
        );
        for input in inputs {
            let artifact = input.as_artifact();
            match artifact {
                Some(a) if a.is_child_of_declared_directory() => {
                    let parent = a.get_parent().expect("TreeFileArtifact.getParent");
                    let parent_relative_path = a
                        .get_parent_relative_path()
                        .expect("TreeFileArtifact.getParentRelativePath");
                    add_mapping(
                        input_map,
                        &path_mapper
                            .map(parent.get_exec_path())
                            .get_relative(&parent_relative_path),
                        input.clone(),
                        base_directory,
                    );
                }
                Some(a) if a.is_runfiles_tree() => {
                    let runfiles = input_metadata_provider
                        .get_runfiles_metadata(&input)
                        .unwrap_or_else(|| {
                            panic!("NullPointerException: no runfiles metadata for {input}")
                        });
                    self.add_single_runfiles_tree_to_inputs(
                        runfiles.get_runfiles_tree().as_ref(),
                        input_map,
                        input_metadata_provider,
                        path_mapper,
                        base_directory,
                    );
                }
                Some(a) if a.is_fileset() => {
                    let fileset =
                        input_metadata_provider
                            .get_fileset(&input)
                            .unwrap_or_else(|| {
                                panic!("NullPointerException: no fileset metadata for {input}")
                            });
                    add_fileset_manifest(
                        input.get_exec_path(),
                        &input,
                        &fileset,
                        input_map,
                        base_directory,
                    );
                }
                _ => add_mapping(
                    input_map,
                    &path_mapper.map(input.get_exec_path()),
                    input.clone(),
                    base_directory,
                ),
            }
        }
    }

    /// Convert the inputs and runfiles of the given spawn to a map from exec-root relative paths
    /// to `ActionInput`s. The returned map does not contain non-empty tree artifacts as they are
    /// expanded to file artifacts. Tree artifacts that would expand to the empty set under the
    /// provided `InputMetadataProvider` are left untouched so that their corresponding empty
    /// directories can be created.
    ///
    /// The returned map contains all runfiles, but not the `MANIFEST`. It is sorted by
    /// `PathFragment.HIERARCHICAL_COMPARATOR`, which orders the paths nested under a directory
    /// directly after it.
    // Java: com/google/devtools/build/lib/exec/SpawnInputExpander.java:217 SpawnInputExpander.getInputMapping
    pub fn get_input_mapping(
        &self,
        spawn: &dyn Spawn,
        input_metadata_provider: &dyn InputMetadataProvider,
        base_directory: &PathFragment,
    ) -> InputMapping {
        let mut input_map = InputMapping::new();
        self.add_inputs(
            &mut input_map,
            spawn.get_input_files(),
            input_metadata_provider,
            spawn.get_path_mapper(),
            base_directory,
        );
        input_map
    }
}

// Java: com/google/devtools/build/lib/exec/SpawnInputExpander.java:147 SpawnInputExpander.addFilesetManifest
fn add_fileset_manifest(
    location: &PathFragment,
    fileset_artifact: &ActionInputRef,
    fileset_output: &FilesetOutputTree,
    input_map: &mut InputMapping,
    base_directory: &PathFragment,
) {
    blaze_util::check_argument!(
        fileset_artifact
            .as_artifact()
            .is_some_and(|a| a.is_fileset()),
        "{:?}",
        fileset_artifact
    );
    for link in &fileset_output.symlinks {
        add_mapping(
            input_map,
            &location.get_relative(&link.name),
            link.target.clone(),
            base_directory,
        );
    }
}

// Java: com/google/devtools/build/lib/exec/SpawnInputExpander.java:229 SpawnInputExpander.mapForRunfiles
fn map_for_runfiles(
    path_mapper: &PathMapper,
    runfiles_dir: &PathFragment,
    exec_path: &PathFragment,
) -> PathFragment {
    if path_mapper.is_noop() {
        return exec_path.clone();
    }
    let runfiles_dir_name = runfiles_dir.get_base_name().to_string();
    blaze_util::check_argument!(runfiles_dir_name.ends_with(".runfiles"));
    // Derive the path of the executable, apply the path mapping to it and then rederive the path
    // of the runfiles dir.
    let executable = runfiles_dir
        .replace_name(&runfiles_dir_name[..runfiles_dir_name.len() - ".runfiles".len()])
        .expect("runfiles dir has a name");
    path_mapper
        .map(&executable)
        .replace_name(&runfiles_dir_name)
        .expect("mapped executable has a name")
        .get_relative(&exec_path.relative_to(runfiles_dir))
}
