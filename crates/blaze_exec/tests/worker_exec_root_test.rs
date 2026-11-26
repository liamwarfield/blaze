//! Port of src/test/java/com/google/devtools/build/lib/worker/WorkerExecRootTest.java.
//!
//! PORT: Java uses an `InMemoryFileSystem`; these tests use the real file system under a fresh
//! temporary directory.

mod worker_common;

use blaze_exec::sandbox::SynchronousTreeDeleter;
use blaze_exec::worker::worker_exec_root::WorkerExecRoot;
use blaze_vfs::{DigestHashFunction, FileSystemExt, Path, PathFragment, UnixFileSystem};
use worker_common::sandbox_helper::{
    SandboxHelper, create_empty_file, read_content, write_content_as_latin1,
};

struct Dirs {
    _tmp: tempfile::TempDir,
    /// The global execroot directory, where real files live.
    exec_root: Path,
    /// The `workDir` directory of the worker. This is the CWD of the worker process.
    work_dir: Path,
}

// Java: src/test/java/com/google/devtools/build/lib/worker/WorkerExecRootTest.java:48 WorkerExecRootTest.setupTestDirs
fn setup_test_dirs() -> Dirs {
    let tmp = tempfile::tempdir().unwrap();
    let fs = UnixFileSystem::create(DigestHashFunction::Sha256);
    let test_root = fs.get_path(tmp.path().to_str().unwrap());
    test_root.create_directory_and_parents().unwrap();

    let exec_root = test_root.get_relative_str("workspace");
    exec_root.create_directory().unwrap();
    let work_dir = exec_root.get_relative_str("sandbox/execroot/__main__");
    work_dir.create_directory_and_parents().unwrap();
    Dirs {
        _tmp: tmp,
        exec_root,
        work_dir,
    }
}

fn create_file_system(root: &mut WorkerExecRoot, helper: &SandboxHelper) {
    root.create_file_system(
        &helper.get_worker_files(),
        &helper.get_sandbox_inputs(),
        &helper.get_sandbox_outputs(),
        &SynchronousTreeDeleter,
    )
    .unwrap();
}

// Java: src/test/java/com/google/devtools/build/lib/worker/WorkerExecRootTest.java:60 WorkerExecRootTest.cleanFileSystem
#[test]
fn clean_file_system() {
    let d = setup_test_dirs();
    let sandbox_helper = SandboxHelper::new(&d.exec_root, &d.work_dir)
        .add_and_create_input_file("worker.sh", "worker.sh", "#!/bin/bash")
        .add_output("very/output.txt")
        .add_worker_file("worker.sh");
    let worker_sh = d.exec_root.get_relative_str("worker.sh");

    let mut worker_exec_root = WorkerExecRoot::new(d.work_dir.clone(), Vec::new(), false);
    create_file_system(&mut worker_exec_root, &sandbox_helper);

    // Pretend to do some work inside the execRoot.
    d.work_dir
        .get_relative_str("tempdir")
        .create_directory()
        .unwrap();
    create_empty_file(&d.work_dir.get_relative_str("very/output.txt"));
    create_empty_file(&d.work_dir.get_relative_str("temp.txt"));
    // Modify the worker.sh so that we're able to detect whether it gets rewritten or not.
    write_content_as_latin1(&worker_sh, "#!/bin/sh");

    // Reuse the same execRoot.
    create_file_system(&mut worker_exec_root, &sandbox_helper);

    assert!(d.work_dir.get_relative_str("worker.sh").exists());
    assert_eq!(
        read_content(&d.work_dir.get_relative_str("worker.sh")),
        "#!/bin/sh"
    );
    assert!(!d.work_dir.get_relative_str("tempdir").exists());
    assert!(!d.work_dir.get_relative_str("very/output.txt").exists());
    assert!(!d.work_dir.get_relative_str("temp.txt").exists());
}

