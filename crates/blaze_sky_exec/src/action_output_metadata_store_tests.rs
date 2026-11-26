//! Port of the `ActionOutputMetadataStoreTest` methods that the test module
//! `action_output_metadata_store_tests` of `skyframe_action_executor.rs` does not port
//! (src/test/java/com/google/devtools/build/lib/skyframe/ActionOutputMetadataStoreTest.java).
//!
//! The 30 `@Test` methods of the Java class and where each is ported (`existing`: the module
//! `action_output_metadata_store_tests` in `skyframe_action_executor.rs`; `here`: this file):
//!
//! | Java method (line)                                                   | Rust test | State |
//! |----------------------------------------------------------------------|-----------|-------|
//! | withNonArtifactInput (136)                                           | here      | pass |
//! | withArtifactInput (150)                                              | here      | pass |
//! | unknownSourceArtifactPermittedDuringInputDiscovery (164)             | here      | pass |
//! | unknownArtifactPermittedDuringInputDiscovery (174)                   | here      | pass |
//! | withKnownOutputArtifactStatsFile (184)                               | here      | pass |
//! | withMissingOutputArtifactStatsFileFailsWithException (193)           | here      | pass |
//! | unknownTreeArtifactPermittedDuringInputDiscovery (202)               | here      | pass |
//! | withUnknownOutputArtifactStatsFileTreeArtifact (213)                 | here      | pass |
//! | createsTreeArtifactValueFromFilesystem (224)                         | existing  | pass |
//! | withDanglingSymlinkInTreeArtifactFailsWithException (248)            | existing  | pass |
//! | resettingOutputs (262)                                               | here      | pass |
//! | injectRemoteArtifactMetadata (286)                                   | here      | pass |
//! | cannotInjectTreeArtifactChildIndividually (305)                      | here      | pass |
//! | canInjectTemplateExpansionOutput (323)                               | here      | pass |
//! | injectRemoteTreeArtifactMetadata (342)                               | here      | pass |
//! | fileArtifactMaterializedAsSymlinkToNonArtifact (376)                 | existing  | pass |
//! | fileArtifactMaterializedAsSymlinkToFileArtifact (400)                | existing  | pass |
//! | treeArtifactMaterializedAsSymlinkToNonArtifact (452)                 | existing  | pass |
//! | treeArtifactMaterializedAsSymlinkToAnotherTreeArtifact (483)         | existing  | pass |
//! | getMetadataFromFilesetMapping (534)                                  | here      | ignored: `ActionInputMap.putFileset` and the `filesetMapping` of `ActionInputMetadataProvider` are not ported |
//! | omitRegularArtifact (554)                                            | here      | pass |
//! | omitTreeArtifact (574)                                               | here      | pass |
//! | outputArtifactNotPreviouslyInjectedInExecutionMode (595)             | here      | pass |
//! | outputArtifactNotPreviouslyInjectedInExecutionMode_writablePermissions (612) | here | pass |
//! | outputTreeArtifactNotPreviouslyInjectedInExecutionMode (638)         | existing  | pass |
//! | enteringExecutionModeClearsCachedOutputs (671)                       | here      | pass |
//! | cannotEnterExecutionModeTwice (707)                                  | here      | pass |
//! | fileArtifactValueFromArtifactCompatibleWithGetMetadata_changed (714) | here      | ignored: `FileArtifactValue.couldBeModifiedSince` is not ported |
//! | fileArtifactValueFromArtifactCompatibleWithGetMetadata_notChanged (735) | here   | ignored: `FileArtifactValue.couldBeModifiedSince` is not ported |
//! | fileArtifactValueForSymlink_readFromCache (755)                      | here      | ignored: the `DigestUtils` digest cache is not ported |
//!
//! PORT: Java's `Scratch` file system is an `InMemoryFileSystem` that records `chmod` calls
//! (`chmodCalls`); here it is [`ChmodRecordingFileSystem`], the local file system under a
//! temporary directory with the same recording (and the same failure on a second `chmod` of one
//! path).

use std::any::Any;
use std::collections::HashMap;
use std::fmt;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::io::{self, Read, Write};
use std::panic::AssertUnwindSafe;
use std::sync::Arc;

use blaze_actions::actions::action_lookup::ActionLookupKey;
use blaze_actions::actions::artifact::SpecialArtifactType;
use blaze_actions::{ActionLookupData, ActionLookupKeyRef, Artifact, ArtifactRoot, RootType};
use blaze_cmdline::Label;
use blaze_exec::actions::action_adapters::artifact_input;
use blaze_exec::actions::cache::{IoOrInterruptedException, OutputMetadataStore};
use blaze_exec::actions::{
    ActionInputRef, BasicActionInput, FileArtifactValue, InputMetadataProvider, TreeArtifactValue,
};
use blaze_vfs::{
    DigestHashFunction, Dirent, FileStatus, FileSystem, FileSystemExt, FileSystemRef, Path,
    PathFragment, Root, UnixFileSystem,
};
use indexmap::IndexMap;
use parking_lot::Mutex;

use super::{
    ActionInputMetadata, ActionOutputMetadataStore, OutputPermissions,
    file_artifact_value_from_artifact,
};
use crate::action_execution_value::ActionExecutionValue;
use crate::action_template_expansion_value::ActionTemplateExpansionKey;

