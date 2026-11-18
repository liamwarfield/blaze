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

impl Spawn for SimpleSpawn {
    fn get_execution_info(&self) -> &IndexMap<String, String> {
        &self.execution_info
    }
    fn get_arguments(&self) -> &[String] {
        &self.arguments
    }
    fn get_environment(&self) -> &IndexMap<String, String> {
        &self.environment
    }
    fn get_tool_files(&self) -> &NestedSet<ActionInputRef> {
        &self.tools
    }
    fn get_input_files(&self) -> &NestedSet<ActionInputRef> {
        &self.inputs
    }
    fn get_output_files(&self) -> &[ActionInputRef] {
        &self.outputs
    }
    fn is_mandatory_output(&self, output: &ActionInputRef) -> bool {
        match &self.mandatory_outputs {
            None => true,
            Some(m) => m.contains(output),
        }
    }
    fn get_resource_owner(&self) -> &dyn ActionExecutionMetadata {
        &*self.owner
    }
    // Java: com/google/devtools/build/lib/actions/SimpleSpawn.java:157 SimpleSpawn.getLocalResources
    fn get_local_resources(&self) -> Result<ResourceSet, ExecException> {
        Ok(self
            .local_resources_cached
            .get_or_init(|| {
                self.local_resources.build_local_resources(
                    self.inputs.memoized_flatten_and_get_size(),
                    self.get_execution_info(),
                    &self.get_combined_exec_properties(),
                )
            })
            .clone())
    }
    fn get_mnemonic(&self) -> String {
        self.owner.get_mnemonic()
    }
    fn get_execution_platform(&self) -> Option<&PlatformInfo> {
        self.owner.get_execution_platform()
    }
    fn get_path_mapper(&self) -> &PathMapper {
        &self.path_mapper
    }
    fn get_simple_class_name(&self) -> &'static str {
        "SimpleSpawn"
    }
}

impl fmt::Debug for SimpleSpawn {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&super::spawns::pretty_print(self))
    }
}
