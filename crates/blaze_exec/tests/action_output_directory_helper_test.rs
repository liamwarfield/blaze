//! Port of the action-file-system cases of `ActionOutputDirectoryHelperTest`
//! (src/test/java/com/google/devtools/build/lib/actions/ActionOutputDirectoryHelperTest.java).
//!
//! PORT: Java's `Scratch` file system is an `InMemoryFileSystem`; the action file system here is
//! `RemoteInMemoryFileSystem` (the in-memory file system Blaze has), and the main file system is
//! the local one under a temporary directory.

mod analysis_actions_common;

use std::io::{self, Read, Write};
use std::sync::Arc;

use analysis_actions_common::{NullArtifactOwner, TestDirs};
use blaze_actions::actions::artifact::SpecialArtifactType;
use blaze_actions::{ActionLookupKeyRef, Artifact, ArtifactRoot, RootType};
use blaze_exec::actions::action_output_directory_helper::ActionOutputDirectoryHelper;
use blaze_exec::exec::remote_action_file_system::RemoteInMemoryFileSystem;
use blaze_vfs::{
    DigestHashFunction, Dirent, DirentType, FileStatus, FileSystem, FileSystemExt, FileSystemRef,
    Path, PathFragment,
};

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

/// `IOException injectedException = new IOException("oh no!")`.
///
/// PORT: `io::Error` has no identity; `isSameInstanceAs(injectedException)` is checked by the
/// payload: an `InjectedException` with the id of the one the test created.
#[derive(Debug)]
struct InjectedException {
    id: usize,
}

impl std::fmt::Display for InjectedException {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("oh no!")
    }
}

impl std::error::Error for InjectedException {}

/// A fresh `injectedException`, distinct from every other one the tests create.
fn new_injected_exception_id() -> usize {
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(1);
    NEXT.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
}

/// `assertThat(e).hasCauseThat().isSameInstanceAs(injectedException)`.
fn assert_cause_is_injected(cause: &io::Error, id: usize) {
    let injected = cause
        .get_ref()
        .and_then(|e| e.downcast_ref::<InjectedException>())
        .unwrap_or_else(|| panic!("cause is not the injected exception: {cause:?}"));
    assert_eq!(injected.id, id);
    assert_eq!(cause.to_string(), "oh no!");
}

/// `new DelegateFileSystem(delegate) {}`, and, with a `failing_path`, the file system of
/// `createFileSystemInjectingException(failingPath, injectedException)`: `createDirectory` and
/// `createDirectoryAndParents` of `failing_path` throw the injected exception.
// Java: src/test/java/com/google/devtools/build/lib/actions/ActionOutputDirectoryHelperTest.java:255 ActionOutputDirectoryHelperTest.createFileSystemInjectingException
#[derive(Debug)]
struct DelegateFileSystem {
    delegate: FileSystemRef,
    failing_path: Option<(PathFragment, usize)>,
}

impl DelegateFileSystem {
    fn injected(&self, path: &PathFragment) -> io::Result<()> {
        match &self.failing_path {
            Some((failing_path, id)) if path == failing_path => {
                Err(io::Error::other(InjectedException { id: *id }))
            }
            _ => Ok(()),
        }
    }
}

