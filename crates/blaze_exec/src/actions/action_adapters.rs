//! Adapters of the execution-layer traits ([`ActionInput`], [`Artifact`],
//! [`ActionExecutionMetadata`]) for `blaze_actions` types (see the seam notes in `mod.rs`).
//!
//! Not a port of a Java class: in Java, `Artifact` implements `ActionInput` and every `Action` is
//! an `ActionExecutionMetadata` directly.

use std::any::Any;
use std::sync::Arc;

use blaze_actions::actions::action::Action;
use blaze_actions::actions::artifact::Artifact as BlazeArtifact;
use blaze_actions::actions::command_lines::ParamFileActionInput;
use blaze_vfs::PathFragment;

use super::action_execution_metadata::{ActionExecutionMetadata, ActionOwner, PlatformInfo};
use super::action_input::{ActionInput, ActionInputRef, Artifact};
use super::virtual_action_input::VirtualActionInput;

// Java: com/google/devtools/build/lib/actions/Artifact.java:142 Artifact (implements ActionInput)
impl ActionInput for BlazeArtifact {
    fn get_exec_path(&self) -> &PathFragment {
        BlazeArtifact::get_exec_path(self)
    }
    fn is_directory(&self) -> bool {
        BlazeArtifact::is_directory(self)
    }
    fn is_symlink(&self) -> bool {
        BlazeArtifact::is_symlink(self)
    }
    fn as_artifact(&self) -> Option<&dyn Artifact> {
        Some(self)
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl Artifact for BlazeArtifact {
    fn is_source_artifact(&self) -> bool {
        BlazeArtifact::is_source_artifact(self)
    }
    fn is_tree_artifact(&self) -> bool {
        BlazeArtifact::is_tree_artifact(self)
    }
    fn is_runfiles_tree(&self) -> bool {
        BlazeArtifact::is_runfiles_tree(self)
    }
    fn is_fileset(&self) -> bool {
        BlazeArtifact::is_fileset(self)
    }
    fn is_constant_metadata(&self) -> bool {
        BlazeArtifact::is_constant_metadata(self)
    }
    fn is_child_of_declared_directory(&self) -> bool {
        BlazeArtifact::is_child_of_declared_directory(self)
    }
    fn get_parent(&self) -> Option<ActionInputRef> {
        BlazeArtifact::get_parent(self).map(|p| ActionInputRef::new(p.clone()))
    }
    fn get_parent_relative_path(&self) -> Option<PathFragment> {
        BlazeArtifact::get_parent_relative_path(self)
    }
    fn get_root_relative_path(&self) -> PathFragment {
        BlazeArtifact::get_root_relative_path(self)
    }
    fn get_root_exec_path(&self) -> PathFragment {
        self.get_root().get_exec_path().clone()
    }
    fn get_runfiles_path(&self) -> PathFragment {
        BlazeArtifact::get_runfiles_path(self)
    }
    fn is_root_external(&self) -> bool {
        self.get_root().is_external()
    }
}

/// `blaze_actions::Artifact` as an [`ActionInputRef`].
pub fn artifact_input(artifact: &BlazeArtifact) -> ActionInputRef {
    ActionInputRef::new(artifact.clone())
}

/// The `blaze_actions::Artifact` behind an [`ActionInputRef`], if it is one.
pub fn as_blaze_artifact(input: &ActionInputRef) -> Option<&BlazeArtifact> {
    input.0.as_any().downcast_ref::<BlazeArtifact>()
}

/// The [`TreeArtifactFactory`](super::action_cache_checker::TreeArtifactFactory) of a build whose
/// action inputs are `blaze_actions` artifacts: `TreeFileArtifact.createTreeOutput` and
/// `TreeArtifactValue.Builder`.
///
/// PORT: proxy metadata (only a non-default `ProxyMetadataFactory` creates proxies;
/// `LocalOutputService` and `RemoteOutputService` both give `NO_PROXIES`) is not ported and fails
/// loudly.
#[derive(Debug, Default, Clone, Copy)]
pub struct ArtifactTreeArtifactFactory;

impl super::action_cache_checker::TreeArtifactFactory for ArtifactTreeArtifactFactory {
    // Java: com/google/devtools/build/lib/actions/Artifact.java:1089 TreeFileArtifact.createTreeOutput
    fn create_tree_output(
        &self,
        parent: &ActionInputRef,
        parent_relative_path: &PathFragment,
    ) -> ActionInputRef {
        let parent = as_blaze_artifact(parent)
            .unwrap_or_else(|| panic!("ClassCastException: {parent} is not a SpecialArtifact"));
        artifact_input(&BlazeArtifact::create_tree_output(
            parent,
            parent_relative_path.clone(),
        ))
    }

    // Java: com/google/devtools/build/lib/actions/Artifact.java:966 ArchivedTreeArtifact.createForTree
    fn create_archived_tree_artifact(&self, parent: &ActionInputRef) -> ActionInputRef {
        let parent = as_blaze_artifact(parent)
            .unwrap_or_else(|| panic!("ClassCastException: {parent} is not a SpecialArtifact"));
        artifact_input(&BlazeArtifact::create_archived_tree_artifact_for_tree(
            parent,
        ))
    }

    fn build_tree_artifact_value(
        &self,
        _parent: &ActionInputRef,
        children: Vec<(ActionInputRef, super::FileArtifactValue)>,
        archived_representation: Option<(ActionInputRef, super::FileArtifactValue)>,
    ) -> super::TreeArtifactValue {
        // Java: com/google/devtools/build/lib/skyframe/TreeArtifactValue.java:686 TreeArtifactValue.Builder.build
        super::TreeArtifactValue::build(children, archived_representation, None)
    }

    fn construct_proxy_tree_metadata(
        &self,
        _parent: &ActionInputRef,
        _proxy_metadata_factory: &dyn super::action_cache_checker::ProxyMetadataFactory,
    ) -> Result<
        super::TreeArtifactValue,
        super::cache::output_metadata_store::IoOrInterruptedException,
    > {
        blaze_util::unported!("ActionCacheChecker.constructProxyTreeMetadata")
    }
}

// Java: com/google/devtools/build/lib/actions/CommandLines.java:349 ParamFileActionInput (implements VirtualActionInput)
impl ActionInput for ParamFileActionInput {
    fn get_exec_path(&self) -> &PathFragment {
        ParamFileActionInput::get_exec_path(self)
    }
    fn is_directory(&self) -> bool {
        false
    }
    fn is_symlink(&self) -> bool {
        false
    }
    fn as_virtual_action_input(&self) -> Option<&dyn VirtualActionInput> {
        Some(self)
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl VirtualActionInput for ParamFileActionInput {
    fn write_to(&self, out: &mut dyn std::io::Write) -> std::io::Result<()> {
        ParamFileActionInput::write_to(self, out)
    }
}

/// `blaze_actions::ActionOwner` as the execution layer's [`ActionOwner`].
pub fn convert_owner(owner: &blaze_actions::ActionOwner) -> ActionOwner {
    ActionOwner {
        label: owner.get_label().cloned(),
        configuration_checksum: Some(owner.get_configuration_checksum().to_string()),
        description: owner.get_description(),
        exec_properties: owner.get_exec_properties().clone(),
        target_kind: owner.get_target_kind().to_owned(),
        is_build_configuration_for_tool: owner.is_build_configuration_for_tool(),
    }
}

/// The execution platform of `action` as the execution layer's [`PlatformInfo`].
pub fn convert_platform(action: &dyn Action) -> Option<PlatformInfo> {
    action.get_execution_platform().map(|p| PlatformInfo {
        label: p.label().clone(),
        exec_properties: p.exec_properties(),
        // Java: com/google/devtools/build/lib/analysis/platform/PlatformInfo.java:130 PlatformInfo.remoteExecutionProperties
        remote_execution_properties: p.remote_execution_properties(),
        // Java: com/google/devtools/build/lib/analysis/constraints/ConstraintConstants.java:83 ConstraintConstants.getOsFromConstraintsOrHost
        // (`platformInfo.constraints().get(OS_CONSTRAINT_SETTING)`; the setting has no default).
        os_constraint: p.constraint_value(&blaze_cmdline::Label::parse_canonical_unchecked(
            "@platforms//os:os",
        )),
    })
}

/// An [`ActionExecutionMetadata`] view of a `blaze_actions` action (what `Spawn.getResourceOwner`
/// returns). It snapshots the action's metadata, since spawns need an owned `Arc`.
#[derive(Debug)]
pub struct ActionMetadataSnapshot {
    mnemonic: String,
    progress_message: Option<String>,
    owner: ActionOwner,
    primary_output: ActionInputRef,
    platform: Option<PlatformInfo>,
    pretty_print: String,
    describe: String,
    may_modify_spawn_outputs_after_execution: bool,
}

impl ActionMetadataSnapshot {
    pub fn of(action: &dyn Action) -> Arc<ActionMetadataSnapshot> {
        Arc::new(ActionMetadataSnapshot {
            mnemonic: action.get_mnemonic(),
            progress_message: action.get_progress_message(None),
            owner: convert_owner(action.get_owner()),
            primary_output: artifact_input(&action.get_primary_output()),
            platform: convert_platform(action),
            pretty_print: action.pretty_print(),
            describe: action.describe(),
            may_modify_spawn_outputs_after_execution: may_modify_spawn_outputs_after_execution(
                action,
            ),
        })
    }
}

/// `action.mayModifySpawnOutputsAfterExecution()`: `ActionExecutionMetadata`'s default (false),
/// overridden by `TestRunnerAction` (true). `JavaCompileAction` and `JavaHeaderCompileAction`
/// also override it in Java; they are not ported.
// Java: com/google/devtools/build/lib/actions/ActionExecutionMetadata.java:125 ActionExecutionMetadata.mayModifySpawnOutputsAfterExecution
fn may_modify_spawn_outputs_after_execution(action: &dyn Action) -> bool {
    match action
        .as_any()
        .downcast_ref::<crate::analysis::test::test_runner_action::TestRunnerAction>()
    {
        Some(test_action) => test_action.may_modify_spawn_outputs_after_execution(),
        None => false,
    }
}

impl ActionExecutionMetadata for ActionMetadataSnapshot {
    fn get_mnemonic(&self) -> String {
        self.mnemonic.clone()
    }
    fn get_progress_message(&self) -> Option<String> {
        self.progress_message.clone()
    }
    fn get_owner(&self) -> &ActionOwner {
        &self.owner
    }
    fn get_primary_output(&self) -> ActionInputRef {
        self.primary_output.clone()
    }
    fn get_execution_platform(&self) -> Option<&PlatformInfo> {
        self.platform.as_ref()
    }
    fn pretty_print(&self) -> String {
        self.pretty_print.clone()
    }
    fn describe(&self) -> String {
        self.describe.clone()
    }
    fn may_modify_spawn_outputs_after_execution(&self) -> bool {
        self.may_modify_spawn_outputs_after_execution
    }
}
