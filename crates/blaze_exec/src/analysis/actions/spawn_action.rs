//! Execution of `lib/analysis/actions/SpawnAction.java` (with `SpawnAction.ActionSpawn`) and of
//! `lib/analysis/actions/StarlarkAction.java` (`afterExecute`).

use std::sync::Arc;

use blaze_actions::actions::action::Action;
use blaze_actions::actions::command_line_limits::CommandLineLimits;
use blaze_actions::analysis::actions::path_mappers;
use blaze_actions::analysis::actions::spawn_action::{
    ResourceSet as AnalysisResourceSet, SpawnAction,
};
use blaze_collect::command_line_item::{CommandLineExpansionException, MapFnError};
use blaze_collect::{NestedSetBuilder, Order};
use blaze_protos::failure_details::{self, failure_detail::Category};
use indexmap::IndexMap;

use super::{KeyMetadataAdapter, failure_detail, to_action_input_set};
use crate::actions::action_adapters::{ActionMetadataSnapshot, artifact_input};
use crate::actions::{
    ActionExecutionException, ActionInputRef, ActionResult, ExecException, ExecutableAction,
    PathMapper, ResourceSet, SimpleSpawn, SpawnResult,
};
use crate::exec::action_execution_context::ActionExecutionContext;
use crate::exec::spawn_strategy_resolver::SpawnStrategyResolver;

/// The checked exceptions of `getSpawn`: `CommandLineExpansionException`,
/// `InterruptedException`.
#[derive(Debug)]
pub enum GetSpawnError {
    CommandLineExpansion(CommandLineExpansionException),
    Interrupted,
}

impl From<MapFnError> for GetSpawnError {
    fn from(e: MapFnError) -> Self {
        match e {
            MapFnError::CommandLineExpansion(e) => GetSpawnError::CommandLineExpansion(e),
            MapFnError::Interrupted(_) => GetSpawnError::Interrupted,
        }
    }
}

impl From<CommandLineExpansionException> for GetSpawnError {
    fn from(e: CommandLineExpansionException) -> Self {
        GetSpawnError::CommandLineExpansion(e)
    }
}

/// The `ResourceSetOrBuilder` of the action as the execution layer's `ResourceSet`.
///
/// PORT: `blaze_actions` keeps the resource set as plain data; Java builds it with
/// `ResourceSet.create(ImmutableMap.of(MEMORY, memoryMb, CPU, cpu, ...extra), localTestCount)`,
/// i.e. memory, cpu, then the extra resources in order.
fn to_resource_set(r: &AnalysisResourceSet) -> ResourceSet {
    let mut resources = IndexMap::new();
    resources.insert(ResourceSet::MEMORY.to_string(), r.memory_mb);
    resources.insert(ResourceSet::CPU.to_string(), r.cpu_usage);
    for (k, v) in &r.extra_resource_usage {
        resources.insert(k.clone(), *v);
    }
    ResourceSet::create_from_map(resources, r.local_test_count)
}

/// `getSpawn(actionExecutionContext)`: `getSpawn(ctx, ctx.getClientEnv(), reportOutputs=true)`.
// Java: com/google/devtools/build/lib/analysis/actions/SpawnAction.java:352 SpawnAction.getSpawn
pub fn get_spawn(
    action: &SpawnAction,
    action_execution_context: &dyn ActionExecutionContext,
) -> Result<SimpleSpawn, GetSpawnError> {
    get_spawn_with_env(
        action,
        action_execution_context,
        action_execution_context.get_client_env(),
        /* report_outputs= */ true,
    )
}

/// Return a spawn that is representative of the command that this Action will execute in the
/// given client environment.
// Java: com/google/devtools/build/lib/analysis/actions/SpawnAction.java:362 SpawnAction.getSpawn
pub fn get_spawn_with_env(
    action: &SpawnAction,
    action_execution_context: &dyn ActionExecutionContext,
    client_env: &IndexMap<String, String>,
    report_outputs: bool,
) -> Result<SimpleSpawn, GetSpawnError> {
    let metadata = KeyMetadataAdapter(action_execution_context.get_input_metadata_provider());
    let path_mapper = path_mappers::create(
        action,
        action.get_output_paths_mode(),
        action.is_starlark_action(),
        Some(&metadata),
    );
    let expanded_command_lines = action.get_command_lines().expand(
        Some(&metadata),
        action.get_primary_output().get_exec_path(),
        path_mapper.as_ref(),
        &get_command_line_limits(action),
    )?;

    // PORT: `path_mappers::create` returns a non-noop mapper only for STRIP, which is unported
    // there; the spawn's mapper is therefore PathMapper.NOOP.
    new_action_spawn(
        expanded_command_lines.arguments().to_vec(),
        action,
        client_env,
        expanded_command_lines
            .get_param_files()
            .iter()
            .map(|p| ActionInputRef::new(p.clone()))
            .collect(),
        report_outputs,
        PathMapper::NOOP,
    )
}

// Java: com/google/devtools/build/lib/analysis/actions/SpawnAction.java:391 SpawnAction.getCommandLineLimits
fn get_command_line_limits(action: &SpawnAction) -> CommandLineLimits {
    CommandLineLimits::new(
        action
            .get_owner()
            .get_build_configuration_info()
            .command_line_limits_max_length as i64,
    )
}

