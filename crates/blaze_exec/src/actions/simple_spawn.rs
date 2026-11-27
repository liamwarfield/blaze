//! Port of `lib/actions/SimpleSpawn.java`.

use std::collections::HashSet;
use std::fmt;
use std::sync::{Arc, OnceLock};

use blaze_collect::{NestedSet, NestedSetBuilder, Order};
use indexmap::IndexMap;

use super::action_execution_metadata::{ActionExecutionMetadata, PlatformInfo};
use super::action_input::ActionInputRef;
use super::exec_exception::ExecException;
use super::path_mapper::PathMapper;
use super::resource_set::{ResourceSet, ResourceSetOrBuilder};
use super::spawn::Spawn;

/// Immutable implementation of a [`Spawn`].
// Java: com/google/devtools/build/lib/actions/SimpleSpawn.java:36 SimpleSpawn
pub struct SimpleSpawn {
    owner: Arc<dyn ActionExecutionMetadata>,
    arguments: Vec<String>,
    environment: IndexMap<String, String>,
    execution_info: IndexMap<String, String>,
    inputs: NestedSet<ActionInputRef>,
    tools: NestedSet<ActionInputRef>,
    outputs: Vec<ActionInputRef>,
    mandatory_outputs: Option<HashSet<ActionInputRef>>,
    path_mapper: PathMapper,
    local_resources: Arc<dyn ResourceSetOrBuilder>,
    local_resources_cached: OnceLock<ResourceSet>,
}

impl SimpleSpawn {
    /// The full constructor.
    // Java: com/google/devtools/build/lib/actions/SimpleSpawn.java:50 SimpleSpawn.<init>
    // Java: com/google/devtools/build/lib/actions/BaseSpawn.java:38 BaseSpawn.BaseSpawn
    pub fn new(
        owner: Arc<dyn ActionExecutionMetadata>,
        arguments: Vec<String>,
        environment: IndexMap<String, String>,
        execution_info: IndexMap<String, String>,
        inputs: NestedSet<ActionInputRef>,
        tools: NestedSet<ActionInputRef>,
        outputs: Vec<ActionInputRef>,
        mandatory_outputs: Option<HashSet<ActionInputRef>>,
        local_resources: Arc<dyn ResourceSetOrBuilder>,
        path_mapper: PathMapper,
    ) -> SimpleSpawn {
        SimpleSpawn {
            owner,
            arguments,
            environment,
            execution_info,
            inputs,
            tools,
            outputs,
            mandatory_outputs,
            path_mapper,
            local_resources,
            local_resources_cached: OnceLock::new(),
        }
    }

    /// The constructor without tools and mandatory outputs.
    // Java: com/google/devtools/build/lib/actions/SimpleSpawn.java:100 SimpleSpawn.<init>
    pub fn new_simple(
        owner: Arc<dyn ActionExecutionMetadata>,
        arguments: Vec<String>,
        environment: IndexMap<String, String>,
        execution_info: IndexMap<String, String>,
        inputs: NestedSet<ActionInputRef>,
        outputs: Vec<ActionInputRef>,
        local_resources: Arc<dyn ResourceSetOrBuilder>,
    ) -> SimpleSpawn {
        SimpleSpawn::new(
            owner,
            arguments,
            environment,
            execution_info,
            inputs,
            NestedSetBuilder::empty_set(Order::STABLE_ORDER),
            outputs,
            None,
            local_resources,
            PathMapper::NOOP,
        )
    }
}

