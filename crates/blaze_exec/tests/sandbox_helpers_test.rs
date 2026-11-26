//! Port of `SandboxHelpersTest.java`.
//!
//! PORT: Java uses an `InMemoryFileSystem` rooted at `/`; these tests use the real file system
//! (a `UnixFileSystem` wrapper that can forbid renames) under a fresh temporary directory.

use std::collections::{BTreeMap, HashSet};
use std::io::{self, Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Barrier, Condvar, Mutex};

use blaze_collect::{NestedSetBuilder, Order};
use blaze_exec::actions::{
    ActionInput, ActionInputRef, BasicActionInput, BasicVirtualActionInput, FakeOwner,
    HierarchicalPath, InputMapping, ResourceSet, SimpleSpawn, VirtualActionInput,
};
use blaze_exec::exec::bin_tools::PathActionInput;
use blaze_exec::sandbox::SynchronousTreeDeleter;
use blaze_exec::sandbox::sandbox_helpers::{self, SandboxContents, SandboxInputs, SandboxOutputs};
use blaze_vfs::{
    DigestHashFunction, Dirent, DirentType, FileStatus, FileSystem, FileSystemExt, FileSystemRef,
    Path, PathFragment, Symlinks, UnixFileSystem,
};
use indexmap::{IndexMap, IndexSet};

/// `CustomInMemoryFileSystem`: the real file system, with `renameTo` failing on request and an
/// optional hook before `setExecutable`.
struct HookFs {
    inner: UnixFileSystem,
    forbid_rename_to: AtomicBool,
    #[allow(clippy::type_complexity)]
    set_executable_hook: Mutex<Option<Arc<dyn Fn() + Send + Sync>>>,
}

impl std::fmt::Debug for HookFs {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("HookFs")
    }
}

impl HookFs {
    fn new() -> HookFs {
        HookFs {
            inner: UnixFileSystem::new(DigestHashFunction::Sha256),
            forbid_rename_to: AtomicBool::new(false),
            set_executable_hook: Mutex::new(None),
        }
    }
}

