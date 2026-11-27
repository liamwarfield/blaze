//! Port of `src/test/java/com/google/devtools/build/lib/exec/SpawnLogContextTestBase.java` (and
#![allow(clippy::too_many_arguments)]
//! the test utilities it uses: `SpawnBuilder`, `FakeActionInputFileCache`, the
//! `ActionsTestUtil` artifact factories). Shared by `spawn_log_expanded_test.rs` and
//! `spawn_log_compact_test.rs`, which supply the [`SpawnLogContextTest`] hooks.
//!
//! Java runs every test once per `@TestParameter` combination (including the class-level
//! `siblingRepositoryLayout`); the Rust tests loop over the combinations, each with a fresh
//! [`Fixture`]. Java's `InMemoryFileSystem` is a real file system in a temporary directory.
#![allow(dead_code)]

use std::any::Any;
use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use blaze_cmdline::Label;
use blaze_collect::{NestedSet, NestedSetBuilder};
use blaze_events::StoredEventHandler;
use blaze_exec::actions::{
    ActionInput, ActionInputRef, Artifact, FakeOwner, FileArtifactValue, FilesetOutputTree,
    HierarchicalPath, InputMapping, InputMetadataProvider, PathMapper, ResourceSet,
    RunfilesArtifactValue, SimpleSpawn, Spawn, SpawnMetrics, SpawnResult, SpawnResultBuilder,
    Status, TreeArtifactValue,
};
use blaze_exec::exec::spawn_log_context::{SpawnLogContext, millis_to_proto};
use blaze_protos::failure_details::{Crash, FailureDetail, failure_detail};
use blaze_protos::google::protobuf::Timestamp;
use blaze_protos::tools::protos::{
    self, Digest, EnvironmentVariable, File, Platform, SpawnExec, platform,
};
use blaze_vfs::{
    DigestHashFunction, Dirent, DirentType, FileStatus, FileSystem, FileSystemExt, FileSystemRef,
    Path, PathFragment, Symlinks, UnixFileSystem,
};
use indexmap::IndexMap;

pub const PRODUCT_NAME: &str = "bazel";
pub const WORKSPACE_NAME: &str = "_main";

/// The per-format hooks (`createSpawnLogContext`, `closeAndAssertLog`).
pub trait SpawnLogContextTest {
    /// `createSpawnLogContext(platformProperties)`.
    fn create_spawn_log_context_with(
        &self,
        f: &Fixture,
        platform_properties: &[(&str, &str)],
    ) -> Box<dyn SpawnLogContext>;

    /// `closeAndAssertLog(context, expected...)`.
    fn close_and_assert_log(
        &self,
        f: &Fixture,
        context: Box<dyn SpawnLogContext>,
        expected: Vec<SpawnExec>,
    );
}

/// The fields and `@Before setup()` of the Java base class.
pub struct Fixture {
    _tmp: tempfile::TempDir,
    pub root: Path,
    pub digest_hash_function: DigestHashFunction,
    pub fs: FileSystemRef,
    pub output_base: Path,
    pub exec_root: Path,
    pub sibling_repository_layout: bool,
    pub root_dir: TestRoot,
    pub output_dir: TestRoot,
    pub stored_event_handler: Arc<StoredEventHandler>,
}

impl Fixture {
    pub fn new(sibling_repository_layout: bool) -> Fixture {
        let tmp = tempfile::tempdir().unwrap();
        let digest_hash_function = DigestHashFunction::Sha256;
        let fs = UnixFileSystem::create(digest_hash_function);
        let root = fs.get_path(std::fs::canonicalize(tmp.path()).unwrap().to_str().unwrap());
        let output_base = root.get_relative_str("home/user/bazel/output_base");
        let exec_root = output_base.get_relative_str(&format!("execroot/{WORKSPACE_NAME}"));
        // `configuration.getBinDirectory(RepositoryName.MAIN)`.
        let output_dir = TestRoot {
            exec_path: PathFragment::create(&format!("{PRODUCT_NAME}-out/k8-fastbuild/bin")),
            is_source: false,
        };
        let root_dir = TestRoot {
            exec_path: PathFragment::empty_fragment(),
            is_source: true,
        };
        Fixture {
            _tmp: tmp,
            root,
            digest_hash_function,
            fs,
            output_base,
            exec_root,
            sibling_repository_layout,
            root_dir,
            output_dir,
            stored_event_handler: Arc::new(StoredEventHandler::new()),
        }
    }

    /// A path in the scratch directory (Java's `fs.getPath("/log")`).
    pub fn scratch_path(&self, name: &str) -> Path {
        self.root.get_relative_str(name)
    }

    /// `Artifact.getPath()`.
    pub fn path_of(&self, input: &ActionInputRef) -> Path {
        self.exec_root.get_relative(input.get_exec_path())
    }

    /// `ArtifactRoot.getRoot().asPath()`.
    pub fn root_path(&self, root: &TestRoot) -> Path {
        self.exec_root.get_relative(&root.exec_path)
    }

    pub fn create_spawn_log_context(
        &self,
        t: &dyn SpawnLogContextTest,
    ) -> Box<dyn SpawnLogContext> {
        t.create_spawn_log_context_with(self, &[])
    }

    /// `getDigest(content)`.
    pub fn get_digest(&self, content: &str) -> Digest {
        Digest {
            hash: hex::encode(<sha2::Sha256 as sha2::Digest>::digest(content.as_bytes())),
            size_bytes: content.len() as i64,
            hash_function_name: self.digest_hash_function.to_string(),
        }
    }

    /// `OutputsMode.getActionFileSystem(fs)`.
    pub fn get_action_file_system(&self, outputs_mode: OutputsMode) -> FileSystemRef {
        match outputs_mode {
            OutputsMode::WithBytes => Arc::clone(&self.fs),
            OutputsMode::WithoutBytes => Arc::new(FakeActionFileSystem {
                delegate: Arc::clone(&self.fs),
            }),
        }
    }

    /// `createTreeArtifactValue(tree)`.
    pub fn create_tree_artifact_value(&self, tree: &ActionInputRef) -> TreeArtifactValue {
        assert!(tree.as_artifact().unwrap().is_tree_artifact());
        let mut children: Vec<(ActionInputRef, FileArtifactValue)> = Vec::new();
        // `TreeArtifactValue.visitTree`: every non-directory descendant.
        fn visit(dir: &Path, rel: &PathFragment, out: &mut Vec<PathFragment>) {
            for dirent in dir.readdir(Symlinks::Nofollow).unwrap() {
                let child_rel = rel.get_child(dirent.get_name());
                if dirent.get_type() == DirentType::Directory {
                    visit(&dir.get_child(dirent.get_name()), &child_rel, out);
                } else {
                    out.push(child_rel);
                }
            }
        }
        let mut rels = Vec::new();
        let tree_path = self.path_of(tree);
        if tree_path.is_directory() {
            visit(&tree_path, &PathFragment::empty_fragment(), &mut rels);
        }
        for rel in rels {
            let child = create_tree_file_artifact(tree, &rel);
            let value = FileArtifactValue::create_for_testing(&self.path_of(&child)).unwrap();
            children.push((child, value));
        }
        // `ImmutableSortedMap` of `TreeFileArtifact`s: by exec path.
        children.sort_by(|a, b| a.0.get_exec_path().cmp(b.0.get_exec_path()));
        TreeArtifactValue {
            child_values: children,
            ..Default::default()
        }
    }

