//! Port of `src/test/java/com/google/devtools/build/lib/actions/ActionCacheCheckerTest.java`.

use std::any::Any;
use std::collections::{HashMap, VecDeque};
use std::io::{self, Read, Write};
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use blaze_collect::{NestedSet, Order};
use blaze_events::{Clock, NullEventHandler};
use blaze_exec::actions::action_cache_checker::{
    ActionCacheChecker, ActionCacheCheckerCacheConfig, ArtifactResolver, CacheCheckAction,
    OutputChecker, PackageRootExceptionOrInterrupted, PackageRootResolver, ProxyMetadataFactory,
    Token, TrustAll,
};
use blaze_exec::actions::cache::action_cache::{
    ActionCache, ActionCacheEntry, ActionCacheStatistics, MissDetail, MissReason,
    OutputPermissions, SerializableTreeArtifactValue,
};
use blaze_exec::actions::cache::compact_persistent_action_cache::CompactPersistentActionCache;
use blaze_exec::actions::cache::metadata_digest_utils::get_expiration_time;
use blaze_exec::actions::cache::output_metadata_store::{
    IoOrInterruptedException, OutputMetadataStore,
};
use blaze_exec::actions::{
    ActionInput, ActionInputRef, Artifact, FileArtifactValue, FilesetOutputTree,
    InputMetadataProvider, RunfilesArtifactValue, TreeArtifactValue,
};
use blaze_shell::InterruptedException;
use blaze_util::Fingerprint;
use blaze_vfs::{DigestHashFunction, Path, PathFragment, UnixFileSystem};
use indexmap::IndexMap;
use sha2::{Digest, Sha256};

// ---------------------------------------------------------------------------------------------
// Test doubles.
// ---------------------------------------------------------------------------------------------

/// Port of `testutil/ManualClock`.
struct ManualClock {
    current_time_millis: AtomicI64,
}

impl Clock for ManualClock {
    fn current_time_millis(&self) -> i64 {
        self.current_time_millis.load(Ordering::SeqCst)
    }
    fn nano_time(&self) -> i64 {
        self.current_time_millis.load(Ordering::SeqCst) * 1_000_000 + 1000 * 1_000_000_000
    }
}

/// A test artifact (source, derived, tree or constant-metadata special artifact).
#[derive(Debug)]
struct FakeArtifact {
    exec_path: PathFragment,
    path: Path,
    tree: bool,
    constant_metadata: bool,
    parent: Option<ActionInputRef>,
    parent_relative_path: Option<PathFragment>,
}

