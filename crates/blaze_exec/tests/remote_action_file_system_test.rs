//! `RemoteActionFileSystemTest` and `RemoteActionFileSystemTestBase` (lib/remote), for the port
//! of `RemoteActionFileSystem` in `blaze_exec::exec::remote_action_file_system`, plus Blaze-only
//! tests: the tree shapes `ActionOutputMetadataStore.constructTreeArtifactValueFromFilesystem`
//! visits through the action file system (review N1), the lost-input record (N5), and the
//! in-process readers that must go through the file system (review R5: `SymlinkAction`'s
//! executable check, template expansion, the test strategy's `test.xml` check).
//!
//! | Java test methods (124)                          | ported | ignored |
//! |--------------------------------------------------|--------|---------|
//! | `RemoteActionFileSystemTestBase` (72)            | 72     | 0       |
//! | `RemoteActionFileSystemTest` (52)                | 52     | 0       |
//! | total                                            | 124    | 0       |
//!
//! `@TestParameter` methods are one Rust test each, looping over the Java
//! parameter values. Two assertions cannot be expressed as in Java and are noted at the test:
//! `testGetInputStream_forRemoteOutputFile` (the prefetch priority, which the blocking prefetcher
//! seam does not carry) and the `FileStatusWithDigest` checks (the digest is read through
//! `stat_metadata`, Java's `FileStatusWithMetadata.getMetadata()`).
//!
//! Java's fixture is an in-memory file system (a Mockito spy) with the exec root `/exec` and the
//! relative output path `out`; here the local file system is the real one under a temporary
//! directory (`analysis_actions_common::TestDirs`: `<tmp>/execroot/_main`, output root
//! `bazel-out/k8-fastbuild/bin`) behind a spy that counts `getDigest` calls, and the relative
//! output path is `bazel-out`.

#![allow(clippy::result_large_err)]

mod analysis_actions_common;

use std::collections::{HashMap, VecDeque};
use std::io::{self, Read, Write};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use analysis_actions_common::{TestDirs, null_action_owner, read, write};
use blaze_actions::{Action, Artifact, SpecialArtifactType};
use blaze_exec::actions::action_adapters::{ActionMetadataSnapshot, artifact_input};
use blaze_exec::actions::{
    ActionExecutionMetadata, ActionInputRef, ExecutableAction, FileArtifactValue,
    FilesetOutputTree, InputMetadataProvider, RunfilesArtifactValue, TreeArtifactValue,
    as_executable,
};
use blaze_exec::exec::action_execution_context::{
    ActionExecutionContext, ActionInputPrefetcher, BulkTransferError,
};
use blaze_exec::exec::remote_action_file_system::RemoteActionFileSystem;
use blaze_vfs::{
    DigestHashFunction, Dirent, DirentType, FileStatus, FileSymlinkLoopException, FileSystem,
    FileSystemRef, NotASymlinkException, PathFragment, Symlinks,
};
use parking_lot::Mutex;
use sha2::Digest as _;

const RELATIVE_OUTPUT_PATH: &str = "bazel-out";

/// `ActionInputMap`: inputs (files, tree artifacts and their children) by exec path.
#[derive(Default)]
struct InputMap {
    files: Mutex<HashMap<PathFragment, (ActionInputRef, FileArtifactValue)>>,
    trees: Mutex<HashMap<PathFragment, (ActionInputRef, Arc<TreeArtifactValue>)>>,
}

impl InputMap {
    fn put(&self, artifact: &Artifact, metadata: FileArtifactValue) {
        self.files.lock().insert(
            artifact.get_exec_path().clone(),
            (artifact_input(artifact), metadata),
        );
    }

    /// `putTreeArtifact(tree, value)`: the tree and its children.
    fn put_tree_artifact(&self, tree: &Artifact, value: TreeArtifactValue) {
        for (child, metadata) in &value.child_values {
            self.files.lock().insert(
                child.get_exec_path().clone(),
                (child.clone(), metadata.clone()),
            );
        }
        self.trees.lock().insert(
            tree.get_exec_path().clone(),
            (artifact_input(tree), Arc::new(value)),
        );
    }
}

impl InputMetadataProvider for InputMap {
    fn get_input_metadata(&self, input: &ActionInputRef) -> io::Result<Option<FileArtifactValue>> {
        if let Some((_, m)) = self.files.lock().get(input.get_exec_path()) {
            return Ok(Some(m.clone()));
        }
        Ok(self
            .trees
            .lock()
            .get(input.get_exec_path())
            .and_then(|(_, t)| t.metadata.clone()))
    }
    fn get_tree_metadata(&self, input: &ActionInputRef) -> Option<Arc<TreeArtifactValue>> {
        self.trees
            .lock()
            .get(input.get_exec_path())
            .map(|(_, t)| t.clone())
    }
    fn get_fileset(&self, _input: &ActionInputRef) -> Option<Arc<FilesetOutputTree>> {
        None
    }
    fn get_runfiles_metadata(&self, _input: &ActionInputRef) -> Option<Arc<RunfilesArtifactValue>> {
        None
    }
    fn get_input(&self, exec_path: &PathFragment) -> Option<ActionInputRef> {
        if let Some((i, _)) = self.files.lock().get(exec_path) {
            return Some(i.clone());
        }
        self.trees.lock().get(exec_path).map(|(i, _)| i.clone())
    }
}

/// The mocked `RemoteActionInputFetcher`: `mockPrefetchFile(path, contents)` writes the
/// contents of a prefetched input to its local path, and every call is recorded. A queued
/// failure is returned by the next call instead.
struct FakeInputFetcher {
    exec_root: blaze_vfs::Path,
    contents: Mutex<HashMap<PathFragment, String>>,
    failures: Mutex<VecDeque<io::Error>>,
    calls: Mutex<Vec<Vec<PathFragment>>>,
}

impl FakeInputFetcher {
    fn new(exec_root: blaze_vfs::Path) -> Arc<FakeInputFetcher> {
        Arc::new(FakeInputFetcher {
            exec_root,
            contents: Mutex::new(HashMap::new()),
            failures: Mutex::new(VecDeque::new()),
            calls: Mutex::new(Vec::new()),
        })
    }

    fn mock_prefetch_file(&self, artifact: &Artifact, contents: &str) {
        self.contents
            .lock()
            .insert(artifact.get_exec_path().clone(), contents.to_string());
    }

    fn fail_next_prefetch(&self, error: io::Error) {
        self.failures.lock().push_back(error);
    }

    fn calls(&self) -> Vec<Vec<PathFragment>> {
        self.calls.lock().clone()
    }
}

impl ActionInputPrefetcher for FakeInputFetcher {
    fn prefetch_files(
        &self,
        _action: &dyn ActionExecutionMetadata,
        inputs: &[ActionInputRef],
        _metadata_provider: &dyn InputMetadataProvider,
    ) -> io::Result<()> {
        self.calls
            .lock()
            .push(inputs.iter().map(|i| i.get_exec_path().clone()).collect());
        if let Some(error) = self.failures.lock().pop_front() {
            return Err(error);
        }
        for input in inputs {
            if let Some(contents) = self.contents.lock().get(input.get_exec_path()) {
                write(
                    &self.exec_root.get_relative(input.get_exec_path()),
                    contents,
                );
            }
        }
        Ok(())
    }
}

/// `SpiedFileSystem.createInMemorySpy()`: the local file system, with the `getDigest` calls
/// counted (`verify(fs, never()).getDigest(any())`, `reset(fs)`).
#[derive(Debug)]
struct SpiedFileSystem {
    delegate: FileSystemRef,
    get_digest_calls: AtomicUsize,
}

impl SpiedFileSystem {
    fn reset(&self) {
        self.get_digest_calls.store(0, Ordering::SeqCst);
    }

    fn get_digest_calls(&self) -> usize {
        self.get_digest_calls.load(Ordering::SeqCst)
    }
}

impl FileSystem for SpiedFileSystem {
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
        self.delegate.create_directory(path)
    }
    fn create_directory_and_parents(&self, path: &PathFragment) -> io::Result<()> {
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
        self.get_digest_calls.fetch_add(1, Ordering::SeqCst);
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

/// An action whose metadata `updateContext` receives (Java: `mock(ActionExecutionMetadata)`).
fn dummy_action(dirs: &TestDirs) -> Arc<dyn ActionExecutionMetadata> {
    let output = dirs.bin_artifact("dummy-output");
    let action = blaze_actions::analysis::actions::file_write_action::FileWriteAction::create(
        null_action_owner(),
        output,
        "",
        false,
        blaze_actions::analysis::actions::file_write_action::Compression::DISALLOW,
    );
    ActionMetadataSnapshot::of(&action)
}

/// `FilesystemTestParam`: which underlying file system of the action file system a test writes
/// to directly.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FilesystemTestParam {
    Local,
    Remote,
}

use FilesystemTestParam::{Local, Remote};

const FILESYSTEM_TEST_PARAMS: [FilesystemTestParam; 2] = [Local, Remote];

impl FilesystemTestParam {
    fn get_filesystem(self, action_fs: &RemoteActionFileSystem) -> FileSystemRef {
        match self {
            Local => action_fs.get_local_file_system().clone(),
            Remote => action_fs.get_remote_in_memory_file_system().clone(),
        }
    }
}

struct Fixture {
    dirs: TestDirs,
    fs: Arc<SpiedFileSystem>,
    fetcher: Arc<FakeInputFetcher>,
}

impl Fixture {
    fn new() -> Fixture {
        let dirs = TestDirs::new();
        // setUp: outputRoot.getRoot().asPath().createDirectoryAndParents()
        dirs.bin_root
            .get_root()
            .as_path()
            .unwrap()
            .create_directory_and_parents()
            .unwrap();
        let fetcher = FakeInputFetcher::new(dirs.exec_root.clone());
        let fs = Arc::new(SpiedFileSystem {
            delegate: dirs.exec_root.get_file_system().clone(),
            get_digest_calls: AtomicUsize::new(0),
        });
        Fixture { dirs, fs, fetcher }
    }

    // Java: RemoteActionFileSystemTest.createActionFileSystem
    fn create_action_file_system_with(&self, inputs: Arc<InputMap>) -> Arc<RemoteActionFileSystem> {
        let fs = RemoteActionFileSystem::new(
            self.fs.clone() as FileSystemRef,
            self.dirs.exec_root.as_fragment().clone(),
            RELATIVE_OUTPUT_PATH,
            inputs,
            Some(self.fetcher.clone() as Arc<dyn ActionInputPrefetcher>),
        );
        fs.update_context(dummy_action(&self.dirs));
        fs.create_directory_and_parents(
            self.dirs
                .bin_root
                .get_root()
                .as_path()
                .unwrap()
                .as_fragment(),
        )
        .unwrap();
        fs
    }

    fn create_action_file_system(&self) -> Arc<RemoteActionFileSystem> {
        self.create_action_file_system_with(Arc::new(InputMap::default()))
    }

    // Java: RemoteActionFileSystemTest.getOutputPath
    fn get_output_path(&self, output_root_relative_path: &str) -> PathFragment {
        self.dirs
            .bin_root
            .get_root()
            .as_path()
            .unwrap()
            .get_relative_str(output_root_relative_path)
            .as_fragment()
            .clone()
    }

    // Java: RemoteActionFileSystemTest.writeLocalFile
    fn write_local_file(&self, path: &PathFragment, content: &str) {
        write(
            &blaze_vfs::Path::create(path.clone(), self.dirs.exec_root.get_file_system()),
            content,
        );
    }

    // Java: RemoteActionFileSystemTest.createRemoteArtifact
    fn create_remote_artifact(&self, path: &str, content: &str, inputs: &InputMap) -> Artifact {
        let a = self.dirs.bin_artifact(path);
        inputs.put(&a, remote_metadata(content, 1));
        a
    }

    /// `ActionsTestUtil.createTreeArtifactWithGeneratingAction(outputRoot, path)`.
    fn create_tree_artifact_with_generating_action(&self, path: &str) -> Artifact {
        let tree = self.dirs.special_artifact(path, SpecialArtifactType::TREE);
        tree.set_generating_action_key(blaze_actions::ActionLookupData::create(
            blaze_actions::ActionLookupKeyRef::new(analysis_actions_common::NullArtifactOwner),
            0,
        ));
        tree
    }

    // Java: RemoteActionFileSystemTest.createRemoteTreeArtifact
    fn create_remote_tree_artifact(
        &self,
        path: &str,
        content_map: &[(&str, &str)],
        inputs: &InputMap,
    ) -> Artifact {
        let tree = self.create_tree_artifact_with_generating_action(path);
        let mut children = Vec::new();
        for (name, content) in content_map {
            let child = Artifact::create_tree_output(&tree, PathFragment::create(name));
            children.push((artifact_input(&child), remote_metadata(content, 0)));
        }
        inputs.put_tree_artifact(&tree, TreeArtifactValue::build(children, None, None));
        tree
    }

    // Java: RemoteActionFileSystemTest.createLocalArtifact
    fn create_local_artifact(&self, path: &str, contents: &str, inputs: &InputMap) -> Artifact {
        let a = self.dirs.bin_artifact(path);
        write(&a.get_path(), contents);
        let p = a.get_path();
        let stat = p.stat_with(Symlinks::Follow).unwrap();
        inputs.put(&a, FileArtifactValue::create_from_stat(&p, &stat).unwrap());
        a
    }

    // Java: RemoteActionFileSystemTest.createLocalTreeArtifact
    fn create_local_tree_artifact(
        &self,
        path: &str,
        content_map: &[(&str, &str)],
        inputs: &InputMap,
    ) -> Artifact {
        let tree = self.create_tree_artifact_with_generating_action(path);
        tree.get_path().create_directory_and_parents().unwrap();
        let mut children = Vec::new();
        for (name, content) in content_map {
            let child = tree.get_path().get_relative_str(name);
            write(&child, content);
            let stat = child.stat_with(Symlinks::Follow).unwrap();
            let child_artifact = Artifact::create_tree_output(&tree, PathFragment::create(name));
            children.push((
                artifact_input(&child_artifact),
                FileArtifactValue::create_from_stat(&child, &stat).unwrap(),
            ));
        }
        inputs.put_tree_artifact(&tree, TreeArtifactValue::build(children, None, None));
        tree
    }
}

