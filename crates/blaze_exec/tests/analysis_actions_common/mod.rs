//! Shared support for the action-execution tests: a test `ActionExecutionContext` (Java's
//! `ActionExecutionContext` built over `TestExecutorBuilder` in `BuildViewTestCase`s), artifact
//! factories (`ActionsTestUtil`) and an `InputMetadataProvider` (`ActionInputMap`).

#![allow(dead_code)]

use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::fmt;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::io;
use std::sync::Arc;

use blaze_actions::actions::action_lookup::ActionLookupKey;
use blaze_actions::actions::platform_info::empty_platform_info;
use blaze_actions::{
    ActionLookupKeyRef, ActionOwner, Artifact, ArtifactOwner, ArtifactRoot, RootType,
    SpecialArtifactType,
};
use blaze_cmdline::Label;
use blaze_events::location::Location;
use blaze_events::{Clock, ExtendedEventHandler, JavaClock, StoredEventHandler};
use blaze_exec::actions::action_execution::ActionContextRef;
use blaze_exec::actions::{
    ActionExecutionMetadata, ActionInputRef, FileArtifactValue, FilesetOutputTree,
    InputMetadataProvider, RunfilesArtifactValue, Spawn, TreeArtifactValue,
};
use blaze_exec::exec::action_execution_context::{ActionContextRegistry, ActionExecutionContext};
use blaze_exec::exec::spawn_runner::ProgressStatus;
use blaze_exec::exec::spawn_strategy_registry::SpawnStrategyRegistry;
use blaze_exec::util::FileOutErr;
use blaze_vfs::{DigestHashFunction, FileSystemRef, Path, PathFragment, Root, UnixFileSystem};
use indexmap::IndexMap;
use parking_lot::Mutex;

/// `ActionsTestUtil.NULL_ARTIFACT_OWNER`.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct NullArtifactOwner;

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

/// `ActionsTestUtil.NULL_ACTION_OWNER`.
pub fn null_action_owner() -> ActionOwner {
    ActionOwner::create_dummy(
        Some(Label::parse_canonical("//null/action:owner").unwrap()),
        Location::new("dummy-file", 0, 0),
        "dummy-kind",
        "dummy-configuration-mnemonic",
        "dummy-configuration",
        None,
        false,
        Some(empty_platform_info()),
        Vec::new(),
        IndexMap::new(),
    )
}

/// A scratch workspace and execroot in a temporary directory (`Scratch` + `BlazeDirectories`).
pub struct TestDirs {
    pub tmp: tempfile::TempDir,
    pub fs: FileSystemRef,
    pub workspace: Path,
    pub exec_root: Path,
    pub source_root: ArtifactRoot,
    pub bin_root: ArtifactRoot,
}

impl TestDirs {
    pub fn new() -> TestDirs {
        let tmp = tempfile::tempdir().unwrap();
        let fs = UnixFileSystem::create(DigestHashFunction::Sha256);
        let base = Path::create(PathFragment::create(tmp.path().to_str().unwrap()), &fs);
        let workspace = base.get_relative_str("workspace");
        let exec_root = base.get_relative_str("execroot/_main");
        workspace.create_directory_and_parents().unwrap();
        exec_root.create_directory_and_parents().unwrap();
        let source_root = ArtifactRoot::as_source_root(Root::from_path(workspace.clone()));
        let bin_root = ArtifactRoot::as_derived_root(
            &exec_root,
            RootType::OUTPUT,
            &["bazel-out", "k8-fastbuild", "bin"],
        );
        TestDirs {
            tmp,
            fs,
            workspace,
            exec_root,
            source_root,
            bin_root,
        }
    }

    /// `getSourceArtifact(path)`.
    pub fn source_artifact(&self, path: &str) -> Artifact {
        Artifact::new_source_artifact(
            self.source_root.clone(),
            PathFragment::create(path),
            ArtifactOwner::NullOwner,
        )
    }

