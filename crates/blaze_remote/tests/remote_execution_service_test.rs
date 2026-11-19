// Options and protos are filled in field by field, as the Java tests set them.
#![allow(clippy::field_reassign_with_default)]
// Java still reads and writes the deprecated REAPI fields (output_files, output_file_symlinks,
// ...), and the tests assert on them.
#![allow(deprecated)]
//! Port of `src/test/java/com/google/devtools/build/lib/remote/RemoteExecutionServiceTest.java`.
//!
//! Java runs the tests on an `InMemoryFileSystem` rooted at `/`; here they run on the real file
//! system under a temporary directory, so Java's absolute paths (`/execroot/_main`, `/tmp/stdout`)
//! are prefixed with that directory.
//!
//! PORT notes that apply to the whole file:
//! - Java's `cache` is a Mockito spy of an `InMemoryCombinedCache` (a `RemoteExecutionCache`) over
//!   a spy of an `InMemoryCacheClient`; the stubs it receives (`getRemoteServerCapabilities`,
//!   `uploadActionResult`) are made by [`TestCacheClient`], a `RemoteCacheClient` that wraps the
//!   shared `InMemoryCacheClient` and answers those calls itself. The `RemoteExecutionCache` is
//!   built over it.
//! - Java's `executor` is a Mockito mock of `RemoteExecutionClient`: [`FakeRemoteExecutionClient`].
//! - Java's `remoteOutputChecker` is a Mockito mock that downloads nothing unless a test stubs
//!   `shouldDownloadOutput`. The port builds a real `RemoteOutputChecker` when the service is
//!   created ([`OutputsToDownload`]): `--remote_download_minimal` (nothing), `all` (the
//!   `any(), any()` stub) or `minimal` with exact-path patterns (the stubs for given paths). The
//!   tests therefore set what to download before they create the service.
//! - Java hands each spawn execution context a `RemoteActionFileSystem` and asserts on its
//!   contents (`actionFs.getDigest(path)`, `actionFs.exists(path)`). blaze_exec's
//!   `SpawnExecutionContext` has no action file system; the service writes into it through the
//!   `RemoteOutputInjector` hook, which the tests record ([`RecordingRemoteOutputInjector`]).
//!   `RemoteActionFileSystem.getDigest` answers from the remote output tree first and from the
//!   local file system otherwise; [`Fixture::action_fs_get_digest`] does the same over the
//!   recorded injections.
//! - `TracingMetadataUtils` needs the tool version: Java's test `BlazeVersionInfo` is empty.

use std::any::Any;
use std::collections::{BTreeMap, HashSet};
use std::fmt;
use std::io;
use std::panic::AssertUnwindSafe;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::{Arc, Once};
use std::time::SystemTime;

use async_trait::async_trait;
use blaze_actions::{
    ActionLookupData, ActionLookupKey, ActionLookupKeyRef, Artifact as BlazeArtifact,
    ArtifactOwner, ArtifactRoot, RootType, SpecialArtifactType,
};
use blaze_cmdline::Label;
use blaze_collect::{NestedSet, NestedSetBuilder, Order};
use blaze_events::{EventKind, ExtendedEventHandler, StoredEventHandler};
use blaze_exec::actions::{
    ActionInputRef, BasicActionInput, FakeOwner, FileArtifactValue, PathMapper, ResourceSet,
    RunfilesTree, SimpleSpawn, Spawn, SpawnResult, SpawnResultBuilder, Status, SymlinkEntry,
    TreeArtifactValue, execution_requirements,
};
use blaze_exec::exec::action_execution_context::SpawnCacheError;
use blaze_exec::util::FileOutErr;
use blaze_optdefs::command::execution_options::ExecutionOptions;
use blaze_optdefs::misc::remote_options::{ConcurrentChangesCheckLevel, RemoteOptions};
use blaze_optdefs::misc::remote_outputs_mode::RemoteOutputsMode;
use blaze_protos::JavaMessage;
use blaze_protos::remote_scrubbing::{Config, config};
use blaze_protos::tools::protos::{CacheSalt, cache_salt};
use blaze_remote::api_version::ApiVersion;
use blaze_remote::combined_cache::CachedActionResult;
use blaze_remote::common::bulk_transfer_exception::BulkTransferException;
use blaze_remote::common::errors;
use blaze_remote::common::{
    ActionKey, Blob, DefaultRemotePathResolver, MissingDigestsFinder, OperationObserver,
    RemoteActionExecutionContext, RemoteCacheClient, RemoteExecutionClient, RemotePathResolver,
    SiblingRepositoryLayoutResolver,
};
use blaze_remote::hooks::{RemoteHooks, RemoteOutputInjector};
use blaze_remote::merkletree::{MerkleTree, MerkleTreeBlob};
use blaze_remote::reapi::{
    ActionCacheUpdateCapabilities, ActionResult, CacheCapabilities, Digest, Directory,
    DirectoryNode, ExecuteRequest, ExecuteResponse, ExecutionCapabilities, FileNode,
    OutputDirectory, OutputFile, OutputSymlink, ServerCapabilities, SymlinkNode, Tree,
    command::EnvironmentVariable, symlink_absolute_path_strategy,
};
use blaze_remote::remote_execution_cache::RemoteExecutionCache;
use blaze_remote::remote_execution_service::{RemoteActionResult, RemoteExecutionService};
use blaze_remote::remote_output_checker::{PatternToDownload, RemoteOutputChecker};
use blaze_remote::runtime::get_from_future;
use blaze_remote::scrubber::Scrubber;
use blaze_remote::store::Store;
use blaze_remote::upload_manifest::{ActionUploadFinishedEvent, ActionUploadStartedEvent};
use blaze_remote::util::DigestUtil;
use blaze_remote::util::async_task_cache::AsyncTaskCacheNoResult;
use blaze_remote::util::temp_path_generator::TempPathGenerator;
use blaze_remote::util::tracing_metadata_utils;
use blaze_repo_fetch::fs_utils;
use blaze_vfs::{
    DigestHashFunction, FileSystemRef, Path, PathFragment, Root, Symlinks, UnixFileSystem,
};
use indexmap::{IndexMap, IndexSet};
use parking_lot::Mutex;
use prost::Message as _;

mod util;

use util::fake_action_input_file_cache::FakeActionInputFileCache;
use util::fake_spawn_execution_context::FakeSpawnExecutionContext;
use util::in_memory_cache_client::InMemoryCacheClient;

/// `TestConstants.WORKSPACE_NAME` (Bazel).
const WORKSPACE_NAME: &str = "_main";

// ---------------------------------------------------------------------------------------------
// ActionsTestUtil
// ---------------------------------------------------------------------------------------------

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
        // `ActionsTestUtil.NULL_LABEL`.
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

/// `ActionsTestUtil.YET_ANOTHER_NULL_ARTIFACT_OWNER`.
#[derive(Debug)]
struct YetAnotherNullArtifactOwner;

impl fmt::Display for YetAnotherNullArtifactOwner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("YET_ANOTHER_NULL_ARTIFACT_OWNER")
    }
}