impl FileSystem for HookFs {
    fn get_digest_function(&self) -> DigestHashFunction {
        self.inner.get_digest_function()
    }
    fn supports_modifications(&self, path: &PathFragment) -> bool {
        self.inner.supports_modifications(path)
    }
    fn supports_symbolic_links_natively(&self, path: &PathFragment) -> bool {
        self.inner.supports_symbolic_links_natively(path)
    }
    fn supports_hard_links_natively(&self, path: &PathFragment) -> bool {
        self.inner.supports_hard_links_natively(path)
    }
    fn may_be_case_or_normalization_insensitive(&self) -> bool {
        self.inner.may_be_case_or_normalization_insensitive()
    }
    fn create_directory(&self, path: &PathFragment) -> io::Result<bool> {
        self.inner.create_directory(path)
    }
    fn create_directory_and_parents(&self, path: &PathFragment) -> io::Result<()> {
        self.inner.create_directory_and_parents(path)
    }
    fn get_file_size(&self, path: &PathFragment, follow_symlinks: bool) -> io::Result<i64> {
        self.inner.get_file_size(path, follow_symlinks)
    }
    fn delete(&self, path: &PathFragment) -> io::Result<bool> {
        self.inner.delete(path)
    }
    fn get_last_modified_time(
        &self,
        path: &PathFragment,
        follow_symlinks: bool,
    ) -> io::Result<i64> {
        self.inner.get_last_modified_time(path, follow_symlinks)
    }
    fn set_last_modified_time(&self, path: &PathFragment, new_time: i64) -> io::Result<()> {
        self.inner.set_last_modified_time(path, new_time)
    }
    fn resolve_one_link(&self, path: &PathFragment) -> io::Result<Option<PathFragment>> {
        self.inner.resolve_one_link(path)
    }
    fn stat(&self, path: &PathFragment, follow_symlinks: bool) -> io::Result<FileStatus> {
        self.inner.stat(path, follow_symlinks)
    }
    fn stat_if_found(
        &self,
        path: &PathFragment,
        follow_symlinks: bool,
    ) -> io::Result<Option<FileStatus>> {
        self.inner.stat_if_found(path, follow_symlinks)
    }
    fn create_symbolic_link(
        &self,
        link_path: &PathFragment,
        target_fragment: &PathFragment,
    ) -> io::Result<()> {
        self.inner.create_symbolic_link(link_path, target_fragment)
    }
    fn read_symbolic_link(&self, path: &PathFragment) -> io::Result<PathFragment> {
        self.inner.read_symbolic_link(path)
    }
    fn exists(&self, path: &PathFragment, follow_symlinks: bool) -> bool {
        self.inner.exists(path, follow_symlinks)
    }
    fn get_directory_entries(&self, path: &PathFragment) -> io::Result<Vec<String>> {
        self.inner.get_directory_entries(path)
    }
    fn readdir(&self, path: &PathFragment, follow_symlinks: bool) -> io::Result<Vec<Dirent>> {
        self.inner.readdir(path, follow_symlinks)
    }
    fn is_readable(&self, path: &PathFragment) -> io::Result<bool> {
        self.inner.is_readable(path)
    }
    fn set_readable(&self, path: &PathFragment, readable: bool) -> io::Result<()> {
        self.inner.set_readable(path, readable)
    }
    fn is_writable(&self, path: &PathFragment) -> io::Result<bool> {
        self.inner.is_writable(path)
    }
    fn set_writable(&self, path: &PathFragment, writable: bool) -> io::Result<()> {
        self.inner.set_writable(path, writable)
    }
    fn is_executable(&self, path: &PathFragment) -> io::Result<bool> {
        self.inner.is_executable(path)
    }
    fn set_executable(&self, path: &PathFragment, executable: bool) -> io::Result<()> {
        let hook = self.set_executable_hook.lock().unwrap().clone();
        if let Some(hook) = hook {
            hook();
        }
        self.inner.set_executable(path, executable)
    }
    fn chmod(&self, path: &PathFragment, mode: u32) -> io::Result<()> {
        self.inner.chmod(path, mode)
    }
    fn get_input_stream(&self, path: &PathFragment) -> io::Result<Box<dyn Read + Send>> {
        self.inner.get_input_stream(path)
    }
    fn get_output_stream(
        &self,
        path: &PathFragment,
        append: bool,
        internal: bool,
    ) -> io::Result<Box<dyn Write + Send>> {
        self.inner.get_output_stream(path, append, internal)
    }
    fn rename_to(&self, source_path: &PathFragment, target_path: &PathFragment) -> io::Result<()> {
        if self.forbid_rename_to.load(Ordering::SeqCst) {
            return Err(io::Error::other("error injected by test"));
        }
        self.inner.rename_to(source_path, target_path)
    }
    fn create_fs_dependent_hard_link(
        &self,
        link_path: &PathFragment,
        original_path: &PathFragment,
    ) -> io::Result<()> {
        self.inner
            .create_fs_dependent_hard_link(link_path, original_path)
    }
}

struct Fixture {
    _tmp: tempfile::TempDir,
    hook_fs: Arc<HookFs>,
    root: Path,
    exec_root: Path,
    sandbox_root: Path,
    tree_deleter: SynchronousTreeDeleter,
}

impl Fixture {
    // Java: SandboxHelpersTest.setUp
    fn new() -> Fixture {
        let tmp = tempfile::tempdir().unwrap();
        let hook_fs = Arc::new(HookFs::new());
        let fs: FileSystemRef = hook_fs.clone();
        let root = fs.get_path(tmp.path().to_str().unwrap());
        let exec_root = root.get_relative_str("execroot");
        exec_root.create_directory_and_parents().unwrap();
        let sandbox_root = root.get_relative_str("sandbox");
        sandbox_root.create_directory_and_parents().unwrap();
        Fixture {
            _tmp: tmp,
            hook_fs,
            root,
            exec_root,
            sandbox_root,
            tree_deleter: SynchronousTreeDeleter,
        }
    }

    fn forbid_rename_to(&self) {
        self.hook_fs.forbid_rename_to.store(true, Ordering::SeqCst);
    }

    /// `scratch.file(path, lines...)`: relative paths resolve against the fixture root.
    fn file(&self, path: &str, lines: &[&str]) -> Path {
        let p = self.root.get_relative_str(path.trim_start_matches('/'));
        p.get_parent_directory()
            .unwrap()
            .create_directory_and_parents()
            .unwrap();
        let mut content = String::new();
        for l in lines {
            content.push_str(l);
            content.push('\n');
        }
        write_content(&p, &content);
        p
    }
}

fn write_content(p: &Path, content: &str) {
    let mut out = p.get_output_stream().unwrap();
    out.write_all(content.as_bytes()).unwrap();
    out.flush().unwrap();
}

