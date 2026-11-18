//! Port of src/test/cpp/workspace_layout_test.cc.

use blaze_client::util::file::{make_directories, remove_recursively, write_file};
use blaze_client::util::path::{Path, join_path};
use blaze_client::workspace_layout::WorkspaceLayout;

// Java: src/test/cpp/workspace_layout_test.cc:50 WorkspaceLayoutTest.GetWorkspace
#[test]
fn get_workspace() {
    let tmp = tempfile::tempdir().unwrap();
    let build_root = join_path(tmp.path().to_str().unwrap(), "build_root");
    let workspace_layout = WorkspaceLayout::new();
    assert!(make_directories(&build_root, 0o755));

    assert!(write_file(b"", &join_path(&build_root, "WORKSPACE"), 0o755));
    let foobar = join_path(&build_root, "foo/bar");
    assert!(make_directories(&foobar, 0o755));

    // "" is returned when there's no workspace path.
    assert_eq!("", workspace_layout.get_workspace("foo/bar"));
    assert!(!workspace_layout.in_workspace("foo/bar"));

    assert_eq!(build_root, workspace_layout.get_workspace(&build_root));
    assert!(workspace_layout.in_workspace(&build_root));

    assert_eq!(build_root, workspace_layout.get_workspace(&foobar));
    assert!(!workspace_layout.in_workspace(&foobar));

    // Now write a REPO.bazel file in foo/bar. It becomes the new workspace root.
    assert!(write_file(b"", &join_path(&foobar, "REPO.bazel"), 0o755));
    assert_eq!(foobar, workspace_layout.get_workspace(&foobar));
    assert!(workspace_layout.in_workspace(&foobar));

    assert!(remove_recursively(&Path::new(&build_root)));
}

#[test]
fn boundary_files_and_pretty_name() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().to_str().unwrap().to_string();
    let workspace_layout = WorkspaceLayout::new();
    for marker in ["MODULE.bazel", "REPO.bazel", "WORKSPACE.bazel", "WORKSPACE"] {
        let ws = join_path(&root, marker.replace('.', "_").as_str());
        assert!(make_directories(&ws, 0o755));
        assert!(!workspace_layout.in_workspace(&ws));
        assert!(write_file(b"", &join_path(&ws, marker), 0o644));
        assert!(workspace_layout.in_workspace(&ws));
    }
    // A directory named like a marker does not count.
    let dir_marker = join_path(&root, "dir_marker");
    assert!(make_directories(
        &join_path(&dir_marker, "MODULE.bazel"),
        0o755
    ));
    assert!(!workspace_layout.in_workspace(&dir_marker));

    assert_eq!(
        "brotli",
        workspace_layout
            .get_pretty_workspace_name("/home/liamwarfield/.cache/blaze/work/w1/brotli")
    );
}
