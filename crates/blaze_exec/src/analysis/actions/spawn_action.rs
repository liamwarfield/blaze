//! Execution of `lib/analysis/actions/SpawnAction.java` (with `SpawnAction.ActionSpawn`) and of
//! `lib/analysis/actions/StarlarkAction.java` (`afterExecute`).

use std::sync::Arc;

use blaze_actions::Artifact;
use blaze_actions::actions::action::Action;
use blaze_actions::actions::command_line_limits::CommandLineLimits;
use blaze_actions::analysis::actions::path_mappers;
use blaze_actions::analysis::actions::spawn_action::{
    ResourceSet as AnalysisResourceSet, ResourceSetBuilder, SpawnAction, SpawnActionClass,
};
use blaze_collect::command_line_item::{CommandLineExpansionException, MapFnError};
use blaze_collect::{NestedSet, NestedSetBuilder, Order};
use blaze_protos::failure_details::{self, failure_detail::Category};
use blaze_shell::util::os::OS;
use indexmap::IndexMap;

use super::{KeyMetadataAdapter, failure_detail, to_action_input_set};
use crate::actions::EnvironmentalExecException;
use crate::actions::action_adapters::{ActionMetadataSnapshot, artifact_input};
use crate::actions::{
    ActionExecutionException, ActionInputRef, ActionResult, ExecException, ExecutableAction,
    PathMapper, ResourceSet, ResourceSetOrBuilder, SimpleSpawn, SpawnResult, UserExecException,
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
        PathMapper::of(path_mapper),
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
    let effective_environment =
        parent.get_effective_environment(client_env, path_mapper.as_actions_path_mapper())?;
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
        match parent.get_resource_set_builder() {
            // Java BaseSpawn.getLocalResources -> buildLocalResources(OS.getCurrent(),
            // action.getInputs().memoizedFlattenAndGetSize(), ...)
            Some(builder) => Arc::new(StarlarkResourceSet {
                builder: builder.clone(),
                action_inputs: parent.get_inputs(),
            }),
            None => Arc::new(to_resource_set(parent.get_resource_set_or_builder())),
        },
        path_mapper,
    ))
}

/// The action's `ResourceSetOrBuilder` when it is a builder (the Starlark `resource_set`
/// callback, `StarlarkActionFactory.StarlarkActionResourceSetBuilder`).
///
/// PORT: Java's `BaseSpawn.getLocalResources` passes the size of the *action's* inputs
/// (`action.getInputs().memoizedFlattenAndGetSize()`), not the spawn's (which add the param
/// files); `action_inputs` keeps them so the size is the action's.
struct StarlarkResourceSet {
    builder: Arc<dyn ResourceSetBuilder>,
    action_inputs: NestedSet<Artifact>,
}

impl std::fmt::Debug for StarlarkResourceSet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "StarlarkResourceSet({:?})", self.builder)
    }
}