/// `ActionsTestUtil.NULL_ARTIFACT_OWNER`.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
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
        let mut h = DefaultHasher::new();
        self.hash(&mut h);
        h.finish()
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

fn null_artifact_owner() -> ActionLookupKeyRef {
    ActionLookupKeyRef::new(NullArtifactOwner)
}

/// The `Scratch` file system of the Java test: every `chmod` is recorded in `chmodCalls`, and a
/// second `chmod` of the same path fails the test.
// Java: src/test/java/com/google/devtools/build/lib/skyframe/ActionOutputMetadataStoreTest.java:77 ActionOutputMetadataStoreTest.chmodCalls
#[derive(Debug)]
struct ChmodRecordingFileSystem {
    delegate: FileSystemRef,
    chmod_calls: Mutex<HashMap<PathFragment, u32>>,
}

impl FileSystem for ChmodRecordingFileSystem {
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
    // Java: src/test/java/com/google/devtools/build/lib/skyframe/ActionOutputMetadataStoreTest.java:83 ActionOutputMetadataStoreTest.scratch.chmod
    fn chmod(&self, path: &PathFragment, mode: u32) -> io::Result<()> {
        {
            let mut chmod_calls = self.chmod_calls.lock();
            if chmod_calls.contains_key(path) {
                panic!("chmod called on {path} twice");
            }
            chmod_calls.insert(path.clone(), mode);
        }
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

/// The fields of the Java test: the scratch file system, `execRoot` (`/workspace` there,
/// `<tmp>/workspace` here), `sourceRoot` and `outputRoot`, created by `createRootDirs`.
struct Fixture {
    dir: tempfile::TempDir,
    scratch: Arc<ChmodRecordingFileSystem>,
    source_root: ArtifactRoot,
    output_root: ArtifactRoot,
}

impl Drop for Fixture {
    /// Lets the temporary directory be deleted after the store made directories read-only.
    fn drop(&mut self) {
        fn make_writable(dir: &std::path::Path) {
            use std::os::unix::fs::PermissionsExt;
            let Ok(metadata) = std::fs::symlink_metadata(dir) else {
                return;
            };
            if !metadata.is_dir() {
                return;
            }
            let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o755));
            if let Ok(entries) = std::fs::read_dir(dir) {
                for entry in entries.flatten() {
                    make_writable(&entry.path());
                }
            }
        }
        make_writable(self.dir.path());
    }
}

impl Fixture {
    // Java: src/test/java/com/google/devtools/build/lib/skyframe/ActionOutputMetadataStoreTest.java:102 ActionOutputMetadataStoreTest.createRootDirs
    fn new() -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let scratch = Arc::new(ChmodRecordingFileSystem {
            delegate: UnixFileSystem::create(DigestHashFunction::Sha256),
            chmod_calls: Mutex::new(HashMap::new()),
        });
        let fs: FileSystemRef = scratch.clone();
        let exec_root = fs.get_path(&format!("{}/workspace", dir.path().to_str().unwrap()));
        let source_root = ArtifactRoot::as_source_root(Root::from_path(exec_root.clone()));
        let output_root = ArtifactRoot::as_derived_root(&exec_root, RootType::OUTPUT, &["out"]);
        source_root
            .get_root()
            .as_path()
            .unwrap()
            .create_directory_and_parents()
            .unwrap();
        output_root
            .get_root()
            .as_path()
            .unwrap()
            .create_directory_and_parents()
            .unwrap();
        Fixture {
            dir,
            scratch,
            source_root,
            output_root,
        }
    }

    /// `chmodCalls`.
    fn chmod_calls(&self) -> HashMap<PathFragment, u32> {
        self.scratch.chmod_calls.lock().clone()
    }

    /// `chmodCalls.clear()`.
    fn clear_chmod_calls(&self) {
        self.scratch.chmod_calls.lock().clear();
    }

    // Java: src/test/java/com/google/devtools/build/lib/skyframe/ActionOutputMetadataStoreTest.java:107 ActionOutputMetadataStoreTest.createStore
    fn create_store(&self, outputs: &[Artifact]) -> ActionOutputMetadataStore {
        ActionOutputMetadataStore::create(
            /* archived_tree_artifacts_enabled= */ false,
            OutputPermissions::READONLY,
            outputs.to_vec(),
        )
    }
}

/// `ActionsTestUtil.createArtifactWithRootRelativePath(root, rootRelativePath)` (and
/// `ActionsTestUtil.createArtifact(root, path)`, the same artifact).
fn create_artifact_with_root_relative_path(
    root: &ArtifactRoot,
    root_relative_path: &str,
) -> Artifact {
    let exec_path = root.get_exec_path().get_relative_str(root_relative_path);
    if root.is_source_root() {
        Artifact::new_source_artifact(
            root.clone(),
            exec_path,
            blaze_actions::actions::action_lookup::ArtifactOwner::NullOwner,
        )
    } else {
        Artifact::create_derived_artifact(root.clone(), exec_path, null_artifact_owner())
    }
}

