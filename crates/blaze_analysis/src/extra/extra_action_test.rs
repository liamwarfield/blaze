//! Port of `analysis/extra/ExtraActionTest.java`.

use std::any::Any;
use std::fmt;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use blaze_actions::actions::action::{
    AbstractAction, Action, CommandLineError, ComputeKeyError, InputMetadataProvider,
};
use blaze_actions::actions::action_lookup::ActionLookupKey;
use blaze_actions::{
    ActionKeyContext, ActionLookupKeyRef, ActionOwner, ActionRef, Artifact, ArtifactRoot, RootType,
};
use blaze_cmdline::Label;
use blaze_collect::{NestedSet, NestedSetBuilder, Order};
use blaze_events::location::Location;
use blaze_protos::blaze::{ExtraActionInfo, JavaCompileInfo};
use blaze_util::Fingerprint;
use blaze_vfs::{DigestHashFunction, Path, PathFragment, UnixFileSystem};
use indexmap::IndexMap;

use super::extra_action::ExtraAction;
use super::extra_action_info_file_write_action::ExtraActionInfoFileWriteAction;

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

/// `ActionsTestUtil.NULL_ACTION_OWNER`.
fn null_action_owner() -> ActionOwner {
    ActionOwner::create_dummy(
        Some(Label::parse_canonical("//null/action:owner").unwrap()),
        Location::new("dummy-file", 0, 0),
        "dummy-kind",
        "dummy-configuration-mnemonic",
        "dummy-configuration",
        None,
        false,
        None,
        Vec::new(),
        IndexMap::new(),
    )
}

/// `ActionsTestUtil.createArtifact(root, path)` under `/out` (a derived root `out`).
fn out_artifact(name: &str) -> Artifact {
    let fs = UnixFileSystem::create(DigestHashFunction::Sha256);
    let exec_root = Path::create(PathFragment::create("/"), &fs);
    let root = ArtifactRoot::as_derived_root(&exec_root, RootType::OUTPUT, &["out"]);
    let exec_path = root.get_exec_path().get_relative_str(name);
    Artifact::create_derived_artifact(root, exec_path, ActionLookupKeyRef::new(NullArtifactOwner))
}

/// `NullAction` with an overridden `getExtraActionInfo` (Java's `SpecifiedInfoAction`), or a
/// mocked action that discovers inputs (`testUpdateInputsNotPassedToShadowedAction`).
#[derive(Debug)]
struct SpecifiedInfoAction {
    base: AbstractAction,
    info: ExtraActionInfo,
    discovers_inputs: bool,
    update_inputs_called: AtomicBool,
}

impl SpecifiedInfoAction {
    fn new(info: ExtraActionInfo, discovers_inputs: bool) -> SpecifiedInfoAction {
        SpecifiedInfoAction {
            base: AbstractAction::new(
                null_action_owner(),
                NestedSetBuilder::empty_set(Order::STABLE_ORDER),
                [out_artifact("null.out")],
            ),
            info,
            discovers_inputs,
            update_inputs_called: AtomicBool::new(false),
        }
    }
}

impl Action for SpecifiedInfoAction {
    fn abstract_action(&self) -> &AbstractAction {
        &self.base
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn get_mnemonic(&self) -> String {
        "Null".to_owned()
    }
    fn compute_key(
        &self,
        _action_key_context: &ActionKeyContext,
        _input_metadata_provider: Option<&dyn InputMetadataProvider>,
        _fp: &mut Fingerprint,
    ) -> Result<(), ComputeKeyError> {
        Ok(())
    }
    fn get_extra_action_info(
        &self,
        _action_key_context: &ActionKeyContext,
    ) -> Result<ExtraActionInfo, CommandLineError> {
        Ok(self.info.clone())
    }
    fn discovers_inputs(&self) -> bool {
        self.discovers_inputs
    }
    fn inputs_discovered(&self) -> bool {
        true
    }
    fn set_inputs_discovered(&self, _inputs_discovered: bool) {}
    fn update_inputs(&self, _inputs: NestedSet<Artifact>) {
        self.update_inputs_called.store(true, Ordering::SeqCst);
    }
}

#[test]
fn test_extra_action_info_affects_mnemonic() {
    let action_key_context = Arc::new(ActionKeyContext::new());
    let info_one = ExtraActionInfo {
        java_compile_info: Some(JavaCompileInfo {
            source_file: vec!["one".to_owned()],
            ..Default::default()
        }),
        ..Default::default()
    };
    let info_two = ExtraActionInfo {
        java_compile_info: Some(JavaCompileInfo {
            source_file: vec!["two".to_owned()],
            ..Default::default()
        }),
        ..Default::default()
    };

    let output = out_artifact("test.out");
    let action_one = ExtraActionInfoFileWriteAction::new(
        null_action_owner(),
        output.clone(),
        Arc::new(SpecifiedInfoAction::new(info_one, false)),
        action_key_context.clone(),
    );
    let action_two = ExtraActionInfoFileWriteAction::new(
        null_action_owner(),
        output,
        Arc::new(SpecifiedInfoAction::new(info_two, false)),
        action_key_context.clone(),
    );

    assert_ne!(
        action_one.get_key(&action_key_context, None).unwrap(),
        action_two.get_key(&action_key_context, None).unwrap()
    );
}

/// Regression test. The Spawn created for extra actions needs to pass the environment of the
/// extra action by getting the result of SpawnAction.getEnvironment() method instead of relying
/// on the default field value of BaseSpawn.environment.
#[test]
#[ignore = "needs a test executor with a fake SpawnStrategy (TestExecutorBuilder)"]
fn test_environment_passed_on_overwrite() {}

#[test]
fn test_update_inputs_not_passed_to_shadowed_action() {
    // PORT: Java's mocked shadowed action; source artifacts are derived ones here (the test is
    // about which action's inputs are updated).
    let extra_in = out_artifact("extra.in");
    let discovered_in = out_artifact("discovered.in");
    let shadowed = Arc::new(SpecifiedInfoAction::new(ExtraActionInfo::default(), true));
    let shadowed_action: ActionRef = shadowed.clone();
    let extra_action = ExtraAction::new(
        null_action_owner(),
        NestedSetBuilder::create(Order::STABLE_ORDER, [extra_in.clone()]),
        vec![out_artifact("test.out")],
        shadowed_action,
        false,
        blaze_actions::actions::command_line::of(Vec::new()),
        Arc::new(blaze_actions::actions::action_environment::EMPTY),
        IndexMap::new(),
        "Executing extra action bla bla".to_owned(),
        "bla bla".to_owned(),
    );
    extra_action.update_inputs(NestedSetBuilder::create(
        Order::STABLE_ORDER,
        [extra_in, discovered_in],
    ));
    assert!(!shadowed.update_inputs_called.load(Ordering::SeqCst));
}

#[test]
#[ignore = "needs action serialization (AutoCodec), not ported"]
fn test_serialization_round_trip_resets_inputs() {}
