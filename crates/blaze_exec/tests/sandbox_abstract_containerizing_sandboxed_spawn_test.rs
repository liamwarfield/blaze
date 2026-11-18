//! Port of `AbstractContainerizingSandboxedSpawnTest.java`.
//!
//! PORT: Java uses an `InMemoryFileSystem`; these tests use the real file system under a fresh
//! temporary directory.

use std::io;
use std::sync::Arc;

use blaze_exec::sandbox::SynchronousTreeDeleter;
use blaze_exec::sandbox::abstract_containerizing_sandboxed_spawn::{
    AbstractContainerizingSandboxedSpawn, ContainerizingSandboxedSpawnHooks,
};
use blaze_exec::sandbox::sandbox_helpers::{self, SandboxInputs, SandboxOutputs};
use blaze_vfs::{
    DigestHashFunction, FileSystemExt, FileSystemRef, Path, PathFragment, Symlinks, UnixFileSystem,
};
use indexmap::{IndexMap, IndexSet};

struct Fixture {
    _tmp: tempfile::TempDir,
    root: Path,
    sandbox_path: Path,
    sandbox_exec_root: Path,
}

impl Fixture {
    // Java: AbstractContainerizingSandboxedSpawnTest.createSandboxExecRoot
    fn new() -> Fixture {
        let tmp = tempfile::tempdir().unwrap();
        let fs: FileSystemRef = UnixFileSystem::create(DigestHashFunction::Sha256);
        let root = fs.get_path(tmp.path().to_str().unwrap());
        let sandbox_path = root.get_relative_str("sandbox");
        sandbox_path.create_directory_and_parents().unwrap();
        let sandbox_exec_root = root.get_relative_str("sandbox/execroot");
        sandbox_exec_root.create_directory_and_parents().unwrap();
        Fixture {
            _tmp: tmp,
            root,
            sandbox_path,
            sandbox_exec_root,
        }
    }

    fn create_containerizing_sandboxed_spawn(
        &self,
        sandbox_inputs: SandboxInputs,
        sandbox_outputs: SandboxOutputs,
    ) -> TestSpawn {
        TestSpawn {
            base: AbstractContainerizingSandboxedSpawn::new(
                self.sandbox_path.clone(),
                self.sandbox_exec_root.clone(),
                /* arguments= */ Vec::new(),
                /* environment= */ IndexMap::new(),
                sandbox_inputs,
                sandbox_outputs,
                /* writable_dirs= */ IndexSet::new(),
                // PORT: Java passes mock(TreeDeleter.class); createFileSystem never deletes.
                Arc::new(SynchronousTreeDeleter),
                /* sandbox_debug_path= */ None,
                /* statistics_path= */ None,
                "Mnemonic".to_string(),
            ),
        }
    }
}

/// The anonymous subclass of the Java test, whose `copyFile` must not be called.
struct TestSpawn {
    base: AbstractContainerizingSandboxedSpawn,
}

impl ContainerizingSandboxedSpawnHooks for TestSpawn {
    fn copy_file(&self, _source: &Path, _target: &Path) -> io::Result<()> {
        panic!("UnsupportedOperationException");
    }
}

impl TestSpawn {
    fn create_file_system(&self) -> io::Result<()> {
        self.base.create_file_system(self)
    }
}

fn create_empty_file(p: &Path) {
    p.get_output_stream().unwrap();
}