// Java: RemoteActionFileSystemTest.getDigest
fn get_digest(content: &str) -> Vec<u8> {
    sha2::Sha256::digest(content.as_bytes()).to_vec()
}

fn remote_metadata(content: &str, location_index: i32) -> FileArtifactValue {
    FileArtifactValue::create_for_remote_file_with_materialization_data(
        get_digest(content),
        content.len() as i64,
        location_index,
        /* expiration_time= */ None,
        /* in_memory_output= */ false,
    )
}

// Java: RemoteActionFileSystemTest.injectRemoteFile
fn inject_remote_file(
    action_fs: &RemoteActionFileSystem,
    path: &PathFragment,
    content: &str,
) -> FileArtifactValue {
    action_fs
        .inject_remote_file(path, get_digest(content), content.len() as i64, None, false)
        .unwrap();
    remote_metadata(content, 1)
}

/// Writes `content` at `path` on the file system `to` designates: a local file, or an injected
/// remote file (`if (toFs.equals(actionFs.getLocalFileSystem())) writeLocalFile else
/// injectRemoteFile`).
fn write_to(
    f: &Fixture,
    action_fs: &RemoteActionFileSystem,
    to: FilesystemTestParam,
    path: &PathFragment,
    content: &str,
) {
    match to {
        Local => f.write_local_file(path, content),
        Remote => {
            inject_remote_file(action_fs, path, content);
        }
    }
}

fn local_fs(action_fs: &RemoteActionFileSystem) -> FileSystemRef {
    action_fs.get_local_file_system().clone()
}

fn remote_fs(action_fs: &RemoteActionFileSystem) -> FileSystemRef {
    action_fs.get_remote_in_memory_file_system().clone()
}

fn read_content(action_fs: &RemoteActionFileSystem, path: &PathFragment) -> String {
    let mut s = String::new();
    action_fs
        .get_input_stream(path)
        .unwrap()
        .read_to_string(&mut s)
        .unwrap();
    s
}

fn assert_file_not_found<T: std::fmt::Debug>(result: io::Result<T>) {
    let e = result.expect_err("expected FileNotFoundException");
    assert_eq!(e.kind(), io::ErrorKind::NotFound, "{e}");
}

// Java: RemoteActionFileSystemTest.assertReaddir
fn assert_readdir(
    action_fs: &RemoteActionFileSystem,
    dir_path: &PathFragment,
    follow_symlinks: bool,
    expected: &[Dirent],
) {
    let mut actual = action_fs.readdir(dir_path, follow_symlinks).unwrap();
    actual.sort();
    let mut expected = expected.to_vec();
    expected.sort();
    assert_eq!(actual, expected);
    let mut names = action_fs.get_directory_entries(dir_path).unwrap();
    names.sort();
    assert_eq!(
        names,
        expected
            .iter()
            .map(|d| d.get_name().to_string())
            .collect::<Vec<_>>()
    );
}

// Java: RemoteActionFileSystemTest.assertReaddirThrows
fn assert_readdir_throws(
    action_fs: &RemoteActionFileSystem,
    dir_path: &PathFragment,
    follow_symlinks: bool,
) {
    assert!(action_fs.readdir(dir_path, follow_symlinks).is_err());
    assert!(action_fs.get_directory_entries(dir_path).is_err());
}

// ---------------------------------------------------------------------------------------------
// RemoteActionFileSystemTestBase
// ---------------------------------------------------------------------------------------------

#[test]
fn exists_file_does_not_exist_returns_false() {
    let f = Fixture::new();
    let action_fs = f.create_action_file_system();
    let path = f.get_output_path("file");

    let exists = action_fs.exists(&path, true);

    assert!(!exists);
}

#[test]
fn exists_local_file_returns_true() {
    let f = Fixture::new();
    let action_fs = f.create_action_file_system();
    let path = f.get_output_path("file");

    f.write_local_file(&path, "local contents");

    assert!(action_fs.exists(&path, true));
}

#[test]
fn exists_remote_file_returns_true() {
    let f = Fixture::new();
    let action_fs = f.create_action_file_system();
    let path = f.get_output_path("file");

    inject_remote_file(&action_fs, &path, "remote contents");

    assert!(action_fs.exists(&path, true));
}

#[test]
fn exists_local_and_remote_file_returns_true() {
    let f = Fixture::new();
    let action_fs = f.create_action_file_system();
    let path = f.get_output_path("file");

    f.write_local_file(&path, "local contents");
    inject_remote_file(&action_fs, &path, "remote contents");

    assert!(action_fs.exists(&path, true));
}

#[test]
fn delete_file_does_not_exist_returns_false() {
    let f = Fixture::new();
    let action_fs = f.create_action_file_system();
    let path = f.get_output_path("file");

    let success = action_fs.get_path(&path).delete().unwrap();

    assert!(!success);
}

#[test]
fn delete_local_file_succeeds() {
    let f = Fixture::new();
    let action_fs = f.create_action_file_system();
    let path = f.get_output_path("file");
    f.write_local_file(&path, "local contents");
    assert!(local_fs(&action_fs).exists(&path, true));

    let success = action_fs.get_path(&path).delete().unwrap();

    assert!(success);
    assert!(!local_fs(&action_fs).exists(&path, true));
}

#[test]
fn delete_remote_file_succeeds() {
    let f = Fixture::new();
    let action_fs = f.create_action_file_system();
    let path = f.get_output_path("file");
    inject_remote_file(&action_fs, &path, "remote contents");
    assert!(remote_fs(&action_fs).exists(&path, true));

    let success = action_fs.get_path(&path).delete().unwrap();

    assert!(success);
    assert!(!action_fs.exists(&path, true));
    assert!(!remote_fs(&action_fs).exists(&path, true));
}

#[test]
fn delete_local_and_remote_file_succeeds() {
    let f = Fixture::new();
    let action_fs = f.create_action_file_system();
    let path = f.get_output_path("file");
    f.write_local_file(&path, "local contents");
    inject_remote_file(&action_fs, &path, "remote contents");
    assert!(local_fs(&action_fs).exists(&path, true));
    assert!(remote_fs(&action_fs).exists(&path, true));

    let success = action_fs.get_path(&path).delete().unwrap();

    assert!(success);
    assert!(!action_fs.exists(&path, true));
    assert!(!local_fs(&action_fs).exists(&path, true));
    assert!(!remote_fs(&action_fs).exists(&path, true));
}

#[test]
fn rename_to_source_file_does_not_exist_throw_error() {
    let f = Fixture::new();
    let action_fs = f.create_action_file_system();
    let path = f.get_output_path("file");
    let new_path = f.get_output_path("file-new");

    assert_file_not_found(action_fs.rename_to(&path, &new_path));
}

#[test]
fn rename_to_target_directory_does_not_exist_throw_error() {
    let f = Fixture::new();
    let action_fs = f.create_action_file_system();
    let path = f.get_output_path("file");
    let new_path = f.get_output_path("dir/file-new");
    f.write_local_file(&path, "local-content");

    assert_file_not_found(action_fs.rename_to(&path, &new_path));
}

#[test]
fn rename_to_only_remote_file_rename_remote_file() {
    let f = Fixture::new();
    let action_fs = f.create_action_file_system();
    let path = f.get_output_path("file");
    inject_remote_file(&action_fs, &path, "remote-content");
    let new_path = f.get_output_path("file-new");

    action_fs.rename_to(&path, &new_path).unwrap();

    assert!(!action_fs.exists(&path, true));
    assert!(action_fs.exists(&new_path, true));
    assert!(!remote_fs(&action_fs).exists(&path, true));
    assert!(remote_fs(&action_fs).exists(&new_path, true));
}

#[test]
fn rename_to_local_and_remote_file_rename_both() {
    let f = Fixture::new();
    let action_fs = f.create_action_file_system();
    let path = f.get_output_path("file");
    inject_remote_file(&action_fs, &path, "remote-content");
    f.write_local_file(&path, "local-content");
    let new_path = f.get_output_path("file-new");

    action_fs.rename_to(&path, &new_path).unwrap();

    assert!(!action_fs.exists(&path, true));
    assert!(action_fs.exists(&new_path, true));
    assert!(!remote_fs(&action_fs).exists(&path, true));
    assert!(remote_fs(&action_fs).exists(&new_path, true));
    assert!(!local_fs(&action_fs).exists(&path, true));
    assert!(local_fs(&action_fs).exists(&new_path, true));
}

#[test]
fn create_directory_and_parents_create_locally_and_remotely() {
    let f = Fixture::new();
    let action_fs = f.create_action_file_system();
    let path = f.get_output_path("dir");

    action_fs.create_directory_and_parents(&path).unwrap();

    assert!(remote_fs(&action_fs).is_directory(&path, true));
    assert!(local_fs(&action_fs).is_directory(&path, true));
}

#[test]
fn create_directory_create_locally_and_remotely() {
    let f = Fixture::new();
    let action_fs = f.create_action_file_system();
    action_fs
        .create_directory_and_parents(&f.get_output_path("parent"))
        .unwrap();
    let path = f.get_output_path("parent/dir");

    action_fs.create_directory(&path).unwrap();

    assert!(remote_fs(&action_fs).is_directory(&path, true));
    assert!(local_fs(&action_fs).is_directory(&path, true));
}

/// `DirectoryEntriesProvider`: `path.readdir(Symlinks.FOLLOW)` names, or
/// `path.getDirectoryEntries()` base names.
type DirectoryEntriesProvider = fn(&blaze_vfs::Path) -> io::Result<Vec<String>>;

fn readdir_names(path: &blaze_vfs::Path) -> io::Result<Vec<String>> {
    Ok(path
        .readdir(Symlinks::Follow)?
        .into_iter()
        .map(|d| d.get_name().to_string())
        .collect())
}

fn get_directory_entries_names(path: &blaze_vfs::Path) -> io::Result<Vec<String>> {
    Ok(path
        .get_directory_entries()?
        .iter()
        .map(|p| p.get_base_name().to_string())
        .collect())
}

fn sorted(mut names: Vec<String>) -> Vec<String> {
    names.sort();
    names
}

// Java: RemoteActionFileSystemTestBase.readdirNonEmptyLocalDirectoryReadFromLocal
fn helper_readdir_non_empty_local_directory_read_from_local(provider: DirectoryEntriesProvider) {
    let f = Fixture::new();
    let action_fs = f.create_action_file_system();
    let dir = f.get_output_path("parent/dir");
    action_fs
        .get_path(&dir)
        .create_directory_and_parents()
        .unwrap();
    f.write_local_file(&dir.get_child("file1"), "content1");
    f.write_local_file(&dir.get_child("file2"), "content2");

    let entries = provider(&action_fs.get_path(&dir)).unwrap();

    assert_eq!(sorted(entries), vec!["file1", "file2"]);
}

// Java: RemoteActionFileSystemTestBase.readdirNonEmptyInMemoryDirectoryReadFromMemory
fn helper_readdir_non_empty_in_memory_directory_read_from_memory(
    provider: DirectoryEntriesProvider,
) {
    let f = Fixture::new();
    let action_fs = f.create_action_file_system();
    let dir = f.get_output_path("parent/dir");
    action_fs
        .get_path(&dir)
        .create_directory_and_parents()
        .unwrap();
    inject_remote_file(&action_fs, &dir.get_child("file1"), "content1");
    inject_remote_file(&action_fs, &dir.get_child("file2"), "content2");

    let entries = provider(&action_fs.get_path(&dir)).unwrap();

    assert_eq!(sorted(entries), vec!["file1", "file2"]);
}

// Java: RemoteActionFileSystemTestBase.readdirNonEmptyLocalAndInMemoryDirectoryCombineThem
fn helper_readdir_non_empty_local_and_in_memory_directory_combine_them(
    provider: DirectoryEntriesProvider,
) {
    let f = Fixture::new();
    let action_fs = f.create_action_file_system();
    let dir = f.get_output_path("parent/dir");
    action_fs
        .get_path(&dir)
        .create_directory_and_parents()
        .unwrap();
    f.write_local_file(&dir.get_child("file1"), "content1");
    f.write_local_file(&dir.get_child("file2"), "content2");
    inject_remote_file(&action_fs, &dir.get_child("file2"), "content2inmemory");
    inject_remote_file(&action_fs, &dir.get_child("file3"), "content3");

    let entries = provider(&action_fs.get_path(&dir)).unwrap();

    assert_eq!(sorted(entries), vec!["file1", "file2", "file3"]);
}

// Java: RemoteActionFileSystemTestBase.readdirNothingThereThrowsFileNotFound
fn helper_readdir_nothing_there_throws_file_not_found(provider: DirectoryEntriesProvider) {
    let f = Fixture::new();
    let action_fs = f.create_action_file_system();
    let dir = f.get_output_path("parent/dir");

    assert_file_not_found(provider(&action_fs.get_path(&dir)));
}

#[test]
fn readdir_non_empty_local_directory_read_from_local() {
    helper_readdir_non_empty_local_directory_read_from_local(readdir_names);
}

#[test]
fn readdir_non_empty_in_memory_directory_read_from_memory() {
    helper_readdir_non_empty_in_memory_directory_read_from_memory(readdir_names);
}

#[test]
fn readdir_non_empty_local_and_in_memory_directory_combine_them() {
    helper_readdir_non_empty_local_and_in_memory_directory_combine_them(readdir_names);
}

#[test]
fn readdir_nothing_there_throws_file_not_found() {
    helper_readdir_nothing_there_throws_file_not_found(readdir_names);
}

#[test]
fn get_directory_entries_non_empty_local_directory_read_from_local() {
    helper_readdir_non_empty_local_directory_read_from_local(get_directory_entries_names);
}

#[test]
fn get_directory_entries_non_empty_in_memory_directory_read_from_memory() {
    helper_readdir_non_empty_in_memory_directory_read_from_memory(get_directory_entries_names);
}

#[test]
fn get_directory_entries_non_empty_local_and_in_memory_directory_combine_them() {
    helper_readdir_non_empty_local_and_in_memory_directory_combine_them(
        get_directory_entries_names,
    );
}

