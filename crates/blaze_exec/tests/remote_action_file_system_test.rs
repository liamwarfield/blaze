//! `RemoteActionFileSystemTest` and `RemoteActionFileSystemTestBase` (lib/remote), for the port
//! of `RemoteActionFileSystem` in `blaze_exec::exec::remote_action_file_system`, plus the
//! in-process readers that must go through it (review R5: `SymlinkAction`'s executable check,
//! template expansion, the test strategy's `test.xml` check).
//!
//! Java's fixture is an in-memory file system with the exec root `/exec` and the relative output
//! path `out`; here the local file system is the real one under a temporary directory
//! (`analysis_actions_common::TestDirs`: `<tmp>/execroot/_main`, output root
//! `bazel-out/k8-fastbuild/bin`), and the relative output path is `bazel-out`.

#![allow(clippy::result_large_err)]

mod analysis_actions_common;

use std::collections::HashMap;
use std::io::{self, Read};
use std::sync::Arc;

use analysis_actions_common::{TestDirs, null_action_owner, read, write};
use blaze_actions::{Action, Artifact, SpecialArtifactType};
use blaze_exec::actions::action_adapters::{ActionMetadataSnapshot, artifact_input};
use blaze_exec::actions::{
    ActionExecutionMetadata, ActionInputRef, ExecutableAction, FileArtifactValue,
    FilesetOutputTree, InputMetadataProvider, RunfilesArtifactValue, TreeArtifactValue,
    as_executable,
};
use blaze_exec::exec::action_execution_context::{ActionExecutionContext, ActionInputPrefetcher};
use blaze_exec::exec::remote_action_file_system::RemoteActionFileSystem;
use blaze_vfs::{Dirent, DirentType, FileSystem, NotASymlinkException, PathFragment};
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
/// contents of a prefetched input to its local path, and every call is recorded.
struct FakeInputFetcher {
    exec_root: blaze_vfs::Path,
    contents: Mutex<HashMap<PathFragment, String>>,
    calls: Mutex<Vec<Vec<PathFragment>>>,
}

impl FakeInputFetcher {
    fn new(exec_root: blaze_vfs::Path) -> Arc<FakeInputFetcher> {
        Arc::new(FakeInputFetcher {
            exec_root,
            contents: Mutex::new(HashMap::new()),
            calls: Mutex::new(Vec::new()),
        })
    }

