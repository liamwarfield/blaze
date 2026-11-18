//! Port of `src/test/java/com/google/devtools/build/lib/analysis/RunfilesTest.java`.

use std::collections::BTreeMap;
use std::sync::Arc;

use blaze_actions::actions::artifact::Artifact;
use blaze_actions::analysis::runfiles::{
    ConflictPolicy, EmptyFilesSupplier, Runfiles, RunfilesBuilder, RunfilesConflictReceiver,
};
use blaze_actions::{ArtifactOwner, ArtifactRoot};
use blaze_cmdline::label_constants::{EXTERNAL_PATH_PREFIX, EXTERNAL_RUNFILES_PATH_PREFIX};
use blaze_util::Fingerprint;
use blaze_vfs::{DigestHashFunction, FileSystemRef, Path, PathFragment, Root, UnixFileSystem};
use indexmap::{IndexMap, IndexSet};

/// `--nested_set_depth_limit` default (BuildLanguageOptions).
const DEFAULT_NESTED_SET_DEPTH_LIMIT: i32 = 3500;

fn fs() -> FileSystemRef {
    UnixFileSystem::create(DigestHashFunction::Sha256)
}

/// `ArtifactRoot.asSourceRoot(Root.fromPath(scratch.resolve("/workspace")))`.
fn workspace_root() -> ArtifactRoot {
    ArtifactRoot::as_source_root(Root::from_path(Path::create(
        PathFragment::create("/workspace"),
        &fs(),
    )))
}

/// `ActionsTestUtil.createArtifactWithRootRelativePath` for a source root.
fn create_artifact_with_root_relative_path(
    root: &ArtifactRoot,
    root_relative_path: PathFragment,
) -> Artifact {
    let exec_path = root.get_exec_path().get_relative(&root_relative_path);
    Artifact::new_source_artifact(root.clone(), exec_path, ArtifactOwner::NullOwner)
}

/// `ActionsTestUtil.createArtifact(root, String)`.
fn create_artifact(root: &ArtifactRoot, path: &str) -> Artifact {
    create_artifact_with_root_relative_path(root, PathFragment::create(path))
}

fn pf(s: &str) -> PathFragment {
    PathFragment::create(s)
}

/// The test's `warningPrefixConflictReceiver()` reporting into an event list.
#[derive(Default)]
struct WarningPrefixConflictReceiver {
    warnings: Vec<String>,
}

impl RunfilesConflictReceiver for WarningPrefixConflictReceiver {
    fn nested_runfiles_tree(&mut self, runfiles_tree: &Artifact) {
        panic!("AssertionError: {}", runfiles_tree);
    }

    fn prefix_conflict(&mut self, message: String) {
        self.warnings.push(message);
    }
}

impl WarningPrefixConflictReceiver {
    fn check_warning(&self) {
        assert!(
            self.warnings
                .iter()
                .any(|w| w.contains("obscured by a -> x")),
            "{:?}",
            self.warnings
        );
        assert_eq!(
            self.warnings.len(),
            1,
            "Runfiles.filterListForObscuringSymlinks should have warned once"
        );
    }
}

fn filtered(
    receiver: &mut dyn RunfilesConflictReceiver,
    map: &IndexMap<PathFragment, Option<Artifact>>,
) -> BTreeMap<PathFragment, Option<Artifact>> {
    Runfiles::filter_list_for_obscuring_symlinks(receiver, map)
        .iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect()
}

#[test]
fn test_filter_list_for_obscuring_symlinks_catches_bad_obscurer() {
    let mut obscuring_map = IndexMap::new();
    let path_a = pf("a");
    let root = workspace_root();
    let artifact_a = create_artifact(&root, "x");
    obscuring_map.insert(path_a.clone(), Some(artifact_a.clone()));
    obscuring_map.insert(pf("a/b"), Some(create_artifact(&root, "c/b")));
    let mut receiver = WarningPrefixConflictReceiver::default();
    assert_eq!(
        filtered(&mut receiver, &obscuring_map),
        BTreeMap::from([(path_a, Some(artifact_a))])
    );
    receiver.check_warning();
}