// Java: src/test/java/com/google/devtools/build/lib/worker/WorkerExecRootTest.java:99 WorkerExecRootTest.createsAndCleansInputSymlinks
#[test]
fn creates_and_cleans_input_symlinks() {
    let d = setup_test_dirs();
    // Simulate existing symlinks in the exec root to check that `WorkerExecRoot` correctly
    // deletes the unnecessary ones and updates the ones that don't point to the right target.
    let sandbox_helper = SandboxHelper::new(&d.exec_root, &d.work_dir)
        .create_symlink("dir/input_symlink_1", "old_content")
        .create_symlink("dir/input_symlink_2", "unchanged")
        .create_symlink("dir/input_symlink_3", "whatever")
        .add_symlink("dir/input_symlink_1", "new_content")
        .add_symlink("dir/input_symlink_2", "unchanged");

    let mut worker_exec_root = WorkerExecRoot::new(d.work_dir.clone(), Vec::new(), false);

    // This should update the `input_symlink_{1,2,3}` according to `SandboxInputs`, i.e., update
    // the first/second (alternatively leave the second unchanged) and delete the third.
    create_file_system(&mut worker_exec_root, &sandbox_helper);

    assert_eq!(
        d.work_dir
            .get_relative_str("dir/input_symlink_1")
            .read_symbolic_link()
            .unwrap(),
        PathFragment::create("new_content")
    );
    assert_eq!(
        d.work_dir
            .get_relative_str("dir/input_symlink_2")
            .read_symbolic_link()
            .unwrap(),
        PathFragment::create("unchanged")
    );
    // Path.exists() follows symlinks; the third symlink must be gone, and the other two are
    // dangling, so check without following.
    assert!(
        !d.work_dir
            .get_relative_str("dir/input_symlink_3")
            .exists_with(blaze_vfs::Symlinks::Nofollow)
    );
}

// Java: src/test/java/com/google/devtools/build/lib/worker/WorkerExecRootTest.java:126 WorkerExecRootTest.createsOutputDirs
#[test]
fn creates_output_dirs() {
    let d = setup_test_dirs();
    let sandbox_helper = SandboxHelper::new(&d.exec_root, &d.work_dir)
        .add_output("dir/foo/bar_kt.jar")
        .add_output("dir/foo/bar_kt.jdeps")
        .add_output("dir/foo/bar_kt-sources.jar")
        .add_output_dir("dir/foo/_kotlinc/bar_kt_jvm/bar_kt_sourcegenfiles")
        .add_output_dir("dir/foo/_kotlinc/bar_kt_jvm/bar_kt_classes")
        .add_output_dir("dir/foo/_kotlinc/bar_kt_jvm/bar_kt_temp")
        .add_output_dir("dir/foo/_kotlinc/bar_kt_jvm/bar_kt_generated_classes");
    let mut worker_exec_root = WorkerExecRoot::new(d.work_dir.clone(), Vec::new(), false);
    create_file_system(&mut worker_exec_root, &sandbox_helper);

    for dir in [
        "dir/foo/_kotlinc/bar_kt_jvm/bar_kt_sourcegenfiles",
        "dir/foo/_kotlinc/bar_kt_jvm/bar_kt_classes",
        "dir/foo/_kotlinc/bar_kt_jvm/bar_kt_temp",
        "dir/foo/_kotlinc/bar_kt_jvm/bar_kt_generated_classes",
    ] {
        assert!(d.work_dir.get_relative_str(dir).exists(), "{dir}");
    }
}