#[test]
fn get_directory_entries_nothing_there_throws_file_not_found() {
    helper_readdir_nothing_there_throws_file_not_found(get_directory_entries_names);
}

#[test]
fn is_readable_file_does_not_exist_throw_error() {
    let f = Fixture::new();
    let action_fs = f.create_action_file_system();
    let path = f.get_output_path("file");

    assert_file_not_found(action_fs.get_path(&path).is_readable());
}

#[test]
fn is_readable_only_remote_file_returns_true() {
    let f = Fixture::new();
    let action_fs = f.create_action_file_system();
    let path = f.get_output_path("file");
    inject_remote_file(&action_fs, &path, "remote-content");

    let readable = action_fs.get_path(&path).is_readable().unwrap();

    assert!(readable);
}

#[test]
fn is_readable_only_remote_directory_returns_true() {
    let f = Fixture::new();
    let action_fs = f.create_action_file_system();
    let path = f.get_output_path("dir");
    remote_fs(&action_fs)
        .create_directory_and_parents(&path)
        .unwrap();

    let readable = action_fs.get_path(&path).is_readable().unwrap();

    assert!(readable);
}

#[test]
fn is_readable_local_readable_file_returns_true() {
    let f = Fixture::new();
    let action_fs = f.create_action_file_system();
    let path = f.get_output_path("file");
    f.write_local_file(&path, "local-content");

    let readable = action_fs.get_path(&path).is_readable().unwrap();

    assert!(readable);
}

#[test]
fn is_readable_local_non_readable_file_returns_false() {
    let f = Fixture::new();
    let action_fs = f.create_action_file_system();
    let path = f.get_output_path("file");
    f.write_local_file(&path, "local-content");
    local_fs(&action_fs).set_readable(&path, false).unwrap();

    let readable = action_fs.get_path(&path).is_readable().unwrap();

    assert!(!readable);
}

#[test]
fn is_readable_local_readable_file_and_remote_file_returns_true() {
    let f = Fixture::new();
    let action_fs = f.create_action_file_system();
    let path = f.get_output_path("file");
    inject_remote_file(&action_fs, &path, "remote-content");
    f.write_local_file(&path, "local-content");

    let readable = action_fs.get_path(&path).is_readable().unwrap();

    assert!(readable);
}

#[test]
fn is_readable_local_non_readable_file_and_remote_file_returns_false() {
    let f = Fixture::new();
    let action_fs = f.create_action_file_system();
    let path = f.get_output_path("file");
    inject_remote_file(&action_fs, &path, "remote-content");
    f.write_local_file(&path, "local-content");
    local_fs(&action_fs).set_readable(&path, false).unwrap();

    let readable = action_fs.get_path(&path).is_readable().unwrap();

    assert!(!readable);
}

#[test]
fn is_writable_file_does_not_exist_throw_error() {
    let f = Fixture::new();
    let action_fs = f.create_action_file_system();
    let path = f.get_output_path("file");

    assert_file_not_found(action_fs.get_path(&path).is_writable());
}

#[test]
fn is_writable_only_remote_file_returns_true() {
    let f = Fixture::new();
    let action_fs = f.create_action_file_system();
    let path = f.get_output_path("file");
    inject_remote_file(&action_fs, &path, "remote-content");

    let writable = action_fs.get_path(&path).is_writable().unwrap();

    assert!(writable);
}

#[test]
fn is_writable_only_remote_directory_returns_true() {
    let f = Fixture::new();
    let action_fs = f.create_action_file_system();
    let path = f.get_output_path("dir");
    remote_fs(&action_fs)
        .create_directory_and_parents(&path)
        .unwrap();

    let writable = action_fs.get_path(&path).is_writable().unwrap();

    assert!(writable);
}

#[test]
fn is_writable_local_writable_file_returns_true() {
    let f = Fixture::new();
    let action_fs = f.create_action_file_system();
    let path = f.get_output_path("file");
    f.write_local_file(&path, "local-content");

    let writable = action_fs.get_path(&path).is_writable().unwrap();

    assert!(writable);
}

#[test]
fn is_writable_local_non_writable_file_returns_false() {
    let f = Fixture::new();
    let action_fs = f.create_action_file_system();
    let path = f.get_output_path("file");
    f.write_local_file(&path, "local-content");
    local_fs(&action_fs).set_writable(&path, false).unwrap();

    let writable = action_fs.get_path(&path).is_writable().unwrap();

    assert!(!writable);
}

#[test]
fn is_writable_local_writable_file_and_remote_file_returns_true() {
    let f = Fixture::new();
    let action_fs = f.create_action_file_system();
    let path = f.get_output_path("file");
    inject_remote_file(&action_fs, &path, "remote-content");
    f.write_local_file(&path, "local-content");

    let writable = action_fs.get_path(&path).is_writable().unwrap();

    assert!(writable);
}

#[test]
fn is_writable_local_non_writable_file_and_remote_file_returns_false() {
    let f = Fixture::new();
    let action_fs = f.create_action_file_system();
    let path = f.get_output_path("file");
    inject_remote_file(&action_fs, &path, "remote-content");
    f.write_local_file(&path, "local-content");
    local_fs(&action_fs).set_writable(&path, false).unwrap();

    let writable = action_fs.get_path(&path).is_writable().unwrap();

    assert!(!writable);
}

#[test]
fn is_writable_local_non_writable_directory_and_remote_directory_returns_false() {
    let f = Fixture::new();
    let action_fs = f.create_action_file_system();
    let path = f.get_output_path("dir");
    remote_fs(&action_fs)
        .create_directory_and_parents(&path)
        .unwrap();
    local_fs(&action_fs)
        .create_directory_and_parents(&path)
        .unwrap();
    local_fs(&action_fs).set_writable(&path, false).unwrap();

    let writable = action_fs.get_path(&path).is_writable().unwrap();

    assert!(!writable);
}

#[test]
fn is_executable_file_does_not_exist_throw_error() {
    let f = Fixture::new();
    let action_fs = f.create_action_file_system();
    let path = f.get_output_path("file");

    assert_file_not_found(action_fs.get_path(&path).is_executable());
}

#[test]
fn is_executable_only_remote_file_returns_true() {
    let f = Fixture::new();
    let action_fs = f.create_action_file_system();
    let path = f.get_output_path("file");
    inject_remote_file(&action_fs, &path, "remote-content");

    let executable = action_fs.get_path(&path).is_executable().unwrap();

    assert!(executable);
}

#[test]
fn is_executable_only_remote_direcotry_returns_true() {
    let f = Fixture::new();
    let action_fs = f.create_action_file_system();
    let path = f.get_output_path("dir");
    remote_fs(&action_fs)
        .create_directory_and_parents(&path)
        .unwrap();

    let executable = action_fs.get_path(&path).is_executable().unwrap();

    assert!(executable);
}

#[test]
fn is_executable_local_executable_file_returns_true() {
    let f = Fixture::new();
    let action_fs = f.create_action_file_system();
    let path = f.get_output_path("file");
    f.write_local_file(&path, "local-content");
    local_fs(&action_fs).set_executable(&path, true).unwrap();

    let executable = action_fs.get_path(&path).is_executable().unwrap();

    assert!(executable);
}

#[test]
fn is_executable_local_non_executable_file_returns_false() {
    let f = Fixture::new();
    let action_fs = f.create_action_file_system();
    let path = f.get_output_path("file");
    f.write_local_file(&path, "local-content");

    let executable = action_fs.get_path(&path).is_executable().unwrap();

    assert!(!executable);
}

#[test]
fn is_executable_local_executable_file_and_remote_file_returns_true() {
    let f = Fixture::new();
    let action_fs = f.create_action_file_system();
    let path = f.get_output_path("file");
    inject_remote_file(&action_fs, &path, "remote-content");
    f.write_local_file(&path, "local-content");
    local_fs(&action_fs).set_executable(&path, true).unwrap();

    let executable = action_fs.get_path(&path).is_executable().unwrap();

    assert!(executable);
}

#[test]
fn is_executable_local_non_executable_file_and_remote_file_returns_false() {
    let f = Fixture::new();
    let action_fs = f.create_action_file_system();
    let path = f.get_output_path("file");
    inject_remote_file(&action_fs, &path, "remote-content");
    f.write_local_file(&path, "local-content");

    let executable = action_fs.get_path(&path).is_executable().unwrap();

    assert!(!executable);
}

#[test]
fn set_readable_file_does_not_exist_throw_error() {
    let f = Fixture::new();
    let action_fs = f.create_action_file_system();
    let path = f.get_output_path("file");

    assert_file_not_found(action_fs.get_path(&path).set_readable(false));
}

#[test]
fn set_readable_only_remote_file_remains_readable() {
    let f = Fixture::new();
    let action_fs = f.create_action_file_system();
    let path = f.get_output_path("file");
    inject_remote_file(&action_fs, &path, "remote-content");

    action_fs.get_path(&path).set_readable(false).unwrap();

    assert!(action_fs.get_path(&path).is_readable().unwrap());
}

#[test]
fn set_readable_only_remote_direcotry_remains_readable() {
    let f = Fixture::new();
    let action_fs = f.create_action_file_system();
    let path = f.get_output_path("dir");
    remote_fs(&action_fs)
        .create_directory_and_parents(&path)
        .unwrap();

    action_fs.get_path(&path).set_readable(false).unwrap();

    assert!(action_fs.get_path(&path).is_readable().unwrap());
}

#[test]
fn set_readable_local_file_change() {
    let f = Fixture::new();
    let action_fs = f.create_action_file_system();
    let path = f.get_output_path("file");
    f.write_local_file(&path, "local-content");
    assert!(action_fs.get_path(&path).is_readable().unwrap());

    action_fs.get_path(&path).set_readable(false).unwrap();

    assert!(!action_fs.get_path(&path).is_readable().unwrap());
}

#[test]
fn set_readable_local_file_and_remote_file_change_local() {
    let f = Fixture::new();
    let action_fs = f.create_action_file_system();
    let path = f.get_output_path("file");
    inject_remote_file(&action_fs, &path, "remote-content");
    f.write_local_file(&path, "local-content");
    assert!(action_fs.get_path(&path).is_readable().unwrap());
    assert!(local_fs(&action_fs).is_readable(&path).unwrap());

    action_fs.get_path(&path).set_readable(false).unwrap();

    assert!(!action_fs.get_path(&path).is_readable().unwrap());
    assert!(!local_fs(&action_fs).is_readable(&path).unwrap());
}

#[test]
fn set_writable_file_does_not_exist_throw_error() {
    let f = Fixture::new();
    let action_fs = f.create_action_file_system();
    let path = f.get_output_path("file");

    assert_file_not_found(action_fs.get_path(&path).set_writable(false));
}

#[test]
fn set_writable_only_remote_file_remains_writable() {
    let f = Fixture::new();
    let action_fs = f.create_action_file_system();
    let path = f.get_output_path("file");
    inject_remote_file(&action_fs, &path, "remote-content");

    action_fs.get_path(&path).set_writable(false).unwrap();

    assert!(action_fs.get_path(&path).is_writable().unwrap());
}

#[test]
fn set_writable_only_remote_direcotry_remains_writable() {
    let f = Fixture::new();
    let action_fs = f.create_action_file_system();
    let path = f.get_output_path("dir");
    remote_fs(&action_fs)
        .create_directory_and_parents(&path)
        .unwrap();

    action_fs.get_path(&path).set_writable(false).unwrap();

    assert!(action_fs.get_path(&path).is_writable().unwrap());
}

#[test]
fn set_writable_local_file_change() {
    let f = Fixture::new();
    let action_fs = f.create_action_file_system();
    let path = f.get_output_path("file");
    f.write_local_file(&path, "local-content");
    assert!(action_fs.get_path(&path).is_writable().unwrap());

    action_fs.get_path(&path).set_writable(false).unwrap();

    assert!(!action_fs.get_path(&path).is_writable().unwrap());
}

#[test]
fn set_writable_local_file_and_remote_file_change_local() {
    let f = Fixture::new();
    let action_fs = f.create_action_file_system();
    let path = f.get_output_path("file");
    inject_remote_file(&action_fs, &path, "remote-content");
    f.write_local_file(&path, "local-content");
    assert!(action_fs.get_path(&path).is_writable().unwrap());
    assert!(local_fs(&action_fs).is_writable(&path).unwrap());

    action_fs.get_path(&path).set_writable(false).unwrap();

    assert!(!action_fs.get_path(&path).is_writable().unwrap());
    assert!(!local_fs(&action_fs).is_writable(&path).unwrap());
}

#[test]
fn set_executable_file_does_not_exist_throw_error() {
    let f = Fixture::new();
    let action_fs = f.create_action_file_system();
    let path = f.get_output_path("file");

    assert_file_not_found(action_fs.get_path(&path).set_executable(false));
}

#[test]
fn set_executable_only_remote_file_remains_executable() {
    let f = Fixture::new();
    let action_fs = f.create_action_file_system();
    let path = f.get_output_path("file");
    inject_remote_file(&action_fs, &path, "remote-content");

    action_fs.get_path(&path).set_executable(false).unwrap();

    assert!(action_fs.get_path(&path).is_executable().unwrap());
}

#[test]
fn set_executable_only_remote_direcotry_remains_executable() {
    let f = Fixture::new();
    let action_fs = f.create_action_file_system();
    let path = f.get_output_path("dir");
    remote_fs(&action_fs)
        .create_directory_and_parents(&path)
        .unwrap();

    action_fs.get_path(&path).set_executable(false).unwrap();

    assert!(action_fs.get_path(&path).is_executable().unwrap());
}

#[test]
fn set_executable_local_file_change() {
    let f = Fixture::new();
    let action_fs = f.create_action_file_system();
    let path = f.get_output_path("file");
    f.write_local_file(&path, "local-content");
    assert!(!action_fs.get_path(&path).is_executable().unwrap());

    action_fs.get_path(&path).set_executable(true).unwrap();

    assert!(action_fs.get_path(&path).is_executable().unwrap());
}