    /// `createInputMetadataProvider(artifacts...)`.
    pub fn create_input_metadata_provider(
        &self,
        artifacts: &[&ActionInputRef],
    ) -> FakeActionInputFileCache {
        let mut builder = FakeActionInputFileCache::default();
        for artifact in artifacts {
            let a = artifact.as_artifact().unwrap();
            if a.is_tree_artifact() {
                // Emulate ActionInputMap: add both tree and children.
                let tree_metadata = self.create_tree_artifact_value(artifact);
                // PORT: the seam's TreeArtifactValue has no aggregate metadata here; the tree's
                // own metadata is never read by the execution log for a tree artifact.
                if let Some(m) = &tree_metadata.metadata {
                    builder.put((*artifact).clone(), m.clone());
                }
                for (child, value) in tree_metadata.child_values {
                    builder.put(child, value);
                }
            } else if artifact.is_symlink() {
                let path = self.path_of(artifact);
                let target = path.read_symbolic_link().unwrap();
                builder.put(
                    (*artifact).clone(),
                    FileArtifactValue::UnresolvedSymlink {
                        target: target.get_path_string().to_string(),
                        digest: <sha2::Sha256 as sha2::Digest>::digest(
                            target.get_path_string().as_bytes(),
                        )
                        .to_vec(),
                    },
                );
            } else if a.is_runfiles_tree() {
                panic!("runfiles trees are not supported by this port of the test base");
            } else {
                builder.put(
                    (*artifact).clone(),
                    FileArtifactValue::create_for_testing(&self.path_of(artifact)).unwrap(),
                );
            }
        }
        builder
    }

    /// `createInputMap(actionInputs...)`.
    pub fn create_input_map(&self, action_inputs: &[&ActionInputRef]) -> Arc<InputMapping> {
        let mut builder = InputMapping::new();
        for action_input in action_inputs {
            if action_input
                .as_artifact()
                .is_some_and(|a| a.is_tree_artifact())
            {
                // Emulate SpawnInputExpander: expand to children, preserve if empty.
                let tree_metadata = self.create_tree_artifact_value(action_input);
                if tree_metadata.child_values.is_empty() {
                    builder.insert(
                        HierarchicalPath(action_input.get_exec_path().clone()),
                        (*action_input).clone(),
                    );
                } else {
                    for child in tree_metadata.get_children() {
                        builder.insert(HierarchicalPath(child.get_exec_path().clone()), child);
                    }
                }
            } else {
                builder.insert(
                    HierarchicalPath(action_input.get_exec_path().clone()),
                    (*action_input).clone(),
                );
            }
        }
        Arc::new(builder)
    }

    /// `context.logSpawn(spawn, inputMetadataProvider, inputMap, fileSystem, timeout, result)`.
    pub fn log_spawn(
        &self,
        context: &dyn SpawnLogContext,
        spawn: &dyn Spawn,
        input_metadata_provider: &dyn InputMetadataProvider,
        input_map: Arc<InputMapping>,
        file_system: &FileSystemRef,
        timeout: Duration,
        result: &SpawnResult,
    ) {
        context
            .log_spawn(
                spawn,
                input_metadata_provider,
                &|| Arc::clone(&input_map),
                file_system,
                timeout.into(),
                result,
            )
            .unwrap();
    }
}

