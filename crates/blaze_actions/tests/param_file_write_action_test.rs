//! Port of `src/test/java/com/google/devtools/build/lib/analysis/actions/ParamFileWriteActionTest.java`.

use std::fmt;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::Arc;

use blaze_actions::actions::action::InputMetadataProvider;
use blaze_actions::actions::action_lookup::{ActionLookupData, ActionLookupKey};
use blaze_actions::actions::command_line::{self, CommandLineRef};
use blaze_actions::actions::parameter_file::ParameterFileType;
use blaze_actions::actions::path_mapper::OutputPathsMode;
use blaze_actions::analysis::actions::abstract_file_write_action::{
    AbstractFileWriteAction, MNEMONIC,
};
use blaze_actions::analysis::actions::custom_command_line::CustomCommandLine;
use blaze_actions::analysis::actions::parameter_file_write_action::ParameterFileWriteAction;
use blaze_actions::{
    Action, ActionKeyContext, ActionLookupKeyRef, ActionOwner, Artifact, ArtifactOwner,
    ArtifactRoot, RootType, SpecialArtifactType,
};
use blaze_cmdline::Label;
use blaze_collect::{NestedSet, NestedSetBuilder, Order};
use blaze_events::location::Location;
use blaze_vfs::{DigestHashFunction, Path, PathFragment, Root, UnixFileSystem};
use indexmap::IndexMap;

/// A test `ActionLookupKey` standing in for `ActionsTestUtil.NULL_ARTIFACT_OWNER`'s key.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct TestKey;

impl fmt::Display for TestKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("TestKey")
    }
}