#[test]
fn test_filter_list_for_obscuring_symlinks_catches_bad_grand_parent_obscurer() {
    let mut obscuring_map = IndexMap::new();
    let path_a = pf("a");
    let root = workspace_root();
    let artifact_a = create_artifact(&root, "x");
    obscuring_map.insert(path_a.clone(), Some(artifact_a.clone()));
    obscuring_map.insert(pf("a/b/c"), Some(create_artifact(&root, "b/c")));
    let mut receiver = WarningPrefixConflictReceiver::default();
    assert_eq!(
        filtered(&mut receiver, &obscuring_map),
        BTreeMap::from([(path_a, Some(artifact_a))])
    );
    receiver.check_warning();
}

#[test]
fn test_filter_list_for_obscuring_symlinks_catches_bad_obscurer_no_listener() {
    let mut obscuring_map = IndexMap::new();
    let path_a = pf("a");
    let root = workspace_root();
    let artifact_a = create_artifact(&root, "a");
    obscuring_map.insert(path_a.clone(), Some(artifact_a.clone()));
    obscuring_map.insert(pf("a/b"), Some(create_artifact(&root, "c/b")));
    let mut receiver = WarningPrefixConflictReceiver::default();
    assert_eq!(
        filtered(&mut receiver, &obscuring_map),
        BTreeMap::from([(path_a, Some(artifact_a))])
    );
}

#[test]
fn test_filter_list_for_obscuring_symlinks_ignores_ok_obscurer() {
    let mut obscuring_map = IndexMap::new();
    let path_a = pf("a");
    let root = workspace_root();
    let artifact_a = create_artifact(&root, "a");
    obscuring_map.insert(path_a.clone(), Some(artifact_a.clone()));
    obscuring_map.insert(pf("a/b"), Some(create_artifact(&root, "a/b")));

    let mut receiver = WarningPrefixConflictReceiver::default();
    assert_eq!(
        filtered(&mut receiver, &obscuring_map),
        BTreeMap::from([(path_a, Some(artifact_a))])
    );
    assert!(receiver.warnings.is_empty());
}

#[test]
fn test_filter_list_for_obscuring_symlinks_no_obscurers() {
    let mut obscuring_map = IndexMap::new();
    let path_a = pf("a");
    let root = workspace_root();
    let artifact_a = create_artifact(&root, "a");
    obscuring_map.insert(path_a.clone(), Some(artifact_a.clone()));
    let path_bc = pf("b/c");
    let artifact_bc = create_artifact(&root, "a/b");
    obscuring_map.insert(path_bc.clone(), Some(artifact_bc.clone()));
    let mut receiver = WarningPrefixConflictReceiver::default();
    assert_eq!(
        filtered(&mut receiver, &obscuring_map),
        BTreeMap::from([(path_a, Some(artifact_a)), (path_bc, Some(artifact_bc))])
    );
    assert!(receiver.warnings.is_empty());
}

#[test]
fn test_builder_merge_conflict_policy_default() {
    let r1 = RunfilesBuilder::new("TESTING").build();
    let r2 = RunfilesBuilder::new("TESTING").merge(&r1).build();
    assert_eq!(r2.get_conflict_policy(), ConflictPolicy::WARN);
}

#[test]
fn test_builder_merge_conflict_policy_inherit() {
    let r1 = RunfilesBuilder::new("TESTING")
        .build()
        .set_conflict_policy(ConflictPolicy::WARN);
    let r2 = RunfilesBuilder::new("TESTING").merge(&r1).build();
    assert_eq!(r2.get_conflict_policy(), ConflictPolicy::WARN);
}

#[test]
fn test_builder_merge_conflict_policy_inherit_strictest() {
    let r1 = RunfilesBuilder::new("TESTING")
        .build()
        .set_conflict_policy(ConflictPolicy::WARN);
    let r2 = RunfilesBuilder::new("TESTING")
        .build()
        .set_conflict_policy(ConflictPolicy::ERROR);
    let r3 = RunfilesBuilder::new("TESTING")
        .merge(&r1)
        .merge(&r2)
        .build();
    assert_eq!(r3.get_conflict_policy(), ConflictPolicy::ERROR);
    // Swap ordering
    let r4 = RunfilesBuilder::new("TESTING")
        .merge(&r2)
        .merge(&r1)
        .build();
    assert_eq!(r4.get_conflict_policy(), ConflictPolicy::ERROR);
}