impl ResourceSetOrBuilder for StarlarkResourceSet {
    fn build_resource_set(&self, inputs_size: i32) -> ResourceSet {
        match self.try_build_resource_set(inputs_size) {
            Ok(r) => r,
            Err(e) => panic!("{}", e.get_message()),
        }
    }

    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkActionFactory.java:923 StarlarkActionResourceSetBuilder.buildResourceSet
    fn try_build_resource_set(&self, _inputs_size: i32) -> Result<ResourceSet, ExecException> {
        let inputs_size = self.action_inputs.memoized_flatten_and_get_size();
        self.builder
            .build_resource_set(OS::get_current(), inputs_size)
            .map(|r| to_resource_set(&r))
            // Java: catch (EvalException e) -> throw new UserExecException(FailureDetail
            // { message, StarlarkAction { code: STARLARK_ACTION_UNKNOWN } })
            .map_err(|e| {
                ExecException::User(UserExecException::new(failure_detail(
                    &e.message,
                    Category::StarlarkAction(failure_details::StarlarkAction {
                        code: failure_details::starlark_action::Code::StarlarkActionUnknown as i32,
                    }),
                )))
            })
    }
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
    action_execution_context: &dyn ActionExecutionContext,
    spawn_results: &[SpawnResult],
    path_mapper: &PathMapper,
) -> Result<(), ExecException> {
    let Some(unused_inputs_list) = action.get_unused_inputs_list() else {
        return Ok(());
    };

    // Get all the action's inputs after execution which will include the shadowed action
    // discovered inputs
    let all_inputs = action.get_inputs();
    // PORT: java-hash-order. Java collects the used inputs in a `HashMap<String, Artifact>` keyed
    // by mapped exec path and passes `values()` to `updateInputs`; `JavaStringHashMap` replays
    // `java.util.HashMap`'s iteration order.
    let mut used_inputs_by_mapped_path = JavaStringHashMap::new();
    for input in all_inputs.to_list().iter() {
        used_inputs_by_mapped_path.put(
            path_mapper
                .map(input.get_exec_path())
                .get_path_string()
                .to_string(),
            input.clone(),
        );
    }
    // Bazel encodes file system paths as raw bytes stored in a Latin-1 encoded string, so we need
    // to make sure to also decode the unused input list as Latin-1.
    let content = get_unused_input_list_input_stream(
        action_execution_context,
        spawn_results,
        &unused_inputs_list,
    )?;
    let content = match content {
        Ok(c) => c,
        Err(e) => {
            return Err(EnvironmentalExecException::with_cause(
                &e,
                unused_inputs_failure_detail(
                    "Unused inputs read failure",
                    failure_details::starlark_action::Code::UnusedInputListReadFailure,
                ),
            )
            .into());
        }
    };
    let text = blaze_util::string_encoding::latin1_decode(&content);
    for line in buffered_reader_lines(&text) {
        let line = java_string_trim(line);
        if line.is_empty() {
            continue;
        }
        used_inputs_by_mapped_path.remove(line);
    }
    action.update_inputs(NestedSetBuilder::wrap(
        Order::STABLE_ORDER,
        used_inputs_by_mapped_path.into_values(),
    ));
    Ok(())
}

/// The content of the unused inputs list: the in-memory output of the first spawn result, else
/// the file. The outer `Err` is the `UserExecException` for a missing file; the inner one an
/// `IOException` of the read (`afterExecute` turns it into an `EnvironmentalExecException`).
// Java: com/google/devtools/build/lib/analysis/actions/StarlarkAction.java:374 EnhancedStarlarkAction.getUnusedInputListInputStream
fn get_unused_input_list_input_stream(
    action_execution_context: &dyn ActionExecutionContext,
    spawn_results: &[SpawnResult],
    unused_inputs_list_artifact: &Artifact,
) -> Result<std::io::Result<Vec<u8>>, ExecException> {
    // Check if the file is in-memory.
    // Note: SpawnActionContext guarantees that the first list entry exists and corresponds to the
    // executed spawn.
    let content =
        spawn_results[0].get_in_memory_output(&artifact_input(unused_inputs_list_artifact));
    if let Some(content) = content {
        return Ok(Ok(content.to_vec()));
    }
    // Fallback to reading from disk.
    let path = action_execution_context.convert_path(
        &action_execution_context
            .get_exec_root()
            .get_relative(unused_inputs_list_artifact.get_exec_path()),
    );
    let mut stream = match path.get_input_stream() {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let message = format!(
                "Action did not create expected output file listing unused inputs: {}",
                unused_inputs_list_artifact.get_exec_path_string()
            );
            return Err(UserExecException::with_cause(
                &e,
                unused_inputs_failure_detail(
                    &message,
                    failure_details::starlark_action::Code::UnusedInputListFileNotFound,
                ),
            )
            .into());
        }
        // PORT: Java's `getInputStream` throws other `IOException`s out of the try-with-resources
        // in `afterExecute`, which wraps them as "Unused inputs read failure".
        Err(e) => return Ok(Err(e)),
    };
    let mut content = Vec::new();
    Ok(std::io::Read::read_to_end(&mut stream, &mut content).map(|_| content))
}

// Java: com/google/devtools/build/lib/analysis/actions/StarlarkAction.java:131 StarlarkAction.createFailureDetail
fn unused_inputs_failure_detail(
    message: &str,
    code: failure_details::starlark_action::Code,
) -> failure_details::FailureDetail {
    failure_detail(
        message,
        Category::StarlarkAction(failure_details::StarlarkAction { code: code as i32 }),
    )
}