impl ActionInput for FakeArtifact {
    fn get_exec_path(&self) -> &PathFragment {
        &self.exec_path
    }
    fn is_directory(&self) -> bool {
        self.tree
    }
    fn is_symlink(&self) -> bool {
        false
    }
    fn as_artifact(&self) -> Option<&dyn Artifact> {
        Some(self)
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl Artifact for FakeArtifact {
    fn is_source_artifact(&self) -> bool {
        false
    }
    fn is_tree_artifact(&self) -> bool {
        self.tree
    }
    fn is_runfiles_tree(&self) -> bool {
        false
    }
    fn is_fileset(&self) -> bool {
        false
    }
    fn is_constant_metadata(&self) -> bool {
        self.constant_metadata
    }
    fn is_child_of_declared_directory(&self) -> bool {
        self.parent.is_some()
    }
    fn get_parent(&self) -> Option<ActionInputRef> {
        self.parent.clone()
    }
    fn get_parent_relative_path(&self) -> Option<PathFragment> {
        self.parent_relative_path.clone()
    }
    fn get_root_relative_path(&self) -> PathFragment {
        self.exec_path.clone()
    }
    fn get_root_exec_path(&self) -> PathFragment {
        PathFragment::create("")
    }
}

fn artifact_path(input: &ActionInputRef) -> Path {
    input
        .as_any()
        .downcast_ref::<FakeArtifact>()
        .unwrap()
        .path
        .clone()
}

fn write_content_as_latin1(path: &Path, content: &str) {
    if let Some(parent) = path.get_parent_directory() {
        parent.create_directory_and_parents().unwrap();
    }
    let mut out = path.get_output_stream().unwrap();
    out.write_all(content.as_bytes()).unwrap();
}

fn read_content(path: &Path) -> Vec<u8> {
    let mut bytes = Vec::new();
    path.get_input_stream()
        .unwrap()
        .read_to_end(&mut bytes)
        .unwrap();
    bytes
}

/// What the action does when "executed".
enum Behavior {
    /// `NullAction.execute`.
    Null,
    /// `WriteEmptyOutputAction.execute`.
    WriteEmptyOutput,
    /// `InjectOutputFileMetadataAction.execute`.
    InjectFile(Mutex<VecDeque<FileArtifactValue>>),
    /// `InjectOutputTreeMetadataAction.execute`.
    InjectTree(Mutex<VecDeque<TreeArtifactValue>>),
    /// `WriteEmptyOutputAction` overridden to inject remote metadata first.
    InjectThenWriteEmptyOutput(FileArtifactValue),
}

/// Port of `ActionsTestUtil.NullAction` and its test subclasses.
struct FakeAction {
    outputs: Vec<ActionInputRef>,
    key: String,
    client_environment_variables: Vec<String>,
    execute_unconditionally: bool,
    is_volatile: bool,
    behavior: Behavior,
}

impl FakeAction {
    fn new(outputs: Vec<ActionInputRef>, behavior: Behavior) -> FakeAction {
        FakeAction {
            outputs,
            // NullAction.computeKey: fp.addString("action").
            key: "action".to_string(),
            client_environment_variables: Vec::new(),
            execute_unconditionally: false,
            is_volatile: false,
            behavior,
        }
    }

    fn execute(&self, output_metadata_store: &dyn OutputMetadataStore) {
        match &self.behavior {
            Behavior::Null => {}
            Behavior::WriteEmptyOutput => self.write_empty_outputs(),
            Behavior::InjectFile(deque) => {
                let metadata = deque.lock().unwrap().pop_front().unwrap();
                output_metadata_store.inject_file(&self.outputs[0], metadata);
            }
            Behavior::InjectTree(deque) => {
                let metadata = deque.lock().unwrap().pop_front().unwrap();
                output_metadata_store.inject_tree(&self.outputs[0], metadata);
            }
            Behavior::InjectThenWriteEmptyOutput(metadata) => {
                output_metadata_store.inject_file(&self.outputs[0], metadata.clone());
                self.write_empty_outputs();
            }
        }
    }

    fn write_empty_outputs(&self) {
        for output in &self.outputs {
            write_content_as_latin1(&artifact_path(output), "");
        }
    }
}

impl CacheCheckAction for FakeAction {
    fn get_key(
        &self,
        _input_metadata_provider: &dyn InputMetadataProvider,
    ) -> Result<String, InterruptedException> {
        Ok(Fingerprint::new()
            .add_string(&self.key)
            .hex_digest_and_reset())
    }
    fn get_inputs(&self) -> NestedSet<ActionInputRef> {
        Order::STABLE_ORDER.empty_set()
    }
    fn get_mandatory_inputs(&self) -> NestedSet<ActionInputRef> {
        Order::STABLE_ORDER.empty_set()
    }
    fn get_allowed_derived_inputs(&self) -> NestedSet<ActionInputRef> {
        Order::STABLE_ORDER.empty_set()
    }
    fn get_outputs(&self) -> Vec<ActionInputRef> {
        self.outputs.clone()
    }
    fn get_primary_output(&self) -> ActionInputRef {
        self.outputs[0].clone()
    }
    fn inputs_known(&self) -> bool {
        true
    }
    fn discovers_inputs(&self) -> bool {
        false
    }
    fn update_inputs(&self, _inputs: NestedSet<ActionInputRef>) {
        panic!("UnsupportedOperationException");
    }
    fn execute_unconditionally(&self) -> bool {
        self.execute_unconditionally
    }
    fn is_volatile(&self) -> bool {
        self.is_volatile
    }
    fn get_client_environment_variables(&self) -> Vec<String> {
        self.client_environment_variables.clone()
    }
    fn pretty_print(&self) -> String {
        "action 'Null'".to_string()
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// An `ActionCache` that allows injecting corruption for testing.
struct CorruptibleActionCache {
    delegate: Arc<CompactPersistentActionCache>,
    corrupted: AtomicBool,
}

impl CorruptibleActionCache {
    fn corrupt_all_entries(&self) {
        self.corrupted.store(true, Ordering::SeqCst);
    }
}

impl ActionCache for CorruptibleActionCache {
    fn put(&self, key: &str, entry: ActionCacheEntry) {
        self.delegate.put(key, entry);
    }
    fn get(&self, key: &str) -> Option<ActionCacheEntry> {
        if self.corrupted.load(Ordering::SeqCst) {
            Some(ActionCacheEntry::corrupted())
        } else {
            self.delegate.get(key)
        }
    }
    fn remove(&self, key: &str) {
        self.delegate.remove(key);
    }
    fn remove_if(&self, predicate: &dyn Fn(&ActionCacheEntry) -> bool) {
        self.delegate.remove_if(predicate);
    }
    fn save(&self) -> io::Result<i64> {
        self.delegate.save()
    }
    fn clear(&self) {
        self.delegate.clear();
    }
    fn trim(
        self: Arc<Self>,
        threshold: f32,
        max_age: Duration,
    ) -> Result<Arc<dyn ActionCache>, IoOrInterruptedException> {
        let trimmed: Arc<dyn ActionCache> = self.delegate.clone().trim(threshold, max_age)?;
        Ok(trimmed)
    }
    fn dump(&self, out: &mut dyn Write) {
        self.delegate.dump(out);
    }
    fn size(&self) -> i32 {
        self.delegate.size()
    }
    fn account_hit(&self) {
        self.delegate.account_hit();
    }
    fn account_miss(&self, reason: MissReason) {
        self.delegate.account_miss(reason);
    }
    fn merge_into_action_cache_statistics(&self, builder: &mut ActionCacheStatistics) {
        self.delegate.merge_into_action_cache_statistics(builder);
    }
    fn reset_statistics(&self) {
        self.delegate.reset_statistics();
    }
}

/// A fake metadata handler that is able to obtain metadata from the file system.
#[derive(Default)]
struct FakeInputMetadataHandler {
    /// Java `Map<Artifact, FileArtifactValue>` (values may be null).
    file_metadata: Mutex<HashMap<ActionInputRef, Option<FileArtifactValue>>>,
    tree_metadata: Mutex<HashMap<ActionInputRef, TreeArtifactValue>>,
}

impl InputMetadataProvider for FakeInputMetadataHandler {
    fn get_input_metadata(&self, input: &ActionInputRef) -> io::Result<Option<FileArtifactValue>> {
        if input.as_artifact().is_none() {
            return Ok(None);
        }
        FileArtifactValue::create_for_testing(&artifact_path(input)).map(Some)
    }
    fn get_tree_metadata(&self, _input: &ActionInputRef) -> Option<Arc<TreeArtifactValue>> {
        panic!("UnsupportedOperationException");
    }
    fn get_fileset(&self, _input: &ActionInputRef) -> Option<Arc<FilesetOutputTree>> {
        panic!("UnsupportedOperationException");
    }
    fn get_runfiles_metadata(&self, _input: &ActionInputRef) -> Option<Arc<RunfilesArtifactValue>> {
        panic!("UnsupportedOperationException");
    }
    fn get_input(&self, _exec_path: &PathFragment) -> Option<ActionInputRef> {
        panic!("UnsupportedOperationException");
    }
}

impl OutputMetadataStore for FakeInputMetadataHandler {
    fn inject_file(&self, output: &ActionInputRef, metadata: FileArtifactValue) {
        self.file_metadata
            .lock()
            .unwrap()
            .insert(output.clone(), Some(metadata));
    }
    fn inject_tree(&self, output: &ActionInputRef, tree: TreeArtifactValue) {
        self.tree_metadata
            .lock()
            .unwrap()
            .insert(output.clone(), tree);
    }
    fn get_output_metadata(
        &self,
        output: &ActionInputRef,
    ) -> Result<Option<FileArtifactValue>, IoOrInterruptedException> {
        let Some(artifact) = output.as_artifact() else {
            return Ok(None);
        };
        if artifact.is_tree_artifact() {
            return Ok(self.get_tree_artifact_value(output)?.metadata);
        }
        if let Some(metadata) = self.file_metadata.lock().unwrap().get(output) {
            return Ok(metadata.clone());
        }
        Ok(Some(FileArtifactValue::create_for_testing(
            &artifact_path(output),
        )?))
    }
    fn get_tree_artifact_value(
        &self,
        output: &ActionInputRef,
    ) -> Result<TreeArtifactValue, IoOrInterruptedException> {
        if let Some(tree) = self.tree_metadata.lock().unwrap().get(output) {
            return Ok(tree.clone());
        }
        let tree_dir = artifact_path(output);
        if !tree_dir.exists() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("{} does not exist", output),
            )
            .into());
        }
        panic!("needs TreeArtifactValue.visitTree and TreeArtifactValue.Builder");
    }
    fn mark_omitted(&self, _output: &ActionInputRef) {
        panic!("UnsupportedOperationException");
    }
    fn artifact_omitted(&self, _artifact: &ActionInputRef) -> bool {
        false
    }
    fn reset_outputs(&self, _outputs: &[ActionInputRef]) {
        panic!("UnsupportedOperationException");
    }
}

/// `FakeArtifactResolverBase`: all operations fail.
struct FakeArtifactResolverBase;

impl ArtifactResolver for FakeArtifactResolverBase {
    fn resolve_source_artifacts(
        &self,
        _exec_paths: &[PathFragment],
        _resolver: &dyn PackageRootResolver,
    ) -> Result<Option<HashMap<PathFragment, ActionInputRef>>, PackageRootExceptionOrInterrupted>
    {
        panic!("UnsupportedOperationException");
    }
}

/// Stands in for `mock(ProxyMetadataFactory.class)`.
#[derive(Default)]
struct FakeProxyMetadataFactory {
    /// `Err(message)` makes the factory throw an `IOException`.
    result: Mutex<Option<Result<Option<FileArtifactValue>, String>>>,
}

impl FakeProxyMetadataFactory {
    fn set(&self, result: Result<Option<FileArtifactValue>, String>) {
        *self.result.lock().unwrap() = Some(result);
    }
}

impl ProxyMetadataFactory for FakeProxyMetadataFactory {
    fn create_proxy_metadata(
        &self,
        _artifact: &ActionInputRef,
    ) -> io::Result<Option<FileArtifactValue>> {
        match &*self.result.lock().unwrap() {
            // Mockito's default answer.
            None => Ok(None),
            Some(Ok(value)) => Ok(value.clone()),
            Some(Err(message)) => Err(io::Error::other(message.clone())),
        }
    }
}

/// Stands in for `mock(OutputChecker.class)`: records the calls, answers `trust`.
struct RecordingOutputChecker {
    trust: bool,
    calls: Mutex<Vec<String>>,
}

impl RecordingOutputChecker {
    fn new(trust: bool) -> RecordingOutputChecker {
        RecordingOutputChecker {
            trust,
            calls: Mutex::new(Vec::new()),
        }
    }
}

impl OutputChecker for RecordingOutputChecker {
    fn should_trust_metadata(&self, file: &dyn ActionInput, _metadata: &FileArtifactValue) -> bool {
        self.calls.lock().unwrap().push(file.get_exec_path_string());
        self.trust
    }
}

/// `CHECK_TTL`: trust metadata that has not expired.
struct CheckTtl;

impl OutputChecker for CheckTtl {
    fn should_trust_metadata(&self, _file: &dyn ActionInput, metadata: &FileArtifactValue) -> bool {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64;
        match get_expiration_time(metadata) {
            None => true,
            Some(expiration_time) => expiration_time > now,
        }
    }
}

/// Port of `ActionsTestUtil.MissDetailsBuilder`.
struct MissDetailsBuilder {
    details: Vec<(MissReason, i32)>,
}

impl MissDetailsBuilder {
    /// Constructs a new builder with all possible cache miss reasons set to zero counts.
    fn new() -> MissDetailsBuilder {
        MissDetailsBuilder {
            details: [
                MissReason::DifferentActionKey,
                MissReason::DifferentDeps,
                MissReason::DifferentEnvironment,
                MissReason::DifferentFiles,
                MissReason::CorruptedCacheEntry,
                MissReason::NotCached,
                MissReason::UnconditionalExecution,
                MissReason::DigestMismatch,
            ]
            .into_iter()
            .map(|r| (r, 0))
            .collect(),
        }
    }