fn read_content(p: &Path) -> String {
    let mut s = String::new();
    p.get_input_stream()
        .unwrap()
        .read_to_string(&mut s)
        .unwrap();
    s
}

fn read_lines(p: &Path) -> Vec<String> {
    read_content(p).lines().map(str::to_string).collect()
}

/// `containsExactly` on sets: compares as unordered sets.
fn hs<'a>(items: impl IntoIterator<Item = &'a PathFragment>) -> HashSet<PathFragment> {
    items.into_iter().cloned().collect()
}

fn input_map(inputs: &[ActionInputRef]) -> InputMapping {
    let mut m: InputMapping = BTreeMap::new();
    for i in inputs {
        m.insert(HierarchicalPath(i.get_exec_path().clone()), i.clone());
    }
    m
}

/// `new ParamFileActionInput(path, ["-a", "-b"], UNQUOTED)`.
// PORT: `ParamFileActionInput` lives in blaze_actions; an UNQUOTED param file writes each argument
// followed by a newline, which a `BasicVirtualActionInput` with that content reproduces.
fn param_file(path: &str, args: &[&str]) -> ActionInputRef {
    let mut content = Vec::new();
    for a in args {
        content.extend_from_slice(a.as_bytes());
        content.push(b'\n');
    }
    ActionInputRef::new(BasicVirtualActionInput::new(
        PathFragment::create(path),
        content,
    ))
}

/// A test artifact (`ActionsTestUtil.createArtifact` / `createUnresolvedSymlinkArtifact`).
#[derive(Debug)]
struct TestInput {
    exec_path: PathFragment,
    symlink: bool,
}

