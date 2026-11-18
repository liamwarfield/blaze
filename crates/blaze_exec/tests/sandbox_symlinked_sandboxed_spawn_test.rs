//! Port of `SymlinkedSandboxedSpawnTest.java`.
//!
//! PORT: Java uses an `InMemoryFileSystem` under `TestUtils.tmpDir()`; these tests use the real
//! file system under a fresh temporary directory.

use std::sync::Arc;

use blaze_exec::sandbox::SynchronousTreeDeleter;
use blaze_exec::sandbox::sandbox_helpers::{SandboxInputs, SandboxOutputs};
use blaze_exec::sandbox::sandboxed_spawn::SandboxedSpawn;
use blaze_exec::sandbox::symlinked_sandboxed_spawn::SymlinkedSandboxedSpawn;
use blaze_vfs::{
    DigestHashFunction, FileSystemExt, FileSystemRef, Path, PathFragment, Symlinks, UnixFileSystem,
};
use indexmap::{IndexMap, IndexSet};

struct Fixture {
    _tmp: tempfile::TempDir,
    workspace_dir: Path,
    sandbox_dir: Path,
    exec_root: Path,
    outputs_dir: Path,
}

impl Fixture {
    // Java: SymlinkedSandboxedSpawnTest.setupTestDirs
    fn new() -> Fixture {
        let tmp = tempfile::tempdir().unwrap();
        let file_system: FileSystemRef = UnixFileSystem::create(DigestHashFunction::Sha256);
        let test_root = file_system.get_path(tmp.path().to_str().unwrap());
        test_root.create_directory_and_parents().unwrap();

        let workspace_dir = test_root.get_relative_str("workspace");
        workspace_dir.create_directory().unwrap();
        let sandbox_dir = test_root.get_relative_str("sandbox");
        sandbox_dir.create_directory().unwrap();
        let exec_root = sandbox_dir.get_relative_str("execroot");
        exec_root.create_directory().unwrap();
        let outputs_dir = test_root.get_relative_str("outputs");
        outputs_dir.create_directory().unwrap();
        Fixture {
            _tmp: tmp,
            workspace_dir,
            sandbox_dir,
            exec_root,
            outputs_dir,
        }
    }
}

fn create_empty_file(p: &Path) {
    p.get_output_stream().unwrap();
}

#[test]
fn create_file_system() {
    let f = Fixture::new();
    let hello_txt = f.workspace_dir.get_relative_str("hello.txt");
    create_empty_file(&hello_txt);

    let symlinked_exec_root = SymlinkedSandboxedSpawn::new(
        f.sandbox_dir.clone(),
        f.exec_root.clone(),
        vec!["/bin/true".to_string()],
        IndexMap::new(),
        SandboxInputs::new(
            [(
                PathFragment::create("such/input.txt"),
                Some(hello_txt.clone()),
            )]
            .into_iter()
            .collect(),
            IndexMap::new(),
            IndexMap::new(),
        ),
        SandboxOutputs::create_from_sets([PathFragment::create("very/output.txt")], []),
        [f.exec_root.get_relative_str("wow/writable")]
            .into_iter()
            .collect::<IndexSet<_>>(),
        Arc::new(SynchronousTreeDeleter),
        /* sandbox_debug_path= */ None,
        /* statistics_path= */ None,
        /* interactive_debug_arguments= */ None,
        "SomeMnemonic",
        /* target_label= */ None,
    );

    symlinked_exec_root.create_file_system().unwrap();

    assert!(
        f.exec_root
            .get_relative_str("such/input.txt")
            .is_symbolic_link()
    );
    assert_eq!(
        f.exec_root
            .get_relative_str("such/input.txt")
            .resolve_symbolic_links()
            .unwrap(),
        hello_txt.resolve_symbolic_links().unwrap()
    );
    assert!(f.exec_root.get_relative_str("very").is_directory());
    assert!(f.exec_root.get_relative_str("wow/writable").is_directory());
}

#[test]
fn copy_outputs() {
    // These tests are very simple because we just rely on
    // AbstractContainerizingSandboxedSpawnTest.testMoveOutputs to properly verify all corner
    // cases.
    let f = Fixture::new();
    let output_file = f.exec_root.get_relative_str("very/output.txt");

    let symlinked_exec_root = SymlinkedSandboxedSpawn::new(
        f.sandbox_dir.clone(),
        f.exec_root.clone(),
        vec!["/bin/true".to_string()],
        IndexMap::new(),
        SandboxInputs::new(IndexMap::new(), IndexMap::new(), IndexMap::new()),
        SandboxOutputs::create_from_sets([output_file.relative_to(&f.exec_root)], []),
        IndexSet::new(),
        Arc::new(SynchronousTreeDeleter),
        /* sandbox_debug_path= */ None,
        /* statistics_path= */ None,
        /* interactive_debug_arguments= */ None,
        "SomeMnemonic",
        /* target_label= */ None,
    );
    symlinked_exec_root.create_file_system().unwrap();

    create_empty_file(&output_file);

    f.outputs_dir
        .get_relative_str("very")
        .create_directory()
        .unwrap();
    symlinked_exec_root.copy_outputs(&f.outputs_dir).unwrap();

    assert!(
        f.outputs_dir
            .get_relative_str("very/output.txt")
            .is_file_with(Symlinks::Nofollow)
    );
}