    /// Sets the count of the given miss reason to the given value.
    fn set(mut self, reason: MissReason, count: i32) -> MissDetailsBuilder {
        for (r, c) in &mut self.details {
            if *r == reason {
                *c = count;
            }
        }
        self
    }

    fn build(self) -> Vec<MissDetail> {
        self.details
            .into_iter()
            .map(|(reason, count)| MissDetail {
                reason: reason as i32,
                count,
            })
            .collect()
    }
}

// ---------------------------------------------------------------------------------------------
// Fixture.
// ---------------------------------------------------------------------------------------------

struct Fixture {
    _tmp: tempfile::TempDir,
    exec_root: Path,
    cache: Arc<CorruptibleActionCache>,
    cache_checker: ActionCacheChecker,
    proxy_metadata_factory: Arc<FakeProxyMetadataFactory>,
}

fn digest(content: &[u8]) -> Vec<u8> {
    Sha256::digest(content).to_vec()
}

impl Fixture {
    fn new() -> Fixture {
        let tmp = tempfile::tempdir().unwrap();
        let fs = UnixFileSystem::create(DigestHashFunction::Sha256);
        let root = Path::create_from_str(tmp.path().to_str().unwrap(), &fs);
        let clock = Arc::new(ManualClock {
            current_time_millis: AtomicI64::new(0),
        });
        let cache_root = root.get_child("cache_root");
        let corrupted_cache_root = root.get_child("corrupted_cache_root");
        let tmp_dir = root.get_child("cache_tmp_dir");
        let exec_root = root.get_child("output");
        let cache = Arc::new(CorruptibleActionCache {
            delegate: Arc::new(
                CompactPersistentActionCache::create(
                    &cache_root,
                    &corrupted_cache_root,
                    &tmp_dir,
                    clock,
                    NullEventHandler::INSTANCE,
                )
                .unwrap(),
            ),
            corrupted: AtomicBool::new(false),
        });
        let proxy_metadata_factory = Arc::new(FakeProxyMetadataFactory::default());
        let cache_checker = create_action_cache_checker(&cache, &proxy_metadata_factory, false);
        Fixture {
            _tmp: tmp,
            exec_root,
            cache,
            cache_checker,
            proxy_metadata_factory,
        }
    }

    fn use_store_output_metadata(&mut self, store_output_metadata: bool) {
        self.cache_checker = create_action_cache_checker(
            &self.cache,
            &self.proxy_metadata_factory,
            store_output_metadata,
        );
    }

    /// `ActionsTestUtil.DUMMY_ARTIFACT`.
    fn dummy_artifact(&self) -> ActionInputRef {
        ActionInputRef::new(FakeArtifact {
            exec_path: PathFragment::create("/dummy"),
            path: self.exec_root.get_child("dummy"),
            tree: false,
            constant_metadata: false,
            parent: None,
            parent_relative_path: None,
        })
    }

    /// `ActionsTestUtil.createArtifact(artifactRoot, path)`.
    fn create_artifact(&self, path: &str) -> ActionInputRef {
        self.create_special(path, false, false)
    }

    /// `createTreeArtifactWithGeneratingAction(artifactRoot, path)`.
    fn create_tree_artifact(&self, path: &str) -> ActionInputRef {
        self.create_special(path, true, false)
    }

    fn create_special(&self, path: &str, tree: bool, constant_metadata: bool) -> ActionInputRef {
        let exec_path = PathFragment::create("bin").get_relative(&PathFragment::create(path));
        ActionInputRef::new(FakeArtifact {
            path: self.exec_root.get_relative(&exec_path),
            exec_path,
            tree,
            constant_metadata,
            parent: None,
            parent_relative_path: None,
        })
    }

    fn write_empty_output_action(&self) -> FakeAction {
        FakeAction::new(vec![self.dummy_artifact()], Behavior::WriteEmptyOutput)
    }

    fn get_token(
        &self,
        action: &FakeAction,
        handler: &FakeInputMetadataHandler,
        output_checker: Option<&dyn OutputChecker>,
    ) -> Option<Token> {
        self.cache_checker
            .get_token_if_need_to_execute(
                action,
                /* resolved_cache_artifacts= */ None,
                /* client_env= */ &IndexMap::new(),
                OutputPermissions::Readonly,
                /* handler= */ None,
                handler,
                handler,
                /* action_execution_salt= */ "",
                output_checker,
                /* use_archived_tree_artifacts= */ false,
            )
            .unwrap()
    }

    /// "Executes" the given action from the point of view of the cache's lifecycle.
    fn run_action(&self, action: &FakeAction) {
        self.run_action_env(action, &IndexMap::new(), "");
    }