#[test]
fn test_move_outputs() {
    let tmp = tempfile::tempdir().unwrap();
    let file_system: FileSystemRef = UnixFileSystem::create(DigestHashFunction::Sha256);
    let test_root = file_system.get_path(tmp.path().to_str().unwrap());
    test_root.create_directory_and_parents().unwrap();

    let exec_root = test_root.get_relative_str("execroot");
    exec_root.create_directory().unwrap();

    let output_file = exec_root.get_relative_str("very/output.txt");
    let output_link = exec_root.get_relative_str("very/output.link");
    let output_dangling = exec_root.get_relative_str("very/output.dangling");
    let output_dir = exec_root.get_relative_str("very/output.dir");
    let output_in_uncreated_target_dir = exec_root.get_relative_str("uncreated/output.txt");

    let outputs: Vec<PathFragment> = vec![
        output_file.relative_to(&exec_root),
        output_link.relative_to(&exec_root),
        output_dangling.relative_to(&exec_root),
        output_in_uncreated_target_dir.relative_to(&exec_root),
    ];
    let output_dirs: Vec<PathFragment> = vec![output_dir.relative_to(&exec_root)];
    for path in &outputs {
        exec_root
            .get_relative(path)
            .get_parent_directory()
            .unwrap()
            .create_directory_and_parents()
            .unwrap();
    }
    for path in &output_dirs {
        exec_root
            .get_relative(path)
            .create_directory_and_parents()
            .unwrap();
    }

    create_empty_file(&output_file);
    output_link
        .create_symbolic_link_to_fragment(&PathFragment::create("output.txt"))
        .unwrap();
    output_dangling
        .create_symbolic_link_to_fragment(&PathFragment::create("doesnotexist"))
        .unwrap();
    output_dir.create_directory().unwrap();
    create_empty_file(&output_dir.get_relative_str("test.txt"));
    create_empty_file(&output_in_uncreated_target_dir);

    let outputs_dir = test_root.get_relative_str("outputs");
    outputs_dir.create_directory().unwrap();
    outputs_dir
        .get_relative_str("very")
        .create_directory()
        .unwrap();
    sandbox_helpers::move_outputs(
        &SandboxOutputs::create_from_sets(outputs, output_dirs),
        &exec_root,
        &outputs_dir,
    )
    .unwrap();

    assert!(
        outputs_dir
            .get_relative_str("very/output.txt")
            .is_file_with(Symlinks::Nofollow)
    );
    assert!(
        outputs_dir
            .get_relative_str("very/output.link")
            .is_symbolic_link()
    );
    assert_eq!(
        outputs_dir
            .get_relative_str("very/output.link")
            .resolve_symbolic_links()
            .unwrap(),
        outputs_dir.get_relative_str("very/output.txt")
    );
    assert!(
        outputs_dir
            .get_relative_str("very/output.dangling")
            .is_symbolic_link()
    );
    assert!(
        outputs_dir
            .get_relative_str("very/output.dangling")
            .resolve_symbolic_links()
            .is_err()
    );
    assert!(
        outputs_dir
            .get_relative_str("very/output.dir")
            .is_directory_with(Symlinks::Nofollow)
    );
    assert!(
        outputs_dir
            .get_relative_str("very/output.dir/test.txt")
            .is_file_with(Symlinks::Nofollow)
    );
    assert!(
        outputs_dir
            .get_relative_str("uncreated/output.txt")
            .is_file_with(Symlinks::Nofollow)
    );
}

#[test]
#[ignore = "counts GoogleLogger warnings; logging is dropped in the port"]
fn test_move_outputs_warn_once_if_copy_happened() {}

/// `PathEntry`: a relative path and its type.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum PathEntry {
    File(PathFragment),
    Directory(PathFragment),
    Symlink(PathFragment),
}

fn file(path: &str) -> PathEntry {
    PathEntry::File(PathFragment::create(path))
}

fn directory(path: &str) -> PathEntry {
    PathEntry::Directory(PathFragment::create(path))
}

fn symlink(path: &str) -> PathEntry {
    PathEntry::Symlink(PathFragment::create(path))
}

/// Return a list of all entries under the provided directory recursively
/// (`FileSystemUtils.traverseTree`).
fn list_directory(directory: &Path) -> Vec<PathEntry> {
    let mut result = Vec::new();
    fn visit(dir: &Path, root: &Path, result: &mut Vec<PathEntry>) {
        for child in dir.get_directory_entries().unwrap() {
            let relative_path = child.as_fragment().relative_to(root.as_fragment());
            let stat = child.stat_with(Symlinks::Nofollow).unwrap();
            if stat.is_file() {
                result.push(PathEntry::File(relative_path));
            } else if stat.is_directory() {
                result.push(PathEntry::Directory(relative_path));
                visit(&child, root, result);
            } else if stat.is_symbolic_link() {
                result.push(PathEntry::Symlink(relative_path));
            } else {
                panic!("Unexpected file type for {}", child);
            }
        }
    }
    visit(directory, directory, &mut result);
    result
}