#[test]
fn test_runfile_added() {
    let root = workspace_root();
    let path_b = pf("repo/b");
    let legacy_path_b = EXTERNAL_PATH_PREFIX.get_relative(&path_b);
    let runfiles_path_b = EXTERNAL_RUNFILES_PATH_PREFIX.get_relative(&path_b);
    let artifact_b = create_artifact_with_root_relative_path(&root, legacy_path_b);

    let runfiles = RunfilesBuilder::new("wsname")
        .add_symlink(runfiles_path_b, artifact_b.clone())
        .build();

    let mut receiver = WarningPrefixConflictReceiver::default();
    assert_eq!(
        runfiles.get_runfiles_inputs_with_receiver(
            &mut receiver,
            /* repo_mapping_manifest= */ None
        ),
        BTreeMap::from([
            (pf("wsname/.runfile"), None),
            (pf("repo/b"), Some(artifact_b)),
        ])
    );
    assert!(receiver.warnings.is_empty());
}

#[test]
fn test_merge_with_symlinks() {
    let root = workspace_root();
    let artifact_a = create_artifact(&root, "a/target");
    let artifact_b = create_artifact(&root, "b/target");
    let runfiles_a = RunfilesBuilder::new("TESTING")
        .add_symlink(pf("a/symlink"), artifact_a.clone())
        .build();
    let runfiles_b = RunfilesBuilder::new("TESTING")
        .add_symlink(pf("b/symlink"), artifact_b.clone())
        .build();

    let runfiles_c = runfiles_a
        .merge(&runfiles_b, DEFAULT_NESTED_SET_DEPTH_LIMIT)
        .unwrap();
    assert_eq!(
        runfiles_c.get_runfiles_inputs(/* repo_mapping_manifest= */ None),
        BTreeMap::from([
            (pf("TESTING/a/symlink"), Some(artifact_a)),
            (pf("TESTING/b/symlink"), Some(artifact_b)),
        ])
    );
}

#[test]
fn merge_all_symlinks() {
    let root = workspace_root();
    let artifact_a = create_artifact(&root, "a/target");
    let artifact_b = create_artifact(&root, "b/target");
    let artifact_c = create_artifact(&root, "c/target");
    let runfiles_a = RunfilesBuilder::new("TESTING")
        .add_symlink(pf("a/symlink"), artifact_a.clone())
        .build();
    let runfiles_b = RunfilesBuilder::new("TESTING")
        .add_symlink(pf("b/symlink"), artifact_b.clone())
        .build();
    let runfiles_c = RunfilesBuilder::new("TESTING")
        .add_symlink(pf("c/symlink"), artifact_c.clone())
        .build();

    let runfiles_merged = runfiles_a
        .merge_all(&[runfiles_b, runfiles_c], DEFAULT_NESTED_SET_DEPTH_LIMIT)
        .unwrap();
    assert_eq!(
        runfiles_merged.get_runfiles_inputs(/* repo_mapping_manifest= */ None),
        BTreeMap::from([
            (pf("TESTING/a/symlink"), Some(artifact_a)),
            (pf("TESTING/b/symlink"), Some(artifact_b)),
            (pf("TESTING/c/symlink"), Some(artifact_c)),
        ])
    );
}

#[test]
fn test_merge_empty_with_non_empty() {
    let root = workspace_root();
    let artifact_a = create_artifact(&root, "a/target");
    let runfiles_b = RunfilesBuilder::new("TESTING")
        .add_artifact(artifact_a)
        .build();

    assert!(Runfiles::ptr_eq(
        &Runfiles::empty()
            .merge(&runfiles_b, DEFAULT_NESTED_SET_DEPTH_LIMIT)
            .unwrap(),
        &runfiles_b
    ));
    assert!(Runfiles::ptr_eq(
        &runfiles_b
            .merge(&Runfiles::empty(), DEFAULT_NESTED_SET_DEPTH_LIMIT)
            .unwrap(),
        &runfiles_b
    ));
}

#[test]
fn merge_all_empty_with_non_empty() {
    let root = workspace_root();
    let artifact = create_artifact(&root, "target");
    let non_empty = RunfilesBuilder::new("TESTING")
        .add_artifact(artifact)
        .build();
    let limit = DEFAULT_NESTED_SET_DEPTH_LIMIT;

    assert!(Runfiles::ptr_eq(
        &Runfiles::empty()
            .merge_all(std::slice::from_ref(&non_empty), limit)
            .unwrap(),
        &non_empty
    ));
    assert!(Runfiles::ptr_eq(
        &Runfiles::empty()
            .merge_all(
                &[Runfiles::empty(), non_empty.clone(), Runfiles::empty()],
                limit
            )
            .unwrap(),
        &non_empty
    ));
    assert!(Runfiles::ptr_eq(
        &non_empty
            .merge_all(&[Runfiles::empty(), Runfiles::empty()], limit)
            .unwrap(),
        &non_empty
    ));
    assert!(Runfiles::ptr_eq(
        &non_empty.merge_all(&[], limit).unwrap(),
        &non_empty
    ));
}

