//! Port of `src/test/java/com/google/devtools/build/lib/remote/RemoteActionInputFetcherTest.java`
//! with the tests of its base class `ActionInputPrefetcherTestBase.java`, and of
//! `RemoteOutputCheckerTest.java`.
//!
//! Java runs the tests on an `InMemoryFileSystem` wrapped in a Mockito spy; here they run on the
//! real file system under a temporary directory. Where Java verifies `doDownloadFile` calls on a
//! spied prefetcher, the port counts the downloads of the in-memory cache client.
#![allow(clippy::field_reassign_with_default)]

mod util;

use std::any::Any;
use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, Once};

use blaze_actions::{
    ActionLookupData, ActionLookupKey, ActionLookupKeyRef, Artifact as BlazeArtifact, ArtifactRoot,
    RootType, SpecialArtifactType,
};
use blaze_cmdline::Label;
use blaze_events::StoredEventHandler;
use blaze_exec::actions::action_input_prefetcher::{Priority, Reason};
use blaze_exec::actions::{
    ActionInputRef, BasicVirtualActionInput, FakeOwner, FileArtifactValue, TreeArtifactValue,
};
use blaze_optdefs::misc::remote_outputs_mode::RemoteOutputsMode;
use blaze_remote::abstract_action_input_prefetcher::OutputPermissions;
use blaze_remote::common::bulk_transfer_exception::BulkTransferException;
use blaze_remote::common::errors;
use blaze_remote::remote_action_input_fetcher::RemoteActionInputFetcher;
use blaze_remote::remote_output_checker::RemoteOutputChecker;
use blaze_remote::util::digest_util::DigestUtil;
use blaze_remote::util::temp_path_generator::TempPathGenerator;
use blaze_remote::util::tracing_metadata_utils;
use blaze_vfs::{DigestHashFunction, Path, PathFragment, UnixFileSystem};
use util::in_memory_cache_client::{InMemoryCacheClient, InMemoryCombinedCache};

static SET_TOOL_VERSION: Once = Once::new();

/// `ActionsTestUtil.NULL_ARTIFACT_OWNER`.
#[derive(Debug)]
struct NullArtifactOwner;

impl fmt::Display for NullArtifactOwner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("NULL_ARTIFACT_OWNER")
    }
}