/// `ActionsTestUtil.createTreeArtifactWithGeneratingAction(root, rootRelativePath)`.
fn create_tree_artifact_with_generating_action(
    root: &ArtifactRoot,
    root_relative_path: &str,
) -> Artifact {
    let exec_path = root.get_exec_path().get_relative_str(root_relative_path);
    let owner = null_artifact_owner();
    let tree_artifact = Artifact::create_special_artifact(
        root.clone(),
        exec_path,
        owner.clone(),
        SpecialArtifactType::TREE,
    );
    tree_artifact.set_generating_action_key(ActionLookupData::create(owner, 0));
    tree_artifact
}

/// `scratch.file(pathName, lines...)`: each line followed by a newline; returns the path.
fn scratch_file(path: &Path, lines: &[&str]) -> Path {
    assert!(!path.exists(), "{path} already exists");
    path.get_parent_directory()
        .unwrap()
        .create_directory_and_parents()
        .unwrap();
    let content: String = lines.iter().map(|l| format!("{l}\n")).collect();
    std::fs::write(path.get_path_string(), content).unwrap();
    path.clone()
}

/// `scratch.overwriteFile(pathName, lines...)`.
fn scratch_overwrite_file(path: &Path, lines: &[&str]) -> Path {
    path.delete().unwrap();
    scratch_file(path, lines)
}

fn get_output_metadata(
    store: &ActionOutputMetadataStore,
    artifact: &Artifact,
) -> io::Result<Option<FileArtifactValue>> {
    match store.get_output_metadata(&artifact_input(artifact)) {
        Ok(value) => Ok(value),
        Err(IoOrInterruptedException::Io(e)) => Err(e),
        Err(IoOrInterruptedException::Interrupted(_)) => panic!("interrupted"),
    }
}

fn get_tree_artifact_value(
    store: &ActionOutputMetadataStore,
    artifact: &Artifact,
) -> TreeArtifactValue {
    match store.get_tree_artifact_value(&artifact_input(artifact)) {
        Ok(tree) => tree,
        Err(IoOrInterruptedException::Io(e)) => panic!("{e}"),
        Err(IoOrInterruptedException::Interrupted(_)) => panic!("interrupted"),
    }
}

/// `TreeArtifactValue.equals`: the digest (in the aggregate metadata), the children, the archived
/// representation and the resolved path.
fn assert_tree_eq(actual: &TreeArtifactValue, expected: &TreeArtifactValue) {
    assert_eq!(actual.metadata, expected.metadata);
    assert_eq!(actual.child_values, expected.child_values);
    assert_eq!(
        actual.archived_representation,
        expected.archived_representation
    );
    assert_eq!(actual.resolved_path, expected.resolved_path);
}