#[test]
fn set_executable_local_file_and_remote_file_change_local() {
    let f = Fixture::new();
    let action_fs = f.create_action_file_system();
    let path = f.get_output_path("file");
    inject_remote_file(&action_fs, &path, "remote-content");
    f.write_local_file(&path, "local-content");
    assert!(!action_fs.get_path(&path).is_executable().unwrap());
    assert!(!local_fs(&action_fs).is_executable(&path).unwrap());

    action_fs.get_path(&path).set_executable(true).unwrap();

    assert!(action_fs.get_path(&path).is_executable().unwrap());
    assert!(local_fs(&action_fs).is_executable(&path).unwrap());
}

#[test]
fn chmod_file_does_not_exist_throw_error() {
    let f = Fixture::new();
    let action_fs = f.create_action_file_system();
    let path = f.get_output_path("file");

    assert_file_not_found(action_fs.get_path(&path).chmod(0o000));
}

#[test]
fn chmod_only_remote_file_remains_same() {
    let f = Fixture::new();
    let action_fs = f.create_action_file_system();
    let path = f.get_output_path("file");
    inject_remote_file(&action_fs, &path, "remote-content");
    assert!(action_fs.get_path(&path).is_readable().unwrap());
    assert!(action_fs.get_path(&path).is_writable().unwrap());
    assert!(action_fs.get_path(&path).is_executable().unwrap());

    action_fs.get_path(&path).chmod(0o000).unwrap();

    assert!(action_fs.get_path(&path).is_readable().unwrap());
    assert!(action_fs.get_path(&path).is_writable().unwrap());
    assert!(action_fs.get_path(&path).is_executable().unwrap());
}

#[test]
fn chmod_only_remote_directory_remains_same() {
    let f = Fixture::new();
    let action_fs = f.create_action_file_system();
    let path = f.get_output_path("dir");
    remote_fs(&action_fs)
        .create_directory_and_parents(&path)
        .unwrap();
    assert!(action_fs.get_path(&path).is_readable().unwrap());
    assert!(action_fs.get_path(&path).is_writable().unwrap());
    assert!(action_fs.get_path(&path).is_executable().unwrap());

    action_fs.get_path(&path).chmod(0o000).unwrap();

    assert!(action_fs.get_path(&path).is_readable().unwrap());
    assert!(action_fs.get_path(&path).is_writable().unwrap());
    assert!(action_fs.get_path(&path).is_executable().unwrap());
}

#[test]
fn chmod_local_file_change() {
    let f = Fixture::new();
    let action_fs = f.create_action_file_system();
    let path = f.get_output_path("file");
    f.write_local_file(&path, "local-content");
    assert!(action_fs.get_path(&path).is_readable().unwrap());
    assert!(action_fs.get_path(&path).is_writable().unwrap());
    assert!(!action_fs.get_path(&path).is_executable().unwrap());

    action_fs.get_path(&path).chmod(0o111).unwrap();

    assert!(!action_fs.get_path(&path).is_readable().unwrap());
    assert!(!action_fs.get_path(&path).is_writable().unwrap());
    assert!(action_fs.get_path(&path).is_executable().unwrap());
}

#[test]
fn chmod_local_file_and_remote_file_change_local() {
    let f = Fixture::new();
    let action_fs = f.create_action_file_system();
    let path = f.get_output_path("file");
    inject_remote_file(&action_fs, &path, "remote-content");
    f.write_local_file(&path, "local-content");
    assert!(action_fs.get_path(&path).is_readable().unwrap());
    assert!(action_fs.get_path(&path).is_writable().unwrap());
    assert!(!action_fs.get_path(&path).is_executable().unwrap());
    assert!(local_fs(&action_fs).is_readable(&path).unwrap());
    assert!(local_fs(&action_fs).is_writable(&path).unwrap());
    assert!(!local_fs(&action_fs).is_executable(&path).unwrap());

    action_fs.get_path(&path).chmod(0o111).unwrap();

    assert!(!action_fs.get_path(&path).is_readable().unwrap());
    assert!(!action_fs.get_path(&path).is_writable().unwrap());
    assert!(action_fs.get_path(&path).is_executable().unwrap());
    assert!(!local_fs(&action_fs).is_readable(&path).unwrap());
    assert!(!local_fs(&action_fs).is_writable(&path).unwrap());
    assert!(local_fs(&action_fs).is_executable(&path).unwrap());
}

#[test]
fn get_last_modified_time_file_does_not_exist_throw_error() {
    let f = Fixture::new();
    let action_fs = f.create_action_file_system();
    let path = f.get_output_path("file");

    assert_file_not_found(action_fs.get_path(&path).get_last_modified_time());
}

#[test]
fn get_last_modified_time_only_remote_file_return_remote() {
    let f = Fixture::new();
    let action_fs = f.create_action_file_system();
    let path = f.get_output_path("file");
    inject_remote_file(&action_fs, &path, "remote-content");

    let mtime = action_fs.get_path(&path).get_last_modified_time().unwrap();

    assert_eq!(
        mtime,
        remote_fs(&action_fs)
            .get_last_modified_time(&path, true)
            .unwrap()
    );
}

#[test]
fn get_last_modified_time_only_local_file_return_local() {
    let f = Fixture::new();
    let action_fs = f.create_action_file_system();
    let path = f.get_output_path("file");
    f.write_local_file(&path, "local-content");

    let mtime = action_fs.get_path(&path).get_last_modified_time().unwrap();

    assert_eq!(
        mtime,
        local_fs(&action_fs)
            .get_last_modified_time(&path, true)
            .unwrap()
    );
}

#[test]
fn get_last_modified_time_local_and_remote_file_return_remote() {
    let f = Fixture::new();
    let action_fs = f.create_action_file_system();
    let path = f.get_output_path("file");
    inject_remote_file(&action_fs, &path, "remote-content");
    f.write_local_file(&path, "local-content");

    let mtime = action_fs.get_path(&path).get_last_modified_time().unwrap();

    assert_eq!(
        mtime,
        remote_fs(&action_fs)
            .get_last_modified_time(&path, true)
            .unwrap()
    );
}

#[test]
fn set_last_modified_time_file_does_not_exist_throw_error() {
    let f = Fixture::new();
    let action_fs = f.create_action_file_system();
    let path = f.get_output_path("file");

    assert_file_not_found(action_fs.get_path(&path).set_last_modified_time(0));
}

#[test]
fn set_last_modified_time_only_remote_file_successfully_set() {
    let f = Fixture::new();
    let action_fs = f.create_action_file_system();
    let path = f.get_output_path("file");
    inject_remote_file(&action_fs, &path, "remote-content");
    assert_ne!(
        action_fs.get_path(&path).get_last_modified_time().unwrap(),
        0
    );

    action_fs.get_path(&path).set_last_modified_time(0).unwrap();

    assert_eq!(
        action_fs.get_path(&path).get_last_modified_time().unwrap(),
        0
    );
}

#[test]
fn set_last_modified_time_only_local_file_successfully_set() {
    let f = Fixture::new();
    let action_fs = f.create_action_file_system();
    let path = f.get_output_path("file");
    f.write_local_file(&path, "local-content");
    assert_ne!(
        action_fs.get_path(&path).get_last_modified_time().unwrap(),
        0
    );

    action_fs.get_path(&path).set_last_modified_time(0).unwrap();

    assert_eq!(
        action_fs.get_path(&path).get_last_modified_time().unwrap(),
        0
    );
}

#[test]
fn set_last_modified_time_local_and_remote_file_change_both() {
    let f = Fixture::new();
    let action_fs = f.create_action_file_system();
    let path = f.get_output_path("file");
    inject_remote_file(&action_fs, &path, "remote-content");
    f.write_local_file(&path, "local-content");
    assert_ne!(
        local_fs(&action_fs)
            .get_last_modified_time(&path, true)
            .unwrap(),
        0
    );
    assert_ne!(
        remote_fs(&action_fs)
            .get_last_modified_time(&path, true)
            .unwrap(),
        0
    );

    action_fs.get_path(&path).set_last_modified_time(0).unwrap();

    assert_eq!(
        local_fs(&action_fs)
            .get_last_modified_time(&path, true)
            .unwrap(),
        0
    );
    assert_eq!(
        remote_fs(&action_fs)
            .get_last_modified_time(&path, true)
            .unwrap(),
        0
    );
}

// ---------------------------------------------------------------------------------------------
// RemoteActionFileSystemTest
// ---------------------------------------------------------------------------------------------

#[test]
fn get_input_stream_for_local_source_file() {
    let f = Fixture::new();
    // arrange
    let artifact = f.dirs.source_artifact("src");
    let action_fs = f.create_action_file_system();
    f.write_local_file(artifact.get_path().as_fragment(), "local contents");

    // act
    let action_fs_path = action_fs.get_path(artifact.get_path().as_fragment());
    let contents = read_content(&action_fs, action_fs_path.as_fragment());

    // assert
    assert!(Arc::ptr_eq(
        action_fs_path.get_file_system(),
        &action_fs.as_file_system()
    ));
    assert_eq!(contents, "local contents");
}

#[test]
fn test_get_input_stream_for_local_output_file() {
    let f = Fixture::new();
    // arrange
    let artifact = f.dirs.bin_artifact("src");
    let action_fs = f.create_action_file_system();
    f.write_local_file(artifact.get_path().as_fragment(), "local contents");

    // act
    let action_fs_path = action_fs.get_path(artifact.get_path().as_fragment());
    let contents = read_content(&action_fs, action_fs_path.as_fragment());

    // assert
    assert!(Arc::ptr_eq(
        action_fs_path.get_file_system(),
        &action_fs.as_file_system()
    ));
    assert_eq!(contents, "local contents");
}

/// PORT: Java also verifies `Priority.CRITICAL` and `Reason.INPUTS`; the blocking
/// `ActionInputPrefetcher` seam of `blaze_exec` carries neither.
#[test]
fn test_get_input_stream_for_remote_output_file() {
    let f = Fixture::new();
    // arrange
    let inputs = Arc::new(InputMap::default());
    let artifact = f.create_remote_artifact("remote-file", "remote contents", &inputs);
    let action_fs = f.create_action_file_system_with(inputs);
    f.fetcher.mock_prefetch_file(&artifact, "remote contents");

    // act
    let action_fs_path = action_fs.get_path(artifact.get_path().as_fragment());
    let contents = read_content(&action_fs, action_fs_path.as_fragment());

    // assert
    assert!(Arc::ptr_eq(
        action_fs_path.get_file_system(),
        &action_fs.as_file_system()
    ));
    assert_eq!(contents, "remote contents");
    // verify(inputFetcher).prefetchFiles(any(), eq(ImmutableList.of(artifact)), ...);
    // verifyNoMoreInteractions(inputFetcher);
    assert_eq!(
        f.fetcher.calls(),
        vec![vec![artifact.get_exec_path().clone()]]
    );
}

#[test]
fn stat_and_exists_from_input_artifact_data_file() {
    let f = Fixture::new();
    let inputs = Arc::new(InputMap::default());
    let artifact = f.create_local_artifact("local-file", "local contents", &inputs);
    let path = artifact.get_path().as_fragment().clone();
    let metadata = inputs
        .get_input_metadata(&artifact_input(&artifact))
        .unwrap()
        .unwrap();
    let action_fs = f.create_action_file_system_with(inputs);

    assert!(action_fs.exists(&path, true));

    let st = action_fs.stat(&path, true).unwrap();
    assert!(st.is_file());
    // assertThat(st).isInstanceOf(FileStatusWithDigest.class); its getDigest():
    let st_metadata = action_fs.stat_metadata(&path, true).unwrap().unwrap();
    assert_eq!(st_metadata.get_digest(), metadata.get_digest());
}

#[test]
fn stat_and_exists_from_input_artifact_data_tree_sub_dir() {
    let f = Fixture::new();
    let inputs = Arc::new(InputMap::default());
    let tree = f.create_local_tree_artifact("tree", &[("subdir/file", "")], &inputs);
    let path = tree.get_path().get_child("subdir").as_fragment().clone();
    let action_fs = f.create_action_file_system_with(inputs);

    assert!(action_fs.exists(&path, true));

    let st = action_fs.stat(&path, true).unwrap();
    assert!(st.is_directory());
}

#[test]
fn stat_and_exists_from_remote_output_tree() {
    let f = Fixture::new();
    let action_fs = f.create_action_file_system();
    let artifact = f.dirs.bin_artifact("out");
    let path = artifact.get_path().as_fragment().clone();
    let metadata = inject_remote_file(
        &action_fs,
        artifact.get_path().as_fragment(),
        "remote contents",
    );

    assert!(action_fs.exists(&path, true));

    let st = action_fs.stat(&path, true).unwrap();
    assert!(st.is_file());
    // assertThat(st).isInstanceOf(FileStatusWithDigest.class); its getDigest():
    let st_metadata = action_fs.stat_metadata(&path, true).unwrap().unwrap();
    assert_eq!(st_metadata.get_digest(), metadata.get_digest());
}

#[test]
fn stat_and_exists_from_local_filesystem() {
    let f = Fixture::new();
    let action_fs = f.create_action_file_system();
    let artifact = f.dirs.bin_artifact("out");
    let path = artifact.get_path().as_fragment().clone();
    f.write_local_file(artifact.get_path().as_fragment(), "local contents");

    assert!(action_fs.exists(&path, true));

    let st = action_fs.stat(&path, true).unwrap();
    assert!(st.is_file());
    assert_eq!(st.get_size(), "local contents".len() as i64);
}

#[test]
fn stat_and_exists_follow_symlinks() {
    for from in FILESYSTEM_TEST_PARAMS {
        for to in FILESYSTEM_TEST_PARAMS {
            let f = Fixture::new();
            let action_fs = f.create_action_file_system();
            let from_fs = from.get_filesystem(&action_fs);

            let link_path = f.get_output_path("sym");
            let target_path = f.get_output_path("target");
            from_fs
                .create_symbolic_link(&link_path, &target_path)
                .unwrap();

            assert!(action_fs.exists(&link_path, false));
            assert!(!action_fs.exists(&link_path, true));
            assert!(
                action_fs
                    .stat(&link_path, false)
                    .unwrap()
                    .is_symbolic_link()
            );
            assert_file_not_found(action_fs.stat(&link_path, true));

            write_to(&f, &action_fs, to, &target_path, "content");

            assert!(action_fs.exists(&link_path, false));
            assert!(
                action_fs
                    .stat(&link_path, false)
                    .unwrap()
                    .is_symbolic_link()
            );
            assert!(action_fs.exists(&link_path, true));
            assert!(action_fs.stat(&link_path, true).unwrap().is_file());
        }
    }
}

