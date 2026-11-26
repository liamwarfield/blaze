//! The `getExtraActionInfo` tests of `analysis/actions/SpawnActionTest.java`
//! (`testExtraActionInfo`, `testExtraActionInfoEnvironmentVariables`).

use std::any::Any;
use std::fmt;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::Arc;

use blaze_actions::actions::action_lookup::ActionLookupKey;
use blaze_actions::actions::command_line;
use blaze_actions::actions::command_lines::CommandLines;
use blaze_actions::actions::path_mapper::OutputPathsMode;
use blaze_actions::analysis::actions::spawn_action::{ResourceSet, SpawnAction};
use blaze_actions::{
    Action, ActionEnvironment, ActionKeyContext, ActionLookupKeyRef, ActionOwner, Artifact,
    ArtifactOwner, ArtifactRoot, CommandAction, RootType,
};
use blaze_cmdline::Label;
use blaze_collect::{NestedSetBuilder, Order};
use blaze_events::location::Location;
use blaze_vfs::{DigestHashFunction, Path, PathFragment, Root, UnixFileSystem};
use indexmap::IndexMap;

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

/// `nullOwnerWithTargetConfig()`.
fn null_owner() -> ActionOwner {
    ActionOwner::create_dummy(
        Some(Label::parse_canonical("//null/action:owner").unwrap()),
        Location::new("dummy-file", 0, 0),
        "dummy-kind",
        "k8-fastbuild",
        "dummy-configuration",
        None,
        false,
        None,
        Vec::new(),
        IndexMap::new(),
    )
}

/// `getSourceArtifact("pkg/welcome.txt")`.
fn welcome_artifact() -> Artifact {
    let fs = UnixFileSystem::create(DigestHashFunction::Sha256);
    let root = ArtifactRoot::as_source_root(Root::from_path(Path::create(
        PathFragment::create("/workspace"),
        &fs,
    )));
    Artifact::new_source_artifact(
        root,
        PathFragment::create("pkg/welcome.txt"),
        ArtifactOwner::NullOwner,
    )
}

/// `getBinArtifactWithNoOwner("dir/destination.txt")`.
fn destination_artifact() -> Artifact {
    let fs = UnixFileSystem::create(DigestHashFunction::Sha256);
    let exec_root = Path::create(PathFragment::create("/execroot/_main"), &fs);
    let root = ArtifactRoot::as_derived_root(
        &exec_root,
        RootType::OUTPUT,
        &["bazel-out", "k8-fastbuild", "bin"],
    );
    let exec_path = root.get_exec_path().get_relative_str("dir/destination.txt");
    Artifact::create_derived_artifact(root, exec_path, ActionLookupKeyRef::new(NullArtifactOwner))
}

/// `createCopyFromWelcomeToDestination(environmentVariables)`: `builder().addInput(welcome)
/// .addOutput(destination).setExecutionInfo({"local": ""}).setExecutable("/bin/cp")
/// .setProgressMessage("hi, mom!").setMnemonic("Dummy").setEnvironment(env)
/// .addCommandLine(CommandLine.of([welcome, destination]))`.
fn create_copy_from_welcome_to_destination(environment_variables: &[(&str, &str)]) -> SpawnAction {
    let welcome = welcome_artifact();
    let destination = destination_artifact();
    let mut execution_info = IndexMap::new();
    execution_info.insert("local".to_owned(), String::new());
    let env: IndexMap<String, String> = environment_variables
        .iter()
        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
        .collect();
    SpawnAction::new(
        null_owner(),
        NestedSetBuilder::empty_set(Order::STABLE_ORDER),
        NestedSetBuilder::create(Order::STABLE_ORDER, [welcome.clone()]),
        [destination.clone()],
        ResourceSet::default_resource_set(),
        CommandLines::of(command_line::of(vec![
            "/bin/cp".to_owned(),
            welcome.get_exec_path_string().to_owned(),
            destination.get_exec_path_string().to_owned(),
        ])),
        Arc::new(ActionEnvironment::create_fixed(env)),
        &execution_info,
        Some("hi, mom!".to_owned()),
        "Dummy",
        OutputPathsMode::OFF,
    )
}

#[test]
fn test_extra_action_info() {
    let action_key_context = ActionKeyContext::new();
    let action = create_copy_from_welcome_to_destination(&[]);
    let info = action.get_extra_action_info(&action_key_context).unwrap();
    assert_eq!(info.mnemonic.as_deref(), Some("Dummy"));

    let spawn_info = info
        .spawn_info
        .as_ref()
        .expect("hasExtension(SpawnInfo.spawnInfo)");

    assert_eq!(spawn_info.argument, action.get_arguments().unwrap());

    let input_paths: Vec<String> = action
        .get_inputs()
        .to_list()
        .iter()
        .map(|a| a.get_exec_path_string().to_owned())
        .collect();
    let output_paths: Vec<String> = action
        .get_outputs()
        .iter()
        .map(|a| a.get_exec_path_string().to_owned())
        .collect();

    assert_eq!(spawn_info.input_file, input_paths);
    assert_eq!(spawn_info.output_file, output_paths);
    let environment = action.get_incomplete_environment_for_testing();
    assert_eq!(spawn_info.variable.len(), environment.len());

    for variable in &spawn_info.variable {
        assert_eq!(environment.get(&variable.name), Some(&variable.value));
    }
}

/// Test that environment variables are not escaped or quoted.
#[test]
fn test_extra_action_info_environment_variables() {
    let env = [
        ("P1", "simple"),
        ("P2", "spaces are not escaped"),
        ("P3", ":"),
        ("P4", ""),
        ("NONSENSE VARIABLE", "value"),
    ];

    let spawn_info = create_copy_from_welcome_to_destination(&env)
        .get_extra_action_info(&ActionKeyContext::new())
        .unwrap()
        .spawn_info
        .unwrap();
    assert_eq!(env.len(), spawn_info.variable.len());
    for variable in &spawn_info.variable {
        assert!(
            env.contains(&(variable.name.as_str(), variable.value.as_str())),
            "{variable:?}"
        );
    }
}
