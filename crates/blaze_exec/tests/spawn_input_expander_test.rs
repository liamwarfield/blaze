//! Port of `src/test/java/com/google/devtools/build/lib/exec/SpawnInputExpanderTest.java`.
//!
//! Artifacts, runfiles trees and the metadata provider are test fakes of the seam traits. A
//! runfiles tree's mapping is given as `Runfiles` computes it in the Java test
//! (`<workspace>/<runfiles path>`, root symlinks at the top, `.runfile` for an otherwise empty
//! workspace directory).

use std::any::Any;
use std::io;
use std::sync::Arc;

use blaze_cmdline::Label;
use blaze_collect::{NestedSet, NestedSetBuilder};
use blaze_exec::actions::{
    ActionInput, ActionInputRef, Artifact, FakeOwner, FileArtifactValue, FilesetOutputSymlink,
    FilesetOutputTree, HierarchicalPath, InputMapping, InputMetadataProvider, PathMapper,
    ResourceSet, RunfilesArtifactValue, RunfilesTree, SimpleSpawn, TreeArtifactValue, empty_marker,
};
use blaze_exec::exec::spawn_input_expander::SpawnInputExpander;
use blaze_vfs::PathFragment;
use indexmap::IndexMap;

#[derive(Debug, Clone, PartialEq)]
enum Kind {
    Source,
    Derived,
    Tree,
    Fileset,
    TreeFile {
        parent: ActionInputRefEq,
        parent_relative_path: PathFragment,
    },
    ArchivedTree,
}

/// `ActionInputRef` with `PartialEq` for the derive above.
#[derive(Debug, Clone)]
struct ActionInputRefEq(ActionInputRef);

impl PartialEq for ActionInputRefEq {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}

/// A fake `Artifact`.
#[derive(Debug)]
struct TestArtifact {
    exec_path: PathFragment,
    root_exec_path: PathFragment,
    kind: Kind,
}