/// `new ActionSpawn(arguments, parent, clientEnv, parent.getInputs(), additionalInputs,
/// reportOutputs, pathMapper)`: a `BaseSpawn` tied to the action.
// Java: com/google/devtools/build/lib/analysis/actions/SpawnAction.java:538 SpawnAction.ActionSpawn.<init>
fn new_action_spawn(
    arguments: Vec<String>,
    parent: &SpawnAction,
    client_env: &IndexMap<String, String>,
    additional_inputs: Vec<ActionInputRef>,
    report_outputs: bool,
    path_mapper: PathMapper,
) -> Result<SimpleSpawn, GetSpawnError> {
    let mut inputs = NestedSetBuilder::<ActionInputRef>::new_builder(Order::STABLE_ORDER);
    inputs.add_transitive(&to_action_input_set(&parent.get_inputs()));
    inputs.add_all(additional_inputs);
    let effective_environment = parent.get_effective_environment(client_env)?;
    // BaseSpawn.getOutputFiles: parent.getOutputs(); ActionSpawn: empty unless reportOutputs.
    let outputs: Vec<ActionInputRef> = if report_outputs {
        parent.get_outputs().iter().map(artifact_input).collect()
    } else {
        Vec::new()
    };
    Ok(SimpleSpawn::new(
        ActionMetadataSnapshot::of(parent),
        arguments,
        effective_environment,
        parent.get_execution_info(),
        inputs.build(),
        // BaseSpawn.getToolFiles: parent.getTools().
        to_action_input_set(&parent.get_tools()),
        outputs,
        None,
        Arc::new(to_resource_set(parent.get_resource_set_or_builder())),
        path_mapper,
    ))
}

// Java: com/google/devtools/build/lib/analysis/actions/SpawnAction.java:299 SpawnAction.createCommandLineException
fn create_command_line_exception(
    action: &SpawnAction,
    e: &CommandLineExpansionException,
) -> ActionExecutionException {
    let detail = failure_detail(
        &e.message,
        Category::Spawn(failure_details::Spawn {
            code: failure_details::spawn::Code::CommandLineExpansionFailure as i32,
            ..Default::default()
        }),
    );
    ActionExecutionException::with_cause(
        e.message.clone(),
        &e.message,
        Some(action),
        /* catastrophe= */ false,
        Some(detail),
    )
}

/// `afterExecute(ctx, spawnResults, pathMapper)`: a no-op for `SpawnAction` and `StarlarkAction`;
/// `EnhancedStarlarkAction` prunes the inputs with its unused inputs list.
// Java: com/google/devtools/build/lib/analysis/actions/SpawnAction.java:271 SpawnAction.afterExecute
// Java: com/google/devtools/build/lib/analysis/actions/StarlarkAction.java:403 EnhancedStarlarkAction.afterExecute
fn after_execute(
    action: &SpawnAction,
    _ctx: &dyn ActionExecutionContext,
    _spawn_results: &[SpawnResult],
) -> Result<(), ExecException> {
    if action.get_unused_inputs_list().is_none() {
        return Ok(());
    }
    // PORT: the pruned inputs are `HashMap.values()` of the used inputs keyed by mapped exec path
    // (Java hash order) and the action's `updateInputs` needs `blaze_actions`' private
    // `inputsDiscovered` flag; not ported.
    blaze_util::unported!(
        "EnhancedStarlarkAction.afterExecute",
        "unused_inputs_list (blaze_actions does not expose updateInputs/setInputsDiscovered)"
    )
}

impl ExecutableAction for SpawnAction {
    // Java: com/google/devtools/build/lib/analysis/actions/SpawnAction.java:278 SpawnAction.execute
    fn execute(
        &self,
        ctx: &dyn ActionExecutionContext,
    ) -> Result<ActionResult, ActionExecutionException> {
        // beforeExecute(actionExecutionContext): no-op for SpawnAction and StarlarkAction.
        let spawn = match get_spawn(self, ctx) {
            Ok(s) => s,
            Err(GetSpawnError::CommandLineExpansion(e)) => {
                return Err(create_command_line_exception(self, &e));
            }
            Err(GetSpawnError::Interrupted) => {
                return Err(ActionExecutionException::from_exec_exception(
                    &ExecException::Interrupted,
                    None,
                    self,
                ));
            }
        };
        let resolver = crate::exec::action_execution_context::get_context::<SpawnStrategyResolver>(
            ctx,
        )
        .unwrap_or_else(|| {
            panic!("NullPointerException: getContext(SpawnStrategyResolver.class) returned null")
        });
        let result = resolver
            .exec(&spawn, ctx)
            .and_then(|result| {
                after_execute(self, ctx, &result)?;
                Ok(result)
            })
            .map_err(|e| match &e {
                ExecException::Spawn(spawn_exec_exception) => {
                    to_action_execution_exception(spawn_exec_exception, self)
                }
                _ => ActionExecutionException::from_exec_exception(&e, None, self),
            })?;
        Ok(ActionResult::create(result))
    }
}

/// `SpawnExecException.toActionExecutionException(action)`.
///
/// PORT: Java returns a `SpawnActionExecutionException` that also carries the failed
/// `SpawnResult`; the execution layer's `ActionExecutionException` has no field for it.
// Java: com/google/devtools/build/lib/exec/SpawnExecException.java:73 SpawnExecException.toActionExecutionException
pub fn to_action_execution_exception(
    e: &crate::actions::SpawnExecException,
    action: &dyn Action,
) -> ActionExecutionException {
    let message = e.get_message_for_action_execution_exception();
    // getFailureDetail(action.describe() + " failed: " + message): checkNotNull(result.failureDetail())
    let code = e
        .get_spawn_result()
        .failure_detail()
        .cloned()
        .unwrap_or_else(|| panic!("NullPointerException: {}", e.message));
    // new SpawnActionExecutionException(this, message, action, code, getSpawnResult()):
    // super(message, e, action, e.isCatastrophic(), code)
    ActionExecutionException::with_cause(
        message,
        &e.message,
        Some(action),
        e.catastrophe,
        Some(code),
    )
}