/// `assertThat(actual).containsExactly(expected...)` (order-insensitive).
fn assert_contains_exactly(actual: Vec<PathEntry>, expected: Vec<PathEntry>) {
    let mut a = actual;
    let mut e = expected;
    a.sort_by_key(|p| format!("{:?}", p));
    e.sort_by_key(|p| format!("{:?}", p));
    assert_eq!(a, e);
}

fn create_sandbox_inputs(files: &[&str], symlinks: &[(&str, &str)]) -> SandboxInputs {
    let files_map: IndexMap<PathFragment, Option<Path>> = files
        .iter()
        .map(|f| (PathFragment::create(f), None))
        .collect();
    SandboxInputs::new(
        files_map,
        /* virtual_inputs= */ IndexMap::new(),
        symlinks
            .iter()
            .map(|(k, v)| (PathFragment::create(k), PathFragment::create(v)))
            .collect(),
    )
}

fn symlinks_anywhere<'a>(symlinks: &[&'a str]) -> Vec<(&'a str, &'static str)> {
    symlinks.iter().map(|s| (*s, "anywhere")).collect()
}

#[test]
fn create_file_system_creates_directories_for_and_input_files() {
    let f = Fixture::new();
    let sandbox_inputs = create_sandbox_inputs(&["a/b"], &symlinks_anywhere(&[]));
    let sandbox_outputs = SandboxOutputs::create_from_sets([], []);
    let sandboxed_spawn = f.create_containerizing_sandboxed_spawn(sandbox_inputs, sandbox_outputs);

    sandboxed_spawn.create_file_system().unwrap();

    assert_contains_exactly(
        list_directory(&f.sandbox_exec_root),
        vec![directory("a"), file("a/b")],
    );
}

#[test]
fn create_file_system_creates_directories_for_and_input_symlinks() {
    let f = Fixture::new();
    let sandbox_inputs = create_sandbox_inputs(&[], &symlinks_anywhere(&["a/b/c"]));
    let sandbox_outputs = SandboxOutputs::create_from_sets([], []);
    let sandboxed_spawn = f.create_containerizing_sandboxed_spawn(sandbox_inputs, sandbox_outputs);

    sandboxed_spawn.create_file_system().unwrap();

    assert_contains_exactly(
        list_directory(&f.sandbox_exec_root),
        vec![directory("a"), directory("a/b"), symlink("a/b/c")],
    );
}

#[test]
fn create_file_system_uplevel_reference_creates_sibling_directory() {
    let f = Fixture::new();
    let sandbox_inputs = create_sandbox_inputs(&["../a/b"], &symlinks_anywhere(&[]));
    let sandbox_outputs = SandboxOutputs::create_from_sets([], []);
    let sandboxed_spawn = f.create_containerizing_sandboxed_spawn(sandbox_inputs, sandbox_outputs);

    sandboxed_spawn.create_file_system().unwrap();

    assert_contains_exactly(
        list_directory(&f.sandbox_exec_root.get_parent_directory().unwrap()),
        vec![directory("a"), file("a/b"), directory("execroot")],
    );
}

#[test]
fn create_file_system_creates_directories_for_output_files() {
    let f = Fixture::new();
    let sandbox_inputs = create_sandbox_inputs(&[], &symlinks_anywhere(&[]));
    let sandbox_outputs = SandboxOutputs::create_from_sets(
        [PathFragment::create("a/b"), PathFragment::create("c/d/e")],
        [],
    );
    let sandboxed_spawn = f.create_containerizing_sandboxed_spawn(sandbox_inputs, sandbox_outputs);

    sandboxed_spawn.create_file_system().unwrap();

    assert_contains_exactly(
        list_directory(&f.sandbox_exec_root),
        vec![directory("a"), directory("c"), directory("c/d")],
    );
}

