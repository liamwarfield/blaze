//! Port of src/test/java/com/google/devtools/build/lib/worker/WorkerFilesHashTest.java.
//!
//! PORT: the Mockito `FileArtifactValue` whose `getDigest()` returns a digest is a regular file
//! value with that digest. Tree artifacts are fakes of the `Artifact` seam (as in
//! `spawn_input_expander_test.rs`). `MissingInputException` is an unchecked exception, which is a
//! panic in Blaze.

use std::any::Any;
use std::io;
use std::sync::Arc;

use blaze_collect::NestedSetBuilder;
use blaze_exec::actions::{
    ActionInput, ActionInputRef, Artifact, BasicActionInput, FakeOwner, FileArtifactValue,
    FilesetOutputTree, InputMetadataProvider, PathMapper, ResourceSet, RunfilesArtifactValue,
    SimpleSpawn, TreeArtifactValue,
};
use blaze_exec::worker::worker_files_hash::WorkerFilesHash;
use blaze_vfs::PathFragment;
use indexmap::IndexMap;

/// A fake derived `Artifact` under the root `bazel-out` of `/execroot`: a tree artifact, or a
/// `TreeFileArtifact` child of one.
#[derive(Debug)]
struct TestArtifact {
    exec_path: PathFragment,
    parent: Option<(ActionInputRef, PathFragment)>,
}

impl ActionInput for TestArtifact {
    fn get_exec_path(&self) -> &PathFragment {
        &self.exec_path
    }
    fn is_directory(&self) -> bool {
        self.parent.is_none()
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
        false
    }
    fn is_tree_artifact(&self) -> bool {
        self.parent.is_none()
    }
    fn is_runfiles_tree(&self) -> bool {
        false
    }
    fn is_fileset(&self) -> bool {
        false
    }
    fn is_child_of_declared_directory(&self) -> bool {
        self.parent.is_some()
    }
    fn get_parent(&self) -> Option<ActionInputRef> {
        self.parent.as_ref().map(|(p, _)| p.clone())
    }
    fn get_parent_relative_path(&self) -> Option<PathFragment> {
        self.parent.as_ref().map(|(_, r)| r.clone())
    }
    fn get_root_relative_path(&self) -> PathFragment {
        self.exec_path
            .relative_to(&PathFragment::create("bazel-out"))
    }
    fn get_root_exec_path(&self) -> PathFragment {
        PathFragment::create("bazel-out")
    }
}

// Java: src/test/java/com/google/devtools/build/lib/worker/WorkerFilesHashTest.java:219 WorkerFilesHashTest.createTreeArtifact
fn create_tree_artifact(root_relative_path: &str) -> ActionInputRef {
    ActionInputRef::new(TestArtifact {
        exec_path: PathFragment::create("bazel-out").get_relative_str(root_relative_path),
        parent: None,
    })
}

/// `TreeFileArtifact.createTreeOutput(parent, relative)`.
fn create_tree_output(parent: &ActionInputRef, relative: &str) -> ActionInputRef {
    ActionInputRef::new(TestArtifact {
        exec_path: parent.get_exec_path().get_relative_str(relative),
        parent: Some((parent.clone(), PathFragment::create(relative))),
    })
}

/// `FileArtifactValue.createForNormalFile(digest, null, size)`.
fn normal_file(digest: &[u8], size: i64) -> FileArtifactValue {
    FileArtifactValue::RegularFile {
        digest: Some(digest.to_vec()),
        proxy: None,
        size,
    }
}

// Java: src/test/java/com/google/devtools/build/lib/worker/WorkerFilesHashTest.java:213 WorkerFilesHashTest.fileArtifactValue
fn file_artifact_value(digest: &[u8]) -> Metadata {
    Metadata::Value(normal_file(digest, 0))
}