#[test]
fn stat_and_exists_not_found() {
    let f = Fixture::new();
    let action_fs = f.create_action_file_system();
    let path = f.get_output_path("does_not_exist");

    assert!(!action_fs.exists(&path, true));

    assert!(action_fs.stat_if_found(&path, true).unwrap().is_none());

    assert_file_not_found(action_fs.stat(&path, true));
}

#[test]
fn stat_and_exists_is_not_directory() {
    let f = Fixture::new();
    let action_fs = f.create_action_file_system();
    let non_dir_path = f.get_output_path("non_dir");
    let path = non_dir_path.get_child("file");

    f.write_local_file(&non_dir_path, "content");

    assert!(!action_fs.exists(&path, true));

    assert!(action_fs.stat_if_found(&path, true).unwrap().is_none());

    assert_file_not_found(action_fs.stat(&path, true));
}

#[test]
fn stat_and_exists_dangling_symlink_not_found() {
    let f = Fixture::new();
    let action_fs = f.create_action_file_system();
    let path = f.get_output_path("sym");

    action_fs
        .get_path(&path)
        .create_symbolic_link_to_fragment(&PathFragment::create("/does_not_exist"))
        .unwrap();

    assert!(!action_fs.exists(&path, true));

    assert!(action_fs.stat_if_found(&path, true).unwrap().is_none());

    assert_file_not_found(action_fs.stat(&path, true));
}

#[test]
fn delete_delete_symlink() {
    let f = Fixture::new();
    let action_fs = f.create_action_file_system();

    let link_path = f.get_output_path("link");
    let target_path = f.get_output_path("target");
    action_fs
        .get_path(&link_path)
        .create_symbolic_link_to_fragment(&target_path)
        .unwrap();
    f.write_local_file(&target_path, "content");

    assert!(action_fs.delete(&link_path).unwrap());
    assert!(!action_fs.exists(&link_path, false));
    assert!(action_fs.exists(&target_path, false));
}

#[test]
fn delete_follow_symlinks() {
    for from in FILESYSTEM_TEST_PARAMS {
        for to in FILESYSTEM_TEST_PARAMS {
            let f = Fixture::new();
            let action_fs = f.create_action_file_system();
            let from_fs = from.get_filesystem(&action_fs);

            let dir_link_path = f.get_output_path("dirLink");
            let dir_target_path = f.get_output_path("dirTarget");
            from_fs
                .create_symbolic_link(&dir_link_path, &dir_target_path)
                .unwrap();
            action_fs
                .get_path(&dir_target_path)
                .create_directory_and_parents()
                .unwrap();

            let naive_path = dir_link_path.get_child("file");
            let canonical_path = dir_target_path.get_child("file");

            write_to(&f, &action_fs, to, &canonical_path, "content");

            assert!(action_fs.delete(&naive_path).unwrap(), "{from:?} {to:?}");
            assert!(!action_fs.exists(&naive_path, false));
            assert!(!action_fs.exists(&canonical_path, false));
        }
    }
}

#[test]
fn delete_invalidates_resolve_symbolic_links_cache() {
    let f = Fixture::new();
    let action_fs = f.create_action_file_system();
    let link_path = f.get_output_path("sym");
    let target_path = f.get_output_path("target");

    action_fs
        .get_path(&link_path)
        .get_parent_directory()
        .unwrap()
        .create_directory_and_parents()
        .unwrap();
    action_fs
        .get_path(&link_path)
        .create_symbolic_link_to_fragment(&target_path)
        .unwrap();
    f.write_local_file(&target_path, "content");

    assert_eq!(
        action_fs
            .get_path(&link_path)
            .resolve_symbolic_links()
            .unwrap(),
        action_fs.get_path(&target_path)
    );

    assert!(action_fs.delete(&link_path).unwrap());
    f.write_local_file(&link_path, "content");

    assert_eq!(
        action_fs
            .get_path(&link_path)
            .resolve_symbolic_links()
            .unwrap(),
        action_fs.get_path(&link_path)
    );
}

#[test]
fn create_output_directories_after_missing_parent_delete() {
    for output_path in ["ns/a", "ns/nested/a"] {
        let f = Fixture::new();
        let action_fs = f.create_action_file_system();
        let namespace = f.get_output_path("ns");
        f.fs.create_symbolic_link(&namespace, &PathFragment::create("missing"))
            .unwrap();

        // Output preparation caches a dangling parent left by a previous build. Another output's
        // preparation then replaces it with a directory, as in AbstractActionInputPrefetcher.
        assert!(!action_fs.delete(&f.get_output_path(output_path)).unwrap());
        let helper =
            blaze_exec::actions::action_output_directory_helper::ActionOutputDirectoryHelper::new();
        helper
            .create_output_directories(&[f.dirs.bin_artifact("ns/b")])
            .unwrap();
        assert!(f.fs.is_directory(&namespace, false));
        assert!(!f.fs.exists(&f.get_output_path("missing"), true));
        // helper.createActionFsOutputDirectories(
        //     ImmutableList.of(ActionsTestUtil.createArtifact(outputRoot, outputPath)),
        //     ArtifactPathResolver.createPathResolver(actionFs, execRoot));
        helper
            .create_action_fs_output_directories(
                &[f.dirs.bin_artifact(output_path)],
                Some(&action_fs.as_file_system()),
            )
            .unwrap();

        assert!(
            action_fs
                .get_path(&f.get_output_path(output_path))
                .get_parent_directory()
                .unwrap()
                .is_directory()
        );
    }
}

#[test]
fn set_last_modified_time_for_remote_output_tree() {
    let f = Fixture::new();
    let action_fs = f.create_action_file_system();
    let artifact = f.dirs.bin_artifact("out");
    let path = artifact.get_path().as_fragment().clone();
    inject_remote_file(
        &action_fs,
        artifact.get_path().as_fragment(),
        "remote contents",
    );

    action_fs
        .get_path(&path)
        .set_last_modified_time(1234567890)
        .unwrap();
    assert_eq!(
        action_fs.get_path(&path).get_last_modified_time().unwrap(),
        1234567890
    );
}

#[test]
fn set_last_modified_time_for_local_filesystem() {
    let f = Fixture::new();
    let action_fs = f.create_action_file_system();
    let artifact = f.dirs.bin_artifact("out");
    let path = artifact.get_path().as_fragment().clone();
    f.write_local_file(artifact.get_path().as_fragment(), "local contents");

    action_fs
        .get_path(&path)
        .set_last_modified_time(1234567890)
        .unwrap();
    assert_eq!(
        action_fs.get_path(&path).get_last_modified_time().unwrap(),
        1234567890
    );
}

#[test]
fn set_last_modified_time_follow_symlinks() {
    for from in FILESYSTEM_TEST_PARAMS {
        for to in FILESYSTEM_TEST_PARAMS {
            let f = Fixture::new();
            let action_fs = f.create_action_file_system();
            let from_fs = from.get_filesystem(&action_fs);

            let link_path = f.get_output_path("sym");
            let target_path = f.get_output_path("target");
            from_fs
                .create_symbolic_link(&link_path, &target_path)
                .unwrap();

            write_to(&f, &action_fs, to, &target_path, "content");

            action_fs
                .get_path(&link_path)
                .set_last_modified_time(1234567890)
                .unwrap();
            assert_eq!(
                action_fs
                    .get_path(&link_path)
                    .get_last_modified_time()
                    .unwrap(),
                1234567890
            );
            assert_eq!(
                action_fs
                    .get_path(&target_path)
                    .get_last_modified_time()
                    .unwrap(),
                1234567890
            );
        }
    }
}

#[test]
fn get_digest_from_input_artifact_data_for_local_artifact() {
    let f = Fixture::new();
    let inputs = Arc::new(InputMap::default());
    let artifact = f.create_remote_artifact("file", "local contents", &inputs);
    let path = artifact.get_path().as_fragment().clone();
    let action_fs = f.create_action_file_system_with(inputs);

    // Verify that we don't fall back to a slow digest.
    f.fs.reset();
    assert_eq!(
        action_fs.get_fast_digest(&path).unwrap(),
        Some(get_digest("local contents"))
    );
    assert_eq!(f.fs.get_digest_calls(), 0);

    assert_eq!(
        action_fs.get_digest(&path).unwrap(),
        get_digest("local contents")
    );
}

#[test]
fn get_digest_from_input_artifact_data_for_remote_artifact() {
    let f = Fixture::new();
    let inputs = Arc::new(InputMap::default());
    let artifact = f.create_remote_artifact("file", "remote contents", &inputs);
    let path = artifact.get_path().as_fragment().clone();
    let action_fs = f.create_action_file_system_with(inputs);

    // Verify that we don't fall back to a slow digest.
    f.fs.reset();
    assert_eq!(
        action_fs.get_fast_digest(&path).unwrap(),
        Some(get_digest("remote contents"))
    );
    assert_eq!(f.fs.get_digest_calls(), 0);

    assert_eq!(
        action_fs.get_digest(&path).unwrap(),
        get_digest("remote contents")
    );
}

#[test]
fn get_digest_from_remote_output_tree() {
    let f = Fixture::new();
    let action_fs = f.create_action_file_system();
    let artifact = f.dirs.bin_artifact("out");
    let path = artifact.get_path().as_fragment().clone();
    inject_remote_file(
        &action_fs,
        artifact.get_path().as_fragment(),
        "remote contents",
    );

    assert_eq!(
        action_fs.get_fast_digest(&path).unwrap(),
        Some(get_digest("remote contents"))
    );
    assert_eq!(
        action_fs.get_digest(&path).unwrap(),
        get_digest("remote contents")
    );
}

#[test]
fn get_digest_from_local_filesystem() {
    let f = Fixture::new();
    let action_fs = f.create_action_file_system();
    let artifact = f.dirs.bin_artifact("out");
    let path = artifact.get_path().as_fragment().clone();
    f.write_local_file(artifact.get_path().as_fragment(), "local contents");

    assert_eq!(action_fs.get_fast_digest(&path).unwrap(), None);
    assert_eq!(
        action_fs.get_digest(&path).unwrap(),
        get_digest("local contents")
    );
}

#[test]
fn get_digest_not_found() {
    let f = Fixture::new();
    let action_fs = f.create_action_file_system();
    let artifact = f.dirs.bin_artifact("out");
    let path = artifact.get_path().as_fragment().clone();

    assert_file_not_found(action_fs.get_fast_digest(&path));
    assert_file_not_found(action_fs.get_digest(&path));
}

#[test]
fn get_digest_follow_symlinks() {
    for from in FILESYSTEM_TEST_PARAMS {
        for to in FILESYSTEM_TEST_PARAMS {
            let f = Fixture::new();
            let action_fs = f.create_action_file_system();
            let from_fs = from.get_filesystem(&action_fs);

            let link_path = f.get_output_path("sym");
            let target_path = f.get_output_path("target");
            from_fs
                .create_symbolic_link(&link_path, &target_path)
                .unwrap();

            if to == Local {
                f.write_local_file(&target_path, "content");
                assert_eq!(action_fs.get_fast_digest(&link_path).unwrap(), None);
            } else {
                inject_remote_file(&action_fs, &target_path, "content");
                assert_eq!(
                    action_fs.get_fast_digest(&link_path).unwrap(),
                    Some(get_digest("content"))
                );
            }

            assert_eq!(
                action_fs.get_digest(&link_path).unwrap(),
                get_digest("content")
            );
        }
    }
}

#[test]
fn readdir_from_remote_output_tree() {
    let f = Fixture::new();
    let action_fs = f.create_action_file_system();
    inject_remote_file(&action_fs, &f.get_output_path("dir/out1"), "contents1");
    inject_remote_file(&action_fs, &f.get_output_path("dir/out2"), "contents2");
    inject_remote_file(
        &action_fs,
        &f.get_output_path("dir/subdir/out3"),
        "contents3",
    );
    let dir_path = f.get_output_path("dir");

    assert_readdir(
        &action_fs,
        &dir_path,
        true,
        &[
            Dirent::new("out1", DirentType::File),
            Dirent::new("out2", DirentType::File),
            Dirent::new("subdir", DirentType::Directory),
        ],
    );

    assert_readdir_throws(&action_fs, &f.get_output_path("dir/out1"), true);
}

#[test]
fn readdir_from_local_filesystem() {
    let f = Fixture::new();
    let action_fs = f.create_action_file_system();
    f.write_local_file(&f.get_output_path("dir/file"), "contents");
    f.write_local_file(&f.get_output_path("dir/subdir/file"), "contents");
    let dir_path = f.get_output_path("dir");

    assert_readdir(
        &action_fs,
        &dir_path,
        true,
        &[
            Dirent::new("file", DirentType::File),
            Dirent::new("subdir", DirentType::Directory),
        ],
    );

    assert_readdir_throws(&action_fs, &f.get_output_path("dir/out1"), true);
}

#[test]
fn readdir_from_input_artifact_data() {
    let f = Fixture::new();
    let inputs = Arc::new(InputMap::default());
    f.create_local_tree_artifact("tree", &[("file", ""), ("dir/subdir/subfile", "")], &inputs);
    let action_fs = f.create_action_file_system_with(inputs);

    assert_readdir(
        &action_fs,
        &f.get_output_path("tree"),
        true,
        &[
            Dirent::new("dir", DirentType::Directory),
            Dirent::new("file", DirentType::File),
        ],
    );

    assert_readdir(
        &action_fs,
        &f.get_output_path("tree/dir"),
        true,
        &[Dirent::new("subdir", DirentType::Directory)],
    );

    assert_readdir(
        &action_fs,
        &f.get_output_path("tree/dir/subdir"),
        true,
        &[Dirent::new("subfile", DirentType::File)],
    );

    assert_readdir_throws(
        &action_fs,
        &f.get_output_path("tree/dir/subdir/subfile"),
        true,
    );
}