    fn run_action_env(
        &self,
        action: &FakeAction,
        client_env: &IndexMap<String, String>,
        salt: &str,
    ) {
        let metadata_handler = FakeInputMetadataHandler::default();
        self.run_action_full(
            action,
            client_env,
            salt,
            &metadata_handler,
            &metadata_handler,
            Some(&TrustAll),
        );
    }

    fn run_action_full(
        &self,
        action: &FakeAction,
        client_env: &IndexMap<String, String>,
        action_execution_salt: &str,
        input_metadata_provider: &dyn InputMetadataProvider,
        output_metadata_store: &dyn OutputMetadataStore,
        output_checker: Option<&dyn OutputChecker>,
    ) {
        let token = self
            .cache_checker
            .get_token_if_need_to_execute(
                action,
                /* resolved_cache_artifacts= */ None,
                client_env,
                OutputPermissions::Readonly,
                /* handler= */ None,
                input_metadata_provider,
                output_metadata_store,
                action_execution_salt,
                output_checker,
                /* use_archived_tree_artifacts= */ false,
            )
            .unwrap();
        self.run_action_with_token(
            action,
            client_env,
            action_execution_salt,
            input_metadata_provider,
            output_metadata_store,
            token.as_ref(),
        );
    }

    fn run_action_with_token(
        &self,
        action: &FakeAction,
        client_env: &IndexMap<String, String>,
        action_execution_salt: &str,
        input_metadata_provider: &dyn InputMetadataProvider,
        output_metadata_store: &dyn OutputMetadataStore,
        token: Option<&Token>,
    ) {
        if let Some(token) = token {
            for artifact in action.get_outputs() {
                // PORT: outputs live in the test's temporary directory, removed with it.
                if let Some(parent) = artifact_path(&artifact).get_parent_directory() {
                    parent.create_directory_and_parents().unwrap();
                }
            }

            // Real action execution would happen here.
            action.execute(output_metadata_store);

            self.cache_checker
                .update_action_cache(
                    action,
                    token,
                    input_metadata_provider,
                    output_metadata_store,
                    client_env,
                    OutputPermissions::Readonly,
                    action_execution_salt,
                    /* use_archived_tree_artifacts= */ false,
                )
                .unwrap();
        }
    }

    /// Ensures that the cache statistics match exactly the given values.
    fn assert_statistics(&self, hits: i32, misses: Vec<MissDetail>) {
        let mut stats = ActionCacheStatistics::default();
        self.cache.merge_into_action_cache_statistics(&mut stats);

        assert_eq!(stats.hits, hits);
        let mut actual = stats.miss_details.clone();
        let mut expected = misses;
        actual.sort_by_key(|d| (d.reason, d.count));
        expected.sort_by_key(|d| (d.reason, d.count));
        assert_eq!(actual, expected);
    }

    fn do_test_not_cached(&self, action: &FakeAction, miss_reason: MissReason) {
        self.run_action(action);

        self.assert_statistics(0, MissDetailsBuilder::new().set(miss_reason, 1).build());
    }

    fn do_test_cached(&self, action: &FakeAction, miss_reason: MissReason) {
        let runs = 5;
        for _ in 0..runs {
            self.run_action(action);
        }

        self.assert_statistics(
            runs - 1,
            MissDetailsBuilder::new().set(miss_reason, 1).build(),
        );
    }

    fn do_test_corrupted_cache_entry(&self, action: &FakeAction) {
        self.cache.corrupt_all_entries();
        self.run_action(action);

        self.assert_statistics(
            0,
            MissDetailsBuilder::new()
                .set(MissReason::CorruptedCacheEntry, 1)
                .build(),
        );
    }
}

fn create_action_cache_checker(
    cache: &Arc<CorruptibleActionCache>,
    proxy_metadata_factory: &Arc<FakeProxyMetadataFactory>,
    store_output_metadata: bool,
) -> ActionCacheChecker {
    let action_cache: Arc<dyn ActionCache> = cache.clone();
    let proxy_metadata_factory: Arc<dyn ProxyMetadataFactory> = proxy_metadata_factory.clone();
    ActionCacheChecker::new(
        Some(action_cache),
        Arc::new(FakeArtifactResolverBase),
        Box::new(|_: &dyn CacheCheckAction| true),
        proxy_metadata_factory,
        Some(
            ActionCacheCheckerCacheConfig::builder()
                .set_enabled(true)
                .set_store_output_metadata(store_output_metadata)
                .build(),
        ),
    )
}

fn create_remote_metadata(content: &str) -> FileArtifactValue {
    create_remote_metadata_full(content, None, None)
}

fn create_remote_metadata_full(
    content: &str,
    expiration_time: Option<i64>,
    resolved_path: Option<PathFragment>,
) -> FileArtifactValue {
    let bytes = content.as_bytes();
    let mut metadata = FileArtifactValue::create_for_remote_file_with_materialization_data(
        digest(bytes),
        bytes.len() as i64,
        1,
        expiration_time,
        /* in_memory_output= */ false,
    );
    if let Some(resolved_path) = resolved_path {
        metadata =
            FileArtifactValue::create_from_existing_with_resolved_path(metadata, resolved_path);
    }
    metadata
}

fn create_proxy_metadata(artifact: &ActionInputRef, content: &str) -> FileArtifactValue {
    let path = artifact_path(artifact);
    write_content_as_latin1(&path, content);
    FileArtifactValue::Proxy {
        delegate: Box::new(FileArtifactValue::create_for_testing(&path).unwrap()),
        path,
    }
}

/// `TreeFileArtifact.createTreeOutput(parent, name)`.
fn create_tree_output(parent: &ActionInputRef, name: &str) -> ActionInputRef {
    let rel = PathFragment::create(name);
    ActionInputRef::new(FakeArtifact {
        exec_path: parent.get_exec_path().get_relative(&rel),
        path: artifact_path(parent).get_relative(&rel),
        tree: false,
        constant_metadata: false,
        parent: Some(parent.clone()),
        parent_relative_path: Some(rel),
    })
}

/// `ArchivedTreeArtifact.createForTree(parent)`.
fn create_archived_for_tree(parent: &ActionInputRef) -> ActionInputRef {
    let exec_path = PathFragment::create(&format!(
        ":archived_tree_artifacts/{}.zip",
        parent.get_exec_path_string()
    ));
    ActionInputRef::new(FakeArtifact {
        path: artifact_path(parent)
            .get_parent_directory()
            .unwrap()
            .get_relative(&exec_path),
        exec_path,
        tree: false,
        constant_metadata: false,
        parent: None,
        parent_relative_path: None,
    })
}

fn create_tree_metadata(
    parent: &ActionInputRef,
    children: &[(&str, FileArtifactValue)],
    archived_artifact_value: Option<FileArtifactValue>,
    resolved_path: Option<PathFragment>,
) -> TreeArtifactValue {
    let mut child_values: Vec<(ActionInputRef, FileArtifactValue)> = children
        .iter()
        .map(|(name, value)| (create_tree_output(parent, name), value.clone()))
        .collect();
    child_values.sort_by(|a, b| a.0.get_exec_path().cmp(b.0.get_exec_path()));
    TreeArtifactValue {
        child_values,
        archived_representation: archived_artifact_value
            .map(|metadata| (create_archived_for_tree(parent), metadata)),
        // PORT: TreeArtifactValue.Builder.build computes the aggregate metadata; these tests only
        // assert the serialized tree, so a fixed aggregate stands in.
        metadata: Some(FileArtifactValue::create_for_directory_with_mtime(0)),
        resolved_path,
    }
}

fn children_map(children: &[(&str, FileArtifactValue)]) -> IndexMap<String, FileArtifactValue> {
    children
        .iter()
        .map(|(k, v)| (k.to_string(), v.clone()))
        .collect()
}

// ---------------------------------------------------------------------------------------------
// Tests.
// ---------------------------------------------------------------------------------------------

#[test]
fn test_no_activity() {
    let f = Fixture::new();
    f.assert_statistics(0, MissDetailsBuilder::new().build());
}

#[test]
fn test_not_cached() {
    let f = Fixture::new();
    f.do_test_not_cached(&f.write_empty_output_action(), MissReason::NotCached);
}

#[test]
fn test_cached() {
    let f = Fixture::new();
    f.do_test_cached(&f.write_empty_output_action(), MissReason::NotCached);
}

#[test]
fn test_corrupted_cache_entry() {
    let f = Fixture::new();
    f.do_test_corrupted_cache_entry(&f.write_empty_output_action());
}

#[test]
fn test_different_action_key() {
    let f = Fixture::new();
    let mut action = f.write_empty_output_action();
    action.key = "key1".to_string();
    f.run_action(&action);
    let mut action = FakeAction::new(vec![f.dummy_artifact()], Behavior::Null);
    action.key = "key2".to_string();
    f.run_action(&action);

    f.assert_statistics(
        0,
        MissDetailsBuilder::new()
            .set(MissReason::DigestMismatch, 1)
            .set(MissReason::NotCached, 1)
            .build(),
    );
}

#[test]
fn test_different_environment() {
    let f = Fixture::new();
    let mut action = f.write_empty_output_action();
    action.client_environment_variables = vec!["used-var".to_string()];

    let env = |pairs: &[(&str, &str)]| -> IndexMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    };