impl ActionLookupKey for TestKey {
    fn get_label(&self) -> Option<Label> {
        None
    }
    fn get_configuration_checksum(&self) -> Option<String> {
        None
    }
    fn function_name(&self) -> &'static str {
        "CONFIGURED_TARGET"
    }
    fn key_equals(&self, other: &dyn ActionLookupKey) -> bool {
        other.as_any().downcast_ref::<TestKey>().is_some()
    }
    fn key_hash(&self) -> u64 {
        let mut h = DefaultHasher::new();
        self.hash(&mut h);
        h.finish()
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// `ActionsTestUtil.NULL_ACTION_OWNER`.
fn null_action_owner() -> ActionOwner {
    ActionOwner::create_dummy(
        None,
        Location::from_file("dummy-file"),
        "dummy-target-kind",
        "dummy-configuration-mnemonic",
        "dummy-configuration",
        None,
        false,
        None,
        Vec::new(),
        IndexMap::new(),
    )
}

struct Fixture {
    exec_root: Path,
    root_dir: ArtifactRoot,
    output_artifact: Artifact,
    tree_artifact: Artifact,
}

// Java: ParamFileWriteActionTest.createArtifacts
fn fixture() -> Fixture {
    let fs = UnixFileSystem::create(DigestHashFunction::Sha256);
    let exec_root = Path::create(PathFragment::create("/exec"), &fs);
    let root_dir = ArtifactRoot::as_derived_root(&exec_root, RootType::OUTPUT, &["out"]);
    // Java: getBinArtifactWithNoOwner("destination.txt") (a file in the bin directory).
    let bin_root = ArtifactRoot::as_derived_root(
        &exec_root,
        RootType::OUTPUT,
        &["bazel-out", "k8-fastbuild", "bin"],
    );
    let output_artifact = Artifact::create_derived_artifact(
        bin_root.clone(),
        bin_root.get_exec_path().get_relative_str("destination.txt"),
        ActionLookupKeyRef::new(TestKey),
    );
    let tree_artifact = create_tree_artifact(&root_dir, "artifact/myTreeFileArtifact");
    Fixture {
        exec_root,
        root_dir,
        output_artifact,
        tree_artifact,
    }
}

// Java: ActionsTestUtil.createTreeArtifactWithGeneratingAction
fn create_tree_artifact(root_dir: &ArtifactRoot, root_relative_path: &str) -> Artifact {
    let owner = ActionLookupKeyRef::new(TestKey);
    let tree = Artifact::create_special_artifact(
        root_dir.clone(),
        root_dir
            .get_exec_path()
            .get_relative_str(root_relative_path),
        owner.clone(),
        SpecialArtifactType::TREE,
    );
    tree.set_generating_action_key(ActionLookupData::create(owner, 0));
    tree
}

impl Fixture {
    fn create_parameter_file_write_action(
        &self,
        input_tree_artifacts: NestedSet<Artifact>,
        command_line: CommandLineRef,
        executable: bool,
    ) -> ParameterFileWriteAction {
        ParameterFileWriteAction::with_inputs(
            null_action_owner(),
            input_tree_artifacts,
            self.output_artifact.clone(),
            command_line,
            ParameterFileType::UNQUOTED,
            executable,
            MNEMONIC,
            /* execution_info= */ &IndexMap::new(),
            OutputPathsMode::OFF,
        )
    }

    fn create_tree_artifact_expansion_command_line_default(&self) -> CommandLineRef {
        Arc::new(
            CustomCommandLine::builder()
                .add("--flag1")
                .add_expanded_tree_artifact_exec_paths(self.tree_artifact.clone())
                .build(),
        )
    }
}

fn create_normal_command_line() -> CommandLineRef {
    Arc::new(
        CustomCommandLine::builder()
            .add("--flag1")
            .add("--flag2")
            .add_all_with_arg("--flag3", ["value1", "value2"])
            .build(),
    )
}

#[test]
fn test_outputs() {
    let f = fixture();
    let action = f.create_parameter_file_write_action(
        NestedSetBuilder::empty_set(Order::STABLE_ORDER),
        create_normal_command_line(),
        false,
    );
    assert_eq!(
        Artifact::to_root_relative_paths(action.get_outputs().iter()),
        vec!["destination.txt".to_string()]
    );
}

#[test]
fn test_inputs() {
    let f = fixture();
    let action = f.create_parameter_file_write_action(
        NestedSetBuilder::create(Order::STABLE_ORDER, [f.tree_artifact.clone()]),
        f.create_tree_artifact_expansion_command_line_default(),
        false,
    );
    assert_eq!(
        Artifact::as_exec_paths(action.get_inputs().to_list().iter()),
        vec!["out/artifact/myTreeFileArtifact".to_string()]
    );
}

#[test]
#[ignore = "execution is in blaze_exec, which depends on this crate; output permissions of a \
            ParameterFileWriteAction are not tested there yet (the FileWriteAction ones are: \
            tests/analysis_actions_execute_test.rs test_can_write_executable_file)"]
fn test_non_executable_output() {}

#[test]
#[ignore = "execution is in blaze_exec, which depends on this crate; output permissions of a \
            ParameterFileWriteAction are not tested there yet (the FileWriteAction ones are: \
            tests/analysis_actions_execute_test.rs test_can_write_executable_file)"]
fn test_executable_output() {}

#[test]
fn test_write_command_line_without_tree_artifact_expansion() {
    // PORT: Java executes the action and reads the output file; execution is not ported (wave
    // W6), so the test checks the bytes of the action's deterministic writer, which is what
    // `execute` writes. `actionResult.spawnResults()` being empty is not checked.
    let f = fixture();
    let action = f.create_parameter_file_write_action(
        NestedSetBuilder::empty_set(Order::STABLE_ORDER),
        create_normal_command_line(),
        false,
    );
    let writer = action.new_deterministic_writer().unwrap();
    let content = String::from_utf8(writer.get_bytes().unwrap()).unwrap();
    assert_eq!(content.trim(), "--flag1\n--flag2\n--flag3\nvalue1\nvalue2");
}

/// Java: `ParamFileWriteActionTest.actionExecutionContext()`'s `FakeActionInputFileCache` with
/// the tree's two children.
struct FakeActionInputFileCache {
    tree: Artifact,
    children: Vec<Artifact>,
}

impl InputMetadataProvider for FakeActionInputFileCache {
    fn get_tree_children(&self, tree: &Artifact) -> Option<Vec<Artifact>> {
        (*tree == self.tree).then(|| self.children.clone())
    }
}

#[test]
fn test_write_command_line_with_tree_artifact_expansion_default() {
    // PORT: Java executes the action with an ActionExecutionContext; execution lives in
    // blaze_exec, so the test checks the bytes of the action's deterministic writer for the
    // context's input metadata provider, which is what `execute` writes.
    let f = fixture();
    let _ = (&f.exec_root, &f.root_dir);
    let action = f.create_parameter_file_write_action(
        NestedSetBuilder::create(Order::STABLE_ORDER, [f.tree_artifact.clone()]),
        f.create_tree_artifact_expansion_command_line_default(),
        false,
    );
    let provider = FakeActionInputFileCache {
        tree: f.tree_artifact.clone(),
        children: vec![
            Artifact::create_tree_output_str(&f.tree_artifact, "artifacts/treeFileArtifact1"),
            Artifact::create_tree_output_str(&f.tree_artifact, "artifacts/treeFileArtifact2"),
        ],
    };
    let writer = action
        .new_deterministic_writer_with_input_metadata_provider(&provider)
        .unwrap();
    let content = String::from_utf8(writer.get_bytes().unwrap()).unwrap();
    assert_eq!(
        content.trim(),
        "--flag1\n\
         out/artifact/myTreeFileArtifact/artifacts/treeFileArtifact1\n\
         out/artifact/myTreeFileArtifact/artifacts/treeFileArtifact2"
    );
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum KeyAttributes {
    Commandline,
    FileType,
}

#[test]
fn test_compute_key() {
    // Java: ActionTester.runTest over the subsets of KeyAttributes: actions built from
    // different attribute subsets have different keys, and rebuilding an action from the same
    // subset gives the same key.
    let fs = UnixFileSystem::create(DigestHashFunction::Sha256);
    let source_root = ArtifactRoot::as_source_root(Root::from_path(Path::create(
        PathFragment::create("/workspace"),
        &fs,
    )));
    let output_artifact = Artifact::new_source_artifact(
        source_root,
        PathFragment::create("output"),
        ArtifactOwner::NullOwner,
    );
    let action_key_context = ActionKeyContext::new();
    let build = |attributes_to_flip: &[KeyAttributes]| {
        let arg = if attributes_to_flip.contains(&KeyAttributes::Commandline) {
            "foo"
        } else {
            "bar"
        };
        let command_line = command_line::of(vec![arg.to_string()]);
        let parameter_file_type = if attributes_to_flip.contains(&KeyAttributes::FileType) {
            ParameterFileType::SHELL_QUOTED
        } else {
            ParameterFileType::UNQUOTED
        };
        ParameterFileWriteAction::new(
            null_action_owner(),
            output_artifact.clone(),
            command_line,
            parameter_file_type,
            false,
        )
    };
    let subsets: Vec<Vec<KeyAttributes>> = vec![
        vec![],
        vec![KeyAttributes::Commandline],
        vec![KeyAttributes::FileType],
        vec![KeyAttributes::Commandline, KeyAttributes::FileType],
    ];
    let keys: Vec<String> = subsets
        .iter()
        .map(|s| {
            build(s.as_slice())
                .get_key(&action_key_context, None)
                .unwrap()
        })
        .collect();
    for i in 0..keys.len() {
        for j in 0..keys.len() {
            if i != j {
                assert_ne!(keys[i], keys[j], "{:?} vs {:?}", subsets[i], subsets[j]);
            }
        }
        assert_eq!(
            build(subsets[i].as_slice())
                .get_key(&action_key_context, None)
                .unwrap(),
            keys[i]
        );
    }
}