/// `assertThrows(<exception>.class, f)` for a Java unchecked exception, which the port raises as
/// a panic whose message starts with the exception's simple name.
fn assert_throws(exception: &str, f: impl FnOnce()) {
    let payload = std::panic::catch_unwind(AssertUnwindSafe(f))
        .err()
        .unwrap_or_else(|| panic!("expected {exception}, nothing was thrown"));
    let message = payload
        .downcast_ref::<String>()
        .cloned()
        .or_else(|| payload.downcast_ref::<&str>().map(|s| s.to_string()))
        .unwrap_or_default();
    assert!(
        message.starts_with(exception),
        "expected {exception}, got: {message}"
    );
}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/ActionOutputMetadataStoreTest.java:136 ActionOutputMetadataStoreTest.withNonArtifactInput
#[test]
fn with_non_artifact_input() {
    let f = Fixture::new();
    let input = ActionInputRef::new(BasicActionInput::from_path("foo/bar"));
    let metadata =
        FileArtifactValue::create_for_normal_file(Some(vec![1, 2, 3]), /* proxy= */ None, 10);
    let map = ActionInputMetadata::with_providers(None);
    map.put_input(input.clone(), metadata.clone());
    assert_eq!(
        map.action_input_map_get_input_metadata_checked(&input),
        Some(metadata)
    );
    let input_metadata_provider = map.action_input_map_only();
    assert_eq!(
        input_metadata_provider.get_input_metadata(&input).unwrap(),
        None
    );
    assert!(f.chmod_calls().is_empty());
}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/ActionOutputMetadataStoreTest.java:150 ActionOutputMetadataStoreTest.withArtifactInput
#[test]
fn with_artifact_input() {
    let f = Fixture::new();
    let artifact = create_artifact_with_root_relative_path(&f.source_root, "src/a");
    let metadata =
        FileArtifactValue::create_for_normal_file(Some(vec![1, 2, 3]), /* proxy= */ None, 10);
    let map = ActionInputMetadata::with_providers(None);
    map.put(&artifact, metadata.clone());
    let input_metadata_provider = map.action_input_map_only();
    assert_eq!(
        input_metadata_provider
            .get_input_metadata(&artifact_input(&artifact))
            .unwrap(),
        Some(metadata)
    );
    assert!(f.chmod_calls().is_empty());
}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/ActionOutputMetadataStoreTest.java:164 ActionOutputMetadataStoreTest.unknownSourceArtifactPermittedDuringInputDiscovery
#[test]
fn unknown_source_artifact_permitted_during_input_discovery() {
    let f = Fixture::new();
    let artifact = create_artifact_with_root_relative_path(&f.source_root, "src/a");
    let input_map = ActionInputMetadata::with_providers(None);
    let input_metadata_provider = input_map.action_input_map_only();
    assert_eq!(
        input_metadata_provider
            .get_input_metadata(&artifact_input(&artifact))
            .unwrap(),
        None
    );
    assert!(f.chmod_calls().is_empty());
}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/ActionOutputMetadataStoreTest.java:174 ActionOutputMetadataStoreTest.unknownArtifactPermittedDuringInputDiscovery
#[test]
fn unknown_artifact_permitted_during_input_discovery() {
    let f = Fixture::new();
    let artifact = create_artifact_with_root_relative_path(&f.output_root, "foo/bar");
    let input_map = ActionInputMetadata::with_providers(None);
    let input_metadata_provider = input_map.action_input_map_only();
    assert_eq!(
        input_metadata_provider
            .get_input_metadata(&artifact_input(&artifact))
            .unwrap(),
        None
    );
    assert!(f.chmod_calls().is_empty());
}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/ActionOutputMetadataStoreTest.java:184 ActionOutputMetadataStoreTest.withKnownOutputArtifactStatsFile
#[test]
fn with_known_output_artifact_stats_file() {
    let f = Fixture::new();
    let artifact = create_artifact_with_root_relative_path(&f.output_root, "foo/bar");
    scratch_file(&artifact.get_path(), &["not empty"]);
    let store = f.create_store(std::slice::from_ref(&artifact));
    assert!(get_output_metadata(&store, &artifact).unwrap().is_some());
    assert!(f.chmod_calls().is_empty());
}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/ActionOutputMetadataStoreTest.java:193 ActionOutputMetadataStoreTest.withMissingOutputArtifactStatsFileFailsWithException
#[test]
fn with_missing_output_artifact_stats_file_fails_with_exception() {
    let f = Fixture::new();
    let artifact = create_artifact_with_root_relative_path(&f.output_root, "foo/bar");
    assert!(!artifact.get_path().exists());
    let store = f.create_store(std::slice::from_ref(&artifact));
    // assertThrows(FileNotFoundException.class, ...)
    let e = get_output_metadata(&store, &artifact).unwrap_err();
    assert_eq!(e.kind(), io::ErrorKind::NotFound, "{e}");
    assert!(f.chmod_calls().is_empty());
}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/ActionOutputMetadataStoreTest.java:202 ActionOutputMetadataStoreTest.unknownTreeArtifactPermittedDuringInputDiscovery
#[test]
fn unknown_tree_artifact_permitted_during_input_discovery() {
    let f = Fixture::new();
    let tree_artifact = create_tree_artifact_with_generating_action(&f.output_root, "foo/bar");
    let artifact = Artifact::create_tree_output_str(&tree_artifact, "baz");
    let input_map = ActionInputMetadata::with_providers(None);
    let input_metadata_provider = input_map.action_input_map_only();
    assert_eq!(
        input_metadata_provider
            .get_input_metadata(&artifact_input(&artifact))
            .unwrap(),
        None
    );
    assert!(f.chmod_calls().is_empty());
}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/ActionOutputMetadataStoreTest.java:213 ActionOutputMetadataStoreTest.withUnknownOutputArtifactStatsFileTreeArtifact
#[test]
fn with_unknown_output_artifact_stats_file_tree_artifact() {
    let f = Fixture::new();
    let tree_artifact = create_tree_artifact_with_generating_action(&f.output_root, "foo/bar");
    let artifact = Artifact::create_tree_output_str(&tree_artifact, "baz");
    scratch_file(&artifact.get_path(), &["not empty"]);
    let store = f.create_store(std::slice::from_ref(&tree_artifact));
    assert!(get_output_metadata(&store, &artifact).unwrap().is_some());
    assert!(f.chmod_calls().is_empty());
}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/ActionOutputMetadataStoreTest.java:262 ActionOutputMetadataStoreTest.resettingOutputs
#[test]
fn resetting_outputs() {
    let f = Fixture::new();
    let artifact = create_artifact_with_root_relative_path(&f.output_root, "foo/bar");
    let output_path = scratch_file(&artifact.get_path(), &["not empty"]);
    let store = f.create_store(std::slice::from_ref(&artifact));
    store.prepare_for_action_execution();

    // The store doesn't have any info. It'll stat the file and discover that it's 10 bytes long.
    assert_eq!(
        get_output_metadata(&store, &artifact)
            .unwrap()
            .unwrap()
            .get_size(),
        10
    );
    assert_eq!(
        f.chmod_calls(),
        HashMap::from([(output_path.as_fragment().clone(), 0o555)])
    );

    // Inject a remote file of size 42.
    store.inject_file(
        &artifact_input(&artifact),
        FileArtifactValue::create_for_remote_file(vec![1, 2, 3], 42, 0),
    );
    assert_eq!(
        get_output_metadata(&store, &artifact)
            .unwrap()
            .unwrap()
            .get_size(),
        42
    );

    // Reset this output, which will make the store stat the file again.
    store.reset_outputs(&[artifact_input(&artifact)]);
    f.clear_chmod_calls();
    assert_eq!(
        get_output_metadata(&store, &artifact)
            .unwrap()
            .unwrap()
            .get_size(),
        10
    );
    // The store should not have chmodded the file as it already has the correct permission.
    assert!(f.chmod_calls().is_empty());
}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/ActionOutputMetadataStoreTest.java:286 ActionOutputMetadataStoreTest.injectRemoteArtifactMetadata
#[test]
fn inject_remote_artifact_metadata() {
    let f = Fixture::new();
    let artifact = create_artifact_with_root_relative_path(&f.output_root, "foo/bar");
    let store = f.create_store(std::slice::from_ref(&artifact));
    store.prepare_for_action_execution();

    let digest = vec![1u8, 2, 3];
    let size = 10;
    store.inject_file(
        &artifact_input(&artifact),
        FileArtifactValue::create_for_remote_file(
            digest.clone(),
            size,
            /* location_index= */ 1,
        ),
    );

    let v = get_output_metadata(&store, &artifact).unwrap();
    assert!(v.is_some());
    let v = v.unwrap();
    assert_eq!(v.get_digest(), Some(&digest[..]));
    assert_eq!(v.get_size(), size);
    assert!(f.chmod_calls().is_empty());
}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/ActionOutputMetadataStoreTest.java:305 ActionOutputMetadataStoreTest.cannotInjectTreeArtifactChildIndividually
#[test]
fn cannot_inject_tree_artifact_child_individually() {
    let f = Fixture::new();
    let tree_artifact = create_tree_artifact_with_generating_action(&f.output_root, "foo/bar");
    let child = Artifact::create_tree_output_str(&tree_artifact, "child");

    let store = f.create_store(std::slice::from_ref(&tree_artifact));
    store.prepare_for_action_execution();

    let child_value = FileArtifactValue::create_for_remote_file(vec![1, 2, 3], 5, 1);

    assert_throws("IllegalArgumentException", || {
        store.inject_file(&artifact_input(&child), child_value)
    });
    assert!(store.get_all_artifact_data().is_empty());
    assert!(store.get_all_tree_artifact_data().is_empty());
    assert!(f.chmod_calls().is_empty());
}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/ActionOutputMetadataStoreTest.java:323 ActionOutputMetadataStoreTest.canInjectTemplateExpansionOutput
#[test]
fn can_inject_template_expansion_output() {
    let f = Fixture::new();
    let tree_artifact = create_tree_artifact_with_generating_action(&f.output_root, "foo/bar");
    // ActionsTestUtil.NULL_TEMPLATE_EXPANSION_ARTIFACT_OWNER:
    // ActionTemplateExpansionValue.key(NULL_ARTIFACT_OWNER, /* actionIndex= */ 0).
    let output = Artifact::create_template_expansion_output(
        &tree_artifact,
        PathFragment::create("output"),
        ActionLookupKeyRef::new(ActionTemplateExpansionKey::of(null_artifact_owner(), 0)),
    );

    let store = f.create_store(std::slice::from_ref(&tree_artifact));
    store.prepare_for_action_execution();

    let value = FileArtifactValue::create_for_remote_file(vec![1, 2, 3], 5, 1);
    store.inject_file(&artifact_input(&output), value.clone());

    assert_eq!(
        store.get_all_artifact_data(),
        IndexMap::from([(output.clone(), value)])
    );
    assert!(store.get_all_tree_artifact_data().is_empty());
    assert!(f.chmod_calls().is_empty());
}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/ActionOutputMetadataStoreTest.java:342 ActionOutputMetadataStoreTest.injectRemoteTreeArtifactMetadata
#[test]
fn inject_remote_tree_artifact_metadata() {
    let f = Fixture::new();
    let tree_artifact = create_tree_artifact_with_generating_action(&f.output_root, "dir");
    let store = f.create_store(std::slice::from_ref(&tree_artifact));
    store.prepare_for_action_execution();

    let tree = TreeArtifactValue::build(
        vec![
            (
                artifact_input(&Artifact::create_tree_output_str(&tree_artifact, "foo")),
                FileArtifactValue::create_for_remote_file(vec![1, 2, 3], 5, 1),
            ),
            (
                artifact_input(&Artifact::create_tree_output_str(&tree_artifact, "bar")),
                FileArtifactValue::create_for_remote_file(vec![4, 5, 6], 10, 1),
            ),
        ],
        None,
        None,
    );

    store.inject_tree(&artifact_input(&tree_artifact), tree.clone());

    let value = get_output_metadata(&store, &tree_artifact).unwrap();
    assert!(value.is_some());
    assert_eq!(value.unwrap().get_digest(), tree.get_digest());
    assert_tree_eq(
        store
            .get_all_tree_artifact_data()
            .get(&tree_artifact)
            .unwrap(),
        &tree,
    );
    assert!(f.chmod_calls().is_empty());

    assert_tree_eq(&get_tree_artifact_value(&store, &tree_artifact), &tree);

    // Make sure that all children are transferred properly into the ActionExecutionValue. If any
    // child is missing, getExistingFileArtifactValue will throw.
    // ActionExecutionValue.create(store, /* richArtifactData= */ null, new NullAction())
    let action_execution_value = ActionExecutionValue::create(
        store.get_all_artifact_data(),
        store.get_all_tree_artifact_data(),
        None,
    );
    for (child, _) in &tree.child_values {
        let child = blaze_exec::actions::action_adapters::as_blaze_artifact(child).unwrap();
        // getExistingFileArtifactValue: checkNotNull (NullPointerException when missing).
        assert!(
            action_execution_value
                .get_existing_file_artifact_value(child)
                .is_some(),
            "NullPointerException: {child}"
        );
    }
}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/ActionOutputMetadataStoreTest.java:534 ActionOutputMetadataStoreTest.getMetadataFromFilesetMapping
#[test]
#[ignore = "ActionInputMap.putFileset (actions/ActionInputMap.java:388) and the filesetMapping \
            lookup of ActionInputMetadataProvider.getInputMetadataChecked \
            (skyframe/ActionInputMetadataProvider.java:84) are not ported in ActionInputMetadata, \
            and blaze_exec's FilesetOutputSymlink has no metadata field \
            (actions/FilesetOutputSymlink.java)"]
