//! Port of the action-file-system cases of `ActionOutputDirectoryHelperTest`
//! (src/test/java/com/google/devtools/build/lib/actions/ActionOutputDirectoryHelperTest.java).
//!
//! PORT: Java's `Scratch` file system is an `InMemoryFileSystem`; the action file system here is
//! `RemoteInMemoryFileSystem` (the in-memory file system Blaze has), and the main file system is
//! the local one under a temporary directory.

mod analysis_actions_common;

use std::sync::Arc;

use analysis_actions_common::{NullArtifactOwner, TestDirs};
use blaze_actions::actions::artifact::SpecialArtifactType;
use blaze_actions::{ActionLookupKeyRef, Artifact, ArtifactRoot, RootType};
use blaze_exec::actions::action_output_directory_helper::ActionOutputDirectoryHelper;
use blaze_exec::exec::remote_action_file_system::RemoteInMemoryFileSystem;
use blaze_vfs::{DigestHashFunction, DirentType, FileSystemExt, FileSystemRef, Path, PathFragment};

/// `OutputSet`: file outputs, tree outputs and the expected leaf directories.
/// (name, file outputs, tree outputs, expected leaf directories)
type OutputSet = (
    &'static str,
    &'static [&'static str],
    &'static [&'static str],
    &'static [&'static str],
);

const OUTPUT_SETS: &[OutputSet] = &[
    ("SINGLE_FILE", &["a/b"], &[], &["a"]),
    (
        "DEEP_DIRECTORY_STRUCTURE",
        &["a/b/c/d/e/f/g/h/i/j/k/l/m"],
        &[],
        &["a/b/c/d/e/f/g/h/i/j/k/l"],
    ),
    (
        "MULTIPLE_FILES",
        &["a/b/c", "a/c", "a/d/1", "a/d/2"],
        &[],
        &["a/b", "a/d"],
    ),
    ("TREE_OUTPUT", &[], &["a/b"], &["a/b"]),
];

/// `createOutputRoot(execRoot)`: `ArtifactRoot.asDerivedRoot(execRoot, RootType.OUTPUT, "out")`.
fn create_output_root(exec_root: &Path) -> ArtifactRoot {
    ArtifactRoot::as_derived_root(exec_root, RootType::OUTPUT, &["out"])
}

/// `createOutput(outputRoot, relativeExecPath)`.
fn create_output(output_root: &ArtifactRoot, relative_exec_path: &str) -> Artifact {
    Artifact::create_derived_artifact(
        output_root.clone(),
        output_root
            .get_exec_path()
            .get_relative(&PathFragment::create(relative_exec_path)),
        ActionLookupKeyRef::new(NullArtifactOwner),
    )
}

/// `createTreeOutput(relativeExecPath)`.
fn create_tree_output(output_root: &ArtifactRoot, relative_exec_path: &str) -> Artifact {
    Artifact::create_special_artifact(
        output_root.clone(),
        output_root
            .get_exec_path()
            .get_relative(&PathFragment::create(relative_exec_path)),
        ActionLookupKeyRef::new(NullArtifactOwner),
        SpecialArtifactType::TREE,
    )
}

/// `DirectoryListingHelper.leafDirectoryEntries(path)`, directories only (the test creates no
/// files): the paths, relative to `root`, of the directories without subdirectories.
fn leaf_directory_entries(root: &Path) -> Vec<String> {
    fn visit(root: &Path, dir: &Path, out: &mut Vec<String>) {
        let mut entries = dir.readdir(blaze_vfs::Symlinks::Nofollow).unwrap();
        entries.sort();
        if entries.is_empty() && dir != root {
            out.push(dir.relative_to(root).get_path_string().to_string());
        }
        for entry in entries {
            assert_eq!(entry.get_type(), DirentType::Directory, "{entry:?}");
            visit(root, &dir.get_child(entry.get_name()), out);
        }
    }
    let mut out = Vec::new();
    visit(root, root, &mut out);
    out.sort();
    out
}

/// The test's `execRoot` and `outputRoot` (`createArtifactRootAndOutputDirectoryHelper`).
fn output_root(dirs: &TestDirs) -> ArtifactRoot {
    create_output_root(&dirs.exec_root)
}