#[test]
fn readdir_from_input_artifact_data_empty_dir() {
    let f = Fixture::new();
    let inputs = Arc::new(InputMap::default());
    f.create_local_tree_artifact("tree", &[], &inputs);
    let action_fs = f.create_action_file_system_with(inputs);

    assert_readdir(&action_fs, &f.get_output_path("tree"), true, &[]);
}

#[test]
fn readdir_from_remote_output_tree_and_local_filesystem() {
    let f = Fixture::new();
    let inputs = Arc::new(InputMap::default());
    let action_fs = f.create_action_file_system_with(inputs);
    f.write_local_file(&f.get_output_path("dir/out1"), "contents1");
    inject_remote_file(&action_fs, &f.get_output_path("dir/out2"), "contents2");
    let dir_path = f.get_output_path("dir");

    assert_readdir(
        &action_fs,
        &dir_path,
        true,
        &[
            Dirent::new("out1", DirentType::File),
            Dirent::new("out2", DirentType::File),
        ],
    );
}

#[test]
fn readdir_from_remote_output_tree_and_local_filesystem_empty_dir() {
    let f = Fixture::new();
    let inputs = Arc::new(InputMap::default());
    let action_fs = f.create_action_file_system_with(inputs);
    let dir_path = f.get_output_path("dir");
    remote_fs(&action_fs)
        .create_directory_and_parents(&dir_path)
        .unwrap();
    local_fs(&action_fs)
        .create_directory_and_parents(&dir_path)
        .unwrap();

    assert_readdir(&action_fs, &dir_path, true, &[]);
}

#[test]
fn readdir_follow_symlinks_for_directory() {
    for from in FILESYSTEM_TEST_PARAMS {
        for to in FILESYSTEM_TEST_PARAMS {
            let f = Fixture::new();
            let action_fs = f.create_action_file_system();
            let from_fs = from.get_filesystem(&action_fs);
            let to_fs = to.get_filesystem(&action_fs);

            let link_path = f.get_output_path("sym");
            let target_path = f.get_output_path("dir");
            let child_path = f.get_output_path("dir/child");

            from_fs
                .create_symbolic_link(&link_path, &target_path)
                .unwrap();
            to_fs.create_directory(&target_path).unwrap();

            write_to(&f, &action_fs, to, &child_path, "content");

            assert_readdir(
                &action_fs,
                &link_path,
                false,
                &[Dirent::new("child", DirentType::File)],
            );
            assert_readdir(
                &action_fs,
                &link_path,
                true,
                &[Dirent::new("child", DirentType::File)],
            );
        }
    }
}

#[test]
fn readdir_follow_symlinks_for_directory_entries() {
    for from in FILESYSTEM_TEST_PARAMS {
        for to in FILESYSTEM_TEST_PARAMS {
            let f = Fixture::new();
            let action_fs = f.create_action_file_system();
            let from_fs = from.get_filesystem(&action_fs);

            let dir_path = f.get_output_path("dir");
            let file_link_path = f.get_output_path("dir/file_sym");
            let file_target_path = f.get_output_path("file_target");
            let dir_link_path = f.get_output_path("dir/dir_sym");
            let dir_target_path = f.get_output_path("dir_target");
            let looping_link_path = f.get_output_path("dir/looping_sym");
            let dangling_link_path = f.get_output_path("dir/dangling_sym");

            from_fs.create_directory(&dir_path).unwrap();
            from_fs
                .create_symbolic_link(&file_link_path, &file_target_path)
                .unwrap();
            from_fs
                .create_symbolic_link(&dir_link_path, &dir_target_path)
                .unwrap();
            from_fs
                .create_symbolic_link(&looping_link_path, &looping_link_path)
                .unwrap();
            from_fs
                .create_symbolic_link(
                    &dangling_link_path,
                    &PathFragment::create("/does_not_exist"),
                )
                .unwrap();

            if to == Local {
                f.write_local_file(&file_target_path, "content");
                local_fs(&action_fs)
                    .create_directory_and_parents(&dir_target_path)
                    .unwrap();
            } else {
                inject_remote_file(&action_fs, &file_target_path, "content");
                action_fs
                    .get_remote_in_memory_file_system()
                    .create_directory_and_parents(&dir_target_path)
                    .unwrap();
            }

            assert_readdir(
                &action_fs,
                &dir_path,
                false,
                &[
                    Dirent::new("file_sym", DirentType::Symlink),
                    Dirent::new("dir_sym", DirentType::Symlink),
                    Dirent::new("looping_sym", DirentType::Symlink),
                    Dirent::new("dangling_sym", DirentType::Symlink),
                ],
            );
            assert_readdir(
                &action_fs,
                &dir_path,
                true,
                &[
                    Dirent::new("file_sym", DirentType::File),
                    Dirent::new("dir_sym", DirentType::Directory),
                    Dirent::new("looping_sym", DirentType::Unknown),
                    Dirent::new("dangling_sym", DirentType::Unknown),
                ],
            );
        }
    }
}

#[test]
fn readdir_non_directory() {
    let f = Fixture::new();
    let action_fs = f.create_action_file_system();
    let artifact = f.dirs.bin_artifact("dir/out");
    let path = artifact
        .get_path()
        .get_parent_directory()
        .unwrap()
        .as_fragment()
        .clone();

    f.write_local_file(&path, "content");

    assert_readdir_throws(&action_fs, &path, true);
}

#[test]
fn readdir_not_found() {
    let f = Fixture::new();
    let action_fs = f.create_action_file_system();
    let artifact = f.dirs.bin_artifact("dir/out");
    let path = artifact
        .get_path()
        .get_parent_directory()
        .unwrap()
        .as_fragment()
        .clone();

    assert_readdir_throws(&action_fs, &path, true);
}

#[test]
fn permissions_follow_symlinks() {
    for from in FILESYSTEM_TEST_PARAMS {
        for to in FILESYSTEM_TEST_PARAMS {
            let f = Fixture::new();
            let action_fs = f.create_action_file_system();
            let from_fs = from.get_filesystem(&action_fs);

            let link_path = f.get_output_path("sym");
            let target_path = f.get_output_path("target");
            from_fs
                .create_symbolic_link(&link_path, &target_path)
                .unwrap();

            assert_file_not_found(action_fs.chmod(&link_path, 0o777));

            write_to(&f, &action_fs, to, &target_path, "content");

            // For a remote file, permissions are always 0777.
            let is_remote = to == Remote;

            assert!(action_fs.get_path(&link_path).is_readable().unwrap());
            assert!(action_fs.get_path(&link_path).is_writable().unwrap());
            assert_eq!(
                action_fs.get_path(&link_path).is_executable().unwrap(),
                is_remote
            );

            action_fs.get_path(&link_path).chmod(0o111).unwrap();
            assert_eq!(
                action_fs.get_path(&link_path).is_readable().unwrap(),
                is_remote
            );
            assert_eq!(
                action_fs.get_path(&link_path).is_writable().unwrap(),
                is_remote
            );
            assert!(action_fs.get_path(&link_path).is_executable().unwrap());

            action_fs.get_path(&link_path).set_readable(true).unwrap();
            action_fs.get_path(&link_path).set_writable(true).unwrap();
            action_fs
                .get_path(&link_path)
                .set_executable(false)
                .unwrap();
            assert!(action_fs.get_path(&link_path).is_readable().unwrap());
            assert!(action_fs.get_path(&link_path).is_writable().unwrap());
            assert_eq!(
                action_fs.get_path(&link_path).is_executable().unwrap(),
                is_remote
            );
        }
    }
}

#[test]
fn read_symbolic_link_from_local_filesystem() {
    let f = Fixture::new();
    let action_fs = f.create_action_file_system();
    let file_path = f.get_output_path("file");
    let link_path = f.get_output_path("sym");
    let target_path = PathFragment::create("/some/path");
    local_fs(&action_fs)
        .create_symbolic_link(&link_path, &target_path)
        .unwrap();
    f.write_local_file(&file_path, "contents");

    assert_eq!(
        action_fs.read_symbolic_link(&link_path).unwrap(),
        target_path
    );

    assert!(NotASymlinkException::is(
        &action_fs.read_symbolic_link(&file_path).unwrap_err()
    ));
}

#[test]
fn read_symbolic_link_from_remote_filesystem() {
    let f = Fixture::new();
    let action_fs = f.create_action_file_system();
    let file_path = f.get_output_path("file");
    let link_path = f.get_output_path("sym");
    let target_path = PathFragment::create("/some/path");
    remote_fs(&action_fs)
        .create_symbolic_link(&link_path, &target_path)
        .unwrap();
    inject_remote_file(&action_fs, &file_path, "contents");

    assert_eq!(
        action_fs.read_symbolic_link(&link_path).unwrap(),
        target_path
    );

    assert!(NotASymlinkException::is(
        &action_fs.read_symbolic_link(&file_path).unwrap_err()
    ));
}

#[test]
fn read_symbolic_link_from_input_artifact_data_regular_file() {
    let f = Fixture::new();
    let inputs = Arc::new(InputMap::default());
    let artifact = f.create_remote_artifact("file", "contents", &inputs);
    let action_fs = f.create_action_file_system_with(inputs);

    assert!(NotASymlinkException::is(
        &action_fs
            .read_symbolic_link(artifact.get_path().as_fragment())
            .unwrap_err()
    ));
}

#[test]
fn read_symbolic_link_from_input_artifact_data_tree_sub_dir() {
    let f = Fixture::new();
    let inputs = Arc::new(InputMap::default());
    let tree = f.create_remote_tree_artifact("tree", &[("subdir/file", "")], &inputs);
    let action_fs = f.create_action_file_system_with(inputs);

    assert!(NotASymlinkException::is(
        &action_fs
            .read_symbolic_link(tree.get_path().as_fragment())
            .unwrap_err()
    ));

    assert!(NotASymlinkException::is(
        &action_fs
            .read_symbolic_link(tree.get_path().get_relative_str("subdir").as_fragment())
            .unwrap_err()
    ));
}

#[test]
fn read_symbolic_link_from_input_artifact_data_unresolved_symlink() {
    let f = Fixture::new();
    let inputs = Arc::new(InputMap::default());
    let action_fs = f.create_action_file_system_with(inputs.clone());

    let symlink = f
        .dirs
        .special_artifact("symlink", SpecialArtifactType::UNRESOLVED_SYMLINK);
    let target_path = PathFragment::create("/some/path");
    // Create symlink on the filesystem so we can digest it, then delete it to verify that its
    // presence in the ActionInputMap is sufficient for readSymbolicLink to work. Note that this
    // is an unrealistic scenario, as symlinks are always materialized even when produced
    // remotely.
    let symlink_path = blaze_vfs::Path::create(
        symlink.get_path().as_fragment().clone(),
        action_fs.get_local_file_system(),
    );
    symlink_path
        .create_symbolic_link_to_fragment(&target_path)
        .unwrap();
    inputs.put(
        &symlink,
        FileArtifactValue::create_for_unresolved_symlink(&symlink_path).unwrap(),
    );
    symlink_path.delete().unwrap();

    assert_eq!(
        action_fs
            .read_symbolic_link(&f.get_output_path("symlink"))
            .unwrap(),
        target_path
    );
}

#[test]
fn read_symbolic_link_not_found() {
    let f = Fixture::new();
    let action_fs = f.create_action_file_system();
    let link_path = f.get_output_path("sym");

    assert_file_not_found(action_fs.read_symbolic_link(&link_path));
}

#[test]
fn create_symbolic_link_local_file_artifact() {
    let f = Fixture::new();
    // arrange
    let inputs = Arc::new(InputMap::default());
    let local_artifact = f.create_local_artifact("local-file", "local contents", &inputs);
    let output_artifact = f.dirs.bin_artifact("out");
    let action_fs = f.create_action_file_system_with(inputs);

    // act
    let link_path = output_artifact.get_path().as_fragment().clone();
    let target_path = local_artifact.get_path().as_fragment().clone();
    let symlink_action_fs = action_fs.get_path(&link_path);
    symlink_action_fs
        .create_symbolic_link(&action_fs.get_path(&target_path))
        .unwrap();

    // assert
    assert!(Arc::ptr_eq(
        symlink_action_fs.get_file_system(),
        &action_fs.as_file_system()
    ));
    assert_eq!(symlink_action_fs.read_symbolic_link().unwrap(), target_path);
    assert_eq!(
        local_fs(&action_fs).read_symbolic_link(&link_path).unwrap(),
        target_path
    );
    assert_eq!(
        local_fs(&action_fs).read_symbolic_link(&link_path).unwrap(),
        target_path
    );
}

#[test]
fn create_symbolic_link_remote_file_artifact() {
    let f = Fixture::new();
    // arrange
    let inputs = Arc::new(InputMap::default());
    let remote_artifact = f.create_remote_artifact("remote-file", "remote contents", &inputs);
    let output_artifact = f.dirs.bin_artifact("out");
    let action_fs = f.create_action_file_system_with(inputs);

    // act
    let link_path = output_artifact.get_path().as_fragment().clone();
    let target_path = remote_artifact.get_path().as_fragment().clone();
    let symlink_action_fs = action_fs.get_path(&link_path);
    symlink_action_fs
        .create_symbolic_link(&action_fs.get_path(&target_path))
        .unwrap();

    // assert
    assert!(Arc::ptr_eq(
        symlink_action_fs.get_file_system(),
        &action_fs.as_file_system()
    ));
    assert_eq!(symlink_action_fs.read_symbolic_link().unwrap(), target_path);
    assert_eq!(
        output_artifact.get_path().read_symbolic_link().unwrap(),
        target_path
    );
    assert_eq!(
        local_fs(&action_fs).read_symbolic_link(&link_path).unwrap(),
        target_path
    );
    assert_eq!(
        local_fs(&action_fs).read_symbolic_link(&link_path).unwrap(),
        target_path
    );
}