#[allow(unreachable_code, unused_variables, clippy::diverging_sub_expression)]
fn get_metadata_from_fileset_mapping() {
    let f = Fixture::new();
    let source_artifact = create_artifact_with_root_relative_path(&f.source_root, "src.txt");
    let metadata = FileArtifactValue::create_for_normal_file(Some(vec![1, 2, 3, 4]), None, 10);
    // FilesetOutputSymlink symlink =
    //     new FilesetOutputSymlink(PathFragment.create("file"), sourceArtifact, metadata);
    let artifact = Artifact::create_special_artifact(
        f.output_root.clone(),
        f.output_root.get_exec_path().get_relative_str("foo/bar"),
        null_artifact_owner(),
        SpecialArtifactType::FILESET,
    );
    let action_input_map = ActionInputMetadata::with_providers(None);
    // actionInputMap.putFileset(artifact, FilesetOutputTree.create(ImmutableList.of(symlink)));
    blaze_util::unported!("ActionInputMap.putFileset");
    let input_metadata_provider = action_input_map.action_input_map_only();

    // isSameInstanceAs(metadata)
    assert_eq!(
        input_metadata_provider
            .get_input_metadata(&artifact_input(&source_artifact))
            .unwrap(),
        Some(metadata)
    );
    assert_eq!(
        input_metadata_provider
            .get_input_metadata(&ActionInputRef::new(BasicActionInput::from_path(
                "does_not_exist"
            )))
            .unwrap(),
        None
    );
    assert!(f.chmod_calls().is_empty());
}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/ActionOutputMetadataStoreTest.java:554 ActionOutputMetadataStoreTest.omitRegularArtifact
#[test]
fn omit_regular_artifact() {
    let f = Fixture::new();
    let omitted = create_artifact_with_root_relative_path(&f.output_root, "omitted");
    let consumed = create_artifact_with_root_relative_path(&f.output_root, "consumed");
    let store = f.create_store(&[omitted.clone(), consumed.clone()]);

    store.prepare_for_action_execution();
    store.mark_omitted(&artifact_input(&omitted));

    assert!(store.artifact_omitted(&artifact_input(&omitted)));
    assert!(!store.artifact_omitted(&artifact_input(&consumed)));
    assert!(store.get_all_tree_artifact_data().is_empty());
    assert!(f.chmod_calls().is_empty());
}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/ActionOutputMetadataStoreTest.java:574 ActionOutputMetadataStoreTest.omitTreeArtifact
#[test]
fn omit_tree_artifact() {
    let f = Fixture::new();
    let omitted_tree = create_tree_artifact_with_generating_action(&f.output_root, "omitted");
    let consumed_tree = create_tree_artifact_with_generating_action(&f.output_root, "consumed");
    let store = f.create_store(&[omitted_tree.clone(), consumed_tree.clone()]);

    store.prepare_for_action_execution();
    store.mark_omitted(&artifact_input(&omitted_tree));
    store.mark_omitted(&artifact_input(&omitted_tree)); // Marking a tree artifact as omitted twice is tolerated.

    assert!(store.artifact_omitted(&artifact_input(&omitted_tree)));
    assert!(!store.artifact_omitted(&artifact_input(&consumed_tree)));
    assert!(store.get_all_artifact_data().is_empty());
    assert!(f.chmod_calls().is_empty());
}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/ActionOutputMetadataStoreTest.java:595 ActionOutputMetadataStoreTest.outputArtifactNotPreviouslyInjectedInExecutionMode
#[test]
fn output_artifact_not_previously_injected_in_execution_mode() {
    let f = Fixture::new();
    let output = create_artifact_with_root_relative_path(&f.output_root, "dir/file.out");
    let output_path = scratch_file(&output.get_path(), &["contents"]);
    let store = f.create_store(std::slice::from_ref(&output));
    store.prepare_for_action_execution();

    let metadata = get_output_metadata(&store, &output).unwrap().unwrap();

    assert_eq!(
        metadata.get_digest(),
        Some(&output_path.get_digest().unwrap()[..])
    );
    assert_eq!(
        store.get_all_artifact_data(),
        IndexMap::from([(output.clone(), metadata)])
    );
    assert!(store.get_all_tree_artifact_data().is_empty());
    assert_eq!(
        f.chmod_calls(),
        HashMap::from([(output_path.as_fragment().clone(), 0o555)])
    );
}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/ActionOutputMetadataStoreTest.java:612 ActionOutputMetadataStoreTest.outputArtifactNotPreviouslyInjectedInExecutionMode_writablePermissions
#[test]
fn output_artifact_not_previously_injected_in_execution_mode_writable_permissions() {
    let f = Fixture::new();
    let output = create_artifact_with_root_relative_path(&f.output_root, "dir/file.out");
    let output_path = scratch_file(&output.get_path(), &["contents"]);
    // ArtifactPathResolver.IDENTITY: no action file system.
    let store = ActionOutputMetadataStore::create(
        /* archived_tree_artifacts_enabled= */ false,
        OutputPermissions::WRITABLE,
        /* outputs= */ vec![output.clone()],
    );
    store.prepare_for_action_execution();

    let metadata = get_output_metadata(&store, &output).unwrap().unwrap();

    assert_eq!(
        metadata.get_digest(),
        Some(&output_path.get_digest().unwrap()[..])
    );
    assert_eq!(
        store.get_all_artifact_data(),
        IndexMap::from([(output.clone(), metadata)])
    );
    assert!(store.get_all_tree_artifact_data().is_empty());
    // Permissions preserved in store, so chmod calls should be empty.
    assert_eq!(
        f.chmod_calls(),
        HashMap::from([(output_path.as_fragment().clone(), 0o755)])
    );
}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/ActionOutputMetadataStoreTest.java:671 ActionOutputMetadataStoreTest.enteringExecutionModeClearsCachedOutputs
#[test]
fn entering_execution_mode_clears_cached_outputs() {
    let f = Fixture::new();
    let artifact = create_artifact_with_root_relative_path(&f.output_root, "output");
    let tree_artifact = create_tree_artifact_with_generating_action(&f.output_root, "tree");
    let child = Artifact::create_tree_output_str(&tree_artifact, "child");
    scratch_file(&artifact.get_path(), &["1"]);
    scratch_file(&child.get_path(), &["1"]);
    let store = f.create_store(&[artifact.clone(), tree_artifact.clone()]);

    let artifact_metadata1 = get_output_metadata(&store, &artifact).unwrap();
    let tree_artifact_metadata1 = get_output_metadata(&store, &tree_artifact).unwrap();
    assert!(artifact_metadata1.is_some());
    assert!(artifact_metadata1.is_some());
    assert_eq!(
        store.get_all_artifact_data().keys().collect::<Vec<_>>(),
        vec![&artifact]
    );
    assert_eq!(
        store
            .get_all_tree_artifact_data()
            .keys()
            .collect::<Vec<_>>(),
        vec![&tree_artifact]
    );

    // Entering execution mode should clear the cached outputs.
    store.prepare_for_action_execution();
    assert!(store.get_all_artifact_data().is_empty());
    assert!(store.get_all_tree_artifact_data().is_empty());

    // Updated metadata should be read from the filesystem.
    scratch_overwrite_file(&artifact.get_path(), &["2"]);
    scratch_overwrite_file(&child.get_path(), &["2"]);
    let artifact_metadata2 = get_output_metadata(&store, &artifact).unwrap();
    let tree_artifact_metadata2 = get_output_metadata(&store, &tree_artifact).unwrap();
    assert!(artifact_metadata2.is_some());
    assert!(tree_artifact_metadata2.is_some());
    assert_ne!(artifact_metadata2, artifact_metadata1);
    assert_ne!(tree_artifact_metadata2, tree_artifact_metadata1);
}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/ActionOutputMetadataStoreTest.java:707 ActionOutputMetadataStoreTest.cannotEnterExecutionModeTwice
#[test]
fn cannot_enter_execution_mode_twice() {
    let f = Fixture::new();
    let store = f.create_store(/* outputs= */ &[]);
    store.prepare_for_action_execution();
    assert_throws("IllegalStateException", || {
        store.prepare_for_action_execution()
    });
}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/ActionOutputMetadataStoreTest.java:714 ActionOutputMetadataStoreTest.fileArtifactValueFromArtifactCompatibleWithGetMetadata_changed
#[test]
#[ignore = "FileArtifactValue.couldBeModifiedSince (actions/FileArtifactValue.java:235) is not \
            ported in blaze_exec's FileArtifactValue"]
