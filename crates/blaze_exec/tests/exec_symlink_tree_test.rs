//! Ports of `exec/SymlinkTreeHelperTest.java` and `exec/SymlinkTreeStrategyTest.java`.

mod analysis_actions_common;

use std::collections::BTreeMap;
use std::sync::Arc;

use analysis_actions_common::{TestDirs, null_action_owner, read, write};
use blaze_actions::SpecialArtifactType;
use blaze_actions::analysis::actions::symlink_tree_action::{
    RunfileSymlinksMode, SymlinkTreeAction,
};
use blaze_actions::analysis::runfiles::{EmptyFilesSupplier, RunfilesBuilder};
use blaze_actions::{ActionEnvironment, Artifact};
use blaze_exec::actions::action_adapters::artifact_input;
use blaze_exec::actions::{ExecException, FilesetOutputSymlink, as_executable};
use blaze_exec::analysis::actions::symlink_tree_action::SymlinkTreeActionContext;
use blaze_exec::exec::symlink_tree_helper::SymlinkTreeHelper;
use blaze_exec::exec::symlink_tree_strategy::{OutputService, SymlinkTreeStrategy, to_path};
use blaze_util::Fingerprint;
use blaze_vfs::{PathFragment, Symlinks};
use indexmap::{IndexMap, IndexSet};
use parking_lot::Mutex;

const WORKSPACE_NAME: &str = "_main";

// ---------------------------------------------------------------------------------------------
// SymlinkTreeHelperTest

#[test]
fn process_fileset_links() {
    let dirs = TestDirs::new();
    let target1 = dirs.bin_artifact("target1");
    let target2 = dirs.bin_artifact("target2");
    let link1 = FilesetOutputSymlink {
        name: PathFragment::create("from1"),
        target: artifact_input(&target1),
    };
    let link2 = FilesetOutputSymlink {
        name: PathFragment::create("from2"),
        target: artifact_input(&target2),
    };

    let symlinks = SymlinkTreeHelper::process_fileset_links(&[link1, link2], "workspace");
    let mut expected = BTreeMap::new();
    expected.insert(
        PathFragment::create("workspace/from1"),
        target1.get_path().as_fragment().clone(),
    );
    expected.insert(
        PathFragment::create("workspace/from2"),
        target2.get_path().as_fragment().clone(),
    );
    assert_eq!(symlinks, expected);
}

#[derive(Clone, Copy, Debug)]
enum TreeType {
    Runfiles,
    Fileset,
}

// Java: SymlinkTreeHelperTest.createSymlinks(@TestParameter TreeType, @TestParameter boolean replace)
fn create_symlinks(tree_type: TreeType, replace: bool) {
    let dirs = TestDirs::new();
    let exec_root = &dirs.exec_root;
    let tree_root = exec_root.get_relative_str("foo.runfiles");
    let input_manifest_path = exec_root.get_relative_str("foo.runfiles_manifest");
    let output_manifest_path = exec_root.get_relative_str("foo.runfiles/MANIFEST");
    let helper = SymlinkTreeHelper::new(
        &input_manifest_path,
        &output_manifest_path,
        &tree_root,
        WORKSPACE_NAME,
    );

    let file = dirs.bin_artifact("file");
    let symlink = dirs.special_artifact("symlink", SpecialArtifactType::UNRESOLVED_SYMLINK);

    write(&file.get_path(), "content");
    symlink
        .get_path()
        .create_symbolic_link_to_fragment(&PathFragment::create("/path/to/target"))
        .unwrap();

    let tree_workspace = tree_root.get_relative_str(WORKSPACE_NAME);
    let tree_empty = tree_workspace.get_relative_str("empty");
    let tree_file = tree_workspace.get_relative_str("file");
    let tree_symlink = tree_workspace.get_relative_str("symlink");
    let tree_missing = tree_workspace.get_relative_str("missing");

    if replace {
        tree_empty.create_directory_and_parents().unwrap();
        tree_file.create_directory_and_parents().unwrap();
        tree_symlink.create_directory_and_parents().unwrap();
        tree_missing.create_directory_and_parents().unwrap();
        tree_workspace.chmod(0o000).unwrap();
    }

    let p = |s: &str| PathFragment::create(&format!("{WORKSPACE_NAME}/{s}"));
    match tree_type {
        TreeType::Runfiles => {
            let mut symlink_map: BTreeMap<PathFragment, Option<Artifact>> = BTreeMap::new();
            symlink_map.insert(p("empty"), None);
            symlink_map.insert(p("file"), Some(file.clone()));
            symlink_map.insert(p("symlink"), Some(symlink.clone()));
            helper.create_runfiles_symlinks(&symlink_map).unwrap();
        }
        TreeType::Fileset => {
            let mut symlink_map: BTreeMap<PathFragment, Option<PathFragment>> = BTreeMap::new();
            symlink_map.insert(p("empty"), None);
            symlink_map.insert(p("file"), Some(file.get_path().as_fragment().clone()));
            symlink_map.insert(p("symlink"), Some(PathFragment::create("/path/to/target")));
            helper.create_fileset_symlinks(&symlink_map).unwrap();
        }
    }

    assert!(tree_root.is_directory());
    assert!(tree_workspace.is_directory());
    assert!(tree_empty.is_file());
    assert!(read(&tree_empty).is_empty());
    assert!(tree_file.is_symbolic_link());
    assert_eq!(
        &tree_file.read_symbolic_link().unwrap(),
        file.get_path().as_fragment()
    );
    assert!(tree_symlink.is_symbolic_link());
    assert_eq!(
        tree_symlink.read_symbolic_link().unwrap(),
        PathFragment::create("/path/to/target")
    );
    assert!(!tree_missing.exists());
}