    /// `getBinArtifactWithNoOwner(rootRelativePath)`.
    pub fn bin_artifact(&self, root_relative_path: &str) -> Artifact {
        Artifact::create_derived_artifact(
            self.bin_root.clone(),
            self.bin_root
                .get_exec_path()
                .get_relative(&PathFragment::create(root_relative_path)),
            ActionLookupKeyRef::new(NullArtifactOwner),
        )
    }

    /// A special (tree, unresolved symlink, ...) artifact under the bin root.
    pub fn special_artifact(&self, root_relative_path: &str, t: SpecialArtifactType) -> Artifact {
        Artifact::create_special_artifact(
            self.bin_root.clone(),
            self.bin_root
                .get_exec_path()
                .get_relative(&PathFragment::create(root_relative_path)),
            ActionLookupKeyRef::new(NullArtifactOwner),
            t,
        )
    }

    pub fn context(&self) -> TestContext {
        TestContext::new(self.exec_root.clone())
    }
}

/// `ActionInputMap` for tests: metadata by exec path.
#[derive(Default)]
pub struct TestMetadataProvider {
    pub metadata: Mutex<HashMap<String, FileArtifactValue>>,
    pub trees: Mutex<HashMap<String, Arc<TreeArtifactValue>>>,
    pub filesets: Mutex<HashMap<String, Arc<FilesetOutputTree>>>,
    /// `getInput(execPath)`.
    pub inputs: Mutex<HashMap<String, ActionInputRef>>,
}

impl TestMetadataProvider {
    /// `inputMap.put(input, FileArtifactValue.createForTesting(input))` /
    /// `putTreeArtifact(input, TreeArtifactValue.empty())`.
    pub fn put_for_testing(&self, artifact: &Artifact) {
        if artifact.is_tree_artifact() {
            self.trees.lock().insert(
                artifact.get_exec_path_string().to_string(),
                Arc::new(TreeArtifactValue::default()),
            );
            self.metadata.lock().insert(
                artifact.get_exec_path_string().to_string(),
                FileArtifactValue::Directory { mtime: 0 },
            );
            return;
        }
        let path = artifact.get_path();
        let value = if path.is_directory() {
            FileArtifactValue::Directory { mtime: 0 }
        } else {
            FileArtifactValue::RegularFile {
                digest: None,
                proxy: None,
                size: 0,
            }
        };
        self.metadata
            .lock()
            .insert(artifact.get_exec_path_string().to_string(), value);
    }
}

impl InputMetadataProvider for TestMetadataProvider {
    fn get_input_metadata(&self, input: &ActionInputRef) -> io::Result<Option<FileArtifactValue>> {
        Ok(self
            .metadata
            .lock()
            .get(input.0.get_exec_path().get_path_string())
            .cloned())
    }
    fn get_tree_metadata(&self, input: &ActionInputRef) -> Option<Arc<TreeArtifactValue>> {
        self.trees
            .lock()
            .get(input.0.get_exec_path().get_path_string())
            .cloned()
    }
    fn get_fileset(&self, input: &ActionInputRef) -> Option<Arc<FilesetOutputTree>> {
        self.filesets
            .lock()
            .get(input.0.get_exec_path().get_path_string())
            .cloned()
    }
    fn get_runfiles_metadata(&self, _input: &ActionInputRef) -> Option<Arc<RunfilesArtifactValue>> {
        None
    }
    fn get_input(&self, exec_path: &PathFragment) -> Option<ActionInputRef> {
        self.inputs.lock().get(exec_path.get_path_string()).cloned()
    }
}