#[allow(unreachable_code, unused_variables, clippy::diverging_sub_expression)]
fn file_artifact_value_from_artifact_compatible_with_get_metadata_changed() {
    let f = Fixture::new();
    let artifact = create_artifact_with_root_relative_path(&f.output_root, "output");
    scratch_file(&artifact.get_path(), &["1"]);
    let store = f.create_store(std::slice::from_ref(&artifact));

    let get_metadata_result = get_output_metadata(&store, &artifact).unwrap();
    assert!(get_metadata_result.is_some());

    scratch_overwrite_file(&artifact.get_path(), &["2"]);
    // ActionOutputMetadataStore.fileArtifactValueFromArtifact(artifact, /* statNoFollow= */ null,
    // SyscallCache.NO_CACHE, /* tsgm= */ null): the IDENTITY resolver.
    let file_artifact_value_from_artifact_result =
        file_artifact_value_from_artifact(&artifact, None, None)
            .unwrap()
            .file_artifact_value;

    // assertThat(fileArtifactValueFromArtifactResult.couldBeModifiedSince(getMetadataResult))
    //     .isTrue();
    let could_be_modified_since: bool =
        blaze_util::unported!("FileArtifactValue.couldBeModifiedSince");
    assert!(could_be_modified_since);
}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/ActionOutputMetadataStoreTest.java:735 ActionOutputMetadataStoreTest.fileArtifactValueFromArtifactCompatibleWithGetMetadata_notChanged
#[test]
#[ignore = "FileArtifactValue.couldBeModifiedSince (actions/FileArtifactValue.java:235) is not \
            ported in blaze_exec's FileArtifactValue"]