#[test]
fn create_symbolic_link_local_tree_artifact() {
    let f = Fixture::new();
    // arrange
    let inputs = Arc::new(InputMap::default());
    let content_map = [("foo", "foo contents"), ("bar", "bar contents")];
    let local_artifact = f.create_local_tree_artifact("remote-dir", &content_map, &inputs);
    let output_artifact = f.create_tree_artifact_with_generating_action("out");
    let action_fs = f.create_action_file_system_with(inputs);

    // act
    let link_path = output_artifact.get_path().as_fragment().clone();
    let target_path = local_artifact.get_path().as_fragment().clone();
    let symlink_action_fs = action_fs.get_path(&link_path);
    symlink_action_fs
        .create_symbolic_link(&action_fs.get_path(&target_path))
        .unwrap();

    // assert
    assert!(Arc::ptr_eq(
        symlink_action_fs.get_file_system(),
        &action_fs.as_file_system()
    ));
    assert_eq!(symlink_action_fs.read_symbolic_link().unwrap(), target_path);
    assert_eq!(
        local_fs(&action_fs).read_symbolic_link(&link_path).unwrap(),
        target_path
    );
    assert_eq!(
        local_fs(&action_fs).read_symbolic_link(&link_path).unwrap(),
        target_path
    );
}

#[test]
fn create_symbolic_link_remote_tree_artifact() {
    let f = Fixture::new();
    // arrange
    let inputs = Arc::new(InputMap::default());
    let content_map = [("foo", "foo contents"), ("bar", "bar contents")];
    let remote_artifact = f.create_remote_tree_artifact("remote-dir", &content_map, &inputs);
    let output_artifact = f.create_tree_artifact_with_generating_action("out");
    let action_fs = f.create_action_file_system_with(inputs);

    // act
    let link_path = output_artifact.get_path().as_fragment().clone();
    let target_path = remote_artifact.get_path().as_fragment().clone();
    let symlink_action_fs = action_fs.get_path(&link_path);
    symlink_action_fs
        .create_symbolic_link(&action_fs.get_path(&target_path))
        .unwrap();

    // assert
    assert!(Arc::ptr_eq(
        symlink_action_fs.get_file_system(),
        &action_fs.as_file_system()
    ));
    assert_eq!(symlink_action_fs.read_symbolic_link().unwrap(), target_path);
    assert_eq!(
        local_fs(&action_fs).read_symbolic_link(&link_path).unwrap(),
        target_path
    );
    assert_eq!(
        local_fs(&action_fs).read_symbolic_link(&link_path).unwrap(),
        target_path
    );
}

#[test]
fn create_symbolic_link_unresolved_symlink() {
    let f = Fixture::new();
    // arrange
    let inputs = Arc::new(InputMap::default());
    let output_artifact = f
        .dirs
        .special_artifact("out", SpecialArtifactType::UNRESOLVED_SYMLINK);
    let action_fs = f.create_action_file_system_with(inputs);
    let target_path = PathFragment::create("some/path");

    // act
    let link_path = output_artifact.get_path().as_fragment().clone();
    let symlink_action_fs = action_fs.get_path(&link_path);
    symlink_action_fs
        .create_symbolic_link_to_fragment(&target_path)
        .unwrap();

    // assert
    assert!(Arc::ptr_eq(
        symlink_action_fs.get_file_system(),
        &action_fs.as_file_system()
    ));
    assert_eq!(symlink_action_fs.read_symbolic_link().unwrap(), target_path);
    assert_eq!(
        local_fs(&action_fs).read_symbolic_link(&link_path).unwrap(),
        target_path
    );
    assert_eq!(
        local_fs(&action_fs).read_symbolic_link(&link_path).unwrap(),
        target_path
    );
}

#[test]
fn create_and_read_symbolic_link_follow_symlinks() {
    for from in FILESYSTEM_TEST_PARAMS {
        // createSymbolicLink writes to both the local and remote filesystem, so it makes no sense
        // to parameterize on the symlink's destination filesystem.
        let f = Fixture::new();
        let action_fs = f.create_action_file_system();
        let from_fs = from.get_filesystem(&action_fs);

        let parent_link_path = f.get_output_path("parent_link");
        let parent_target_path = f.get_output_path("parent_target");
        from_fs
            .create_symbolic_link(&parent_link_path, &parent_target_path)
            .unwrap();
        action_fs
            .get_path(&parent_target_path)
            .create_directory_and_parents()
            .unwrap();

        let link_path = f.get_output_path("parent_target/link");
        let target_path = PathFragment::create("/some/path");
        action_fs
            .get_path(&link_path)
            .create_symbolic_link_to_fragment(&target_path)
            .unwrap();

        assert_eq!(
            action_fs.get_path(&link_path).read_symbolic_link().unwrap(),
            target_path
        );
    }
}

#[test]
fn resolve_symbolic_links() {
    for a in FILESYSTEM_TEST_PARAMS {
        for b in FILESYSTEM_TEST_PARAMS {
            let f = Fixture::new();
            let action_fs = f.create_action_file_system();
            let a_fs = a.get_filesystem(&action_fs);
            let b_fs = b.get_filesystem(&action_fs);

            // /a
            //  |- asub
            //  |  `- afile
            //  `- abssym -> /b/bsub
            //  `- relsym -> asub
            // /b
            //  `- bsub
            //     `- bfile

            a_fs.create_directory_and_parents(&f.get_output_path("a/asub"))
                .unwrap();
            a_fs.create_symbolic_link(&f.get_output_path("a/abssym"), &f.get_output_path("b/bsub"))
                .unwrap();
            a_fs.create_symbolic_link(
                &f.get_output_path("a/relsym"),
                &PathFragment::create("asub"),
            )
            .unwrap();
            write_to(
                &f,
                &action_fs,
                a,
                &f.get_output_path("a/asub/afile"),
                "content",
            );

            b_fs.create_directory_and_parents(&f.get_output_path("b/bsub"))
                .unwrap();
            write_to(
                &f,
                &action_fs,
                b,
                &f.get_output_path("b/bsub/bfile"),
                "content",
            );

            assert_eq!(
                action_fs
                    .get_path(&f.get_output_path("a/relsym/afile"))
                    .resolve_symbolic_links()
                    .unwrap(),
                action_fs.get_path(&f.get_output_path("a/asub/afile"))
            );

            assert_file_not_found(
                action_fs
                    .get_path(&f.get_output_path("a/bsub/nofile"))
                    .resolve_symbolic_links(),
            );

            assert_eq!(
                action_fs
                    .get_path(&f.get_output_path("a/abssym/bfile"))
                    .resolve_symbolic_links()
                    .unwrap(),
                action_fs.get_path(&f.get_output_path("b/bsub/bfile"))
            );

            assert_file_not_found(
                action_fs
                    .get_path(&f.get_output_path("b/bsub/nofile"))
                    .resolve_symbolic_links(),
            );
        }
    }
}

#[test]
fn rename_to_only_local_file_rename_local_file() {
    let f = Fixture::new();
    let action_fs = f.create_action_file_system();
    let path = f.get_output_path("file");
    f.write_local_file(&path, "local-content");
    let new_path = f.get_output_path("file-new");

    action_fs.rename_to(&path, &new_path).unwrap();

    assert!(!action_fs.exists(&path, true));
    assert!(action_fs.exists(&new_path, true));
    assert!(!local_fs(&action_fs).exists(&path, true));
    assert!(local_fs(&action_fs).exists(&new_path, true));
}

#[test]
fn rename_to_move_symlink() {
    let f = Fixture::new();
    let action_fs = f.create_action_file_system();
    let old_link_path = f.get_output_path("oldLink");
    let new_link_path = f.get_output_path("newLink");
    let target_path = f.get_output_path("target");
    action_fs
        .get_path(&old_link_path)
        .create_symbolic_link_to_fragment(&target_path)
        .unwrap();
    f.write_local_file(&target_path, "content");

    action_fs.rename_to(&old_link_path, &new_link_path).unwrap();

    assert!(
        !action_fs
            .get_path(&old_link_path)
            .exists_with(Symlinks::Nofollow)
    );
    assert!(
        action_fs
            .get_path(&new_link_path)
            .exists_with(Symlinks::Nofollow)
    );
    assert_eq!(
        action_fs
            .get_path(&new_link_path)
            .read_symbolic_link()
            .unwrap(),
        target_path
    );
    assert!(action_fs.get_path(&target_path).exists());
}

#[test]
fn rename_to_follow_symlinks() {
    for from in FILESYSTEM_TEST_PARAMS {
        for to in FILESYSTEM_TEST_PARAMS {
            let f = Fixture::new();
            let action_fs = f.create_action_file_system();
            let from_fs = from.get_filesystem(&action_fs);

            let src_dir_link_path = f.get_output_path("srcDirLink");
            let src_dir_target_path = f.get_output_path("srcDirTarget");
            let naive_src_path = src_dir_link_path.get_child("oldFile");
            let canonical_src_path = src_dir_target_path.get_child("oldFile");

            let dst_dir_link_path = f.get_output_path("dstDirLink");
            let dst_dir_target_path = f.get_output_path("dstDirTarget");
            let naive_dst_path = dst_dir_link_path.get_child("newFile");
            let canonical_dst_path = dst_dir_target_path.get_child("newFile");

            action_fs
                .get_path(&src_dir_target_path)
                .create_directory_and_parents()
                .unwrap();
            action_fs
                .get_path(&dst_dir_target_path)
                .create_directory_and_parents()
                .unwrap();

            from_fs
                .create_symbolic_link(&src_dir_link_path, &src_dir_target_path)
                .unwrap();
            from_fs
                .create_symbolic_link(&dst_dir_link_path, &dst_dir_target_path)
                .unwrap();

            write_to(&f, &action_fs, to, &canonical_src_path, "content");

            action_fs
                .rename_to(&naive_src_path, &naive_dst_path)
                .unwrap();

            assert!(!action_fs.exists(&naive_src_path, true));
            assert!(!action_fs.exists(&canonical_src_path, true));
            assert!(action_fs.exists(&naive_dst_path, true));
            assert!(action_fs.exists(&canonical_dst_path, true));
        }
    }
}

#[test]
fn rename_to_invalidates_resolve_symbolic_links_cache() {
    let f = Fixture::new();
    let action_fs = f.create_action_file_system();
    let link_path = f.get_output_path("sym");
    let target_path = f.get_output_path("target");
    let renamed_path = f.get_output_path("renamed");

    action_fs
        .get_path(&link_path)
        .get_parent_directory()
        .unwrap()
        .create_directory_and_parents()
        .unwrap();
    action_fs
        .get_path(&link_path)
        .create_symbolic_link_to_fragment(&target_path)
        .unwrap();
    f.write_local_file(&target_path, "content");

    assert_eq!(
        action_fs
            .get_path(&link_path)
            .resolve_symbolic_links()
            .unwrap(),
        action_fs.get_path(&target_path)
    );

    action_fs.rename_to(&link_path, &renamed_path).unwrap();
    f.write_local_file(&link_path, "content");

    assert_eq!(
        action_fs
            .get_path(&link_path)
            .resolve_symbolic_links()
            .unwrap(),
        action_fs.get_path(&link_path)
    );
}

// ---------------------------------------------------------------------------------------------
// Blaze-only tests
// ---------------------------------------------------------------------------------------------

/// `injectRemoteFile` ignores a path outside the output base (`isOutput`).
#[test]
fn inject_remote_file_outside_the_output_base_is_ignored() {
    let f = Fixture::new();
    let action_fs = f.create_action_file_system();
    let path = f
        .dirs
        .exec_root
        .get_relative_str("src/file")
        .as_fragment()
        .clone();
    inject_remote_file(&action_fs, &path, "remote contents");
    assert!(!action_fs.exists(&path, true));
}

/// `ActionOutputMetadataStore.constructTreeArtifactValueFromFilesystem` on the action file
/// system (review N1 (i)): a cached tree whose files are remote-only and which contains a
/// relative symlink created locally (`lib/libfoo.so -> libfoo.so.1`). `readdir` merges the
/// remote file with the local link, and stat and `resolveSymbolicLinks` follow the link to the
/// remote file (`RemoteActionFileSystem.java:835-890`, `readSymbolicLinkInternal` falling back
/// to the local file system).
#[test]
fn tree_with_local_relative_symlink_to_remote_only_file() {
    let f = Fixture::new();
    let action_fs = f.create_action_file_system();
    let lib = f.get_output_path("tree/lib");
    let real = lib.get_child("libfoo.so.1");
    let link = lib.get_child("libfoo.so");
    let metadata = inject_remote_file(&action_fs, &real, "elf");
    local_fs(&action_fs)
        .create_directory_and_parents(&lib)
        .unwrap();
    local_fs(&action_fs)
        .create_symbolic_link(&link, &PathFragment::create("libfoo.so.1"))
        .unwrap();

    assert_readdir(
        &action_fs,
        &lib,
        false,
        &[
            Dirent::new("libfoo.so", DirentType::Symlink),
            Dirent::new("libfoo.so.1", DirentType::File),
        ],
    );
    assert_readdir(
        &action_fs,
        &lib,
        true,
        &[
            Dirent::new("libfoo.so", DirentType::File),
            Dirent::new("libfoo.so.1", DirentType::File),
        ],
    );
    assert_readdir(
        &action_fs,
        &f.get_output_path("tree"),
        false,
        &[Dirent::new("lib", DirentType::Directory)],
    );
    assert!(action_fs.stat(&link, false).unwrap().is_symbolic_link());
    let st = action_fs.stat(&link, true).unwrap();
    assert!(st.is_file());
    assert_eq!(st.get_size(), 3);
    assert_eq!(action_fs.resolve_symbolic_links(&link).unwrap(), real);
    assert_eq!(
        action_fs
            .stat_metadata(&link, true)
            .unwrap()
            .unwrap()
            .get_digest(),
        metadata.get_digest()
    );
    assert!(action_fs.is_remote(&link).unwrap());
}