impl ActionInput for TestInput {
    fn get_exec_path(&self) -> &PathFragment {
        &self.exec_path
    }
    fn is_directory(&self) -> bool {
        false
    }
    fn is_symlink(&self) -> bool {
        self.symlink
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

fn artifact(exec_path: &str) -> ActionInputRef {
    ActionInputRef::new(TestInput {
        exec_path: PathFragment::create(exec_path),
        symlink: false,
    })
}

fn symlink_artifact(exec_path: &str) -> ActionInputRef {
    ActionInputRef::new(TestInput {
        exec_path: PathFragment::create(exec_path),
        symlink: true,
    })
}

/// `new SpawnBuilder().withOutputs(outputs...).build()`.
fn spawn_with_outputs(outputs: &[&str]) -> SimpleSpawn {
    SimpleSpawn::new_simple(
        Arc::new(FakeOwner::new("Mnemonic", Some("progress message"), None)),
        Vec::new(),
        IndexMap::new(),
        IndexMap::new(),
        NestedSetBuilder::empty_set(Order::STABLE_ORDER),
        outputs
            .iter()
            .map(|o| ActionInputRef::new(BasicActionInput::from_path(o)))
            .collect(),
        Arc::new(ResourceSet::zero()),
    )
}

#[test]
fn process_input_files_materializes_param_file() {
    let f = Fixture::new();
    let param_file = param_file("paramFile", &["-a", "-b"]);

    let inputs =
        sandbox_helpers::process_input_files(&input_map(&[param_file]), &f.exec_root).unwrap();

    let expected: IndexMap<PathFragment, Option<Path>> = [(
        PathFragment::create("paramFile"),
        Some(f.exec_root.get_child("paramFile")),
    )]
    .into_iter()
    .collect();
    assert_eq!(inputs.get_files(), &expected);
    assert!(inputs.get_symlinks().is_empty());
    assert_eq!(
        read_lines(&f.exec_root.get_child("paramFile")),
        vec!["-a", "-b"]
    );
    assert!(f.exec_root.get_child("paramFile").is_executable().unwrap());
}

#[test]
fn process_input_files_materializes_bin_tools_file() {
    let f = Fixture::new();
    let tool = ActionInputRef::new(PathActionInput::new(
        f.file("tool", &["#!/bin/bash", "echo hello"]),
        PathFragment::create("_bin/say_hello"),
    ));

    let inputs = sandbox_helpers::process_input_files(&input_map(&[tool]), &f.exec_root).unwrap();

    let expected: IndexMap<PathFragment, Option<Path>> = [(
        PathFragment::create("_bin/say_hello"),
        Some(f.exec_root.get_relative_str("_bin/say_hello")),
    )]
    .into_iter()
    .collect();
    assert_eq!(inputs.get_files(), &expected);
    assert!(inputs.get_symlinks().is_empty());
    assert_eq!(
        read_lines(&f.exec_root.get_relative_str("_bin/say_hello")),
        vec!["#!/bin/bash", "echo hello"]
    );
    assert!(
        f.exec_root
            .get_relative_str("_bin/say_hello")
            .is_executable()
            .unwrap()
    );
}

#[test]
fn process_input_files_omits_inputs_nested_under_directory_input() {
    let f = Fixture::new();
    // A directory artifact and inputs nested under it.
    let directory = artifact("out/dir");
    let nested_file = artifact("out/dir/sub/nested");
    let nested_symlink = symlink_artifact("out/dir/link");
    f.exec_root
        .get_relative_str("out/dir")
        .create_directory_and_parents()
        .unwrap();
    f.exec_root
        .get_relative_str("out/dir/link")
        .create_symbolic_link_to_fragment(&PathFragment::create("nested_target"))
        .unwrap();
    let other = artifact("out/other");
    // An input symlink and an input nested under it, which is retained.
    let symlink = symlink_artifact("out/link");
    let file_under_symlink = artifact("out/link/file");
    f.exec_root
        .get_relative_str("out/link")
        .create_symbolic_link_to_fragment(&PathFragment::create("target"))
        .unwrap();

    let inputs = sandbox_helpers::process_input_files(
        &input_map(&[
            nested_file,
            directory.clone(),
            other.clone(),
            file_under_symlink.clone(),
            nested_symlink,
            symlink.clone(),
        ]),
        &f.exec_root,
    )
    .unwrap();

    let mut files: Vec<(PathFragment, Option<Path>)> = inputs
        .get_files()
        .iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    files.sort_by(|a, b| a.0.cmp(&b.0));
    let mut expected = vec![
        (
            directory.get_exec_path().clone(),
            Some(f.exec_root.get_relative(directory.get_exec_path())),
        ),
        (
            other.get_exec_path().clone(),
            Some(f.exec_root.get_relative(other.get_exec_path())),
        ),
        (
            file_under_symlink.get_exec_path().clone(),
            Some(f.exec_root.get_relative(file_under_symlink.get_exec_path())),
        ),
    ];
    expected.sort_by(|a, b| a.0.cmp(&b.0));
    assert_eq!(files, expected);
    let symlinks: Vec<(PathFragment, PathFragment)> = inputs
        .get_symlinks()
        .iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    assert_eq!(
        symlinks,
        vec![(
            symlink.get_exec_path().clone(),
            PathFragment::create("target")
        )]
    );
}

/// A counting semaphore (`java.util.concurrent.Semaphore`).
struct Semaphore {
    permits: Mutex<i32>,
    cv: Condvar,
}

impl Semaphore {
    fn new(permits: i32) -> Semaphore {
        Semaphore {
            permits: Mutex::new(permits),
            cv: Condvar::new(),
        }
    }
    fn acquire(&self) {
        let mut p = self.permits.lock().unwrap();
        while *p <= 0 {
            p = self.cv.wait(p).unwrap();
        }
        *p -= 1;
    }
    fn release(&self) {
        *self.permits.lock().unwrap() += 1;
        self.cv.notify_one();
    }
}

/// Test simulating a scenario when 2 parallel writes of the same virtual input both complete
/// write of the temp file and then proceed with post-processing steps one-by-one.
#[test]
fn sandbox_input_materialize_virtual_input_parallel_writes_for_same_input_writes_correct_file() {
    let f = Fixture::new();
    let input = ActionInputRef::new(BasicVirtualActionInput::new(
        PathFragment::create("file"),
        b"hello".to_vec(),
    ));
    let both_wrote_temp_file = Arc::new(Barrier::new(2));
    let finish_processing_semaphore = Arc::new(Semaphore::new(1));
    {
        let barrier = both_wrote_temp_file.clone();
        let semaphore = finish_processing_semaphore.clone();
        *f.hook_fs.set_executable_hook.lock().unwrap() = Some(Arc::new(move || {
            barrier.wait();
            semaphore.acquire();
        }));
    }
    let custom_exec_root = f.root.get_relative_str("custom_execroot");
    custom_exec_root.create_directory_and_parents().unwrap();

    let thread = {
        let input = input.clone();
        let exec_root = custom_exec_root.clone();
        let semaphore = finish_processing_semaphore.clone();
        std::thread::spawn(move || {
            sandbox_helpers::process_input_files(&input_map(&[input]), &exec_root).unwrap();
            semaphore.release();
        })
    };
    sandbox_helpers::process_input_files(&input_map(&[input]), &custom_exec_root).unwrap();
    finish_processing_semaphore.release();
    thread.join().unwrap();
    *f.hook_fs.set_executable_hook.lock().unwrap() = None;

    assert_eq!(
        custom_exec_root.readdir(Symlinks::Nofollow).unwrap(),
        vec![Dirent::new("file", DirentType::File)]
    );
    let output_file = custom_exec_root.get_child("file");
    assert_eq!(read_lines(&output_file), vec!["hello"]);
    assert!(output_file.is_executable().unwrap());
}

#[test]
fn atomically_write_virtual_input_writes_param_file() {
    let f = Fixture::new();
    let param_file = param_file("paramFile", &["-a", "-b"]);
    let outputs = f.root.get_relative_str("outputs");

    param_file
        .as_virtual_action_input()
        .unwrap()
        .atomically_write_relative_to(&outputs)
        .unwrap();

    assert_eq!(
        outputs.readdir(Symlinks::Nofollow).unwrap(),
        vec![Dirent::new("paramFile", DirentType::File)]
    );
    let output_file = outputs.get_relative_str("paramFile");
    assert_eq!(read_lines(&output_file), vec!["-a", "-b"]);
    assert!(output_file.is_executable().unwrap());
}

#[test]
fn atomically_write_virtual_input_writes_bin_tools_file() {
    let f = Fixture::new();
    let tool = PathActionInput::new(
        f.file("tool", &["tool_code"]),
        PathFragment::create("tools/tool"),
    );
    let outputs = f.root.get_relative_str("outputs");

    tool.atomically_write_relative_to(&outputs).unwrap();

    assert_eq!(
        outputs.readdir(Symlinks::Nofollow).unwrap(),
        vec![Dirent::new("tools", DirentType::Directory)]
    );
    let output_file = outputs.get_relative_str("tools/tool");
    assert_eq!(read_lines(&output_file), vec!["tool_code"]);
    assert!(output_file.is_executable().unwrap());
}

#[test]
fn clean_existing_updates_dirs() {
    let f = Fixture::new();
    let input_txt = f.root.get_relative_str("hello.txt");
    let root_dir = f.exec_root.get_parent_directory().unwrap();
    let input1 = PathFragment::create("existing/directory/with/input1.txt");
    let input2 = PathFragment::create("partial/directory/input2.txt");
    let input3 = PathFragment::create("new/directory/input3.txt");
    let inputs = SandboxInputs::new(
        [
            (input1.clone(), Some(input_txt.clone())),
            (input2.clone(), Some(input_txt.clone())),
            (input3.clone(), Some(input_txt.clone())),
        ]
        .into_iter()
        .collect(),
        IndexMap::new(),
        IndexMap::new(),
    );
    let mut inputs_to_create: IndexSet<PathFragment> = IndexSet::new();
    let mut dirs_to_create: IndexSet<PathFragment> = IndexSet::new();
    let input_files: Vec<PathFragment> = inputs
        .get_files()
        .keys()
        .chain(inputs.get_symlinks().keys())
        .cloned()
        .collect();
    sandbox_helpers::populate_inputs_and_dirs_to_create(
        &IndexSet::<PathFragment>::new(),
        &mut inputs_to_create,
        &mut dirs_to_create,
        &input_files,
        &SandboxOutputs::create_from_sets([PathFragment::create("out/dir/output.txt")], []),
    );

    let input_dir1 = input1.get_parent_directory().unwrap();
    let input_dir2 = input2.get_parent_directory().unwrap();
    let input_dir3 = input3.get_parent_directory().unwrap();
    let output_dir = PathFragment::create("out/dir");
    assert_eq!(
        hs(dirs_to_create.iter()),
        hs([&input_dir1, &input_dir2, &input_dir3, &output_dir])
    );
    assert_eq!(hs(inputs_to_create.iter()), hs([&input1, &input2, &input3]));

    // inputdir1 exists fully
    f.exec_root
        .get_relative(&input_dir1)
        .create_directory_and_parents()
        .unwrap();
    // inputdir2 exists partially, should be kept nonetheless.
    f.exec_root
        .get_relative(&input_dir2)
        .get_parent_directory()
        .unwrap()
        .get_relative_str("doomedSubdir")
        .create_directory_and_parents()
        .unwrap();
    // inputDir3 just doesn't exist
    // outputDir only exists partially
    f.exec_root
        .get_relative(&output_dir)
        .get_parent_directory()
        .unwrap()
        .create_directory_and_parents()
        .unwrap();
    f.exec_root
        .get_relative_str("justSomeDir/thatIsDoomed")
        .create_directory_and_parents()
        .unwrap();
    // `thiswillbeafile/output` simulates a directory that was in the stashed dir but whose same
    // path is used later for a regular file.
    f.exec_root
        .get_relative_str("thiswillbeafile/output")
        .create_directory_and_parents()
        .unwrap();
    f.file("execroot/thiswillbeafile/output/file1", &[]);
    dirs_to_create.insert(PathFragment::create("thiswillbeafile"));
    let input4 = PathFragment::create("thiswillbeafile/output");
    let inputs2 = SandboxInputs::new(
        [
            (input1.clone(), Some(input_txt.clone())),
            (input2.clone(), Some(input_txt.clone())),
            (input3.clone(), Some(input_txt.clone())),
            (input4.clone(), Some(input_txt.clone())),
        ]
        .into_iter()
        .collect(),
        IndexMap::new(),
        IndexMap::new(),
    );
    sandbox_helpers::clean_existing(
        &root_dir,
        &inputs2,
        &mut inputs_to_create,
        &mut dirs_to_create,
        &f.exec_root,
        Some(&f.tree_deleter),
    )
    .unwrap();
    assert_eq!(
        hs(dirs_to_create.iter()),
        hs([&input_dir2, &input_dir3, &output_dir])
    );
    assert!(
        f.exec_root
            .get_relative_str("existing/directory/with")
            .exists()
    );
    assert!(f.exec_root.get_relative_str("partial").exists());
    assert!(
        !f.exec_root
            .get_relative_str("partial/doomedSubdir")
            .exists()
    );
    assert!(!f.exec_root.get_relative_str("partial/directory").exists());
    assert!(
        !f.exec_root
            .get_relative_str("justSomeDir/thatIsDoomed")
            .exists()
    );
    assert!(f.exec_root.get_relative_str("out").exists());
    assert!(!f.exec_root.get_relative_str("out/dir").exists());
}

#[test]
fn create_content_map_with_only_empty_input_tracks_containing_directories() {
    let f = Fixture::new();
    let empty_input = PathFragment::create("api/__init__.py");
    let inputs = SandboxInputs::new(
        [(empty_input, None)].into_iter().collect(),
        IndexMap::new(),
        IndexMap::new(),
    );

    let sandbox_contents: SandboxContents = sandbox_helpers::create_content_map(
        &f.exec_root,
        &inputs,
        &SandboxOutputs::get_empty_instance(),
    );

    let work_dir_contents = sandbox_contents
        .dir_map()
        .get(f.exec_root.get_base_name())
        .expect("workDirContents");
    let api_contents = work_dir_contents.dir_map().get("api").expect("apiContents");
    assert_eq!(api_contents.file_map().get("__init__.py"), Some(&None));
}

#[test]
fn update_content_map_preserves_empty_input_in_updated_directory() {
    // PORT: Java advances a ManualClock by 1 ms; the real clock is used here, with a sleep long
    // enough for the file system's coarse timestamps to move past `timestamp`.
    let f = Fixture::new();
    let work_dir = f.root.get_relative_str("execroot");
    let empty_file = f.file("execroot/api/__init__.py", &[]);
    let empty_input = PathFragment::create("api/__init__.py");
    let inputs = SandboxInputs::new(
        [(empty_input, None)].into_iter().collect(),
        IndexMap::new(),
        IndexMap::new(),
    );
    let mut contents = sandbox_helpers::create_content_map(
        &work_dir,
        &inputs,
        &SandboxOutputs::get_empty_instance(),
    );
    std::thread::sleep(std::time::Duration::from_millis(50));
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64;

    std::thread::sleep(std::time::Duration::from_millis(50));
    let unexpected_file = f.file("execroot/api/unexpected", &["unexpected"]);
    sandbox_helpers::update_content_map(
        &work_dir.get_parent_directory().unwrap(),
        timestamp,
        &mut contents,
    )
    .unwrap();

    assert!(empty_file.is_file());
    assert!(!unexpected_file.exists());
    assert_eq!(
        contents.dir_map()["execroot"].dir_map()["api"]
            .file_map()
            .get("__init__.py"),
        Some(&None)
    );
}

fn clean_existing_with_in_memory_contents_tracks_empty_inputs(keep_empty_input: bool) {
    let f = Fixture::new();
    let empty_input = PathFragment::create("api/__init__.py");
    let shared_input = PathFragment::create("api/shared.py");
    let shared_source = f.file("inputs/shared.py", &["shared"]);
    let empty_file = f.file("execroot/api/__init__.py", &[]);
    f.exec_root
        .get_relative(&shared_input)
        .create_symbolic_link_to_fragment(shared_source.as_fragment())
        .unwrap();

    let previous_files: IndexMap<PathFragment, Option<Path>> = [
        (empty_input.clone(), None),
        (shared_input.clone(), Some(shared_source.clone())),
    ]
    .into_iter()
    .collect();
    let previous_inputs =
        SandboxInputs::new(previous_files.clone(), IndexMap::new(), IndexMap::new());
    let sandbox_contents = sandbox_helpers::create_content_map(
        &f.exec_root,
        &previous_inputs,
        &SandboxOutputs::get_empty_instance(),
    );

    let mut current_files = previous_files.clone();
    if !keep_empty_input {
        current_files.shift_remove(&empty_input);
    }
    let current_inputs =
        SandboxInputs::new(current_files.clone(), IndexMap::new(), IndexMap::new());
    let mut inputs_to_create: IndexSet<PathFragment> = IndexSet::new();
    let mut dirs_to_create: IndexSet<PathFragment> = IndexSet::new();
    sandbox_helpers::populate_inputs_and_dirs_to_create(
        &IndexSet::<PathFragment>::new(),
        &mut inputs_to_create,
        &mut dirs_to_create,
        current_files.keys(),
        &SandboxOutputs::get_empty_instance(),
    );

    sandbox_helpers::clean_existing_with_contents(
        &f.exec_root.get_parent_directory().unwrap(),
        &current_inputs,
        &mut inputs_to_create,
        &mut dirs_to_create,
        &f.exec_root,
        Some(&f.tree_deleter),
        Some(&sandbox_contents),
    )
    .unwrap();

    assert_eq!(empty_file.exists(), keep_empty_input);
    assert!(inputs_to_create.is_empty());
}

#[test]
fn clean_existing_with_in_memory_contents_tracks_empty_inputs_keep_empty_input_true() {
    clean_existing_with_in_memory_contents_tracks_empty_inputs(true);
}

#[test]
fn clean_existing_with_in_memory_contents_tracks_empty_inputs_keep_empty_input_false() {
    clean_existing_with_in_memory_contents_tracks_empty_inputs(false);
}

#[test]
#[ignore = "needs a non-noop PathMapper (path mapping is not ported) and tree artifacts from blaze_actions"]
fn populate_inputs_and_dirs_to_create_creates_mapped_directories() {}

fn move_outputs_moves_file(force_copy: bool) {
    let f = Fixture::new();
    if force_copy {
        f.forbid_rename_to();
    }

    let sandbox_file = f.sandbox_root.get_relative_str("output");
    write_content(&sandbox_file, "hello");

    let spawn = spawn_with_outputs(&["output"]);
    sandbox_helpers::move_outputs(
        &sandbox_helpers::get_outputs(&spawn),
        &f.sandbox_root,
        &f.exec_root,
    )
    .unwrap();

    let real_file = f.exec_root.get_relative_str("output");
    assert!(real_file.is_file());
    assert_eq!(read_content(&real_file), "hello");
}

#[test]
fn move_outputs_moves_file_force_copy_false() {
    move_outputs_moves_file(false);
}

#[test]
fn move_outputs_moves_file_force_copy_true() {
    move_outputs_moves_file(true);
}

fn move_outputs_moves_symlink(force_copy: bool) {
    let f = Fixture::new();
    if force_copy {
        f.forbid_rename_to();
    }

    let sandbox_symlink = f.sandbox_root.get_relative_str("output");
    sandbox_symlink
        .create_symbolic_link_to_fragment(&PathFragment::create("target"))
        .unwrap();

    let spawn = spawn_with_outputs(&["output"]);
    sandbox_helpers::move_outputs(
        &sandbox_helpers::get_outputs(&spawn),
        &f.sandbox_root,
        &f.exec_root,
    )
    .unwrap();

    let real_symlink = f.exec_root.get_relative_str("output");
    assert!(real_symlink.is_symbolic_link());
    assert_eq!(
        real_symlink.read_symbolic_link().unwrap(),
        PathFragment::create("target")
    );
}

#[test]
fn move_outputs_moves_symlink_force_copy_false() {
    move_outputs_moves_symlink(false);
}

#[test]
fn move_outputs_moves_symlink_force_copy_true() {
    move_outputs_moves_symlink(true);
}

fn move_outputs_moves_directory(force_copy: bool) {
    let f = Fixture::new();
    if force_copy {
        f.forbid_rename_to();
    }

    let sandbox_dir = f.sandbox_root.get_relative_str("output");
    sandbox_dir.create_directory_and_parents().unwrap();
    write_content(&sandbox_dir.get_relative_str("file"), "hello");
    sandbox_dir
        .get_relative_str("symlink")
        .create_symbolic_link_to_fragment(&PathFragment::create("target"))
        .unwrap();
    sandbox_dir
        .get_relative_str("subdir")
        .create_directory_and_parents()
        .unwrap();

    let spawn = spawn_with_outputs(&["output"]);
    sandbox_helpers::move_outputs(
        &sandbox_helpers::get_outputs(&spawn),
        &f.sandbox_root,
        &f.exec_root,
    )
    .unwrap();

    let real_dir = f.exec_root.get_relative_str("output");
    assert!(real_dir.is_directory());
    assert!(real_dir.get_relative_str("file").is_file());
    assert_eq!(read_content(&real_dir.get_relative_str("file")), "hello");
    assert!(real_dir.get_relative_str("symlink").is_symbolic_link());
    assert_eq!(
        real_dir
            .get_relative_str("symlink")
            .read_symbolic_link()
            .unwrap(),
        PathFragment::create("target")
    );
    assert!(real_dir.get_relative_str("subdir").is_directory());
}

#[test]
fn move_outputs_moves_directory_force_copy_false() {
    move_outputs_moves_directory(false);
}

#[test]
fn move_outputs_moves_directory_force_copy_true() {
    move_outputs_moves_directory(true);
}

fn move_outputs_ignores_missing(force_copy: bool) {
    let f = Fixture::new();
    if force_copy {
        f.forbid_rename_to();
    }

    let spawn = spawn_with_outputs(&["output"]);
    sandbox_helpers::move_outputs(
        &sandbox_helpers::get_outputs(&spawn),
        &f.sandbox_root,
        &f.exec_root,
    )
    .unwrap();

    assert!(!f.exec_root.get_relative_str("output").exists());
}

#[test]
fn move_outputs_ignores_missing_force_copy_false() {
    move_outputs_ignores_missing(false);
}

#[test]
fn move_outputs_ignores_missing_force_copy_true() {
    move_outputs_ignores_missing(true);
}

#[test]
fn move_outputs_fixes_permissions_on_file_when_copying() {
    let f = Fixture::new();
    f.forbid_rename_to();

    let sandbox_file = f.sandbox_root.get_relative_str("output");
    write_content(&sandbox_file, "hello");
    sandbox_file.chmod(0).unwrap();

    let spawn = spawn_with_outputs(&["output"]);
    sandbox_helpers::move_outputs(
        &sandbox_helpers::get_outputs(&spawn),
        &f.sandbox_root,
        &f.exec_root,
    )
    .unwrap();

    let real_file = f.exec_root.get_relative_str("output");
    assert!(real_file.is_file());
    assert_eq!(read_content(&real_file), "hello");
}

#[test]
fn move_outputs_fixes_permissions_on_directory_when_copying() {
    let f = Fixture::new();
    f.forbid_rename_to();

    let sandbox_dir = f.sandbox_root.get_relative_str("output");
    sandbox_dir.create_directory_and_parents().unwrap();
    write_content(&sandbox_dir.get_relative_str("file"), "hello");
    sandbox_dir.chmod(0).unwrap();

    let spawn = spawn_with_outputs(&["output"]);
    sandbox_helpers::move_outputs(
        &sandbox_helpers::get_outputs(&spawn),
        &f.sandbox_root,
        &f.exec_root,
    )
    .unwrap();

    let real_dir = f.exec_root.get_relative_str("output");
    assert!(real_dir.is_directory());
    assert!(real_dir.get_relative_str("file").is_file());
    assert_eq!(read_content(&real_dir.get_relative_str("file")), "hello");
}

#[test]
#[ignore = "needs a non-noop PathMapper (path mapping is not ported)"]
fn move_outputs_mapped_path_moved_to_unmapped_path_force_copy_false() {}

#[test]
#[ignore = "needs a non-noop PathMapper (path mapping is not ported)"]
fn move_outputs_mapped_path_moved_to_unmapped_path_force_copy_true() {}