#[allow(unreachable_code, unused_variables, clippy::diverging_sub_expression)]
fn file_artifact_value_from_artifact_compatible_with_get_metadata_not_changed() {
    let f = Fixture::new();
    let artifact = create_artifact_with_root_relative_path(&f.output_root, "output");
    scratch_file(&artifact.get_path(), &["contents"]);
    let store = f.create_store(std::slice::from_ref(&artifact));

    let get_metadata_result = get_output_metadata(&store, &artifact).unwrap();
    assert!(get_metadata_result.is_some());

    // ActionOutputMetadataStore.fileArtifactValueFromArtifact(artifact, /* statNoFollow= */ null,
    // SyscallCache.NO_CACHE, /* tsgm= */ null): the IDENTITY resolver.
    let file_artifact_value_from_artifact_result =
        file_artifact_value_from_artifact(&artifact, None, None)
            .unwrap()
            .file_artifact_value;

    // assertThat(fileArtifactValueFromArtifactResult.couldBeModifiedSince(getMetadataResult))
    //     .isFalse();
    let could_be_modified_since: bool =
        blaze_util::unported!("FileArtifactValue.couldBeModifiedSince");
    assert!(!could_be_modified_since);
}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/ActionOutputMetadataStoreTest.java:755 ActionOutputMetadataStoreTest.fileArtifactValueForSymlink_readFromCache
#[test]
#[ignore = "the digest cache of DigestUtils (vfs/DigestUtils.java:84 configureCache, :109 \
            getCacheStats) is not ported"]