impl ActionInput for TestArtifact {
    fn get_exec_path(&self) -> &PathFragment {
        &self.exec_path
    }
    fn is_directory(&self) -> bool {
        self.kind == Kind::Tree
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

impl Artifact for TestArtifact {
    fn is_source_artifact(&self) -> bool {
        self.kind == Kind::Source
    }
    fn is_tree_artifact(&self) -> bool {
        self.kind == Kind::Tree
    }
    fn is_runfiles_tree(&self) -> bool {
        false
    }
    fn is_fileset(&self) -> bool {
        self.kind == Kind::Fileset
    }
    fn is_child_of_declared_directory(&self) -> bool {
        matches!(self.kind, Kind::TreeFile { .. })
    }
    fn is_archived_tree_artifact(&self) -> bool {
        self.kind == Kind::ArchivedTree
    }
    fn get_parent(&self) -> Option<ActionInputRef> {
        match &self.kind {
            Kind::TreeFile { parent, .. } => Some(parent.0.clone()),
            _ => None,
        }
    }
    fn get_parent_relative_path(&self) -> Option<PathFragment> {
        match &self.kind {
            Kind::TreeFile {
                parent_relative_path,
                ..
            } => Some(parent_relative_path.clone()),
            _ => None,
        }
    }
    fn get_root_relative_path(&self) -> PathFragment {
        self.exec_path.relative_to(&self.root_exec_path)
    }
    fn get_root_exec_path(&self) -> PathFragment {
        self.root_exec_path.clone()
    }
}

fn artifact(root_exec_path: &str, exec_path: &str, kind: Kind) -> ActionInputRef {
    ActionInputRef::new(TestArtifact {
        exec_path: PathFragment::create(exec_path),
        root_exec_path: PathFragment::create(root_exec_path),
        kind,
    })
}

/// `ActionsTestUtil.createArtifact(ArtifactRoot.asSourceRoot(Root.fromPath("/root")),
/// "/root/<rel>")`: a source artifact with exec path `<rel>`.
fn source_artifact(rel: &str) -> ActionInputRef {
    artifact("", rel, Kind::Source)
}

/// `createTreeArtifact(relPath)`: a tree artifact under the derived root `out`.
fn create_tree_artifact(rel_path: &str) -> ActionInputRef {
    artifact("out", &format!("out/{rel_path}"), Kind::Tree)
}

/// `TreeFileArtifact.createTreeOutput(parent, relative)`.
fn create_tree_output(parent: &ActionInputRef, relative: &str) -> ActionInputRef {
    artifact(
        "out",
        &format!("{}/{relative}", parent.get_exec_path()),
        Kind::TreeFile {
            parent: ActionInputRefEq(parent.clone()),
            parent_relative_path: PathFragment::create(relative),
        },
    )
}

/// `createFilesetArtifact(relPath)`.
fn create_fileset_artifact(rel_path: &str) -> ActionInputRef {
    artifact("out", &format!("out/{rel_path}"), Kind::Fileset)
}

/// `createFileset(execPath)`: `SpecialArtifact.create(rootDir, execPath, ..., FILESET)`.
fn create_fileset(exec_path: &str) -> ActionInputRef {
    artifact("out", exec_path, Kind::Fileset)
}

/// `filesetSymlink(from, to)`.
fn fileset_symlink(from: &str, to: &str) -> FilesetOutputSymlink {
    FilesetOutputSymlink {
        name: PathFragment::create(from),
        target: artifact("out", &format!("out/{to}"), Kind::Derived),
    }
}

fn normal_file_value() -> FileArtifactValue {
    FileArtifactValue::RegularFile {
        digest: Some(vec![1]),
        proxy: None,
        size: 1,
    }
}

/// `AnalysisTestUtil.createRunfilesTree(execPath, runfiles)`.
#[derive(Debug)]
struct TestRunfilesTree {
    exec_path: PathFragment,
    mapping: Vec<(PathFragment, Option<ActionInputRef>)>,
}

impl TestRunfilesTree {
    fn new(exec_path: &str, mapping: Vec<(&str, Option<ActionInputRef>)>) -> TestRunfilesTree {
        TestRunfilesTree {
            exec_path: PathFragment::create(exec_path),
            mapping: mapping
                .into_iter()
                .map(|(k, v)| (PathFragment::create(k), v))
                .collect(),
        }
    }
}

impl RunfilesTree for TestRunfilesTree {
    fn get_exec_path(&self) -> PathFragment {
        self.exec_path.clone()
    }
    fn get_mapping(&self) -> Vec<(PathFragment, Option<ActionInputRef>)> {
        self.mapping.clone()
    }
    fn get_workspace_name(&self) -> String {
        "workspace".to_string()
    }
    fn get_artifacts_at_canonical_locations_for_logging(&self) -> NestedSet<ActionInputRef> {
        unimplemented!()
    }
    fn get_symlinks_for_logging(&self) -> NestedSet<blaze_exec::actions::SymlinkEntry> {
        unimplemented!()
    }
    fn get_root_symlinks_for_logging(&self) -> NestedSet<blaze_exec::actions::SymlinkEntry> {
        unimplemented!()
    }
    fn get_empty_filenames_for_logging(&self) -> Vec<PathFragment> {
        unimplemented!()
    }
    fn get_repo_mapping_manifest_for_logging(&self) -> Option<ActionInputRef> {
        unimplemented!()
    }
}

/// `FakeActionInputFileCache`.
#[derive(Default)]
struct FakeActionInputFileCache {
    tree_artifacts: Vec<(ActionInputRef, Arc<TreeArtifactValue>)>,
    filesets: Vec<(ActionInputRef, Arc<FilesetOutputTree>)>,
}

impl FakeActionInputFileCache {
    fn put_tree_artifact(&mut self, tree: &ActionInputRef, value: TreeArtifactValue) {
        self.tree_artifacts.push((tree.clone(), Arc::new(value)));
    }
    fn put_fileset(&mut self, fileset: &ActionInputRef, value: FilesetOutputTree) {
        self.filesets.push((fileset.clone(), Arc::new(value)));
    }
}

impl InputMetadataProvider for FakeActionInputFileCache {
    fn get_input_metadata(&self, _input: &ActionInputRef) -> io::Result<Option<FileArtifactValue>> {
        Ok(None)
    }
    fn get_tree_metadata(&self, input: &ActionInputRef) -> Option<Arc<TreeArtifactValue>> {
        self.tree_artifacts
            .iter()
            .find(|(a, _)| a == input)
            .map(|(_, v)| v.clone())
    }
    fn get_fileset(&self, input: &ActionInputRef) -> Option<Arc<FilesetOutputTree>> {
        self.filesets
            .iter()
            .find(|(a, _)| a == input)
            .map(|(_, v)| v.clone())
    }
    fn get_runfiles_metadata(&self, _input: &ActionInputRef) -> Option<Arc<RunfilesArtifactValue>> {
        None
    }
    fn get_input(&self, _exec_path: &PathFragment) -> Option<ActionInputRef> {
        None
    }
}

/// Truth `assertThat(inputMap).containsExactly(k1, v1, k2, v2, ...)` (identity of values).
fn assert_contains_exactly(input_map: &InputMapping, expected: &[(&str, &ActionInputRef)]) {
    assert_eq!(
        input_map.len(),
        expected.len(),
        "actual: {:?}",
        input_map.keys().collect::<Vec<_>>()
    );
    for (k, v) in expected {
        let actual = input_map
            .get(&HierarchicalPath(PathFragment::create(k)))
            .unwrap_or_else(|| {
                panic!(
                    "missing {k}; actual: {:?}",
                    input_map.keys().collect::<Vec<_>>()
                )
            });
        assert!(
            Arc::ptr_eq(&actual.0, &v.0),
            "{k}: expected {v:?}, got {actual:?}"
        );
    }
}

fn tree_with_two_files(
    cache: &mut FakeActionInputFileCache,
) -> (ActionInputRef, ActionInputRef, ActionInputRef) {
    let tree_artifact = create_tree_artifact("treeArtifact");
    let file1 = create_tree_output(&tree_artifact, "file1");
    let file2 = create_tree_output(&tree_artifact, "file2");
    cache.put_tree_artifact(
        &tree_artifact,
        TreeArtifactValue {
            child_values: vec![
                (file1.clone(), normal_file_value()),
                (file2.clone(), normal_file_value()),
            ],
            ..Default::default()
        },
    );
    (tree_artifact, file1, file2)
}

#[test]
fn test_runfiles_single_file() {
    let artifact = source_artifact("dir/file");
    let runfiles_tree = TestRunfilesTree::new(
        "runfiles",
        vec![("workspace/dir/file", Some(artifact.clone()))],
    );
    let mut input_map = InputMapping::new();

    SpawnInputExpander::new().add_single_runfiles_tree_to_inputs(
        &runfiles_tree,
        &mut input_map,
        &FakeActionInputFileCache::default(),
        &PathMapper::NOOP,
        &PathFragment::empty_fragment(),
    );

    assert_contains_exactly(&input_map, &[("runfiles/workspace/dir/file", &artifact)]);
}

#[test]
fn test_runfiles_with_fileset() {
    let fileset = create_fileset_artifact("foo/biz/fs_out");
    let runfiles_tree = TestRunfilesTree::new(
        "runfiles",
        vec![("workspace/foo/biz/fs_out", Some(fileset.clone()))],
    );
    let link = fileset_symlink("zizz", "xyz/zizz");
    let fileset_output_tree = FilesetOutputTree {
        symlinks: vec![link.clone()],
    };

    let mut fake_action_input_file_cache = FakeActionInputFileCache::default();
    fake_action_input_file_cache.put_fileset(&fileset, fileset_output_tree);
    let mut input_map = InputMapping::new();
    SpawnInputExpander::new().add_single_runfiles_tree_to_inputs(
        &runfiles_tree,
        &mut input_map,
        &fake_action_input_file_cache,
        &PathMapper::NOOP,
        &PathFragment::empty_fragment(),
    );

    assert_contains_exactly(
        &input_map,
        &[("runfiles/workspace/foo/biz/fs_out/zizz", &link.target)],
    );
}

#[test]
fn test_runfiles_directory_non_strict() {
    let artifact = source_artifact("dir/file");
    let runfiles_tree = TestRunfilesTree::new(
        "runfiles",
        vec![("workspace/dir/file", Some(artifact.clone()))],
    );
    let mut input_map = InputMapping::new();

    SpawnInputExpander::new().add_single_runfiles_tree_to_inputs(
        &runfiles_tree,
        &mut input_map,
        &FakeActionInputFileCache::default(),
        &PathMapper::NOOP,
        &PathFragment::empty_fragment(),
    );
    assert_contains_exactly(&input_map, &[("runfiles/workspace/dir/file", &artifact)]);
}

#[test]
fn test_runfiles_two_files() {
    let artifact1 = source_artifact("dir/file");
    let artifact2 = source_artifact("dir/baz");
    let runfiles_tree = TestRunfilesTree::new(
        "runfiles",
        vec![
            ("workspace/dir/baz", Some(artifact2.clone())),
            ("workspace/dir/file", Some(artifact1.clone())),
        ],
    );
    let mut input_map = InputMapping::new();

    SpawnInputExpander::new().add_single_runfiles_tree_to_inputs(
        &runfiles_tree,
        &mut input_map,
        &FakeActionInputFileCache::default(),
        &PathMapper::NOOP,
        &PathFragment::empty_fragment(),
    );
    assert_contains_exactly(
        &input_map,
        &[
            ("runfiles/workspace/dir/file", &artifact1),
            ("runfiles/workspace/dir/baz", &artifact2),
        ],
    );
}

#[test]
#[ignore = "needs PathMapper other than NOOP (path mapping is not ported)"]
fn test_runfiles_two_files_path_mapped() {}

#[test]
fn test_runfiles_symlink() {
    let artifact = source_artifact("dir/file");
    let runfiles_tree = TestRunfilesTree::new(
        "runfiles",
        vec![("workspace/symlink", Some(artifact.clone()))],
    );
    let mut input_map = InputMapping::new();

    SpawnInputExpander::new().add_single_runfiles_tree_to_inputs(
        &runfiles_tree,
        &mut input_map,
        &FakeActionInputFileCache::default(),
        &PathMapper::NOOP,
        &PathFragment::empty_fragment(),
    );

    assert_contains_exactly(&input_map, &[("runfiles/workspace/symlink", &artifact)]);
}

#[test]
fn test_runfiles_root_symlink() {
    let artifact = source_artifact("dir/file");
    // If there's no other entry, Runfiles adds an empty file in the workspace to make sure the
    // directory gets created.
    let runfiles_tree = TestRunfilesTree::new(
        "runfiles",
        vec![
            ("symlink", Some(artifact.clone())),
            ("workspace/.runfile", None),
        ],
    );
    let mut input_map = InputMapping::new();

    SpawnInputExpander::new().add_single_runfiles_tree_to_inputs(
        &runfiles_tree,
        &mut input_map,
        &FakeActionInputFileCache::default(),
        &PathMapper::NOOP,
        &PathFragment::empty_fragment(),
    );

    let marker = empty_marker();
    assert_contains_exactly(
        &input_map,
        &[
            ("runfiles/symlink", &artifact),
            ("runfiles/workspace/.runfile", &marker),
        ],
    );
}

#[test]
fn test_runfiles_with_tree_artifacts() {
    let mut fake_action_input_file_cache = FakeActionInputFileCache::default();
    let (tree_artifact, file1, file2) = tree_with_two_files(&mut fake_action_input_file_cache);

    let runfiles_tree = TestRunfilesTree::new(
        "runfiles",
        vec![("workspace/treeArtifact", Some(tree_artifact.clone()))],
    );
    let mut input_map = InputMapping::new();

    SpawnInputExpander::new().add_single_runfiles_tree_to_inputs(
        &runfiles_tree,
        &mut input_map,
        &fake_action_input_file_cache,
        &PathMapper::NOOP,
        &PathFragment::empty_fragment(),
    );

    assert_contains_exactly(
        &input_map,
        &[
            ("runfiles/workspace/treeArtifact/file1", &file1),
            ("runfiles/workspace/treeArtifact/file2", &file2),
        ],
    );
}

#[test]
#[ignore = "needs PathMapper other than NOOP (path mapping is not ported)"]
fn test_runfiles_with_tree_artifacts_path_mapped() {}

#[test]
fn test_runfiles_with_archived_tree_artifacts() {
    let tree_artifact = create_tree_artifact("treeArtifact");
    let archived_tree_artifact = artifact(
        "out",
        "out/:archived_tree_artifacts/treeArtifact.zip",
        Kind::ArchivedTree,
    );
    let mut fake_action_input_file_cache = FakeActionInputFileCache::default();
    fake_action_input_file_cache.put_tree_artifact(
        &tree_artifact,
        TreeArtifactValue {
            archived_representation: Some((
                archived_tree_artifact.clone(),
                FileArtifactValue::MissingFileMarker,
            )),
            ..Default::default()
        },
    );

    let runfiles_tree = TestRunfilesTree::new(
        "runfiles",
        vec![("workspace/treeArtifact", Some(tree_artifact.clone()))],
    );
    let mut input_map = InputMapping::new();

    let expander =
        SpawnInputExpander::with_expand_archived_tree_artifacts(/* expand= */ false);
    expander.add_single_runfiles_tree_to_inputs(
        &runfiles_tree,
        &mut input_map,
        &fake_action_input_file_cache,
        &PathMapper::NOOP,
        &PathFragment::empty_fragment(),
    );

    assert_contains_exactly(
        &input_map,
        &[("runfiles/workspace/treeArtifact", &archived_tree_artifact)],
    );
}

#[test]
fn test_runfiles_with_tree_artifacts_in_symlinks() {
    let mut fake_action_input_file_cache = FakeActionInputFileCache::default();
    let (tree_artifact, file1, file2) = tree_with_two_files(&mut fake_action_input_file_cache);

    let runfiles_tree = TestRunfilesTree::new(
        "runfiles",
        vec![("workspace/symlink", Some(tree_artifact.clone()))],
    );
    let mut input_map = InputMapping::new();

    SpawnInputExpander::new().add_single_runfiles_tree_to_inputs(
        &runfiles_tree,
        &mut input_map,
        &fake_action_input_file_cache,
        &PathMapper::NOOP,
        &PathFragment::empty_fragment(),
    );

    assert_contains_exactly(
        &input_map,
        &[
            ("runfiles/workspace/symlink/file1", &file1),
            ("runfiles/workspace/symlink/file2", &file2),
        ],
    );
}

#[test]
fn test_tree_artifacts_in_inputs() {
    let mut fake_action_input_file_cache = FakeActionInputFileCache::default();
    let (tree_artifact, file1, file2) = tree_with_two_files(&mut fake_action_input_file_cache);

    // new SpawnBuilder("/bin/echo", "Hello World").withInput(treeArtifact).build()
    let owner = FakeOwner::new(
        "Mnemonic",
        Some("progress message"),
        Some(Label::parse_canonical_unchecked("//dummy:label")),
    );
    let mut inputs = NestedSetBuilder::stable_order();
    inputs.add(tree_artifact.clone());
    let spawn = SimpleSpawn::new_simple(
        Arc::new(owner),
        vec!["/bin/echo".to_string(), "Hello World".to_string()],
        IndexMap::new(),
        IndexMap::new(),
        inputs.build(),
        vec![],
        Arc::new(ResourceSet::zero()),
    );
    let input_mappings = SpawnInputExpander::new().get_input_mapping(
        &spawn,
        &fake_action_input_file_cache,
        &PathFragment::empty_fragment(),
    );

    assert_eq!(input_mappings.len(), 2);
    assert_contains_exactly(
        &input_mappings,
        &[
            ("out/treeArtifact/file1", &file1),
            ("out/treeArtifact/file2", &file2),
        ],
    );
}

#[test]
fn test_empty_manifest() {
    let mut fileset_mappings = IndexMap::new();
    fileset_mappings.insert(
        create_fileset("out"),
        Arc::new(FilesetOutputTree::default()),
    );
    let mut input_map = InputMapping::new();

    SpawnInputExpander::add_fileset_manifests(
        &fileset_mappings,
        &mut input_map,
        &PathFragment::empty_fragment(),
    );

    assert!(input_map.is_empty());
}

#[test]
fn fileset() {
    let link1 = fileset_symlink("foo/bar", "dir/file1");
    let link2 = fileset_symlink("foo/baz", "dir/file2");
    let fileset = create_fileset("out");
    let mut fileset_mappings = IndexMap::new();
    fileset_mappings.insert(
        fileset,
        Arc::new(FilesetOutputTree {
            symlinks: vec![link1.clone(), link2.clone()],
        }),
    );
    let mut input_map = InputMapping::new();

    SpawnInputExpander::add_fileset_manifests(
        &fileset_mappings,
        &mut input_map,
        &PathFragment::empty_fragment(),
    );

    assert_contains_exactly(
        &input_map,
        &[
            ("out/foo/bar", &link1.target),
            ("out/foo/baz", &link2.target),
        ],
    );
}