// Java: src/test/java/com/google/devtools/build/lib/worker/WorkerExecRootTest.java:149 WorkerExecRootTest.workspaceFilesAreNotDeleted
#[test]
fn workspace_files_are_not_deleted() {
    let d = setup_test_dirs();
    // We want to check that `WorkerExecRoot` deletes pre-existing symlinks if they're not in the
    // `SandboxInputs`, but it does not delete the files that the symlinks point to (i.e., the
    // files in the workspace directory).
    let needed_workspace_file = d.exec_root.get_relative_str("needed_file");
    let other_workspace_file = d.exec_root.get_relative_str("other_file");

    let sandbox_helper = SandboxHelper::new(&d.exec_root, &d.work_dir)
        .add_input_file("needed_file", Some("needed_file"))
        .create_workspace_dir_file("needed_file", "needed workspace content")
        .create_workspace_dir_file("other_file", "other workspace content")
        .create_symlink("needed_file", needed_workspace_file.get_path_string())
        .create_symlink("other_file", other_workspace_file.get_path_string());

    let mut worker_exec_root = WorkerExecRoot::new(d.work_dir.clone(), Vec::new(), false);
    create_file_system(&mut worker_exec_root, &sandbox_helper);

    assert_eq!(
        &d.work_dir
            .get_relative_str("needed_file")
            .read_symbolic_link()
            .unwrap(),
        needed_workspace_file.as_fragment()
    );
    assert!(!d.work_dir.get_relative_str("other_file").exists());

    assert_eq!(
        read_content(&needed_workspace_file),
        "needed workspace content"
    );
    assert_eq!(
        read_content(&other_workspace_file),
        "other workspace content"
    );
}

// Java: src/test/java/com/google/devtools/build/lib/worker/WorkerExecRootTest.java:180 WorkerExecRootTest.recreatesEmptyFiles
#[test]
fn recreates_empty_files() {
    let d = setup_test_dirs();
    // Simulate existing non-empty file in the exec root to check that `WorkerExecRoot` will clear
    // the contents as requested by `SandboxInputs`.
    // This is interesting, because the filepath is a key in `SandboxInputs`, but its value is
    // `null`, which means "create an empty file". So after `createFileSystem` the file should be
    // empty.
    let sandbox_helper = SandboxHelper::new(&d.exec_root, &d.work_dir)
        .create_exec_root_file("some_file", "some content")
        .add_input_file("some_file", None);

    let mut worker_exec_root =
        WorkerExecRoot::new(sandbox_helper.work_dir.clone(), Vec::new(), false);
    create_file_system(&mut worker_exec_root, &sandbox_helper);

    assert!(read_content(&d.work_dir.get_relative_str("some_file")).is_empty());
}

// Java: src/test/java/com/google/devtools/build/lib/worker/WorkerExecRootTest.java:205 WorkerExecRootTest.createsAndDeletesSiblingExternalRepoFiles
#[test]
fn creates_and_deletes_sibling_external_repo_files() {
    let d = setup_test_dirs();
    // With the sibling repository layout, external repository source files are no longer
    // symlinked under <execroot>/external/<repo name>/<path>. Instead, they become *siblings* of
    // the main workspace files in that they're placed at <execroot>/../<repo name>/<path>.
    // Simulate this layout and check if inputs are correctly created and irrelevant symlinks are
    // deleted.

    let foo_repo_dir = d.exec_root.get_relative_str("external_dir/foo");
    let input1 = foo_repo_dir.get_relative_str("bar/input1");
    let input2 = foo_repo_dir.get_relative_str("input2");
    let random = foo_repo_dir.get_relative_str("bar/random");

    let sandbox_helper = SandboxHelper::new(&d.exec_root, &d.work_dir)
        .create_workspace_dir_file("external_dir/foo/bar/input1", "This is input1.")
        .create_workspace_dir_file("external_dir/foo/input2", "This is input2.")
        .create_workspace_dir_file("external_dir/foo/bar/random", "This is random.")
        .create_symlink("../foo/bar/input1", input1.get_path_string())
        .create_symlink("../foo/bar/random", random.get_path_string())
        .add_input_file("../foo/bar/input1", Some(input1.get_path_string()))
        .add_input_file("../foo/input2", Some(input2.get_path_string()));

    let mut worker_exec_root = WorkerExecRoot::new(d.work_dir.clone(), Vec::new(), false);
    create_file_system(&mut worker_exec_root, &sandbox_helper);

    assert_eq!(
        &d.work_dir
            .get_relative_str("../foo/bar/input1")
            .read_symbolic_link()
            .unwrap(),
        input1.as_fragment()
    );
    assert_eq!(
        &d.work_dir
            .get_relative_str("../foo/input2")
            .read_symbolic_link()
            .unwrap(),
        input2.as_fragment()
    );
    assert!(!d.work_dir.get_relative_str("bar/random").exists());
}