#[test]
fn merge_all_empty_with_empty() {
    let limit = DEFAULT_NESTED_SET_DEPTH_LIMIT;
    assert!(Runfiles::ptr_eq(
        &Runfiles::empty().merge_all(&[], limit).unwrap(),
        &Runfiles::empty()
    ));
    assert!(Runfiles::ptr_eq(
        &Runfiles::empty()
            .merge_all(&[Runfiles::empty(), Runfiles::empty()], limit)
            .unwrap(),
        &Runfiles::empty()
    ));
}

#[test]
fn merge_exceeds_depth_limit_throws_exception() {
    let root = workspace_root();
    let artifact_a = create_artifact(&root, "a/target");
    let artifact_b = create_artifact(&root, "b/target");
    let artifact_c = create_artifact(&root, "c/target");
    let runfiles_a = RunfilesBuilder::new("TESTING")
        .add_artifact(artifact_a)
        .build();
    let runfiles_b = RunfilesBuilder::new("TESTING")
        .add_artifact(artifact_b)
        .build();
    let runfiles_c = RunfilesBuilder::new("TESTING")
        .add_artifact(artifact_c)
        .build();
    // --nested_set_depth_limit=2
    let limit = 2;

    let merge_ab = runfiles_a.merge(&runfiles_b, limit).unwrap();
    let expected = merge_ab.merge(&runfiles_c, limit).unwrap_err();
    assert!(
        expected
            .get_message()
            .contains("artifacts depset depth 3 exceeds limit (2)"),
        "{}",
        expected.get_message()
    );
}

#[test]
fn merge_all_exceeds_depth_limit_throws_exception() {
    let root = workspace_root();
    let artifact_a = create_artifact(&root, "a/target");
    let artifact_b = create_artifact(&root, "b/target");
    let artifact_c = create_artifact(&root, "c/target");
    let sympath_a = pf("a/symlink");
    let sympath_b = pf("b/symlink");
    let sympath_c = pf("c/symlink");
    let runfiles_a = RunfilesBuilder::new("TESTING")
        .add_symlink(sympath_a, artifact_a)
        .build();
    let runfiles_b = RunfilesBuilder::new("TESTING")
        .add_symlink(sympath_b, artifact_b)
        .build();
    let runfiles_c = RunfilesBuilder::new("TESTING")
        .add_symlink(sympath_c, artifact_c)
        .build();
    // --nested_set_depth_limit=2
    let limit = 2;

    let merge_all_ab = runfiles_a.merge_all(&[runfiles_b], limit).unwrap();
    let expected = merge_all_ab.merge_all(&[runfiles_c], limit).unwrap_err();
    assert!(
        expected
            .get_message()
            .contains("symlinks depset depth 3 exceeds limit (2)"),
        "{}",
        expected.get_message()
    );
}

/// The test's anonymous supplier: `<basename>-empty` next to each manifest path.
struct DashEmptySupplier;

impl EmptyFilesSupplier for DashEmptySupplier {
    fn get_extra_paths(&self, manifest_paths: &IndexSet<PathFragment>) -> Vec<PathFragment> {
        manifest_paths
            .iter()
            .map(|f| {
                f.replace_name(&format!("{}-empty", f.get_base_name()))
                    .unwrap()
            })
            .collect()
    }

    fn fingerprint(&self, _fingerprint: &mut Fingerprint) {}

    fn get_class_name(&self) -> String {
        "com.google.devtools.build.lib.analysis.RunfilesTest$1".to_string()
    }
}

#[test]
fn test_get_empty_filenames() {
    let root = workspace_root();
    let artifact = create_artifact(&root, "my-artifact");
    let runfiles = RunfilesBuilder::new("TESTING")
        .add_artifact(artifact.clone())
        .add_symlink(pf("my-symlink"), artifact.clone())
        .add_root_symlink(pf("my-root-symlink"), artifact)
        .set_empty_files_supplier(Arc::new(DashEmptySupplier))
        .build();
    let mut actual = runfiles.get_empty_filenames();
    actual.sort();
    let mut expected = vec![pf("my-artifact-empty"), pf("my-symlink-empty")];
    expected.sort();
    assert_eq!(actual, expected);
}
