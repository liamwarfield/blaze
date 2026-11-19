// Options and protos are filled in field by field, as the Java tests set them.
#![allow(clippy::field_reassign_with_default)]
//! Port of
//! `src/test/java/com/google/devtools/build/lib/remote/merkletree/MerkleTreeComputerTest.java`.
//!
//! The Java test runs on an `InMemoryFileSystem` rooted at `/execroot/_main`; here the exec root
//! is `execroot/_main` in a temporary directory of the host file system. Artifacts are real
//! `blaze_actions` artifacts (the port of `ActionsTestUtil`'s factories); the runfiles tree is a
//! fake with the mapping `Runfiles` computes in the Java test.

use std::any::Any;
use std::collections::{BTreeMap, HashSet};
use std::fmt;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::io::{self, Write};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use blaze_actions::{
    ActionLookupData, ActionLookupKey, ActionLookupKeyRef, Artifact, ArtifactOwner, ArtifactRoot,
    RootType, SpecialArtifactType,
};
use blaze_cmdline::Label;
use blaze_collect::{NestedSet, NestedSetBuilder, Order};
use blaze_exec::actions::action_adapters::artifact_input;
use blaze_exec::actions::{
    ActionInputRef, BasicVirtualActionInput, ExecException, FakeOwner, FileArtifactValue,
    FilesetOutputTree, InputMapping, InputMetadataProvider, PathMapper, ResourceSet,
    RunfilesArtifactValue, RunfilesTree, SimpleSpawn, Spawn, SymlinkEntry, TreeArtifactValue,
    VirtualActionInput,
};
use blaze_exec::exec::action_execution_context::SpawnCacheError;
use blaze_exec::exec::spawn_input_expander::SpawnInputExpander;
use blaze_exec::exec::{ProgressStatus, SpawnExecutionContext};
use blaze_exec::util::FileOutErr;
use blaze_protos::JavaMessage;
use blaze_remote::common::remote_path_resolver::create_default;
use blaze_remote::common::{RemoteActionExecutionContext, RemotePathResolver, errors};
use blaze_remote::merkletree::{
    BlobPolicy, MerkleTree, MerkleTreeBlob, MerkleTreeComputer, MerkleTreeRootOnly,
    MerkleTreeUploadable, MerkleTreeUploader,
};
use blaze_remote::reapi::{
    Digest, Directory, DirectoryNode, FileNode, NodeProperties, NodeProperty,
};
use blaze_remote::util::DigestUtil;
use blaze_remote::util::tracing_metadata_utils::set_tool_version;
use blaze_vfs::{DigestHashFunction, Path, PathFragment, Root, UnixFileSystem};
use indexmap::IndexMap;
use parking_lot::Mutex;

const WORKSPACE_NAME: &str = "_main";

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
        Some(Label::parse_canonical_unchecked("//null/action:owner"))
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

// Java: ActionsTestUtil.createArtifactWithExecPath
fn create_artifact_with_exec_path(root: &ArtifactRoot, exec_path: PathFragment) -> Artifact {
    if root.is_source_root() {
        Artifact::new_source_artifact(root.clone(), exec_path, ArtifactOwner::NullOwner)
    } else {
        Artifact::create_derived_artifact(root.clone(), exec_path, null_artifact_owner())
    }
}

// Java: ActionsTestUtil.createArtifact(ArtifactRoot root, String path)
fn create_artifact(root: &ArtifactRoot, path: &str) -> Artifact {
    create_artifact_with_exec_path(root, root.get_exec_path().get_relative_str(path))
}

// Java: ActionsTestUtil.createRunfilesArtifact
fn create_runfiles_artifact(root: &ArtifactRoot, exec_path: &str) -> Artifact {
    Artifact::create_special_artifact(
        root.clone(),
        PathFragment::create(exec_path),
        null_artifact_owner(),
        SpecialArtifactType::RUNFILES,
    )
}

// Java: ActionsTestUtil.createTreeArtifactWithGeneratingAction(ArtifactRoot root, String rootRelativePath)
fn create_tree_artifact_with_generating_action(
    root: &ArtifactRoot,
    root_relative_path: &str,
) -> Artifact {
    let owner = null_artifact_owner();
    let tree_artifact = Artifact::create_special_artifact(
        root.clone(),
        root.get_exec_path().get_relative_str(root_relative_path),
        owner.clone(),
        SpecialArtifactType::TREE,
    );
    // Java: ActionsTestUtil.NULL_ACTION_LOOKUP_DATA
    tree_artifact.set_generating_action_key(ActionLookupData::create(owner, 0));
    tree_artifact
}

// Java: FileSystemUtils.writeContentAsLatin1
fn write_content_as_latin1(path: &Path, content: &str) {
    let mut out = path.get_output_stream().unwrap();
    out.write_all(content.as_bytes()).unwrap();
    out.flush().unwrap();
}

/// `AnalysisTestUtil.createRunfilesTree(execPath, runfiles)` for
/// `new Runfiles.Builder(workspaceName).addArtifact(artifact).build()`: every artifact is at its
/// canonical location `<workspace name>/<runfiles path>`.
#[derive(Debug)]
struct TestRunfilesTree {
    exec_path: PathFragment,
    artifacts: Vec<Artifact>,
}