impl ActionLookupKey for NullArtifactOwner {
    fn get_label(&self) -> Option<Label> {
        Some(Label::parse_canonical("//null/action:owner").unwrap())
    }
    fn get_configuration_checksum(&self) -> Option<String> {
        None
    }
    fn function_name(&self) -> &'static str {
        "NULL_ARTIFACT_OWNER"
    }
    fn key_equals(&self, other: &dyn ActionLookupKey) -> bool {
        other.as_any().is::<NullArtifactOwner>()
    }
    fn key_hash(&self) -> u64 {
        1
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

fn null_artifact_owner() -> ActionLookupKeyRef {
    ActionLookupKeyRef::new(NullArtifactOwner)
}

/// The metadata the tests hand the prefetcher (`Map<ActionInput, FileArtifactValue>`).
type Metadata = Vec<(ActionInputRef, FileArtifactValue)>;

fn lookup(metadata: &Metadata, input: &ActionInputRef) -> Option<FileArtifactValue> {
    metadata
        .iter()
        .find(|(i, _)| i.get_exec_path() == input.get_exec_path())
        .map(|(_, m)| m.clone())
}

// Java: com/google/devtools/build/lib/remote/ActionInputPrefetcherTestBase.java:101 ActionInputPrefetcherTestBase.setUp
struct Fixture {
    _dir: tempfile::TempDir,
    exec_root: Path,
    artifact_root: ArtifactRoot,
    temp_path_generator: Arc<TempPathGenerator>,
    digest_util: DigestUtil,
    action: FakeOwner,
}

impl Fixture {
    fn set_up() -> Fixture {
        SET_TOOL_VERSION.call_once(|| tracing_metadata_utils::set_tool_version(""));
        let dir = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(dir.path())
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();
        let fs = UnixFileSystem::create(DigestHashFunction::Sha256);
        let exec_root = Path::create_from_str(&format!("{root}/exec"), &fs);
        exec_root.create_directory_and_parents().unwrap();
        let artifact_root = ArtifactRoot::as_derived_root(&exec_root, RootType::OUTPUT, &["root"]);
        artifact_root
            .get_root()
            .as_path()
            .unwrap()
            .create_directory_and_parents()
            .unwrap();
        let temp_dir = Path::create_from_str(&format!("{root}/tmp"), &fs);
        temp_dir.create_directory_and_parents().unwrap();
        Fixture {
            _dir: dir,
            exec_root,
            artifact_root,
            temp_path_generator: Arc::new(TempPathGenerator::new(temp_dir)),
            digest_util: DigestUtil::new(DigestHashFunction::Sha256),
            action: FakeOwner::new("DummyAction", None, None),
        }
    }

    fn root_path(&self) -> Path {
        self.artifact_root.get_root().as_path().unwrap().clone()
    }

    // Java: com/google/devtools/build/lib/remote/ActionInputPrefetcherTestBase.java:122 ActionInputPrefetcherTestBase.createRemoteArtifact
    fn create_remote_artifact(
        &self,
        path_fragment: &str,
        contents: &str,
        resolved_path: Option<PathFragment>,
        metadata: &mut Metadata,
        cas: &mut HashMap<blaze_remote::reapi::Digest, Vec<u8>>,
    ) -> ActionInputRef {
        let exec_path = self
            .artifact_root
            .get_exec_path()
            .get_relative(&PathFragment::create(path_fragment));
        let a = ActionInputRef::new(BlazeArtifact::create_derived_artifact(
            self.artifact_root.clone(),
            exec_path,
            null_artifact_owner(),
        ));
        let digest = self.digest_util.compute(contents.as_bytes());
        let mut f = FileArtifactValue::create_for_remote_file_with_materialization_data(
            DigestUtil::to_binary_digest(&digest),
            contents.len() as i64,
            /* location_index= */ 1,
            /* expiration_time_millis= */ None,
            /* in_memory_output= */ false,
        );
        if let Some(resolved_path) = resolved_path {
            f = FileArtifactValue::create_from_existing_with_resolved_path(f, resolved_path);
        }
        metadata.push((a.clone(), f));
        cas.insert(digest, contents.as_bytes().to_vec());
        a
    }

    // Java: com/google/devtools/build/lib/remote/ActionInputPrefetcherTestBase.java:151 ActionInputPrefetcherTestBase.createRemoteTreeArtifact
    fn create_remote_tree_artifact(
        &self,
        path_fragment: &str,
        local_content_map: &[(&str, &str)],
        remote_content_map: &[(&str, &str)],
        resolved_path: Option<PathFragment>,
        metadata: &mut Metadata,
        cas: &mut HashMap<blaze_remote::reapi::Digest, Vec<u8>>,
    ) -> (BlazeArtifact, Vec<ActionInputRef>) {
        let parent = BlazeArtifact::create_special_artifact(
            self.artifact_root.clone(),
            self.artifact_root
                .get_exec_path()
                .get_relative(&PathFragment::create(path_fragment)),
            null_artifact_owner(),
            SpecialArtifactType::TREE,
        );
        parent.set_generating_action_key(ActionLookupData::create(null_artifact_owner(), 0));
        let mut children = Vec::new();
        for (path, contents) in local_content_map {
            let child = BlazeArtifact::create_tree_output_str(&parent, path);
            let digest = self.digest_util.compute(contents.as_bytes());
            let value = FileArtifactValue::create_for_normal_file(
                Some(DigestUtil::to_binary_digest(&digest)),
                /* proxy= */ None,
                contents.len() as i64,
            );
            cas.insert(digest, contents.as_bytes().to_vec());
            children.push((ActionInputRef::new(child), value));
        }
        for (path, contents) in remote_content_map {
            let child = BlazeArtifact::create_tree_output_str(&parent, path);
            let digest = self.digest_util.compute(contents.as_bytes());
            let value = FileArtifactValue::create_for_remote_file_with_materialization_data(
                DigestUtil::to_binary_digest(&digest),
                contents.len() as i64,
                1,
                None,
                false,
            );
            cas.insert(digest, contents.as_bytes().to_vec());
            children.push((ActionInputRef::new(child), value));
        }
        let tree = TreeArtifactValue::build(children.clone(), None, resolved_path);
        metadata.push((
            ActionInputRef::new(parent.clone()),
            tree.metadata.clone().unwrap(),
        ));
        for (child, value) in &children {
            metadata.push((child.clone(), value.clone()));
        }
        // `treeValue.getChildren()`: sorted by exec path.
        let children: Vec<ActionInputRef> =
            tree.child_values.iter().map(|(c, _)| c.clone()).collect();
        (parent, children)
    }

    // Java: com/google/devtools/build/lib/remote/RemoteActionInputFetcherTest.java:76 RemoteActionInputFetcherTest.createPrefetcher
    fn create_prefetcher(
        &self,
        cas: HashMap<blaze_remote::reapi::Digest, Vec<u8>>,
    ) -> (RemoteActionInputFetcher, Arc<InMemoryCacheClient>) {
        let (combined_cache, client) =
            InMemoryCombinedCache::new_with_cas_entries(cas, self.digest_util.clone()).into_parts();
        let fetcher = RemoteActionInputFetcher::new(
            Arc::new(StoredEventHandler::new()),
            "none".to_string(),
            "none".to_string(),
            combined_cache,
            self.exec_root.clone(),
            self.temp_path_generator.clone(),
            dummy_remote_output_checker(),
            OutputPermissions::READONLY,
        );
        (fetcher, client.unwrap())
    }

    fn prefetch(
        &self,
        prefetcher: &RemoteActionInputFetcher,
        metadata: &Metadata,
    ) -> std::io::Result<()> {
        let inputs: Vec<ActionInputRef> = metadata.iter().map(|(i, _)| i.clone()).collect();
        let supplier = |input: &ActionInputRef| Ok(lookup(metadata, input));
        blaze_remote::runtime::get_from_future(prefetcher.base().prefetch_files_interruptibly(
            Some(&self.action),
            &inputs,
            &supplier,
            Priority::MEDIUM,
            Reason::INPUTS,
        ))
    }

    fn path_of(&self, input: &ActionInputRef) -> Path {
        self.exec_root.get_relative(input.get_exec_path())
    }
}

/// `DUMMY_REMOTE_OUTPUT_CHECKER`.
fn dummy_remote_output_checker() -> Arc<RemoteOutputChecker> {
    Arc::new(RemoteOutputChecker::new(
        "build",
        RemoteOutputsMode::MINIMAL,
        Vec::new(),
    ))
}

fn read_content(path: &Path) -> String {
    std::fs::read_to_string(path.get_path_string()).unwrap()
}

fn write_content(path: &Path, contents: &str) {
    path.get_parent_directory()
        .unwrap()
        .create_directory_and_parents()
        .unwrap();
    blaze_repo_fetch::fs_utils::write_content(path, contents.as_bytes()).unwrap();
}

// Java: com/google/devtools/build/lib/remote/ActionInputPrefetcherTestBase.java:1220 ActionInputPrefetcherTestBase.assertReadableNonWritableAndExecutable
fn assert_readable_non_writable_and_executable(path: &Path) {
    assert!(path.is_readable().unwrap(), "{path} should be readable");
    assert!(
        !path.is_writable().unwrap(),
        "{path} should not be writable"
    );
    assert!(path.is_executable().unwrap(), "{path} should be executable");
}

fn sorted(mut paths: Vec<Path>) -> Vec<String> {
    let mut v: Vec<String> = paths
        .drain(..)
        .map(|p| p.get_path_string().to_string())
        .collect();
    v.sort();
    v
}

#[test]
fn prefetch_files_file_exists_do_not_download() {
    let f = Fixture::set_up();
    let mut metadata = Metadata::new();
    let mut cas = HashMap::new();
    let a = f.create_remote_artifact("file", "hello world", None, &mut metadata, &mut cas);
    write_content(&f.path_of(&a), "hello world");
    let (prefetcher, client) = f.create_prefetcher(cas);

    f.prefetch(&prefetcher, &metadata).unwrap();

    assert_eq!(client.get_num_successful_downloads(), 0);
    assert_eq!(
        sorted(prefetcher.base().downloaded_files()),
        sorted(vec![f.path_of(&a)])
    );
    assert!(prefetcher.base().downloads_in_progress().is_empty());
}

#[test]
#[ignore = "needs FileArtifactValue.setContentsProxy (Blaze's FileArtifactValue is immutable)"]
fn prefetch_files_file_exists_records_contents_proxy() {}

#[test]
fn prefetch_files_file_exists_but_content_mismatches_download() {
    let f = Fixture::set_up();
    let mut metadata = Metadata::new();
    let mut cas = HashMap::new();
    let a = f.create_remote_artifact("file", "hello world remote", None, &mut metadata, &mut cas);
    write_content(&f.path_of(&a), "hello world local");
    let (prefetcher, client) = f.create_prefetcher(cas);

    f.prefetch(&prefetcher, &metadata).unwrap();

    assert_eq!(client.get_num_successful_downloads(), 1);
    assert_eq!(
        sorted(prefetcher.base().downloaded_files()),
        sorted(vec![f.path_of(&a)])
    );
    assert!(prefetcher.base().downloads_in_progress().is_empty());
    assert_eq!(read_content(&f.path_of(&a)), "hello world remote");
}

#[test]
fn invalidate_downloads_redownloads_file_deleted_after_download() {
    let f = Fixture::set_up();
    let mut metadata = Metadata::new();
    let mut cas = HashMap::new();
    let invalidated =
        f.create_remote_artifact("file1", "hello world", None, &mut metadata, &mut cas);
    let not_invalidated =
        f.create_remote_artifact("file2", "fizz buzz", None, &mut metadata, &mut cas);
    let (prefetcher, _) = f.create_prefetcher(cas);

    f.prefetch(&prefetcher, &metadata).unwrap();
    assert_eq!(read_content(&f.path_of(&invalidated)), "hello world");

    // Both files are deleted behind the prefetcher's back, but only one of them is invalidated.
    f.path_of(&invalidated).delete().unwrap();
    f.path_of(&not_invalidated).delete().unwrap();
    prefetcher
        .base()
        .invalidate_downloads(&[invalidated.get_exec_path().clone()]);

    f.prefetch(&prefetcher, &metadata).unwrap();
    assert_eq!(read_content(&f.path_of(&invalidated)), "hello world");
    // Downloads that weren't invalidated are still assumed to be in place.
    assert!(!f.path_of(&not_invalidated).exists());
}

#[test]
fn invalidate_downloads_intact_file_not_redownloaded() {
    let f = Fixture::set_up();
    let mut metadata = Metadata::new();
    let mut cas = HashMap::new();
    let a = f.create_remote_artifact("file", "hello world", None, &mut metadata, &mut cas);
    let (prefetcher, client) = f.create_prefetcher(cas);

    f.prefetch(&prefetcher, &metadata).unwrap();
    assert_eq!(read_content(&f.path_of(&a)), "hello world");

    prefetcher
        .base()
        .invalidate_downloads(&[a.get_exec_path().clone()]);
    f.prefetch(&prefetcher, &metadata).unwrap();

    // The second prefetch checks the file against the file system rather than assuming that it
    // is still in place, but doesn't download it again as it is intact.
    // PORT: Java also verifies the `statIfFound` on a spied file system.
    assert_eq!(client.get_num_successful_downloads(), 1);
    assert_eq!(read_content(&f.path_of(&a)), "hello world");
}

#[test]
fn prefetch_files_download_remote_files() {
    let f = Fixture::set_up();
    let mut metadata = Metadata::new();
    let mut cas = HashMap::new();
    let a1 = f.create_remote_artifact("file1", "hello world", None, &mut metadata, &mut cas);
    let a2 = f.create_remote_artifact("file2", "fizz buzz", None, &mut metadata, &mut cas);
    let (prefetcher, _) = f.create_prefetcher(cas);

    f.prefetch(&prefetcher, &metadata).unwrap();

    assert_eq!(read_content(&f.path_of(&a1)), "hello world");
    assert_readable_non_writable_and_executable(&f.path_of(&a1));
    assert_eq!(read_content(&f.path_of(&a2)), "fizz buzz");
    assert_readable_non_writable_and_executable(&f.path_of(&a2));
    assert_eq!(
        sorted(prefetcher.base().downloaded_files()),
        sorted(vec![f.path_of(&a1), f.path_of(&a2)])
    );
    assert!(prefetcher.base().downloads_in_progress().is_empty());
}

#[test]
fn prefetch_files_download_remote_files_with_resolved_path() {
    let f = Fixture::set_up();
    let mut metadata = Metadata::new();
    let mut cas = HashMap::new();
    let resolved_path = f.root_path().get_child("target").as_fragment().clone();
    let a = f.create_remote_artifact(
        "file",
        "hello world",
        Some(resolved_path.clone()),
        &mut metadata,
        &mut cas,
    );
    let (prefetcher, _) = f.create_prefetcher(cas);

    f.prefetch(&prefetcher, &metadata).unwrap();

    let path = f.path_of(&a);
    assert!(path.is_symbolic_link());
    assert_eq!(path.read_symbolic_link().unwrap(), resolved_path);
    assert_eq!(read_content(&path), "hello world");
    assert_eq!(
        sorted(prefetcher.base().downloaded_files()),
        sorted(vec![path.clone(), f.root_path().get_child("target")])
    );
    assert!(prefetcher.base().downloads_in_progress().is_empty());
}

#[test]
fn prefetch_files_download_remote_trees() {
    let f = Fixture::set_up();
    let mut metadata = Metadata::new();
    let mut cas = HashMap::new();
    let (tree, children) = f.create_remote_tree_artifact(
        "dir",
        &[],
        &[("file1", "content1"), ("nested_dir/file2", "content2")],
        None,
        &mut metadata,
        &mut cas,
    );
    let (prefetcher, _) = f.create_prefetcher(cas);

    // The tree artifact itself is not passed (only its expanded children).
    let children_metadata: Metadata = metadata
        .iter()
        .filter(|(i, _)| {
            children
                .iter()
                .any(|c| c.get_exec_path() == i.get_exec_path())
        })
        .cloned()
        .collect();
    let inputs: Vec<ActionInputRef> = children_metadata.iter().map(|(i, _)| i.clone()).collect();
    let supplier = |input: &ActionInputRef| Ok(lookup(&metadata, input));
    blaze_remote::runtime::get_from_future(prefetcher.base().prefetch_files_interruptibly(
        Some(&f.action),
        &inputs,
        &supplier,
        Priority::MEDIUM,
        Reason::INPUTS,
    ))
    .unwrap();

    let tree_path = f.exec_root.get_relative(tree.get_exec_path());
    assert_eq!(
        read_content(&tree_path.get_relative_str("file1")),
        "content1"
    );
    assert_eq!(
        read_content(&tree_path.get_relative_str("nested_dir/file2")),
        "content2"
    );
    assert_readable_non_writable_and_executable(&tree_path.get_relative_str("file1"));
    // The tree's directories get the output permissions back.
    assert!(!tree_path.is_writable().unwrap());
    assert!(
        !tree_path
            .get_relative_str("nested_dir")
            .is_writable()
            .unwrap()
    );
    assert!(prefetcher.base().downloads_in_progress().is_empty());
}

/// Prefetches `inputs` with `metadata` as the supplier.
fn prefetch_inputs(
    f: &Fixture,
    prefetcher: &RemoteActionInputFetcher,
    inputs: &[ActionInputRef],
    metadata: &Metadata,
) {
    let supplier = |input: &ActionInputRef| Ok(lookup(metadata, input));
    blaze_remote::runtime::get_from_future(prefetcher.base().prefetch_files_interruptibly(
        Some(&f.action),
        inputs,
        &supplier,
        Priority::MEDIUM,
        Reason::INPUTS,
    ))
    .unwrap();
}

// Java: com/google/devtools/build/lib/remote/ActionInputPrefetcherTestBase.java:1226 ActionInputPrefetcherTestBase.assertTreeReadableNonWritableAndExecutable
fn assert_tree_readable_non_writable_and_executable(path: &Path) {
    assert_readable_non_writable_and_executable(path);
    for entry in std::fs::read_dir(path.get_path_string()).unwrap() {
        let child = path.get_child(entry.unwrap().file_name().to_str().unwrap());
        if child.is_directory() {
            assert_tree_readable_non_writable_and_executable(&child);
        } else {
            assert_readable_non_writable_and_executable(&child);
        }
    }
}

// Java: com/google/devtools/build/lib/remote/ActionInputPrefetcherTestBase.java:318 ActionInputPrefetcherTestBase.prefetchFiles_downloadRemoteTrees_partial
#[test]
fn prefetch_files_download_remote_trees_partial() {
    let f = Fixture::set_up();
    let mut metadata = Metadata::new();
    let mut cas = HashMap::new();
    let (tree, children) = f.create_remote_tree_artifact(
        "dir",
        &[("file1", "content1")],
        &[("file2", "content2")],
        None,
        &mut metadata,
        &mut cas,
    );
    let first_child = children[0].clone();
    let second_child = children[1].clone();
    let (prefetcher, _) = f.create_prefetcher(cas);

    prefetch_inputs(
        &f,
        &prefetcher,
        &[first_child.clone(), second_child.clone()],
        &metadata,
    );

    assert!(!f.path_of(&first_child).exists());
    assert_eq!(read_content(&f.path_of(&second_child)), "content2");
    assert_tree_readable_non_writable_and_executable(
        &f.exec_root.get_relative(tree.get_exec_path()),
    );
    assert_eq!(
        sorted(prefetcher.base().downloaded_files()),
        sorted(vec![f.path_of(&second_child)])
    );
}

// Java: com/google/devtools/build/lib/remote/ActionInputPrefetcherTestBase.java:345 ActionInputPrefetcherTestBase.prefetchFiles_downloadRemoteTrees_withResolvedPath
#[test]
fn prefetch_files_download_remote_trees_with_resolved_path() {
    let f = Fixture::set_up();
    let mut metadata = Metadata::new();
    let mut cas = HashMap::new();
    let resolved_path = f.root_path().get_child("target").as_fragment().clone();
    let (tree, children) = f.create_remote_tree_artifact(
        "dir",
        &[],
        &[("file1", "content1"), ("nested_dir/file2", "content2")],
        Some(resolved_path.clone()),
        &mut metadata,
        &mut cas,
    );
    let first_child = children[0].clone();
    let second_child = children[1].clone();
    let (prefetcher, _) = f.create_prefetcher(cas);

    prefetch_inputs(&f, &prefetcher, &children, &metadata);

    let tree_path = f.exec_root.get_relative(tree.get_exec_path());
    assert!(tree_path.is_symbolic_link());
    assert_eq!(tree_path.read_symbolic_link().unwrap(), resolved_path);
    assert_eq!(read_content(&f.path_of(&first_child)), "content1");
    assert_eq!(read_content(&f.path_of(&second_child)), "content2");
    let resolved = f.root_path().get_child("target");
    assert_tree_readable_non_writable_and_executable(&resolved);
    let relative = |c: &ActionInputRef| {
        c.as_artifact()
            .and_then(|a| a.get_parent_relative_path())
            .unwrap()
    };
    assert_eq!(
        sorted(prefetcher.base().downloaded_files()),
        sorted(vec![
            tree_path.clone(),
            resolved.get_relative(&relative(&first_child)),
            resolved.get_relative(&relative(&second_child)),
        ])
    );
    assert!(prefetcher.base().downloads_in_progress().is_empty());
}

#[test]
#[ignore = "port pending: tree artifacts with local children, resolved paths and action template expansions"]
fn prefetch_files_download_remote_trees_for_action_template_expansion() {}

#[test]
fn prefetch_files_missing_files_fails() {
    let f = Fixture::set_up();
    let mut metadata = Metadata::new();
    let mut cas = HashMap::new();
    let a = f.create_remote_artifact("file1", "hello world", None, &mut metadata, &mut cas);
    let (prefetcher, _) = f.create_prefetcher(HashMap::new());
    let _ = cas;

    let e = f.prefetch(&prefetcher, &metadata).unwrap_err();
    assert!(
        errors::downcast_ref::<BulkTransferException>(&e).is_some(),
        "{e}"
    );
    assert!(
        prefetcher
            .base()
            .downloaded_files()
            .iter()
            .all(|p| *p != f.path_of(&a))
    );
    assert!(prefetcher.base().downloads_in_progress().is_empty());
}

#[test]
fn prefetch_files_ignore_non_remote_files() {
    let f = Fixture::set_up();
    let a = ActionInputRef::new(BlazeArtifact::create_derived_artifact(
        f.artifact_root.clone(),
        f.artifact_root
            .get_exec_path()
            .get_relative(&PathFragment::create("file1")),
        null_artifact_owner(),
    ));
    write_content(&f.path_of(&a), "hello world");
    let metadata: Metadata = vec![(
        a.clone(),
        FileArtifactValue::create_for_testing(&f.path_of(&a)).unwrap(),
    )];
    let (prefetcher, client) = f.create_prefetcher(HashMap::new());

    f.prefetch(&prefetcher, &metadata).unwrap();

    assert_eq!(client.get_num_successful_downloads(), 0);
    assert!(prefetcher.base().downloads_in_progress().is_empty());
}

#[test]
#[ignore = "port pending: local children of a tree artifact"]
fn prefetch_files_ignore_non_remote_files_tree() {}

#[test]
#[ignore = "needs SpiedFileSystem (Mockito verification of file system calls)"]
fn prefetch_files_tree_files_minimize_filesystem_operations() {}

#[test]
#[ignore = "needs SpiedFileSystem (Mockito verification of file system calls)"]
fn prefetch_files_tree_files_sequential_calls_reopen_ancestors() {}

#[test]
#[ignore = "needs DelayedChmodFileSystem and a thread pool racing prefetchFiles calls"]
fn prefetch_files_tree_files_multiple_threads_wait_for_permissions_to_be_set() {}

#[test]
#[ignore = "needs DelayedChmodFileSystem and a thread pool racing prefetchFiles calls"]
fn prefetch_files_tree_files_concurrent_finalizations_do_not_race_permissions() {}

#[test]
#[ignore = "needs DelayedChmodFileSystem and a thread pool racing prefetchFiles calls"]
fn prefetch_files_tree_files_many_concurrent_overlapping_calls() {}

#[test]
#[ignore = "needs thread interruption (Blaze has none, see crate::runtime)"]
fn prefetch_files_multiple_threads_download_is_cancelled() {}

#[test]
#[ignore = "needs thread interruption (Blaze has none, see crate::runtime)"]
fn prefetch_files_multiple_threads_download_is_not_cancelled_by_other_threads() {}

#[test]
#[ignore = "needs thread interruption (Blaze has none, see crate::runtime)"]
fn prefetch_file_interrupting_metadata_supplier_interrupts_download() {}

#[test]
#[ignore = "needs thread interruption (Blaze has none, see crate::runtime)"]
fn prefetch_files_on_interrupt_delete_partial_downloaded_file() {}

#[test]
#[ignore = "needs LostInputsExecException (action rewinding)"]
fn missing_inputs_exception_has_lost_inputs() {}

// Java: com/google/devtools/build/lib/remote/RemoteActionInputFetcherTest.java:90 RemoteActionInputFetcherTest.testStagingVirtualActionInput
#[test]
fn test_staging_virtual_action_input() {
    let f = Fixture::set_up();
    let (prefetcher, _) = f.create_prefetcher(HashMap::new());
    let a = ActionInputRef::new(BasicVirtualActionInput::new(
        PathFragment::create("file1"),
        b"hello world".to_vec(),
    ));

    let supplier = |_: &ActionInputRef| Ok(None);
    blaze_remote::runtime::get_from_future(prefetcher.base().prefetch_files_interruptibly(
        Some(&f.action),
        std::slice::from_ref(&a),
        &supplier,
        Priority::MEDIUM,
        Reason::INPUTS,
    ))
    .unwrap();

    let p = f.exec_root.get_relative(a.get_exec_path());
    assert_eq!(read_content(&p), "hello world");
    assert!(p.is_executable().unwrap());
    assert!(prefetcher.base().downloaded_files().is_empty());
    assert!(prefetcher.base().downloads_in_progress().is_empty());
}

// Java: com/google/devtools/build/lib/remote/RemoteActionInputFetcherTest.java:124 RemoteActionInputFetcherTest.testStagingEmptyVirtualActionInput
#[test]
#[ignore = "needs VirtualActionInput.EMPTY_MARKER as an ActionInputRef"]
fn test_staging_empty_virtual_action_input() {}

// Java: com/google/devtools/build/lib/remote/RemoteActionInputFetcherTest.java:154 RemoteActionInputFetcherTest.prefetchFiles_missingFiles_failsWithSpecificMessage
#[test]
fn prefetch_files_missing_files_fails_with_specific_message() {
    let f = Fixture::set_up();
    let mut metadata = Metadata::new();
    let mut cas = HashMap::new();
    let a = f.create_remote_artifact("file1", "hello world", None, &mut metadata, &mut cas);
    let digest = cas.keys().next().unwrap().clone();
    let (prefetcher, _) = f.create_prefetcher(HashMap::new());

    let e = f.prefetch(&prefetcher, &metadata).unwrap_err();
    let bulk = errors::downcast_ref::<BulkTransferException>(&e).expect("BulkTransferException");
    assert_eq!(
        bulk.get_message(),
        format!(
            "Missing digest: {}/{} for {}",
            digest.hash,
            digest.size_bytes,
            a.get_exec_path().get_path_string()
        )
    );
}

#[test]
#[ignore = "needs the remote repository contents cache (RemoteExternalOverlayFileSystem)"]
fn inject_remote_repo_invalid_path_throws_io_exception() {}

#[test]
#[ignore = "needs action rewinding (handleRewoundActionOutputs) and an overlay file system"]
fn rewound_action_output_exec_root_on_overlay_file_system_redownloaded() {}

#[test]
#[ignore = "needs an overlay file system (forHostFileSystem)"]
fn prefetch_files_exec_root_on_overlay_file_system_replaces_parent_file() {}

// Java: com/google/devtools/build/lib/remote/RemoteOutputCheckerTest.java:38 RemoteOutputCheckerTest.testShouldDownloadOutput
#[test]
fn test_should_download_output() {
    let f = Fixture::set_up();
    let remote_output_checker =
        RemoteOutputChecker::new("build", RemoteOutputsMode::MINIMAL, Vec::new());
    let exec_root = ArtifactRoot::as_derived_root(&f.exec_root, RootType::OUTPUT, &["out"]);
    let tree = BlazeArtifact::create_special_artifact(
        exec_root.clone(),
        PathFragment::create("out/foo/bar"),
        null_artifact_owner(),
        SpecialArtifactType::TREE,
    );
    tree.set_generating_action_key(ActionLookupData::create(null_artifact_owner(), 0));
    remote_output_checker.add_output_to_download(&tree);
    remote_output_checker.add_output_to_download(&BlazeArtifact::create_derived_artifact(
        exec_root,
        PathFragment::create("out/foo/bar-baz"),
        null_artifact_owner(),
    ));
    let p = PathFragment::create;
    assert!(!remote_output_checker.should_download_output_path(&p("out/foo/bar-quz"), None));
    assert!(remote_output_checker.should_download_output_path(&p("out/foo/bar"), None));
    assert!(
        remote_output_checker
            .should_download_output_path(&p("out/foo/bar/data.txt"), Some(&p("out/foo/bar")))
    );
    assert!(remote_output_checker.should_download_output_path(&p("out/foo/bar-baz"), None));
}