/// The test `ActionExecutionContext`.
pub struct TestContext {
    pub exec_root: Path,
    pub metadata: TestMetadataProvider,
    pub out_err: FileOutErr,
    pub client_env: IndexMap<String, String>,
    pub events: StoredEventHandler,
    pub clock: JavaClock,
    pub contexts: HashMap<TypeId, ActionContextRef>,
    pub spawn_strategy_registry: Option<Arc<SpawnStrategyRegistry>>,
    pub rich_artifact_data: Mutex<Option<Arc<RunfilesArtifactValue>>>,
    pub progress: Mutex<Vec<String>>,
    /// `getActionFileSystem()`: the `RemoteActionFileSystem` of build without the bytes.
    pub action_file_system:
        Option<Arc<blaze_exec::exec::remote_action_file_system::RemoteActionFileSystem>>,
}

impl TestContext {
    /// A context with the default action contexts of `blaze_exec::analysis::actions` registered
    /// (`FileWriteStrategy`, `LocalTemplateExpansionStrategy`, `SymlinkTreeStrategy`,
    /// `SpawnStrategyResolver`).
    pub fn new(exec_root: Path) -> TestContext {
        blaze_exec::analysis::actions::register();
        let out_err = FileOutErr::new(
            exec_root.get_relative_str("stdout"),
            exec_root.get_relative_str("stderr"),
        );
        let contexts = blaze_exec::analysis::actions::default_action_contexts("_main")
            .into_iter()
            .collect();
        TestContext {
            exec_root,
            metadata: TestMetadataProvider::default(),
            out_err,
            client_env: IndexMap::new(),
            events: StoredEventHandler::new(),
            clock: JavaClock,
            contexts,
            spawn_strategy_registry: None,
            rich_artifact_data: Mutex::new(None),
            progress: Mutex::new(Vec::new()),
            action_file_system: None,
        }
    }

    pub fn register(&mut self, entry: (TypeId, ActionContextRef)) {
        self.contexts.insert(entry.0, entry.1);
    }
}

impl ActionContextRegistry for TestContext {
    fn get_spawn_strategy_registry(&self) -> Option<Arc<SpawnStrategyRegistry>> {
        self.spawn_strategy_registry.clone()
    }
    fn get_context_any(&self, type_id: TypeId) -> Option<ActionContextRef> {
        self.contexts.get(&type_id).cloned()
    }
}

impl ActionExecutionContext for TestContext {
    fn get_exec_root(&self) -> &Path {
        &self.exec_root
    }
    fn get_input_metadata_provider(&self) -> &dyn InputMetadataProvider {
        &self.metadata
    }
    fn get_file_out_err(&self) -> &FileOutErr {
        &self.out_err
    }
    fn get_client_env(&self) -> &IndexMap<String, String> {
        &self.client_env
    }
    fn get_event_handler(&self) -> &dyn ExtendedEventHandler {
        &self.events
    }
    fn get_clock(&self) -> &dyn Clock {
        &self.clock
    }
    fn maybe_report_subcommand(&self, _spawn: &dyn Spawn, _spawn_runner: Option<&str>) {}
    fn post_progress(&self, action: &dyn ActionExecutionMetadata, progress: &ProgressStatus) {
        self.progress
            .lock()
            .push(format!("{:?} {}", progress, action.get_mnemonic()));
    }
    fn set_rich_artifact_data(&self, data: Arc<RunfilesArtifactValue>) {
        *self.rich_artifact_data.lock() = Some(data);
    }
    fn get_remote_action_file_system(
        &self,
    ) -> Option<Arc<blaze_exec::exec::remote_action_file_system::RemoteActionFileSystem>> {
        self.action_file_system.clone()
    }
}

/// `FileSystemUtils.readContent(path)`.
pub fn read(path: &Path) -> Vec<u8> {
    std::fs::read(path.get_path_string()).unwrap()
}

/// `FileSystemUtils.writeContent(path, UTF_8, content)` (creating parent directories).
pub fn write(path: &Path, content: &str) {
    path.get_parent_directory()
        .unwrap()
        .create_directory_and_parents()
        .unwrap();
    std::fs::write(path.get_path_string(), content).unwrap();
}