impl RunfilesTree for TestRunfilesTree {
    fn get_exec_path(&self) -> PathFragment {
        self.exec_path.clone()
    }
    fn get_mapping(&self) -> Vec<(PathFragment, Option<ActionInputRef>)> {
        self.artifacts
            .iter()
            .map(|artifact| {
                (
                    PathFragment::create(WORKSPACE_NAME)
                        .get_relative(&artifact.get_runfiles_path()),
                    Some(artifact_input(artifact)),
                )
            })
            .collect()
    }
    fn get_workspace_name(&self) -> String {
        WORKSPACE_NAME.to_string()
    }
    fn get_artifacts_at_canonical_locations_for_logging(&self) -> NestedSet<ActionInputRef> {
        let mut builder = NestedSetBuilder::stable_order();
        for artifact in &self.artifacts {
            builder.add(artifact_input(artifact));
        }
        builder.build()
    }
    fn get_symlinks_for_logging(&self) -> NestedSet<SymlinkEntry> {
        NestedSetBuilder::empty_set(Order::STABLE_ORDER)
    }
    fn get_root_symlinks_for_logging(&self) -> NestedSet<SymlinkEntry> {
        NestedSetBuilder::empty_set(Order::STABLE_ORDER)
    }
    fn get_empty_filenames_for_logging(&self) -> Vec<PathFragment> {
        Vec::new()
    }
    fn get_repo_mapping_manifest_for_logging(&self) -> Option<ActionInputRef> {
        None
    }
}

/// `FakeActionInputFileCache`.
// Java: com/google/devtools/build/lib/exec/util/FakeActionInputFileCache.java
#[derive(Default)]
struct FakeActionInputFileCache {
    inputs: Mutex<Vec<(ActionInputRef, FileArtifactValue)>>,
    tree_artifacts: Mutex<Vec<(ActionInputRef, Arc<TreeArtifactValue>)>>,
    runfiles_inputs: Mutex<Vec<(ActionInputRef, Arc<RunfilesArtifactValue>)>>,
}

impl FakeActionInputFileCache {
    fn put(&self, artifact: &Artifact, metadata: FileArtifactValue) {
        self.inputs
            .lock()
            .push((artifact_input(artifact), metadata));
    }

    fn put_tree_artifact(&self, action_input: &Artifact, tree_artifact_value: TreeArtifactValue) {
        self.tree_artifacts
            .lock()
            .push((artifact_input(action_input), Arc::new(tree_artifact_value)));
    }

    fn put_runfiles_tree(
        &self,
        runfiles_tree_artifact: &Artifact,
        runfiles_tree: TestRunfilesTree,
    ) {
        let runfiles_artifact_value = RunfilesArtifactValue::new(
            Arc::new(runfiles_tree),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
        );
        self.runfiles_inputs.lock().push((
            artifact_input(runfiles_tree_artifact),
            Arc::new(runfiles_artifact_value),
        ));
    }
}

impl InputMetadataProvider for FakeActionInputFileCache {
    fn get_input_metadata(&self, input: &ActionInputRef) -> io::Result<Option<FileArtifactValue>> {
        let mut result = None;
        if input
            .as_artifact()
            .is_some_and(|artifact| artifact.is_child_of_declared_directory())
        {
            for (tree, value) in self.tree_artifacts.lock().iter() {
                if input.get_exec_path().starts_with(tree.get_exec_path()) {
                    result = value
                        .child_values
                        .iter()
                        .find(|(child, _)| child == input)
                        .map(|(_, metadata)| metadata.clone());
                    break;
                }
            }
        } else {
            result = self
                .inputs
                .lock()
                .iter()
                .find(|(artifact, _)| artifact == input)
                .map(|(_, metadata)| metadata.clone());
        }

        if result == Some(FileArtifactValue::MissingFileMarker) {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("File '{}' does not exist", input.get_exec_path_string()),
            ));
        }

        Ok(result)
    }

    fn get_tree_metadata(&self, input: &ActionInputRef) -> Option<Arc<TreeArtifactValue>> {
        self.tree_artifacts
            .lock()
            .iter()
            .find(|(artifact, _)| artifact == input)
            .map(|(_, value)| value.clone())
    }

    fn get_fileset(&self, _input: &ActionInputRef) -> Option<Arc<FilesetOutputTree>> {
        None
    }

    fn get_runfiles_metadata(&self, input: &ActionInputRef) -> Option<Arc<RunfilesArtifactValue>> {
        self.runfiles_inputs
            .lock()
            .iter()
            .find(|(artifact, _)| artifact == input)
            .map(|(_, value)| value.clone())
    }

    fn get_input(&self, exec_path: &PathFragment) -> Option<ActionInputRef> {
        for (i, _) in self.inputs.lock().iter() {
            if i.get_exec_path() == exec_path {
                return Some(i.clone());
            }
        }

        for (tree, value) in self.tree_artifacts.lock().iter() {
            if !exec_path.starts_with(tree.get_exec_path()) {
                continue;
            }

            for (c, _) in &value.child_values {
                if c.get_exec_path() == exec_path {
                    return Some(c.clone());
                }
            }
        }

        None
    }
}

/// `DelegatingPairInputMetadataProvider`.
// Java: com/google/devtools/build/lib/actions/DelegatingPairInputMetadataProvider.java
struct DelegatingPairInputMetadataProvider<'a> {
    primary: &'a dyn InputMetadataProvider,
    secondary: &'a dyn InputMetadataProvider,
}