    f.run_action_env(&action, &env(&[("unused-var", "1")]), ""); // Not cached.
    f.run_action_env(&action, &env(&[]), ""); // Cache hit because we only modified uninteresting variables.
    f.run_action_env(&action, &env(&[("used-var", "2")]), ""); // Cache miss because of different environment.
    f.run_action_env(&action, &env(&[("used-var", "2")]), ""); // Cache hit because we did not change anything.

    f.assert_statistics(
        2,
        MissDetailsBuilder::new()
            .set(MissReason::DigestMismatch, 1)
            .set(MissReason::NotCached, 1)
            .build(),
    );
}

#[test]
fn test_different_salt() {
    let f = Fixture::new();
    let action = f.write_empty_output_action();
    let env: IndexMap<String, String> =
        IndexMap::from([("unused-var".to_string(), "1".to_string())]);

    // Not cached.
    f.run_action_env(&action, &env, "foo");
    // Cache hit because actionExecutionSalt did not change.
    f.run_action_env(&action, &env, "foo");
    // Cache miss because actionExecutionSalt changed.
    f.run_action_env(&action, &env, "bar");

    f.assert_statistics(
        1,
        MissDetailsBuilder::new()
            .set(MissReason::DigestMismatch, 1)
            .set(MissReason::NotCached, 1)
            .build(),
    );
}

#[test]
fn test_different_files() {
    let f = Fixture::new();
    let action = f.write_empty_output_action();
    f.run_action(&action); // Not cached.
    assert!(read_content(&artifact_path(&action.get_primary_output())).is_empty());
    write_content_as_latin1(&artifact_path(&action.get_primary_output()), "modified");
    f.run_action(&action); // Cache miss because output files were modified.

    f.assert_statistics(
        0,
        MissDetailsBuilder::new()
            .set(MissReason::DigestMismatch, 1)
            .set(MissReason::NotCached, 1)
            .build(),
    );
}

#[test]
fn test_unconditional_execution() {
    let f = Fixture::new();
    let mut action = f.write_empty_output_action();
    action.execute_unconditionally = true;
    action.is_volatile = true;

    let runs = 5;
    for _ in 0..runs {
        f.run_action(&action);
    }

    f.assert_statistics(
        0,
        MissDetailsBuilder::new()
            .set(MissReason::UnconditionalExecution, runs)
            .build(),
    );
}

#[test]
fn test_deleted_constant_metadata_output_causes_reexecution() {
    let f = Fixture::new();
    let output = f.create_special("bin/dummy", false, /* constant_metadata= */ true);
    artifact_path(&output)
        .get_parent_directory()
        .unwrap()
        .create_directory_and_parents()
        .unwrap();
    let action = FakeAction::new(vec![output.clone()], Behavior::WriteEmptyOutput);
    f.run_action(&action);
    artifact_path(&output).delete().unwrap();
    let fake_metadata_handler = FakeInputMetadataHandler::default();
    assert!(
        f.get_token(&action, &fake_metadata_handler, Some(&TrustAll))
            .is_some()
    );
}

#[test]
fn save_output_metadata_remote_file_metadata_saved() {
    let mut f = Fixture::new();
    f.use_store_output_metadata(true);
    let output = f.create_artifact("bin/dummy");
    let content = "content";
    let action = FakeAction::new(
        vec![output.clone()],
        Behavior::InjectFile(Mutex::new(VecDeque::from([create_remote_metadata(
            content,
        )]))),
    );

    // Not cached.
    f.run_action(&action);

    assert!(!artifact_path(&output).exists());
    let entry = f.cache.get(&output.get_exec_path_string());
    assert!(entry.is_some());
    assert_eq!(
        entry.unwrap().get_output_file(&*output),
        Some(&create_remote_metadata(content))
    );
    f.assert_statistics(
        0,
        MissDetailsBuilder::new()
            .set(MissReason::NotCached, 1)
            .build(),
    );
}

#[test]
fn save_output_metadata_local_file_metadata_not_saved() {
    let mut f = Fixture::new();
    f.use_store_output_metadata(true);
    let output = f.create_artifact("bin/dummy");
    let action = FakeAction::new(vec![output.clone()], Behavior::WriteEmptyOutput);
    let _ = artifact_path(&output).delete();

    f.run_action(&action);

    assert!(artifact_path(&output).exists());
    let entry = f.cache.get(&output.get_exec_path_string());
    assert!(entry.is_some());
    assert_eq!(entry.unwrap().get_output_file(&*output), None);
    f.assert_statistics(
        0,
        MissDetailsBuilder::new()
            .set(MissReason::NotCached, 1)
            .build(),
    );
}

#[test]
fn save_output_metadata_remote_metadata_injected_and_local_files_stored() {
    let mut f = Fixture::new();
    f.use_store_output_metadata(true);
    let output = f.create_artifact("bin/dummy");
    let action = FakeAction::new(
        vec![output.clone()],
        Behavior::InjectThenWriteEmptyOutput(create_remote_metadata("")),
    );
    let _ = artifact_path(&output).delete();

    f.run_action(&action);

    assert!(artifact_path(&output).exists());
    let entry = f.cache.get(&output.get_exec_path_string());
    assert!(entry.is_some());
    assert_eq!(
        entry.unwrap().get_output_file(&*output),
        Some(&create_remote_metadata(""))
    );
    f.assert_statistics(
        0,
        MissDetailsBuilder::new()
            .set(MissReason::NotCached, 1)
            .build(),
    );
}