#[test]
fn create_file_system_creates_output_directories() {
    let f = Fixture::new();
    let sandbox_inputs = create_sandbox_inputs(&[], &symlinks_anywhere(&[]));
    let sandbox_outputs = SandboxOutputs::create_from_sets(
        [],
        [PathFragment::create("a/b"), PathFragment::create("c/d/e")],
    );
    let sandboxed_spawn = f.create_containerizing_sandboxed_spawn(sandbox_inputs, sandbox_outputs);

    sandboxed_spawn.create_file_system().unwrap();

    assert_contains_exactly(
        list_directory(&f.sandbox_exec_root),
        vec![
            directory("a"),
            directory("a/b"),
            directory("c"),
            directory("c/d"),
            directory("c/d/e"),
        ],
    );
}

#[test]
fn create_file_system_nested_file_and_directory_creates_directories_and_file() {
    let f = Fixture::new();
    let sandbox_inputs = create_sandbox_inputs(&["a/b/file"], &symlinks_anywhere(&[]));
    let sandbox_outputs = SandboxOutputs::create_from_sets([], [PathFragment::create("a")]);
    let sandboxed_spawn = f.create_containerizing_sandboxed_spawn(sandbox_inputs, sandbox_outputs);

    sandboxed_spawn.create_file_system().unwrap();

    assert_contains_exactly(
        list_directory(&f.sandbox_exec_root),
        vec![directory("a"), directory("a/b"), file("a/b/file")],
    );
}

#[test]
fn create_file_system_overlapping_paths_creates_all_directories() {
    let f = Fixture::new();
    let sandbox_inputs =
        create_sandbox_inputs(&["1/2/file1"], &symlinks_anywhere(&["1/2/3/symlink"]));
    let sandbox_outputs = SandboxOutputs::create_from_sets(
        [PathFragment::create("1/2/file2")],
        [PathFragment::create("1"), PathFragment::create("2/3/4")],
    );
    let sandboxed_spawn = f.create_containerizing_sandboxed_spawn(sandbox_inputs, sandbox_outputs);

    sandboxed_spawn.create_file_system().unwrap();

    assert_contains_exactly(
        list_directory(&f.sandbox_exec_root),
        vec![
            directory("1"),
            directory("1/2"),
            file("1/2/file1"),
            directory("1/2/3"),
            symlink("1/2/3/symlink"),
            directory("2"),
            directory("2/3"),
            directory("2/3/4"),
        ],
    );
}

#[test]
#[should_panic(expected = "IllegalArgumentException")]
fn create_file_system_file_in_up_up_level_reference_fails() {
    let f = Fixture::new();
    let sandbox_inputs = create_sandbox_inputs(&["../../file"], &symlinks_anywhere(&[]));
    let sandbox_outputs = SandboxOutputs::create_from_sets([], []);
    let sandboxed_spawn = f.create_containerizing_sandboxed_spawn(sandbox_inputs, sandbox_outputs);

    let _ = sandboxed_spawn.create_file_system();
}

#[test]
fn create_file_system_overlapping_symlink_and_parent_creates_correct_parents_and_fails() {
    let f = Fixture::new();
    let sandbox_inputs = create_sandbox_inputs(&["1/2/3/file", "1/4/file"], &[("1/2", "4")]);
    let sandbox_outputs = SandboxOutputs::create_from_sets([], []);
    let sandboxed_spawn = f.create_containerizing_sandboxed_spawn(sandbox_inputs, sandbox_outputs);

    assert!(sandboxed_spawn.create_file_system().is_err());

    let entries = list_directory(&f.sandbox_exec_root);
    for e in [
        directory("1"),
        directory("1/2"),
        directory("1/2/3"),
        directory("1/4"),
    ] {
        assert!(entries.contains(&e), "missing {:?} in {:?}", e, entries);
    }
    assert!(!entries.contains(&directory("1/4/3")));
    let _ = &f.root;
}