// Java: src/test/java/com/google/devtools/build/lib/actions/ActionOutputDirectoryHelperTest.java:115 ActionOutputDirectoryHelperTest.createOutputDirectories_createsExpectedDirectories
// PORT: @TestParameter DirectoryCache: only CACHE_ENABLED; Blaze's knownDirectories has no
// Caffeine spec (`--directory_creation_cache`), so CACHE_DISABLED (maximumSize=0) cannot be
// expressed.
#[test]
fn create_output_directories_creates_expected_directories() {
    // @TestParameter OutputSet outputSet
    for (name, file_outputs, tree_outputs, expected_directories) in OUTPUT_SETS {
        let dirs = TestDirs::new();
        let output_root = output_root(&dirs);
        let output_directory_helper = ActionOutputDirectoryHelper::new();
        let mut outputs: Vec<Artifact> = file_outputs
            .iter()
            .map(|p| create_output(&output_root, p))
            .collect();
        outputs.extend(
            tree_outputs
                .iter()
                .map(|p| create_tree_output(&output_root, p)),
        );

        output_directory_helper
            .create_output_directories(&outputs)
            .unwrap();

        assert_eq!(
            leaf_directory_entries(output_root.get_root().as_path().unwrap()),
            expected_directories
                .iter()
                .map(|d| d.to_string())
                .collect::<Vec<_>>(),
            "{name}"
        );
    }
}

// Java: src/test/java/com/google/devtools/build/lib/actions/ActionOutputDirectoryHelperTest.java:126 ActionOutputDirectoryHelperTest.createOutputDirectories_makesOutputDirectoryWritable
#[test]
fn create_output_directories_makes_output_directory_writable() {
    let dirs = TestDirs::new();
    let output_root = output_root(&dirs);
    let output_directory_helper = ActionOutputDirectoryHelper::new();
    let file_output = create_output(&output_root, "dir/file");
    let parent_dir = file_output.get_path().get_parent_directory().unwrap();
    parent_dir.create_directory_and_parents().unwrap();
    parent_dir.set_writable(false).unwrap();
    parent_dir.set_executable(false).unwrap();

    output_directory_helper
        .create_output_directories(std::slice::from_ref(&file_output))
        .unwrap();

    assert!(parent_dir.is_directory());
    assert!(parent_dir.is_readable().unwrap());
    assert!(parent_dir.is_writable().unwrap());
    assert!(parent_dir.is_executable().unwrap());
}

// Java: src/test/java/com/google/devtools/build/lib/actions/ActionOutputDirectoryHelperTest.java:143 ActionOutputDirectoryHelperTest.createOutputDirectories_overwritesExistingFileAtParentPath
#[test]
fn create_output_directories_overwrites_existing_file_at_parent_path() {
    let dirs = TestDirs::new();
    let output_root = output_root(&dirs);
    let output_directory_helper = ActionOutputDirectoryHelper::new();
    let file_output = create_output(&output_root, "dir/file");
    let parent_path = file_output.get_path().get_parent_directory().unwrap();
    parent_path
        .get_parent_directory()
        .unwrap()
        .create_directory_and_parents()
        .unwrap();
    std::fs::write(parent_path.get_path_string(), "garbage").unwrap();
    parent_path.set_writable(false).unwrap();

    output_directory_helper
        .create_output_directories(std::slice::from_ref(&file_output))
        .unwrap();

    assert!(parent_path.is_directory());
    assert!(parent_path.is_readable().unwrap());
    assert!(parent_path.is_writable().unwrap());
    assert!(parent_path.is_executable().unwrap());
}

// Java: src/test/java/com/google/devtools/build/lib/actions/ActionOutputDirectoryHelperTest.java:160 ActionOutputDirectoryHelperTest.createOutputDirectories_overwritesExistingFileAtGrandparentPath
#[test]
fn create_output_directories_overwrites_existing_file_at_grandparent_path() {
    let dirs = TestDirs::new();
    let output_root = output_root(&dirs);
    let output_directory_helper = ActionOutputDirectoryHelper::new();
    let file_output = create_output(&output_root, "dir/subdir/file");
    let parent_path = file_output.get_path().get_parent_directory().unwrap();
    let grandparent_path = parent_path.get_parent_directory().unwrap();
    grandparent_path
        .get_parent_directory()
        .unwrap()
        .create_directory_and_parents()
        .unwrap();
    std::fs::write(grandparent_path.get_path_string(), "garbage").unwrap();
    grandparent_path.set_writable(false).unwrap();

    output_directory_helper
        .create_output_directories(std::slice::from_ref(&file_output))
        .unwrap();

    assert!(parent_path.is_directory());
    assert!(parent_path.is_readable().unwrap());
    assert!(parent_path.is_writable().unwrap());
    assert!(parent_path.is_executable().unwrap());
}

