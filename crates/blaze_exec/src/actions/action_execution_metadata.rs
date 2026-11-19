//! `ActionExecutionMetadata`, `ActionOwner` and `PlatformInfo` as seen by spawn execution.
//! Seam with `blaze_actions` (actions implement [`ActionExecutionMetadata`]).

use std::fmt;

use blaze_cmdline::Label;
use indexmap::IndexMap;

use super::action_input::ActionInputRef;

/// The parts of `PlatformInfo` the execution layer reads.
// Java: com/google/devtools/build/lib/analysis/platform/PlatformInfo.java PlatformInfo
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlatformInfo {
    /// `label()`.
    pub label: Label,
    /// `execProperties()`.
    pub exec_properties: IndexMap<String, String>,
    /// `remoteExecutionProperties()`.
    pub remote_execution_properties: String,
}

/// The parts of `ActionOwner` the execution layer reads.
// Java: com/google/devtools/build/lib/actions/ActionOwner.java ActionOwner
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ActionOwner {
    /// `getLabel()`.
    pub label: Option<Label>,
    /// `getConfigurationChecksum()`.
    pub configuration_checksum: Option<String>,
    /// `getDescription()` (target description, e.g. `//:brotli`).
    pub description: Option<String>,
    /// `getExecProperties()`.
    pub exec_properties: IndexMap<String, String>,
}

impl ActionOwner {
    /// `getLabel()`.
    pub fn get_label(&self) -> Option<&Label> {
        self.label.as_ref()
    }
    /// `getConfigurationChecksum()`.
    pub fn get_configuration_checksum(&self) -> Option<&str> {
        self.configuration_checksum.as_deref()
    }
    /// `getDescription()`.
    pub fn get_description(&self) -> Option<&str> {
        self.description.as_deref()
    }
    /// `getExecProperties()`.
    pub fn get_exec_properties(&self) -> &IndexMap<String, String> {
        &self.exec_properties
    }
}

/// Port of `ActionExecutionMetadata` (what `Spawn.getResourceOwner()` returns).
// Java: com/google/devtools/build/lib/actions/ActionExecutionMetadata.java ActionExecutionMetadata
pub trait ActionExecutionMetadata: Send + Sync + fmt::Debug {
    /// `getMnemonic()`.
    fn get_mnemonic(&self) -> String;
    /// `getProgressMessage()`.
    fn get_progress_message(&self) -> Option<String>;
    /// `getOwner()`.
    fn get_owner(&self) -> &ActionOwner;
    /// `getPrimaryOutput()`.
    fn get_primary_output(&self) -> ActionInputRef;
    /// `getExecutionPlatform()`.
    fn get_execution_platform(&self) -> Option<&PlatformInfo>;
    /// `prettyPrint()`.
    fn pretty_print(&self) -> String;
    /// `describe()`: progress message or `"Executing <mnemonic>"`-style text.
    fn describe(&self) -> String {
        self.get_progress_message()
            .unwrap_or_else(|| format!("Executing {}", self.get_mnemonic()))
    }
    /// Returns true if the action may modify spawn outputs after the spawn has executed.
    ///
    /// If this returns true, any kind of spawn output caching or reuse needs to happen
    /// synchronously directly after the spawn execution.
    // Java: com/google/devtools/build/lib/actions/ActionExecutionMetadata.java:125 ActionExecutionMetadata.mayModifySpawnOutputsAfterExecution
    fn may_modify_spawn_outputs_after_execution(&self) -> bool {
        false
    }
}

/// A fixed `ActionExecutionMetadata` (Java tests use `FakeOwner`; the replay harness uses it to
/// stand in for the action that produced a recorded spawn).
// Java: com/google/devtools/build/lib/exec/util/FakeOwner.java FakeOwner
#[derive(Clone, Debug)]
pub struct FakeOwner {
    pub mnemonic: String,
    pub progress_message: Option<String>,
    pub owner: ActionOwner,
    pub primary_output: ActionInputRef,
    pub platform: Option<PlatformInfo>,
    /// `mayModifySpawnOutputsAfterExecution()` of the action this owner stands in for (Java's
    /// `FakeOwner` inherits the default, false; the replay harness sets it for test actions).
    pub may_modify_spawn_outputs_after_execution: bool,
}

impl FakeOwner {
    /// `new FakeOwner(mnemonic, progressMessage, ownerLabel)`.
    pub fn new(mnemonic: &str, progress_message: Option<&str>, owner_label: Option<Label>) -> Self {
        FakeOwner {
            mnemonic: mnemonic.to_string(),
            progress_message: progress_message.map(str::to_string),
            owner: ActionOwner {
                description: owner_label.as_ref().map(|l| l.to_string()),
                label: owner_label,
                configuration_checksum: None,
                exec_properties: IndexMap::new(),
            },
            primary_output: ActionInputRef::new(super::action_input::BasicActionInput::from_path(
                "/dummy/output",
            )),
            platform: None,
            may_modify_spawn_outputs_after_execution: false,
        }
    }
}

impl ActionExecutionMetadata for FakeOwner {
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
        format!("action '{}'", self.describe())
    }
    fn may_modify_spawn_outputs_after_execution(&self) -> bool {
        self.may_modify_spawn_outputs_after_execution
    }
}
