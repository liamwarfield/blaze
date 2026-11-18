//! Port of `lib/actions/Spawn.java`.

use std::fmt;

use blaze_cmdline::Label;
use blaze_collect::NestedSet;
use indexmap::IndexMap;

use super::action_execution_metadata::{ActionExecutionMetadata, PlatformInfo};
use super::action_input::ActionInputRef;
use super::exec_exception::ExecException;
use super::path_mapper::PathMapper;
use super::resource_set::ResourceSet;

/// An object-oriented representation of a process to be executed, including its arguments,
/// environment, inputs and outputs.
///
/// Seam: `blaze_actions`' spawn-producing actions (`SpawnAction`, `CppCompileAction`, ...) hand
/// the strategies a value implementing this trait; [`super::simple_spawn::SimpleSpawn`] is the
/// usual implementation.
// Java: com/google/devtools/build/lib/actions/Spawn.java:33 Spawn
pub trait Spawn: Send + Sync + fmt::Debug {
    /// `getExecutionInfo()`.
    fn get_execution_info(&self) -> &IndexMap<String, String>;
    /// `getArguments()`.
    fn get_arguments(&self) -> &[String];
    /// `getEnvironment()`.
    fn get_environment(&self) -> &IndexMap<String, String>;
    /// `getToolFiles()`.
    fn get_tool_files(&self) -> &NestedSet<ActionInputRef>;
    /// `getInputFiles()`.
    fn get_input_files(&self) -> &NestedSet<ActionInputRef>;
    /// `getOutputFiles()`.
    fn get_output_files(&self) -> &[ActionInputRef];
    /// `isMandatoryOutput(ActionInput)`.
    fn is_mandatory_output(&self, _output: &ActionInputRef) -> bool {
        true
    }
    /// `getResourceOwner()`.
    fn get_resource_owner(&self) -> &dyn ActionExecutionMetadata;
    /// `getLocalResources()`.
    fn get_local_resources(&self) -> Result<ResourceSet, ExecException>;
    /// `getMnemonic()`.
    fn get_mnemonic(&self) -> String;
    /// `getCombinedExecProperties()`.
    fn get_combined_exec_properties(&self) -> IndexMap<String, String> {
        self.get_resource_owner()
            .get_owner()
            .get_exec_properties()
            .clone()
    }
    /// `getExecutionPlatform()`.
    fn get_execution_platform(&self) -> Option<&PlatformInfo>;
    /// `getExecutionPlatformLabel()`.
    fn get_execution_platform_label(&self) -> Option<Label> {
        self.get_execution_platform().map(|p| p.label.clone())
    }
    /// `getConfigurationChecksum()`.
    fn get_configuration_checksum(&self) -> Option<String> {
        self.get_resource_owner()
            .get_owner()
            .get_configuration_checksum()
            .map(str::to_string)
    }
    /// `getTargetDescription()`.
    fn get_target_description(&self) -> Option<String> {
        self.get_resource_owner()
            .get_owner()
            .get_description()
            .map(str::to_string)
    }
    /// `getTargetLabel()`.
    fn get_target_label(&self) -> Option<Label> {
        self.get_resource_owner().get_owner().get_label().cloned()
    }
    /// `getPathMapper()`.
    fn get_path_mapper(&self) -> &PathMapper {
        &PathMapper::NOOP
    }
    /// `getClass().getSimpleName()` (for `Spawns.prettyPrint`).
    fn get_simple_class_name(&self) -> &'static str {
        "Spawn"
    }
}

/// `Spawn` as a `DescribableExecutionUnit` (for `CommandFailureUtils.describeCommandFailure`).
pub struct SpawnDescribable<'a>(pub &'a dyn Spawn);

impl blaze_shell::DescribableExecutionUnit for SpawnDescribable<'_> {
    fn get_arguments(&self) -> Vec<String> {
        self.0.get_arguments().to_vec()
    }
    fn get_environment(&self) -> Option<IndexMap<String, String>> {
        Some(self.0.get_environment().clone())
    }
    fn get_mnemonic(&self) -> String {
        self.0.get_mnemonic()
    }
    fn get_configuration_checksum(&self) -> Option<String> {
        self.0.get_configuration_checksum()
    }
    fn get_target_description(&self) -> Option<String> {
        self.0.get_target_description()
    }
    fn get_execution_platform_label(&self) -> Option<Label> {
        self.0.get_execution_platform_label()
    }
}