impl InputMetadataProvider for DelegatingPairInputMetadataProvider<'_> {
    fn get_input_metadata(&self, input: &ActionInputRef) -> io::Result<Option<FileArtifactValue>> {
        let metadata = self.primary.get_input_metadata(input)?;
        match metadata {
            Some(metadata) if metadata != FileArtifactValue::MissingFileMarker => {
                Ok(Some(metadata))
            }
            _ => self.secondary.get_input_metadata(input),
        }
    }
    fn get_tree_metadata(&self, input: &ActionInputRef) -> Option<Arc<TreeArtifactValue>> {
        self.primary
            .get_tree_metadata(input)
            .or_else(|| self.secondary.get_tree_metadata(input))
    }
    fn get_fileset(&self, input: &ActionInputRef) -> Option<Arc<FilesetOutputTree>> {
        self.primary
            .get_fileset(input)
            .or_else(|| self.secondary.get_fileset(input))
    }
    fn get_runfiles_metadata(&self, input: &ActionInputRef) -> Option<Arc<RunfilesArtifactValue>> {
        self.primary
            .get_runfiles_metadata(input)
            .or_else(|| self.secondary.get_runfiles_metadata(input))
    }
    fn get_input(&self, exec_path: &PathFragment) -> Option<ActionInputRef> {
        self.primary
            .get_input(exec_path)
            .or_else(|| self.secondary.get_input(exec_path))
    }
}

/// The anonymous `InputMetadataProvider` of
/// `testSubtreeComputationCancelled_subsequentReusingCallNotAffected`.
///
/// PORT: the Java provider blocks in `Thread.sleep` when the tree file's metadata is requested
/// for the first time and the test interrupts the build thread. Blaze has no thread interruption:
/// an interrupted build is an `io::ErrorKind::Interrupted` error (see `blaze_remote::runtime`),
/// which this provider returns at the point where the Java one is interrupted.
struct InterruptedMetadataProvider {
    tree_file_artifact: ActionInputRef,
    tree_file_metadata_accessed: AtomicBool,
}

impl InputMetadataProvider for InterruptedMetadataProvider {
    fn get_input_metadata(&self, input: &ActionInputRef) -> io::Result<Option<FileArtifactValue>> {
        if input != &self.tree_file_artifact
            || self.tree_file_metadata_accessed.load(Ordering::SeqCst)
        {
            return Ok(None);
        }
        self.tree_file_metadata_accessed
            .store(true, Ordering::SeqCst);
        Err(errors::interrupted())
    }
    fn get_tree_metadata(&self, _input: &ActionInputRef) -> Option<Arc<TreeArtifactValue>> {
        None
    }
    fn get_fileset(&self, _input: &ActionInputRef) -> Option<Arc<FilesetOutputTree>> {
        None
    }
    fn get_runfiles_metadata(&self, _input: &ActionInputRef) -> Option<Arc<RunfilesArtifactValue>> {
        None
    }
    fn get_input(&self, _exec_path: &PathFragment) -> Option<ActionInputRef> {
        None
    }
}

/// `FakeSpawnExecutionContext`.
// Java: com/google/devtools/build/lib/remote/util/FakeSpawnExecutionContext.java
struct FakeSpawnExecutionContext<'a> {
    spawn: &'a dyn Spawn,
    input_metadata_provider: &'a dyn InputMetadataProvider,
    out_err: FileOutErr,
    client_env: IndexMap<String, String>,
    digest: Mutex<Option<blaze_protos::tools::protos::Digest>>,
}

impl SpawnExecutionContext for FakeSpawnExecutionContext<'_> {
    fn get_id(&self) -> i32 {
        0
    }
    fn set_digest(&self, digest: blaze_protos::tools::protos::Digest) {
        *self.digest.lock() = Some(digest);
    }
    fn get_digest(&self) -> Option<blaze_protos::tools::protos::Digest> {
        self.digest.lock().clone()
    }
    fn prefetch_inputs_and_wait(&self) -> Result<(), ExecException> {
        panic!("UnsupportedOperationException")
    }
    fn get_input_metadata_provider(&self) -> &dyn InputMetadataProvider {
        self.input_metadata_provider
    }
    fn lock_output_files(
        &self,
        _exit_code: i32,
        _error_message: &str,
        _out_err: &FileOutErr,
    ) -> Result<(), ExecException> {
        Ok(())
    }
    fn speculating(&self) -> bool {
        false
    }
    fn get_timeout(&self) -> Duration {
        Duration::ZERO
    }
    fn get_file_out_err(&self) -> &FileOutErr {
        &self.out_err
    }
    fn get_input_mapping(
        &self,
        base_directory: &PathFragment,
        _will_access_repeatedly: bool,
    ) -> Arc<InputMapping> {
        Arc::new(SpawnInputExpander::new().get_input_mapping(
            self.spawn,
            self.input_metadata_provider,
            base_directory,
        ))
    }
    fn report(&self, _progress: ProgressStatus) {
        // Intentionally left empty.
    }
    fn get_client_env(&self) -> &IndexMap<String, String> {
        &self.client_env
    }
}

/// A `MerkleTreeUploader` that only counts calls to `ensure_inputs_present`.
#[derive(Default)]
struct CountingMerkleTreeUploader {
    ensure_inputs_present_count: AtomicI32,
}