#[allow(unreachable_code, unused_variables, clippy::diverging_sub_expression)]
fn file_artifact_value_for_symlink_read_from_cache() {
    // DigestUtils.configureCache(1);
    blaze_util::unported!("DigestUtils.configureCache");
    let f = Fixture::new();
    let target = create_artifact_with_root_relative_path(&f.output_root, "target");
    scratch_file(&target.get_path(), &["contents"]);
    let symlink = create_artifact_with_root_relative_path(&f.output_root, "symlink");
    symlink
        .get_path()
        .create_symbolic_link(&target.get_path())
        .unwrap();
    let store = f.create_store(&[target.clone(), symlink.clone()]);
    let target_metadata = get_output_metadata(&store, &target).unwrap();
    let hit_count: u64 = blaze_util::unported!("DigestUtils.getCacheStats");
    assert_eq!(hit_count, 0);

    let symlink_metadata = get_output_metadata(&store, &symlink).unwrap();

    assert_eq!(symlink_metadata, target_metadata);
    let hit_count: u64 = blaze_util::unported!("DigestUtils.getCacheStats");
    assert_eq!(hit_count, 1);
}

/// Not a Java test: no `ActionOutputMetadataStoreTest` method covers `getAllTreeArtifactData`
/// with a missing tree. Java `ActionOutputMetadataStore.getAllTreeArtifactData`
/// (skyframe/ActionOutputMetadataStore.java:144) returns `ImmutableMap.copyOf(treeArtifactData)`,
/// which "may contain copies of `TreeArtifactValue.MISSING_TREE_ARTIFACT`": the marker
/// `getTreeArtifactValue` stores (:241) when `constructTreeArtifactValueFromFilesystem` finds no
/// directory (:258). `ActionExecutionValue.create` (skyframe/ActionExecutionValue.java:81) calls
/// `getChildValues()` on each tree value, which the marker answers with
/// `UnsupportedOperationException` (skyframe/TreeArtifactValue.java:445).
#[test]
fn get_all_tree_artifact_data_contains_missing_tree_artifact_marker() {
    let f = Fixture::new();
    let tree_artifact = create_tree_artifact_with_generating_action(&f.output_root, "foo/bar");
    let store = f.create_store(std::slice::from_ref(&tree_artifact));
    store.prepare_for_action_execution();

    let e = get_output_metadata(&store, &tree_artifact).unwrap_err();
    assert_eq!(e.kind(), io::ErrorKind::NotFound, "{e}");

    let all_tree_artifact_data = store.get_all_tree_artifact_data();
    assert_eq!(
        all_tree_artifact_data.keys().cloned().collect::<Vec<_>>(),
        vec![tree_artifact.clone()]
    );
    assert!(crate::action_execution_value::is_missing_tree_artifact(
        all_tree_artifact_data.get(&tree_artifact).unwrap()
    ));

    assert_throws(
        "UnsupportedOperationException: MISSING_TREE_ARTIFACT",
        || {
            ActionExecutionValue::create(
                store.get_all_artifact_data(),
                store.get_all_tree_artifact_data(),
                None,
            );
        },
    );
}