// Java: src/test/java/com/google/devtools/build/lib/actions/ActionOutputDirectoryHelperTest.java:196 ActionOutputDirectoryHelperTest.createOutputDirectories_ioExceptionWhenCreatingDirectory_fails
#[test]
#[ignore = "needs createFileSystemInjectingException (a FileSystem whose createDirectory fails with a given IOException) and DelegateFileSystem, which are not ported"]
fn create_output_directories_io_exception_when_creating_directory_fails() {}

// Java: src/test/java/com/google/devtools/build/lib/actions/ActionOutputDirectoryHelperTest.java:236 ActionOutputDirectoryHelperTest.invalidateTreeArtifactDirectoryCreation_onlyInvalidatesTreeArtifactDirs
#[test]
fn invalidate_tree_artifact_directory_creation_only_invalidates_tree_artifact_dirs() {
    let dirs = TestDirs::new();
    let output_root = output_root(&dirs);
    let output_directory_helper = ActionOutputDirectoryHelper::new();
    let regular_output = create_output(&output_root, "example/regular/file.txt");
    let tree_output = create_tree_output(&output_root, "example/tree/tree_dir");
    let outputs = vec![regular_output.clone(), tree_output.clone()];

    output_directory_helper
        .create_output_directories(&outputs)
        .unwrap();
    regular_output
        .get_path()
        .get_parent_directory()
        .unwrap()
        .delete_tree()
        .unwrap();
    tree_output.get_path().delete_tree().unwrap();
    output_directory_helper.invalidate_tree_artifact_directory_creation(&outputs);
    output_directory_helper
        .create_output_directories(&outputs)
        .unwrap();

    // Only tree artifact directories are recreated.
    assert!(
        !regular_output
            .get_path()
            .get_parent_directory()
            .unwrap()
            .exists()
    );
    assert!(tree_output.get_path().exists());
}

// Java: src/test/java/com/google/devtools/build/lib/actions/ActionOutputDirectoryHelperTest.java:180 ActionOutputDirectoryHelperTest.createActionFsOutputDirectories_createsExpectedDirectoriesInActionFs
#[test]
fn create_action_fs_output_directories_creates_expected_directories_in_action_fs() {
    // @TestParameter OutputSet outputSet
    for (name, file_outputs, tree_outputs, expected_directories) in OUTPUT_SETS {
        let dirs = TestDirs::new();
        let exec_root = dirs.exec_root.clone();
        let output_root = create_output_root(&exec_root);
        let output_directory_helper = ActionOutputDirectoryHelper::new();
        let action_file_system: FileSystemRef =
            Arc::new(RemoteInMemoryFileSystem::new(DigestHashFunction::Sha256));
        let mut outputs: Vec<Artifact> = file_outputs
            .iter()
            .map(|p| create_output(&output_root, p))
            .collect();
        outputs.extend(
            tree_outputs
                .iter()
                .map(|p| create_tree_output(&output_root, p)),
        );

        output_directory_helper
            .create_action_fs_output_directories(&outputs, Some(&action_file_system))
            .unwrap();

        let output_root_path = output_root.get_root().as_path().unwrap().clone();
        assert!(!output_root_path.exists(), "{name}");
        assert_eq!(
            leaf_directory_entries(
                &action_file_system.get_path_fragment(output_root_path.as_fragment())
            ),
            expected_directories
                .iter()
                .map(|d| d.to_string())
                .collect::<Vec<_>>(),
            "{name}"
        );
    }
}

// Java: src/test/java/com/google/devtools/build/lib/actions/ActionOutputDirectoryHelperTest.java:216 ActionOutputDirectoryHelperTest.createActionFsOutputDirectories_ioExceptionWhenCreatingDirectory_fails
#[test]
#[ignore = "needs createFileSystemInjectingException (a FileSystem whose createDirectory fails with a given IOException) and DelegateFileSystem, which are not ported"]
fn create_action_fs_output_directories_io_exception_when_creating_directory_fails() {}