impl FileSystem for DelegateFileSystem {
    fn get_digest_function(&self) -> DigestHashFunction {
        self.delegate.get_digest_function()
    }
    fn supports_modifications(&self, path: &PathFragment) -> bool {
        self.delegate.supports_modifications(path)
    }
    fn supports_symbolic_links_natively(&self, path: &PathFragment) -> bool {
        self.delegate.supports_symbolic_links_natively(path)
    }
    fn supports_hard_links_natively(&self, path: &PathFragment) -> bool {
        self.delegate.supports_hard_links_natively(path)
    }
    fn may_be_case_or_normalization_insensitive(&self) -> bool {
        self.delegate.may_be_case_or_normalization_insensitive()
    }
    fn create_directory(&self, path: &PathFragment) -> io::Result<bool> {
        self.injected(path)?;
        self.delegate.create_directory(path)
    }
    fn create_directory_and_parents(&self, path: &PathFragment) -> io::Result<()> {
        self.injected(path)?;
        self.delegate.create_directory_and_parents(path)
    }
    fn get_file_size(&self, path: &PathFragment, follow_symlinks: bool) -> io::Result<i64> {
        self.delegate.get_file_size(path, follow_symlinks)
    }
    fn delete(&self, path: &PathFragment) -> io::Result<bool> {
        self.delegate.delete(path)
    }
    fn get_last_modified_time(
        &self,
        path: &PathFragment,
        follow_symlinks: bool,
    ) -> io::Result<i64> {
        self.delegate.get_last_modified_time(path, follow_symlinks)
    }
    fn set_last_modified_time(&self, path: &PathFragment, new_time: i64) -> io::Result<()> {
        self.delegate.set_last_modified_time(path, new_time)
    }
    fn get_fast_digest(&self, path: &PathFragment) -> io::Result<Option<Vec<u8>>> {
        self.delegate.get_fast_digest(path)
    }
    fn get_digest(&self, path: &PathFragment) -> io::Result<Vec<u8>> {
        self.delegate.get_digest(path)
    }
    fn resolve_one_link(&self, path: &PathFragment) -> io::Result<Option<PathFragment>> {
        self.delegate.resolve_one_link(path)
    }
    fn resolve_symbolic_links(&self, path: &PathFragment) -> io::Result<PathFragment> {
        self.delegate.resolve_symbolic_links(path)
    }
    fn stat(&self, path: &PathFragment, follow_symlinks: bool) -> io::Result<FileStatus> {
        self.delegate.stat(path, follow_symlinks)
    }
    fn stat_metadata(
        &self,
        path: &PathFragment,
        follow_symlinks: bool,
    ) -> io::Result<Option<std::sync::Arc<dyn blaze_vfs::StatMetadata>>> {
        self.delegate.stat_metadata(path, follow_symlinks)
    }
    fn stat_nullable(&self, path: &PathFragment, follow_symlinks: bool) -> Option<FileStatus> {
        self.delegate.stat_nullable(path, follow_symlinks)
    }
    fn stat_if_found(
        &self,
        path: &PathFragment,
        follow_symlinks: bool,
    ) -> io::Result<Option<FileStatus>> {
        self.delegate.stat_if_found(path, follow_symlinks)
    }
    fn create_symbolic_link(
        &self,
        link_path: &PathFragment,
        target_fragment: &PathFragment,
    ) -> io::Result<()> {
        self.delegate
            .create_symbolic_link(link_path, target_fragment)
    }
    fn read_symbolic_link(&self, path: &PathFragment) -> io::Result<PathFragment> {
        self.delegate.read_symbolic_link(path)
    }
    fn exists(&self, path: &PathFragment, follow_symlinks: bool) -> bool {
        self.delegate.exists(path, follow_symlinks)
    }
    fn get_directory_entries(&self, path: &PathFragment) -> io::Result<Vec<String>> {
        self.delegate.get_directory_entries(path)
    }
    fn readdir(&self, path: &PathFragment, follow_symlinks: bool) -> io::Result<Vec<Dirent>> {
        self.delegate.readdir(path, follow_symlinks)
    }
    fn is_readable(&self, path: &PathFragment) -> io::Result<bool> {
        self.delegate.is_readable(path)
    }
    fn set_readable(&self, path: &PathFragment, readable: bool) -> io::Result<()> {
        self.delegate.set_readable(path, readable)
    }
    fn is_writable(&self, path: &PathFragment) -> io::Result<bool> {
        self.delegate.is_writable(path)
    }
    fn set_writable(&self, path: &PathFragment, writable: bool) -> io::Result<()> {
        self.delegate.set_writable(path, writable)
    }
    fn is_executable(&self, path: &PathFragment) -> io::Result<bool> {
        self.delegate.is_executable(path)
    }
    fn set_executable(&self, path: &PathFragment, executable: bool) -> io::Result<()> {
        self.delegate.set_executable(path, executable)
    }
    fn chmod(&self, path: &PathFragment, mode: u32) -> io::Result<()> {
        self.delegate.chmod(path, mode)
    }
    fn get_input_stream(&self, path: &PathFragment) -> io::Result<Box<dyn Read + Send>> {
        self.delegate.get_input_stream(path)
    }
    fn get_output_stream(
        &self,
        path: &PathFragment,
        append: bool,
        internal: bool,
    ) -> io::Result<Box<dyn Write + Send>> {
        self.delegate.get_output_stream(path, append, internal)
    }
    fn rename_to(&self, source_path: &PathFragment, target_path: &PathFragment) -> io::Result<()> {
        self.delegate.rename_to(source_path, target_path)
    }
    fn create_hard_link(
        &self,
        link_path: &PathFragment,
        original_path: &PathFragment,
    ) -> io::Result<()> {
        self.delegate.create_hard_link(link_path, original_path)
    }
    fn create_fs_dependent_hard_link(
        &self,
        link_path: &PathFragment,
        original_path: &PathFragment,
    ) -> io::Result<()> {
        self.delegate
            .create_fs_dependent_hard_link(link_path, original_path)
    }
}