/// `BufferedReader.readLine()` over the whole text: lines end at `\n`, `\r` or `\r\n`; a final
/// line without terminator is returned, an empty remainder is not.
fn buffered_reader_lines(text: &str) -> Vec<&str> {
    let mut lines = Vec::new();
    let bytes = text.as_bytes();
    let mut start = 0;
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'\n' => {
                lines.push(&text[start..i]);
                i += 1;
                start = i;
            }
            b'\r' => {
                lines.push(&text[start..i]);
                i += 1;
                if i < bytes.len() && bytes[i] == b'\n' {
                    i += 1;
                }
                start = i;
            }
            _ => i += 1,
        }
    }
    if start < bytes.len() {
        lines.push(&text[start..]);
    }
    lines
}

/// `String.trim()`: strips leading and trailing chars `<= ' '`.
fn java_string_trim(s: &str) -> &str {
    s.trim_matches(|c: char| c <= ' ')
}

/// A `java.util.HashMap<String, V>` that replays Java's iteration order: `String.hashCode`
/// spread by `h ^ (h >>> 16)`, a power-of-two table starting at 16 buckets that doubles when the
/// size exceeds 3/4 of the capacity (or when a bin of a table below 64 buckets grows past 8
/// entries, `treeifyBin`), insertion order within a bucket (kept by `resize`), `put` of a present
/// key keeps its position. Removal does not shrink the table.
///
/// PORT: java-hash-order. A bin that reaches nine entries in a table of 64 or more buckets
/// becomes a red-black tree in Java (`treeifyBin`), whose `next` order is not the insertion
/// order (`treeify` moves the root to the front, `putTreeVal` links a new node after its
/// parent); that is not replayed, and such a bin fails loudly (review exec-2, E2-4).
struct JavaStringHashMap<V> {
    table: Vec<Vec<(u32, String, V)>>,
    size: usize,
}

impl<V> JavaStringHashMap<V> {
    fn new() -> Self {
        JavaStringHashMap {
            table: (0..16).map(|_| Vec::new()).collect(),
            size: 0,
        }
    }

    /// `String.hashCode()` (UTF-16 code units; internal strings are one char per byte).
    fn hash(key: &str) -> u32 {
        let mut h: i32 = 0;
        for unit in key.encode_utf16() {
            h = h.wrapping_mul(31).wrapping_add(unit as i32);
        }
        let h = h as u32;
        h ^ (h >> 16)
    }

    fn resize(&mut self) {
        let new_cap = self.table.len() * 2;
        let mut new_table: Vec<Vec<(u32, String, V)>> = (0..new_cap).map(|_| Vec::new()).collect();
        for bin in self.table.drain(..) {
            for (h, k, v) in bin {
                new_table[(h as usize) & (new_cap - 1)].push((h, k, v));
            }
        }
        self.table = new_table;
    }

    fn put(&mut self, key: String, value: V) {
        const TREEIFY_THRESHOLD: usize = 8;
        const MIN_TREEIFY_CAPACITY: usize = 64;
        let h = Self::hash(&key);
        let idx = (h as usize) & (self.table.len() - 1);
        if let Some(entry) = self.table[idx]
            .iter_mut()
            .find(|(hh, k, _)| *hh == h && *k == key)
        {
            entry.2 = value;
            return;
        }
        self.table[idx].push((h, key, value));
        // `binCount >= TREEIFY_THRESHOLD - 1` after appending to a list of eight: `treeifyBin`.
        if self.table[idx].len() > TREEIFY_THRESHOLD {
            if self.table.len() < MIN_TREEIFY_CAPACITY {
                self.resize();
            } else {
                blaze_util::unported!(
                    "HashMap.treeifyBin",
                    "nine mapped input paths in one bin of a table of {} buckets: the iteration \
                     order of a tree bin is not replayed",
                    self.table.len()
                );
            }
        }
        self.size += 1;
        if self.size > self.table.len() * 3 / 4 {
            self.resize();
        }
    }

    fn remove(&mut self, key: &str) {
        let h = Self::hash(key);
        let idx = (h as usize) & (self.table.len() - 1);
        if let Some(pos) = self.table[idx]
            .iter()
            .position(|(hh, k, _)| *hh == h && k == key)
        {
            self.table[idx].remove(pos);
            self.size -= 1;
        }
    }