#[async_trait]
impl MerkleTreeUploader for CountingMerkleTreeUploader {
    async fn upload_blob(
        &self,
        _context: &RemoteActionExecutionContext<'_>,
        _digest: &Digest,
        _data: bytes::Bytes,
        _force: bool,
    ) -> io::Result<()> {
        Ok(())
    }

    async fn upload_file(
        &self,
        _context: &RemoteActionExecutionContext<'_>,
        _remote_path_resolver: Option<&dyn RemotePathResolver>,
        _digest: &Digest,
        _path: &Path,
        _force: bool,
    ) -> io::Result<()> {
        Ok(())
    }

    async fn upload_virtual_action_input(
        &self,
        _context: &RemoteActionExecutionContext<'_>,
        _digest: &Digest,
        _virtual_action_input: &dyn VirtualActionInput,
        _force: bool,
    ) -> io::Result<()> {
        Ok(())
    }

    async fn ensure_inputs_present(
        &self,
        _context: &RemoteActionExecutionContext<'_>,
        _merkle_tree: &MerkleTreeUploadable,
        _force: bool,
        _remote_path_resolver: Option<&dyn RemotePathResolver>,
    ) -> io::Result<()> {
        self.ensure_inputs_present_count
            .fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

/// The fields of the Java test class and its `setUp`.
struct MerkleTreeComputerTest {
    _tmp: tempfile::TempDir,
    exec_root: Path,
    artifact_root: ArtifactRoot,
}

impl MerkleTreeComputerTest {
    // Java: MerkleTreeComputerTest.setUp
    fn set_up() -> MerkleTreeComputerTest {
        // `TracingMetadataUtils.buildMetadata` reads the version of the running binary.
        set_tool_version("test");
        let tmp = tempfile::tempdir().unwrap();
        let fs = UnixFileSystem::create(DigestHashFunction::Sha256);
        let exec_root = Path::create(PathFragment::create(tmp.path().to_str().unwrap()), &fs)
            .get_relative_str("execroot/_main");
        exec_root.create_directory_and_parents().unwrap();
        let artifact_root =
            ArtifactRoot::as_derived_root(&exec_root, RootType::OUTPUT, &["outputs"]);
        artifact_root
            .get_root()
            .as_path()
            .unwrap()
            .create_directory_and_parents()
            .unwrap();
        MerkleTreeComputerTest {
            _tmp: tmp,
            exec_root,
            artifact_root,
        }
    }

    // Java: MerkleTreeComputerTest.createMerkleTreeComputer
    fn create_merkle_tree_computer(
        &self,
        uploader: Option<Arc<dyn MerkleTreeUploader>>,
    ) -> MerkleTreeComputer {
        MerkleTreeComputer::new(
            DigestUtil::new(DigestHashFunction::Sha256),
            uploader,
            "buildRequestId",
            "commandId",
            WORKSPACE_NAME,
            // PORT: the exec root stands for `FakeSpawnExecutionContext.getPathResolver()`
            // (`ArtifactPathResolver.forExecRoot(execRoot)`).
            self.exec_root.clone(),
        )
    }

    // Java: MerkleTreeComputerTest.createSpawnExecutionContext
    fn create_spawn_execution_context<'a>(
        &self,
        spawn: &'a dyn Spawn,
        input_metadata_provider: &'a dyn InputMetadataProvider,
    ) -> FakeSpawnExecutionContext<'a> {
        FakeSpawnExecutionContext {
            spawn,
            input_metadata_provider,
            out_err: FileOutErr::new(
                self.exec_root.get_relative_str("stdout"),
                self.exec_root.get_relative_str("stderr"),
            ),
            client_env: IndexMap::new(),
            digest: Mutex::new(None),
        }
    }

    /// `merkleTreeComputer.buildForSpawn(spawn, toolInputs, /* scrubber= */ null,
    /// createSpawnExecutionContext(spawn, inputMetadataProvider),
    /// RemotePathResolver.createDefault(execRoot), blobPolicy)`.
    fn build_for_spawn(
        &self,
        merkle_tree_computer: &MerkleTreeComputer,
        spawn: &dyn Spawn,
        tool_inputs: &HashSet<PathFragment>,
        input_metadata_provider: &dyn InputMetadataProvider,
        blob_policy: BlobPolicy,
    ) -> Result<MerkleTree, SpawnCacheError> {
        let spawn_execution_context =
            self.create_spawn_execution_context(spawn, input_metadata_provider);
        let remote_path_resolver = create_default(self.exec_root.clone());
        merkle_tree_computer.build_for_spawn(
            spawn,
            tool_inputs,
            /* scrubber= */ None,
            &spawn_execution_context,
            &*remote_path_resolver,
            blob_policy,
        )
    }