// Java: src/test/java/com/google/devtools/build/lib/actions/ActionOutputDirectoryHelperTest.java:255 ActionOutputDirectoryHelperTest.createFileSystemInjectingException
fn create_file_system_injecting_exception(
    exec_root: &Path,
    failing_path: PathFragment,
    injected_exception: usize,
) -> FileSystemRef {
    Arc::new(DelegateFileSystem {
        delegate: exec_root.get_file_system().clone(),
        failing_path: Some((failing_path, injected_exception)),
    })
}

// Java: src/test/java/com/google/devtools/build/lib/actions/ActionOutputDirectoryHelperTest.java:196 ActionOutputDirectoryHelperTest.createOutputDirectories_ioExceptionWhenCreatingDirectory_fails
// PORT: @TestParameter DirectoryCache: only CACHE_ENABLED, as
// create_output_directories_creates_expected_directories.
#[test]
fn create_output_directories_io_exception_when_creating_directory_fails() {
    let dirs = TestDirs::new();
    let exec_root = dirs.exec_root.clone();
    let output_root = output_root(&dirs);
    let output_directory_helper = ActionOutputDirectoryHelper::new();
    let injected_exception = new_injected_exception_id();
    let output_root_path = output_root
        .get_root()
        .as_path()
        .unwrap()
        .as_fragment()
        .clone();
    let fs_with_failures = create_file_system_injecting_exception(
        &exec_root,
        output_root_path.get_relative(&PathFragment::create("dir")),
        injected_exception,
    );
    let root_with_failure = create_output_root(&Path::create(
        exec_root.as_fragment().clone(),
        &fs_with_failures,
    ));
    let outputs = vec![create_output(&root_with_failure, "dir/file")];

    let e = output_directory_helper
        .create_output_directories(&outputs)
        .unwrap_err();

    assert_eq!(
        e.get_directory_path(),
        &output_root_path.get_relative(&PathFragment::create("dir"))
    );
    assert_cause_is_injected(e.get_cause(), injected_exception);
}

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
fn create_action_fs_output_directories_io_exception_when_creating_directory_fails() {
    let dirs = TestDirs::new();
    let exec_root = dirs.exec_root.clone();
    let output_root = output_root(&dirs);
    let output_directory_helper = ActionOutputDirectoryHelper::new();
    let injected_exception = new_injected_exception_id();
    let output_root_path = output_root
        .get_root()
        .as_path()
        .unwrap()
        .as_fragment()
        .clone();
    let fs_with_failures = create_file_system_injecting_exception(
        &exec_root,
        output_root_path.get_relative(&PathFragment::create("dir")),
        injected_exception,
    );
    let root_with_failure = create_output_root(&Path::create(
        exec_root.as_fragment().clone(),
        &fs_with_failures,
    ));
    let outputs = vec![create_output(&root_with_failure, "dir/file")];
    let action_file_system: FileSystemRef = Arc::new(DelegateFileSystem {
        delegate: fs_with_failures,
        failing_path: None,
    });
    // ArtifactPathResolver.createPathResolver(actionFileSystem, execRoot)
    let path_resolver = Some(&action_file_system);

    let e = output_directory_helper
        .create_action_fs_output_directories(&outputs, path_resolver)
        .unwrap_err();

    assert_eq!(
        e.get_directory_path(),
        &output_root_path.get_relative(&PathFragment::create("dir"))
    );
    assert_cause_is_injected(e.get_cause(), injected_exception);
}
