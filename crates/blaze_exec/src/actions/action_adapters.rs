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
    // Java: com/google/devtools/build/lib/actions/ArtifactRoot.java:238 ArtifactRoot.isExternal
    fn is_root_external(&self) -> bool {
        self.get_root().is_external()
    }
    // Java: com/google/devtools/build/lib/actions/Artifact.java:437 Artifact.getOwner
    fn get_owner(&self) -> Option<blaze_cmdline::Label> {
        BlazeArtifact::get_owner(self)
    }
    // Java: com/google/devtools/build/lib/actions/Artifact.java:636 Artifact.getRunfilesPath
    fn get_runfiles_path(&self) -> PathFragment {
        BlazeArtifact::get_runfiles_path(self)
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
    }
}

/// The execution platform of `action` as the execution layer's [`PlatformInfo`].
pub fn convert_platform(action: &dyn Action) -> Option<PlatformInfo> {
    action.get_execution_platform().map(|p| PlatformInfo {
        label: p.label().clone(),
        exec_properties: p.exec_properties(),
        // PORT: remoteExecutionProperties is only set by remote-execution platforms.
        remote_execution_properties: String::new(),
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
            // Java: virtual dispatch; of the ported actions only `TestRunnerAction` overrides
            // `mayModifySpawnOutputsAfterExecution` (JavaCompileAction and
            // JavaHeaderCompileAction, the others, are not ported).
            may_modify_spawn_outputs_after_execution: action
                .as_any()
                .downcast_ref::<crate::analysis::test::test_runner_action::TestRunnerAction>()
                .is_some_and(|a| a.may_modify_spawn_outputs_after_execution()),
        })
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