    /// A tree artifact `outputs/<root_relative_path>` with one child `file` of the given content,
    /// registered in `fake_file_cache` (the first lines of the Java tests with a tree artifact).
    fn create_tree_artifact_with_file(
        &self,
        fake_file_cache: &FakeActionInputFileCache,
        root_relative_path: &str,
        child: &str,
        content: &str,
    ) -> (Artifact, Artifact) {
        let tree_artifact_input =
            create_tree_artifact_with_generating_action(&self.artifact_root, root_relative_path);
        let tree_file_artifact = Artifact::create_tree_output_str(&tree_artifact_input, child);
        tree_file_artifact
            .get_path()
            .get_parent_directory()
            .unwrap()
            .create_directory_and_parents()
            .unwrap();
        write_content_as_latin1(&tree_file_artifact.get_path(), content);
        // Java: TreeArtifactValue.newBuilder(treeArtifactInput).putChild(treeFileArtifact,
        //     FileArtifactValue.createForTesting(treeFileArtifact)).build()
        let tree_artifact_value = TreeArtifactValue::build(
            vec![(
                artifact_input(&tree_file_artifact),
                FileArtifactValue::create_for_testing(&tree_file_artifact.get_path()).unwrap(),
            )],
            None,
            None,
        );
        fake_file_cache.put_tree_artifact(&tree_artifact_input, tree_artifact_value);
        (tree_artifact_input, tree_file_artifact)
    }
}

/// `new SpawnBuilder().withInputs(inputs).withOutputs(outputs).build()`.
// Java: com/google/devtools/build/lib/exec/util/SpawnBuilder.java SpawnBuilder.build
fn build_spawn(inputs: &[&Artifact], outputs: &[&Artifact]) -> SimpleSpawn {
    build_spawn_with_inputs(
        inputs.iter().map(|input| artifact_input(input)).collect(),
        outputs,
    )
}

fn build_spawn_with_inputs(inputs: Vec<ActionInputRef>, outputs: &[&Artifact]) -> SimpleSpawn {
    let owner = FakeOwner::new(
        "Mnemonic",
        Some("progress message"),
        Some(Label::parse_canonical_unchecked("//dummy:label")),
    );
    let mut inputs_builder = NestedSetBuilder::stable_order();
    for input in inputs {
        inputs_builder.add(input);
    }
    SimpleSpawn::new(
        Arc::new(owner),
        Vec::new(),
        IndexMap::new(),
        IndexMap::new(),
        inputs_builder.build(),
        NestedSetBuilder::empty_set(Order::STABLE_ORDER),
        outputs
            .iter()
            .map(|output| artifact_input(output))
            .collect(),
        None,
        Arc::new(ResourceSet::zero()),
        PathMapper::NOOP,
    )
}

#[test]
fn test_subtree_computation_cancelled_subsequent_reusing_call_not_affected() {
    let t = MerkleTreeComputerTest::set_up();
    let fake_file_cache = FakeActionInputFileCache::default();
    let (tree_artifact_input, tree_file_artifact) = t.create_tree_artifact_with_file(
        &fake_file_cache,
        "dir/subdir/tree_artifact",
        "file",
        "file content",
    );
    let spawn = build_spawn(&[&tree_artifact_input], &[]);
    let merkle_tree_computer = t.create_merkle_tree_computer(/* uploader= */ None);

    let interrupted_metadata_provider = InterruptedMetadataProvider {
        tree_file_artifact: artifact_input(&tree_file_artifact),
        tree_file_metadata_accessed: AtomicBool::new(false),
    };
    let delayed_metadata_provider = DelegatingPairInputMetadataProvider {
        primary: &interrupted_metadata_provider,
        secondary: &fake_file_cache,
    };
    // The Merkle subtree for the tree artifact starts to build and is interrupted.
    let captured_throwable = t.build_for_spawn(
        &merkle_tree_computer,
        &spawn,
        &HashSet::new(),
        &delayed_metadata_provider,
        BlobPolicy::Keep,
    );
    assert!(
        interrupted_metadata_provider
            .tree_file_metadata_accessed
            .load(Ordering::SeqCst)
    );
    match captured_throwable {
        Err(SpawnCacheError::Io(e)) => assert!(errors::is_interrupted(&e), "{e}"),
        other => panic!("expected an InterruptedException, got {other:?}"),
    }

    // Expected to succeed despite the subtree computation having been canceled.
    let _unused = t
        .build_for_spawn(
            &merkle_tree_computer,
            &spawn,
            &HashSet::new(),
            &fake_file_cache,
            BlobPolicy::Keep,
        )
        .unwrap();
}

#[test]
#[ignore = "needs in-flight deduplication of sub-tree computations (TaskDeduplicator) and thread interruption"]
fn test_subtree_computation_cancelled_concurrent_reusing_call_not_affected() {
    // The Java test starts a build whose metadata provider blocks while the subtree of a tree
    // artifact is being computed, starts ten more builds of the same spawn and waits until each
    // of them blocks on the first one's subtree computation, interrupts the first build and
    // asserts that the ten others succeed and that "all threads share a single upload of the
    // subtree" (`ensureInputsPresentCount == 1`).
    //
    // Missing in the port: `MerkleTreeComputer` has no in-flight deduplication (every caller
    // computes and uploads a sub-tree that is not in the persistent cache yet, so the ten
    // builds neither block on the first one nor share its upload), and there is no thread
    // interruption that cancels one caller of a shared computation.
    panic!(
        "not ported: MerkleTreeComputer.inFlightComputations (TaskDeduplicator) and Thread.interrupt"
    );
}