#[test]
fn save_output_metadata_not_saved_if_disabled() {
    let f = Fixture::new();
    let output = f.create_artifact("bin/dummy");
    let content = "content";
    let action = FakeAction::new(
        vec![output.clone()],
        Behavior::InjectFile(Mutex::new(VecDeque::from([create_remote_metadata(
            content,
        )]))),
    );

    f.run_action(&action);

    assert!(!artifact_path(&output).exists());
    let entry = f.cache.get(&output.get_exec_path_string());
    assert!(entry.is_some());
    assert_eq!(entry.unwrap().get_output_file(&*output), None);
    f.assert_statistics(
        0,
        MissDetailsBuilder::new()
            .set(MissReason::NotCached, 1)
            .build(),
    );
}

#[test]
fn save_output_metadata_remote_file_metadata_loaded() {
    let mut f = Fixture::new();
    f.use_store_output_metadata(true);
    let output = f.create_artifact("bin/dummy");
    let content = "content";
    let action = FakeAction::new(
        vec![output.clone()],
        Behavior::InjectFile(Mutex::new(VecDeque::from([create_remote_metadata(
            content,
        )]))),
    );
    let metadata_handler = FakeInputMetadataHandler::default();

    f.run_action(&action);
    let token = f.get_token(&action, &metadata_handler, Some(&TrustAll));

    assert!(!artifact_path(&output).exists());
    assert!(token.is_none());
    let entry = f.cache.get(&output.get_exec_path_string());
    assert!(entry.is_some());
    assert_eq!(
        entry.unwrap().get_output_file(&*output),
        Some(&create_remote_metadata(content))
    );
    assert_eq!(
        metadata_handler.get_output_metadata(&output).unwrap(),
        Some(create_remote_metadata(content))
    );
}

#[test]
fn save_output_metadata_remote_file_expired_remote_file_metadata_not_loaded() {
    let mut f = Fixture::new();
    f.use_store_output_metadata(true);
    let output = f.create_artifact("bin/dummy");
    let content = "content";
    let action = FakeAction::new(
        vec![output.clone()],
        Behavior::InjectFile(Mutex::new(VecDeque::from([create_remote_metadata_full(
            content,
            /* expiration_time= */ Some(1),
            /* resolved_path= */ None,
        )]))),
    );
    let metadata_handler = FakeInputMetadataHandler::default();

    f.run_action(&action);
    let token = f.get_token(&action, &metadata_handler, Some(&CheckTtl));

    assert!(!artifact_path(&output).exists());
    assert!(token.is_some());
    let entry = f.cache.get(&output.get_exec_path_string());
    assert!(entry.is_none());
}

#[test]
fn save_output_metadata_store_output_metadata_disabled_remote_file_metadata_not_loaded() {
    let mut f = Fixture::new();
    f.use_store_output_metadata(false);
    let output = f.create_artifact("bin/dummy");
    let content = "content";
    let action = FakeAction::new(
        vec![output.clone()],
        Behavior::InjectFile(Mutex::new(VecDeque::from([create_remote_metadata(
            content,
        )]))),
    );
    let metadata_handler = FakeInputMetadataHandler::default();

    f.run_action(&action);
    let token = f.get_token(&action, &metadata_handler, /* output_checker= */ None);

    assert!(!artifact_path(&output).exists());
    assert!(token.is_some());
    let entry = f.cache.get(&output.get_exec_path_string());
    assert!(entry.is_none());
}

fn do_save_output_metadata_local_metadata_is_same_as_remote_metadata_cached(
    has_resolved_path: bool,
) {
    let mut f = Fixture::new();
    f.use_store_output_metadata(true);
    let output = f.create_artifact("bin/dummy");
    let content = "content";
    let resolved_path = if has_resolved_path {
        Some(
            f.exec_root
                .get_relative_str("some/path")
                .as_fragment()
                .clone(),
        )
    } else {
        None
    };
    let action = FakeAction::new(
        vec![output.clone()],
        Behavior::InjectFile(Mutex::new(VecDeque::from([create_remote_metadata_full(
            content,
            None,
            resolved_path.clone(),
        )]))),
    );
    f.run_action(&action);
    f.assert_statistics(
        0,
        MissDetailsBuilder::new()
            .set(MissReason::NotCached, 1)
            .build(),
    );

    write_content_as_latin1(&artifact_path(&output), content);
    // Cached since local metadata is same as remote metadata
    f.run_action(&action);

    f.assert_statistics(
        1,
        MissDetailsBuilder::new()
            .set(MissReason::NotCached, 1)
            .build(),
    );
    let entry = f.cache.get(&output.get_exec_path_string());
    assert!(entry.is_some());
    assert_eq!(
        entry.unwrap().get_output_file(&*output),
        Some(&create_remote_metadata_full(content, None, resolved_path))
    );
}

#[test]
fn save_output_metadata_local_metadata_is_same_as_remote_metadata_cached() {
    // @TestParameter boolean hasResolvedPath
    do_save_output_metadata_local_metadata_is_same_as_remote_metadata_cached(false);
    do_save_output_metadata_local_metadata_is_same_as_remote_metadata_cached(true);
}

#[test]
fn save_output_metadata_local_metadata_is_different_from_remote_metadata_not_cached() {
    let mut f = Fixture::new();
    f.use_store_output_metadata(true);
    let output = f.create_artifact("bin/dummy");
    let content1 = "content1";
    let content2 = "content2";
    let action = FakeAction::new(
        vec![output.clone()],
        Behavior::InjectFile(Mutex::new(VecDeque::from([
            create_remote_metadata(content1),
            create_remote_metadata(content2),
        ]))),
    );
    f.run_action(&action);
    f.assert_statistics(
        0,
        MissDetailsBuilder::new()
            .set(MissReason::NotCached, 1)
            .build(),
    );

    write_content_as_latin1(&artifact_path(&output), content2);

    // Assert that if local file exists, shouldTrustArtifact is not called for the remote
    // metadata.
    let metadata_handler = FakeInputMetadataHandler::default();
    let output_checker = RecordingOutputChecker::new(false);
    let token = f.get_token(&action, &metadata_handler, Some(&output_checker));
    let calls = output_checker.calls.lock().unwrap().clone();
    assert_eq!(
        calls.iter().filter(|c| c.ends_with("bin/dummy")).count(),
        1,
        "{:?}",
        calls
    );
    // Not cached since local file changed
    f.run_action_with_token(
        &action,
        /* client_env= */ &IndexMap::new(),
        /* action_execution_salt= */ "",
        &metadata_handler,
        &metadata_handler,
        token.as_ref(),
    );

    f.assert_statistics(
        0,
        MissDetailsBuilder::new()
            .set(MissReason::NotCached, 1)
            .set(MissReason::DigestMismatch, 1)
            .build(),
    );
    let entry = f.cache.get(&output.get_exec_path_string());
    assert!(entry.is_some());
    assert_eq!(
        entry.unwrap().get_output_file(&*output),
        Some(&create_remote_metadata(content2))
    );
}