enum Metadata {
    Value(FileArtifactValue),
    Exception(&'static str),
}

/// `createMetadataProvider(inputMetadataOrExceptions)` and `FakeActionInputFileCache`: metadata
/// by exec path, and tree artifact values.
#[derive(Default)]
struct FakeMetadataProvider {
    metadata: Vec<(String, Metadata)>,
    trees: Vec<(ActionInputRef, Arc<TreeArtifactValue>)>,
}

impl InputMetadataProvider for FakeMetadataProvider {
    fn get_input_metadata(&self, input: &ActionInputRef) -> io::Result<Option<FileArtifactValue>> {
        match self
            .metadata
            .iter()
            .find(|(k, _)| *k == input.get_exec_path_string())
        {
            None => Ok(None),
            Some((_, Metadata::Value(v))) => Ok(Some(v.clone())),
            Some((_, Metadata::Exception(message))) => Err(io::Error::other(*message)),
        }
    }
    fn get_tree_metadata(&self, input: &ActionInputRef) -> Option<Arc<TreeArtifactValue>> {
        self.trees
            .iter()
            .find(|(a, _)| a == input)
            .map(|(_, v)| v.clone())
    }
    fn get_fileset(&self, _input: &ActionInputRef) -> Option<Arc<FilesetOutputTree>> {
        unimplemented!("UnsupportedOperationException")
    }
    fn get_runfiles_metadata(&self, _input: &ActionInputRef) -> Option<Arc<RunfilesArtifactValue>> {
        unimplemented!("UnsupportedOperationException")
    }
    fn get_input(&self, _exec_path: &PathFragment) -> Option<ActionInputRef> {
        unimplemented!("UnsupportedOperationException")
    }
}

/// `new SpawnBuilder().withTool(tools...).withInputs(inputs...).build()`.
fn spawn(tools: &[ActionInputRef], inputs: &[&str]) -> SimpleSpawn {
    let mut tools_set = NestedSetBuilder::stable_order();
    let mut inputs_set = NestedSetBuilder::stable_order();
    for tool in tools {
        tools_set.add(tool.clone());
        inputs_set.add(tool.clone());
    }
    for input in inputs {
        inputs_set.add(ActionInputRef::new(BasicActionInput::from_path(input)));
    }
    SimpleSpawn::new(
        Arc::new(FakeOwner::new("Mnemonic", None, None)),
        vec!["/bin/echo".to_string(), "Hello World".to_string()],
        IndexMap::new(),
        IndexMap::new(),
        inputs_set.build(),
        tools_set.build(),
        Vec::new(),
        None,
        Arc::new(ResourceSet::zero()),
        PathMapper::NOOP,
    )
}

fn from_path(path: &str) -> ActionInputRef {
    ActionInputRef::new(BasicActionInput::from_path(path))
}

// Java: src/test/java/com/google/devtools/build/lib/worker/WorkerFilesHashTest.java:52 WorkerFilesHashTest.getWorkerFilesWithDigests_returnsToolsWithCorrectDigests
#[test]
fn get_worker_files_with_digests_returns_tools_with_correct_digests() {
    let tool1_digest = b"text1".to_vec();
    let tool2_digest = b"text2".to_vec();
    let input_metadata_provider = FakeMetadataProvider {
        metadata: vec![
            ("tool1".to_string(), file_artifact_value(&tool1_digest)),
            ("tool2".to_string(), file_artifact_value(&tool2_digest)),
        ],
        ..Default::default()
    };
    let spawn = spawn(&[from_path("tool1"), from_path("tool2")], &[]);

    let files_with_digests =
        WorkerFilesHash::get_worker_files_with_digests(&spawn, &input_metadata_provider).unwrap();

    assert_eq!(
        files_with_digests.into_iter().collect::<Vec<_>>(),
        vec![
            (PathFragment::create("tool1"), Some(tool1_digest)),
            (PathFragment::create("tool2"), Some(tool2_digest)),
        ]
    );
}

// Java: src/test/java/com/google/devtools/build/lib/worker/WorkerFilesHashTest.java:74 WorkerFilesHashTest.getWorkerFilesWithDigests_treeArtifactTool_returnsExpanded
#[test]
fn get_worker_files_with_digests_tree_artifact_tool_returns_expanded() {
    let tree = create_tree_artifact("tree");
    let child1 = create_tree_output(&tree, "child1");
    let child2 = create_tree_output(&tree, "child2");
    let child1_digest = b"text1".to_vec();
    let child2_digest = b"text2".to_vec();

    let spawn = spawn(std::slice::from_ref(&tree), &[]);

    let tree_artifact_value = TreeArtifactValue {
        child_values: vec![
            (child1.clone(), normal_file(&child1_digest, 123)),
            (child2.clone(), normal_file(&child2_digest, 456)),
        ],
        ..Default::default()
    };

    let fake_action_input_file_cache = FakeMetadataProvider {
        metadata: vec![
            (
                child1.get_exec_path_string(),
                Metadata::Value(normal_file(&child1_digest, 123)),
            ),
            (
                child2.get_exec_path_string(),
                Metadata::Value(normal_file(&child2_digest, 456)),
            ),
        ],
        trees: vec![(tree.clone(), Arc::new(tree_artifact_value))],
    };
    let files_with_digests =
        WorkerFilesHash::get_worker_files_with_digests(&spawn, &fake_action_input_file_cache)
            .unwrap();

    assert_eq!(
        files_with_digests.into_iter().collect::<Vec<_>>(),
        vec![
            (child1.get_exec_path().clone(), Some(child1_digest)),
            (child2.get_exec_path().clone(), Some(child2_digest)),
        ]
    );
}

// Java: src/test/java/com/google/devtools/build/lib/worker/WorkerFilesHashTest.java:112 WorkerFilesHashTest.getWorkerFilesWithDigests_spawnWithInputsButNoTools_returnsEmpty
#[test]
fn get_worker_files_with_digests_spawn_with_inputs_but_no_tools_returns_empty() {
    let input_metadata_provider = FakeMetadataProvider::default();
    let spawn = spawn(&[], &["file1", "file2"]);

    let files_with_digests =
        WorkerFilesHash::get_worker_files_with_digests(&spawn, &input_metadata_provider).unwrap();

    assert!(files_with_digests.is_empty());
}

// Java: src/test/java/com/google/devtools/build/lib/worker/WorkerFilesHashTest.java:123 WorkerFilesHashTest.getWorkerFilesWithDigests_missingDigestForTool_fails
#[test]
#[should_panic(expected = "MissingInputException")]
fn get_worker_files_with_digests_missing_digest_for_tool_fails() {
    let input_metadata_provider = FakeMetadataProvider::default();
    let spawn = spawn(&[from_path("tool")], &[]);

    let _ = WorkerFilesHash::get_worker_files_with_digests(&spawn, &input_metadata_provider);
}

// Java: src/test/java/com/google/devtools/build/lib/worker/WorkerFilesHashTest.java:133 WorkerFilesHashTest.getWorkerFilesWithDigests_ioExceptionForToolMetadata_fails
#[test]
fn get_worker_files_with_digests_io_exception_for_tool_metadata_fails() {
    let input_metadata_provider = FakeMetadataProvider {
        metadata: vec![("tool".to_string(), Metadata::Exception("oh no"))],
        ..Default::default()
    };
    let spawn = spawn(&[from_path("tool")], &[]);

    let thrown = WorkerFilesHash::get_worker_files_with_digests(&spawn, &input_metadata_provider)
        .unwrap_err();

    // assertThat(thrown).isSameInstanceAs(injected)
    assert_eq!(thrown.kind(), io::ErrorKind::Other);
    assert_eq!(thrown.to_string(), "oh no");
}