#[test]
fn tree_artifact_subtree_reused_by_later_build() {
    let t = MerkleTreeComputerTest::set_up();
    let fake_file_cache = FakeActionInputFileCache::default();
    let (tree_artifact_input, _tree_file_artifact) = t.create_tree_artifact_with_file(
        &fake_file_cache,
        "dir/subdir/reused_tree_artifact",
        "file",
        "reused file content",
    );
    let spawn = build_spawn(&[&tree_artifact_input], &[]);
    let uploader = Arc::new(CountingMerkleTreeUploader::default());

    for _ in 0..2 {
        let _unused = t
            .build_for_spawn(
                &t.create_merkle_tree_computer(Some(
                    uploader.clone() as Arc<dyn MerkleTreeUploader>
                )),
                &spawn,
                &HashSet::new(),
                &fake_file_cache,
                BlobPolicy::Keep,
            )
            .unwrap();
    }

    assert_eq!(
        uploader.ensure_inputs_present_count.load(Ordering::SeqCst),
        1
    );
}

#[test]
fn runfiles_tree_subtree_reused_by_later_build() {
    let t = MerkleTreeComputerTest::set_up();
    let fake_file_cache = FakeActionInputFileCache::default();
    let runfile = create_artifact(&t.artifact_root, "pkg/tool");
    runfile
        .get_path()
        .get_parent_directory()
        .unwrap()
        .create_directory_and_parents()
        .unwrap();
    write_content_as_latin1(&runfile.get_path(), "tool content");
    fake_file_cache.put(
        &runfile,
        FileArtifactValue::create_for_testing(&runfile.get_path()).unwrap(),
    );
    let runfiles_tree_artifact = create_runfiles_artifact(
        &t.artifact_root,
        t.artifact_root
            .get_exec_path()
            .get_relative_str("pkg/tool.runfiles")
            .get_path_string(),
    );
    fake_file_cache.put_runfiles_tree(
        &runfiles_tree_artifact,
        // Java: AnalysisTestUtil.createRunfilesTree(runfilesTreeArtifact.getExecPath(),
        //     new Runfiles.Builder(TestConstants.WORKSPACE_NAME).addArtifact(runfile).build())
        TestRunfilesTree {
            exec_path: runfiles_tree_artifact.get_exec_path().clone(),
            artifacts: vec![runfile.clone()],
        },
    );
    let spawn = build_spawn(&[&runfiles_tree_artifact], &[]);
    let uploader = Arc::new(CountingMerkleTreeUploader::default());

    for _ in 0..2 {
        let _unused = t
            .build_for_spawn(
                &t.create_merkle_tree_computer(Some(
                    uploader.clone() as Arc<dyn MerkleTreeUploader>
                )),
                &spawn,
                &HashSet::new(),
                &fake_file_cache,
                BlobPolicy::Keep,
            )
            .unwrap();
    }

    assert_eq!(
        uploader.ensure_inputs_present_count.load(Ordering::SeqCst),
        1
    );
}

#[test]
fn source_directory_subtree_reused_by_later_build() {
    let t = MerkleTreeComputerTest::set_up();
    let source_root = ArtifactRoot::as_source_root(Root::from_path(t.exec_root.clone()));
    let source_directory =
        create_artifact_with_exec_path(&source_root, PathFragment::create("src_dir"));
    let source_directory_path = source_directory.get_path();
    source_directory_path
        .create_directory_and_parents()
        .unwrap();
    write_content_as_latin1(
        &source_directory_path.get_child("file"),
        "source directory file content",
    );
    let fake_file_cache = FakeActionInputFileCache::default();
    fake_file_cache.put(
        &source_directory,
        FileArtifactValue::create_for_testing(&source_directory_path).unwrap(),
    );
    let spawn = build_spawn(&[&source_directory], &[]);
    let uploader = Arc::new(CountingMerkleTreeUploader::default());

    for _ in 0..2 {
        let _unused = t
            .build_for_spawn(
                &t.create_merkle_tree_computer(Some(
                    uploader.clone() as Arc<dyn MerkleTreeUploader>
                )),
                &spawn,
                &HashSet::new(),
                &fake_file_cache,
                BlobPolicy::Keep,
            )
            .unwrap();
    }

    assert_eq!(
        uploader.ensure_inputs_present_count.load(Ordering::SeqCst),
        1
    );
}

#[test]
#[ignore = "needs a PathMapper other than NOOP (blaze_exec::actions::PathMapper has no custom mappers)"]
fn duplicate_mapped_path_same_digest_succeeds() {
    let t = MerkleTreeComputerTest::set_up();
    // Two artifacts from different configurations that map to the same path with identical
    // content.
    let fastbuild_root = ArtifactRoot::as_derived_root(
        &t.exec_root,
        RootType::OUTPUT,
        &["outputs", "k8-fastbuild", "bin"],
    );
    fastbuild_root
        .get_root()
        .as_path()
        .unwrap()
        .create_directory_and_parents()
        .unwrap();
    let st_root = ArtifactRoot::as_derived_root(
        &t.exec_root,
        RootType::OUTPUT,
        &["outputs", "k8-fastbuild-ST-1234", "bin"],
    );
    st_root
        .get_root()
        .as_path()
        .unwrap()
        .create_directory_and_parents()
        .unwrap();

    let artifact1 = create_artifact(&fastbuild_root, "pkg/foo.h");
    let artifact2 = create_artifact(&st_root, "pkg/foo.h");
    for artifact in [&artifact1, &artifact2] {
        artifact
            .get_path()
            .get_parent_directory()
            .unwrap()
            .create_directory_and_parents()
            .unwrap();
        write_content_as_latin1(&artifact.get_path(), "same content");
    }

    let cache = FakeActionInputFileCache::default();
    let metadata1 = FileArtifactValue::create_for_testing(&artifact1.get_path()).unwrap();
    let metadata2 = FileArtifactValue::create_for_testing(&artifact2.get_path()).unwrap();
    cache.put(&artifact1, metadata1);
    cache.put(&artifact2, metadata2);

    // Java: a `PathMapper` whose `map` strips the config segment, so that both artifacts map to
    // `outputs/cfg/bin/pkg/foo.h`; the spawn is built with
    // `new SpawnBuilder().withInputs(artifact1, artifact2).setPathMapper(strippingMapper)` and
    // `merkleTreeComputer.buildForSpawn(spawn, ImmutableSet.of(), /* scrubber= */ null,
    // createSpawnExecutionContext(spawn, cache), RemotePathResolver.createDefault(execRoot),
    // MerkleTreeComputer.BlobPolicy.KEEP)` should succeed without error — same digest allows the
    // collision.
    //
    // Missing in the port: `blaze_exec::actions::PathMapper` is an enum with only `Noop`, so the
    // two inputs cannot be mapped to the same path.
    panic!("not ported: a stripping PathMapper (PathMapper.map override)");
}