// `BaseSpawn` (and `SpawnAction.ActionSpawn`, which extends it) is this `SimpleSpawn`, built by
// the execution layer over the action's arguments, environment, execution info, inputs, tools and
// outputs; its methods are the ones below.
impl Spawn for SimpleSpawn {
    // Java: com/google/devtools/build/lib/actions/SimpleSpawn.java:115 SimpleSpawn.getExecutionInfo
    // Java: com/google/devtools/build/lib/actions/BaseSpawn.java:51 BaseSpawn.getExecutionInfo
    fn get_execution_info(&self) -> &IndexMap<String, String> {
        &self.execution_info
    }
    // Java: com/google/devtools/build/lib/actions/SimpleSpawn.java:120 SimpleSpawn.getArguments
    // Java: com/google/devtools/build/lib/actions/BaseSpawn.java:56 BaseSpawn.getArguments
    fn get_arguments(&self) -> &[String] {
        &self.arguments
    }
    // Java: com/google/devtools/build/lib/actions/SimpleSpawn.java:125 SimpleSpawn.getEnvironment
    // Java: com/google/devtools/build/lib/actions/BaseSpawn.java:63 BaseSpawn.getEnvironment
    fn get_environment(&self) -> &IndexMap<String, String> {
        &self.environment
    }
    // Java: com/google/devtools/build/lib/actions/SimpleSpawn.java:135 SimpleSpawn.getToolFiles
    // Java: com/google/devtools/build/lib/actions/BaseSpawn.java:68 BaseSpawn.getToolFiles
    fn get_tool_files(&self) -> &NestedSet<ActionInputRef> {
        &self.tools
    }
    // Java: com/google/devtools/build/lib/actions/SimpleSpawn.java:130 SimpleSpawn.getInputFiles
    // Java: com/google/devtools/build/lib/actions/BaseSpawn.java:73 BaseSpawn.getInputFiles
    fn get_input_files(&self) -> &NestedSet<ActionInputRef> {
        &self.inputs
    }
    // Java: com/google/devtools/build/lib/actions/SimpleSpawn.java:140 SimpleSpawn.getOutputFiles
    // Java: com/google/devtools/build/lib/actions/BaseSpawn.java:78 BaseSpawn.getOutputFiles
    fn get_output_files(&self) -> &[ActionInputRef] {
        &self.outputs
    }
    // Java: com/google/devtools/build/lib/actions/SimpleSpawn.java:145 SimpleSpawn.isMandatoryOutput
    fn is_mandatory_output(&self, output: &ActionInputRef) -> bool {
        match &self.mandatory_outputs {
            None => true,
            Some(m) => m.contains(output),
        }
    }
    // Java: com/google/devtools/build/lib/actions/SimpleSpawn.java:150 SimpleSpawn.getResourceOwner
    // Java: com/google/devtools/build/lib/actions/BaseSpawn.java:83 BaseSpawn.getResourceOwner
    fn get_resource_owner(&self) -> &dyn ActionExecutionMetadata {
        &*self.owner
    }
    // Java: com/google/devtools/build/lib/actions/SimpleSpawn.java:157 SimpleSpawn.getLocalResources
    // Java: com/google/devtools/build/lib/actions/BaseSpawn.java:88 BaseSpawn.getLocalResources
    fn get_local_resources(&self) -> Result<ResourceSet, ExecException> {
        if let Some(result) = self.local_resources_cached.get() {
            return Ok(result.clone());
        }
        // Not expected to be called concurrently, and an idempotent computation if it is.
        let result = self.local_resources.build_local_resources(
            self.inputs.memoized_flatten_and_get_size(),
            self.get_execution_info(),
            &self.get_combined_exec_properties(),
        )?;
        Ok(self.local_resources_cached.get_or_init(|| result).clone())
    }
    // Java: com/google/devtools/build/lib/actions/SimpleSpawn.java:176 SimpleSpawn.getMnemonic
    // Java: com/google/devtools/build/lib/actions/BaseSpawn.java:104 BaseSpawn.getMnemonic
    fn get_mnemonic(&self) -> String {
        self.owner.get_mnemonic()
    }
    // Java: com/google/devtools/build/lib/actions/SimpleSpawn.java:181 SimpleSpawn.getExecutionPlatform
    // Java: com/google/devtools/build/lib/actions/BaseSpawn.java:109 BaseSpawn.getExecutionPlatform
    fn get_execution_platform(&self) -> Option<&PlatformInfo> {
        self.owner.get_execution_platform()
    }
    // Java: com/google/devtools/build/lib/actions/SimpleSpawn.java:171 SimpleSpawn.getPathMapper
    fn get_path_mapper(&self) -> &PathMapper {
        &self.path_mapper
    }
    fn get_simple_class_name(&self) -> &'static str {
        "SimpleSpawn"
    }
}

impl fmt::Debug for SimpleSpawn {
    // Java: com/google/devtools/build/lib/actions/SimpleSpawn.java:188 SimpleSpawn.toString
    // Java: com/google/devtools/build/lib/actions/BaseSpawn.java:115 BaseSpawn.toString
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&super::spawns::pretty_print(self))
    }
}