    fn mock_prefetch_file(&self, artifact: &Artifact, contents: &str) {
        self.contents
            .lock()
            .insert(artifact.get_exec_path().clone(), contents.to_string());
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

struct Fixture {
    dirs: TestDirs,
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
        Fixture { dirs, fetcher }
    }

    // Java: RemoteActionFileSystemTest.createActionFileSystem
    fn create_action_file_system_with(&self, inputs: Arc<InputMap>) -> Arc<RemoteActionFileSystem> {
        let fs = RemoteActionFileSystem::new(
            self.dirs.exec_root.get_file_system().clone(),
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
        inputs.put(&a, remote_metadata(content));
        a
    }

    // Java: RemoteActionFileSystemTest.createLocalArtifact
    fn create_local_artifact(&self, path: &str, contents: &str, inputs: &InputMap) -> Artifact {
        let a = self.dirs.bin_artifact(path);
        write(&a.get_path(), contents);
        let p = a.get_path();
        let stat = p.stat_with(blaze_vfs::Symlinks::Follow).unwrap();
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
        // ActionsTestUtil.createTreeArtifactWithGeneratingAction
        let tree = self.dirs.special_artifact(path, SpecialArtifactType::TREE);
        tree.set_generating_action_key(blaze_actions::ActionLookupData::create(
            blaze_actions::ActionLookupKeyRef::new(analysis_actions_common::NullArtifactOwner),
            0,
        ));
        tree.get_path().create_directory_and_parents().unwrap();
        let mut children = Vec::new();
        for (name, content) in content_map {
            let child = tree.get_path().get_relative_str(name);
            write(&child, content);
            let stat = child.stat_with(blaze_vfs::Symlinks::Follow).unwrap();
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

fn remote_metadata(content: &str) -> FileArtifactValue {
    FileArtifactValue::create_for_remote_file_with_materialization_data(
        get_digest(content),
        content.len() as i64,
        /* location_index= */ 1,
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
    remote_metadata(content)
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

fn assert_readdir_throws(action_fs: &RemoteActionFileSystem, dir_path: &PathFragment) {
    assert!(action_fs.readdir(dir_path, true).is_err());
    assert!(action_fs.get_directory_entries(dir_path).is_err());
}

#[test]
fn get_input_stream_for_local_source_file() {
    let f = Fixture::new();
    let artifact = f.dirs.source_artifact("src");
    let action_fs = f.create_action_file_system();
    f.write_local_file(artifact.get_path().as_fragment(), "local contents");

    let action_fs_path = action_fs.get_path(artifact.get_path().as_fragment());
    let contents = read_content(&action_fs, action_fs_path.as_fragment());

    assert!(Arc::ptr_eq(
        action_fs_path.get_file_system(),
        &action_fs.as_file_system()
    ));
    assert_eq!(contents, "local contents");
}

#[test]
fn test_get_input_stream_for_local_output_file() {
    let f = Fixture::new();
    let artifact = f.dirs.bin_artifact("src");
    let action_fs = f.create_action_file_system();
    f.write_local_file(artifact.get_path().as_fragment(), "local contents");

    let contents = read_content(&action_fs, artifact.get_path().as_fragment());

    assert_eq!(contents, "local contents");
    assert!(f.fetcher.calls().is_empty());
}

#[test]
fn test_get_input_stream_for_remote_output_file() {
    let f = Fixture::new();
    let inputs = Arc::new(InputMap::default());
    let artifact = f.create_remote_artifact("remote-file", "remote contents", &inputs);
    let action_fs = f.create_action_file_system_with(inputs);
    f.fetcher.mock_prefetch_file(&artifact, "remote contents");

    let contents = read_content(&action_fs, artifact.get_path().as_fragment());

    assert_eq!(contents, "remote contents");
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
    // FileStatusWithDigest.getDigest()
    assert_eq!(
        action_fs
            .stat_metadata(&path, true)
            .unwrap()
            .unwrap()
            .get_digest(),
        metadata.get_digest()
    );
}

#[test]
fn stat_and_exists_from_input_artifact_data_tree_sub_dir() {
    let f = Fixture::new();
    let inputs = Arc::new(InputMap::default());
    let tree = f.create_local_tree_artifact("tree", &[("subdir/file", "")], &inputs);
    let path = tree.get_path().get_child("subdir").as_fragment().clone();
    // The tree's files exist only in the input metadata.
    tree.get_path().delete_tree().unwrap();
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
    let metadata = inject_remote_file(&action_fs, &path, "remote contents");

    assert!(action_fs.exists(&path, true));
    let st = action_fs.stat(&path, true).unwrap();
    assert!(st.is_file());
    assert_eq!(
        action_fs
            .stat_metadata(&path, true)
            .unwrap()
            .unwrap()
            .get_digest(),
        metadata.get_digest()
    );
}

#[test]
fn stat_and_exists_from_local_filesystem() {
    let f = Fixture::new();
    let action_fs = f.create_action_file_system();
    let artifact = f.dirs.bin_artifact("out");
    let path = artifact.get_path().as_fragment().clone();
    f.write_local_file(&path, "local contents");

    assert!(action_fs.exists(&path, true));
    let st = action_fs.stat(&path, true).unwrap();
    assert!(st.is_file());
    assert_eq!(st.get_size(), "local contents".len() as i64);
}

/// `statAndExists_followSymlinks` with `from = LOCAL` and both `to` values.
#[test]
fn stat_and_exists_follow_symlinks() {
    for to_remote in [false, true] {
        let f = Fixture::new();
        let action_fs = f.create_action_file_system();
        let link_path = f.get_output_path("sym");
        let target_path = f.get_output_path("target");
        action_fs
            .get_local_file_system()
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
        assert_eq!(
            action_fs.stat(&link_path, true).unwrap_err().kind(),
            io::ErrorKind::NotFound
        );

        if to_remote {
            inject_remote_file(&action_fs, &target_path, "content");
        } else {
            f.write_local_file(&target_path, "content");
        }

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

#[test]
fn stat_and_exists_not_found() {
    let f = Fixture::new();
    let action_fs = f.create_action_file_system();
    let path = f.get_output_path("does_not_exist");

    assert!(!action_fs.exists(&path, true));
    assert!(action_fs.stat_if_found(&path, true).unwrap().is_none());
    assert_eq!(
        action_fs.stat(&path, true).unwrap_err().kind(),
        io::ErrorKind::NotFound
    );
}

#[test]
fn stat_and_exists_dangling_symlink_not_found() {
    let f = Fixture::new();
    let action_fs = f.create_action_file_system();
    let path = f.get_output_path("sym");
    action_fs
        .create_symbolic_link(&path, &PathFragment::create("/does_not_exist"))
        .unwrap();

    assert!(!action_fs.exists(&path, true));
    assert!(action_fs.stat_if_found(&path, true).unwrap().is_none());
    assert_eq!(
        action_fs.stat(&path, true).unwrap_err().kind(),
        io::ErrorKind::NotFound
    );
}

#[test]
fn get_digest_from_input_artifact_data_for_remote_artifact() {
    let f = Fixture::new();
    let inputs = Arc::new(InputMap::default());
    let artifact = f.create_remote_artifact("file", "remote contents", &inputs);
    let action_fs = f.create_action_file_system_with(inputs);
    let path = artifact.get_path().as_fragment().clone();

    assert_eq!(
        action_fs.get_fast_digest(&path).unwrap(),
        Some(get_digest("remote contents"))
    );
    assert_eq!(
        action_fs.get_digest(&path).unwrap(),
        get_digest("remote contents")
    );
    assert!(f.fetcher.calls().is_empty());
}

#[test]
fn get_digest_from_remote_output_tree() {
    let f = Fixture::new();
    let action_fs = f.create_action_file_system();
    let path = f.get_output_path("file");
    inject_remote_file(&action_fs, &path, "remote contents");

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
    let path = f.get_output_path("file");
    f.write_local_file(&path, "local contents");

    assert_eq!(
        action_fs.get_digest(&path).unwrap(),
        get_digest("local contents")
    );
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
    assert_readdir_throws(&action_fs, &f.get_output_path("dir/out1"));
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
    assert_readdir_throws(&action_fs, &f.get_output_path("dir/out1"));
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
    assert_readdir_throws(&action_fs, &f.get_output_path("tree/dir/subdir/subfile"));
}

#[test]
fn readdir_from_remote_output_tree_and_local_filesystem() {
    let f = Fixture::new();
    let action_fs = f.create_action_file_system();
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

/// `permissions_followSymlinks` with `from = LOCAL` and both `to` values.
#[test]
fn permissions_follow_symlinks() {
    for to_remote in [false, true] {
        let f = Fixture::new();
        let action_fs = f.create_action_file_system();
        let link_path = f.get_output_path("sym");
        let target_path = f.get_output_path("target");
        action_fs
            .get_local_file_system()
            .create_symbolic_link(&link_path, &target_path)
            .unwrap();

        assert_eq!(
            action_fs.chmod(&link_path, 0o777).unwrap_err().kind(),
            io::ErrorKind::NotFound
        );

        if to_remote {
            inject_remote_file(&action_fs, &target_path, "content");
        } else {
            f.write_local_file(&target_path, "content");
        }

        // For a remote file, permissions are always 0777.
        let is_remote = to_remote;

        assert!(action_fs.is_readable(&link_path).unwrap());
        assert!(action_fs.is_writable(&link_path).unwrap());
        assert_eq!(action_fs.is_executable(&link_path).unwrap(), is_remote);

        action_fs.chmod(&link_path, 0o111).unwrap();
        assert_eq!(action_fs.is_readable(&link_path).unwrap(), is_remote);
        assert_eq!(action_fs.is_writable(&link_path).unwrap(), is_remote);
        assert!(action_fs.is_executable(&link_path).unwrap());

        action_fs.set_readable(&link_path, true).unwrap();
        action_fs.set_writable(&link_path, true).unwrap();
        action_fs.set_executable(&link_path, false).unwrap();
        assert!(action_fs.is_readable(&link_path).unwrap());
        assert!(action_fs.is_writable(&link_path).unwrap());
        assert_eq!(action_fs.is_executable(&link_path).unwrap(), is_remote);
    }
}

#[test]
fn read_symbolic_link_from_local_filesystem() {
    let f = Fixture::new();
    let action_fs = f.create_action_file_system();
    let file_path = f.get_output_path("file");
    let link_path = f.get_output_path("sym");
    let target_path = PathFragment::create("/some/path");
    action_fs
        .get_local_file_system()
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
fn read_symbolic_link_from_input_artifact_data_regular_file() {
    let f = Fixture::new();
    let inputs = Arc::new(InputMap::default());
    let artifact = f.create_remote_artifact("file", "content", &inputs);
    let action_fs = f.create_action_file_system_with(inputs);

    assert!(NotASymlinkException::is(
        &action_fs
            .read_symbolic_link(artifact.get_path().as_fragment())
            .unwrap_err()
    ));
}

#[test]
fn read_symbolic_link_not_found() {
    let f = Fixture::new();
    let action_fs = f.create_action_file_system();
    let link_path = f.get_output_path("sym");
    assert_eq!(
        action_fs.read_symbolic_link(&link_path).unwrap_err().kind(),
        io::ErrorKind::NotFound
    );
}

// RemoteActionFileSystemTestBase

#[test]
fn exists_file_does_not_exist_returns_false() {
    let f = Fixture::new();
    let action_fs = f.create_action_file_system();
    assert!(!action_fs.exists(&f.get_output_path("file"), true));
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
    assert!(!action_fs.delete(&f.get_output_path("file")).unwrap());
}

#[test]
fn delete_local_file_succeeds() {
    let f = Fixture::new();
    let action_fs = f.create_action_file_system();
    let path = f.get_output_path("file");
    f.write_local_file(&path, "local contents");
    assert!(action_fs.get_local_file_system().exists(&path, true));

    assert!(action_fs.delete(&path).unwrap());
    assert!(!action_fs.get_local_file_system().exists(&path, true));
}

#[test]
fn delete_remote_file_succeeds() {
    let f = Fixture::new();
    let action_fs = f.create_action_file_system();
    let path = f.get_output_path("file");
    inject_remote_file(&action_fs, &path, "remote contents");
    assert!(
        action_fs
            .get_remote_output_tree()
            .get_metadata(&path)
            .is_some()
    );

    assert!(action_fs.delete(&path).unwrap());
    assert!(!action_fs.exists(&path, true));
    assert!(
        action_fs
            .get_remote_output_tree()
            .get_metadata(&path)
            .is_none()
    );
}

#[test]
fn delete_local_and_remote_file_succeeds() {
    let f = Fixture::new();
    let action_fs = f.create_action_file_system();
    let path = f.get_output_path("file");
    f.write_local_file(&path, "local contents");
    inject_remote_file(&action_fs, &path, "remote contents");

    assert!(action_fs.delete(&path).unwrap());
    assert!(!action_fs.exists(&path, true));
    assert!(!action_fs.get_local_file_system().exists(&path, true));
    assert!(
        action_fs
            .get_remote_output_tree()
            .get_metadata(&path)
            .is_none()
    );
}

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

#[test]
#[ignore = "needs symbolic links in the remote output tree (PORT: Blaze keeps them on the local file system only)"]
fn read_symbolic_link_from_remote_filesystem() {}

#[test]
#[ignore = "not ported yet: the remaining RemoteActionFileSystemTest cases (delete/rename/createSymbolicLink/resolveSymbolicLinks/setLastModifiedTime and the REMOTE `from` parameters)"]
fn remaining_remote_action_file_system_tests() {}

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
            remote_metadata("#!/bin/sh\n"),
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
    use blaze_exec::exec::action_execution_context::BulkTransferError;

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