// ---------------------------------------------------------------------------------------------
// PORT: not in MerkleTreeComputerTest.java at the pin. The tests below compare the trees with
// `Directory` messages built by hand as the Remote Execution API defines them (and serialized
// with Java's field order), so that a change of the root digest is caught without a remote
// execution service.
// ---------------------------------------------------------------------------------------------

fn file_node(name: &str, digest: &Digest, is_tool: bool) -> FileNode {
    FileNode {
        name: name.to_string(),
        digest: Some(digest.clone()),
        is_executable: true,
        node_properties: is_tool.then(|| NodeProperties {
            properties: vec![NodeProperty {
                name: "bazel_tool_input".to_string(),
                value: String::new(),
            }],
            mtime: None,
            unix_mode: None,
        }),
    }
}

fn directory_node(name: &str, digest: &Digest) -> DirectoryNode {
    DirectoryNode {
        name: name.to_string(),
        digest: Some(digest.clone()),
    }
}

fn directory(files: Vec<FileNode>, directories: Vec<DirectoryNode>) -> Directory {
    Directory {
        files,
        directories,
        symlinks: Vec::new(),
        node_properties: None,
    }
}

#[test]
fn build_for_files_matches_hand_built_directories() {
    let t = MerkleTreeComputerTest::set_up();
    let digest_util = DigestUtil::new(DigestHashFunction::Sha256);
    let files_root = t.exec_root.get_relative_str("files");
    files_root.create_directory_and_parents().unwrap();
    let mut inputs: BTreeMap<PathFragment, Path> = BTreeMap::new();
    for (logical_path, file_name, content) in [
        ("a/b/file1", "one", "content of file1"),
        ("a/file2", "two", "content of file2"),
        ("c", "three", "content of c"),
        ("d", "four", ""),
    ] {
        let path = files_root.get_child(file_name);
        write_content_as_latin1(&path, content);
        inputs.insert(PathFragment::create(logical_path), path);
    }

    let merkle_tree = t
        .create_merkle_tree_computer(/* uploader= */ None)
        .build_for_files(&inputs)
        .unwrap();

    let file1 = digest_util.compute(b"content of file1");
    let file2 = digest_util.compute(b"content of file2");
    let c = digest_util.compute(b"content of c");
    let empty = digest_util.compute(b"");
    let dir_a_b = directory(vec![file_node("file1", &file1, false)], vec![]);
    let dir_a_b_digest = digest_util.compute(&dir_a_b.to_byte_array());
    let dir_a = directory(
        vec![file_node("file2", &file2, false)],
        vec![directory_node("b", &dir_a_b_digest)],
    );
    let dir_a_digest = digest_util.compute(&dir_a.to_byte_array());
    let root = directory(
        vec![file_node("c", &c, false), file_node("d", &empty, false)],
        vec![directory_node("a", &dir_a_digest)],
    );
    let root_digest = digest_util.compute(&root.to_byte_array());

    assert_eq!(merkle_tree.digest(), &root_digest);
    assert_eq!(merkle_tree.input_files(), 4);
    assert_eq!(
        merkle_tree.input_bytes(),
        file1.size_bytes
            + file2.size_bytes
            + c.size_bytes
            + dir_a_b_digest.size_bytes
            + dir_a_digest.size_bytes
            + root_digest.size_bytes
    );
    assert!(merkle_tree.root().is_blobs_uploaded());
    // The empty blob doesn't have to be uploaded and is thus never included in the blobs map.
    let expected_digests: HashSet<Digest> = [
        file1,
        file2,
        c,
        dir_a_b_digest,
        dir_a_digest,
        root_digest.clone(),
    ]
    .into_iter()
    .collect();
    assert_eq!(
        merkle_tree
            .all_digests()
            .into_iter()
            .collect::<HashSet<_>>(),
        expected_digests
    );
    match merkle_tree.blobs().get(&root_digest) {
        Some(MerkleTreeBlob::Bytes(data)) => assert_eq!(&data[..], &root.to_byte_array()[..]),
        other => panic!("expected the root directory blob, got {other:?}"),
    }
}