#[test]
fn save_output_metadata_trusted_remote_metadata_from_output_store_cached() {
    let mut f = Fixture::new();
    f.use_store_output_metadata(true);
    let output = f.create_artifact("bin/dummy");
    let content = "content";
    let metadata = create_remote_metadata(content);
    let action = FakeAction::new(
        vec![output.clone()],
        Behavior::InjectFile(Mutex::new(VecDeque::from([
            metadata.clone(),
            metadata.clone(),
        ]))),
    );
    f.run_action(&action);
    f.assert_statistics(
        0,
        MissDetailsBuilder::new()
            .set(MissReason::NotCached, 1)
            .build(),
    );

    let fake_output_metadata_store = FakeInputMetadataHandler::default();
    f.run_action_full(
        &action,
        /* client_env= */ &IndexMap::new(),
        /* action_execution_salt= */ "",
        &FakeInputMetadataHandler::default(),
        &fake_output_metadata_store,
        Some(&TrustAll),
    );
    f.assert_statistics(
        1,
        MissDetailsBuilder::new()
            .set(MissReason::NotCached, 1)
            .build(),
    );
    let file_metadata = fake_output_metadata_store.file_metadata.lock().unwrap();
    assert_eq!(file_metadata.len(), 1);
    assert_eq!(file_metadata.get(&output), Some(&Some(metadata.clone())));

    let entry = f.cache.get(&output.get_exec_path_string());
    assert!(entry.is_some());
    assert_eq!(entry.unwrap().get_output_file(&*output), Some(&metadata));
}

#[test]
fn save_output_metadata_untrusted_remote_metadata_from_output_store_not_cached() {
    let mut f = Fixture::new();
    f.use_store_output_metadata(true);
    let output = f.create_artifact("bin/dummy");
    let content = "content";
    let metadata = create_remote_metadata(content);
    let action = FakeAction::new(
        vec![output.clone()],
        Behavior::InjectFile(Mutex::new(VecDeque::from([
            metadata.clone(),
            metadata.clone(),
        ]))),
    );
    f.run_action(&action);
    f.assert_statistics(
        0,
        MissDetailsBuilder::new()
            .set(MissReason::NotCached, 1)
            .build(),
    );

    let output_checker = RecordingOutputChecker::new(false);

    f.run_action_full(
        &action,
        /* client_env= */ &IndexMap::new(),
        /* action_execution_salt= */ "",
        &FakeInputMetadataHandler::default(),
        &FakeInputMetadataHandler::default(),
        Some(&output_checker),
    );

    f.assert_statistics(
        0,
        MissDetailsBuilder::new()
            .set(MissReason::NotCached, 1)
            .set(MissReason::DigestMismatch, 1)
            .build(),
    );

    let entry = f.cache.get(&output.get_exec_path_string());
    assert!(entry.is_some());
    assert_eq!(entry.unwrap().get_output_file(&*output), Some(&metadata));
}

fn assert_tree_saved(
    f: &Fixture,
    output: &ActionInputRef,
    expected: SerializableTreeArtifactValue,
) {
    assert!(!artifact_path(output).exists());
    let entry = f.cache.get(&output.get_exec_path_string());
    assert!(entry.is_some());
    assert_eq!(entry.unwrap().get_output_tree(&**output), Some(&expected));
    f.assert_statistics(
        0,
        MissDetailsBuilder::new()
            .set(MissReason::NotCached, 1)
            .build(),
    );
}

#[test]
fn save_output_metadata_tree_metadata_remote_file_metadata_saved() {
    let mut f = Fixture::new();
    f.use_store_output_metadata(true);
    let output = f.create_tree_artifact("bin/dummy");
    let children = [
        ("file1", create_remote_metadata("content1")),
        ("file2", create_remote_metadata("content2")),
    ];
    let action = FakeAction::new(
        vec![output.clone()],
        Behavior::InjectTree(Mutex::new(VecDeque::from([create_tree_metadata(
            &output, &children, None, None,
        )]))),
    );

    f.run_action(&action);

    assert_tree_saved(
        &f,
        &output,
        SerializableTreeArtifactValue::new(children_map(&children), None, None),
    );
}

#[test]
fn save_output_metadata_tree_metadata_remote_archived_artifact_saved() {
    let mut f = Fixture::new();
    f.use_store_output_metadata(true);
    let output = f.create_tree_artifact("bin/dummy");
    let action = FakeAction::new(
        vec![output.clone()],
        Behavior::InjectTree(Mutex::new(VecDeque::from([create_tree_metadata(
            &output,
            &[],
            Some(create_remote_metadata("content")),
            None,
        )]))),
    );

    f.run_action(&action);

    assert_tree_saved(
        &f,
        &output,
        SerializableTreeArtifactValue::new(
            IndexMap::new(),
            Some(create_remote_metadata("content")),
            None,
        ),
    );
}

#[test]
fn save_output_metadata_tree_metadata_resolved_path_saved() {
    let mut f = Fixture::new();
    f.use_store_output_metadata(true);
    let output = f.create_tree_artifact("bin/dummy");
    let resolved_path = f
        .exec_root
        .get_relative_str("some/path")
        .as_fragment()
        .clone();
    let action = FakeAction::new(
        vec![output.clone()],
        Behavior::InjectTree(Mutex::new(VecDeque::from([create_tree_metadata(
            &output,
            &[],
            None,
            Some(resolved_path.clone()),
        )]))),
    );

    f.run_action(&action);

    assert_tree_saved(
        &f,
        &output,
        SerializableTreeArtifactValue::new(IndexMap::new(), None, Some(resolved_path)),
    );
}

#[test]
fn save_output_metadata_tree_metadata_local_file_metadata_not_saved() {
    let mut f = Fixture::new();
    f.use_store_output_metadata(true);
    let output = f.create_tree_artifact("bin/dummy");
    let file2 = f.exec_root.get_child("file2");
    write_content_as_latin1(&file2, "");
    let children = [
        ("file1", create_remote_metadata("content1")),
        (
            "file2",
            FileArtifactValue::create_for_testing(&file2).unwrap(),
        ),
    ];
    file2.delete().unwrap();
    let action = FakeAction::new(
        vec![output.clone()],
        Behavior::InjectTree(Mutex::new(VecDeque::from([create_tree_metadata(
            &output, &children, None, None,
        )]))),
    );

    f.run_action(&action);

    assert_tree_saved(
        &f,
        &output,
        SerializableTreeArtifactValue::new(
            IndexMap::from([("file1".to_string(), create_remote_metadata("content1"))]),
            None,
            None,
        ),
    );
}

#[test]
fn save_output_metadata_tree_metadata_local_archived_artifact_not_saved() {
    let mut f = Fixture::new();
    f.use_store_output_metadata(true);
    let output = f.create_tree_artifact("bin/dummy");
    let archive = f.exec_root.get_child("archive");
    write_content_as_latin1(&archive, "");
    let action = FakeAction::new(
        vec![output.clone()],
        Behavior::InjectTree(Mutex::new(VecDeque::from([create_tree_metadata(
            &output,
            &[],
            Some(FileArtifactValue::create_for_testing(&archive).unwrap()),
            None,
        )]))),
    );
    archive.delete().unwrap();

    f.run_action(&action);

    assert!(!artifact_path(&output).exists());
    let entry = f.cache.get(&output.get_exec_path_string()).unwrap();
    assert_eq!(
        entry
            .get_output_tree(&*output)
            .unwrap()
            .archived_file_value(),
        None
    );
    f.assert_statistics(
        0,
        MissDetailsBuilder::new()
            .set(MissReason::NotCached, 1)
            .build(),
    );
}