/// An `ArtifactRoot`: its exec path and whether it is a source root.
#[derive(Clone, Debug)]
pub struct TestRoot {
    pub exec_path: PathFragment,
    pub is_source: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArtifactKind {
    Source,
    Derived,
    Tree,
    TreeFile,
    UnresolvedSymlink,
    Fileset,
    RunfilesTree,
}

/// A test `Artifact` (source, derived or special).
#[derive(Debug)]
pub struct TestArtifact {
    exec_path: PathFragment,
    root_exec_path: PathFragment,
    root_relative_path: PathFragment,
    kind: ArtifactKind,
    parent: Option<ActionInputRef>,
    parent_relative_path: Option<PathFragment>,
}

impl ActionInput for TestArtifact {
    fn get_exec_path(&self) -> &PathFragment {
        &self.exec_path
    }
    fn is_directory(&self) -> bool {
        matches!(self.kind, ArtifactKind::Tree | ArtifactKind::Fileset)
    }
    fn is_symlink(&self) -> bool {
        self.kind == ArtifactKind::UnresolvedSymlink
    }
    fn as_artifact(&self) -> Option<&dyn Artifact> {
        Some(self)
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl Artifact for TestArtifact {
    fn is_source_artifact(&self) -> bool {
        self.kind == ArtifactKind::Source
    }
    fn is_tree_artifact(&self) -> bool {
        self.kind == ArtifactKind::Tree
    }
    fn is_runfiles_tree(&self) -> bool {
        self.kind == ArtifactKind::RunfilesTree
    }
    fn is_fileset(&self) -> bool {
        self.kind == ArtifactKind::Fileset
    }
    fn is_child_of_declared_directory(&self) -> bool {
        self.kind == ArtifactKind::TreeFile
    }
    fn get_parent(&self) -> Option<ActionInputRef> {
        self.parent.clone()
    }
    fn get_parent_relative_path(&self) -> Option<PathFragment> {
        self.parent_relative_path.clone()
    }
    fn get_root_relative_path(&self) -> PathFragment {
        self.root_relative_path.clone()
    }
    fn get_root_exec_path(&self) -> PathFragment {
        self.root_exec_path.clone()
    }
}

fn create_special(root: &TestRoot, path: &str, kind: ArtifactKind) -> ActionInputRef {
    let root_relative_path = PathFragment::create(path);
    ActionInputRef::new(TestArtifact {
        exec_path: root.exec_path.get_relative(&root_relative_path),
        root_exec_path: root.exec_path.clone(),
        root_relative_path,
        kind,
        parent: None,
        parent_relative_path: None,
    })
}

/// `ActionsTestUtil.createArtifact(root, path)`.
pub fn create_artifact(root: &TestRoot, path: &str) -> ActionInputRef {
    create_special(
        root,
        path,
        if root.is_source {
            ArtifactKind::Source
        } else {
            ArtifactKind::Derived
        },
    )
}

/// `ActionsTestUtil.createTreeArtifactWithGeneratingAction(root, path)`.
pub fn create_tree_artifact_with_generating_action(root: &TestRoot, path: &str) -> ActionInputRef {
    create_special(root, path, ArtifactKind::Tree)
}

/// `ActionsTestUtil.createUnresolvedSymlinkArtifact(root, path)`.
pub fn create_unresolved_symlink_artifact(root: &TestRoot, path: &str) -> ActionInputRef {
    create_special(root, path, ArtifactKind::UnresolvedSymlink)
}

/// `SpecialArtifact.create(root, root.getExecPath().getRelative(path), owner, FILESET)`.
pub fn create_fileset_artifact(root: &TestRoot, path: &str) -> ActionInputRef {
    create_special(root, path, ArtifactKind::Fileset)
}

/// `TreeFileArtifact.createTreeOutput(tree, parentRelativePath)`.
pub fn create_tree_file_artifact(
    tree: &ActionInputRef,
    parent_relative_path: &PathFragment,
) -> ActionInputRef {
    let a = tree.as_artifact().unwrap();
    ActionInputRef::new(TestArtifact {
        exec_path: tree.get_exec_path().get_relative(parent_relative_path),
        root_exec_path: a.get_root_exec_path(),
        root_relative_path: a
            .get_root_relative_path()
            .get_relative(parent_relative_path),
        kind: ArtifactKind::TreeFile,
        parent: Some(tree.clone()),
        parent_relative_path: Some(parent_relative_path.clone()),
    })
}

/// `new ParamFileActionInput(PathFragment.create(path), args, ParameterFileType.UNQUOTED)`.
///
pub fn create_param_file_action_input(path: &str, args: &[&str]) -> ActionInputRef {
    ActionInputRef::new(
        blaze_actions::actions::command_lines::ParamFileActionInput::new(
            PathFragment::create(path),
            args.iter().map(|arg| arg.to_string()).collect(),
            blaze_actions::actions::parameter_file::ParameterFileType::UNQUOTED,
        ),
    )
}

/// `FakeActionInputFileCache`.
#[derive(Default)]
pub struct FakeActionInputFileCache {
    inputs: HashMap<ActionInputRef, FileArtifactValue>,
    tree_artifacts: HashMap<ActionInputRef, Arc<TreeArtifactValue>>,
    runfiles_inputs: HashMap<ActionInputRef, Arc<RunfilesArtifactValue>>,
}

impl FakeActionInputFileCache {
    pub fn put(&mut self, artifact: ActionInputRef, metadata: FileArtifactValue) {
        self.inputs.insert(artifact, metadata);
    }

    pub fn put_tree_artifact(&mut self, action_input: ActionInputRef, value: TreeArtifactValue) {
        self.tree_artifacts.insert(action_input, Arc::new(value));
    }
}

impl InputMetadataProvider for FakeActionInputFileCache {
    fn get_input_metadata(&self, input: &ActionInputRef) -> io::Result<Option<FileArtifactValue>> {
        let mut result = None;
        if input
            .as_artifact()
            .is_some_and(|a| a.is_child_of_declared_directory())
        {
            for (tree, value) in &self.tree_artifacts {
                if input.get_exec_path().starts_with(tree.get_exec_path()) {
                    result = value
                        .child_values
                        .iter()
                        .find(|(c, _)| c == input)
                        .map(|(_, m)| m.clone());
                    break;
                }
            }
        } else {
            result = self.inputs.get(input).cloned();
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
        self.tree_artifacts.get(input).cloned()
    }

    fn get_fileset(&self, _input: &ActionInputRef) -> Option<Arc<FilesetOutputTree>> {
        None
    }

    fn get_runfiles_metadata(&self, input: &ActionInputRef) -> Option<Arc<RunfilesArtifactValue>> {
        self.runfiles_inputs.get(input).cloned()
    }

    fn get_input(&self, exec_path: &PathFragment) -> Option<ActionInputRef> {
        for i in self.inputs.keys() {
            if i.get_exec_path() == exec_path {
                return Some(i.clone());
            }
        }
        for (tree, value) in &self.tree_artifacts {
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

/// `SpawnBuilder`.
pub struct SpawnBuilder {
    mnemonic: String,
    progress_message: String,
    owner_label: String,
    args: Vec<String>,
    environment: IndexMap<String, String>,
    execution_info: IndexMap<String, String>,
    exec_properties: IndexMap<String, String>,
    inputs: NestedSetBuilder<ActionInputRef>,
    outputs: Vec<ActionInputRef>,
    tools: NestedSetBuilder<ActionInputRef>,
}

impl SpawnBuilder {
    pub fn new(args: &[&str]) -> SpawnBuilder {
        SpawnBuilder {
            mnemonic: "Mnemonic".to_string(),
            progress_message: "progress message".to_string(),
            owner_label: "//dummy:label".to_string(),
            args: args.iter().map(|s| s.to_string()).collect(),
            environment: IndexMap::new(),
            execution_info: IndexMap::new(),
            exec_properties: IndexMap::new(),
            inputs: NestedSetBuilder::stable_order(),
            outputs: Vec::new(),
            tools: NestedSetBuilder::stable_order(),
        }
    }

    pub fn build(&self) -> SimpleSpawn {
        let mut owner = FakeOwner::new(
            &self.mnemonic,
            Some(self.progress_message.as_str()),
            Some(Label::parse_canonical_unchecked(&self.owner_label)),
        );
        owner.owner.exec_properties = self.exec_properties.clone();
        // `ImmutableSet.copyOf(outputs)`.
        let mut outputs: Vec<ActionInputRef> = Vec::new();
        for o in &self.outputs {
            if !outputs.contains(o) {
                outputs.push(o.clone());
            }
        }
        SimpleSpawn::new(
            Arc::new(owner),
            self.args.clone(),
            self.environment.clone(),
            self.execution_info.clone(),
            self.inputs.build(),
            self.tools.build(),
            outputs,
            None,
            Arc::new(ResourceSet::zero()),
            PathMapper::NOOP,
        )
    }

    pub fn with_mnemonic(mut self, mnemonic: &str) -> Self {
        self.mnemonic = mnemonic.to_string();
        self
    }

    pub fn with_environment(mut self, key: &str, value: &str) -> Self {
        self.environment.insert(key.to_string(), value.to_string());
        self
    }

    pub fn with_execution_info(mut self, key: &str, value: &str) -> Self {
        self.execution_info
            .insert(key.to_string(), value.to_string());
        self
    }

    pub fn with_combined_exec_properties(mut self, exec_properties: &[(&str, &str)]) -> Self {
        self.exec_properties = exec_properties
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        self
    }

    pub fn with_input(mut self, input: &ActionInputRef) -> Self {
        self.inputs.add(input.clone());
        self
    }

    pub fn with_inputs(mut self, inputs: &[&ActionInputRef]) -> Self {
        for input in inputs {
            self.inputs.add((*input).clone());
        }
        self
    }

    pub fn with_input_set(mut self, inputs: &NestedSet<ActionInputRef>) -> Self {
        self.inputs.add_transitive(inputs);
        self
    }

    pub fn with_outputs(mut self, outputs: &[&ActionInputRef]) -> Self {
        for output in outputs {
            self.outputs.push((*output).clone());
        }
        self
    }

    pub fn with_tools(mut self, tools: &[&ActionInputRef]) -> Self {
        for tool in tools {
            self.tools.add((*tool).clone());
        }
        self
    }

    pub fn with_tool_set(mut self, tools: &NestedSet<ActionInputRef>) -> Self {
        self.tools.add_transitive(tools);
        self
    }
}

/// `FakeActionFileSystem`: a file system that provides a fast digest, but refuses to compute it
/// from the file contents (which won't be available when building without the bytes).
#[derive(Debug)]
pub struct FakeActionFileSystem {
    delegate: FileSystemRef,
}

impl FileSystem for FakeActionFileSystem {
    fn get_fast_digest(&self, path: &PathFragment) -> io::Result<Option<Vec<u8>>> {
        self.delegate.get_digest(path).map(Some)
    }
    fn get_digest(&self, _path: &PathFragment) -> io::Result<Vec<u8>> {
        panic!("UnsupportedOperationException")
    }

    // DelegateFileSystem.
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
    fn stat(&self, path: &PathFragment, follow_symlinks: bool) -> io::Result<FileStatus> {
        self.delegate.stat(path, follow_symlinks)
    }
    fn stat_metadata(
        &self,
        path: &PathFragment,
        follow_symlinks: bool,
    ) -> io::Result<Option<std::sync::Arc<dyn blaze_vfs::StatMetadata>>> {
        self.delegate.stat_metadata(path, follow_symlinks)
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
    fn create_fs_dependent_hard_link(
        &self,
        link_path: &PathFragment,
        original_path: &PathFragment,
    ) -> io::Result<()> {
        self.delegate
            .create_fs_dependent_hard_link(link_path, original_path)
    }
}

/// Test parameter determining whether the spawn inputs are also tool inputs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputsMode {
    Tools,
    NonTools,
}

impl InputsMode {
    pub const ALL: [InputsMode; 2] = [InputsMode::Tools, InputsMode::NonTools];
    pub fn is_tool(self) -> bool {
        self == InputsMode::Tools
    }
}

/// Test parameter determining whether to emulate building with or without the bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutputsMode {
    WithBytes,
    WithoutBytes,
}

impl OutputsMode {
    pub const ALL: [OutputsMode; 2] = [OutputsMode::WithBytes, OutputsMode::WithoutBytes];
}

/// Test parameter determining whether an input/output directory should be empty.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DirContents {
    Empty,
    NonEmpty,
}

impl DirContents {
    pub const ALL: [DirContents; 2] = [DirContents::Empty, DirContents::NonEmpty];
    pub fn is_empty(self) -> bool {
        self == DirContents::Empty
    }
}

/// Test parameter determining whether an output is indirected through a symlink.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutputIndirection {
    Direct,
    Indirect,
}

impl OutputIndirection {
    pub const ALL: [OutputIndirection; 2] =
        [OutputIndirection::Direct, OutputIndirection::Indirect];
    pub fn via_symlink(self) -> bool {
        self == OutputIndirection::Indirect
    }
}

/// The class-level `@TestParameter boolean siblingRepositoryLayout`.
pub const SIBLING_REPOSITORY_LAYOUT: [bool; 2] = [false, true];

pub fn default_timeout() -> Duration {
    Duration::ZERO
}

pub fn default_spawn_builder() -> SpawnBuilder {
    SpawnBuilder::new(&["cmd", "--opt"])
}

pub fn default_spawn() -> SimpleSpawn {
    default_spawn_builder().build()
}

pub fn default_spawn_result_builder() -> SpawnResultBuilder {
    let mut builder = SpawnResultBuilder::new();
    builder
        .set_runner_name("runner")
        .set_status(Status::Success);
    builder
}

pub fn default_spawn_result() -> SpawnResult {
    default_spawn_result_builder().build()
}

pub fn default_spawn_exec() -> SpawnExec {
    SpawnExec {
        command_args: vec!["cmd".to_string(), "--opt".to_string()],
        runner: "runner".to_string(),
        remotable: true,
        cacheable: true,
        remote_cacheable: true,
        mnemonic: "Mnemonic".to_string(),
        target_label: "//dummy:label".to_string(),
        metrics: Some(protos::SpawnMetrics::default()),
        ..Default::default()
    }
}

pub fn file(path: &str, digest: Option<Digest>, is_tool: bool) -> File {
    File {
        path: path.to_string(),
        digest,
        is_tool,
        ..Default::default()
    }
}

pub fn write_file(path: &Path, contents: &str) {
    path.get_parent_directory()
        .unwrap()
        .create_directory_and_parents()
        .unwrap();
    let mut out = path.get_output_stream().unwrap();
    out.write_all(contents.as_bytes()).unwrap();
    out.flush().unwrap();
}

fn bin(path: &str) -> String {
    format!("{PRODUCT_NAME}-out/k8-fastbuild/bin/{path}")
}

// ---- The tests of SpawnLogContextTestBase. ----

pub fn test_file_input(t: &dyn SpawnLogContextTest) {
    for sibling in SIBLING_REPOSITORY_LAYOUT {
        for inputs_mode in InputsMode::ALL {
            let f = Fixture::new(sibling);
            let file_input = create_artifact(&f.root_dir, "file");

            write_file(&f.path_of(&file_input), "abc");

            let mut spawn = default_spawn_builder().with_inputs(&[&file_input]);
            if inputs_mode.is_tool() {
                spawn = spawn.with_tools(&[&file_input]);
            }

            let context = f.create_spawn_log_context(t);

            f.log_spawn(
                &*context,
                &spawn.build(),
                &f.create_input_metadata_provider(&[&file_input]),
                f.create_input_map(&[&file_input]),
                &f.fs,
                default_timeout(),
                &default_spawn_result(),
            );

            let mut expected = default_spawn_exec();
            expected.inputs.push(file(
                "file",
                Some(f.get_digest("abc")),
                inputs_mode.is_tool(),
            ));
            t.close_and_assert_log(&f, context, vec![expected]);
        }
    }
}

pub fn test_file_input_with_directory_contents(t: &dyn SpawnLogContextTest) {
    for sibling in SIBLING_REPOSITORY_LAYOUT {
        for inputs_mode in InputsMode::ALL {
            for dir_contents in DirContents::ALL {
                let f = Fixture::new(sibling);
                let file_input = create_artifact(&f.root_dir, "file");

                f.path_of(&file_input)
                    .create_directory_and_parents()
                    .unwrap();
                if !dir_contents.is_empty() {
                    write_file(&f.path_of(&file_input).get_child("file"), "abc");
                }

                let mut spawn = default_spawn_builder().with_inputs(&[&file_input]);
                if inputs_mode.is_tool() {
                    spawn = spawn.with_tools(&[&file_input]);
                }

                let context = f.create_spawn_log_context(t);

                f.log_spawn(
                    &*context,
                    &spawn.build(),
                    &f.create_input_metadata_provider(&[&file_input]),
                    f.create_input_map(&[&file_input]),
                    &f.fs,
                    default_timeout(),
                    &default_spawn_result(),
                );

                let mut expected = default_spawn_exec();
                if !dir_contents.is_empty() {
                    expected.inputs.push(file(
                        "file/file",
                        Some(f.get_digest("abc")),
                        inputs_mode.is_tool(),
                    ));
                }
                t.close_and_assert_log(&f, context, vec![expected]);
            }
        }
    }
}

pub fn test_directory_input(t: &dyn SpawnLogContextTest) {
    for sibling in SIBLING_REPOSITORY_LAYOUT {
        for inputs_mode in InputsMode::ALL {
            for dir_contents in DirContents::ALL {
                let f = Fixture::new(sibling);
                let dir_input = create_artifact(&f.root_dir, "dir");

                f.path_of(&dir_input)
                    .create_directory_and_parents()
                    .unwrap();
                if !dir_contents.is_empty() {
                    write_file(&f.path_of(&dir_input).get_child("file"), "abc");
                }

                let mut spawn = default_spawn_builder().with_inputs(&[&dir_input]);
                if inputs_mode == InputsMode::Tools {
                    spawn = spawn.with_tools(&[&dir_input]);
                }

                let context = f.create_spawn_log_context(t);

                f.log_spawn(
                    &*context,
                    &spawn.build(),
                    &f.create_input_metadata_provider(&[&dir_input]),
                    f.create_input_map(&[&dir_input]),
                    &f.fs,
                    default_timeout(),
                    &default_spawn_result(),
                );

                let mut expected = default_spawn_exec();
                if !dir_contents.is_empty() {
                    expected.inputs.push(file(
                        "dir/file",
                        Some(f.get_digest("abc")),
                        inputs_mode.is_tool(),
                    ));
                }
                t.close_and_assert_log(&f, context, vec![expected]);
            }
        }
    }
}

pub fn test_tree_input(t: &dyn SpawnLogContextTest) {
    for sibling in SIBLING_REPOSITORY_LAYOUT {
        for inputs_mode in InputsMode::ALL {
            for dir_contents in DirContents::ALL {
                let f = Fixture::new(sibling);
                let tree_input = create_tree_artifact_with_generating_action(&f.output_dir, "tree");

                f.path_of(&tree_input)
                    .create_directory_and_parents()
                    .unwrap();
                if !dir_contents.is_empty() {
                    write_file(&f.path_of(&tree_input).get_child("child"), "abc");
                }

                let mut spawn = default_spawn_builder().with_inputs(&[&tree_input]);
                if inputs_mode.is_tool() {
                    spawn = spawn.with_tools(&[&tree_input]);
                }

                let context = f.create_spawn_log_context(t);

                f.log_spawn(
                    &*context,
                    &spawn.build(),
                    &f.create_input_metadata_provider(&[&tree_input]),
                    f.create_input_map(&[&tree_input]),
                    &f.fs,
                    default_timeout(),
                    &default_spawn_result(),
                );

                let mut expected = default_spawn_exec();
                if !dir_contents.is_empty() {
                    expected.inputs.push(file(
                        &bin("tree/child"),
                        Some(f.get_digest("abc")),
                        inputs_mode.is_tool(),
                    ));
                }
                t.close_and_assert_log(&f, context, vec![expected]);
            }
        }
    }
}

pub fn test_unresolved_symlink_input(t: &dyn SpawnLogContextTest) {
    for sibling in SIBLING_REPOSITORY_LAYOUT {
        for inputs_mode in InputsMode::ALL {
            let f = Fixture::new(sibling);
            let symlink_input = create_unresolved_symlink_artifact(&f.output_dir, "symlink");

            f.path_of(&symlink_input)
                .get_parent_directory()
                .unwrap()
                .create_directory_and_parents()
                .unwrap();
            f.path_of(&symlink_input)
                .create_symbolic_link_to_fragment(&PathFragment::create("/some/path"))
                .unwrap();

            let mut spawn = default_spawn_builder().with_inputs(&[&symlink_input]);
            if inputs_mode.is_tool() {
                spawn = spawn.with_tools(&[&symlink_input]);
            }

            let context = f.create_spawn_log_context(t);

            f.log_spawn(
                &*context,
                &spawn.build(),
                &f.create_input_metadata_provider(&[&symlink_input]),
                f.create_input_map(&[&symlink_input]),
                &f.fs,
                default_timeout(),
                &default_spawn_result(),
            );

            let mut expected = default_spawn_exec();
            expected.inputs.push(File {
                path: bin("symlink"),
                symlink_target_path: "/some/path".to_string(),
                is_tool: inputs_mode.is_tool(),
                ..Default::default()
            });
            t.close_and_assert_log(&f, context, vec![expected]);
        }
    }
}

pub fn test_fileset_input(t: &dyn SpawnLogContextTest) {
    for sibling in SIBLING_REPOSITORY_LAYOUT {
        for dir_contents in DirContents::ALL {
            let f = Fixture::new(sibling);
            let fileset_input = create_fileset_artifact(&f.output_dir, "dir");

            f.path_of(&fileset_input)
                .create_directory_and_parents()
                .unwrap();
            if !dir_contents.is_empty() {
                let target = f.scratch_path("file.txt");
                write_file(&target, "abc");
                f.path_of(&fileset_input)
                    .get_child("file.txt")
                    .create_symbolic_link_to_fragment(target.as_fragment())
                    .unwrap();
            }

            let spawn = default_spawn_builder().with_input(&fileset_input).build();

            let context = f.create_spawn_log_context(t);

            f.log_spawn(
                &*context,
                &spawn,
                &f.create_input_metadata_provider(&[&fileset_input]),
                f.create_input_map(&[&fileset_input]),
                &f.fs,
                default_timeout(),
                &default_spawn_result(),
            );

            let mut expected = default_spawn_exec();
            if !dir_contents.is_empty() {
                expected
                    .inputs
                    .push(file(&bin("dir/file.txt"), Some(f.get_digest("abc")), false));
            }
            t.close_and_assert_log(&f, context, vec![expected]);
        }
    }
}

pub fn test_param_file_input(t: &dyn SpawnLogContextTest) {
    for sibling in SIBLING_REPOSITORY_LAYOUT {
        let f = Fixture::new(sibling);
        let param_file_input = create_param_file_action_input("foo.params", &["a", "b", "c"]);

        // Do not materialize the file on disk, which would be the case when running remotely.
        let spawn = default_spawn_builder().with_inputs(&[&param_file_input]);

        let context = f.create_spawn_log_context(t);

        f.log_spawn(
            &*context,
            &spawn.build(),
            // ParamFileActionInputs appear in the input map but not in the metadata provider.
            &f.create_input_metadata_provider(&[]),
            f.create_input_map(&[&param_file_input]),
            &f.fs,
            default_timeout(),
            &default_spawn_result(),
        );

        let mut expected = default_spawn_exec();
        expected
            .inputs
            .push(file("foo.params", Some(f.get_digest("a\nb\nc\n")), false));
        t.close_and_assert_log(&f, context, vec![expected]);
    }
}

pub fn test_file_output(t: &dyn SpawnLogContextTest) {
    for sibling in SIBLING_REPOSITORY_LAYOUT {
        for outputs_mode in OutputsMode::ALL {
            for indirection in OutputIndirection::ALL {
                let f = Fixture::new(sibling);
                let file_output = create_artifact(&f.output_dir, "file");

                let actual_path = if indirection.via_symlink() {
                    f.root_path(&f.output_dir).get_child("actual")
                } else {
                    f.path_of(&file_output)
                };

                if indirection.via_symlink() {
                    f.path_of(&file_output)
                        .get_parent_directory()
                        .unwrap()
                        .create_directory_and_parents()
                        .unwrap();
                    f.path_of(&file_output)
                        .create_symbolic_link(&actual_path)
                        .unwrap();
                }

                write_file(&actual_path, "abc");

                let spawn = default_spawn_builder()
                    .with_outputs(&[&file_output])
                    .build();

                let context = f.create_spawn_log_context(t);

                f.log_spawn(
                    &*context,
                    &spawn,
                    &f.create_input_metadata_provider(&[]),
                    f.create_input_map(&[]),
                    &f.get_action_file_system(outputs_mode),
                    default_timeout(),
                    &default_spawn_result(),
                );

                let mut expected = default_spawn_exec();
                expected.listed_outputs.push(bin("file"));
                expected
                    .actual_outputs
                    .push(file(&bin("file"), Some(f.get_digest("abc")), false));
                t.close_and_assert_log(&f, context, vec![expected]);
            }
        }
    }
}

pub fn test_file_output_with_invalid_type(t: &dyn SpawnLogContextTest) {
    for sibling in SIBLING_REPOSITORY_LAYOUT {
        for outputs_mode in OutputsMode::ALL {
            let f = Fixture::new(sibling);
            let file_output = create_artifact(&f.output_dir, "file");

            f.path_of(&file_output)
                .create_directory_and_parents()
                .unwrap();
            write_file(&f.path_of(&file_output).get_child("file"), "abc");

            let spawn = default_spawn_builder().with_outputs(&[&file_output]);

            let context = f.create_spawn_log_context(t);

            f.log_spawn(
                &*context,
                &spawn.build(),
                &f.create_input_metadata_provider(&[]),
                f.create_input_map(&[]),
                &f.get_action_file_system(outputs_mode),
                default_timeout(),
                &default_spawn_result(),
            );

            let mut expected = default_spawn_exec();
            expected.listed_outputs.push(bin("file"));
            t.close_and_assert_log(&f, context, vec![expected]);
        }
    }
}

pub fn test_tree_output(t: &dyn SpawnLogContextTest) {
    for sibling in SIBLING_REPOSITORY_LAYOUT {
        for outputs_mode in OutputsMode::ALL {
            for dir_contents in DirContents::ALL {
                for indirection in OutputIndirection::ALL {
                    let f = Fixture::new(sibling);
                    let tree_output =
                        create_tree_artifact_with_generating_action(&f.output_dir, "tree");

                    let actual_path = if indirection.via_symlink() {
                        f.root_path(&f.output_dir).get_child("actual")
                    } else {
                        f.path_of(&tree_output)
                    };

                    if indirection.via_symlink() {
                        f.path_of(&tree_output)
                            .get_parent_directory()
                            .unwrap()
                            .create_directory_and_parents()
                            .unwrap();
                        f.path_of(&tree_output)
                            .create_symbolic_link(&actual_path)
                            .unwrap();
                    }

                    actual_path.create_directory_and_parents().unwrap();
                    if !dir_contents.is_empty() {
                        let first_child_path = actual_path.get_relative_str("dir1/file1");
                        let second_child_path = actual_path.get_relative_str("dir2/file2");
                        write_file(&first_child_path, "abc");
                        write_file(&second_child_path, "def");
                        let empty_subdir_path = actual_path.get_relative_str("dir3");
                        empty_subdir_path.create_directory_and_parents().unwrap();
                    }

                    let spawn = default_spawn_builder()
                        .with_outputs(&[&tree_output])
                        .build();

                    let context = f.create_spawn_log_context(t);

                    f.log_spawn(
                        &*context,
                        &spawn,
                        &f.create_input_metadata_provider(&[]),
                        f.create_input_map(&[]),
                        &f.get_action_file_system(outputs_mode),
                        default_timeout(),
                        &default_spawn_result(),
                    );

                    let mut expected = default_spawn_exec();
                    expected.listed_outputs.push(bin("tree"));
                    if !dir_contents.is_empty() {
                        expected.actual_outputs.push(file(
                            &bin("tree/dir1/file1"),
                            Some(f.get_digest("abc")),
                            false,
                        ));
                        expected.actual_outputs.push(file(
                            &bin("tree/dir2/file2"),
                            Some(f.get_digest("def")),
                            false,
                        ));
                    }
                    t.close_and_assert_log(&f, context, vec![expected]);
                }
            }
        }
    }
}

pub fn test_tree_output_with_invalid_type(t: &dyn SpawnLogContextTest) {
    for sibling in SIBLING_REPOSITORY_LAYOUT {
        for outputs_mode in OutputsMode::ALL {
            let f = Fixture::new(sibling);
            let tree_output = create_tree_artifact_with_generating_action(&f.output_dir, "tree");

            write_file(&f.path_of(&tree_output), "abc");

            let spawn = default_spawn_builder().with_outputs(&[&tree_output]);

            let context = f.create_spawn_log_context(t);

            f.log_spawn(
                &*context,
                &spawn.build(),
                &f.create_input_metadata_provider(&[]),
                f.create_input_map(&[]),
                &f.get_action_file_system(outputs_mode),
                default_timeout(),
                &default_spawn_result(),
            );

            let mut expected = default_spawn_exec();
            expected.listed_outputs.push(bin("tree"));
            t.close_and_assert_log(&f, context, vec![expected]);
        }
    }
}

pub fn test_unresolved_symlink_output(t: &dyn SpawnLogContextTest) {
    for sibling in SIBLING_REPOSITORY_LAYOUT {
        for outputs_mode in OutputsMode::ALL {
            let f = Fixture::new(sibling);
            let symlink_output = create_unresolved_symlink_artifact(&f.output_dir, "symlink");

            f.path_of(&symlink_output)
                .get_parent_directory()
                .unwrap()
                .create_directory_and_parents()
                .unwrap();
            f.path_of(&symlink_output)
                .create_symbolic_link_to_fragment(&PathFragment::create("/some/path"))
                .unwrap();

            let spawn = default_spawn_builder().with_outputs(&[&symlink_output]);

            let context = f.create_spawn_log_context(t);

            f.log_spawn(
                &*context,
                &spawn.build(),
                &f.create_input_metadata_provider(&[]),
                f.create_input_map(&[]),
                &f.get_action_file_system(outputs_mode),
                default_timeout(),
                &default_spawn_result(),
            );

            let mut expected = default_spawn_exec();
            expected.listed_outputs.push(bin("symlink"));
            expected.actual_outputs.push(File {
                path: bin("symlink"),
                symlink_target_path: "/some/path".to_string(),
                ..Default::default()
            });
            t.close_and_assert_log(&f, context, vec![expected]);
        }
    }
}

pub fn test_unresolved_symlink_output_with_invalid_type(t: &dyn SpawnLogContextTest) {
    for sibling in SIBLING_REPOSITORY_LAYOUT {
        for outputs_mode in OutputsMode::ALL {
            let f = Fixture::new(sibling);
            let symlink_output = create_unresolved_symlink_artifact(&f.output_dir, "symlink");

            write_file(&f.path_of(&symlink_output), "abc");

            let spawn = default_spawn_builder().with_outputs(&[&symlink_output]);

            let context = f.create_spawn_log_context(t);

            f.log_spawn(
                &*context,
                &spawn.build(),
                &f.create_input_metadata_provider(&[]),
                f.create_input_map(&[]),
                &f.get_action_file_system(outputs_mode),
                default_timeout(),
                &default_spawn_result(),
            );

            let mut expected = default_spawn_exec();
            expected.listed_outputs.push(bin("symlink"));
            t.close_and_assert_log(&f, context, vec![expected]);
        }
    }
}

pub fn test_missing_output(t: &dyn SpawnLogContextTest) {
    for sibling in SIBLING_REPOSITORY_LAYOUT {
        for outputs_mode in OutputsMode::ALL {
            let f = Fixture::new(sibling);
            let missing_output = create_artifact(&f.output_dir, "missing");

            let spawn = default_spawn_builder().with_outputs(&[&missing_output]);

            let context = f.create_spawn_log_context(t);

            f.log_spawn(
                &*context,
                &spawn.build(),
                &f.create_input_metadata_provider(&[]),
                f.create_input_map(&[]),
                &f.get_action_file_system(outputs_mode),
                default_timeout(),
                &default_spawn_result(),
            );

            let mut expected = default_spawn_exec();
            expected.listed_outputs.push(bin("missing"));
            t.close_and_assert_log(&f, context, vec![expected]);
        }
    }
}

/// Logs `spawn` with no inputs or outputs and asserts the log holds `expected`.
fn log_simple(
    t: &dyn SpawnLogContextTest,
    f: &Fixture,
    context: Box<dyn SpawnLogContext>,
    spawn: &dyn Spawn,
    timeout: Duration,
    result: &SpawnResult,
    expected: SpawnExec,
) {
    f.log_spawn(
        &*context,
        spawn,
        &f.create_input_metadata_provider(&[]),
        f.create_input_map(&[]),
        &f.fs,
        timeout,
        result,
    );
    t.close_and_assert_log(f, context, vec![expected]);
}

/// Not a test of Java's `SpawnLogContextTestBase`: a spawn whose argument, environment entry,
/// input path and output path are non-ASCII internal strings (`é`, `🐷` and a lone byte FF).
/// `SpawnLogContext.getEnvironmentVariables` always applies `internalToUnicode` to the
/// environment; the argument and the paths go through `convert`, which is what the context under
/// test does with them (`CompactSpawnLogContext`: `internalToUnicode`;
/// `ExpandedSpawnLogContext`: nothing, the internal string is stored as is).
// Java: com/google/devtools/build/lib/exec/SpawnLogContext.java:117 SpawnLogContext.getEnvironmentVariables
pub fn test_non_ascii_strings(t: &dyn SpawnLogContextTest, convert: fn(&str) -> String) {
    use blaze_util::string_encoding::{internal_to_unicode, latin1_decode};
    let internal = |b: &[u8]| latin1_decode(b).into_owned();
    let arg = internal(b"a\xc3\xa9\xf0\x9f\x90\xb7\xff");
    let env_name = internal(b"V\xc3\x84R");
    let env_value = internal(b"v\xc3\xa4lue\xf0\x9f\x90\xb7");
    let input_name = internal(b"in_\xc3\xa9_\xff");
    let output_name = internal(b"out_\xc3\xbc");
    for sibling in SIBLING_REPOSITORY_LAYOUT {
        let f = Fixture::new(sibling);
        let file_input = create_artifact(&f.root_dir, &input_name);
        let file_output = create_artifact(&f.output_dir, &output_name);
        write_file(&f.path_of(&file_input), "abc");
        write_file(&f.path_of(&file_output), "def");

        let spawn = SpawnBuilder::new(&["cmd", &arg])
            .with_environment(&env_name, &env_value)
            .with_inputs(&[&file_input])
            .with_outputs(&[&file_output])
            .build();

        let context = f.create_spawn_log_context(t);

        f.log_spawn(
            &*context,
            &spawn,
            &f.create_input_metadata_provider(&[&file_input]),
            f.create_input_map(&[&file_input]),
            &f.fs,
            default_timeout(),
            &default_spawn_result(),
        );

        let mut expected = default_spawn_exec();
        expected.command_args = vec!["cmd".to_string(), convert(&arg)];
        expected.environment_variables.push(EnvironmentVariable {
            name: internal_to_unicode(&env_name).into_owned(),
            value: internal_to_unicode(&env_value).into_owned(),
        });
        expected.inputs.push(file(
            &convert(&input_name),
            Some(f.get_digest("abc")),
            false,
        ));
        expected.listed_outputs.push(convert(&bin(&output_name)));
        expected.actual_outputs.push(file(
            &convert(&bin(&output_name)),
            Some(f.get_digest("def")),
            false,
        ));
        t.close_and_assert_log(&f, context, vec![expected]);
    }
}

pub fn test_environment(t: &dyn SpawnLogContextTest) {
    for sibling in SIBLING_REPOSITORY_LAYOUT {
        let f = Fixture::new(sibling);
        let spawn = default_spawn_builder()
            .with_environment("SPAM", "eggs")
            .with_environment("FOO", "bar")
            .build();

        let context = f.create_spawn_log_context(t);

        let mut expected = default_spawn_exec();
        expected.environment_variables.push(EnvironmentVariable {
            name: "FOO".to_string(),
            value: "bar".to_string(),
        });
        expected.environment_variables.push(EnvironmentVariable {
            name: "SPAM".to_string(),
            value: "eggs".to_string(),
        });
        log_simple(
            t,
            &f,
            context,
            &spawn,
            default_timeout(),
            &default_spawn_result(),
            expected,
        );
    }
}

fn property(name: &str, value: &str) -> platform::Property {
    platform::Property {
        name: name.to_string(),
        value: value.to_string(),
    }
}

pub fn test_default_platform_properties(t: &dyn SpawnLogContextTest) {
    for sibling in SIBLING_REPOSITORY_LAYOUT {
        let f = Fixture::new(sibling);
        let context = t.create_spawn_log_context_with(&f, &[("a", "1"), ("b", "2")]);

        let mut expected = default_spawn_exec();
        expected.platform = Some(Platform {
            properties: vec![property("a", "1"), property("b", "2")],
        });
        log_simple(
            t,
            &f,
            context,
            &default_spawn(),
            default_timeout(),
            &default_spawn_result(),
            expected,
        );
    }
}

pub fn test_spawn_platform_properties(t: &dyn SpawnLogContextTest) {
    for sibling in SIBLING_REPOSITORY_LAYOUT {
        let f = Fixture::new(sibling);
        let spawn = default_spawn_builder()
            .with_combined_exec_properties(&[("a", "3"), ("c", "4")])
            .build();

        let context = t.create_spawn_log_context_with(&f, &[("a", "1"), ("b", "2")]);

        // The spawn properties should override the default properties.
        let mut expected = default_spawn_exec();
        expected.platform = Some(Platform {
            properties: vec![property("a", "3"), property("b", "2"), property("c", "4")],
        });
        log_simple(
            t,
            &f,
            context,
            &spawn,
            default_timeout(),
            &default_spawn_result(),
            expected,
        );
    }
}

pub fn test_execution_info(t: &dyn SpawnLogContextTest) {
    for sibling in SIBLING_REPOSITORY_LAYOUT {
        for requirement in ["no-remote", "no-cache", "no-remote-cache"] {
            let f = Fixture::new(sibling);
            let spawn = default_spawn_builder()
                .with_execution_info(requirement, "")
                .build();

            let context = f.create_spawn_log_context(t);

            let mut expected = default_spawn_exec();
            expected.remotable = requirement != "no-remote";
            expected.cacheable = requirement != "no-cache";
            expected.remote_cacheable = requirement != "no-cache"
                && requirement != "no-remote"
                && requirement != "no-remote-cache";
            log_simple(
                t,
                &f,
                context,
                &spawn,
                default_timeout(),
                &default_spawn_result(),
                expected,
            );
        }
    }
}

pub fn test_cache_hit(t: &dyn SpawnLogContextTest) {
    for sibling in SIBLING_REPOSITORY_LAYOUT {
        let f = Fixture::new(sibling);
        let context = f.create_spawn_log_context(t);

        let mut result = default_spawn_result_builder();
        result.set_cache_hit(true);

        let mut expected = default_spawn_exec();
        expected.cache_hit = true;
        log_simple(
            t,
            &f,
            context,
            &default_spawn(),
            default_timeout(),
            &result.build(),
            expected,
        );
    }
}

pub fn test_digest(t: &dyn SpawnLogContextTest) {
    for sibling in SIBLING_REPOSITORY_LAYOUT {
        let f = Fixture::new(sibling);
        let context = f.create_spawn_log_context(t);

        let digest = f.get_digest("something");

        let mut result = default_spawn_result_builder();
        result.set_digest(Some(digest.clone()));

        let mut expected = default_spawn_exec();
        expected.digest = Some(digest);
        log_simple(
            t,
            &f,
            context,
            &default_spawn(),
            default_timeout(),
            &result.build(),
            expected,
        );
    }
}

pub fn test_timeout(t: &dyn SpawnLogContextTest) {
    for sibling in SIBLING_REPOSITORY_LAYOUT {
        let f = Fixture::new(sibling);
        let context = f.create_spawn_log_context(t);

        let mut expected = default_spawn_exec();
        expected.timeout_millis = Duration::from_secs(42).as_millis() as i64;
        log_simple(
            t,
            &f,
            context,
            &default_spawn(),
            /* timeout= */ Duration::from_secs(42),
            &default_spawn_result(),
            expected,
        );
    }
}

pub fn test_spawn_metrics(t: &dyn SpawnLogContextTest) {
    for sibling in SIBLING_REPOSITORY_LAYOUT {
        let f = Fixture::new(sibling);
        let metrics = SpawnMetrics {
            total_time_in_ms: 1,
            measured_memory_peak_bytes: 42,
            ..SpawnMetrics::for_exec(blaze_exec::actions::ExecKind::Local)
        };

        let context = f.create_spawn_log_context(t);

        let now = SystemTime::now();
        let mut result = default_spawn_result_builder();
        result.set_spawn_metrics(metrics).set_start_time(now);

        // `Timestamps.fromDate(Date.from(now))`: millisecond precision.
        let millis = now.duration_since(UNIX_EPOCH).unwrap().as_millis() as i64;
        let mut expected = default_spawn_exec();
        expected.metrics = Some(protos::SpawnMetrics {
            total_time: Some(millis_to_proto(1)),
            measured_memory_peak_bytes: 42,
            start_time: Some(Timestamp {
                seconds: millis.div_euclid(1000),
                nanos: (millis.rem_euclid(1000) * 1_000_000) as i32,
            }),
            ..Default::default()
        });
        log_simple(
            t,
            &f,
            context,
            &default_spawn(),
            default_timeout(),
            &result.build(),
            expected,
        );
    }
}

pub fn test_status(t: &dyn SpawnLogContextTest) {
    for sibling in SIBLING_REPOSITORY_LAYOUT {
        let f = Fixture::new(sibling);
        let context = f.create_spawn_log_context(t);

        // SpawnResult requires a non-zero exit code and non-null failure details when the status
        // isn't successful.
        let mut result = default_spawn_result_builder();
        result
            .set_status(Status::NonZeroExit)
            .set_exit_code(37)
            .set_failure_detail(FailureDetail {
                message: "oops".to_string(),
                category: Some(failure_detail::Category::Crash(Crash::default())),
            });

        let mut expected = default_spawn_exec();
        expected.exit_code = 37;
        expected.status = Status::NonZeroExit.to_string();
        log_simple(
            t,
            &f,
            context,
            &default_spawn(),
            default_timeout(),
            &result.build(),
            expected,
        );
    }
}