#[test]
fn build_for_spawn_matches_hand_built_directories() {
    let t = MerkleTreeComputerTest::set_up();
    let digest_util = DigestUtil::new(DigestHashFunction::Sha256);
    let fake_file_cache = FakeActionInputFileCache::default();

    // A tool file.
    let file = create_artifact(&t.artifact_root, "pkg/file");
    file.get_path()
        .get_parent_directory()
        .unwrap()
        .create_directory_and_parents()
        .unwrap();
    write_content_as_latin1(&file.get_path(), "content of pkg/file");
    fake_file_cache.put(
        &file,
        FileArtifactValue::create_for_testing(&file.get_path()).unwrap(),
    );
    // A tree artifact with a nested child.
    let (tree, _child) = t.create_tree_artifact_with_file(
        &fake_file_cache,
        "tree",
        "sub/child",
        "content of tree/sub/child",
    );
    // A virtual input.
    let params = ActionInputRef::new(BasicVirtualActionInput::new(
        PathFragment::create("params/file.params"),
        b"--flag\n".to_vec(),
    ));
    // An output directory, which is staged as an empty input directory.
    let output_directory = Artifact::create_special_artifact(
        t.artifact_root.clone(),
        t.artifact_root.get_exec_path().get_relative_str("out_dir"),
        null_artifact_owner(),
        SpecialArtifactType::TREE,
    );
    let spawn = build_spawn_with_inputs(
        vec![artifact_input(&tree), params, artifact_input(&file)],
        &[&output_directory],
    );
    let tool_inputs: HashSet<PathFragment> = [PathFragment::create("outputs/pkg/file")]
        .into_iter()
        .collect();

    let file_digest = digest_util.compute(b"content of pkg/file");
    let child_digest = digest_util.compute(b"content of tree/sub/child");
    let params_digest = digest_util.compute(b"--flag\n");
    let empty_digest = digest_util.compute(b"");
    let dir_tree_sub = directory(vec![file_node("child", &child_digest, false)], vec![]);
    let dir_tree_sub_digest = digest_util.compute(&dir_tree_sub.to_byte_array());
    let dir_tree = directory(vec![], vec![directory_node("sub", &dir_tree_sub_digest)]);
    let dir_tree_digest = digest_util.compute(&dir_tree.to_byte_array());
    let dir_pkg = directory(vec![file_node("file", &file_digest, true)], vec![]);
    let dir_pkg_digest = digest_util.compute(&dir_pkg.to_byte_array());
    let dir_outputs = directory(
        vec![],
        vec![
            directory_node("out_dir", &empty_digest),
            directory_node("pkg", &dir_pkg_digest),
            directory_node("tree", &dir_tree_digest),
        ],
    );
    let dir_outputs_digest = digest_util.compute(&dir_outputs.to_byte_array());
    let dir_params = directory(
        vec![file_node("file.params", &params_digest, false)],
        vec![],
    );
    let dir_params_digest = digest_util.compute(&dir_params.to_byte_array());
    let root = directory(
        vec![],
        vec![
            directory_node("outputs", &dir_outputs_digest),
            directory_node("params", &dir_params_digest),
        ],
    );
    let root_digest = digest_util.compute(&root.to_byte_array());
    let expected_input_bytes = file_digest.size_bytes
        + child_digest.size_bytes
        + params_digest.size_bytes
        + dir_tree_sub_digest.size_bytes
        + dir_tree_digest.size_bytes
        + dir_pkg_digest.size_bytes
        + dir_outputs_digest.size_bytes
        + dir_params_digest.size_bytes
        + root_digest.size_bytes;

    let merkle_tree_computer = t.create_merkle_tree_computer(/* uploader= */ None);
    let discarded = t
        .build_for_spawn(
            &merkle_tree_computer,
            &spawn,
            &tool_inputs,
            &fake_file_cache,
            BlobPolicy::Discard,
        )
        .unwrap();
    match &discarded {
        MerkleTree::RootOnly(root_only @ MerkleTreeRootOnly::BlobsDiscarded { .. }) => {
            assert_eq!(root_only.digest(), &root_digest);
            assert_eq!(root_only.input_files(), 3);
            assert_eq!(root_only.input_bytes(), expected_input_bytes);
        }
        other => panic!("expected MerkleTree.RootOnly.BlobsDiscarded, got {other:?}"),
    }

    let kept = t
        .build_for_spawn(
            &merkle_tree_computer,
            &spawn,
            &tool_inputs,
            &fake_file_cache,
            BlobPolicy::Keep,
        )
        .unwrap();
    match &kept {
        MerkleTree::Uploadable(uploadable) => {
            assert_eq!(uploadable.digest(), &root_digest);
            assert_eq!(uploadable.input_files(), 3);
            assert_eq!(uploadable.input_bytes(), expected_input_bytes);
            // The blobs of the tree artifact stay with its own sub-tree.
            let expected_digests: HashSet<Digest> = [
                file_digest,
                params_digest,
                dir_pkg_digest,
                dir_outputs_digest,
                dir_params_digest,
                root_digest.clone(),
            ]
            .into_iter()
            .collect();
            assert_eq!(
                uploadable.all_digests().into_iter().collect::<HashSet<_>>(),
                expected_digests
            );
        }
        other => panic!("expected MerkleTree.Uploadable, got {other:?}"),
    }
}