    fn into_values(self) -> Vec<V> {
        self.table
            .into_iter()
            .flatten()
            .map(|(_, _, v)| v)
            .collect()
    }
}

impl ExecutableAction for SpawnAction {
    // Java: com/google/devtools/build/lib/analysis/actions/StarlarkAction.java:349 EnhancedStarlarkAction.discoverInputs
    fn discover_inputs(
        &self,
        _action_execution_context: &dyn ActionExecutionContext,
    ) -> Result<Option<NestedSet<Artifact>>, ActionExecutionException> {
        let SpawnActionClass::EnhancedStarlark(d) = self.get_class() else {
            panic!(
                "IllegalStateException: Not an input-discovering action: {}",
                self.to_string_java()
            )
        };
        // If the Starlark action shadows another action and the shadowed action discovers its
        // inputs, we get the shadowed action's discovered inputs and append it to the Starlark
        // action inputs.
        if let Some(shadowed_action_obj) =
            d.shadowed_action.as_ref().filter(|s| s.discovers_inputs())
        {
            let old_inputs = self.get_inputs();
            let Some(input_files_for_extra_action) =
                shadowed_action_obj.get_input_files_for_extra_action()
            else {
                return Ok(None);
            };
            // StarlarkAction.createInputs(shadowed inputs, extra-action inputs, all inputs).
            let mut inputs = NestedSetBuilder::new_builder(Order::STABLE_ORDER);
            inputs.add_transitive(&shadowed_action_obj.get_inputs());
            inputs.add_transitive(&input_files_for_extra_action);
            inputs.add_transitive(&d.all_starlark_action_inputs);
            self.update_inputs(inputs.build());
            // Sets.difference(getInputs().toSet(), oldInputs.toSet()), in the first set's order.
            let old_inputs = old_inputs.to_set();
            return Ok(Some(NestedSetBuilder::wrap(
                Order::STABLE_ORDER,
                self.get_inputs()
                    .to_set()
                    .into_iter()
                    .filter(|a| !old_inputs.contains(a)),
            )));
        }
        // Otherwise, we need to "re-discover" all the original inputs: the unused ones that were
        // removed might now be needed.
        self.update_inputs(d.all_starlark_action_inputs.clone());
        Ok(Some(d.all_starlark_action_inputs.clone()))
    }

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
                after_execute(
                    self,
                    ctx,
                    &result,
                    crate::actions::Spawn::get_path_mapper(&spawn),
                )?;
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
/// Java returns a `SpawnActionExecutionException`, which also carries the failed `SpawnResult`.
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
    .with_spawn_result(e.get_spawn_result().clone())
}

#[cfg(test)]
mod java_string_hash_map_tests {
    use super::JavaStringHashMap;

    /// Sixteen strings with one `String.hashCode()` ("Aa" and "BB" collide).
    fn colliding_keys() -> Vec<String> {
        (0..16)
            .map(|i: u32| {
                (0..4)
                    .map(|bit| if i & (1 << bit) != 0 { "BB" } else { "Aa" })
                    .collect()
            })
            .collect()
    }

    // A bin of nine in a table below 64 buckets resizes the table (`treeifyBin`); a list bin
    // keeps the insertion order.
    #[test]
    fn treeify_bin_resizes_a_small_table() {
        let keys = colliding_keys();
        let mut map = JavaStringHashMap::new();
        for (i, key) in keys.iter().take(9).enumerate() {
            map.put(key.clone(), i);
        }
        assert_eq!(map.table.len(), 32);
        map.put(keys[9].clone(), 9);
        assert_eq!(map.table.len(), 64);
        assert_eq!(map.into_values(), (0..10).collect::<Vec<_>>());
    }

    // Review exec-2, E2-4: Java would make the bin a tree; the port fails loudly.
    #[test]
    #[should_panic(expected = "HashMap.treeifyBin")]
    fn treeify_bin_of_a_large_table_fails_loudly() {
        let keys = colliding_keys();
        let mut map = JavaStringHashMap::new();
        for (i, key) in keys.iter().take(11).enumerate() {
            map.put(key.clone(), i);
        }
    }
}
