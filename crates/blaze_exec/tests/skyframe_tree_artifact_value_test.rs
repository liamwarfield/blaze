//! Port of `src/test/java/com/google/devtools/build/lib/skyframe/TreeArtifactValueTest.java`
//! (the `TreeArtifactValue.Builder` part; `TreeArtifactValue` lives in
//! `blaze_exec::actions::input_metadata_provider`), plus the remote output tree's directory
//! listing that `ActionOutputMetadataStore` merges into a tree.

use std::any::Any;

use blaze_exec::actions::{ActionInput, ActionInputRef, Artifact, FileArtifactValue};
use blaze_exec::actions::{FileStateType, TreeArtifactValue};
use blaze_exec::exec::action_execution_context::RemoteOutputTree;
use blaze_vfs::PathFragment;

#[derive(Debug)]
struct FakeArtifact {
    exec_path: PathFragment,
    tree: bool,
    parent: Option<ActionInputRef>,
    parent_relative_path: Option<PathFragment>,
}

impl ActionInput for FakeArtifact {
    fn get_exec_path(&self) -> &PathFragment {
        &self.exec_path
    }
    fn is_directory(&self) -> bool {
        self.tree
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

impl Artifact for FakeArtifact {
    fn is_source_artifact(&self) -> bool {
        false
    }
    fn is_tree_artifact(&self) -> bool {
        self.tree
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
        self.parent.clone()
    }
    fn get_parent_relative_path(&self) -> Option<PathFragment> {
        self.parent_relative_path.clone()
    }
    fn get_root_relative_path(&self) -> PathFragment {
        self.exec_path.clone()
    }
    fn get_root_exec_path(&self) -> PathFragment {
        PathFragment::create("")
    }
}

/// `createTreeArtifactWithGeneratingAction(root, execPath)`.
fn create_tree_artifact(exec_path: &str) -> ActionInputRef {
    ActionInputRef::new(FakeArtifact {
        exec_path: PathFragment::create(exec_path),
        tree: true,
        parent: None,
        parent_relative_path: None,
    })
}

/// `TreeFileArtifact.createTreeOutput(parent, name)`.
fn create_tree_output(parent: &ActionInputRef, name: &str) -> ActionInputRef {
    let rel = PathFragment::create(name);
    ActionInputRef::new(FakeArtifact {
        exec_path: parent.get_exec_path().get_relative(&rel),
        tree: false,
        parent: Some(parent.clone()),
        parent_relative_path: Some(rel),
    })
}

/// `FileArtifactValue.createForRemoteFile(new byte[] {(byte) id}, id, id)`.
fn metadata_with_id(id: i32) -> FileArtifactValue {
    FileArtifactValue::create_for_remote_file(vec![id as u8], id as i64, id)
}

fn local_metadata(id: u8) -> FileArtifactValue {
    FileArtifactValue::create_for_normal_file(Some(vec![id]), None, id as i64)
}

#[test]
fn creates_correct_value() {
    let parent = create_tree_artifact("bin/tree");
    let child1 = create_tree_output(&parent, "child1");
    let child2 = create_tree_output(&parent, "child2");
    let metadata1 = metadata_with_id(1);
    let metadata2 = metadata_with_id(2);

    let tree = TreeArtifactValue::build(
        vec![
            (child1.clone(), metadata1.clone()),
            (child2.clone(), metadata2.clone()),
        ],
        None,
        None,
    );

    assert_eq!(tree.get_children(), vec![child1.clone(), child2.clone()]);
    assert_eq!(
        tree.child_values,
        vec![(child1, metadata1), (child2, metadata2)]
    );
    let metadata = tree.metadata.as_ref().unwrap();
    assert!(metadata.get_digest().is_some());
    // Every child is remote: the composite value is remote (TreeArtifactCompositeFileArtifactValue).
    assert!(metadata.is_remote());
    assert_eq!(metadata.get_type(), FileStateType::Directory);
    assert_eq!(metadata.get_size(), 0);
}

/// `ArchivedTreeArtifact.createForTree(specialArtifact)` for a tree under the test's `bin` root:
/// `bin/:archived_tree_artifacts/<root-relative path>.zip` (`embedDerivedTreeRoot` of the root
/// `bin`).
fn create_archived_tree_artifact(parent: &ActionInputRef) -> ActionInputRef {
    let root_relative = parent
        .get_exec_path()
        .relative_to(&PathFragment::create("bin"));
    ActionInputRef::new(FakeArtifact {
        exec_path: PathFragment::create("bin/:archived_tree_artifacts")
            .get_relative(&PathFragment::create(&format!("{root_relative}.zip"))),
        tree: false,
        parent: Some(parent.clone()),
        parent_relative_path: None,
    })
}

#[test]
fn creates_correct_value_with_archived_representation() {
    let parent = create_tree_artifact("bin/tree");
    let child1 = create_tree_output(&parent, "child1");
    let child2 = create_tree_output(&parent, "child2");
    let archived_tree_artifact = create_archived_tree_artifact(&parent);
    let child1_metadata = metadata_with_id(1);
    let child2_metadata = metadata_with_id(2);
    let archived_artifact_metadata = metadata_with_id(3);

    let tree = TreeArtifactValue::build(
        vec![
            (child1.clone(), child1_metadata.clone()),
            (child2.clone(), child2_metadata.clone()),
        ],
        Some((
            archived_tree_artifact.clone(),
            archived_artifact_metadata.clone(),
        )),
        None,
    );

    assert_eq!(tree.get_children(), vec![child1.clone(), child2.clone()]);
    assert_eq!(
        tree.child_values,
        vec![
            (child1.clone(), child1_metadata),
            (child2.clone(), child2_metadata)
        ]
    );
    let child_paths: Vec<PathFragment> = tree
        .child_values
        .iter()
        .map(|(child, _)| {
            child
                .as_artifact()
                .unwrap()
                .get_parent_relative_path()
                .unwrap()
        })
        .collect();
    assert_eq!(
        child_paths,
        vec![
            PathFragment::create("child1"),
            PathFragment::create("child2")
        ]
    );
    let metadata = tree.metadata.as_ref().unwrap();
    assert!(metadata.get_digest().is_some());
    assert_eq!(
        tree.archived_representation,
        Some((archived_tree_artifact, archived_artifact_metadata))
    );
}

#[test]
fn creates_correct_value_with_resolved_path() {
    let target_path = PathFragment::create("/some/target/path");

    let tree = TreeArtifactValue::build(Vec::new(), None, Some(target_path.clone()));

    assert_eq!(tree.resolved_path, Some(target_path.clone()));
    assert_eq!(
        tree.metadata.as_ref().unwrap().get_resolved_path(),
        Some(&target_path)
    );
}

#[test]
fn empty() {
    let empty_tree = TreeArtifactValue::build(Vec::new(), None, None);

    assert!(empty_tree.get_children().is_empty());
    assert!(empty_tree.child_values.is_empty());
    assert!(empty_tree.metadata.as_ref().unwrap().get_digest().is_some());
}

#[test]
fn creates_canonical_empty_instance() {
    // PORT: `isSameInstanceAs(TreeArtifactValue.empty())`: the port has no shared instance;
    // both builds give EMPTY's children and metadata.
    let a = TreeArtifactValue::build(Vec::new(), None, None);
    let b = TreeArtifactValue::build(Vec::new(), None, None);
    assert_eq!(a.child_values, b.child_values);
    assert_eq!(a.metadata, b.metadata);
}

#[test]
fn order_independence() {
    let parent = create_tree_artifact("bin/tree");
    let child1 = create_tree_output(&parent, "child1");
    let child2 = create_tree_output(&parent, "child2");
    let metadata1 = metadata_with_id(1);
    let metadata2 = metadata_with_id(2);

    let tree1 = TreeArtifactValue::build(
        vec![
            (child1.clone(), metadata1.clone()),
            (child2.clone(), metadata2.clone()),
        ],
        None,
        None,
    );
    let tree2 =
        TreeArtifactValue::build(vec![(child2, metadata2), (child1, metadata1)], None, None);

    // `TreeArtifactValue.equals`: the digest (the composite metadata) and the children.
    assert_eq!(tree1.metadata, tree2.metadata);
    assert_eq!(tree1.child_values, tree2.child_values);
}

/// `TreeArtifactCompositeFileArtifactValue.isRemote` is true only when every child is remote
/// ("Tolerate a mix of local and remote children"), and its `equals` compares the digest and
/// the resolved path only.
#[test]
fn composite_metadata_is_remote_only_for_entirely_remote_children() {
    let parent = create_tree_artifact("bin/tree");
    let child1 = create_tree_output(&parent, "child1");
    let child2 = create_tree_output(&parent, "child2");

    let mixed = TreeArtifactValue::build(
        vec![
            (child1.clone(), metadata_with_id(1)),
            (child2.clone(), local_metadata(2)),
        ],
        None,
        None,
    );
    assert!(!mixed.metadata.as_ref().unwrap().is_remote());

    let remote = TreeArtifactValue::build(
        vec![(child1, metadata_with_id(1)), (child2, metadata_with_id(2))],
        None,
        None,
    );
    let metadata = remote.metadata.clone().unwrap();
    assert!(metadata.is_remote());
    let FileArtifactValue::TreeArtifactComposite { digest, .. } = &metadata else {
        panic!("expected a TreeArtifactCompositeFileArtifactValue: {metadata:?}");
    };
    let local_twin = FileArtifactValue::TreeArtifactComposite {
        digest: digest.clone(),
        is_remote: blaze_exec::actions::file_artifact_value::NotInEquality(false),
        resolved_path: None,
    };
    assert_eq!(metadata, local_twin);
}

/// `isEntirelyRemote()`: the `entirelyRemote` of `Builder.build` (every child remote, false for
/// `EMPTY`, vacuously true for a tree with only a resolved path).
#[test]
fn is_entirely_remote_follows_builder_build() {
    let parent = create_tree_artifact("bin/tree");
    let child1 = create_tree_output(&parent, "child1");
    let child2 = create_tree_output(&parent, "child2");

    assert!(!TreeArtifactValue::build(Vec::new(), None, None).is_entirely_remote());
    assert!(
        TreeArtifactValue::build(Vec::new(), None, Some(PathFragment::create("/target")))
            .is_entirely_remote()
    );
    assert!(
        TreeArtifactValue::build(
            vec![
                (child1.clone(), metadata_with_id(1)),
                (child2.clone(), metadata_with_id(2))
            ],
            None,
            None,
        )
        .is_entirely_remote()
    );
    assert!(
        !TreeArtifactValue::build(
            vec![(child1, metadata_with_id(1)), (child2, local_metadata(2))],
            None,
            None,
        )
        .is_entirely_remote()
    );
}

/// `getTotalChildBytes()`: the sizes of the regular-file children (`totalChildSize`).
#[test]
fn get_total_child_bytes_sums_regular_file_children() {
    let parent = create_tree_artifact("bin/tree");
    let child1 = create_tree_output(&parent, "child1");
    let child2 = create_tree_output(&parent, "child2");
    let tree = TreeArtifactValue::build(
        vec![(child1, local_metadata(3)), (child2, metadata_with_id(4))],
        None,
        None,
    );
    assert_eq!(tree.get_total_child_bytes(), 7);
    assert_eq!(
        TreeArtifactValue::build(Vec::new(), None, None).get_total_child_bytes(),
        0
    );
}

#[test]
#[ignore = "TreeArtifactValue.MultiBuilder has no caller at Bazel 9.3.2 and is not ported"]
fn multi_builder_empty_injects_nothing() {}

#[test]
#[ignore = "TreeArtifactValue.MultiBuilder has no caller at Bazel 9.3.2 and is not ported"]
fn multi_builder_injects_empty_tree_artifact() {}

#[test]
#[ignore = "TreeArtifactValue.MultiBuilder has no caller at Bazel 9.3.2 and is not ported"]
fn multi_builder_injects_single_tree_artifact() {}

#[test]
#[ignore = "TreeArtifactValue.MultiBuilder has no caller at Bazel 9.3.2 and is not ported"]
fn multi_builder_injects_multiple_tree_artifacts() {}

#[test]
#[ignore = "TreeArtifactValue.MultiBuilder and archived tree artifacts are not ported"]
fn multi_builder_injects_tree_artifact_with_archived_representation() {}

#[test]
#[ignore = "TreeArtifactValue.MultiBuilder and archived tree artifacts are not ported"]
fn multi_builder_injects_empty_tree_artifact_with_archived_representation() {}

#[test]
#[ignore = "TreeArtifactValue.MultiBuilder and archived tree artifacts are not ported"]
fn multi_builder_injects_tree_artifacts_with_and_without_archived_representation() {}

/// The remote half of `RemoteActionFileSystem.readdir` for a tree artifact whose files were not
/// downloaded: the injected files below the directory, relative to it, sorted.
#[test]
fn remote_output_tree_lists_the_files_under_a_directory() {
    let tree = RemoteOutputTree::new();
    let root = PathFragment::create("/execroot/bin/tree");
    tree.inject_file(root.get_relative_str("sub/b.txt"), metadata_with_id(2));
    tree.inject_file(root.get_relative_str("a.txt"), metadata_with_id(1));
    tree.inject_file(
        PathFragment::create("/execroot/bin/tree2/c.txt"),
        metadata_with_id(3),
    );
    tree.inject_file(PathFragment::create("/execroot/bin/x"), metadata_with_id(4));

    assert_eq!(
        tree.get_files_under(&root),
        vec![
            (PathFragment::create("a.txt"), metadata_with_id(1)),
            (PathFragment::create("sub/b.txt"), metadata_with_id(2)),
        ]
    );
    assert!(
        tree.get_files_under(&PathFragment::create("/execroot/bin/none"))
            .is_empty()
    );
}