/// `ctx.actions.symlink(output = <declared directory>, target_file = <tree>)` whose input tree
/// is remote-only (review N1 (ii)): the symlink, created through the action file system, lists
/// the input tree's children through the input tree directory cache
/// (`RemoteActionFileSystem.java:843-852`).
#[test]
fn symlink_to_remote_only_input_tree_lists_its_children() {
    let f = Fixture::new();
    let inputs = Arc::new(InputMap::default());
    let tree = f.create_remote_tree_artifact("in_tree", &[("a", "a"), ("sub/b", "bb")], &inputs);
    let output = f.create_tree_artifact_with_generating_action("out_tree");
    let action_fs = f.create_action_file_system_with(inputs);

    let link = output.get_path().as_fragment().clone();
    action_fs
        .create_symbolic_link(&link, tree.get_path().as_fragment())
        .unwrap();

    assert_readdir(
        &action_fs,
        &link,
        false,
        &[
            Dirent::new("a", DirentType::File),
            Dirent::new("sub", DirentType::Directory),
        ],
    );
    assert_readdir(
        &action_fs,
        &link.get_child("sub"),
        false,
        &[Dirent::new("b", DirentType::File)],
    );
    let st = action_fs
        .stat(&link.get_relative_str("sub/b"), true)
        .unwrap();
    assert!(st.is_file());
    assert_eq!(st.get_size(), 2);
    assert!(
        action_fs
            .stat(&link.get_child("sub"), true)
            .unwrap()
            .is_directory()
    );
}

/// A symlink loop is reported as `FileSymlinkLoopException` by `resolveSymbolicLinks`
/// (`PathCanonicalizer.java:128`).
#[test]
fn resolve_symbolic_links_loop() {
    let f = Fixture::new();
    let action_fs = f.create_action_file_system();
    let link = f.get_output_path("loop");
    action_fs.create_symbolic_link(&link, &link).unwrap();

    let e = action_fs.resolve_symbolic_links(&link).unwrap_err();
    assert!(FileSymlinkLoopException::is(&e), "{e}");
}

/// `getInputStream` records the lost inputs of a `BulkTransferException` and rethrows it;
/// `checkForLostInputs` merges the records in order (`RemoteActionFileSystem.java:380-393,
/// 914-923`, `LostInputsExecException.combine`). Failures that are not lost inputs are not
/// recorded.
#[test]
fn get_input_stream_records_lost_inputs() {
    let f = Fixture::new();
    let inputs = Arc::new(InputMap::default());
    let a = f.create_remote_artifact("a", "aaa", &inputs);
    let b = f.create_remote_artifact("b", "bbbb", &inputs);
    let action_fs = f.create_action_file_system_with(inputs);
    assert!(action_fs.check_for_lost_inputs().is_none());

    // Not a lost input: an unknown path (getLostArtifacts is empty).
    f.fetcher
        .fail_next_prefetch(io::Error::other(BulkTransferError {
            message: "boom".into(),
            causes: Some(vec![(
                "ccc/3".into(),
                Some(PathFragment::create("bazel-out/unknown")),
            )]),
        }));
    let e = action_fs
        .get_input_stream(a.get_path().as_fragment())
        .err()
        .unwrap();
    assert_eq!(e.to_string(), "boom");
    assert!(action_fs.check_for_lost_inputs().is_none());

    f.fetcher
        .fail_next_prefetch(io::Error::other(BulkTransferError {
            message: "lost a".into(),
            causes: Some(vec![("aaa/3".into(), Some(a.get_exec_path().clone()))]),
        }));
    let e = action_fs
        .get_input_stream(a.get_path().as_fragment())
        .err()
        .unwrap();
    assert_eq!(e.to_string(), "lost a");

    f.fetcher
        .fail_next_prefetch(io::Error::other(BulkTransferError {
            message: "lost b and a".into(),
            causes: Some(vec![
                ("bbbb/4".into(), Some(b.get_exec_path().clone())),
                ("aaa/3".into(), Some(a.get_exec_path().clone())),
            ]),
        }));
    assert!(
        action_fs
            .get_input_stream(b.get_path().as_fragment())
            .is_err()
    );

    let lost = action_fs.check_for_lost_inputs().unwrap();
    assert_eq!(
        lost.by_digest.keys().collect::<Vec<_>>(),
        vec!["aaa/3", "bbbb/4"]
    );
    assert_eq!(lost.by_digest["aaa/3"].len(), 1);
    assert_eq!(
        lost.by_digest["bbbb/4"]
            .iter()
            .map(|i| i.get_exec_path().clone())
            .collect::<Vec<_>>(),
        vec![b.get_exec_path().clone()]
    );
}

/// `getInputStream`: an interrupt while fetching is reported as "Received interrupt while
/// fetching file '<path>'" (`RemoteActionFileSystem.java:383-385`).
#[test]
fn get_input_stream_interrupted() {
    let f = Fixture::new();
    let inputs = Arc::new(InputMap::default());
    let a = f.create_remote_artifact("a", "aaa", &inputs);
    let action_fs = f.create_action_file_system_with(inputs);
    f.fetcher
        .fail_next_prefetch(io::Error::new(io::ErrorKind::Interrupted, "interrupted"));

    let e = action_fs
        .get_input_stream(a.get_path().as_fragment())
        .err()
        .unwrap();
    assert_eq!(
        e.to_string(),
        format!(
            "Received interrupt while fetching file '{}'",
            a.get_path().get_path_string()
        )
    );
    assert!(action_fs.check_for_lost_inputs().is_none());
}

/// `statFromMetadata` has no permissions in Java (-1): a caller asks `isExecutable()`, which is
/// the local file's bit for a local input and true for a remote-only one. The status carries
/// those answers (see `RemoteActionFileSystem::unknown_permissions`).
#[test]
fn stat_from_metadata_permissions_are_the_file_systems_answers() {
    let f = Fixture::new();
    let inputs = Arc::new(InputMap::default());
    let local = f.create_local_artifact("local-input", "local", &inputs);
    let remote = f.create_remote_artifact("remote-input", "remote", &inputs);
    let action_fs = f.create_action_file_system_with(inputs);

    let local_path = local.get_path().as_fragment().clone();
    let st = action_fs.stat(&local_path, true).unwrap();
    assert_eq!(
        st.get_permissions() & 0o100 != 0,
        action_fs.is_executable(&local_path).unwrap()
    );
    assert!(!action_fs.is_executable(&local_path).unwrap());

    let remote_path = remote.get_path().as_fragment().clone();
    let st = action_fs.stat(&remote_path, true).unwrap();
    assert_eq!(st.get_permissions(), 0o777);
}

// The in-process readers of an action (review R5). Each one reads an input that exists only
// remotely; reading the local path directly (bypassing the action file system) fails, so these
// tests fail for a reader that bypasses `getInputPath` / `convertPath`.

mod in_process_readers {
    use super::*;
    use blaze_actions::analysis::actions::symlink_action::SymlinkAction;
    use blaze_actions::analysis::actions::template_expansion_action::TemplateExpansionAction;

    fn execute(
        ctx: &analysis_actions_common::TestContext,
        action: &dyn Action,
    ) -> Result<blaze_exec::actions::ActionResult, blaze_exec::actions::ActionExecutionException>
    {
        let executable: &dyn ExecutableAction =
            as_executable(action).expect("action class has a registered ExecutableAction");
        executable.execute(ctx)
    }

    /// `SymlinkAction.maybeVerifyTargetIsExecutable` stats the primary input through the action
    /// file system: a generated tool that exists only remotely is an executable file. Without
    /// the action file system the local stat finds nothing.
    #[test]
    fn executable_symlink_to_remote_only_input() {
        let f = Fixture::new();
        let inputs = Arc::new(InputMap::default());
        let input_artifact = f.create_remote_artifact("tool", "#!/bin/sh\n", &inputs);
        let output_artifact = f.dirs.bin_artifact("tool-link");
        let action = SymlinkAction::to_executable(
            null_action_owner(),
            input_artifact.clone(),
            output_artifact.clone(),
            Some("Test symlink action".to_string()),
        );

        let mut ctx = f.dirs.context();
        ctx.metadata.metadata.lock().insert(
            input_artifact.get_exec_path_string().to_string(),
            remote_metadata("#!/bin/sh\n", 1),
        );
        // A reader that bypasses the action file system sees no file.
        let e = execute(&ctx, &action).unwrap_err();
        assert_eq!(
            e.get_message(),
            format!("'{}' is not a file", input_artifact.get_exec_path_string())
        );

        ctx.action_file_system = Some(f.create_action_file_system_with(inputs));
        execute(&ctx, &action).unwrap();
        assert!(output_artifact.get_path().is_symbolic_link());
        // Nothing was downloaded.
        assert!(f.fetcher.calls().is_empty());
        assert!(!input_artifact.get_path().exists());
    }

    /// `ArtifactTemplate.getContent(ctx.getPathResolver())` reads the template through the
    /// action file system, whose `getInputStream` downloads a remote template first.
    #[test]
    fn template_expansion_of_remote_only_template() {
        let f = Fixture::new();
        let inputs = Arc::new(InputMap::default());
        let template = f.create_remote_artifact("template.txt", "key=%key%\n", &inputs);
        f.fetcher.mock_prefetch_file(&template, "key=%key%\n");
        let output_artifact = f.dirs.bin_artifact("expanded.txt");
        let action = TemplateExpansionAction::for_template_artifact(
            null_action_owner(),
            template.clone(),
            output_artifact.clone(),
            vec![blaze_actions::analysis::actions::substitution::Substitution::of("%key%", "foo")],
            false,
        );

        let mut ctx = f.dirs.context();
        // A reader that bypasses the action file system finds no template.
        assert!(execute(&ctx, &action).is_err());
        assert!(f.fetcher.calls().is_empty());

        ctx.action_file_system = Some(f.create_action_file_system_with(inputs));
        execute(&ctx, &action).unwrap();
        assert_eq!(read(&output_artifact.get_path()), b"key=foo\n");
        assert_eq!(
            f.fetcher.calls(),
            vec![vec![template.get_exec_path().clone()]]
        );
    }

    /// `StandaloneTestStrategy`: `xmlOutputPath.exists()` is asked of the action file system
    /// (`getPathResolver().convertPath`), which sees a `test.xml` injected by a cached test.
    #[test]
    fn test_xml_injected_by_a_cached_test_exists() {
        let f = Fixture::new();
        let action_fs = f.create_action_file_system();
        let xml = f.dirs.bin_artifact("pkg/t/test.xml");
        inject_remote_file(&action_fs, xml.get_path().as_fragment(), "<testsuites/>");

        let mut ctx = f.dirs.context();
        assert!(!ctx.convert_path(&xml.get_path()).exists());
        ctx.action_file_system = Some(action_fs);
        assert!(ctx.convert_path(&xml.get_path()).exists());
        assert!(!xml.get_path().exists());
    }
}

/// `AbstractSpawnStrategy.SpawnExecutionContextImpl.prefetchInputs`: a `BulkTransferException`
/// of the prefetcher whose causes are all cache misses of known inputs is a
/// `LostInputsExecException`; any other is an `EnvironmentalExecException` "Failed to fetch
/// blobs because of a remote cache error." with `Spawn.REMOTE_CACHE_EVICTED` (review R6).
mod prefetch_inputs {
    use super::*;

    struct FailingPrefetcher(Mutex<Option<BulkTransferError>>);

    impl ActionInputPrefetcher for FailingPrefetcher {
        fn prefetch_files(
            &self,
            _action: &dyn ActionExecutionMetadata,
            _inputs: &[ActionInputRef],
            _metadata_provider: &dyn InputMetadataProvider,
        ) -> io::Result<()> {
            Err(io::Error::other(self.0.lock().take().unwrap()))
        }
    }

    fn prefetch(
        f: &Fixture,
        input: &Artifact,
        error: BulkTransferError,
    ) -> blaze_exec::actions::ExecException {
        let mut ctx = f.dirs.context();
        ctx.metadata.inputs.lock().insert(
            input.get_exec_path_string().to_string(),
            artifact_input(input),
        );
        let prefetcher: Arc<dyn ActionInputPrefetcher> =
            Arc::new(FailingPrefetcher(Mutex::new(Some(error))));
        ctx.contexts.insert(
            std::any::TypeId::of::<Arc<dyn ActionInputPrefetcher>>(),
            Arc::new(prefetcher),
        );
        let owner = dummy_action(&f.dirs);
        ctx.prefetch_files(&*owner, &[artifact_input(input)], &ctx.metadata)
            .unwrap_err()
    }

    #[test]
    fn cache_misses_of_known_inputs_are_lost_inputs() {
        let f = Fixture::new();
        let input = f.dirs.bin_artifact("_objs/a.o");
        let e = prefetch(
            &f,
            &input,
            BulkTransferError {
                message: "Missing digest: abc/3 for bazel-out/k8-fastbuild/bin/_objs/a.o".into(),
                causes: Some(vec![("abc/3".into(), Some(input.get_exec_path().clone()))]),
            },
        );
        let lost = e.get_lost_inputs().expect("a LostInputsExecException");
        assert_eq!(lost.by_digest.keys().collect::<Vec<_>>(), vec!["abc/3"]);
        assert_eq!(e.get_message(), "lost inputs with digests: abc/3");
    }

    #[test]
    fn other_bulk_transfer_failures_are_remote_cache_evicted() {
        let f = Fixture::new();
        let input = f.dirs.bin_artifact("_objs/a.o");
        for causes in [
            None,
            // An unknown input: getLostArtifacts is empty.
            Some(vec![(
                "abc/3".to_string(),
                Some(PathFragment::create("bazel-out/unknown")),
            )]),
            // stdout/stderr: no exec path.
            Some(vec![("abc/3".to_string(), None)]),
        ] {
            let e = prefetch(
                &f,
                &input,
                BulkTransferError {
                    message: "boom".into(),
                    causes,
                },
            );
            assert!(e.get_lost_inputs().is_none());
            assert_eq!(
                e.get_message(),
                "Failed to fetch blobs because of a remote cache error.: boom"
            );
            let detail = e.get_failure_detail("m").unwrap();
            assert_eq!(
                detail.category,
                Some(
                    blaze_protos::failure_details::failure_detail::Category::Spawn(
                        blaze_protos::failure_details::Spawn {
                            code: blaze_protos::failure_details::spawn::Code::RemoteCacheEvicted
                                as i32,
                            ..Default::default()
                        }
                    )
                )
            );
        }
    }
}