#[test]
#[ignore = "needs TreeArtifactValue.Builder (TreeArtifactFactory) and visitTree ports"]
fn save_output_metadata_empty_tree_metadata_saved() {}

#[test]
#[ignore = "needs TreeArtifactValue.Builder (TreeArtifactFactory) and visitTree ports"]
fn save_output_metadata_tree_metadata_remote_file_metadata_loaded() {}

#[test]
#[ignore = "needs TreeArtifactValue.Builder (TreeArtifactFactory) and visitTree ports"]
fn save_output_metadata_tree_metadata_local_file_metadata_loaded() {}

#[test]
#[ignore = "needs TreeArtifactValue.Builder (TreeArtifactFactory) and visitTree ports"]
fn save_output_metadata_tree_metadata_local_archived_artifact_loaded() {}

#[test]
#[ignore = "needs TreeArtifactValue.Builder (TreeArtifactFactory) and visitTree ports"]
fn save_output_metadata_tree_file_expired_tree_metadata_not_loaded() {}

#[test]
#[ignore = "needs TreeArtifactValue.Builder (TreeArtifactFactory) and visitTree ports"]
fn save_output_metadata_archived_representation_expired_tree_metadata_not_loaded() {}

#[test]
#[ignore = "needs TreeArtifactValue.Builder (TreeArtifactFactory) and visitTree ports"]
fn save_output_metadata_toggle_archived_tree_artifacts_not_loaded() {}

#[test]
#[ignore = "needs TreeArtifactValue.Builder (TreeArtifactFactory) and visitTree ports"]
fn save_output_metadata_tree_metadata_with_same_local_file_metadata_cached() {}

#[test]
#[ignore = "needs TreeArtifactValue.Builder (TreeArtifactFactory) and visitTree ports"]
fn save_output_metadata_tree_metadata_with_same_local_archived_artifact_cached() {}

#[test]
#[ignore = "needs TreeArtifactValue.Builder (TreeArtifactFactory) and visitTree ports"]
fn save_output_metadata_trusted_remote_tree_metadata_from_output_store_cached() {}

#[test]
#[ignore = "needs TreeArtifactValue.Builder (TreeArtifactFactory) and visitTree ports"]
fn save_output_metadata_empty_tree_metadata_from_output_store_cached() {}

#[test]
#[ignore = "needs TreeArtifactValue.Builder (TreeArtifactFactory) and visitTree ports"]
fn save_output_metadata_untrusted_remote_tree_metadata_from_output_store_not_cached() {}

/// Shared body of the proxy-metadata tests: run once, then re-run with the local metadata hidden
/// from the output store (emulating an action file system).
fn run_with_hidden_local_metadata(
    f: &Fixture,
    action: &FakeAction,
    output: &ActionInputRef,
) -> FakeInputMetadataHandler {
    let fake_output_metadata_store = FakeInputMetadataHandler::default();

    // Hide the local metadata from the OutputMetadataStore, emulating an action file system.
    fake_output_metadata_store
        .file_metadata
        .lock()
        .unwrap()
        .insert(output.clone(), None);

    f.run_action_full(
        action,
        /* client_env= */ &IndexMap::new(),
        /* action_execution_salt= */ "",
        &FakeInputMetadataHandler::default(),
        &fake_output_metadata_store,
        Some(&TrustAll),
    );
    fake_output_metadata_store
}

fn proxy_action(f: &mut Fixture) -> (ActionInputRef, FileArtifactValue, FakeAction) {
    f.use_store_output_metadata(true);
    let output = f.create_artifact("bin/dummy");
    let content = "content";
    let metadata = create_proxy_metadata(&output, content);
    let action = FakeAction::new(
        vec![output.clone()],
        Behavior::InjectFile(Mutex::new(VecDeque::from([
            metadata.clone(),
            metadata.clone(),
        ]))),
    );
    f.run_action(&action);
    f.assert_statistics(
        0,
        MissDetailsBuilder::new()
            .set(MissReason::NotCached, 1)
            .build(),
    );
    (output, metadata, action)
}

fn assert_proxy_outputs(f: &Fixture, output: &ActionInputRef) {
    let entry = f.cache.get(&output.get_exec_path_string());
    assert!(entry.is_some());
    assert_eq!(
        entry.unwrap().get_proxy_outputs(),
        &[output.get_exec_path_string()][..]
    );
}

#[test]
fn save_output_metadata_same_proxy_metadata_cached_and_injected() {
    let mut f = Fixture::new();
    let (output, metadata, action) = proxy_action(&mut f);

    f.proxy_metadata_factory.set(Ok(Some(metadata.clone())));
    let fake_output_metadata_store = run_with_hidden_local_metadata(&f, &action, &output);

    f.assert_statistics(
        1,
        MissDetailsBuilder::new()
            .set(MissReason::NotCached, 1)
            .build(),
    );
    let file_metadata = fake_output_metadata_store.file_metadata.lock().unwrap();
    assert_eq!(file_metadata.len(), 1);
    assert_eq!(file_metadata.get(&output), Some(&Some(metadata)));

    assert_proxy_outputs(&f, &output);
}

#[test]
fn save_output_metadata_different_proxy_metadata_not_cached() {
    let mut f = Fixture::new();
    let (output, _metadata, action) = proxy_action(&mut f);

    f.proxy_metadata_factory
        .set(Ok(Some(create_proxy_metadata(&output, "changed"))));
    run_with_hidden_local_metadata(&f, &action, &output);

    f.assert_statistics(
        0,
        MissDetailsBuilder::new()
            .set(MissReason::NotCached, 1)
            .set(MissReason::DigestMismatch, 1)
            .build(),
    );

    assert_proxy_outputs(&f, &output);
}

#[test]
fn save_output_metadata_null_proxy_metadata_not_cached() {
    let mut f = Fixture::new();
    let (output, _metadata, action) = proxy_action(&mut f);

    f.proxy_metadata_factory.set(Ok(None));
    run_with_hidden_local_metadata(&f, &action, &output);

    f.assert_statistics(
        0,
        MissDetailsBuilder::new()
            .set(MissReason::NotCached, 1)
            .set(MissReason::DigestMismatch, 1)
            .build(),
    );

    assert_proxy_outputs(&f, &output);
}

#[test]
fn save_output_metadata_io_exception_proxy_metadata_not_cached() {
    let mut f = Fixture::new();
    let (output, _metadata, action) = proxy_action(&mut f);

    f.proxy_metadata_factory.set(Err("IO error".to_string()));
    run_with_hidden_local_metadata(&f, &action, &output);

    f.assert_statistics(
        0,
        MissDetailsBuilder::new()
            .set(MissReason::NotCached, 1)
            .set(MissReason::DigestMismatch, 1)
            .build(),
    );

    assert_proxy_outputs(&f, &output);
}