#[test]
fn create_symlinks_runfiles_replace_false() {
    create_symlinks(TreeType::Runfiles, false);
}

#[test]
fn create_symlinks_runfiles_replace_true() {
    create_symlinks(TreeType::Runfiles, true);
}

#[test]
fn create_symlinks_fileset_replace_false() {
    create_symlinks(TreeType::Fileset, false);
}

#[test]
fn create_symlinks_fileset_replace_true() {
    create_symlinks(TreeType::Fileset, true);
}

// ---------------------------------------------------------------------------------------------
// SymlinkTreeStrategyTest

#[test]
fn test_artifact_to_path_conversion() {
    let dirs = TestDirs::new();
    let artifact = dirs.bin_artifact("dir/foo");
    assert_eq!(
        to_path(Some(&artifact)),
        Some(artifact.get_path().as_fragment().clone())
    );
    assert_eq!(to_path(None), None);
}

/// The test's `EmptyFilesSupplier` returning `dir/empty`.
struct DirEmpty;

impl EmptyFilesSupplier for DirEmpty {
    fn get_extra_paths(&self, _manifest_paths: &IndexSet<PathFragment>) -> Vec<PathFragment> {
        vec![PathFragment::create("dir/empty")]
    }
    fn fingerprint(&self, _fp: &mut Fingerprint) {}
    fn get_class_name(&self) -> String {
        "SymlinkTreeStrategyTest$1".to_string()
    }
}

/// A mock `OutputService` recording `createSymlinkTree` calls.
struct MockOutputService {
    can_create: bool,
    calls: Mutex<Vec<BTreeMap<PathFragment, Option<PathFragment>>>>,
    on_create: Box<dyn Fn() + Send + Sync>,
}

impl OutputService for MockOutputService {
    fn can_create_symlink_tree(&self) -> bool {
        self.can_create
    }
    fn create_symlink_tree(
        &self,
        symlinks: &BTreeMap<PathFragment, Option<PathFragment>>,
        _symlink_tree_root: &PathFragment,
    ) -> Result<(), ExecException> {
        self.calls.lock().push(symlinks.clone());
        (self.on_create)();
        Ok(())
    }
}

fn symlink_tree_action(dirs: &TestDirs) -> (SymlinkTreeAction, Artifact, Artifact) {
    let input_manifest = dirs.bin_artifact("dir/manifest.in");
    let output_manifest = dirs.bin_artifact("dir.runfiles/MANIFEST");
    let runfile = dirs.bin_artifact("dir/runfile");
    let mut b = RunfilesBuilder::new("TESTING");
    b.set_empty_files_supplier(Arc::new(DirEmpty));
    b.add_artifact(runfile.clone());
    let runfiles = b.build();
    let action = SymlinkTreeAction::new(
        null_action_owner(),
        input_manifest,
        Some(runfiles),
        output_manifest.clone(),
        /* repo_mapping_manifest= */ None,
        ActionEnvironment::create(IndexMap::new(), IndexSet::new()),
        RunfileSymlinksMode::CREATE,
        "workspace".to_string(),
    );
    (action, output_manifest, runfile)
}

fn execute_with(
    dirs: &TestDirs,
    action: &SymlinkTreeAction,
    output_service: Arc<MockOutputService>,
) {
    let mut ctx = dirs.context();
    let strategy: Arc<dyn SymlinkTreeActionContext> = Arc::new(SymlinkTreeStrategy::new(
        output_service,
        "_main".to_string(),
    ));
    ctx.register((
        std::any::TypeId::of::<Arc<dyn SymlinkTreeActionContext>>(),
        Arc::new(strategy),
    ));
    as_executable(action).unwrap().execute(&ctx).unwrap();
}

#[test]
fn with_output_service() {
    let dirs = TestDirs::new();
    let (action, output_manifest, runfile) = symlink_tree_action(&dirs);
    let parent = output_manifest.get_path().get_parent_directory().unwrap();
    let output_service = Arc::new(MockOutputService {
        can_create: true,
        calls: Mutex::new(Vec::new()),
        on_create: Box::new(move || parent.create_directory_and_parents().unwrap()),
    });
    execute_with(&dirs, &action, output_service.clone());

    let calls = output_service.calls.lock();
    assert_eq!(calls.len(), 1);
    let mut expected = BTreeMap::new();
    expected.insert(
        PathFragment::create("TESTING/dir/runfile"),
        Some(runfile.get_path().as_fragment().clone()),
    );
    expected.insert(PathFragment::create("TESTING/dir/empty"), None);
    assert_eq!(calls[0], expected);
}

#[test]
fn without_output_service() {
    let dirs = TestDirs::new();
    let (action, output_manifest, runfile) = symlink_tree_action(&dirs);
    let output_service = Arc::new(MockOutputService {
        can_create: false,
        calls: Mutex::new(Vec::new()),
        on_create: Box::new(|| {}),
    });
    execute_with(&dirs, &action, output_service.clone());
    // Check that the OutputService is not used.
    assert!(output_service.calls.lock().is_empty());

    let parent = output_manifest.get_path().get_parent_directory().unwrap();
    let p = parent.get_relative_str("TESTING/dir/runfile");
    assert!(
        p.exists_with(Symlinks::Nofollow),
        "Path {p} expected to exist"
    );
    assert!(p.is_symbolic_link(), "Path {p} expected to be a symlink");
    assert_eq!(
        &p.read_symbolic_link().unwrap(),
        runfile.get_path().as_fragment()
    );
    let q = parent.get_relative_str("TESTING/dir/empty");
    assert!(q.is_file(), "Path {q} expected to be a file");
    assert!(read(&q).is_empty());
}