impl ActionLookupKey for YetAnotherNullArtifactOwner {
    fn get_label(&self) -> Option<Label> {
        // `ActionsTestUtil.YET_ANOTHER_NULL_LABEL`.
        Some(Label::parse_canonical("//yet/another/null/action:owner").unwrap())
    }
    fn get_configuration_checksum(&self) -> Option<String> {
        None
    }
    fn function_name(&self) -> &'static str {
        "YET_ANOTHER_NULL_ARTIFACT_OWNER"
    }
    fn key_equals(&self, other: &dyn ActionLookupKey) -> bool {
        other.as_any().is::<YetAnotherNullArtifactOwner>()
    }
    fn key_hash(&self) -> u64 {
        2
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

fn null_artifact_owner() -> ActionLookupKeyRef {
    ActionLookupKeyRef::new(NullArtifactOwner)
}

/// `ActionsTestUtil.NULL_ACTION_LOOKUP_DATA`.
fn null_action_lookup_data() -> ActionLookupData {
    ActionLookupData::create(null_artifact_owner(), 0)
}

/// An artifact as an `ActionInput`.
fn input(artifact: &BlazeArtifact) -> ActionInputRef {
    ActionInputRef::new(artifact.clone())
}

/// `ActionInputHelper.fromPath(path)`.
fn from_path(path: &str) -> ActionInputRef {
    ActionInputRef::new(BasicActionInput::from_path(path))
}

/// `ActionsTestUtil.createArtifactWithExecPath(root, execPath)`.
// Java: com/google/devtools/build/lib/actions/util/ActionsTestUtil.java:274 ActionsTestUtil.createArtifactWithExecPath
fn create_artifact_with_exec_path(root: &ArtifactRoot, exec_path: PathFragment) -> BlazeArtifact {
    if root.is_source_root() {
        BlazeArtifact::new_source_artifact(root.clone(), exec_path, ArtifactOwner::NullOwner)
    } else {
        BlazeArtifact::create_derived_artifact(root.clone(), exec_path, null_artifact_owner())
    }
}

/// `ActionsTestUtil.createArtifactWithRootRelativePath(root, rootRelativePath)`.
// Java: com/google/devtools/build/lib/actions/util/ActionsTestUtil.java:268 ActionsTestUtil.createArtifactWithRootRelativePath
fn create_artifact_with_root_relative_path(
    root: &ArtifactRoot,
    root_relative_path: &PathFragment,
) -> BlazeArtifact {
    let exec_path = root.get_exec_path().get_relative(root_relative_path);
    create_artifact_with_exec_path(root, exec_path)
}

/// `ActionsTestUtil.createArtifact(root, path)` for a `Path`.
// Java: com/google/devtools/build/lib/actions/util/ActionsTestUtil.java:260 ActionsTestUtil.createArtifact
fn create_artifact(root: &ArtifactRoot, path: &Path) -> BlazeArtifact {
    create_artifact_with_root_relative_path(root, &root.get_root().relativize(path))
}

/// `ActionsTestUtil.createArtifact(root, path)` for a `String`.
// Java: com/google/devtools/build/lib/actions/util/ActionsTestUtil.java:264 ActionsTestUtil.createArtifact
fn create_artifact_str(root: &ArtifactRoot, path: &str) -> BlazeArtifact {
    create_artifact_with_root_relative_path(root, &PathFragment::create(path))
}

/// `ActionsTestUtil.createRunfilesArtifact(root, execPath)`.
// Java: com/google/devtools/build/lib/actions/util/ActionsTestUtil.java:280 ActionsTestUtil.createRunfilesArtifact
fn create_runfiles_artifact(root: &ArtifactRoot, exec_path: &str) -> BlazeArtifact {
    BlazeArtifact::create_special_artifact(
        root.clone(),
        PathFragment::create(exec_path),
        null_artifact_owner(),
        SpecialArtifactType::RUNFILES,
    )
}

/// `ActionsTestUtil.createTreeArtifactWithGeneratingAction(root, execPath)`.
// Java: com/google/devtools/build/lib/actions/util/ActionsTestUtil.java:290 ActionsTestUtil.createTreeArtifactWithGeneratingAction
fn create_tree_artifact_with_generating_action(
    root: &ArtifactRoot,
    exec_path: PathFragment,
) -> BlazeArtifact {
    let tree_artifact = BlazeArtifact::create_special_artifact(
        root.clone(),
        exec_path,
        null_artifact_owner(),
        SpecialArtifactType::TREE,
    );
    tree_artifact.set_generating_action_key(null_action_lookup_data());
    tree_artifact
}

/// `ActionsTestUtil.createTreeArtifactWithGeneratingAction(root, rootRelativePath)`.
// Java: com/google/devtools/build/lib/actions/util/ActionsTestUtil.java:298 ActionsTestUtil.createTreeArtifactWithGeneratingAction
fn create_tree_artifact_with_generating_action_str(
    root: &ArtifactRoot,
    root_relative_path: &str,
) -> BlazeArtifact {
    create_tree_artifact_with_generating_action(
        root,
        root.get_exec_path()
            .get_relative(&PathFragment::create(root_relative_path)),
    )
}

/// `ActionsTestUtil.createUnresolvedSymlinkArtifact(root, execPath)`.
// Java: com/google/devtools/build/lib/actions/util/ActionsTestUtil.java:304 ActionsTestUtil.createUnresolvedSymlinkArtifact
fn create_unresolved_symlink_artifact(root: &ArtifactRoot, exec_path: &str) -> BlazeArtifact {
    create_unresolved_symlink_artifact_with_exec_path(
        root,
        root.get_exec_path()
            .get_relative(&PathFragment::create(exec_path)),
    )
}

/// `ActionsTestUtil.createUnresolvedSymlinkArtifactWithExecPath(root, execPath)`.
// Java: com/google/devtools/build/lib/actions/util/ActionsTestUtil.java:310 ActionsTestUtil.createUnresolvedSymlinkArtifactWithExecPath
fn create_unresolved_symlink_artifact_with_exec_path(
    root: &ArtifactRoot,
    exec_path: PathFragment,
) -> BlazeArtifact {
    BlazeArtifact::create_special_artifact(
        root.clone(),
        exec_path,
        null_artifact_owner(),
        SpecialArtifactType::UNRESOLVED_SYMLINK,
    )
}

// ---------------------------------------------------------------------------------------------
// SpawnBuilder
// ---------------------------------------------------------------------------------------------

/// Port of the test class `lib/exec/util/SpawnBuilder.java` (the parts these tests use).
// Java: com/google/devtools/build/lib/exec/util/SpawnBuilder.java:41 SpawnBuilder
struct SpawnBuilder {
    args: Vec<String>,
    environment: IndexMap<String, String>,
    execution_info: IndexMap<String, String>,
    inputs: Vec<ActionInputRef>,
    outputs: Vec<ActionInputRef>,
    tools: Vec<ActionInputRef>,
}

impl SpawnBuilder {
    fn new(args: &[&str]) -> SpawnBuilder {
        SpawnBuilder {
            args: args.iter().map(|a| a.to_string()).collect(),
            environment: IndexMap::new(),
            execution_info: IndexMap::new(),
            inputs: Vec::new(),
            outputs: Vec::new(),
            tools: Vec::new(),
        }
    }

    fn with_output(mut self, output: ActionInputRef) -> SpawnBuilder {
        self.outputs.push(output);
        self
    }

    fn with_inputs(mut self, inputs: impl IntoIterator<Item = ActionInputRef>) -> SpawnBuilder {
        self.inputs.extend(inputs);
        self
    }

    fn with_tools(mut self, tools: impl IntoIterator<Item = ActionInputRef>) -> SpawnBuilder {
        self.tools.extend(tools);
        self
    }

    fn with_environment(mut self, key: &str, value: &str) -> SpawnBuilder {
        self.environment.insert(key.to_string(), value.to_string());
        self
    }

    fn with_execution_info(mut self, key: &str, value: &str) -> SpawnBuilder {
        self.execution_info
            .insert(key.to_string(), value.to_string());
        self
    }

    // Java: com/google/devtools/build/lib/exec/util/SpawnBuilder.java:66 SpawnBuilder.build
    fn build(self) -> Arc<dyn Spawn> {
        // `new FakeOwner(mnemonic, progressMessage, ownerLabel, ...)` with the builder's defaults.
        let owner = FakeOwner::new(
            "Mnemonic",
            Some("progress message"),
            Some(Label::parse_canonical("//dummy:label").unwrap()),
        );
        let mut inputs = NestedSetBuilder::stable_order();
        inputs.add_all(self.inputs);
        let mut tools = NestedSetBuilder::stable_order();
        tools.add_all(self.tools);
        // `ImmutableSet.copyOf(outputs)`.
        let outputs: IndexSet<ActionInputRef> = self.outputs.into_iter().collect();
        Arc::new(SimpleSpawn::new(
            Arc::new(owner),
            self.args,
            self.environment,
            self.execution_info,
            inputs.build(),
            tools.build(),
            outputs.into_iter().collect(),
            /* mandatoryOutputs= */ None,
            Arc::new(ResourceSet::zero()),
            PathMapper::NOOP,
        ))
    }
}

// ---------------------------------------------------------------------------------------------
// Test doubles
// ---------------------------------------------------------------------------------------------

/// The spied `InMemoryCacheClient` of Java's `cache`, with the stubs the tests put on the spied
/// `InMemoryCombinedCache` (see the file comment).
struct TestCacheClient {
    inner: Arc<InMemoryCacheClient>,
    /// `doReturn(capabilities).when(cache).getRemoteServerCapabilities()`.
    server_capabilities: Mutex<ServerCapabilities>,
    /// `doReturn(Futures.immediateFailedFuture(new IOException(message))).when(cache)
    /// .uploadActionResult(any(), any(), any())`.
    upload_action_result_failure: Mutex<Option<String>>,
}

#[async_trait]
impl MissingDigestsFinder for TestCacheClient {
    async fn find_missing_digests(
        &self,
        context: &RemoteActionExecutionContext<'_>,
        digests: &[Digest],
    ) -> io::Result<IndexSet<Digest>> {
        self.inner.find_missing_digests(context, digests).await
    }
}

#[async_trait]
impl RemoteCacheClient for TestCacheClient {
    fn cas_upload_cache(&self) -> &AsyncTaskCacheNoResult<Digest> {
        self.inner.cas_upload_cache()
    }

    fn get_server_capabilities(&self) -> io::Result<ServerCapabilities> {
        let capabilities = self.server_capabilities.lock().clone();
        Ok(capabilities)
    }

    async fn get_authority(&self) -> io::Result<String> {
        self.inner.get_authority().await
    }

    async fn download_action_result(
        &self,
        context: &RemoteActionExecutionContext<'_>,
        action_key: &ActionKey,
        inline_out_err: bool,
        inline_output_files: &HashSet<String>,
    ) -> io::Result<Option<ActionResult>> {
        self.inner
            .download_action_result(context, action_key, inline_out_err, inline_output_files)
            .await
    }

    async fn upload_action_result(
        &self,
        context: &RemoteActionExecutionContext<'_>,
        action_key: &ActionKey,
        action_result: &ActionResult,
    ) -> io::Result<()> {
        let failure = self.upload_action_result_failure.lock().clone();
        if let Some(message) = failure {
            return Err(errors::io_exception(message));
        }
        self.inner
            .upload_action_result(context, action_key, action_result)
            .await
    }

    async fn download_blob(
        &self,
        context: &RemoteActionExecutionContext<'_>,
        digest: &Digest,
        out: &mut (dyn io::Write + Send),
    ) -> io::Result<()> {
        self.inner.download_blob(context, digest, out).await
    }

    async fn upload_blob_impl(
        &self,
        context: &RemoteActionExecutionContext<'_>,
        digest: &Digest,
        blob: &dyn Blob,
    ) -> io::Result<()> {
        self.inner.upload_blob_impl(context, digest, blob).await
    }

    fn close(&self) {
        self.inner.close();
    }
}

/// `mock(RemoteExecutionClient.class)` with `getServerCapabilities` stubbed.
struct FakeRemoteExecutionClient {
    server_capabilities: Mutex<ServerCapabilities>,
}

impl RemoteExecutionClient for FakeRemoteExecutionClient {
    fn get_server_capabilities(&self) -> io::Result<ServerCapabilities> {
        let capabilities = self.server_capabilities.lock().clone();
        Ok(capabilities)
    }

    fn execute_remotely(
        &self,
        _context: &RemoteActionExecutionContext<'_>,
        _request: ExecuteRequest,
        _observer: &dyn OperationObserver,
    ) -> io::Result<ExecuteResponse> {
        // An unstubbed Mockito method returns null.
        panic!("NullPointerException: RemoteExecutionClient.executeRemotely is not stubbed");
    }

    fn close(&self) {}
}

/// What `RemoteActionFileSystem.injectRemoteFile` received for one path.
#[derive(Clone, Debug)]
#[allow(dead_code)]
struct InjectedRemoteFile {
    digest: Vec<u8>,
    size: i64,
    expiration_time: SystemTime,
    in_memory_output: bool,
}

/// Records the remote files the service injects into the action file system.
#[derive(Default)]
struct RecordingRemoteOutputInjector {
    injected: Mutex<IndexMap<PathFragment, InjectedRemoteFile>>,
}

impl RemoteOutputInjector for RecordingRemoteOutputInjector {
    fn inject_remote_file(
        &self,
        path: &PathFragment,
        digest: Vec<u8>,
        size: i64,
        expiration_time: SystemTime,
        in_memory_output: bool,
    ) -> io::Result<()> {
        self.injected.lock().insert(
            path.clone(),
            InjectedRemoteFile {
                digest,
                size,
                expiration_time,
                in_memory_output,
            },
        );
        Ok(())
    }
}

/// What the mocked `RemoteOutputChecker` answers to `shouldDownloadOutput(execPath,
/// treeRootExecPath)`.
#[derive(Clone)]
enum OutputsToDownload {
    /// The mock's default (`false`).
    Nothing,
    /// `when(remoteOutputChecker.shouldDownloadOutput(any(), any())).thenReturn(true)`.
    All,
    /// `when(remoteOutputChecker.shouldDownloadOutput(PathFragment.create(path), ...))
    /// .thenReturn(true)` for each path.
    Paths(Vec<&'static str>),
}

/// `RemoteTreeArtifact`-less `RunfilesTree` of `createRunfilesTree(root, artifacts)`.
// Java: com/google/devtools/build/lib/remote/RemoteExecutionServiceTest.java:3327 RemoteExecutionServiceTest.createRunfilesTree
#[derive(Debug)]
struct TestRunfilesTree {
    root: PathFragment,
    artifacts: Vec<ActionInputRef>,
    mapping: Vec<(PathFragment, Option<ActionInputRef>)>,
}

impl TestRunfilesTree {
    fn new(root: &str, artifacts: &[BlazeArtifact]) -> TestRunfilesTree {
        // `toImmutableSortedMap(naturalOrder(), artifact ->
        // PathFragment.create(WORKSPACE_NAME).getRelative(artifact.getRunfilesPath()), identity(),
        // (a, b) -> b)`. PathFragments compare by their path strings.
        let mut mapping: BTreeMap<String, ActionInputRef> = BTreeMap::new();
        for artifact in artifacts {
            let key = PathFragment::create(WORKSPACE_NAME)
                .get_relative(&artifact.get_runfiles_path())
                .get_path_string()
                .to_string();
            mapping.insert(key, input(artifact));
        }
        TestRunfilesTree {
            root: PathFragment::create(root),
            artifacts: artifacts.iter().map(input).collect(),
            mapping: mapping
                .into_iter()
                .map(|(k, v)| (PathFragment::create(&k), Some(v)))
                .collect(),
        }
    }
}

impl RunfilesTree for TestRunfilesTree {
    fn get_exec_path(&self) -> PathFragment {
        self.root.clone()
    }
    fn get_mapping(&self) -> Vec<(PathFragment, Option<ActionInputRef>)> {
        self.mapping.clone()
    }
    fn get_workspace_name(&self) -> String {
        WORKSPACE_NAME.to_string()
    }
    fn is_mapping_cached(&self) -> bool {
        false
    }
    fn get_artifacts_at_canonical_locations_for_logging(&self) -> NestedSet<ActionInputRef> {
        NestedSetBuilder::wrap(Order::STABLE_ORDER, self.artifacts.clone())
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

// ---------------------------------------------------------------------------------------------
// Fixture
// ---------------------------------------------------------------------------------------------

static SET_TOOL_VERSION: Once = Once::new();

fn digest_util() -> DigestUtil {
    DigestUtil::new(DigestHashFunction::Sha256)
}

fn cache_capabilities() -> CacheCapabilities {
    CacheCapabilities {
        action_cache_update_capabilities: Some(ActionCacheUpdateCapabilities {
            update_enabled: true,
        }),
        symlink_absolute_path_strategy: symlink_absolute_path_strategy::Value::Allowed as i32,
        ..Default::default()
    }
}

/// In the past, Bazel only supports RemoteApi version 2.0. Use this to ensure we are backward
/// compatible with Servers that only support 2.0.
fn legacy_remote_executor_capabilities() -> ServerCapabilities {
    ServerCapabilities {
        cache_capabilities: Some(cache_capabilities()),
        low_api_version: Some(ApiVersion::TWO_POINT_ZERO.to_sem_ver()),
        high_api_version: Some(ApiVersion::TWO_POINT_ZERO.to_sem_ver()),
        execution_capabilities: Some(ExecutionCapabilities {
            exec_enabled: true,
            ..Default::default()
        }),
        ..Default::default()
    }
}

fn remote_executor_capabilities() -> ServerCapabilities {
    ServerCapabilities {
        cache_capabilities: Some(cache_capabilities()),
        low_api_version: Some(ApiVersion::LOW.to_sem_ver()),
        high_api_version: Some(ApiVersion::HIGH.to_sem_ver()),
        execution_capabilities: Some(ExecutionCapabilities {
            exec_enabled: true,
            ..Default::default()
        }),
        ..Default::default()
    }
}

/// The fields of the Java test class, as `setUp` leaves them.
struct Fixture {
    _dir: tempfile::TempDir,
    fs: FileSystemRef,
    digest_util: DigestUtil,
    event_handler: Arc<StoredEventHandler>,
    remote_options: RemoteOptions,
    exec_root: Path,
    source_root: ArtifactRoot,
    artifact_root: ArtifactRoot,
    temp_path_generator: Arc<TempPathGenerator>,
    fake_file_cache: Arc<FakeActionInputFileCache>,
    remote_path_resolver: Arc<dyn RemotePathResolver>,
    out_err: Arc<FileOutErr>,
    /// The `InMemoryCacheClient` behind `cache`.
    in_memory_cache_client: Arc<InMemoryCacheClient>,
    cache_client: Arc<TestCacheClient>,
    cache: Arc<RemoteExecutionCache>,
    executor: Arc<FakeRemoteExecutionClient>,
    remote_action_execution_context: RemoteActionExecutionContext<'static>,
    // Port-only state (see the file comment).
    outputs_to_download: OutputsToDownload,
    injector: Arc<RecordingRemoteOutputInjector>,
    scrubber: Option<Arc<Scrubber>>,
}

impl Fixture {
    // Java: com/google/devtools/build/lib/remote/RemoteExecutionServiceTest.java:232 RemoteExecutionServiceTest.setUp
    fn set_up() -> Fixture {
        SET_TOOL_VERSION.call_once(|| tracing_metadata_utils::set_tool_version(""));

        let dir = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(dir.path())
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();

        let remote_options = RemoteOptions::default();

        let fs = UnixFileSystem::create(DigestHashFunction::Sha256);

        let exec_root = Path::create_from_str(&format!("{root}/execroot/_main"), &fs);
        exec_root.create_directory_and_parents().unwrap();

        let artifact_root =
            ArtifactRoot::as_derived_root(&exec_root, RootType::OUTPUT, &["outputs"]);
        let source_root = ArtifactRoot::as_source_root(Root::from_path(exec_root.clone()));

        artifact_root
            .get_root()
            .as_path()
            .expect("checkNotNull")
            .create_directory_and_parents()
            .unwrap();

        let temp_path_generator = Arc::new(TempPathGenerator::new(Path::create_from_str(
            &format!("{root}/execroot/_tmp/actions/remote"),
            &fs,
        )));

        let fake_file_cache = Arc::new(FakeActionInputFileCache::new(exec_root.clone()));

        let remote_path_resolver: Arc<dyn RemotePathResolver> =
            Arc::new(DefaultRemotePathResolver::new(exec_root.clone()));

        let stdout = Path::create_from_str(&format!("{root}/tmp/stdout"), &fs);
        let stderr = Path::create_from_str(&format!("{root}/tmp/stderr"), &fs);
        stdout
            .get_parent_directory()
            .expect("checkNotNull")
            .create_directory_and_parents()
            .unwrap();
        stderr
            .get_parent_directory()
            .expect("checkNotNull")
            .create_directory_and_parents()
            .unwrap();
        let out_err = Arc::new(FileOutErr::new(stdout, stderr));

        let digest_util = digest_util();
        let in_memory_cache_client = Arc::new(InMemoryCacheClient::new());
        let cache_client = Arc::new(TestCacheClient {
            inner: in_memory_cache_client.clone(),
            // `doReturn(remoteExecutorCapabilities).when(cache).getRemoteServerCapabilities()`
            server_capabilities: Mutex::new(remote_executor_capabilities()),
            upload_action_result_failure: Mutex::new(None),
        });
        let cache = Arc::new(RemoteExecutionCache::new(
            cache_client.clone(),
            /* diskCacheClient= */ None,
            /* symlinkTemplate= */ None,
            digest_util.clone(),
            /* chunkingFunction= */ None,
        ));
        // `when(executor.getServerCapabilities()).thenReturn(remoteExecutorCapabilities)`
        let executor = Arc::new(FakeRemoteExecutionClient {
            server_capabilities: Mutex::new(remote_executor_capabilities()),
        });

        let metadata = tracing_metadata_utils::build_metadata("none", "none", "action-id", None);
        let remote_action_execution_context = RemoteActionExecutionContext::create(metadata);

        Fixture {
            _dir: dir,
            fs,
            digest_util,
            event_handler: Arc::new(StoredEventHandler::new()),
            remote_options,
            exec_root,
            source_root,
            artifact_root,
            temp_path_generator,
            fake_file_cache,
            remote_path_resolver,
            out_err,
            in_memory_cache_client,
            cache_client,
            cache,
            executor,
            remote_action_execution_context,
            outputs_to_download: OutputsToDownload::Nothing,
            injector: Arc::new(RecordingRemoteOutputInjector::default()),
            scrubber: None,
        }
    }

    /// `doReturn(capabilities).when(cache).getRemoteServerCapabilities()` and
    /// `when(executor.getServerCapabilities()).thenReturn(capabilities)`.
    fn set_server_capabilities(&self, capabilities: ServerCapabilities) {
        *self.cache_client.server_capabilities.lock() = capabilities.clone();
        *self.executor.server_capabilities.lock() = capabilities;
    }

    // `cache.addContents(remoteActionExecutionContext, ...)`.
    // Java: com/google/devtools/build/lib/remote/InMemoryCombinedCache.java:74 InMemoryCombinedCache.addContents
    fn add_contents(&self, bytes: &[u8]) -> Digest {
        let digest = self.digest_util.compute(bytes);
        get_from_future(self.cache_client.upload_bytes(
            &self.remote_action_execution_context,
            &digest,
            bytes::Bytes::copy_from_slice(bytes),
            /* force= */ false,
        ))
        .unwrap();
        digest
    }

    // Java: com/google/devtools/build/lib/remote/InMemoryCombinedCache.java:69 InMemoryCombinedCache.addContents
    fn add_contents_str(&self, txt: &str) -> Digest {
        self.add_contents(txt.as_bytes())
    }

    // Java: com/google/devtools/build/lib/remote/InMemoryCombinedCache.java:83 InMemoryCombinedCache.addContents
    fn add_contents_message<M: JavaMessage>(&self, m: &M) -> Digest {
        self.add_contents(&m.to_byte_array())
    }

    // Java: com/google/devtools/build/lib/remote/InMemoryCombinedCache.java:88 InMemoryCombinedCache.addException
    fn add_exception_str(&self, txt: &str, e: io::Error) -> Digest {
        let digest = self.digest_util.compute(txt.as_bytes());
        self.in_memory_cache_client
            .add_download_failure(digest.clone(), e);
        digest
    }

    // Java: com/google/devtools/build/lib/remote/InMemoryCombinedCache.java:94 InMemoryCombinedCache.addException
    fn add_exception_message<M: JavaMessage>(&self, m: &M, e: io::Error) -> Digest {
        let digest = self.digest_util.compute_message(m);
        self.in_memory_cache_client
            .add_download_failure(digest.clone(), e);
        digest
    }

    /// `getFromFuture(cache.findMissingDigests(remoteActionExecutionContext, digests))`.
    fn find_missing_digests(&self, digests: &[Digest]) -> IndexSet<Digest> {
        get_from_future(
            self.cache
                .find_missing_digests(&self.remote_action_execution_context, digests),
        )
        .unwrap()
    }

    /// `execRoot.getRelative(path)`.
    fn exec_path(&self, path: &str) -> Path {
        self.exec_root.get_relative_str(path)
    }

    /// `execRoot.asFragment().getRelative(path)`.
    fn exec_fragment(&self, path: &str) -> PathFragment {
        self.exec_root.as_fragment().get_relative_str(path)
    }

    /// `actionFs.getDigest(path)`: the injected metadata of a remote output, or the digest of the
    /// local file.
    fn action_fs_get_digest(&self, path: &PathFragment) -> Option<Vec<u8>> {
        if let Some(injected) = self.injector.injected.lock().get(path) {
            return Some(injected.digest.clone());
        }
        let local = Path::create(path.clone(), &self.fs);
        if !local.exists() {
            return None;
        }
        Some(local.get_digest().unwrap())
    }

    /// `actionFs.exists(path)`.
    fn action_fs_exists(&self, path: &PathFragment) -> bool {
        self.injector.injected.lock().contains_key(path)
            || Path::create(path.clone(), &self.fs).exists()
    }

    // Java: com/google/devtools/build/lib/remote/RemoteExecutionServiceTest.java:3201 RemoteExecutionServiceTest.newSpawnFromResult
    fn new_spawn_from_result(&self, result: &RemoteActionResult) -> Arc<dyn Spawn> {
        self.new_spawn_from_result_with_execution_info(IndexMap::new(), result)
    }

    // Java: com/google/devtools/build/lib/remote/RemoteExecutionServiceTest.java:3205 RemoteExecutionServiceTest.newSpawnFromResult
    fn new_spawn_from_result_with_execution_info(
        &self,
        execution_info: IndexMap<String, String>,
        result: &RemoteActionResult,
    ) -> Arc<dyn Spawn> {
        // PORT: StringEncoding identity (`unicodeToInternal`).
        let mut outputs: IndexSet<ActionInputRef> = IndexSet::new();
        for file in result.get_output_files() {
            let path = self
                .remote_path_resolver
                .output_path_to_local_path(&file.path);
            let output = create_artifact(&self.artifact_root, &path);
            outputs.insert(input(&output));
        }

        for directory in result.get_output_directories() {
            let path = self
                .remote_path_resolver
                .output_path_to_local_path(&directory.path);
            let output = create_tree_artifact_with_generating_action(
                &self.artifact_root,
                path.relative_to(&self.exec_root),
            );
            outputs.insert(input(&output));
        }

        for file_symlink in result.get_output_file_symlinks() {
            let path = self
                .remote_path_resolver
                .output_path_to_local_path(&file_symlink.path);
            let output = create_artifact(&self.artifact_root, &path);
            outputs.insert(input(&output));
        }

        for directory_symlink in result.get_output_directory_symlinks() {
            let path = self
                .remote_path_resolver
                .output_path_to_local_path(&directory_symlink.path);
            let output = create_tree_artifact_with_generating_action(
                &self.artifact_root,
                path.relative_to(&self.exec_root),
            );
            outputs.insert(input(&output));
        }

        for symlink in result.get_output_symlinks() {
            let path = self
                .remote_path_resolver
                .output_path_to_local_path(&symlink.path);
            let output = create_artifact(&self.artifact_root, &path);
            outputs.insert(input(&output));
        }

        new_spawn(execution_info, outputs)
    }

    // Java: com/google/devtools/build/lib/remote/RemoteExecutionServiceTest.java:3247 RemoteExecutionServiceTest.newSpawnFromResultWithInMemoryOutput
    fn new_spawn_from_result_with_in_memory_output(
        &self,
        result: &RemoteActionResult,
        in_memory_output: &PathFragment,
    ) -> Arc<dyn Spawn> {
        let mut execution_info = IndexMap::new();
        execution_info.insert(
            execution_requirements::REMOTE_EXECUTION_INLINE_OUTPUTS.to_string(),
            in_memory_output.get_path_string().to_string(),
        );
        self.new_spawn_from_result_with_execution_info(execution_info, result)
    }

    // Java: com/google/devtools/build/lib/remote/RemoteExecutionServiceTest.java:3272 RemoteExecutionServiceTest.newSpawnExecutionContext
    fn new_spawn_execution_context(&self, spawn: &Arc<dyn Spawn>) -> FakeSpawnExecutionContext {
        self.new_spawn_execution_context_with_out_err(spawn, self.out_err.clone())
    }

    /// PORT: Java builds a `RemoteActionInputFetcher` and a `RemoteActionFileSystem` for the
    /// context; the file system is the service's `RemoteOutputInjector` hook here (see the file
    /// comment).
    // Java: com/google/devtools/build/lib/remote/RemoteExecutionServiceTest.java:3276 RemoteExecutionServiceTest.newSpawnExecutionContext
    fn new_spawn_execution_context_with_out_err(
        &self,
        spawn: &Arc<dyn Spawn>,
        out_err: Arc<FileOutErr>,
    ) -> FakeSpawnExecutionContext {
        FakeSpawnExecutionContext::new(
            spawn.clone(),
            self.fake_file_cache.clone(),
            self.exec_root.clone(),
            out_err,
        )
    }

    // Java: com/google/devtools/build/lib/remote/RemoteExecutionServiceTest.java:3301 RemoteExecutionServiceTest.newRemoteExecutionService
    fn new_remote_execution_service(&self) -> RemoteExecutionService {
        self.new_remote_execution_service_with_options(&self.remote_options)
    }

    // Java: com/google/devtools/build/lib/remote/RemoteExecutionServiceTest.java:3305 RemoteExecutionServiceTest.newRemoteExecutionService
    fn new_remote_execution_service_with_options(
        &self,
        remote_options: &RemoteOptions,
    ) -> RemoteExecutionService {
        let remote_output_checker = match &self.outputs_to_download {
            OutputsToDownload::Nothing => {
                RemoteOutputChecker::new("build", RemoteOutputsMode::MINIMAL, Vec::new())
            }
            OutputsToDownload::All => {
                RemoteOutputChecker::new("build", RemoteOutputsMode::ALL, Vec::new())
            }
            OutputsToDownload::Paths(paths) => {
                let patterns: Vec<PatternToDownload> = paths
                    .iter()
                    .map(|path| {
                        let path = path.to_string();
                        Arc::new(move |s: &str| s == path) as PatternToDownload
                    })
                    .collect();
                RemoteOutputChecker::new("build", RemoteOutputsMode::MINIMAL, patterns)
            }
        };
        let reporter: Arc<dyn ExtendedEventHandler> = self.event_handler.clone();
        let executor: Arc<dyn RemoteExecutionClient> = self.executor.clone();
        let injector: Arc<dyn RemoteOutputInjector> = self.injector.clone();
        RemoteExecutionService::new(
            reporter,
            /* verboseFailures= */ true,
            self.exec_root.clone(),
            self.remote_path_resolver.clone(),
            "none",
            "none",
            WORKSPACE_NAME,
            self.digest_util.clone(),
            Arc::new(remote_options.clone()),
            Arc::new(ExecutionOptions::default()),
            /* combinedCache= */ None,
            Some(self.cache.clone()),
            Some(executor),
            self.temp_path_generator.clone(),
            /* captureCorruptedOutputsDir= */ None,
            Some(Arc::new(remote_output_checker)),
            // `Sets.newConcurrentHashSet()`
            Arc::new(Mutex::new(HashSet::new())),
            self.scrubber.clone(),
        )
        .with_hooks(RemoteHooks {
            remote_output_injector: Some(injector),
        })
    }

    // Java: com/google/devtools/build/lib/remote/RemoteExecutionServiceTest.java:3406 RemoteExecutionServiceTest.createOutputDirectories
    fn create_output_directories(&self, spawn: &Arc<dyn Spawn>) {
        for input in spawn.get_output_files() {
            let mut dir = self.exec_root.get_relative(input.get_exec_path());
            if !input.is_directory() {
                dir = dir.get_parent_directory().unwrap();
            }
            dir.create_directory_and_parents().unwrap();
        }
    }
}

// Java: com/google/devtools/build/lib/remote/RemoteExecutionServiceTest.java:3253 RemoteExecutionServiceTest.newSpawn
fn new_spawn(
    execution_info: IndexMap<String, String>,
    outputs: IndexSet<ActionInputRef>,
) -> Arc<dyn Spawn> {
    new_spawn_with_inputs(
        execution_info,
        outputs,
        NestedSetBuilder::empty_set(Order::STABLE_ORDER),
    )
}

// Java: com/google/devtools/build/lib/remote/RemoteExecutionServiceTest.java:3258 RemoteExecutionServiceTest.newSpawn
fn new_spawn_with_inputs(
    execution_info: IndexMap<String, String>,
    outputs: IndexSet<ActionInputRef>,
    inputs: NestedSet<ActionInputRef>,
) -> Arc<dyn Spawn> {
    Arc::new(SimpleSpawn::new_simple(
        Arc::new(FakeOwner::new(
            "foo",
            Some("bar"),
            Some(Label::parse_canonical("//dummy:label").unwrap()),
        )),
        /* arguments= */ Vec::new(),
        /* environment= */ IndexMap::new(),
        /* executionInfo= */ execution_info,
        /* inputs= */ inputs,
        /* outputs= */ outputs.into_iter().collect(),
        Arc::new(ResourceSet::zero()),
    ))
}

/// `uploadOutputsAndWait(service, action, result)`.
// Java: com/google/devtools/build/lib/remote/RemoteExecutionServiceTest.java:3416 RemoteExecutionServiceTest.uploadOutputsAndWait
fn upload_outputs_and_wait(
    service: &RemoteExecutionService,
    action: &blaze_remote::remote_action::RemoteAction<'_>,
    result: &SpawnResult,
) {
    let (tx, rx) = std::sync::mpsc::channel::<()>();
    service
        .upload_outputs_with_concurrent_changes_check_level(
            action,
            result,
            Box::new(move || {
                let _ = tx.send(());
            }),
            ConcurrentChangesCheckLevel::OFF,
        )
        .unwrap();
    rx.recv().unwrap();
}

/// `new SpawnResult.Builder().setExitCode(0).setStatus(SUCCESS).setRunnerName("test").build()`.
fn successful_spawn_result() -> SpawnResult {
    let mut builder = SpawnResultBuilder::new();
    builder
        .set_exit_code(0)
        .set_status(Status::Success)
        .set_runner_name("test");
    builder.build()
}

/// `RemoteActionResult.createFromCache(CachedActionResult.remote(r))`.
fn from_cache(r: ActionResult) -> RemoteActionResult {
    RemoteActionResult::create_from_cache(CachedActionResult::remote(Some(r)))
}

/// `readContent(path, UTF_8)`.
fn read_content(path: &Path) -> String {
    String::from_utf8(fs_utils::read_content(path).unwrap()).unwrap()
}

/// The `IOException` of a call that throws `IOException` / `ExecException`.
fn expect_io_error<T>(r: Result<T, SpawnCacheError>) -> io::Error {
    match r {
        Ok(_) => panic!("expected an IOException, the call succeeded"),
        Err(SpawnCacheError::Io(e)) => e,
        Err(SpawnCacheError::Exec(e)) => {
            panic!(
                "expected an IOException, got an ExecException: {}",
                e.get_message()
            )
        }
    }
}

/// `assertThrows(BulkTransferException.class, ...)`.
fn expect_bulk_transfer_exception<T>(r: Result<T, SpawnCacheError>) -> BulkTransferException {
    let e = expect_io_error(r);
    match errors::downcast::<BulkTransferException>(e) {
        Ok(b) => b,
        Err(e) => panic!("expected a BulkTransferException, got {e:?}"),
    }
}

/// `Throwables.getRootCause(e).getMessage()`.
fn root_cause_message(e: &(dyn std::error::Error + 'static)) -> String {
    let mut cause = e;
    while let Some(source) = cause.source() {
        cause = source;
    }
    cause.to_string()
}

/// The message of a panic payload.
fn panic_message(payload: &(dyn Any + Send)) -> String {
    if let Some(s) = payload.downcast_ref::<&str>() {
        s.to_string()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "<non-string panic>".to_string()
    }
}

// Java: com/google/devtools/build/lib/remote/RemoteExecutionServiceTest.java:883 RemoteExecutionServiceTest.file
fn file(name: &str, content: &str) -> FileNode {
    FileNode {
        name: name.to_string(),
        digest: Some(digest_util().compute_as_utf8(content)),
        is_executable: true,
        ..Default::default()
    }
}

// Java: com/google/devtools/build/lib/remote/RemoteExecutionServiceTest.java:891 RemoteExecutionServiceTest.symlink
fn symlink(name: &str, target: &str) -> SymlinkNode {
    SymlinkNode {
        name: name.to_string(),
        target: target.to_string(),
        ..Default::default()
    }
}

/// A `FileNode` or a `SymlinkNode` (Java's `Message` list of `dir`).
enum Entry {
    File(FileNode),
    Symlink(SymlinkNode),
}

// Java: com/google/devtools/build/lib/remote/RemoteExecutionServiceTest.java:895 RemoteExecutionServiceTest.dir
fn dir(files_and_symlinks: Vec<Entry>, dirs: Vec<(&str, Directory)>) -> Directory {
    let mut builder = Directory::default();
    for entry in files_and_symlinks {
        match entry {
            Entry::File(file_node) => builder.files.push(file_node),
            Entry::Symlink(symlink_node) => builder.symlinks.push(symlink_node),
        }
    }
    for (name, dir) in dirs {
        builder.directories.push(DirectoryNode {
            name: name.to_string(),
            digest: Some(digest_util().compute_message(&dir)),
        });
    }
    builder
}

/// A deterministic shuffle (`Collections.shuffle(list, random)`).
///
/// PORT: Java's `RandomGeneratorFactory.getDefault()` (L32X64MixRandom) is not reproduced; the
/// test only checks that the order of the inputs does not matter, so any permutation per seed
/// serves.
fn shuffle<T>(list: &mut [T], seed: u64) {
    let mut state = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
    for i in (1..list.len()).rev() {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        let j = (state % (i as u64 + 1)) as usize;
        list.swap(i, j);
    }
}

// ---------------------------------------------------------------------------------------------
// buildRemoteAction
// ---------------------------------------------------------------------------------------------

// Java: com/google/devtools/build/lib/remote/RemoteExecutionServiceTest.java:271 RemoteExecutionServiceTest.buildRemoteAction_withRegularFileAsOutput
#[test]
fn build_remote_action_with_regular_file_as_output() {
    let f = Fixture::set_up();
    let exec_path = f.exec_fragment("path/to/tree");
    let spawn = SpawnBuilder::new(&["dummy"])
        .with_output(input(&create_artifact_with_exec_path(
            &f.artifact_root,
            exec_path.clone(),
        )))
        .build();
    let context = f.new_spawn_execution_context(&spawn);
    let service = f.new_remote_execution_service();

    let remote_action = service.build_remote_action(&*spawn, &context).unwrap();

    assert!(remote_action.get_command().output_files.is_empty());
    assert!(remote_action.get_command().output_directories.is_empty());
    assert_eq!(
        remote_action.get_command().output_paths,
        vec![exec_path.get_path_string().to_string()]
    );
}

// Java: com/google/devtools/build/lib/remote/RemoteExecutionServiceTest.java:288 RemoteExecutionServiceTest.legacy_buildRemoteAction_withRegularFileAsOutput
#[test]
fn legacy_build_remote_action_with_regular_file_as_output() {
    let f = Fixture::set_up();
    f.set_server_capabilities(legacy_remote_executor_capabilities());
    let exec_path = f.exec_fragment("path/to/tree");
    let spawn = SpawnBuilder::new(&["dummy"])
        .with_output(input(&create_artifact_with_exec_path(
            &f.artifact_root,
            exec_path.clone(),
        )))
        .build();
    let context = f.new_spawn_execution_context(&spawn);
    let service = f.new_remote_execution_service();

    let remote_action = service.build_remote_action(&*spawn, &context).unwrap();

    assert_eq!(
        remote_action.get_command().output_files,
        vec![exec_path.get_path_string().to_string()]
    );
    assert!(remote_action.get_command().output_directories.is_empty());
    assert!(remote_action.get_command().output_paths.is_empty());
}

// Java: com/google/devtools/build/lib/remote/RemoteExecutionServiceTest.java:307 RemoteExecutionServiceTest.buildRemoteAction_withTreeArtifactAsOutput
#[test]
fn build_remote_action_with_tree_artifact_as_output() {
    let f = Fixture::set_up();
    let spawn = SpawnBuilder::new(&["dummy"])
        .with_output(input(&create_tree_artifact_with_generating_action(
            &f.artifact_root,
            PathFragment::create("path/to/dir"),
        )))
        .build();
    let context = f.new_spawn_execution_context(&spawn);
    let service = f.new_remote_execution_service();

    let remote_action = service.build_remote_action(&*spawn, &context).unwrap();

    assert!(remote_action.get_command().output_files.is_empty());
    assert!(remote_action.get_command().output_directories.is_empty());
    assert_eq!(
        remote_action.get_command().output_paths,
        vec!["path/to/dir".to_string()]
    );
}

// Java: com/google/devtools/build/lib/remote/RemoteExecutionServiceTest.java:325 RemoteExecutionServiceTest.legacy_buildRemoteAction_withTreeArtifactAsOutput
#[test]
fn legacy_build_remote_action_with_tree_artifact_as_output() {
    let f = Fixture::set_up();
    f.set_server_capabilities(legacy_remote_executor_capabilities());
    let spawn = SpawnBuilder::new(&["dummy"])
        .with_output(input(&create_tree_artifact_with_generating_action(
            &f.artifact_root,
            PathFragment::create("path/to/dir"),
        )))
        .build();
    let context = f.new_spawn_execution_context(&spawn);
    let service = f.new_remote_execution_service();

    let remote_action = service.build_remote_action(&*spawn, &context).unwrap();

    assert!(remote_action.get_command().output_files.is_empty());
    assert_eq!(
        remote_action.get_command().output_directories,
        vec!["path/to/dir".to_string()]
    );
    assert!(remote_action.get_command().output_paths.is_empty());
}

// Java: com/google/devtools/build/lib/remote/RemoteExecutionServiceTest.java:345 RemoteExecutionServiceTest.buildRemoteAction_withUnresolvedSymlinkAsOutput
#[test]
fn build_remote_action_with_unresolved_symlink_as_output() {
    let f = Fixture::set_up();
    let spawn = SpawnBuilder::new(&["dummy"])
        .with_output(input(&create_unresolved_symlink_artifact_with_exec_path(
            &f.artifact_root,
            PathFragment::create("path/to/link"),
        )))
        .build();
    let context = f.new_spawn_execution_context(&spawn);
    let service = f.new_remote_execution_service();

    let remote_action = service.build_remote_action(&*spawn, &context).unwrap();

    assert!(remote_action.get_command().output_files.is_empty());
    assert!(remote_action.get_command().output_directories.is_empty());
    assert_eq!(
        remote_action.get_command().output_paths,
        vec!["path/to/link".to_string()]
    );
}

// Java: com/google/devtools/build/lib/remote/RemoteExecutionServiceTest.java:363 RemoteExecutionServiceTest.legacy_buildRemoteAction_withUnresolvedSymlinkAsOutput
#[test]
fn legacy_build_remote_action_with_unresolved_symlink_as_output() {
    let f = Fixture::set_up();
    f.set_server_capabilities(legacy_remote_executor_capabilities());
    let spawn = SpawnBuilder::new(&["dummy"])
        .with_output(input(&create_unresolved_symlink_artifact_with_exec_path(
            &f.artifact_root,
            PathFragment::create("path/to/link"),
        )))
        .build();
    let context = f.new_spawn_execution_context(&spawn);
    let service = f.new_remote_execution_service();

    let remote_action = service.build_remote_action(&*spawn, &context).unwrap();

    assert_eq!(
        remote_action.get_command().output_files,
        vec!["path/to/link".to_string()]
    );
    assert!(remote_action.get_command().output_directories.is_empty());
    assert!(remote_action.get_command().output_paths.is_empty());
}

// Java: com/google/devtools/build/lib/remote/RemoteExecutionServiceTest.java:383 RemoteExecutionServiceTest.buildRemoteAction_withActionInputAsOutput
#[test]
fn build_remote_action_with_action_input_as_output() {
    let f = Fixture::set_up();
    let spawn = SpawnBuilder::new(&["dummy"])
        .with_output(from_path("path/to/file"))
        .build();
    let context = f.new_spawn_execution_context(&spawn);
    let service = f.new_remote_execution_service();

    let remote_action = service.build_remote_action(&*spawn, &context).unwrap();

    assert!(remote_action.get_command().output_files.is_empty());
    assert!(remote_action.get_command().output_directories.is_empty());
    assert_eq!(
        remote_action.get_command().output_paths,
        vec!["path/to/file".to_string()]
    );
}

// Java: com/google/devtools/build/lib/remote/RemoteExecutionServiceTest.java:399 RemoteExecutionServiceTest.legacy_buildRemoteAction_withActionInputFileAsOutput
#[test]
fn legacy_build_remote_action_with_action_input_file_as_output() {
    let f = Fixture::set_up();
    f.set_server_capabilities(legacy_remote_executor_capabilities());
    let spawn = SpawnBuilder::new(&["dummy"])
        .with_output(from_path("path/to/file"))
        .build();
    let context = f.new_spawn_execution_context(&spawn);
    let service = f.new_remote_execution_service();

    let remote_action = service.build_remote_action(&*spawn, &context).unwrap();

    assert_eq!(
        remote_action.get_command().output_files,
        vec!["path/to/file".to_string()]
    );
    assert!(remote_action.get_command().output_directories.is_empty());
    assert!(remote_action.get_command().output_paths.is_empty());
}

// Java: com/google/devtools/build/lib/remote/RemoteExecutionServiceTest.java:417 RemoteExecutionServiceTest.buildRemoteAction_withActionInputDirectoryAsOutput
#[test]
fn build_remote_action_with_action_input_directory_as_output() {
    let f = Fixture::set_up();
    let spawn = SpawnBuilder::new(&["dummy"])
        .with_output(from_path("path/to/dir"))
        .build();
    let context = f.new_spawn_execution_context(&spawn);
    let service = f.new_remote_execution_service();

    let remote_action = service.build_remote_action(&*spawn, &context).unwrap();

    assert!(remote_action.get_command().output_files.is_empty());
    assert!(remote_action.get_command().output_directories.is_empty());
    assert_eq!(
        remote_action.get_command().output_paths,
        vec!["path/to/dir".to_string()]
    );
}

// Java: com/google/devtools/build/lib/remote/RemoteExecutionServiceTest.java:433 RemoteExecutionServiceTest.buildRemoteAction_generateActionSalt_differentiateWorkspaceCache
#[test]
fn build_remote_action_generate_action_salt_differentiate_workspace_cache() {
    let f = Fixture::set_up();
    let spawn = SpawnBuilder::new(&["dummy"])
        .with_execution_info(execution_requirements::DIFFERENTIATE_WORKSPACE_CACHE, "aa")
        .build();
    let context = f.new_spawn_execution_context(&spawn);
    let service = f.new_remote_execution_service();

    let remote_action = service.build_remote_action(&*spawn, &context).unwrap();

    let expected = CacheSalt {
        may_be_executed_remotely: true,
        workspace: "aa".to_string(),
        ..Default::default()
    };
    assert_eq!(remote_action.get_action().salt, expected.to_byte_array());
}

// Java: com/google/devtools/build/lib/remote/RemoteExecutionServiceTest.java:449 RemoteExecutionServiceTest.buildRemoteAction_generateActionSalt_noRemoteExec
#[test]
fn build_remote_action_generate_action_salt_no_remote_exec() {
    let f = Fixture::set_up();
    let spawn = SpawnBuilder::new(&["dummy"])
        .with_execution_info(execution_requirements::NO_REMOTE_EXEC, "")
        .build();
    let context = f.new_spawn_execution_context(&spawn);
    let service = f.new_remote_execution_service();

    let remote_action = service.build_remote_action(&*spawn, &context).unwrap();

    let expected = CacheSalt {
        may_be_executed_remotely: false,
        ..Default::default()
    };
    assert_eq!(remote_action.get_action().salt, expected.to_byte_array());
}

/// `buildRemoteAction_goldenTest(@TestParameter({"1", "2", "3"}) int seed)`.
///
/// PORT: the JOL check of the retained size of an uploadable Merkle tree (seed 1) measures Java
/// heap objects and has no counterpart.
// Java: com/google/devtools/build/lib/remote/RemoteExecutionServiceTest.java:464 RemoteExecutionServiceTest.buildRemoteAction_goldenTest
fn build_remote_action_golden_test(seed: u64) {
    let mut f = Fixture::set_up();
    let mut inputs: Vec<BlazeArtifact> = Vec::new();

    // PORT: StringEncoding identity (`unicodeToInternal`).
    let files = [
        "root_file1",
        "root_file2",
        "root_file3",
        "dir/subdir/file1",
        "dir/subdir/file2",
        "dir/subdir/file3",
        "dir/subdir/subdir2/file1",
        "dir/subdir/subdir2/file2",
        "dir/subdir/subdir2/file3",
        "dir/file1",
        "dir/file2",
        "dir/file3",
        // These paths sort differently depending on whether they are sorted as Java Unicode
        // or Bazel internal strings.
        "path/ﾐ",
        "path/🔥",
        // These paths sort differently depending on whether they are sorted as Strings or as
        // PathFragments.
        "srcs/system/foo.txt",
        "srcs/system-root/bar.txt",
    ];
    for file in files {
        let artifact = create_artifact_str(&f.artifact_root, file);
        f.fake_file_cache
            .create_scratch_input(&input(&artifact), &format!("content of {file}"))
            .unwrap();
        inputs.push(artifact);
    }

    let tree_artifact_input = create_tree_artifact_with_generating_action_str(
        &f.artifact_root,
        "dir/subdir/tree_artifact",
    );
    let mut tree_children: Vec<(ActionInputRef, FileArtifactValue)> = Vec::new();
    for file in files {
        let tree_file_artifact = BlazeArtifact::create_tree_output_str(&tree_artifact_input, file);
        let digest = f
            .fake_file_cache
            .create_scratch_input(
                &input(&tree_file_artifact),
                &format!("content of tree file {file}"),
            )
            .unwrap();
        tree_children.push((
            input(&tree_file_artifact),
            FileArtifactValue::create_for_normal_file(
                Some(DigestUtil::to_binary_digest(&digest)),
                None,
                digest.size_bytes,
            ),
        ));
    }
    f.fake_file_cache.add_tree_artifact(
        &input(&tree_artifact_input),
        TreeArtifactValue::build(tree_children, None, None),
    );
    inputs.push(tree_artifact_input);

    let empty_dir = create_tree_artifact_with_generating_action_str(&f.artifact_root, "empty_dir");
    f.fake_file_cache.add_tree_artifact(
        &input(&empty_dir),
        TreeArtifactValue::build(Vec::new(), None, None),
    );
    inputs.push(empty_dir);
    // Add an artifact with the same path but different owner to verify that the directory isn't
    // duplicated in the Merkle tree.
    // PORT: blaze_exec's `ActionInputRef` compares inputs by kind and exec path (not owner), so
    // the fake's map and the spawn's input set hold one entry for both artifacts.
    let exec_path = f
        .artifact_root
        .get_exec_path()
        .get_relative(&PathFragment::create("empty_dir"));
    let yet_another_null_artifact_owner = ActionLookupKeyRef::new(YetAnotherNullArtifactOwner);
    let empty_dir_with_different_owner = BlazeArtifact::create_special_artifact(
        f.artifact_root.clone(),
        exec_path,
        yet_another_null_artifact_owner.clone(),
        SpecialArtifactType::TREE,
    );
    empty_dir_with_different_owner
        .set_generating_action_key(ActionLookupData::create(yet_another_null_artifact_owner, 0));
    f.fake_file_cache.add_tree_artifact(
        &input(&empty_dir_with_different_owner),
        TreeArtifactValue::build(Vec::new(), None, None),
    );
    inputs.push(empty_dir_with_different_owner);

    let unresolved_symlink = create_unresolved_symlink_artifact(&f.artifact_root, "dir/some_link");
    f.fake_file_cache
        .create_scratch_input_symlink(&input(&unresolved_symlink), "some/target")
        .unwrap();
    inputs.push(unresolved_symlink);

    let runfiles_tree_root = f
        .artifact_root
        .get_exec_path()
        .get_relative(&PathFragment::create("dir/my_tool.runfiles"));
    let runfiles_tree =
        create_runfiles_artifact(&f.artifact_root, runfiles_tree_root.get_path_string());
    f.fake_file_cache.add_runfiles_tree(
        &input(&runfiles_tree),
        Arc::new(TestRunfilesTree::new(
            runfiles_tree_root.get_path_string(),
            &inputs,
        )),
    );
    inputs.push(runfiles_tree);

    let output_dir =
        create_tree_artifact_with_generating_action_str(&f.artifact_root, "dir/output_dir");

    let src_dir = create_artifact_with_exec_path(&f.source_root, PathFragment::create("src_dir"));
    let _unused = f
        .fake_file_cache
        .create_scratch_input_directory(
            &input(&src_dir),
            &Tree {
                root: Some(dir(
                    vec![
                        f_entry("file1", "content of src_dir/file1"),
                        f_entry("file2", "content of src_dir/file2"),
                        f_entry("file3", "content of src_dir/file3"),
                    ],
                    vec![],
                )),
                children: Vec::new(),
            },
        )
        .unwrap();
    inputs.push(src_dir.clone());
    let src_file1 =
        create_artifact_with_exec_path(&f.source_root, src_dir.get_exec_path().get_child("file1"));
    f.fake_file_cache
        .create_scratch_input(&input(&src_file1), "content of src_dir/file1")
        .unwrap();
    inputs.push(src_file1);
    let src_file2 =
        create_artifact_with_exec_path(&f.source_root, src_dir.get_exec_path().get_child("file2"));
    f.fake_file_cache
        .create_scratch_input(&input(&src_file2), "content of src_dir/file2")
        .unwrap();
    inputs.push(src_file2);
    let src_file3 =
        create_artifact_with_exec_path(&f.source_root, src_dir.get_exec_path().get_child("file3"));
    f.fake_file_cache
        .create_scratch_input(&input(&src_file3), "content of src_dir/file3")
        .unwrap();
    // Explicitly don't add srcFile3 to inputs so that srcDir overlaps non-trivially with input
    // files.

    // Verify that the order of inputs does not affect the result.
    shuffle(&mut inputs, seed);
    let spawn = SpawnBuilder::new(&["my", "args"])
        .with_inputs(inputs.iter().map(input))
        .with_output(input(&output_dir))
        .build();
    let context = f.new_spawn_execution_context(&spawn);
    f.remote_options.remote_discard_merkle_trees = false;
    let service = f.new_remote_execution_service_with_options(&f.remote_options);

    let empty_directory = || dir(vec![], vec![]);
    let tree_artifact_directory = dir(
        vec![
            f_entry("root_file1", "content of tree file root_file1"),
            f_entry("root_file2", "content of tree file root_file2"),
            f_entry("root_file3", "content of tree file root_file3"),
        ],
        vec![
            (
                "dir",
                dir(
                    vec![
                        f_entry("file1", "content of tree file dir/file1"),
                        f_entry("file2", "content of tree file dir/file2"),
                        f_entry("file3", "content of tree file dir/file3"),
                    ],
                    vec![(
                        "subdir",
                        dir(
                            vec![
                                f_entry("file1", "content of tree file dir/subdir/file1"),
                                f_entry("file2", "content of tree file dir/subdir/file2"),
                                f_entry("file3", "content of tree file dir/subdir/file3"),
                            ],
                            vec![(
                                "subdir2",
                                dir(
                                    vec![
                                        f_entry(
                                            "file1",
                                            "content of tree file dir/subdir/subdir2/file1",
                                        ),
                                        f_entry(
                                            "file2",
                                            "content of tree file dir/subdir/subdir2/file2",
                                        ),
                                        f_entry(
                                            "file3",
                                            "content of tree file dir/subdir/subdir2/file3",
                                        ),
                                    ],
                                    vec![],
                                ),
                            )],
                        ),
                    )],
                ),
            ),
            (
                "path",
                dir(
                    vec![
                        f_entry("ﾐ", "content of tree file path/ﾐ"),
                        f_entry("🔥", "content of tree file path/🔥"),
                    ],
                    vec![],
                ),
            ),
            (
                "srcs",
                dir(
                    vec![],
                    vec![
                        (
                            "system",
                            dir(
                                vec![f_entry(
                                    "foo.txt",
                                    "content of tree file srcs/system/foo.txt",
                                )],
                                vec![],
                            ),
                        ),
                        (
                            "system-root",
                            dir(
                                vec![f_entry(
                                    "bar.txt",
                                    "content of tree file srcs/system-root/bar.txt",
                                )],
                                vec![],
                            ),
                        ),
                    ],
                ),
            ),
        ],
    );
    let srcs_directory = || {
        dir(
            vec![],
            vec![
                (
                    "system",
                    dir(
                        vec![f_entry("foo.txt", "content of srcs/system/foo.txt")],
                        vec![],
                    ),
                ),
                (
                    "system-root",
                    dir(
                        vec![f_entry("bar.txt", "content of srcs/system-root/bar.txt")],
                        vec![],
                    ),
                ),
            ],
        )
    };
    let path_directory = || {
        dir(
            vec![
                f_entry("ﾐ", "content of path/ﾐ"),
                f_entry("🔥", "content of path/🔥"),
            ],
            vec![],
        )
    };
    let subdir2_directory = || {
        dir(
            vec![
                f_entry("file1", "content of dir/subdir/subdir2/file1"),
                f_entry("file2", "content of dir/subdir/subdir2/file2"),
                f_entry("file3", "content of dir/subdir/subdir2/file3"),
            ],
            vec![],
        )
    };
    let subdir_directory = || {
        dir(
            vec![
                f_entry("file1", "content of dir/subdir/file1"),
                f_entry("file2", "content of dir/subdir/file2"),
                f_entry("file3", "content of dir/subdir/file3"),
            ],
            vec![
                ("subdir2", subdir2_directory()),
                ("tree_artifact", tree_artifact_directory.clone()),
            ],
        )
    };
    let runfiles_directory = dir(
        vec![],
        vec![(
            WORKSPACE_NAME,
            dir(
                vec![
                    f_entry("root_file1", "content of root_file1"),
                    f_entry("root_file2", "content of root_file2"),
                    f_entry("root_file3", "content of root_file3"),
                ],
                vec![
                    (
                        "dir",
                        dir(
                            vec![
                                f_entry("file1", "content of dir/file1"),
                                f_entry("file2", "content of dir/file2"),
                                f_entry("file3", "content of dir/file3"),
                                Entry::Symlink(symlink("some_link", "some/target")),
                            ],
                            vec![("subdir", subdir_directory())],
                        ),
                    ),
                    ("empty_dir", empty_directory()),
                    ("path", path_directory()),
                    ("srcs", srcs_directory()),
                ],
            ),
        )],
    );
    let dir_directory = dir(
        vec![
            f_entry("file1", "content of dir/file1"),
            f_entry("file2", "content of dir/file2"),
            f_entry("file3", "content of dir/file3"),
            Entry::Symlink(symlink("some_link", "some/target")),
        ],
        vec![
            ("my_tool.runfiles", runfiles_directory),
            ("output_dir", empty_directory()),
            ("subdir", subdir_directory()),
        ],
    );
    let root_directory = dir(
        vec![],
        vec![
            (
                "outputs",
                dir(
                    vec![
                        f_entry("root_file1", "content of root_file1"),
                        f_entry("root_file2", "content of root_file2"),
                        f_entry("root_file3", "content of root_file3"),
                    ],
                    vec![
                        ("dir", dir_directory),
                        ("empty_dir", empty_directory()),
                        ("path", path_directory()),
                        ("srcs", srcs_directory()),
                    ],
                ),
            ),
            (
                "src_dir",
                dir(
                    vec![
                        f_entry("file1", "content of src_dir/file1"),
                        f_entry("file2", "content of src_dir/file2"),
                        f_entry("file3", "content of src_dir/file3"),
                    ],
                    vec![],
                ),
            ),
        ],
    );

    // The product name and the main workspace name differ between Bazel and Blaze, both of them
    // making their way into paths (`TestConstants.PRODUCT_NAME` is "bazel").
    let expected_digest = DigestUtil::from_string(
        "ff6cfaefd3fe05996e6e06e818ff462b7e5632a730e48111a8643bbf58d6e01f/164",
    );

    assert_eq!(
        f.digest_util.compute_message(&root_directory),
        expected_digest
    );

    // Verify that multiple concurrent Merkle tree builds all produce the same result and don't
    // interfere with each other.
    let mut exceptions: Vec<String> = Vec::new();
    std::thread::scope(|s| {
        let handles: Vec<_> = (0..16)
            .map(|_| {
                s.spawn(|| {
                    let remote_action = service.build_remote_action(&*spawn, &context).unwrap();
                    assert_eq!(remote_action.get_merkle_tree().digest(), &expected_digest);
                })
            })
            .collect();
        for handle in handles {
            if let Err(e) = handle.join() {
                exceptions.push(panic_message(&*e));
            }
        }
    });
    if !exceptions.is_empty() {
        panic!("Exceptions in golden test runs: {exceptions:?}");
    }
}

/// `file(name, content)` as an entry of `dir`.
fn f_entry(name: &str, content: &str) -> Entry {
    Entry::File(file(name, content))
}

// Java: com/google/devtools/build/lib/remote/RemoteExecutionServiceTest.java:464 RemoteExecutionServiceTest.buildRemoteAction_goldenTest
#[test]
fn build_remote_action_golden_test_seed_1() {
    build_remote_action_golden_test(1);
}

// Java: com/google/devtools/build/lib/remote/RemoteExecutionServiceTest.java:464 RemoteExecutionServiceTest.buildRemoteAction_goldenTest
#[test]
fn build_remote_action_golden_test_seed_2() {
    build_remote_action_golden_test(2);
}

// Java: com/google/devtools/build/lib/remote/RemoteExecutionServiceTest.java:464 RemoteExecutionServiceTest.buildRemoteAction_goldenTest
#[test]
fn build_remote_action_golden_test_seed_3() {
    build_remote_action_golden_test(3);
}

// ---------------------------------------------------------------------------------------------
// downloadOutputs
// ---------------------------------------------------------------------------------------------

// Java: com/google/devtools/build/lib/remote/RemoteExecutionServiceTest.java:914 RemoteExecutionServiceTest.downloadOutputs_executableBitIgnored
#[test]
fn download_outputs_executable_bit_ignored() {
    // Test that executable bit of downloaded output files are ignored since it will be chmod 555
    // after action execution.

    // arrange
    let mut f = Fixture::set_up();
    let foo_digest = f.add_contents_str("foo-contents");
    let bar_digest = f.add_contents_str("bar-contents");
    let tree = Tree {
        root: Some(Directory {
            files: vec![FileNode {
                name: "bar".to_string(),
                digest: Some(bar_digest),
                is_executable: true,
                ..Default::default()
            }],
            ..Default::default()
        }),
        children: Vec::new(),
    };
    let tree_digest = f.add_contents(&tree.to_byte_array());
    let builder = ActionResult {
        output_files: vec![OutputFile {
            path: "outputs/foo".to_string(),
            digest: Some(foo_digest),
            ..Default::default()
        }],
        output_directories: vec![OutputDirectory {
            path: "outputs/dir".to_string(),
            tree_digest: Some(tree_digest),
            ..Default::default()
        }],
        ..Default::default()
    };
    let result = from_cache(builder);
    let spawn = f.new_spawn_from_result(&result);
    let context = f.new_spawn_execution_context(&spawn);
    f.outputs_to_download = OutputsToDownload::All;
    let service = f.new_remote_execution_service();
    let action = service.build_remote_action(&*spawn, &context).unwrap();
    f.create_output_directories(&spawn);

    // act
    service.download_outputs(&action, &result).unwrap();

    // assert
    assert!(!f.exec_path("outputs/foo").is_executable().unwrap());
    assert!(!f.exec_path("outputs/dir/bar").is_executable().unwrap());
    assert!(context.is_lock_output_files_called());
}

// Java: com/google/devtools/build/lib/remote/RemoteExecutionServiceTest.java:955 RemoteExecutionServiceTest.downloadOutputs_siblingLayout
#[test]
fn download_outputs_sibling_layout() {
    // arrange
    let mut f = Fixture::set_up();
    f.remote_path_resolver = Arc::new(SiblingRepositoryLayoutResolver::new(f.exec_root.clone()));

    let foo_digest = f.add_contents_str("foo-contents");
    let bar_digest = f.add_contents_str("bar-contents");
    let builder = ActionResult {
        output_files: vec![
            OutputFile {
                path: "outputs/foo".to_string(),
                digest: Some(foo_digest),
                ..Default::default()
            },
            OutputFile {
                path: "outputs/bar".to_string(),
                digest: Some(bar_digest),
                ..Default::default()
            },
        ],
        ..Default::default()
    };
    let result = from_cache(builder);
    let spawn = f.new_spawn_from_result(&result);
    let context = f.new_spawn_execution_context(&spawn);
    f.outputs_to_download = OutputsToDownload::All;
    let service = f.new_remote_execution_service();
    let action = service.build_remote_action(&*spawn, &context).unwrap();
    f.create_output_directories(&spawn);

    // act
    service.download_outputs(&action, &result).unwrap();

    // assert
    assert_eq!(read_content(&f.exec_path("outputs/foo")), "foo-contents");
    assert_eq!(read_content(&f.exec_path("outputs/bar")), "bar-contents");
    assert!(context.is_lock_output_files_called());
}

// Java: com/google/devtools/build/lib/remote/RemoteExecutionServiceTest.java:984 RemoteExecutionServiceTest.downloadOutputs_outputFiles
#[test]
fn download_outputs_output_files() {
    // arrange
    let mut f = Fixture::set_up();
    let d1 = f.add_contents_str("content1");
    let r = ActionResult {
        exit_code: 0,
        output_files: vec![OutputFile {
            path: "outputs/file1".to_string(),
            digest: Some(d1.clone()),
            ..Default::default()
        }],
        ..Default::default()
    };

    let result = from_cache(r);
    let spawn = f.new_spawn_from_result(&result);
    let context = f.new_spawn_execution_context(&spawn);
    f.outputs_to_download = OutputsToDownload::All;
    let service = f.new_remote_execution_service();
    let action = service.build_remote_action(&*spawn, &context).unwrap();

    // act
    let in_memory_output = service.download_outputs(&action, &result).unwrap();

    // assert
    assert!(in_memory_output.is_none());
    assert_eq!(
        f.action_fs_get_digest(&f.exec_fragment("outputs/file1")),
        Some(DigestUtil::to_binary_digest(&d1))
    );
    assert_eq!(read_content(&f.exec_path("outputs/file1")), "content1");
    assert!(context.is_lock_output_files_called());
}

// Java: com/google/devtools/build/lib/remote/RemoteExecutionServiceTest.java:1014 RemoteExecutionServiceTest.downloadOutputs_outputDirectories
#[test]
fn download_outputs_output_directories() {
    // Test that downloading an output directory works.

    // arrange
    let mut f = Fixture::set_up();
    let foo_digest = f.add_contents_str("foo-contents");
    let bar_digest = f.add_contents_str("bar-contents");
    let tree = Tree {
        root: Some(Directory {
            files: vec![
                FileNode {
                    name: "foo".to_string(),
                    digest: Some(foo_digest.clone()),
                    is_executable: true,
                    ..Default::default()
                },
                FileNode {
                    name: "subdir/bar".to_string(),
                    digest: Some(bar_digest.clone()),
                    is_executable: true,
                    ..Default::default()
                },
            ],
            ..Default::default()
        }),
        children: Vec::new(),
    };
    let tree_digest = f.add_contents(&tree.to_byte_array());
    let builder = ActionResult {
        output_directories: vec![OutputDirectory {
            path: "outputs/a/dir".to_string(),
            tree_digest: Some(tree_digest),
            ..Default::default()
        }],
        ..Default::default()
    };
    let result = from_cache(builder);
    let spawn = f.new_spawn_from_result(&result);
    let context = f.new_spawn_execution_context(&spawn);
    f.outputs_to_download = OutputsToDownload::All;
    let service = f.new_remote_execution_service();
    let action = service.build_remote_action(&*spawn, &context).unwrap();

    // act
    service.download_outputs(&action, &result).unwrap();

    // assert
    assert_eq!(
        f.action_fs_get_digest(&f.exec_fragment("outputs/a/dir/foo")),
        Some(DigestUtil::to_binary_digest(&foo_digest))
    );
    assert_eq!(
        f.action_fs_get_digest(&f.exec_fragment("outputs/a/dir/subdir/bar")),
        Some(DigestUtil::to_binary_digest(&bar_digest))
    );
    assert_eq!(
        read_content(&f.exec_path("outputs/a/dir/foo")),
        "foo-contents"
    );
    assert_eq!(
        read_content(&f.exec_path("outputs/a/dir/subdir/bar")),
        "bar-contents"
    );
    assert!(context.is_lock_output_files_called());
}

// Java: com/google/devtools/build/lib/remote/RemoteExecutionServiceTest.java:1064 RemoteExecutionServiceTest.downloadOutputs_emptyOutputDirectories_works
#[test]
fn download_outputs_empty_output_directories_works() {
    // Test that downloading an empty output directory works.

    // arrange
    let mut f = Fixture::set_up();
    let bar_tree_message = Tree {
        root: Some(Directory::default()),
        children: Vec::new(),
    };
    // Don't add barTreeMessage to the cache, the Tree proto for an empty output directory is
    // recognized by its digest.
    let bar_tree_digest = f.digest_util.compute_message(&bar_tree_message);
    let builder = ActionResult {
        output_directories: vec![OutputDirectory {
            path: "outputs/a/bar".to_string(),
            tree_digest: Some(bar_tree_digest),
            ..Default::default()
        }],
        ..Default::default()
    };
    let result = from_cache(builder);
    let spawn = f.new_spawn_from_result(&result);
    let context = f.new_spawn_execution_context(&spawn);
    f.outputs_to_download = OutputsToDownload::All;
    let service = f.new_remote_execution_service();
    let action = service.build_remote_action(&*spawn, &context).unwrap();
    f.create_output_directories(&spawn);

    // act
    service.download_outputs(&action, &result).unwrap();

    // assert
    assert!(f.exec_path("outputs/a/bar").is_directory());
    assert!(context.is_lock_output_files_called());
}

// Java: com/google/devtools/build/lib/remote/RemoteExecutionServiceTest.java:1093 RemoteExecutionServiceTest.downloadOutputs_nestedOutputDirectories_works
#[test]
fn download_outputs_nested_output_directories_works() {
    // Test that downloading a nested output directory works.

    // arrange
    let mut f = Fixture::set_up();
    let foo_digest = f.add_contents_str("foo-contents");
    let qux_digest = f.add_contents_str("qux-contents");
    let wobble_dir_message = Directory {
        files: vec![FileNode {
            name: "qux".to_string(),
            digest: Some(qux_digest.clone()),
            is_executable: true,
            ..Default::default()
        }],
        ..Default::default()
    };
    let wobble_dir_digest = f.add_contents(&wobble_dir_message.to_byte_array());
    let bar_tree_message = Tree {
        root: Some(Directory {
            files: vec![FileNode {
                name: "qux".to_string(),
                digest: Some(qux_digest),
                is_executable: true,
                ..Default::default()
            }],
            directories: vec![DirectoryNode {
                name: "wobble".to_string(),
                digest: Some(wobble_dir_digest),
            }],
            ..Default::default()
        }),
        children: vec![wobble_dir_message],
    };
    let bar_tree_digest = f.add_contents(&bar_tree_message.to_byte_array());
    let builder = ActionResult {
        output_files: vec![OutputFile {
            path: "outputs/a/foo".to_string(),
            digest: Some(foo_digest),
            ..Default::default()
        }],
        output_directories: vec![OutputDirectory {
            path: "outputs/a/bar".to_string(),
            tree_digest: Some(bar_tree_digest),
            ..Default::default()
        }],
        ..Default::default()
    };
    let result = from_cache(builder);
    let spawn = f.new_spawn_from_result(&result);
    let context = f.new_spawn_execution_context(&spawn);
    f.outputs_to_download = OutputsToDownload::All;
    let service = f.new_remote_execution_service();
    let action = service.build_remote_action(&*spawn, &context).unwrap();
    f.create_output_directories(&spawn);

    // act
    service.download_outputs(&action, &result).unwrap();

    // assert
    assert_eq!(read_content(&f.exec_path("outputs/a/foo")), "foo-contents");
    assert_eq!(
        read_content(&f.exec_path("outputs/a/bar/wobble/qux")),
        "qux-contents"
    );
    assert!(context.is_lock_output_files_called());
}

// Java: com/google/devtools/build/lib/remote/RemoteExecutionServiceTest.java:1145 RemoteExecutionServiceTest.downloadOutputs_outputDirectoriesWithNestedFile_works
#[test]
fn download_outputs_output_directories_with_nested_file_works() {
    // Test that downloading an output directory containing a named output file works.

    // arrange
    let mut f = Fixture::set_up();
    let foo_digest = f.add_contents_str("foo-contents");
    let bar_digest = f.add_contents_str("bar-contents");
    let subdir_tree_message = Tree {
        root: Some(Directory {
            files: vec![
                FileNode {
                    name: "foo".to_string(),
                    digest: Some(foo_digest.clone()),
                    is_executable: true,
                    ..Default::default()
                },
                FileNode {
                    name: "bar".to_string(),
                    digest: Some(bar_digest),
                    is_executable: true,
                    ..Default::default()
                },
            ],
            ..Default::default()
        }),
        children: Vec::new(),
    };
    let subdir_tree_digest = f.add_contents(&subdir_tree_message.to_byte_array());
    let builder = ActionResult {
        output_files: vec![OutputFile {
            path: "outputs/subdir/foo".to_string(),
            digest: Some(foo_digest),
            ..Default::default()
        }],
        output_directories: vec![OutputDirectory {
            path: "outputs/subdir".to_string(),
            tree_digest: Some(subdir_tree_digest),
            ..Default::default()
        }],
        ..Default::default()
    };
    let result = from_cache(builder);
    let spawn = f.new_spawn_from_result(&result);
    let context = f.new_spawn_execution_context(&spawn);
    f.outputs_to_download = OutputsToDownload::All;
    let service = f.new_remote_execution_service();
    let action = service.build_remote_action(&*spawn, &context).unwrap();
    f.create_output_directories(&spawn);

    // act
    service.download_outputs(&action, &result).unwrap();

    // assert
    assert_eq!(
        read_content(&f.exec_path("outputs/subdir/foo")),
        "foo-contents"
    );
    assert_eq!(
        read_content(&f.exec_path("outputs/subdir/bar")),
        "bar-contents"
    );
    assert!(context.is_lock_output_files_called());
}

// Java: com/google/devtools/build/lib/remote/RemoteExecutionServiceTest.java:1193 RemoteExecutionServiceTest.downloadOutputs_outputDirectoriesWithSameHash_works
#[test]
fn download_outputs_output_directories_with_same_hash_works() {
    // Test that downloading an output directory works when two Directory
    // protos have the same hash i.e. because they have the same name and contents or are empty.

    /*
     * /bar/foo/file
     * /foo/file
     */

    // arrange
    let mut f = Fixture::set_up();
    let file_digest = f.add_contents_str("file");
    let file = FileNode {
        name: "file".to_string(),
        digest: Some(file_digest),
        is_executable: true,
        ..Default::default()
    };
    let foo_dir = Directory {
        files: vec![file],
        ..Default::default()
    };
    let foo_digest = f.add_contents(&foo_dir.to_byte_array());
    let foo_dir_node = DirectoryNode {
        name: "foo".to_string(),
        digest: Some(foo_digest),
    };
    let bar_dir = Directory {
        directories: vec![foo_dir_node.clone()],
        ..Default::default()
    };
    let bar_digest = f.add_contents(&bar_dir.to_byte_array());
    let bar_dir_node = DirectoryNode {
        name: "bar".to_string(),
        digest: Some(bar_digest),
    };
    let root_dir = Directory {
        directories: vec![foo_dir_node, bar_dir_node],
        ..Default::default()
    };
    let tree = Tree {
        root: Some(root_dir),
        children: vec![bar_dir, foo_dir.clone(), foo_dir],
    };
    let tree_digest = f.add_contents(&tree.to_byte_array());
    let builder = ActionResult {
        output_directories: vec![OutputDirectory {
            path: "outputs/a".to_string(),
            tree_digest: Some(tree_digest),
            ..Default::default()
        }],
        ..Default::default()
    };
    let result = from_cache(builder);
    let spawn = f.new_spawn_from_result(&result);
    let context = f.new_spawn_execution_context(&spawn);
    f.outputs_to_download = OutputsToDownload::All;
    let service = f.new_remote_execution_service();
    let action = service.build_remote_action(&*spawn, &context).unwrap();
    f.create_output_directories(&spawn);

    // act
    service.download_outputs(&action, &result).unwrap();

    // assert
    assert_eq!(read_content(&f.exec_path("outputs/a/bar/foo/file")), "file");
    assert_eq!(read_content(&f.exec_path("outputs/a/foo/file")), "file");
    assert!(context.is_lock_output_files_called());
}

// Java: com/google/devtools/build/lib/remote/RemoteExecutionServiceTest.java:1247 RemoteExecutionServiceTest.downloadOutputs_outputDirectoriesWithUnicodeFilenames_works
#[test]
fn download_outputs_output_directories_with_unicode_filenames_works() {
    // arrange
    let mut f = Fixture::set_up();
    let internationalization_digest = f.add_contents_str("hello");
    let tree = Tree {
        root: Some(Directory {
            files: vec![FileNode {
                name: "Iñtërnâtiônàlizætiøn".to_string(),
                digest: Some(internationalization_digest),
                ..Default::default()
            }],
            symlinks: vec![SymlinkNode {
                name: "東京都".to_string(),
                target: "京都市".to_string(),
                ..Default::default()
            }],
            ..Default::default()
        }),
        children: Vec::new(),
    };
    let tree_digest = f.add_contents(&tree.to_byte_array());
    let builder = ActionResult {
        output_directories: vec![OutputDirectory {
            path: "outputs/dir".to_string(),
            tree_digest: Some(tree_digest),
            ..Default::default()
        }],
        ..Default::default()
    };
    let result = from_cache(builder);
    let spawn = f.new_spawn_from_result(&result);
    let context = f.new_spawn_execution_context(&spawn);
    f.outputs_to_download = OutputsToDownload::All;
    let service = f.new_remote_execution_service();
    let action = service.build_remote_action(&*spawn, &context).unwrap();
    f.create_output_directories(&spawn);

    // act
    service.download_outputs(&action, &result).unwrap();

    // assert
    // PORT: StringEncoding identity (`unicodeToInternal`).
    assert_eq!(
        read_content(&f.exec_path("outputs/dir/Iñtërnâtiônàlizætiøn")),
        "hello"
    );
    assert_eq!(
        f.exec_path("outputs/dir/東京都")
            .read_symbolic_link()
            .unwrap(),
        PathFragment::create("京都市")
    );
    assert!(context.is_lock_output_files_called());
}

// Java: com/google/devtools/build/lib/remote/RemoteExecutionServiceTest.java:1287 RemoteExecutionServiceTest.downloadOutputs_relativeFileSymlink_success
#[test]
fn download_outputs_relative_file_symlink_success() {
    let f = Fixture::set_up();
    let builder = ActionResult {
        output_file_symlinks: vec![OutputSymlink {
            path: "outputs/a/b/link".to_string(),
            target: "../../foo".to_string(),
            ..Default::default()
        }],
        ..Default::default()
    };
    let result = from_cache(builder);
    let spawn = f.new_spawn_from_result(&result);
    let context = f.new_spawn_execution_context(&spawn);
    let service = f.new_remote_execution_service();
    let action = service.build_remote_action(&*spawn, &context).unwrap();

    // Doesn't check for dangling links, hence download succeeds.
    service.download_outputs(&action, &result).unwrap();

    let path = f.exec_path("outputs/a/b/link");
    assert!(path.is_symbolic_link());
    assert_eq!(
        path.read_symbolic_link().unwrap(),
        PathFragment::create("../../foo")
    );
    assert!(context.is_lock_output_files_called());
}

// Java: com/google/devtools/build/lib/remote/RemoteExecutionServiceTest.java:1307 RemoteExecutionServiceTest.downloadOutputs_relativeDirectorySymlink_success
#[test]
fn download_outputs_relative_directory_symlink_success() {
    let mut f = Fixture::set_up();
    let builder = ActionResult {
        output_directory_symlinks: vec![OutputSymlink {
            path: "outputs/a/b/link".to_string(),
            target: "foo".to_string(),
            ..Default::default()
        }],
        ..Default::default()
    };
    let result = from_cache(builder);
    let spawn = f.new_spawn_from_result(&result);
    let context = f.new_spawn_execution_context(&spawn);
    f.outputs_to_download = OutputsToDownload::All;
    let service = f.new_remote_execution_service();
    let action = service.build_remote_action(&*spawn, &context).unwrap();
    f.create_output_directories(&spawn);

    // Doesn't check for dangling links, hence download succeeds.
    service.download_outputs(&action, &result).unwrap();

    let path = f.exec_path("outputs/a/b/link");
    assert!(path.is_symbolic_link());
    assert_eq!(
        path.read_symbolic_link().unwrap(),
        PathFragment::create("foo")
    );
    assert!(context.is_lock_output_files_called());
}

// Java: com/google/devtools/build/lib/remote/RemoteExecutionServiceTest.java:1330 RemoteExecutionServiceTest.downloadOutputs_relativeOutputSymlinks_success
#[test]
fn download_outputs_relative_output_symlinks_success() {
    // Test that download outputs works when the action result only contains output_symlinks
    // and not output_file_symlinks or output_directory_symlinks, which are deprecated.
    let mut f = Fixture::set_up();
    let builder = ActionResult {
        output_symlinks: vec![OutputSymlink {
            path: "outputs/a/b/link".to_string(),
            target: "../../foo".to_string(),
            ..Default::default()
        }],
        ..Default::default()
    };
    let result = from_cache(builder);
    let spawn = f.new_spawn_from_result(&result);
    let context = f.new_spawn_execution_context(&spawn);
    f.outputs_to_download = OutputsToDownload::All;
    let service = f.new_remote_execution_service();
    let action = service.build_remote_action(&*spawn, &context).unwrap();
    f.create_output_directories(&spawn);

    // Doesn't check for dangling links, hence download succeeds.
    service.download_outputs(&action, &result).unwrap();

    let path = f.exec_path("outputs/a/b/link");
    assert!(path.is_symbolic_link());
    assert_eq!(
        path.read_symbolic_link().unwrap(),
        PathFragment::create("../../foo")
    );
    assert!(context.is_lock_output_files_called());
}

// Java: com/google/devtools/build/lib/remote/RemoteExecutionServiceTest.java:1355 RemoteExecutionServiceTest.downloadOutputs_outputSymlinksCompatibility_success
#[test]
fn download_outputs_output_symlinks_compatibility_success() {
    // Test that download outputs works when the action result contains both output_symlinks
    // and output_file_symlinks (or output_directory_symlinks).
    //
    // Remote Execution Server may set both fields to ensure backward compatibility with
    // clients that don't support output_symlinks.
    let mut f = Fixture::set_up();
    let builder = ActionResult {
        output_file_symlinks: vec![OutputSymlink {
            path: "outputs/a/b/link".to_string(),
            target: "foo".to_string(),
            ..Default::default()
        }],
        output_symlinks: vec![OutputSymlink {
            path: "outputs/a/b/link".to_string(),
            target: "foo".to_string(),
            ..Default::default()
        }],
        ..Default::default()
    };
    let result = from_cache(builder);
    let spawn = f.new_spawn_from_result(&result);
    let context = f.new_spawn_execution_context(&spawn);
    f.outputs_to_download = OutputsToDownload::All;
    let service = f.new_remote_execution_service();
    let action = service.build_remote_action(&*spawn, &context).unwrap();
    f.create_output_directories(&spawn);

    // Doesn't check for dangling links, hence download succeeds.
    service.download_outputs(&action, &result).unwrap();

    let path = f.exec_path("outputs/a/b/link");
    assert!(path.is_symbolic_link());
    assert_eq!(
        path.read_symbolic_link().unwrap(),
        PathFragment::create("foo")
    );
    assert!(context.is_lock_output_files_called());
}

// Java: com/google/devtools/build/lib/remote/RemoteExecutionServiceTest.java:1384 RemoteExecutionServiceTest.downloadOutputs_symlinksInDirectory_success
#[test]
fn download_outputs_symlinks_in_directory_success() {
    let mut f = Fixture::set_up();
    let tree = Tree {
        root: Some(Directory {
            symlinks: vec![
                SymlinkNode {
                    name: "rel".to_string(),
                    target: "foo".to_string(),
                    ..Default::default()
                },
                SymlinkNode {
                    name: "abs".to_string(),
                    target: "/bar".to_string(),
                    ..Default::default()
                },
            ],
            ..Default::default()
        }),
        children: Vec::new(),
    };
    let tree_digest = f.add_contents(&tree.to_byte_array());
    let builder = ActionResult {
        output_directories: vec![OutputDirectory {
            path: "outputs/dir".to_string(),
            tree_digest: Some(tree_digest),
            ..Default::default()
        }],
        ..Default::default()
    };
    let result = from_cache(builder);
    let spawn = f.new_spawn_from_result(&result);
    let context = f.new_spawn_execution_context(&spawn);
    f.outputs_to_download = OutputsToDownload::All;
    let service = f.new_remote_execution_service_with_options(&f.remote_options);
    let action = service.build_remote_action(&*spawn, &context).unwrap();
    f.create_output_directories(&spawn);

    // Doesn't check for dangling links, hence download succeeds.
    service.download_outputs(&action, &result).unwrap();

    let rel_path = f.exec_path("outputs/dir/rel");
    assert!(rel_path.is_symbolic_link());
    assert_eq!(
        rel_path.read_symbolic_link().unwrap(),
        PathFragment::create("foo")
    );
    let abs_path = f.exec_path("outputs/dir/abs");
    assert!(abs_path.is_symbolic_link());
    assert_eq!(
        abs_path.read_symbolic_link().unwrap(),
        PathFragment::create("/bar")
    );
    assert!(context.is_lock_output_files_called());
}

// Java: com/google/devtools/build/lib/remote/RemoteExecutionServiceTest.java:1418 RemoteExecutionServiceTest.downloadOutputs_absoluteFileSymlink_success
#[test]
fn download_outputs_absolute_file_symlink_success() {
    let mut f = Fixture::set_up();
    let builder = ActionResult {
        output_file_symlinks: vec![OutputSymlink {
            path: "outputs/foo".to_string(),
            target: "/abs/link".to_string(),
            ..Default::default()
        }],
        ..Default::default()
    };
    let result = from_cache(builder);
    let spawn = f.new_spawn_from_result(&result);
    let context = f.new_spawn_execution_context(&spawn);
    f.outputs_to_download = OutputsToDownload::All;
    let service = f.new_remote_execution_service();
    let action = service.build_remote_action(&*spawn, &context).unwrap();
    f.create_output_directories(&spawn);

    service.download_outputs(&action, &result).unwrap();

    let path = f.exec_path("outputs/foo");
    assert!(path.is_symbolic_link());
    assert_eq!(
        path.read_symbolic_link().unwrap(),
        PathFragment::create("/abs/link")
    );
    assert!(context.is_lock_output_files_called());
}

// Java: com/google/devtools/build/lib/remote/RemoteExecutionServiceTest.java:1440 RemoteExecutionServiceTest.downloadOutputs_absoluteDirectorySymlink_success
#[test]
fn download_outputs_absolute_directory_symlink_success() {
    let mut f = Fixture::set_up();
    let builder = ActionResult {
        output_directory_symlinks: vec![OutputSymlink {
            path: "outputs/foo".to_string(),
            target: "/abs/link".to_string(),
            ..Default::default()
        }],
        ..Default::default()
    };
    let result = from_cache(builder);
    let spawn = f.new_spawn_from_result(&result);
    let context = f.new_spawn_execution_context(&spawn);
    f.outputs_to_download = OutputsToDownload::All;
    let service = f.new_remote_execution_service();
    let action = service.build_remote_action(&*spawn, &context).unwrap();
    f.create_output_directories(&spawn);

    service.download_outputs(&action, &result).unwrap();

    let path = f.exec_path("outputs/foo");
    assert_eq!(
        path.read_symbolic_link().unwrap(),
        PathFragment::create("/abs/link")
    );
    assert!(context.is_lock_output_files_called());
}

// Java: com/google/devtools/build/lib/remote/RemoteExecutionServiceTest.java:1461 RemoteExecutionServiceTest.downloadOutputs_symlinkCollision_error
#[test]
fn download_outputs_symlink_collision_error() {
    let mut f = Fixture::set_up();
    let builder = ActionResult {
        output_directory_symlinks: vec![OutputSymlink {
            path: "outputs/foo".to_string(),
            target: "foo1".to_string(),
            ..Default::default()
        }],
        output_symlinks: vec![OutputSymlink {
            path: "outputs/foo".to_string(),
            target: "foo2".to_string(),
            ..Default::default()
        }],
        ..Default::default()
    };
    let result = from_cache(builder);
    let spawn = SpawnBuilder::new(&["dummy"])
        .with_output(input(&create_artifact_with_root_relative_path(
            &f.artifact_root,
            &PathFragment::create("foo"),
        )))
        .build();
    let context = f.new_spawn_execution_context(&spawn);
    f.outputs_to_download = OutputsToDownload::All;
    let service = f.new_remote_execution_service();
    let action = service.build_remote_action(&*spawn, &context).unwrap();

    let expected = expect_io_error(service.download_outputs(&action, &result));

    // `expected.getSuppressed()` is empty.
    assert!(errors::downcast_ref::<BulkTransferException>(&expected).is_none_or(|b| b.is_empty()));
    let message = expected.to_string();
    assert!(message.contains("Symlink path collision"), "{message}");
    assert!(message.contains("outputs/foo"), "{message}");
    assert!(message.contains("foo1"), "{message}");
    assert!(message.contains("foo2"), "{message}");
}

// Java: com/google/devtools/build/lib/remote/RemoteExecutionServiceTest.java:1490 RemoteExecutionServiceTest.downloadOutputs_onActionFailure_downloadEverything
#[test]
fn download_outputs_on_action_failure_download_everything() {
    // Test that all outputs are downloaded for a failed action, even if the outputs mode says
    // otherwise.

    // arrange
    let f = Fixture::set_up();
    let foo_digest = f.add_contents_str("foo-contents");
    let bar_digest = f.add_contents_str("bar-contents");
    let tree = Tree {
        root: Some(Directory {
            files: vec![FileNode {
                name: "bar".to_string(),
                digest: Some(bar_digest),
                is_executable: true,
                ..Default::default()
            }],
            ..Default::default()
        }),
        children: Vec::new(),
    };
    let tree_digest = f.add_contents(&tree.to_byte_array());
    let builder = ActionResult {
        output_files: vec![OutputFile {
            path: "outputs/foo".to_string(),
            digest: Some(foo_digest),
            ..Default::default()
        }],
        output_directories: vec![OutputDirectory {
            path: "outputs/dir".to_string(),
            tree_digest: Some(tree_digest),
            ..Default::default()
        }],
        exit_code: 1,
        ..Default::default()
    };
    let result = from_cache(builder);
    let spawn = f.new_spawn_from_result(&result);
    let context = f.new_spawn_execution_context(&spawn);
    let service = f.new_remote_execution_service();
    let action = service.build_remote_action(&*spawn, &context).unwrap();
    f.create_output_directories(&spawn);

    // act
    service.download_outputs(&action, &result).unwrap();

    // assert
    assert_eq!(read_content(&f.exec_path("outputs/foo")), "foo-contents");
    assert_eq!(
        read_content(&f.exec_path("outputs/dir/bar")),
        "bar-contents"
    );
    assert!(context.is_lock_output_files_called());
}

// Java: com/google/devtools/build/lib/remote/RemoteExecutionServiceTest.java:1531 RemoteExecutionServiceTest.downloadOutputs_onDownloadFailure_maintainDirectories
#[test]
fn download_outputs_on_download_failure_maintain_directories() {
    // Test that output directories created prior to spawn execution are not deleted on failure.
    let mut f = Fixture::set_up();
    let tree_file_digest = f.add_exception_str(
        "outputs/outputdir/outputfile",
        errors::io_exception("download failed"),
    );
    let tree = Tree {
        root: Some(Directory {
            files: vec![FileNode {
                name: "outputfile".to_string(),
                digest: Some(tree_file_digest),
                is_executable: true,
                ..Default::default()
            }],
            ..Default::default()
        }),
        children: Vec::new(),
    };
    let tree_digest = f.add_contents(&tree.to_byte_array());
    let other_file_digest = f.add_exception_str(
        "outputs/otherdir/otherfile",
        errors::io_exception("download failed"),
    );
    let builder = ActionResult {
        output_directories: vec![OutputDirectory {
            path: "outputs/outputdir".to_string(),
            tree_digest: Some(tree_digest),
            ..Default::default()
        }],
        output_files: vec![OutputFile {
            path: "outputs/otherdir/otherfile".to_string(),
            digest: Some(other_file_digest),
            ..Default::default()
        }],
        ..Default::default()
    };
    let result = from_cache(builder);
    let spawn = f.new_spawn_from_result(&result);
    let context = f.new_spawn_execution_context(&spawn);
    f.outputs_to_download = OutputsToDownload::All;
    let service = f.new_remote_execution_service();
    let action = service.build_remote_action(&*spawn, &context).unwrap();
    f.create_output_directories(&spawn);

    let _e = expect_bulk_transfer_exception(service.download_outputs(&action, &result));

    assert_eq!(f.in_memory_cache_client.get_num_failed_downloads(), 2);
    assert!(f.exec_path("outputs/outputdir").exists());
    assert!(!f.exec_path("outputs/outputdir/outputfile").exists());
    assert!(f.exec_path("outputs/otherdir").exists());
    assert!(!f.exec_path("outputs/otherdir/otherfile").exists());
    assert!(!context.is_lock_output_files_called());
}

// Java: com/google/devtools/build/lib/remote/RemoteExecutionServiceTest.java:1573 RemoteExecutionServiceTest.downloadOutputs_onError_waitForRemainingDownloadsToComplete
#[test]
fn download_outputs_on_error_wait_for_remaining_downloads_to_complete() {
    // If one or more downloads of output files / directories fail then the code should
    // wait for all downloads to have been completed before it tries to clean up partially
    // downloaded files.
    let mut f = Fixture::set_up();
    let digest1 = f.add_contents_str("file1");
    let digest2 = f.add_exception_str("file2", errors::io_exception("download failed"));
    let digest3 = f.add_contents_str("file3");
    let action_result = ActionResult {
        exit_code: 0,
        output_files: vec![
            OutputFile {
                path: "outputs/file1".to_string(),
                digest: Some(digest1),
                ..Default::default()
            },
            OutputFile {
                path: "outputs/file2".to_string(),
                digest: Some(digest2),
                ..Default::default()
            },
            OutputFile {
                path: "outputs/file3".to_string(),
                digest: Some(digest3),
                ..Default::default()
            },
        ],
        ..Default::default()
    };
    let result = from_cache(action_result);
    let spawn = f.new_spawn_from_result(&result);
    let context = f.new_spawn_execution_context(&spawn);
    f.outputs_to_download = OutputsToDownload::All;
    let service = f.new_remote_execution_service();
    let action = service.build_remote_action(&*spawn, &context).unwrap();
    f.create_output_directories(&spawn);

    let download_exception =
        expect_bulk_transfer_exception(service.download_outputs(&action, &result));

    assert_eq!(download_exception.get_suppressed().len(), 1);
    assert_eq!(f.in_memory_cache_client.get_num_successful_downloads(), 2);
    assert_eq!(f.in_memory_cache_client.get_num_failed_downloads(), 1);
    let e = &download_exception.get_suppressed()[0];
    assert_eq!(root_cause_message(&**e), "download failed");
    assert!(!context.is_lock_output_files_called());
}

// Java: com/google/devtools/build/lib/remote/RemoteExecutionServiceTest.java:1610 RemoteExecutionServiceTest.downloadOutputs_withMultipleErrors_addsThemAsSuppressed
#[test]
fn download_outputs_with_multiple_errors_adds_them_as_suppressed() {
    let mut f = Fixture::set_up();
    let digest1 = f.add_contents_str("file1");
    let digest2 = f.add_exception_str("file2", errors::io_exception("file2 failed"));
    let digest3 = f.add_exception_str("file3", errors::io_exception("file3 failed"));
    let action_result = ActionResult {
        exit_code: 0,
        output_files: vec![
            OutputFile {
                path: "outputs/file1".to_string(),
                digest: Some(digest1),
                ..Default::default()
            },
            OutputFile {
                path: "outputs/file2".to_string(),
                digest: Some(digest2),
                ..Default::default()
            },
            OutputFile {
                path: "outputs/file3".to_string(),
                digest: Some(digest3),
                ..Default::default()
            },
        ],
        ..Default::default()
    };
    let result = from_cache(action_result);
    let spawn = f.new_spawn_from_result(&result);
    let context = f.new_spawn_execution_context(&spawn);
    f.outputs_to_download = OutputsToDownload::All;
    let service = f.new_remote_execution_service();
    let action = service.build_remote_action(&*spawn, &context).unwrap();
    f.create_output_directories(&spawn);

    let e = expect_bulk_transfer_exception(service.download_outputs(&action, &result));

    assert_eq!(e.get_suppressed().len(), 2);
    let message0 = e.get_suppressed()[0].to_string();
    assert!(
        message0 == "file2 failed" || message0 == "file3 failed",
        "{message0}"
    );
    let message1 = e.get_suppressed()[1].to_string();
    assert!(
        message1 == "file2 failed" || message1 == "file3 failed",
        "{message1}"
    );
}

// Java: com/google/devtools/build/lib/remote/RemoteExecutionServiceTest.java:1642 RemoteExecutionServiceTest.downloadOutputs_withDuplicateIOErrors_doesNotSuppress
#[test]
fn download_outputs_with_duplicate_io_errors_does_not_suppress() {
    let mut f = Fixture::set_up();
    let digest1 = f.add_contents_str("file1");
    // PORT: Java registers one exception object for both digests; an `io::Error` cannot be
    // cloned, so each digest gets an equal one.
    let digest2 = f.add_exception_str("file2", errors::io_exception("reused io exception"));
    let digest3 = f.add_exception_str("file3", errors::io_exception("reused io exception"));
    let action_result = ActionResult {
        exit_code: 0,
        output_files: vec![
            OutputFile {
                path: "outputs/file1".to_string(),
                digest: Some(digest1),
                ..Default::default()
            },
            OutputFile {
                path: "outputs/file2".to_string(),
                digest: Some(digest2),
                ..Default::default()
            },
            OutputFile {
                path: "outputs/file3".to_string(),
                digest: Some(digest3),
                ..Default::default()
            },
        ],
        ..Default::default()
    };
    let result = from_cache(action_result);
    let spawn = f.new_spawn_from_result(&result);
    let context = f.new_spawn_execution_context(&spawn);
    f.outputs_to_download = OutputsToDownload::All;
    let service = f.new_remote_execution_service();
    let action = service.build_remote_action(&*spawn, &context).unwrap();
    f.create_output_directories(&spawn);

    let download_exception =
        expect_bulk_transfer_exception(service.download_outputs(&action, &result));

    for t in download_exception.get_suppressed() {
        assert_eq!(root_cause_message(&**t), "reused io exception");
    }
}

// Java: com/google/devtools/build/lib/remote/RemoteExecutionServiceTest.java:1675 RemoteExecutionServiceTest.downloadOutputs_withDuplicateInterruptions_doesNotSuppress
#[test]
fn download_outputs_with_duplicate_interruptions_does_not_suppress() {
    let mut f = Fixture::set_up();
    let digest1 = f.add_contents_str("file1");
    // `new InterruptedException("reused interruption")`.
    let digest2 = f.add_exception_str(
        "file2",
        io::Error::new(io::ErrorKind::Interrupted, "reused interruption"),
    );
    let digest3 = f.add_exception_str(
        "file3",
        io::Error::new(io::ErrorKind::Interrupted, "reused interruption"),
    );
    let action_result = ActionResult {
        exit_code: 0,
        output_files: vec![
            OutputFile {
                path: "outputs/file1".to_string(),
                digest: Some(digest1),
                ..Default::default()
            },
            OutputFile {
                path: "outputs/file2".to_string(),
                digest: Some(digest2),
                ..Default::default()
            },
            OutputFile {
                path: "outputs/file3".to_string(),
                digest: Some(digest3),
                ..Default::default()
            },
        ],
        ..Default::default()
    };
    let result = from_cache(action_result);
    let spawn = f.new_spawn_from_result(&result);
    let context = f.new_spawn_execution_context(&spawn);
    f.outputs_to_download = OutputsToDownload::All;
    let service = f.new_remote_execution_service();
    let action = service.build_remote_action(&*spawn, &context).unwrap();
    f.create_output_directories(&spawn);

    // `assertThrows(InterruptedException.class, ...)`.
    let e = match service.download_outputs(&action, &result) {
        Ok(_) => panic!("expected an InterruptedException, the call succeeded"),
        Err(SpawnCacheError::Io(e)) => e,
        Err(SpawnCacheError::Exec(e)) => panic!(
            "expected an InterruptedException, got an ExecException: {}",
            e.get_message()
        ),
    };
    assert!(errors::is_interrupted(&e), "{e:?}");

    // `e.getSuppressed()` is empty.
    assert!(errors::downcast_ref::<BulkTransferException>(&e).is_none());
    assert_eq!(root_cause_message(&e), "reused interruption");
}

// Java: com/google/devtools/build/lib/remote/RemoteExecutionServiceTest.java:1705 RemoteExecutionServiceTest.downloadOutputs_withStdoutStderrOnSuccess_writable
#[test]
fn download_outputs_with_stdout_stderr_on_success_writable() {
    // Tests that fetching stdout/stderr as a digest works and that OutErr is still
    // writable afterwards.
    // PORT: Java spies on `outErr` and on its child to verify that `childOutErr()` is called twice
    // and `clearOut()` / `clearErr()` once on the child; `FileOutErr` is a concrete struct here
    // and cannot be spied on, so those verifications are left out.
    let mut f = Fixture::set_up();
    let digest_stdout = f.add_contents_str("stdout");
    let digest_stderr = f.add_contents_str("stderr");
    let action_result = ActionResult {
        exit_code: 0,
        stdout_digest: Some(digest_stdout),
        stderr_digest: Some(digest_stderr),
        ..Default::default()
    };
    let result = from_cache(action_result);
    let spawn = f.new_spawn_from_result(&result);
    let context = f.new_spawn_execution_context_with_out_err(&spawn, f.out_err.clone());
    f.outputs_to_download = OutputsToDownload::All;
    let service = f.new_remote_execution_service();
    let action = service.build_remote_action(&*spawn, &context).unwrap();
    f.create_output_directories(&spawn);

    service.download_outputs(&action, &result).unwrap();

    assert!(f.out_err.get_output_path().exists());
    assert!(f.out_err.get_error_path().exists());
    if let Err(err) = f
        .out_err
        .write_out(&[0])
        .and_then(|()| f.out_err.write_err(&[0]))
    {
        panic!("outErr should still be writable after download finished: {err}");
    }
    assert!(context.is_lock_output_files_called());
}

// Java: com/google/devtools/build/lib/remote/RemoteExecutionServiceTest.java:1747 RemoteExecutionServiceTest.downloadOutputs_withStdoutStderrOnFailure_writableAndEmpty
#[test]
fn download_outputs_with_stdout_stderr_on_failure_writable_and_empty() {
    // Test that when downloading stdout/stderr fails the OutErr is still writable
    // and empty.
    // PORT: see download_outputs_with_stdout_stderr_on_success_writable for the spies.
    let mut f = Fixture::set_up();
    // Don't add stdout/stderr as a known blob to the remote cache so that downloading it will fail
    let digest_stdout = f.digest_util.compute_as_utf8("stdout");
    let digest_stderr = f.digest_util.compute_as_utf8("stderr");
    let action_result = ActionResult {
        exit_code: 0,
        stdout_digest: Some(digest_stdout),
        stderr_digest: Some(digest_stderr),
        ..Default::default()
    };
    let result = from_cache(action_result);
    let spawn = f.new_spawn_from_result(&result);
    let context = f.new_spawn_execution_context_with_out_err(&spawn, f.out_err.clone());
    f.outputs_to_download = OutputsToDownload::All;
    let service = f.new_remote_execution_service();
    let action = service.build_remote_action(&*spawn, &context).unwrap();
    f.create_output_directories(&spawn);

    let _e = expect_bulk_transfer_exception(service.download_outputs(&action, &result));

    assert!(!f.out_err.get_output_path().exists());
    assert!(!f.out_err.get_error_path().exists());
    if let Err(err) = f
        .out_err
        .write_out(&[0])
        .and_then(|()| f.out_err.write_err(&[0]))
    {
        panic!("outErr should still be writable after download failed: {err}");
    }
    assert!(!context.is_lock_output_files_called());
}

// Java: com/google/devtools/build/lib/remote/RemoteExecutionServiceTest.java:1790 RemoteExecutionServiceTest.downloadOutputs_outputNameClashesWithTempName_success
#[test]
fn download_outputs_output_name_clashes_with_temp_name_success() {
    let mut f = Fixture::set_up();
    let d1 = f.add_contents_str("content1");
    let d2 = f.add_contents_str("content2");
    let r = ActionResult {
        exit_code: 0,
        output_files: vec![
            OutputFile {
                path: "outputs/foo.tmp".to_string(),
                digest: Some(d1),
                ..Default::default()
            },
            OutputFile {
                path: "outputs/foo".to_string(),
                digest: Some(d2),
                ..Default::default()
            },
        ],
        ..Default::default()
    };
    let result = from_cache(r);
    let spawn = f.new_spawn_from_result(&result);
    let context = f.new_spawn_execution_context(&spawn);
    f.outputs_to_download = OutputsToDownload::All;
    let service = f.new_remote_execution_service();
    let action = service.build_remote_action(&*spawn, &context).unwrap();
    f.create_output_directories(&spawn);

    service.download_outputs(&action, &result).unwrap();

    assert_eq!(read_content(&f.exec_path("outputs/foo.tmp")), "content1");
    assert_eq!(read_content(&f.exec_path("outputs/foo")), "content2");
    assert!(context.is_lock_output_files_called());
}

// Java: com/google/devtools/build/lib/remote/RemoteExecutionServiceTest.java:1816 RemoteExecutionServiceTest.downloadOutputs_outputFiles_partialDownload
#[test]
fn download_outputs_output_files_partial_download() {
    // arrange
    let mut f = Fixture::set_up();
    let d1 = f.add_contents_str("content1");
    let d2 = f.add_contents_str("content2");
    let r = ActionResult {
        exit_code: 0,
        output_files: vec![
            OutputFile {
                path: "outputs/file1".to_string(),
                digest: Some(d1.clone()),
                ..Default::default()
            },
            OutputFile {
                path: "outputs/file2".to_string(),
                digest: Some(d2.clone()),
                ..Default::default()
            },
        ],
        ..Default::default()
    };

    let result = from_cache(r);
    let spawn = f.new_spawn_from_result(&result);
    let context = f.new_spawn_execution_context(&spawn);
    // `shouldDownloadOutput(PathFragment.create("outputs/file1"), /* treeRootExecPath= */ null)`
    f.outputs_to_download = OutputsToDownload::Paths(vec!["outputs/file1"]);
    let service = f.new_remote_execution_service();
    let action = service.build_remote_action(&*spawn, &context).unwrap();
    f.create_output_directories(&spawn);

    // act
    let in_memory_output = service.download_outputs(&action, &result).unwrap();

    // assert
    assert!(in_memory_output.is_none());
    assert_eq!(
        f.action_fs_get_digest(&f.exec_fragment("outputs/file1")),
        Some(DigestUtil::to_binary_digest(&d1))
    );
    assert_eq!(
        f.action_fs_get_digest(&f.exec_fragment("outputs/file2")),
        Some(DigestUtil::to_binary_digest(&d2))
    );
    assert!(f.exec_path("outputs/file1").exists());
    assert!(!f.exec_path("outputs/file2").exists());
    assert!(context.is_lock_output_files_called());
}

// Java: com/google/devtools/build/lib/remote/RemoteExecutionServiceTest.java:1853 RemoteExecutionServiceTest.downloadOutputs_outputFiles_noDownload
#[test]
fn download_outputs_output_files_no_download() {
    // arrange
    let f = Fixture::set_up();
    let d1 = f.add_contents_str("content1");
    let d2 = f.add_contents_str("content2");
    let r = ActionResult {
        exit_code: 0,
        output_files: vec![
            OutputFile {
                path: "outputs/file1".to_string(),
                digest: Some(d1.clone()),
                ..Default::default()
            },
            OutputFile {
                path: "outputs/file2".to_string(),
                digest: Some(d2.clone()),
                ..Default::default()
            },
        ],
        ..Default::default()
    };

    let result = from_cache(r);
    let spawn = f.new_spawn_from_result(&result);
    let context = f.new_spawn_execution_context(&spawn);
    let service = f.new_remote_execution_service();
    let action = service.build_remote_action(&*spawn, &context).unwrap();
    f.create_output_directories(&spawn);

    // act
    let in_memory_output = service.download_outputs(&action, &result).unwrap();

    // assert
    assert!(in_memory_output.is_none());
    assert_eq!(
        f.action_fs_get_digest(&f.exec_fragment("outputs/file1")),
        Some(DigestUtil::to_binary_digest(&d1))
    );
    assert_eq!(
        f.action_fs_get_digest(&f.exec_fragment("outputs/file2")),
        Some(DigestUtil::to_binary_digest(&d2))
    );
    assert!(!f.exec_path("outputs/file1").exists());
    assert!(!f.exec_path("outputs/file2").exists());
    assert!(context.is_lock_output_files_called());
}

/// The output directory of the `outputDirectories_*Download` tests:
///
/// ```text
/// dir/file1
/// dir/a/file2
/// ```
///
/// Returns `(d1, d2, a, t)`.
fn output_directory_with_nested_file(f: &Fixture) -> (Digest, Digest, Tree) {
    let d1 = f.add_contents_str("content1");
    let d2 = f.add_contents_str("content2");
    let file1 = FileNode {
        name: "file1".to_string(),
        digest: Some(d1.clone()),
        is_executable: true,
        ..Default::default()
    };
    let file2 = FileNode {
        name: "file2".to_string(),
        digest: Some(d2.clone()),
        is_executable: true,
        ..Default::default()
    };
    let a = Directory {
        files: vec![file2],
        ..Default::default()
    };
    let da = f.add_contents_message(&a);
    let root = Directory {
        files: vec![file1],
        directories: vec![DirectoryNode {
            name: "a".to_string(),
            digest: Some(da),
        }],
        ..Default::default()
    };
    let t = Tree {
        root: Some(root),
        children: vec![a],
    };
    (d1, d2, t)
}

// Java: com/google/devtools/build/lib/remote/RemoteExecutionServiceTest.java:1887 RemoteExecutionServiceTest.downloadOutputs_outputDirectories_partialDownload
#[test]
fn download_outputs_output_directories_partial_download() {
    // arrange

    // Output Directory:
    // dir/file1
    // dir/a/file2
    let mut f = Fixture::set_up();
    let (d1, d2, t) = output_directory_with_nested_file(&f);
    let dt = f.add_contents_message(&t);
    let r = ActionResult {
        exit_code: 0,
        output_directories: vec![OutputDirectory {
            path: "outputs/dir".to_string(),
            tree_digest: Some(dt),
            ..Default::default()
        }],
        ..Default::default()
    };
    let result = from_cache(r);
    let spawn = f.new_spawn_from_result(&result);
    let context = f.new_spawn_execution_context(&spawn);
    // `shouldDownloadOutput(PathFragment.create("outputs/dir/file1"),
    // PathFragment.create("outputs/dir"))`
    f.outputs_to_download = OutputsToDownload::Paths(vec!["outputs/dir/file1"]);
    let service = f.new_remote_execution_service();
    let action = service.build_remote_action(&*spawn, &context).unwrap();
    f.create_output_directories(&spawn);

    // act
    let in_memory_output = service.download_outputs(&action, &result).unwrap();

    // assert
    assert!(in_memory_output.is_none());
    assert_eq!(
        f.action_fs_get_digest(&f.exec_fragment("outputs/dir/file1")),
        Some(DigestUtil::to_binary_digest(&d1))
    );
    assert_eq!(
        f.action_fs_get_digest(&f.exec_fragment("outputs/dir/a/file2")),
        Some(DigestUtil::to_binary_digest(&d2))
    );
    assert!(f.exec_path("outputs/dir/file1").exists());
    assert!(!f.exec_path("outputs/dir/a").exists());
    assert!(context.is_lock_output_files_called());
}

// Java: com/google/devtools/build/lib/remote/RemoteExecutionServiceTest.java:1940 RemoteExecutionServiceTest.downloadOutputs_outputDirectories_noDownload
#[test]
fn download_outputs_output_directories_no_download() {
    // arrange

    // Output Directory:
    // dir/file1
    // dir/a/file2
    let f = Fixture::set_up();
    let (d1, d2, t) = output_directory_with_nested_file(&f);
    let dt = f.add_contents_message(&t);
    let r = ActionResult {
        exit_code: 0,
        output_directories: vec![OutputDirectory {
            path: "outputs/dir".to_string(),
            tree_digest: Some(dt),
            ..Default::default()
        }],
        ..Default::default()
    };

    let result = from_cache(r);
    let spawn = f.new_spawn_from_result(&result);
    let context = f.new_spawn_execution_context(&spawn);
    let service = f.new_remote_execution_service();
    let action = service.build_remote_action(&*spawn, &context).unwrap();
    f.create_output_directories(&spawn);

    // act
    let in_memory_output = service.download_outputs(&action, &result).unwrap();

    // assert
    assert!(in_memory_output.is_none());
    assert_eq!(
        f.action_fs_get_digest(&f.exec_fragment("outputs/dir/file1")),
        Some(DigestUtil::to_binary_digest(&d1))
    );
    assert_eq!(
        f.action_fs_get_digest(&f.exec_fragment("outputs/dir/a/file2")),
        Some(DigestUtil::to_binary_digest(&d2))
    );
    assert!(!f.exec_path("outputs/dir/file1").exists());
    assert!(!f.exec_path("outputs/dir/a").exists());
    assert!(context.is_lock_output_files_called());
}

// Java: com/google/devtools/build/lib/remote/RemoteExecutionServiceTest.java:1991 RemoteExecutionServiceTest.downloadOutputs_outputDirectories_doNotDownload_failProperly
#[test]
fn download_outputs_output_directories_do_not_download_fail_properly() {
    // Test that we properly fail when downloading the metadata of an output
    // directory fails

    // arrange

    // Output Directory:
    // dir/file1
    // dir/a/file2
    let f = Fixture::set_up();
    let (_d1, _d2, t) = output_directory_with_nested_file(&f);
    // Downloading the tree will fail
    let dt = f.add_exception_message(&t, errors::io_exception("entry not found"));
    let r = ActionResult {
        exit_code: 0,
        output_directories: vec![OutputDirectory {
            path: "outputs/dir".to_string(),
            tree_digest: Some(dt),
            ..Default::default()
        }],
        ..Default::default()
    };

    let result = from_cache(r);
    let spawn = f.new_spawn_from_result(&result);
    let context = f.new_spawn_execution_context(&spawn);
    let service = f.new_remote_execution_service();
    let action = service.build_remote_action(&*spawn, &context).unwrap();
    f.create_output_directories(&spawn);

    // act
    let e = expect_bulk_transfer_exception(service.download_outputs(&action, &result));

    // assert
    assert_eq!(e.get_suppressed().len(), 1);
    // PORT: Java compares the suppressed exception with the registered one by identity; the
    // port shares the registered error (`errors::shared`), so its message is compared.
    assert_eq!(e.get_suppressed()[0].to_string(), "entry not found");
    assert!(!context.is_lock_output_files_called());
}

// Java: com/google/devtools/build/lib/remote/RemoteExecutionServiceTest.java:2042 RemoteExecutionServiceTest.downloadOutputs_nonInlinedStdoutAndStderr_alwaysDownload
#[test]
fn download_outputs_non_inlined_stdout_and_stderr_always_download() {
    // arrange
    let f = Fixture::set_up();
    let d_out = f.add_contents_str("stdout");
    let d_err = f.add_contents_str("stderr");
    let r = ActionResult {
        exit_code: 0,
        stdout_digest: Some(d_out.clone()),
        stderr_digest: Some(d_err.clone()),
        ..Default::default()
    };

    let result = from_cache(r);
    let spawn = f.new_spawn_from_result(&result);
    let context = f.new_spawn_execution_context(&spawn);
    let service = f.new_remote_execution_service();
    let action = service.build_remote_action(&*spawn, &context).unwrap();
    f.create_output_directories(&spawn);

    // act
    let in_memory_output = service.download_outputs(&action, &result).unwrap();

    // assert
    assert!(in_memory_output.is_none());
    assert_eq!(
        f.action_fs_get_digest(f.out_err.get_output_path().as_fragment()),
        Some(DigestUtil::to_binary_digest(&d_out))
    );
    assert_eq!(
        f.action_fs_get_digest(f.out_err.get_error_path().as_fragment()),
        Some(DigestUtil::to_binary_digest(&d_err))
    );
    assert_eq!(f.out_err.out_as_latin1(), "stdout");
    assert_eq!(f.out_err.err_as_latin1(), "stderr");
    let output_base = f.artifact_root.get_root().as_path().expect("checkNotNull");
    assert!(output_base.readdir(Symlinks::Nofollow).unwrap().is_empty());
    assert!(context.is_lock_output_files_called());
}

// Java: com/google/devtools/build/lib/remote/RemoteExecutionServiceTest.java:2076 RemoteExecutionServiceTest.downloadOutputs_inlinedStdoutAndStderr_alwaysDownload
#[test]
fn download_outputs_inlined_stdout_and_stderr_always_download() {
    // arrange
    let f = Fixture::set_up();
    let d_out = f.digest_util.compute(b"stdout");
    let d_err = f.digest_util.compute(b"stderr");
    let r = ActionResult {
        exit_code: 0,
        stdout_raw: b"stdout".to_vec(),
        stderr_raw: b"stderr".to_vec(),
        ..Default::default()
    };

    let result = from_cache(r);
    let spawn = f.new_spawn_from_result(&result);
    let context = f.new_spawn_execution_context(&spawn);
    let service = f.new_remote_execution_service();
    let action = service.build_remote_action(&*spawn, &context).unwrap();
    f.create_output_directories(&spawn);

    // act
    let in_memory_output = service.download_outputs(&action, &result).unwrap();

    // assert
    assert!(in_memory_output.is_none());
    assert_eq!(
        f.action_fs_get_digest(f.out_err.get_output_path().as_fragment()),
        Some(DigestUtil::to_binary_digest(&d_out))
    );
    assert_eq!(
        f.action_fs_get_digest(f.out_err.get_error_path().as_fragment()),
        Some(DigestUtil::to_binary_digest(&d_err))
    );
    assert!(in_memory_output.is_none());
    assert_eq!(f.out_err.out_as_latin1(), "stdout");
    assert_eq!(f.out_err.err_as_latin1(), "stderr");
    let output_base = f.artifact_root.get_root().as_path().expect("checkNotNull");
    assert!(output_base.readdir(Symlinks::Nofollow).unwrap().is_empty());
    assert!(context.is_lock_output_files_called());
}

// Java: com/google/devtools/build/lib/remote/RemoteExecutionServiceTest.java:2111 RemoteExecutionServiceTest.downloadOutputs_inMemoryOutput_doNotDownload
#[test]
fn download_outputs_in_memory_output_do_not_download() {
    // Test that downloading an in memory output works

    // arrange
    let f = Fixture::set_up();
    let d1 = f.add_contents_str("content1");
    let d2 = f.add_contents_str("content2");
    let r = ActionResult {
        exit_code: 0,
        output_files: vec![
            OutputFile {
                path: "outputs/file1".to_string(),
                digest: Some(d1.clone()),
                ..Default::default()
            },
            OutputFile {
                path: "outputs/file2".to_string(),
                digest: Some(d2.clone()),
                ..Default::default()
            },
        ],
        ..Default::default()
    };

    let result = from_cache(r);
    // a1 should be provided as an InMemoryOutput
    let in_memory_output_path_fragment = PathFragment::create("outputs/file1");
    let spawn =
        f.new_spawn_from_result_with_in_memory_output(&result, &in_memory_output_path_fragment);
    let context = f.new_spawn_execution_context(&spawn);
    let service = f.new_remote_execution_service();
    let action = service.build_remote_action(&*spawn, &context).unwrap();
    f.create_output_directories(&spawn);

    // act
    let in_memory_output = service.download_outputs(&action, &result).unwrap();

    // assert
    let in_memory_output = in_memory_output.expect("inMemoryOutput is not null");
    assert_eq!(&in_memory_output.get_contents()[..], b"content1");
    assert_eq!(
        in_memory_output.get_output(),
        &input(&create_artifact_str(&f.artifact_root, "file1"))
    );
    assert_eq!(
        f.action_fs_get_digest(&f.exec_fragment("outputs/file1")),
        Some(DigestUtil::to_binary_digest(&d1))
    );
    assert_eq!(
        f.action_fs_get_digest(&f.exec_fragment("outputs/file2")),
        Some(DigestUtil::to_binary_digest(&d2))
    );
    assert!(!f.exec_path("outputs/file1").exists());
    assert!(!f.exec_path("outputs/file2").exists());
    assert!(context.is_lock_output_files_called());
}

// Java: com/google/devtools/build/lib/remote/RemoteExecutionServiceTest.java:2153 RemoteExecutionServiceTest.downloadOutputs_missingInMemoryOutput_returnsNull
#[test]
fn download_outputs_missing_in_memory_output_returns_null() {
    // Test that downloadOutputs returns null if a declared in-memory output is missing from action
    // result.

    // arrange
    let f = Fixture::set_up();
    let r = ActionResult {
        exit_code: 0,
        ..Default::default()
    };
    let result = from_cache(r);
    let a1 = create_artifact_str(&f.artifact_root, "file1");
    // set file1 as declared output but not mandatory output
    let mut execution_info = IndexMap::new();
    execution_info.insert(
        execution_requirements::REMOTE_EXECUTION_INLINE_OUTPUTS.to_string(),
        "outputs/file1".to_string(),
    );
    let spawn: Arc<dyn Spawn> = Arc::new(SimpleSpawn::new(
        Arc::new(FakeOwner::new(
            "foo",
            Some("bar"),
            Some(Label::parse_canonical("//dummy:label").unwrap()),
        )),
        /* arguments= */ Vec::new(),
        /* environment= */ IndexMap::new(),
        /* executionInfo= */ execution_info,
        /* inputs= */ NestedSetBuilder::empty_set(Order::STABLE_ORDER),
        /* tools= */ NestedSetBuilder::empty_set(Order::STABLE_ORDER),
        /* outputs= */ vec![input(&a1)],
        /* mandatoryOutputs= */ Some(HashSet::new()),
        Arc::new(ResourceSet::zero()),
        PathMapper::NOOP,
    ));

    let context = f.new_spawn_execution_context(&spawn);
    let service = f.new_remote_execution_service();
    let action = service.build_remote_action(&*spawn, &context).unwrap();
    f.create_output_directories(&spawn);

    // act
    let in_memory_output = service.download_outputs(&action, &result).unwrap();

    // assert
    assert!(in_memory_output.is_none());
    // The in memory file metadata also should not have been injected.
    assert!(!f.action_fs_exists(&f.exec_root.as_fragment().get_relative(a1.get_exec_path())));
}

// Java: com/google/devtools/build/lib/remote/RemoteExecutionServiceTest.java:2190 RemoteExecutionServiceTest.downloadOutputs_missingMandatoryOutputs_reportError
#[test]
fn download_outputs_missing_mandatory_outputs_report_error() {
    // Test that an AC which misses mandatory outputs is correctly ignored.
    let f = Fixture::set_up();
    let foo_digest = f.add_contents_str("foo-contents");
    let builder = ActionResult {
        output_files: vec![OutputFile {
            path: "outputs/foo".to_string(),
            digest: Some(foo_digest),
            ..Default::default()
        }],
        ..Default::default()
    };
    let result = from_cache(builder);
    let mut outputs: IndexSet<ActionInputRef> = IndexSet::new();
    let expected_output_files = ["outputs/foo", "outputs/bar"];
    for output_file in expected_output_files {
        let path = f
            .remote_path_resolver
            .output_path_to_local_path(output_file);
        let output = create_artifact(&f.artifact_root, &path);
        outputs.insert(input(&output));
    }
    let spawn = new_spawn(IndexMap::new(), outputs);
    let context = f.new_spawn_execution_context(&spawn);
    let service = f.new_remote_execution_service();
    let action = service.build_remote_action(&*spawn, &context).unwrap();
    f.create_output_directories(&spawn);

    let error = expect_io_error(service.download_outputs(&action, &result));

    let message = error.to_string();
    assert!(
        regex::Regex::new("mandatory output .+ was not created")
            .unwrap()
            .is_match(&message),
        "{message}"
    );
}

// Java: com/google/devtools/build/lib/remote/RemoteExecutionServiceTest.java:2217 RemoteExecutionServiceTest.downloadOutputs_pathUnmapped
#[test]
#[ignore = "needs a path mapper: blaze_exec's PathMapper has only PathMapper.NOOP and RemotePathResolver.createMapped is unported"]
fn download_outputs_path_unmapped() {
    // Test that the output of a remote action with path mapping applied is downloaded into the
    // correct unmapped local path.
    let mut f = Fixture::set_up();
    let d1 = f.add_contents_str("content1");
    let d2 = f.add_contents_str("content2");
    let output1 = create_artifact_str(&f.artifact_root, "bin/config/dir/output1");
    let output2 = create_artifact_str(&f.artifact_root, "bin/other_dir/output2");
    let r = ActionResult {
        exit_code: 0,
        // The action result includes the mapped paths.
        output_files: vec![
            OutputFile {
                path: "outputs/bin/dir/output1".to_string(),
                digest: Some(d1.clone()),
                ..Default::default()
            },
            OutputFile {
                path: "outputs/bin/other_dir/output2".to_string(),
                digest: Some(d2.clone()),
                ..Default::default()
            },
        ],
        ..Default::default()
    };
    // PORT: Java's spawn has `.setPathMapper(execPath ->
    // PathFragment.create(execPath.getPathString().replaceAll("config/", "")))`; blaze_exec's
    // `PathMapper` cannot express it.
    let spawn = SpawnBuilder::new(&["unused"])
        .with_output(input(&output1))
        .with_output(input(&output2))
        .build();
    let result = from_cache(r);
    let context = f.new_spawn_execution_context(&spawn);
    f.outputs_to_download = OutputsToDownload::Paths(vec![
        "outputs/bin/config/dir/output1",
        "outputs/bin/other_dir/output2",
    ]);
    let service = f.new_remote_execution_service();
    let action = service.build_remote_action(&*spawn, &context).unwrap();

    let in_memory_output = service.download_outputs(&action, &result).unwrap();

    assert!(in_memory_output.is_none());
    assert_eq!(
        f.action_fs_get_digest(output1.get_path().as_fragment()),
        Some(DigestUtil::to_binary_digest(&d1))
    );
    assert_eq!(read_content(&output1.get_path()), "content1");
    assert_eq!(
        f.action_fs_get_digest(output2.get_path().as_fragment()),
        Some(DigestUtil::to_binary_digest(&d2))
    );
    assert_eq!(read_content(&output2.get_path()), "content2");
    assert!(context.is_lock_output_files_called());
}

// ---------------------------------------------------------------------------------------------
// uploadOutputs
// ---------------------------------------------------------------------------------------------

// Java: com/google/devtools/build/lib/remote/RemoteExecutionServiceTest.java:2264 RemoteExecutionServiceTest.uploadOutputs_uploadDirectory_works
#[test]
fn upload_outputs_upload_directory_works() {
    // Test that uploading a directory works.

    // arrange
    let f = Fixture::set_up();
    let foo_digest = f
        .fake_file_cache
        .create_scratch_input(&from_path("outputs/a/foo"), "xyz")
        .unwrap();
    let qux_digest = f
        .fake_file_cache
        .create_scratch_input(&from_path("outputs/bar/qux"), "abc")
        .unwrap();
    let bar_digest = f
        .fake_file_cache
        .create_scratch_input_directory(
            &from_path("outputs/bar"),
            &Tree {
                root: Some(Directory {
                    files: vec![FileNode {
                        name: "qux".to_string(),
                        digest: Some(qux_digest.clone()),
                        is_executable: true,
                        ..Default::default()
                    }],
                    ..Default::default()
                }),
                children: Vec::new(),
            },
        )
        .unwrap();
    let foo_file = f.exec_path("outputs/a/foo");
    let qux_file = f.exec_path("outputs/bar/qux");
    qux_file.set_executable(true).unwrap();
    let bar_dir = f.exec_path("outputs/bar");
    let output_file = create_artifact(&f.artifact_root, &foo_file);
    let output_directory = create_tree_artifact_with_generating_action(
        &f.artifact_root,
        bar_dir.relative_to(&f.exec_root),
    );
    let service = f.new_remote_execution_service();
    let spawn = new_spawn(
        IndexMap::new(),
        [input(&output_file), input(&output_directory)]
            .into_iter()
            .collect(),
    );
    let context = f.new_spawn_execution_context(&spawn);
    let action = service.build_remote_action(&*spawn, &context).unwrap();
    let spawn_result = successful_spawn_result();

    // act
    let manifest = service
        .build_upload_manifest(&action, &spawn_result)
        .unwrap();
    upload_outputs_and_wait(&service, &action, &spawn_result);

    // assert
    let expected_result = ActionResult {
        output_files: vec![OutputFile {
            path: "outputs/a/foo".to_string(),
            digest: Some(foo_digest.clone()),
            is_executable: true,
            ..Default::default()
        }],
        output_directories: vec![OutputDirectory {
            path: "outputs/bar".to_string(),
            tree_digest: Some(bar_digest.clone()),
            is_topologically_sorted: true,
            ..Default::default()
        }],
        ..Default::default()
    };
    assert_eq!(manifest.get_action_result(), expected_result);

    let to_query = [foo_digest, qux_digest, bar_digest];
    assert!(f.find_missing_digests(&to_query).is_empty());
}

// Java: com/google/devtools/build/lib/remote/RemoteExecutionServiceTest.java:2329 RemoteExecutionServiceTest.uploadOutputs_uploadEmptyDirectory_works
#[test]
fn upload_outputs_upload_empty_directory_works() {
    // Test that uploading an empty directory works.

    // arrange
    let f = Fixture::set_up();
    let bar_digest = f
        .fake_file_cache
        .create_scratch_input_directory(
            &from_path("outputs/bar"),
            &Tree {
                root: Some(Directory::default()),
                children: Vec::new(),
            },
        )
        .unwrap();
    let bar_dir = f.exec_path("outputs/bar");
    let output_directory = create_tree_artifact_with_generating_action(
        &f.artifact_root,
        bar_dir.relative_to(&f.exec_root),
    );
    let service = f.new_remote_execution_service();
    let spawn = new_spawn(
        IndexMap::new(),
        [input(&output_directory)].into_iter().collect(),
    );
    let context = f.new_spawn_execution_context(&spawn);
    let action = service.build_remote_action(&*spawn, &context).unwrap();
    let spawn_result = successful_spawn_result();

    // act
    let manifest = service
        .build_upload_manifest(&action, &spawn_result)
        .unwrap();
    upload_outputs_and_wait(&service, &action, &spawn_result);

    // assert
    let expected_result = ActionResult {
        output_directories: vec![OutputDirectory {
            path: "outputs/bar".to_string(),
            tree_digest: Some(bar_digest.clone()),
            is_topologically_sorted: true,
            ..Default::default()
        }],
        ..Default::default()
    };
    assert_eq!(manifest.get_action_result(), expected_result);
    assert!(f.find_missing_digests(&[bar_digest]).is_empty());
}

// Java: com/google/devtools/build/lib/remote/RemoteExecutionServiceTest.java:2372 RemoteExecutionServiceTest.uploadOutputs_uploadNestedDirectory_works
#[test]
fn upload_outputs_upload_nested_directory_works() {
    // Test that uploading a nested directory works.

    // arrange
    let f = Fixture::set_up();
    let wobble_digest = f
        .fake_file_cache
        .create_scratch_input(&from_path("outputs/bar/test/wobble"), "xyz")
        .unwrap();
    let qux_digest = f
        .fake_file_cache
        .create_scratch_input(&from_path("outputs/bar/qux"), "abc")
        .unwrap();
    let test_dir_message = Directory {
        files: vec![FileNode {
            name: "wobble".to_string(),
            digest: Some(wobble_digest.clone()),
            is_executable: true,
            ..Default::default()
        }],
        ..Default::default()
    };
    let test_digest = f.digest_util.compute_message(&test_dir_message);
    let bar_tree = Tree {
        root: Some(Directory {
            files: vec![FileNode {
                name: "qux".to_string(),
                digest: Some(qux_digest.clone()),
                is_executable: true,
                ..Default::default()
            }],
            directories: vec![DirectoryNode {
                name: "test".to_string(),
                digest: Some(test_digest),
            }],
            ..Default::default()
        }),
        children: vec![test_dir_message],
    };
    let bar_digest = f
        .fake_file_cache
        .create_scratch_input_directory(&from_path("outputs/bar"), &bar_tree)
        .unwrap();

    let qux_file = f.exec_path("outputs/bar/qux");
    qux_file.set_executable(true).unwrap();
    let bar_dir = f.exec_path("outputs/bar");

    let output_directory = create_tree_artifact_with_generating_action(
        &f.artifact_root,
        bar_dir.relative_to(&f.exec_root),
    );
    let service = f.new_remote_execution_service();
    let spawn = new_spawn(
        IndexMap::new(),
        [input(&output_directory)].into_iter().collect(),
    );
    let context = f.new_spawn_execution_context(&spawn);
    let action = service.build_remote_action(&*spawn, &context).unwrap();
    let spawn_result = successful_spawn_result();

    // act
    let manifest = service
        .build_upload_manifest(&action, &spawn_result)
        .unwrap();
    upload_outputs_and_wait(&service, &action, &spawn_result);

    // assert
    let expected_result = ActionResult {
        output_directories: vec![OutputDirectory {
            path: "outputs/bar".to_string(),
            tree_digest: Some(bar_digest.clone()),
            is_topologically_sorted: true,
            ..Default::default()
        }],
        ..Default::default()
    };
    assert_eq!(manifest.get_action_result(), expected_result);

    let to_query = [wobble_digest, qux_digest, bar_digest];
    assert!(f.find_missing_digests(&to_query).is_empty());
}

// Java: com/google/devtools/build/lib/remote/RemoteExecutionServiceTest.java:2444 RemoteExecutionServiceTest.doUploadDanglingSymlink
fn do_upload_dangling_symlink(target_path: PathFragment) {
    // arrange
    let f = Fixture::set_up();
    let link_path = f.exec_path("outputs/link");
    link_path
        .create_symbolic_link_to_fragment(&target_path)
        .unwrap();
    let output_symlink = create_unresolved_symlink_artifact_with_exec_path(
        &f.artifact_root,
        link_path.relative_to(&f.exec_root),
    );
    let service = f.new_remote_execution_service();
    let spawn = new_spawn(
        IndexMap::new(),
        [input(&output_symlink)].into_iter().collect(),
    );
    let context = f.new_spawn_execution_context(&spawn);
    let action = service.build_remote_action(&*spawn, &context).unwrap();
    let spawn_result = successful_spawn_result();

    // act
    let manifest = service
        .build_upload_manifest(&action, &spawn_result)
        .unwrap();
    upload_outputs_and_wait(&service, &action, &spawn_result);

    // assert
    let expected_result = ActionResult {
        output_file_symlinks: vec![OutputSymlink {
            path: "outputs/link".to_string(),
            target: target_path.get_path_string().to_string(),
            ..Default::default()
        }],
        output_symlinks: vec![OutputSymlink {
            path: "outputs/link".to_string(),
            target: target_path.get_path_string().to_string(),
            ..Default::default()
        }],
        ..Default::default()
    };
    assert_eq!(manifest.get_action_result(), expected_result);
}

// Java: com/google/devtools/build/lib/remote/RemoteExecutionServiceTest.java:2480 RemoteExecutionServiceTest.uploadOutputs_uploadRelativeDanglingSymlink
#[test]
fn upload_outputs_upload_relative_dangling_symlink() {
    do_upload_dangling_symlink(PathFragment::create("some/path"));
}

// Java: com/google/devtools/build/lib/remote/RemoteExecutionServiceTest.java:2485 RemoteExecutionServiceTest.uploadOutputs_uploadAbsoluteDanglingSymlink
#[test]
fn upload_outputs_upload_absolute_dangling_symlink() {
    do_upload_dangling_symlink(PathFragment::create("/some/path"));
}

// Java: com/google/devtools/build/lib/remote/RemoteExecutionServiceTest.java:2490 RemoteExecutionServiceTest.uploadOutputs_emptyOutputs_doNotPerformUpload
#[test]
fn upload_outputs_empty_outputs_do_not_perform_upload() {
    // Test that uploading an empty output does not try to perform an upload.

    // arrange
    let f = Fixture::set_up();
    let empty_digest = f
        .fake_file_cache
        .create_scratch_input(&from_path("outputs/bar/test/wobble"), "")
        .unwrap();
    let file = f.exec_path("outputs/bar/test/wobble");
    let output_file = create_artifact(&f.artifact_root, &file);
    let service = f.new_remote_execution_service();
    let spawn = new_spawn(IndexMap::new(), [input(&output_file)].into_iter().collect());
    let context = f.new_spawn_execution_context(&spawn);
    let action = service.build_remote_action(&*spawn, &context).unwrap();
    let spawn_result = successful_spawn_result();

    // act
    upload_outputs_and_wait(&service, &action, &spawn_result);

    // assert
    let missing = f.find_missing_digests(std::slice::from_ref(&empty_digest));
    assert_eq!(missing.into_iter().collect::<Vec<_>>(), vec![empty_digest]);
}

// Java: com/google/devtools/build/lib/remote/RemoteExecutionServiceTest.java:2521 RemoteExecutionServiceTest.uploadOutputs_backgroundExecutorRejects_runsCompletionCallback
#[test]
fn upload_outputs_background_executor_rejects_runs_completion_callback() {
    let f = Fixture::set_up();
    let service = f.new_remote_execution_service();
    let spawn = new_spawn(IndexMap::new(), IndexSet::new());
    let context = f.new_spawn_execution_context(&spawn);
    let action = service.build_remote_action(&*spawn, &context).unwrap();
    let spawn_result = successful_spawn_result();
    service.shutdown_background_task_executor_for_testing();
    let completion_calls = Arc::new(AtomicI32::new(0));

    // `assertThrows(RejectedExecutionException.class, ...)`: an unchecked exception is a panic.
    let calls = completion_calls.clone();
    let thrown = std::panic::catch_unwind(AssertUnwindSafe(|| {
        service.upload_outputs_with_concurrent_changes_check_level(
            &action,
            &spawn_result,
            Box::new(move || {
                calls.fetch_add(1, Ordering::SeqCst);
            }),
            ConcurrentChangesCheckLevel::OFF,
        )
    }));
    let payload = match thrown {
        Ok(r) => panic!("expected a RejectedExecutionException, got {r:?}"),
        Err(payload) => payload,
    };
    assert!(
        panic_message(&*payload).contains("RejectedExecutionException"),
        "{}",
        panic_message(&*payload)
    );

    assert_eq!(completion_calls.load(Ordering::SeqCst), 1);
}

// Java: com/google/devtools/build/lib/remote/RemoteExecutionServiceTest.java:2548 RemoteExecutionServiceTest.outputUploadTask_backgroundExecutorRejects_awaitsConcurrentCompletion
#[test]
#[ignore = "needs RemoteExecutionService.OutputUploadTask with requestCancellation/awaitCompletion: the port's OutputUploadTask is private and has no cancellation (no RemoteRewoundActionSynchronizer)"]
fn output_upload_task_background_executor_rejects_awaits_concurrent_completion() {
    // Java builds `service.new OutputUploadTask(action, spawnResult, false, callback)`, cancels it
    // from one thread (the callback blocks), shuts the background executor down and checks that
    // `task.start()` on a second thread throws RejectedExecutionException only after the
    // concurrent cancellation's callback has finished.
    let f = Fixture::set_up();
    let service = f.new_remote_execution_service();
    let spawn = new_spawn(IndexMap::new(), IndexSet::new());
    let context = f.new_spawn_execution_context(&spawn);
    let _action = service.build_remote_action(&*spawn, &context).unwrap();
    let _spawn_result = successful_spawn_result();
    blaze_util::unported!(
        "RemoteExecutionService.OutputUploadTask.requestCancellation (no public OutputUploadTask)"
    );
}

// Java: com/google/devtools/build/lib/remote/RemoteExecutionServiceTest.java:2597 RemoteExecutionServiceTest.outputUploadTask_completes_unregistersItself
#[test]
#[ignore = "needs RemoteRewoundActionSynchronizer and RemoteOutputService (not ported: no output service, no rewinding)"]
fn output_upload_task_completes_unregisters_itself() {
    // Java registers the task with a RemoteRewoundActionSynchronizer through a mocked
    // RemoteOutputService and checks that the completed upload unregisters itself
    // (`synchronizer.hasRegisteredOutputUploadTasks(spawnOwner)` becomes false).
    blaze_util::unported!("RemoteRewoundActionSynchronizer.hasRegisteredOutputUploadTasks");
}

// Java: com/google/devtools/build/lib/remote/RemoteExecutionServiceTest.java:2654 RemoteExecutionServiceTest.outputUploadTask_cancelBeforeStart_runsCompletionCallbackOnce
#[test]
#[ignore = "needs RemoteExecutionService.OutputUploadTask with requestCancellation/awaitCompletion: the port's OutputUploadTask is private and has no cancellation (no RemoteRewoundActionSynchronizer)"]
fn output_upload_task_cancel_before_start_runs_completion_callback_once() {
    // Java: task.requestCancellation(); task.awaitCompletion(); (twice), then shuts the
    // background executor down, expects `task.start()` to throw RejectedExecutionException and
    // the completion callback to have run exactly once.
    let f = Fixture::set_up();
    let service = f.new_remote_execution_service();
    let spawn = new_spawn(IndexMap::new(), IndexSet::new());
    let context = f.new_spawn_execution_context(&spawn);
    let _action = service.build_remote_action(&*spawn, &context).unwrap();
    let _spawn_result = successful_spawn_result();
    blaze_util::unported!(
        "RemoteExecutionService.OutputUploadTask.requestCancellation (no public OutputUploadTask)"
    );
}

// Java: com/google/devtools/build/lib/remote/RemoteExecutionServiceTest.java:2683 RemoteExecutionServiceTest.uploadOutputs_uploadFails_printWarning
#[test]
fn upload_outputs_upload_fails_print_warning() {
    let f = Fixture::set_up();
    let service = f.new_remote_execution_service();
    let spawn = new_spawn(IndexMap::new(), IndexSet::new());
    let context = f.new_spawn_execution_context(&spawn);
    let action = service.build_remote_action(&*spawn, &context).unwrap();
    let spawn_result = successful_spawn_result();
    // `doReturn(Futures.immediateFailedFuture(new IOException("cache down"))).when(cache)
    // .uploadActionResult(any(), any(), any())`.
    // PORT: the stub is on the cache client, which `CombinedCache.uploadActionResult` calls for
    // the remote cache (there is no disk cache).
    *f.cache_client.upload_action_result_failure.lock() = Some("cache down".to_string());

    upload_outputs_and_wait(&service, &action, &spawn_result);

    let events = f.event_handler.get_events();
    assert_eq!(events.len(), 1, "{events:?}");
    let evt = &events[0];
    assert_eq!(evt.get_kind(), EventKind::Warning);
    assert!(
        evt.get_message().contains("cache down"),
        "{}",
        evt.get_message()
    );
}

// Java: com/google/devtools/build/lib/remote/RemoteExecutionServiceTest.java:2707 RemoteExecutionServiceTest.uploadOutputs_firesUploadEvents
#[test]
fn upload_outputs_fires_upload_events() {
    let f = Fixture::set_up();
    let digest = f
        .fake_file_cache
        .create_scratch_input(&from_path("outputs/file"), "content")
        .unwrap();
    let file = f.exec_path("outputs/file");
    let output_file = create_artifact(&f.artifact_root, &file);
    let service = f.new_remote_execution_service();
    let spawn = new_spawn(IndexMap::new(), [input(&output_file)].into_iter().collect());
    let context = f.new_spawn_execution_context(&spawn);
    let action = service.build_remote_action(&*spawn, &context).unwrap();
    let spawn_result = successful_spawn_result();

    upload_outputs_and_wait(&service, &action, &spawn_result);

    // `containsAtLeast(ActionUploadStartedEvent.create(spawn.getResourceOwner(), Store.CAS,
    // digest), ...)`.
    // PORT: the events are compared by store and digest (they hold a copy of the owner's data,
    // all from the one spawn owner of the test).
    let posts = f.event_handler.get_posts();
    let mut started: Vec<(Store, Digest)> = Vec::new();
    let mut finished: Vec<(Store, Digest)> = Vec::new();
    for post in &posts {
        let post: &dyn Any = &**post;
        if let Some(e) = post.downcast_ref::<ActionUploadStartedEvent>() {
            started.push((e.store(), e.digest().clone()));
        } else if let Some(e) = post.downcast_ref::<ActionUploadFinishedEvent>() {
            finished.push((e.store(), e.digest().clone()));
        }
    }
    let action_digest = action.get_action_key().digest().clone();
    assert!(started.contains(&(Store::Cas, digest.clone())), "{posts:?}");
    assert!(finished.contains(&(Store::Cas, digest)), "{posts:?}");
    assert!(
        started.contains(&(Store::Ac, action_digest.clone())),
        "{posts:?}"
    );
    assert!(finished.contains(&(Store::Ac, action_digest)), "{posts:?}");
}

// Java: com/google/devtools/build/lib/remote/RemoteExecutionServiceTest.java:2736 RemoteExecutionServiceTest.uploadOutputs_missingMandatoryOutputs_dontUpload
#[test]
fn upload_outputs_missing_mandatory_outputs_dont_upload() {
    let f = Fixture::set_up();
    let file = f.exec_path("outputs/file");
    let output_file = create_artifact(&f.artifact_root, &file);
    let service = f.new_remote_execution_service();
    let spawn = new_spawn(IndexMap::new(), [input(&output_file)].into_iter().collect());
    let context = f.new_spawn_execution_context(&spawn);
    let action = service.build_remote_action(&*spawn, &context).unwrap();
    let spawn_result = successful_spawn_result();

    upload_outputs_and_wait(&service, &action, &spawn_result);

    // assert
    assert!(
        f.in_memory_cache_client
            .get_num_find_missing_digests()
            .is_empty()
    );
}

// ---------------------------------------------------------------------------------------------
// uploadInputsIfNotPresent
// ---------------------------------------------------------------------------------------------

// Java: com/google/devtools/build/lib/remote/RemoteExecutionServiceTest.java:2757 RemoteExecutionServiceTest.uploadInputsIfNotPresent_deduplicateFindMissingBlobCalls
#[test]
fn upload_inputs_if_not_present_deduplicate_find_missing_blob_calls() {
    let task_count = 100;
    let f = Fixture::set_up();
    let input = from_path("inputs/foo");
    let input_digest = f
        .fake_file_cache
        .create_scratch_input(&input, "input-foo")
        .unwrap();
    let service = f.new_remote_execution_service();
    let spawn = new_spawn_with_inputs(
        IndexMap::new(),
        IndexSet::new(),
        NestedSetBuilder::create(Order::STABLE_ORDER, [input]),
    );
    let context = f.new_spawn_execution_context(&spawn);
    let action = service.build_remote_action(&*spawn, &context).unwrap();

    // `Executors.newFixedThreadPool(taskCount)`: one thread per task.
    let error: Mutex<Option<String>> = Mutex::new(None);
    std::thread::scope(|s| {
        let handles: Vec<_> = (0..task_count)
            .map(|_| {
                s.spawn(|| {
                    if let Err(e) =
                        service.upload_inputs_if_not_present(&action, /* force= */ false)
                    {
                        *error.lock() = Some(format!("{e:?}"));
                    }
                })
            })
            .collect();
        for handle in handles {
            if let Err(e) = handle.join() {
                *error.lock() = Some(panic_message(&*e));
            }
        }
    });

    assert_eq!(*error.lock(), None);
    let num_find_missing_digests = f.in_memory_cache_client.get_num_find_missing_digests();
    assert_eq!(num_find_missing_digests.get(&input_digest), Some(&1));
    for num in num_find_missing_digests.values() {
        assert_eq!(*num, 1);
    }
}

// Java: com/google/devtools/build/lib/remote/RemoteExecutionServiceTest.java:2798 RemoteExecutionServiceTest.uploadInputsIfNotPresent_sameInputs_interruptOne_keepOthers
#[test]
#[ignore = "needs thread interruption: runtime::get_from_future does not observe Thread.interrupt (Blaze has no thread interruption)"]
fn upload_inputs_if_not_present_same_inputs_interrupt_one_keep_others() {
    let task_count = 100;
    let f = Fixture::set_up();
    let input = from_path("inputs/foo");
    f.fake_file_cache
        .create_scratch_input(&input, "input-foo")
        .unwrap();
    let service = f.new_remote_execution_service();
    let spawn = new_spawn_with_inputs(
        IndexMap::new(),
        IndexSet::new(),
        NestedSetBuilder::create(Order::STABLE_ORDER, [input]),
    );
    let context = f.new_spawn_execution_context(&spawn);
    let action = service.build_remote_action(&*spawn, &context).unwrap();

    let error: Mutex<Option<String>> = Mutex::new(None);
    std::thread::scope(|s| {
        for i in 0..task_count {
            // `random.nextBoolean()`.
            let should_interrupt = i % 2 == 0;
            let service = &service;
            let action = &action;
            let error = &error;
            s.spawn(move || {
                if should_interrupt {
                    // `Thread.currentThread().interrupt()`.
                    blaze_shell::thread_interrupt::interrupt_current();
                }
                match service.upload_inputs_if_not_present(action, /* force= */ false) {
                    Ok(()) => {}
                    Err(SpawnCacheError::Io(e))
                        if should_interrupt && errors::is_interrupted(&e) => {}
                    Err(SpawnCacheError::Exec(blaze_exec::actions::ExecException::Interrupted))
                        if should_interrupt => {}
                    Err(e) => *error.lock() = Some(format!("{e:?}")),
                }
            });
        }
    });

    assert_eq!(*error.lock(), None);
}

// Java: com/google/devtools/build/lib/remote/RemoteExecutionServiceTest.java:2839 RemoteExecutionServiceTest.uploadInputsIfNotPresent_interrupted_requestCancelled
#[test]
#[ignore = "needs thread interruption of a blocked runtime::get_from_future and a cache client whose uploadBlobImpl returns a settable future (Mockito doAnswer)"]
fn upload_inputs_if_not_present_interrupted_request_cancelled() {
    // Java stubs `cache.remoteCacheClient.uploadBlobImpl` to return a SettableFuture, starts
    // `uploadInputsIfNotPresent` on a thread, interrupts the thread once the upload started and
    // checks that the upload future was cancelled.
    blaze_util::unported!("Thread.interrupt of a thread blocked in Utils.getFromFuture");
}

// ---------------------------------------------------------------------------------------------
// buildRemoteAction (persistent workers, scrubbing, path mapping, platforms, sorting)
// ---------------------------------------------------------------------------------------------

// Java: com/google/devtools/build/lib/remote/RemoteExecutionServiceTest.java:2884 RemoteExecutionServiceTest.buildRemoteActionForRemotePersistentWorkers
#[test]
#[ignore = "subject unported: RemoteExecutionService.computePersistentWorkerSignature (WorkerParser); the enablePathMapping=true cases also need a path mapper"]
fn build_remote_action_for_remote_persistent_workers() {
    // `@TestParameter boolean enablePathMapping, @TestParameter boolean siblingRepositoryLayout`.
    // PORT: only enablePathMapping=false can be built (blaze_exec's PathMapper is NOOP only).
    for sibling_repository_layout in [false, true] {
        let enable_path_mapping = false;
        let mut f = Fixture::set_up();
        let input_artifact = create_artifact_str(&f.artifact_root, "input");
        f.fake_file_cache
            .create_scratch_input(&input(&input_artifact), "value")
            .unwrap();
        let tool_input = create_artifact_str(&f.artifact_root, "worker_input");
        f.fake_file_cache
            .create_scratch_input(&input(&tool_input), "worker value")
            .unwrap();

        let tool_dat = create_artifact_str(&f.artifact_root, "tool.dat");
        f.fake_file_cache
            .create_scratch_input(&input(&tool_dat), "tool.dat")
            .unwrap();
        let runfiles_tree = Arc::new(TestRunfilesTree::new(
            "outputs/worker_input.runfiles",
            std::slice::from_ref(&tool_dat),
        ));
        let runfiles_artifact =
            create_runfiles_artifact(&f.artifact_root, "outputs/worker_input.runfiles");
        f.fake_file_cache
            .add_runfiles_tree(&input(&runfiles_artifact), runfiles_tree);

        let spawn = SpawnBuilder::new(&["@flagfile"])
            .with_execution_info(execution_requirements::SUPPORTS_WORKERS, "1")
            .with_inputs([
                input(&input_artifact),
                input(&tool_input),
                input(&runfiles_artifact),
            ])
            .with_tools([input(&tool_input), input(&runfiles_artifact)])
            .build();
        let context = f.new_spawn_execution_context(&spawn);
        f.remote_options.mark_tool_inputs = true;
        f.remote_options.remote_discard_merkle_trees = false;
        f.remote_path_resolver = if sibling_repository_layout {
            Arc::new(SiblingRepositoryLayoutResolver::new(f.exec_root.clone()))
        } else {
            Arc::new(DefaultRemotePathResolver::new(f.exec_root.clone()))
        };
        let service = f.new_remote_execution_service_with_options(&f.remote_options);

        // Check that worker files are properly marked in the merkle tree.
        let tool_input_property = || blaze_remote::reapi::NodeProperties {
            properties: vec![blaze_remote::reapi::NodeProperty {
                name: "bazel_tool_input".to_string(),
                ..Default::default()
            }],
            ..Default::default()
        };
        let runfiles_sub_directory = Directory {
            files: vec![FileNode {
                name: "tool.dat".to_string(),
                digest: Some(f.digest_util.compute_as_utf8("tool.dat")),
                is_executable: true,
                node_properties: Some(tool_input_property()),
            }],
            ..Default::default()
        };

        let runfiles_directory = Directory {
            directories: vec![DirectoryNode {
                name: WORKSPACE_NAME.to_string(),
                digest: Some(f.digest_util.compute_message(&runfiles_sub_directory)),
            }],
            ..Default::default()
        };

        let input_file = FileNode {
            name: "input".to_string(),
            digest: Some(f.digest_util.compute_as_utf8("value")),
            is_executable: true,
            ..Default::default()
        };
        let tool_file = FileNode {
            name: "worker_input".to_string(),
            digest: Some(f.digest_util.compute_as_utf8("worker value")),
            is_executable: true,
            node_properties: Some(tool_input_property()),
        };
        let outputs_directory = Directory {
            files: vec![input_file, tool_file],
            directories: vec![DirectoryNode {
                name: "worker_input.runfiles".to_string(),
                digest: Some(f.digest_util.compute_message(&runfiles_directory)),
            }],
            ..Default::default()
        };

        let mut root_directory = Directory {
            directories: vec![DirectoryNode {
                name: if enable_path_mapping {
                    "mapped_outputs".to_string()
                } else {
                    "outputs".to_string()
                },
                digest: Some(f.digest_util.compute_message(&outputs_directory)),
            }],
            ..Default::default()
        };

        if sibling_repository_layout {
            root_directory = Directory {
                directories: vec![DirectoryNode {
                    name: "_main".to_string(),
                    digest: Some(f.digest_util.compute_message(&root_directory)),
                }],
                ..Default::default()
            };
        }

        let remote_action1 = service.build_remote_action(&*spawn, &context).unwrap();
        let MerkleTree::Uploadable(merkle_tree) = remote_action1.get_merkle_tree() else {
            panic!("ClassCastException: not a MerkleTree.Uploadable");
        };
        let Some(MerkleTreeBlob::Bytes(root_bytes)) =
            merkle_tree.blobs().get(merkle_tree.digest()).cloned()
        else {
            panic!("the root blob is not a byte[]");
        };
        assert_eq!(Directory::decode(&root_bytes[..]).unwrap(), root_directory);
        let platform1 = remote_action1
            .get_action()
            .platform
            .clone()
            .unwrap_or_default();
        assert_eq!(platform1.properties.len(), 1);
        assert_eq!(platform1.properties[0].name, "persistentWorkerKey");

        // Check that if a non-tool input changes, the persistent worker key does not change.
        f.fake_file_cache
            .create_scratch_input(&input(&input_artifact), "value2")
            .unwrap();
        let remote_action2 = service.build_remote_action(&*spawn, &context).unwrap();
        assert_eq!(
            remote_action2.get_action().platform,
            remote_action1.get_action().platform
        );

        // Check that if a tool input changes, the persistent worker key changes.
        f.fake_file_cache
            .create_scratch_input(&input(&tool_input), "worker value2")
            .unwrap();
        let remote_action3 = service.build_remote_action(&*spawn, &context).unwrap();
        let platform3 = remote_action3
            .get_action()
            .platform
            .clone()
            .unwrap_or_default();
        assert_eq!(platform3.properties.len(), 1);
        assert_eq!(platform3.properties[0].name, "persistentWorkerKey");
        assert_ne!(platform3.properties[0].value, platform1.properties[0].value);
    }
}

// Java: com/google/devtools/build/lib/remote/RemoteExecutionServiceTest.java:3014 RemoteExecutionServiceTest.buildRemoteActionWithScrubbing
#[test]
#[ignore = "needs ActionOwner.getTargetKind / isBuildConfigurationForTool on blaze_exec's ActionOwner (Scrubber.SpawnScrubber.matches); --experimental_remote_scrubbing_config is not ported"]
fn build_remote_action_with_scrubbing() {
    let mut f = Fixture::set_up();
    let kept_input = create_artifact_str(&f.artifact_root, "kept_input");
    f.fake_file_cache
        .create_scratch_input(&input(&kept_input), "kept")
        .unwrap();
    let scrubbed_input = create_artifact_str(&f.artifact_root, "scrubbed_input");
    f.fake_file_cache
        .create_scratch_input(&input(&scrubbed_input), "scrubbed")
        .unwrap();

    let spawn = SpawnBuilder::new(&["some/path/cmd"])
        .with_inputs([input(&kept_input), input(&scrubbed_input)])
        .with_execution_info(execution_requirements::NO_REMOTE_EXEC, "")
        .build();

    let context = f.new_spawn_execution_context(&spawn);
    // `remoteOptions.scrubber = new Scrubber(...)`.
    // PORT: the service takes the scrubber as an argument (see `RemoteExecutionService::new`).
    let scrubber = Scrubber::new(&Config {
        rules: vec![config::Rule {
            matcher: None,
            transform: Some(config::Transform {
                salt: "NaCl".to_string(),
                omitted_inputs: vec![".*scrubbed.*".to_string()],
                arg_replacements: vec![config::Replacement {
                    source: "some/path".to_string(),
                    target: "another/dir".to_string(),
                }],
            }),
        }],
    })
    .unwrap_or_else(|e| panic!("{}", e.get_message()));
    f.scrubber = Some(Arc::new(scrubber));
    f.remote_options.remote_discard_merkle_trees = false;
    let service = f.new_remote_execution_service_with_options(&f.remote_options);

    let remote_action = service.build_remote_action(&*spawn, &context).unwrap();

    let MerkleTree::Uploadable(merkle_tree) = remote_action.get_merkle_tree() else {
        panic!("ClassCastException: not a MerkleTree.Uploadable");
    };
    let blobs = merkle_tree.blobs();
    let blob_bytes = |digest: &Digest| -> bytes::Bytes {
        match blobs.get(digest) {
            Some(MerkleTreeBlob::Bytes(b)) => b.clone(),
            other => panic!("ClassCastException: {other:?} is not a byte[]"),
        }
    };
    let root_proto = Directory::decode(&blob_bytes(merkle_tree.digest())[..]).unwrap();
    let actual_root_dir =
        Directory::decode(&blob_bytes(root_proto.directories[0].digest.as_ref().unwrap())[..])
            .unwrap();

    let expected_root_dir = Directory {
        files: vec![FileNode {
            name: "kept_input".to_string(),
            digest: Some(Digest {
                hash: "79f076abdd19a752db7267bfff2f9022161d120dea919fdaca2ffdfc24ca8c96"
                    .to_string(),
                size_bytes: 4,
            }),
            is_executable: true,
            ..Default::default()
        }],
        ..Default::default()
    };

    assert_eq!(actual_root_dir, expected_root_dir);

    assert_eq!(
        remote_action.get_command().arguments,
        vec!["another/dir/cmd".to_string()]
    );

    assert_eq!(
        remote_action.get_action().salt,
        CacheSalt {
            scrub_salt: Some(cache_salt::ScrubSalt {
                salt: "NaCl".to_string(),
            }),
            ..Default::default()
        }
        .to_byte_array()
    );
}

// Java: com/google/devtools/build/lib/remote/RemoteExecutionServiceTest.java:3082 RemoteExecutionServiceTest.buildRemoteActionWithPathMapping
#[test]
#[ignore = "needs a path mapper: blaze_exec's PathMapper has only PathMapper.NOOP and RemotePathResolver.createMapped is unported"]
fn build_remote_action_with_path_mapping() {
    let mut f = Fixture::set_up();
    let mapped_input = create_artifact_str(&f.artifact_root, "bin/config/input1");
    f.fake_file_cache
        .create_scratch_input(&input(&mapped_input), "value1")
        .unwrap();
    let unmapped_input = create_artifact_str(&f.artifact_root, "bin/input2");
    f.fake_file_cache
        .create_scratch_input(&input(&unmapped_input), "value2")
        .unwrap();
    let output_dir =
        create_tree_artifact_with_generating_action_str(&f.artifact_root, "bin/config/output_dir");
    // PORT: Java's `.setPathMapper(execPath ->
    // PathFragment.create(execPath.getPathString().replaceAll("config/", "")))` cannot be
    // expressed with blaze_exec's `PathMapper`.
    let spawn = SpawnBuilder::new(&["unused"])
        .with_inputs([input(&mapped_input), input(&unmapped_input)])
        .with_output(from_path("outputs/bin/config/dir/output1"))
        .with_output(from_path("outputs/bin/other_dir/output2"))
        .with_output(input(&output_dir))
        .build();
    let context = f.new_spawn_execution_context(&spawn);
    f.remote_options.remote_discard_merkle_trees = false;
    let service = f.new_remote_execution_service_with_options(&f.remote_options);

    // Check that inputs and outputs of the remote action are mapped correctly.
    let remote_action = service.build_remote_action(&*spawn, &context).unwrap();
    let input_map: Vec<(PathFragment, ActionInputRef)> = remote_action
        .get_input_map(false)
        .iter()
        .map(|(k, v)| (k.0.clone(), v.clone()))
        .collect();
    assert_eq!(
        input_map,
        vec![
            (
                PathFragment::create("outputs/bin/input1"),
                input(&mapped_input)
            ),
            (
                PathFragment::create("outputs/bin/input2"),
                input(&unmapped_input)
            ),
        ]
    );
    assert!(remote_action.get_command().output_files.is_empty());
    assert!(remote_action.get_command().output_directories.is_empty());
    assert_eq!(
        remote_action.get_command().output_paths,
        vec![
            "outputs/bin/dir/output1".to_string(),
            "outputs/bin/other_dir/output2".to_string(),
            "outputs/bin/output_dir".to_string(),
        ]
    );

    // Check that the Merkle tree nodes are mapped correctly, including the output directory.
    let MerkleTree::Uploadable(merkle_tree) = remote_action.get_merkle_tree() else {
        panic!("ClassCastException: not a MerkleTree.Uploadable");
    };
    let blobs = merkle_tree.blobs();
    let blob_bytes = |digest: &Digest| -> bytes::Bytes {
        match blobs.get(digest) {
            Some(MerkleTreeBlob::Bytes(b)) => b.clone(),
            other => panic!("ClassCastException: {other:?} is not a byte[]"),
        }
    };
    let root_proto = Directory::decode(&blob_bytes(merkle_tree.digest())[..]).unwrap();
    let outputs_directory =
        Directory::decode(&blob_bytes(root_proto.directories[0].digest.as_ref().unwrap())[..])
            .unwrap();
    assert_eq!(outputs_directory.directories.len(), 1);
    let bin_directory = Directory::decode(
        &blob_bytes(outputs_directory.directories[0].digest.as_ref().unwrap())[..],
    )
    .unwrap();
    assert_eq!(
        bin_directory
            .files
            .iter()
            .map(|n| n.name.as_str())
            .collect::<Vec<_>>(),
        vec!["input1", "input2"]
    );
    assert_eq!(
        bin_directory
            .directories
            .iter()
            .map(|n| n.name.as_str())
            .collect::<Vec<_>>(),
        vec!["output_dir"]
    );
}

// Java: com/google/devtools/build/lib/remote/RemoteExecutionServiceTest.java:3136 RemoteExecutionServiceTest.buildRemoteAction_executablePathConformsToPlatform
#[test]
#[ignore = "needs a hook: blaze_exec's PlatformInfo has no constraints (OS_TO_DEFAULT_CONSTRAINT_VALUE), and the Windows OsPathPolicy is not ported"]
fn build_remote_action_executable_path_conforms_to_platform() {
    // `@TestParameter OS executionOs`: Java builds the spawn
    // `new SpawnBuilder("path/to/pkg/script.bat", "some/other/arg").withOutputs("out")
    // .withPlatform(PlatformInfo.builder().addConstraint(OS_TO_DEFAULT_CONSTRAINT_VALUE.get(os))
    // .build())` and expects the first argument to be "path\\to\\pkg\\script.bat" for Windows and
    // "path/to/pkg/script.bat" otherwise.
    blaze_util::unported!("PlatformInfo.builder().addConstraint (blaze_exec PlatformInfo)");
}

// Java: com/google/devtools/build/lib/remote/RemoteExecutionServiceTest.java:3160 RemoteExecutionServiceTest.buildRemoteAction_codePointSorting
#[test]
fn build_remote_action_code_point_sorting() {
    let f = Fixture::set_up();
    // PORT: StringEncoding identity (`unicodeToInternal`).
    let one = "path/ﾐ";
    let one_f = f.exec_fragment(one);
    let two = "path/🔥";
    let two_f = f.exec_fragment(two);
    let three = "sort/system-foo";
    let three_f = f.exec_fragment(three);
    let four = "sort/system/foo";
    let four_f = f.exec_fragment(four);
    let spawn = SpawnBuilder::new(&["dummy"])
        .with_output(input(&create_artifact_with_exec_path(
            &f.artifact_root,
            one_f,
        )))
        .with_output(input(&create_artifact_with_exec_path(
            &f.artifact_root,
            two_f,
        )))
        .with_output(input(&create_artifact_with_exec_path(
            &f.artifact_root,
            three_f,
        )))
        .with_output(input(&create_artifact_with_exec_path(
            &f.artifact_root,
            four_f,
        )))
        .with_environment(one, "value1")
        .with_environment(two, "value2")
        .with_environment(three, "value3")
        .with_environment(four, "value4")
        .build();
    let context = f.new_spawn_execution_context(&spawn);
    let service = f.new_remote_execution_service();

    let remote_action = service.build_remote_action(&*spawn, &context).unwrap();

    assert_eq!(
        remote_action.get_command().output_paths,
        vec![
            f.exec_path(one).get_path_string().to_string(),
            f.exec_path(two).get_path_string().to_string(),
            f.exec_path(three).get_path_string().to_string(),
            f.exec_path(four).get_path_string().to_string(),
        ]
    );
    let env = |name: &str, value: &str| EnvironmentVariable {
        name: name.to_string(),
        value: value.to_string(),
    };
    assert_eq!(
        remote_action.get_command().environment_variables,
        vec![
            env(one, "value1"),
            env(two, "value2"),
            env(three, "value3"),
            env(four, "value4"),
        ]
    );
}
