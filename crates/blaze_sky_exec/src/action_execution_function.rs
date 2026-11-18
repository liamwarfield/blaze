//! `ActionExecutionFunction`: the `ACTION_EXECUTION` SkyFunction.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use blaze_actions::actions::action::{Action, ActionRef};
use blaze_actions::{ActionLookupData, Artifact};
use blaze_collect::{NestedSet, NestedSetBuilder, Order};
use blaze_exec::actions::ActionExecutionException;
use blaze_exec::actions::action_cache_checker::{
    PackageRootExceptionOrInterrupted, PackageRootResolver, Token,
};
use blaze_exec::actions::action_execution::{ActionFailed, FailedActionInfo};
use blaze_protos::failure_details::execution;
use blaze_skyframe::{
    ComputeError, Environment, Interrupted, SkyFunctionException, SkyKey, SkyValue, Transience,
};
use blaze_util::check_state;
use blaze_vfs::{PathFragment, Root};
use indexmap::IndexSet;

use crate::action_execution_value::ActionExecutionValue;
use crate::analysis_seam::AnalysisSeam;
use crate::artifact_function::{
    ArtifactValue, MissingArtifactValue, SourceArtifactException, execution_failure,
};
use crate::keys::{ActionExecutionKey, ArtifactKey, ArtifactNestedSetKey, artifact_key};
use crate::skyframe_action_executor::{
    ActionInputMetadata, ContainingPackageRootLookup, ExecutionFailure, SkyframeActionExecutor,
};

/// `ActionExecutionFunction`: executes an action once all its inputs are available.
// Java: com/google/devtools/build/lib/skyframe/ActionExecutionFunction.java:137 ActionExecutionFunction
pub struct ActionExecutionFunction {
    pub skyframe_action_executor: Arc<SkyframeActionExecutor>,
    pub seam: Arc<dyn AnalysisSeam>,
}

/// `ActionUtils.getActionForLookupData`.
// Java: com/google/devtools/build/lib/skyframe/ActionUtils.java:40 ActionUtils.getActionForLookupData
pub fn get_action_for_lookup_data(
    seam: &dyn AnalysisSeam,
    env: &mut Environment<'_>,
    action_lookup_data: &ActionLookupData,
) -> Result<Option<ActionRef>, ComputeError> {
    let Some(value) =
        seam.get_action_lookup_value(action_lookup_data.get_action_lookup_key(), env)?
    else {
        return Ok(None);
    };
    Ok(Some(
        value.get_action(action_lookup_data.get_action_index()),
    ))
}

/// An `ExecutionFailure` as the SkyFunction's error: an `ActionExecutionFunctionException`
/// (persistent) or an interrupt.
fn to_compute_error(e: ExecutionFailure) -> ComputeError {
    match e {
        ExecutionFailure::Exception(e) => {
            SkyFunctionException::new(e, Transience::Persistent).into()
        }
        ExecutionFailure::Interrupted => ComputeError::Interrupted(Interrupted),
    }
}

/// An action's inputs needed for execution. May not just be the result of `Action.getInputs()`.
/// If the action cache's view of this action contains additional inputs, it will request
/// metadata for them, so we consider those inputs as dependencies of this action as well.
// Java: com/google/devtools/build/lib/skyframe/ActionExecutionFunction.java:558 ActionExecutionFunction.AllInputs
struct AllInputs {
    default_inputs: NestedSet<Artifact>,
    action_cache_inputs: Option<Vec<Artifact>>,
}

impl AllInputs {
    /// Compute the inputs to request from Skyframe.
    // Java: com/google/devtools/build/lib/skyframe/ActionExecutionFunction.java:573 AllInputs.getAllInputs
    fn get_all_inputs(&self) -> NestedSet<Artifact> {
        let mut builder = NestedSetBuilder::new_builder(Order::STABLE_ORDER);
        builder.add_transitive(&self.default_inputs);
        if let Some(action_cache_inputs) = &self.action_cache_inputs {
            // actionCacheInputs is never a NestedSet.
            builder.add_all(action_cache_inputs.iter().cloned());
        }
        builder.build()
    }
}

/// The state of one `ActionExecutionFunction` evaluation.
///
/// PORT: Java keeps it across Skyframe restarts (`env.getState`); Blaze's evaluator does not
/// restart, so it lives for one `compute` call. The metadata of inputs discovered after execution
/// (Java's separate `lateDiscoveredInputArtifactData`, kept apart only because the action file
/// system reads `inputArtifactData` concurrently) goes into the same map.
// Java: com/google/devtools/build/lib/skyframe/ActionExecutionFunction.java:1190 ActionExecutionFunction.InputDiscoveryState
struct InputDiscoveryState {
    all_inputs: AllInputs,
    /// Mutable map containing metadata for known artifacts.
    input_artifact_data: Arc<ActionInputMetadata>,
    token: Option<Token>,
    discovered_inputs: Option<NestedSet<Artifact>>,
}

/// Skyframe implementation of `PackageRootResolver`. Should be used only from SkyFunctions,
/// because it uses `SkyFunction.Environment` for evaluation of `ContainingPackageLookupValue`.
// Java: com/google/devtools/build/lib/skyframe/ActionExecutionFunction.java:584 ActionExecutionFunction.PackageRootResolverWithEnvironment
struct PackageRootResolverWithEnvironment<'a, 'b> {
    env: RefCell<&'a mut Environment<'b>>,
    lookup: Option<Arc<dyn ContainingPackageRootLookup>>,
    /// `!packageLookupsRequested.isEmpty()`.
    package_lookups_requested: Cell<bool>,
}

impl PackageRootResolver for PackageRootResolverWithEnvironment<'_, '_> {
    // Java: com/google/devtools/build/lib/skyframe/ActionExecutionFunction.java:594 PackageRootResolverWithEnvironment.findPackageRootsForFiles
    fn find_package_roots_for_files(
        &self,
        exec_paths: &[PathFragment],
    ) -> Result<Option<HashMap<PathFragment, Option<Root>>>, PackageRootExceptionOrInterrupted>
    {
        check_state!(
            !self.package_lookups_requested.get(),
            "resolver should only be called once: {:?}",
            exec_paths
        );
        if exec_paths.is_empty() {
            // PORT: with no exec paths Java requests only PrecomputedValue.STARLARK_SEMANTICS
            // (present in every build) and an empty set of package lookups, and returns an empty
            // map.
            return Ok(Some(HashMap::new()));
        }
        self.package_lookups_requested.set(true);
        let Some(lookup) = &self.lookup else {
            blaze_util::unported!(
                "ActionExecutionFunction.PackageRootResolverWithEnvironment.findPackageRootsForFiles \
                 (no ContainingPackageRootLookup given to the executor)"
            );
        };
        let mut env = self.env.borrow_mut();
        lookup.find_package_roots_for_files(exec_paths, &mut **env)
    }
}

impl ActionExecutionFunction {
    // Java: com/google/devtools/build/lib/skyframe/ActionExecutionFunction.java:175 ActionExecutionFunction.compute
    pub fn compute(
        &self,
        key: &ActionExecutionKey,
        env: &mut Environment<'_>,
    ) -> Result<Option<ActionExecutionValue>, ComputeError> {
        let action_lookup_data = &key.0;
        // PORT: remote analysis caching (retrieveRemoteSkyValue) is not ported.
        let Some(action) = get_action_for_lookup_data(&*self.seam, env, action_lookup_data)? else {
            return Ok(None);
        };
        match self.compute_internal(action_lookup_data, &action, env) {
            Err(ComputeError::Exception(e)) => {
                self.skyframe_action_executor.record_execution_error();
                Err(ComputeError::Exception(e))
            }
            other => other,
        }
    }

    // Java: com/google/devtools/build/lib/skyframe/ActionExecutionFunction.java:213 ActionExecutionFunction.computeInternal
    fn compute_internal(
        &self,
        action_lookup_data: &ActionLookupData,
        action: &ActionRef,
        env: &mut Environment<'_>,
    ) -> Result<Option<ActionExecutionValue>, ComputeError> {
        // PORT: Actions.dependsOnBuildId (BUILD_ID precomputed) and the CLIENT_ENVIRONMENT_VARIABLE
        // dependencies of action.getClientEnvironmentVariables() are not requested: the executor
        // holds the client environment of this single build, and the action cache checker reads
        // the action's variables from it (computeEffectiveEnvironment).

        // If two actions are shared and the first one executes, when the second one goes to
        // execute, we should detect that and short-circuit.
        let previous_execution = self
            .skyframe_action_executor
            .probe_action_execution(action.as_ref());

        let all_inputs = match self.collect_inputs(action, env) {
            Ok(Some(all_inputs)) => all_inputs,
            // Missing deps.
            Ok(None) => return Ok(None),
            Err(e) => return Err(to_compute_error(e)),
        };
        let all_input_set = all_inputs.get_all_inputs();

        // PORT: ActionInputCollectedEvent (BEP/metrics only) is not posted.
        let input_dep_keys =
            get_input_dep_keys(&all_input_set, &action.get_scheduling_dependencies());
        let input_deps_result = env.get_values_and_exceptions(&input_dep_keys)?;

        let mut input_map = None;
        if !previous_execution {
            // Do we actually need to find our metadata?
            match self.check_inputs(
                env,
                action.as_ref(),
                &input_deps_result,
                &all_input_set,
                &input_dep_keys,
            )? {
                Ok(m) => input_map = m,
                Err(e) => {
                    return Err(SkyFunctionException::new(e, Transience::Persistent).into());
                }
            }
        }
        if env.values_missing() {
            // There was missing artifact metadata in the graph. Wait for it to be present.
            return Ok(None);
        }

        let mut state = InputDiscoveryState {
            all_inputs,
            input_artifact_data: Arc::new(input_map.unwrap_or_else(|| {
                ActionInputMetadata::new(self.skyframe_action_executor.get_exec_root().clone())
            })),
            token: None,
            discovered_inputs: None,
        };

        // PORT: the action execution semaphore (acquireActionExecutionSemaphore) only bounds
        // concurrency; the evaluator's parallelism does.
        let action_start_time = Instant::now();
        let result = match self.check_cache_and_execute_if_needed(
            action,
            &mut state,
            env,
            action_lookup_data,
            previous_execution,
            action_start_time,
        ) {
            Ok(result) => result,
            // In this case we do not report the error to the action reporter because we have
            // already done it in SkyframeActionExecutor.reportErrorIfNotAbortingMode() method.
            Err(e) => return Err(to_compute_error(e)),
        };

        if env.values_missing() {
            // This usually happens only for input-discovering actions.
            return Ok(None);
        }
        Ok(result)
    }

    /// An action's inputs needed for execution; `None` if some dependencies were missing.
    // Java: com/google/devtools/build/lib/skyframe/ActionExecutionFunction.java:543 ActionExecutionFunction.collectInputs
    fn collect_inputs(
        &self,
        action: &ActionRef,
        env: &mut Environment<'_>,
    ) -> Result<Option<AllInputs>, ExecutionFailure> {
        let all_known_inputs = action.get_inputs();
        if action.inputs_known() {
            return Ok(Some(AllInputs {
                default_inputs: all_known_inputs,
                action_cache_inputs: None,
            }));
        }

        check_state!(action.discovers_inputs(), "{}", action.to_string_java());
        let action_cache_inputs = {
            let resolver = PackageRootResolverWithEnvironment {
                env: RefCell::new(&mut *env),
                lookup: self
                    .skyframe_action_executor
                    .get_containing_package_root_lookup()
                    .cloned(),
                package_lookups_requested: Cell::new(false),
            };
            self.skyframe_action_executor
                .get_action_cached_inputs(action.as_ref(), &resolver)?
        };
        let Some(action_cache_inputs) = action_cache_inputs else {
            check_state!(env.values_missing(), "{}", action.to_string_java());
            return Ok(None);
        };
        Ok(Some(AllInputs {
            default_inputs: all_known_inputs,
            action_cache_inputs: Some(action_cache_inputs),
        }))
    }

    // Java: com/google/devtools/build/lib/skyframe/ActionExecutionFunction.java:690 ActionExecutionFunction.checkCacheAndExecuteIfNeeded
    fn check_cache_and_execute_if_needed(
        &self,
        action: &ActionRef,
        state: &mut InputDiscoveryState,
        env: &mut Environment<'_>,
        action_lookup_data: &ActionLookupData,
        previous_execution: bool,
        action_start_time: Instant,
    ) -> Result<Option<ActionExecutionValue>, ExecutionFailure> {
        let executor = &*self.skyframe_action_executor;
        let client_env = executor.get_client_env().clone();
        if previous_execution {
            // There are two cases where we can already have an ActionExecutionState for a
            // specific output:
            // 1. Another instance of a shared action won the race and got executed first.
            // 2. The action was already started earlier, and this SkyFunction got restarted
            //    since there's progress to be made.
            // In either case, we must use this ActionExecutionState to continue. Note that in the
            // first case, we don't have any input metadata available, so we couldn't re-execute
            // the action even if we wanted to.
            // PORT: previousAction.getResultOrDependOnFuture is the shared execution's OnceLock:
            // execute_action returns the first execution's result.
            let output_metadata_store =
                Arc::new(executor.create_output_metadata_store(action.as_ref()));
            return executor.execute_action(
                env,
                action,
                state.input_artifact_data.clone(),
                output_metadata_store,
                action_start_time,
                action_lookup_data,
                &mut |_, _, _, _, _| Ok(()),
            );
        }

        // PORT: ArtifactPathResolver.createPathResolver(null, execRoot) is the identity resolver
        // (no action file system).
        let output_metadata_store =
            Arc::new(executor.create_output_metadata_store(action.as_ref()));

        // We only need to check the action cache if we haven't done it on a previous run.
        state.token = executor.check_action_cache(
            env.listener(),
            action,
            &state.input_artifact_data,
            &output_metadata_store,
            action_start_time,
            state.all_inputs.action_cache_inputs.as_deref(),
            &client_env,
        )?;

        let Some(token) = state.token.clone() else {
            let reconstructed_rich_artifact_data = executor
                .reconstruct_rich_data_on_action_cache_hit(
                    action.as_ref(),
                    &state.input_artifact_data,
                );
            return Ok(Some(ActionExecutionValue::create_with_rich_metadata(
                output_metadata_store.get_all_artifact_data(),
                output_metadata_store.get_all_tree_artifact_data(),
                reconstructed_rich_artifact_data,
            )));
        };

        output_metadata_store.prepare_for_action_execution();

        if action.discovers_inputs() {
            if state.discovered_inputs.is_none() {
                // PORT: action.prepareInputDiscovery() is not part of the ported Action protocol;
                // its only override (CppCompileAction) clears additional inputs left over from an
                // earlier build, and actions are created fresh for each build here.
                let discovered_inputs = executor.discover_inputs(
                    env.listener(),
                    action,
                    state.input_artifact_data.clone(),
                )?;
                if env.values_missing() {
                    check_state!(
                        discovered_inputs.is_none(),
                        "Inputs were discovered but more deps were requested by {}",
                        action.to_string_java()
                    );
                    return Ok(None);
                }
                state.discovered_inputs = Some(discovered_inputs.unwrap_or_else(|| {
                    panic!(
                        "NullPointerException: Input discovery returned null but no more deps \
                         were requested by {}",
                        action.to_string_java()
                    )
                }));
            }

            self.add_discovered_inputs(state, env, action.as_ref(), false)?;
            if env.values_missing() {
                return Ok(None);
            }

            // PORT: DiscoveredInputsEvent (spawn metrics of input discovery) is not posted.
        }

        let input_artifact_data = state.input_artifact_data.clone();
        executor.execute_action(
            env,
            action,
            input_artifact_data,
            output_metadata_store,
            action_start_time,
            action_lookup_data,
            // Java: com/google/devtools/build/lib/skyframe/ActionExecutionFunction.java:807 ActionPostprocessingImpl.run
            &mut |env, action, input_metadata_provider, output_metadata_store, client_env| {
                if action.discovers_inputs() {
                    state.discovered_inputs = Some(action.get_inputs());
                    self.add_discovered_inputs(state, env, action.as_ref(), true)?;
                    if env.values_missing() {
                        return Ok(());
                    }
                }
                check_state!(!env.values_missing(), "{}", action.to_string_java());
                // state.inputMetadataProviderIncludingLateDiscoveredInputs(inputMetadataProvider):
                // late discovered inputs are in the same map (see InputDiscoveryState).
                executor.update_action_cache(
                    action,
                    input_metadata_provider,
                    output_metadata_store,
                    &token,
                    client_env,
                )
            },
        )
    }

    /// Adds the metadata of `state.discovered_inputs` that isn't known yet to the state.
    ///
    /// PORT: Java adds inputs discovered after execution to a separate map
    /// (`lateDiscoveredInputArtifactData`) only because `ActionInputMap` is not thread-safe; here
    /// both go into the same (synchronized) map.
    // Java: com/google/devtools/build/lib/skyframe/ActionExecutionFunction.java:845 ActionExecutionFunction.addDiscoveredInputs
    fn add_discovered_inputs(
        &self,
        state: &InputDiscoveryState,
        env: &mut Environment<'_>,
        action_for_error: &dyn Action,
        _after_execution: bool,
    ) -> Result<(), ExecutionFailure> {
        // Filter down to unknown discovered inputs eagerly instead of using a lazy filter to
        // reduce iteration cost.
        let discovered_inputs = state.discovered_inputs.as_ref().expect("discovered inputs");
        let mut unknown_discovered_inputs: Vec<Artifact> = Vec::new();
        for input in discovered_inputs.to_list().iter() {
            if !state.input_artifact_data.is_known(input) {
                unknown_discovered_inputs.push(input.clone());
            }
        }

        if unknown_discovered_inputs.is_empty() {
            return Ok(());
        }

        let input_data = &state.input_artifact_data;
        let keys: Vec<SkyKey> = unknown_discovered_inputs.iter().map(artifact_key).collect();
        let non_mandatory_discovered = env
            .get_values_and_exceptions(&keys)
            .map_err(|_| ExecutionFailure::Interrupted)?;
        for (input, key) in unknown_discovered_inputs.iter().zip(keys.iter()) {
            let retrieved_metadata =
                match get_discovered_input_value(&non_mandatory_discovered, key) {
                    Ok(v) => v,
                    Err(e) => {
                        if !input.is_source_artifact() {
                            panic!(
                                "IllegalStateException: Non-source artifact had \
                             SourceArtifactException {} {}",
                                input.to_debug_string(),
                                action_for_error.pretty_print()
                            );
                        }

                        self.skyframe_action_executor.print_error(
                            &e.get_failure_detail().message,
                            action_for_error,
                            None,
                        );
                        // We don't create a specific cause for the artifact as we do in
                        // #handleMissingFile because it likely has no label, so we'd have to use the
                        // Action's label anyway. Just use the default ActionFailed event constructed by
                        // ActionExecutionException.
                        let message = "discovered input file does not exist";
                        // Java: com/google/devtools/build/lib/skyframe/ActionExecutionFunction.java:1601 ActionExecutionFunction.createDetailedExitCodeForMissingDiscoveredInput
                        let code = execution_failure(
                            message.to_string(),
                            execution::Code::DiscoveredInputDoesNotExist,
                        );
                        return Err(ActionExecutionException::new(
                            message,
                            Some(action_for_error),
                            false,
                            Some(code),
                        )
                        .into());
                    }
                };
            let Some(retrieved_metadata) = retrieved_metadata else {
                check_state!(
                    env.values_missing(),
                    "{} had no metadata but all values were present for {}",
                    input,
                    action_for_error.to_string_java()
                );
                continue;
            };
            match retrieved_metadata {
                DiscoveredInputValue::Artifact(ArtifactValue::Tree(tree_value)) => {
                    if let Some((archived, archived_value)) = &tree_value.archived_representation {
                        let archived =
                            blaze_exec::actions::action_adapters::as_blaze_artifact(archived)
                                .expect("archived tree artifact")
                                .clone();
                        input_data.put(&archived, archived_value.clone());
                    }
                    input_data.put_tree_artifact(input, tree_value);
                }
                DiscoveredInputValue::Action(action_execution_value) => {
                    let value = action_execution_value
                        .get_existing_file_artifact_value(input)
                        .unwrap_or_else(|| panic!("NullPointerException: {input}"))
                        .clone();
                    input_data.put(input, value);
                }
                DiscoveredInputValue::Artifact(ArtifactValue::Missing(_)) => {
                    input_data.put(
                        input,
                        blaze_exec::actions::FileArtifactValue::MissingFileMarker,
                    );
                }
                DiscoveredInputValue::Artifact(ArtifactValue::File(file_artifact_value)) => {
                    input_data.put(input, file_artifact_value);
                }
            }
        }
        Ok(())
    }

    /// Checks the input values: accumulates errors of failed inputs and missing mandatory source
    /// files, and builds the `ActionInputMap`.
    // Java: com/google/devtools/build/lib/skyframe/ActionExecutionFunction.java:968 ActionExecutionFunction.checkInputs
    fn check_inputs(
        &self,
        env: &mut Environment<'_>,
        action: &dyn Action,
        input_deps_result: &blaze_skyframe::SkyframeLookupResult,
        all_inputs: &NestedSet<Artifact>,
        input_dep_keys: &[SkyKey],
    ) -> Result<Result<Option<ActionInputMetadata>, ActionExecutionException>, ComputeError> {
        let is_mandatory = make_mandatory_input_predicate(action);

        let mut handler = ExceptionHandler::new(action, &self.skyframe_action_executor);
        // Java: com/google/devtools/build/lib/skyframe/ActionExecutionFunction.java:1387 ActionExecutionFunctionExceptionHandler.accumulateAndMaybeThrowExceptions
        let mut some_inputs_missing = false;
        for key in input_dep_keys {
            match get_input_dep(input_deps_result, key) {
                Err(InputDepError::Action(e)) => handler.handle_action_execution_exception(e),
                Err(InputDepError::Source(e)) => {
                    let input = &key.downcast::<ArtifactKey>().expect("source key").0;
                    handler.handle_source_artifact_exception(input, is_mandatory(input), e);
                }
                Ok(Some(v)) => {
                    if let Some(ArtifactValue::Missing(_)) = v.downcast_ref::<ArtifactValue>() {
                        some_inputs_missing = true;
                    }
                }
                Ok(None) => {}
            }
        }
        if let Err(e) = handler.maybe_throw_exception() {
            return Ok(Err(e));
        }
        let _ = some_inputs_missing;
        if env.values_missing() {
            return Ok(Ok(None));
        }

        let input_map =
            ActionInputMetadata::new(self.skyframe_action_executor.get_exec_root().clone());
        for input in all_inputs.to_list().iter() {
            let Some(value) = lookup_input(input, input_dep_keys, input_deps_result, env)? else {
                continue;
            };
            // Java: com/google/devtools/build/lib/skyframe/ActionExecutionFunction.java:1084 ActionExecutionFunction.getAndCheckInputSkyValue
            if let InputValue::Artifact(ArtifactValue::Missing(m)) = &value {
                if !is_mandatory(input) {
                    input_map.put(
                        input,
                        blaze_exec::actions::FileArtifactValue::MissingFileMarker,
                    );
                    continue;
                }
                handler.accumulate_missing_file_artifact_value(input, m);
                continue;
            }
            add_to_map(&input_map, input, &value);
        }
        // After accumulating the inputs, we might find some mandatory artifact with
        // SourceFileInErrorArtifactValue.
        if let Err(e) = handler.maybe_throw_exception() {
            return Ok(Err(e));
        }
        Ok(Ok(Some(input_map)))
    }
}

// Java: com/google/devtools/build/lib/skyframe/ActionExecutionFunction.java:923 ActionExecutionFunction.makeMandatoryInputPredicate
fn make_mandatory_input_predicate(action: &dyn Action) -> impl Fn(&Artifact) -> bool + '_ {
    let discovers_inputs = action.discovers_inputs();
    // Lazily flatten the NestedSet in case the predicate is never needed. It's only used in the
    // exceptional case of a missing artifact.
    let mandatory_inputs: std::cell::OnceCell<IndexSet<Artifact>> = std::cell::OnceCell::new();
    let scheduling_dependencies: std::cell::OnceCell<IndexSet<Artifact>> =
        std::cell::OnceCell::new();
    move |input: &Artifact| {
        if !discovers_inputs {
            return true;
        }
        if !input.is_source_artifact() {
            return true;
        }
        if mandatory_inputs
            .get_or_init(|| {
                action
                    .get_mandatory_inputs()
                    .to_list()
                    .iter()
                    .cloned()
                    .collect()
            })
            .contains(input)
        {
            return true;
        }
        scheduling_dependencies
            .get_or_init(|| {
                action
                    .get_scheduling_dependencies()
                    .to_list()
                    .iter()
                    .cloned()
                    .collect()
            })
            .contains(input)
    }
}

/// The value of a discovered input's key: an `ArtifactValue` (source and tree artifacts) or the
/// generating action's `ActionExecutionValue`.
enum DiscoveredInputValue {
    Artifact(ArtifactValue),
    Action(Arc<ActionExecutionValue>),
}

/// `getOrThrow(Artifact.key(input), SourceArtifactException.class)`.
fn get_discovered_input_value(
    result: &blaze_skyframe::SkyframeLookupResult,
    key: &SkyKey,
) -> Result<Option<DiscoveredInputValue>, SourceArtifactException> {
    let catch = |e: &blaze_skyframe::SkyError| e.downcast_ref::<SourceArtifactException>().cloned();
    if let Some(k) = key.downcast::<ArtifactKey>() {
        return result
            .get_or_throw_with(k, catch)
            .map(|v| v.map(|v| DiscoveredInputValue::Artifact((*v).clone())));
    }
    let k = key
        .downcast::<ActionExecutionKey>()
        .expect("discovered input key");
    result
        .get_or_throw_with(k, catch)
        .map(|v| v.map(DiscoveredInputValue::Action))
}

// Java: com/google/devtools/build/lib/skyframe/ActionExecutionFunction.java:403 ActionExecutionFunction.getInputDepKeys
fn get_input_dep_keys(
    all_inputs: &NestedSet<Artifact>,
    scheduling_dependencies: &NestedSet<Artifact>,
) -> Vec<SkyKey> {
    let mut result: IndexSet<SkyKey> = IndexSet::new();
    // We "unwrap" the NestedSet and evaluate the first layer of direct Artifacts here in order to
    // save memory.
    for leaf in all_inputs.get_leaves() {
        result.insert(artifact_key(&leaf));
    }
    if scheduling_dependencies.is_singleton() {
        result.insert(artifact_key(scheduling_dependencies.get_singleton()));
    } else if !scheduling_dependencies.is_empty() {
        result.insert(SkyKey::new(ArtifactNestedSetKey::create(
            scheduling_dependencies.clone(),
        )));
    }
    for non_leaf in all_inputs.get_non_leaves() {
        result.insert(SkyKey::new(ArtifactNestedSetKey::create(non_leaf)));
    }
    result.into_iter().collect()
}

/// The exceptions `getOrThrow(key, SourceArtifactException.class,
/// ActionExecutionException.class, ...)` catches for an input dependency.
enum InputDepError {
    Action(ActionExecutionException),
    Source(SourceArtifactException),
}

fn catch_input_dep_error(e: &blaze_skyframe::SkyError) -> Option<InputDepError> {
    if let Some(x) = e.downcast_ref::<ActionExecutionException>() {
        return Some(InputDepError::Action(x.clone()));
    }
    e.downcast_ref::<SourceArtifactException>()
        .map(|x| InputDepError::Source(x.clone()))
}

fn get_input_dep(
    result: &blaze_skyframe::SkyframeLookupResult,
    key: &SkyKey,
) -> Result<Option<SkyValue>, InputDepError> {
    if let Some(k) = key.downcast::<ArtifactKey>() {
        return result
            .get_or_throw_with(k, catch_input_dep_error)
            .map(|v| v.map(|v| v as SkyValue));
    }
    if let Some(k) = key.downcast::<ActionExecutionKey>() {
        return result
            .get_or_throw_with(k, catch_input_dep_error)
            .map(|v| v.map(|v| v as SkyValue));
    }
    let k = key
        .downcast::<ArtifactNestedSetKey>()
        .expect("input dep key");
    result
        .get_or_throw_with(k, catch_input_dep_error)
        .map(|v| v.map(|v| v as SkyValue))
}

enum InputValue {
    Artifact(ArtifactValue),
    Action(Arc<ActionExecutionValue>),
}

/// Looks up the value for an input without adding additional Skyframe dependencies (if the
/// input's key is a direct dependency) or from the graph (inputs below an
/// `ArtifactNestedSetKey`).
// Java: com/google/devtools/build/lib/skyframe/ActionExecutionFunction.java:1128 ActionExecutionFunction.lookupInput
fn lookup_input(
    input: &Artifact,
    input_dep_keys: &[SkyKey],
    input_deps_result: &blaze_skyframe::SkyframeLookupResult,
    env: &mut Environment<'_>,
) -> Result<Option<InputValue>, ComputeError> {
    let key = artifact_key(input);
    let value: Option<SkyValue> = if input_dep_keys.contains(&key) {
        input_deps_result.get(&key)
    } else {
        // PORT: MemoizingEvaluator.getExistingEntryAtCurrentlyEvaluatingVersion; requesting the
        // already-done key adds a dependency edge, which a non-incremental build does not use.
        env.get_value_dyn(&key)?
    };
    let Some(value) = value else {
        return Ok(None);
    };
    if let Some(k) = key.downcast::<ArtifactKey>() {
        let _ = k;
        let v = value.downcast::<ArtifactValue>().expect("ARTIFACT value");
        return Ok(Some(InputValue::Artifact((*v).clone())));
    }
    let v = value
        .downcast::<ActionExecutionValue>()
        .expect("ACTION_EXECUTION value");
    Ok(Some(InputValue::Action(v)))
}

/// `ActionInputMapHelper.addToMap`.
// Java: com/google/devtools/build/lib/skyframe/ActionInputMapHelper.java:46 ActionInputMapHelper.addToMap
fn add_to_map(input_map: &ActionInputMetadata, key: &Artifact, value: &InputValue) {
    match value {
        InputValue::Artifact(ArtifactValue::File(f)) => input_map.put(key, f.clone()),
        InputValue::Artifact(ArtifactValue::Tree(t)) => {
            input_map.put_tree_artifact(key, t.clone());
            for (child, v) in &t.child_values {
                if let Some(a) = blaze_exec::actions::action_adapters::as_blaze_artifact(child) {
                    input_map.put(a, v.clone());
                }
            }
        }
        InputValue::Artifact(ArtifactValue::Missing(_)) => {}
        InputValue::Action(action_value) => {
            if key.is_runfiles_tree() {
                if let Some(r) = action_value.get_rich_artifact_data() {
                    input_map.put_runfiles_metadata(
                        key,
                        r.clone(),
                        action_value.get_rich_artifact_metadata().cloned(),
                    );
                }
                return;
            }
            if key.is_tree_artifact() {
                if let Some(t) = action_value.get_tree_artifact_value(key) {
                    input_map.put_tree_artifact(key, t.clone());
                }
                return;
            }
            // Java: com/google/devtools/build/lib/skyframe/ActionExecutionValue.java:164 ActionExecutionValue.getExistingFileArtifactValue
            let f = action_value
                .get_existing_file_artifact_value(key)
                .unwrap_or_else(|| {
                    panic!(
                        "NullPointerException: Missing artifact {} (generating action key {:?}) in {:?}",
                        key,
                        key.get_generating_action_key(),
                        action_value
                    )
                });
            input_map.put(key, f.clone());
        }
    }
}

/// A `LabelCause`: a missing or unreadable source input.
// Java: com/google/devtools/build/lib/causes/LabelCause.java:23 LabelCause
#[allow(dead_code)] // `label` is the BEP root-cause id (not ported).
struct LabelCause {
    label: Option<blaze_cmdline::Label>,
    failure_detail: blaze_protos::failure_details::FailureDetail,
}

/// Accumulates the errors of an action's inputs.
// Java: com/google/devtools/build/lib/skyframe/ActionExecutionFunction.java:1354 ActionExecutionFunction.ActionExecutionFunctionExceptionHandler
struct ExceptionHandler<'a> {
    action: &'a dyn Action,
    executor: &'a SkyframeActionExecutor,
    missing_artifact_causes: Vec<LabelCause>,
    transitive_causes: Vec<Vec<ActionFailed>>,
    first_action_execution_exception: Option<ActionExecutionException>,
}

impl<'a> ExceptionHandler<'a> {
    fn new(action: &'a dyn Action, executor: &'a SkyframeActionExecutor) -> ExceptionHandler<'a> {
        ExceptionHandler {
            action,
            executor,
            missing_artifact_causes: Vec::new(),
            transitive_causes: Vec::new(),
            first_action_execution_exception: None,
        }
    }

    // Java: com/google/devtools/build/lib/skyframe/ActionExecutionFunction.java:1523 ActionExecutionFunctionExceptionHandler.handleActionExecutionExceptionPerArtifact
    // PORT: Java maps the failed key back to the mandatory inputs it stands for; every derived
    // input is mandatory, so the per-artifact check reduces to recording the exception once.
    fn handle_action_execution_exception(&mut self, e: ActionExecutionException) {
        // Prefer a catastrophic exception as the one we propagate.
        let replace = match &self.first_action_execution_exception {
            None => true,
            Some(first) => !first.is_catastrophe() && e.is_catastrophe(),
        };
        self.transitive_causes.push(e.get_root_causes().to_vec());
        if replace {
            self.first_action_execution_exception = Some(e);
        }
    }

    // Java: com/google/devtools/build/lib/skyframe/ActionExecutionFunction.java:1466 ActionExecutionFunctionExceptionHandler.handleSourceArtifactExceptionFromSkykey
    fn handle_source_artifact_exception(
        &mut self,
        input: &Artifact,
        is_mandatory: bool,
        e: SourceArtifactException,
    ) {
        if is_mandatory {
            self.missing_artifact_causes.push(LabelCause {
                label: input
                    .get_owner()
                    .or_else(|| self.action.get_owner().get_label().cloned()),
                failure_detail: e.get_failure_detail().clone(),
            });
        }
    }

    // Java: com/google/devtools/build/lib/skyframe/ActionExecutionFunction.java:1486 ActionExecutionFunctionExceptionHandler.accumulateMissingFileArtifactValue
    fn accumulate_missing_file_artifact_value(
        &mut self,
        input: &Artifact,
        missing: &MissingArtifactValue,
    ) {
        self.missing_artifact_causes.push(LabelCause {
            label: input
                .get_owner()
                .or_else(|| self.action.get_owner().get_label().cloned()),
            failure_detail: missing.get_failure_detail().clone(),
        });
    }

    /// Throws the accumulated exception, if any.
    // Java: com/google/devtools/build/lib/skyframe/ActionExecutionFunction.java:1493 ActionExecutionFunctionExceptionHandler.maybeThrowException
    fn maybe_throw_exception(&mut self) -> Result<(), ActionExecutionException> {
        for missing_input in &self.missing_artifact_causes {
            self.executor
                .print_error(&missing_input.failure_detail.message, self.action, None);
        }
        // We need to rethrow the first exception because it can contain a useful error message.
        if let Some(first) = self.first_action_execution_exception.take() {
            if self.missing_artifact_causes.is_empty() && self.transitive_causes.len() == 1 {
                // In the case a single action failed, just propagate the exception upward.
                return Err(first);
            }
            // PORT: LabelCause root causes of missing inputs are not ActionFailed causes; only
            // the transitive action failures are kept.
            let all_causes: Vec<ActionFailed> =
                self.transitive_causes.iter().flatten().cloned().collect();
            return Err(ActionExecutionException::with_root_causes(
                first.get_message().to_string(),
                Some(FailedActionInfo::of(self.action)),
                all_causes,
                first.is_catastrophe(),
                first.get_failure_detail().cloned(),
            ));
        }
        if !self.missing_artifact_causes.is_empty() {
            return Err(self.throw_source_error_exception());
        }
        Ok(())
    }

    // Java: com/google/devtools/build/lib/skyframe/ActionExecutionFunction.java:1584 ActionExecutionFunction.throwSourceErrorException
    fn throw_source_error_exception(&self) -> ActionExecutionException {
        // Java: com/google/devtools/build/lib/skyframe/ActionExecutionFunction.java:1543 ActionExecutionFunction.createSourceErrorCodeAndMessage
        let mut saw_source_artifact_exception = false;
        let mut saw_missing_file = false;
        for cause in &self.missing_artifact_causes {
            if let Some(blaze_protos::failure_details::failure_detail::Category::Execution(e)) =
                &cause.failure_detail.category
            {
                if e.code == execution::Code::SourceInputIoException as i32 {
                    saw_source_artifact_exception = true;
                } else if e.code == execution::Code::SourceInputMissing as i32 {
                    saw_missing_file = true;
                }
            } else {
                saw_source_artifact_exception = true;
            }
        }
        // PORT: DetailedExitCodeComparator picks the highest-priority code; all causes here carry
        // Execution codes of equal exit-code rank, so the first one is used.
        let code = self.missing_artifact_causes[0].failure_detail.clone();
        let parts: Vec<&str> = [
            saw_source_artifact_exception.then_some("are in error"),
            saw_missing_file.then_some("do not exist"),
        ]
        .into_iter()
        .flatten()
        .collect();
        let error_message = format!(
            "{} input file(s) {}",
            self.missing_artifact_causes.len(),
            parts.join(" or ")
        );
        let ex = ActionExecutionException::with_root_causes(
            error_message,
            Some(FailedActionInfo::of(self.action)),
            Vec::new(),
            false,
            Some(code),
        );
        self.executor
            .print_error(ex.get_message(), self.action, None);
        ex
    }
}

/// Port of (a subset of) `TimestampBuilderTest` and
/// `TimestampBuilderWithCompactPersistentActionCacheTest`: the action cache decides whether
/// `ACTION_EXECUTION` re-executes an action. Java's "button" action is a `FileWriteAction` or a
/// `SymlinkAction` here; it was "pressed" when an `ActionCompletionEvent` was posted.
#[cfg(test)]
mod timestamp_builder_tests {
    use std::any::Any;
    use std::fmt;
    use std::hash::{DefaultHasher, Hash, Hasher};
    use std::sync::Arc;

    use blaze_actions::actions::action::ActionRef;
    use blaze_actions::actions::action_lookup::ActionLookupKey;
    use blaze_actions::actions::platform_info::empty_platform_info;
    use blaze_actions::analysis::actions::file_write_action::{Compression, FileWriteAction};
    use blaze_actions::analysis::actions::symlink_action::SymlinkAction;
    use blaze_actions::{
        ActionLookupData, ActionLookupKeyRef, ActionOwner, Artifact, ArtifactOwner, ArtifactRoot,
        RootType,
    };
    use blaze_cmdline::Label;
    use blaze_events::location::Location;
    use blaze_events::{JavaClock, NullEventHandler, StoredEventHandler};
    use blaze_exec::actions::CacheCheckAction;
    use blaze_exec::actions::action_cache_checker::{ActionCacheChecker, NoProxies};
    use blaze_exec::actions::cache::{ActionCache, CompactPersistentActionCache};
    use blaze_skyframe::{ComputeError, Environment, EvaluationContext, EvaluatorBuilder, SkyKey};
    use blaze_vfs::{DigestHashFunction, FileSystemExt, Path, PathFragment, Root, UnixFileSystem};
    use indexmap::IndexMap;

    use crate::analysis_seam::{ActionLookupValue, AnalysisSeam, ConfiguredTargetView};
    use crate::events::{ActionCompletionEvent, downcast_event};
    use crate::keys::ActionExecutionKey;
    use crate::skyframe_action_executor::{
        ActionCacheSetup, ActionContexts, ArtifactFactoryResolver, CachedActionEvent,
        OutputPermissions, SkyframeActionExecutor, SkyframeActionExecutorConfig,
    };

    #[derive(Clone, Debug, PartialEq, Eq, Hash)]
    struct Key(String);

    impl fmt::Display for Key {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "ConfiguredTargetKey{{label={}}}", self.0)
        }
    }

    impl ActionLookupKey for Key {
        fn get_label(&self) -> Option<Label> {
            Some(Label::parse_canonical(&self.0).unwrap())
        }
        fn get_configuration_checksum(&self) -> Option<String> {
            Some("abc".to_string())
        }
        fn function_name(&self) -> &'static str {
            "CONFIGURED_TARGET"
        }
        fn key_equals(&self, other: &dyn ActionLookupKey) -> bool {
            other.as_any().downcast_ref::<Key>() == Some(self)
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

    struct Target(Vec<ActionRef>);

    impl ActionLookupValue for Target {
        fn get_actions(&self) -> &[ActionRef] {
            &self.0
        }
    }

    struct Seam(Arc<Target>);

    impl AnalysisSeam for Seam {
        fn get_action_lookup_value(
            &self,
            _key: &ActionLookupKeyRef,
            _env: &mut Environment<'_>,
        ) -> Result<Option<Arc<dyn ActionLookupValue>>, ComputeError> {
            Ok(Some(self.0.clone() as Arc<dyn ActionLookupValue>))
        }
        fn get_configured_target(
            &self,
            _key: &ActionLookupKeyRef,
            _env: &mut Environment<'_>,
        ) -> Result<Option<Arc<dyn ConfiguredTargetView>>, ComputeError> {
            unreachable!("only ACTION_EXECUTION keys are evaluated")
        }
    }

    fn owner() -> ActionOwner {
        ActionOwner::create_dummy(
            Some(Label::parse_canonical("//pkg:t").unwrap()),
            Location::from_file_line_column("/ws/BUILD", 1, 1),
            "genrule rule",
            "k8-fastbuild",
            "abc",
            None,
            false,
            Some(empty_platform_info()),
            Vec::new(),
            IndexMap::new(),
        )
    }

    struct Fixture {
        _dir: tempfile::TempDir,
        exec_root: Path,
        key: ActionLookupKeyRef,
    }

    /// What one build did: executed actions ("button pressed") and action cache hits.
    #[derive(Debug, PartialEq, Eq)]
    struct BuildCounts {
        executed: usize,
        cached: usize,
    }

    impl Fixture {
        fn new() -> Fixture {
            let dir = tempfile::tempdir().unwrap();
            let fs = UnixFileSystem::create(DigestHashFunction::Sha256);
            let exec_root = fs
                .get_path(dir.path().to_str().unwrap())
                .get_child("execroot");
            exec_root.create_directory_and_parents().unwrap();
            Fixture {
                _dir: dir,
                exec_root,
                key: ActionLookupKeyRef::new(Key("//pkg:t".to_string())),
            }
        }

        fn create_derived_artifact(&self, name: &str) -> Artifact {
            let root = ArtifactRoot::as_derived_root(
                &self.exec_root,
                RootType::OUTPUT,
                &["bazel-out", "k8-fastbuild", "bin"],
            );
            let exec_path = root.get_exec_path().get_relative_str(name);
            Artifact::create_derived_artifact(root, exec_path, self.key.clone())
        }

        fn create_source_artifact(&self, name: &str) -> Artifact {
            Artifact::new_source_artifact(
                ArtifactRoot::as_source_root(Root::from_path(self.exec_root.clone())),
                PathFragment::create(name),
                ArtifactOwner::NullOwner,
            )
        }

        /// `CompactPersistentActionCache.create(cacheRoot, corruptedCacheRoot, tmpDir, ...)`.
        fn create_cache(&self, name: &str) -> Arc<dyn ActionCache> {
            let base = self.exec_root.get_parent_directory().unwrap();
            Arc::new(
                CompactPersistentActionCache::create(
                    &base.get_child(name),
                    &base.get_child(&format!("{name}.bad")),
                    &base.get_child(&format!("{name}.tmp")),
                    Arc::new(JavaClock),
                    NullEventHandler::INSTANCE,
                )
                .unwrap(),
            )
        }

        /// Registers `actions` as the actions of the fixture's target (sets the generating
        /// action keys of their outputs).
        fn register_actions(&self, actions: Vec<ActionRef>) -> Vec<ActionRef> {
            for (i, a) in actions.iter().enumerate() {
                for o in a.get_outputs() {
                    o.set_generating_action_key(ActionLookupData::create(
                        self.key.clone(),
                        i as i32,
                    ));
                }
            }
            actions
        }

        /// `buildArtifacts(cachingBuilder(), ...)`: evaluates the generating action of `output`
        /// with a fresh evaluator (one build).
        fn build(
            &self,
            actions: &[ActionRef],
            cache: &Arc<dyn ActionCache>,
            output: &Artifact,
        ) -> BuildCounts {
            let (result, events) = self.evaluate(actions, cache, output);
            assert!(!result.has_error(), "{:?}", result.error_map());
            let posts = events.get_posts();
            BuildCounts {
                executed: posts
                    .iter()
                    .filter(|p| downcast_event::<ActionCompletionEvent>(p).is_some())
                    .count(),
                cached: posts
                    .iter()
                    .filter(|p| downcast_event::<CachedActionEvent>(p).is_some())
                    .count(),
            }
        }

        /// Evaluates the generating action of `output` (one build).
        fn evaluate(
            &self,
            actions: &[ActionRef],
            cache: &Arc<dyn ActionCache>,
            output: &Artifact,
        ) -> (blaze_skyframe::EvaluationResult, Arc<StoredEventHandler>) {
            blaze_exec::analysis::actions::register();
            let mut contexts = ActionContexts::default();
            for (type_id, ctx) in blaze_exec::analysis::actions::default_action_contexts("_main") {
                contexts.register_raw(type_id, ctx);
            }
            let events = Arc::new(StoredEventHandler::new());
            let checker = Arc::new(ActionCacheChecker::new(
                Some(cache.clone()),
                Arc::new(ArtifactFactoryResolver(None)),
                Box::new(|_: &dyn CacheCheckAction| true),
                Arc::new(NoProxies),
                None,
            ));
            let executor = Arc::new(
                SkyframeActionExecutor::new(SkyframeActionExecutorConfig {
                    exec_root: self.exec_root.clone(),
                    action_out_dir: self
                        .exec_root
                        .get_parent_directory()
                        .unwrap()
                        .get_child("action_outs"),
                    reporter: events.clone(),
                    contexts,
                    client_env: IndexMap::new(),
                    keep_going: false,
                    verbose_failures: false,
                    output_permissions: OutputPermissions::READONLY,
                })
                .with_action_cache(ActionCacheSetup {
                    action_cache_checker: checker,
                    action_execution_salt: String::new(),
                    explain: false,
                    containing_package_root_lookup: None,
                }),
            );
            let evaluator = crate::register_execution_functions(
                EvaluatorBuilder::new(),
                executor,
                Arc::new(Seam(Arc::new(Target(actions.to_vec())))),
                None,
            )
            .build();
            let ctx = EvaluationContext {
                keep_going: false,
                event_handler: events.clone(),
                ..EvaluationContext::default()
            };
            let root = SkyKey::new(ActionExecutionKey(
                output.get_generating_action_key().clone(),
            ));
            let result = evaluator.evaluate(&[root], &ctx).unwrap();
            (result, events)
        }
    }

    const BUILT: BuildCounts = BuildCounts {
        executed: 1,
        cached: 0,
    };
    const NOT_REBUILT: BuildCounts = BuildCounts {
        executed: 0,
        cached: 1,
    };

    fn write_action(f: &Fixture, output: &Artifact, content: &str) -> ActionRef {
        let _ = f;
        Arc::new(FileWriteAction::create(
            owner(),
            output.clone(),
            content.to_string(),
            false,
            Compression::DISALLOW,
        ))
    }

    fn write_file(path: &Path, content: &str) {
        let p = std::path::Path::new(path.get_path_string());
        if p.exists() {
            let mut perms = std::fs::metadata(p).unwrap().permissions();
            #[allow(clippy::permissions_set_readonly_false)]
            perms.set_readonly(false);
            std::fs::set_permissions(p, perms).unwrap();
        }
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, content).unwrap();
    }

    #[test]
    fn test_caching_builder_caches_until_reset() {
        // [action] -> hello
        let f = Fixture::new();
        let hello = f.create_derived_artifact("hello");
        let actions = f.register_actions(vec![write_action(&f, &hello, "content")]);
        let cache = f.create_cache("cache");

        assert_eq!(f.build(&actions, &cache, &hello), BUILT);
        assert_eq!(f.build(&actions, &cache, &hello), NOT_REBUILT);

        // inMemoryCache.reset()
        let cache = f.create_cache("other_cache");
        assert_eq!(f.build(&actions, &cache, &hello), BUILT);
    }

    #[test]
    fn test_modifying_input_causes_action_reexecution() {
        // hello -> [action] -> goodbye
        let f = Fixture::new();
        let hello = f.create_source_artifact("hello");
        write_file(&hello.get_path(), "");
        let goodbye = f.create_derived_artifact("goodbye");
        let symlink: ActionRef = Arc::new(SymlinkAction::to_artifact(
            owner(),
            hello.clone(),
            goodbye.clone(),
            None,
        ));
        let actions = f.register_actions(vec![symlink]);
        let cache = f.create_cache("cache");

        assert_eq!(f.build(&actions, &cache, &goodbye), BUILT);
        assert_eq!(f.build(&actions, &cache, &goodbye), NOT_REBUILT);

        write_file(&hello.get_path(), "new content");
        assert_eq!(f.build(&actions, &cache, &goodbye), BUILT);
        assert_eq!(f.build(&actions, &cache, &goodbye), NOT_REBUILT);
    }

    #[test]
    fn test_only_modifying_input_content_causes_reexecution() {
        // hello -> [action] -> goodbye
        let f = Fixture::new();
        let hello = f.create_source_artifact("hello");
        write_file(&hello.get_path(), "content1");
        let goodbye = f.create_derived_artifact("goodbye");
        let symlink: ActionRef = Arc::new(SymlinkAction::to_artifact(
            owner(),
            hello.clone(),
            goodbye.clone(),
            None,
        ));
        let actions = f.register_actions(vec![symlink]);
        let cache = f.create_cache("cache");

        assert_eq!(f.build(&actions, &cache, &goodbye), BUILT);
        assert_eq!(f.build(&actions, &cache, &goodbye), NOT_REBUILT);

        // FileSystemUtils.touchFile(hello.getPath())
        std::fs::File::options()
            .write(true)
            .open(hello.get_path().get_path_string())
            .unwrap()
            .set_modified(std::time::SystemTime::now() + std::time::Duration::from_secs(10))
            .unwrap();
        // still not rebuilt
        assert_eq!(f.build(&actions, &cache, &goodbye), NOT_REBUILT);

        write_file(&hello.get_path(), "content2");
        assert_eq!(f.build(&actions, &cache, &goodbye), BUILT);
        assert_eq!(f.build(&actions, &cache, &goodbye), NOT_REBUILT);
    }

    #[test]
    fn test_modifying_output_causes_action_reexecution() {
        // [action] -> hello
        let f = Fixture::new();
        let hello = f.create_derived_artifact("hello");
        let actions = f.register_actions(vec![write_action(&f, &hello, "content")]);
        let cache = f.create_cache("cache");

        assert_eq!(f.build(&actions, &cache, &hello), BUILT);
        assert_eq!(f.build(&actions, &cache, &hello), NOT_REBUILT);

        // Changing the *output* file 'hello' causes 'action' to re-execute, to make things
        // consistent again.
        write_file(&hello.get_path(), "new content");
        assert_eq!(f.build(&actions, &cache, &hello), BUILT);
        assert_eq!(f.build(&actions, &cache, &hello), NOT_REBUILT);
    }

    #[test]
    #[ignore = "needs an action with non-mandatory source inputs (Java's TestAction button)"]
    fn test_unneeded_inputs() {}

    /// From `TimestampBuilderWithCompactPersistentActionCacheTest.testUnneededInputs`: "Creating
    /// a new persistent cache does not cause a rebuild".
    #[test]
    fn creating_a_new_persistent_cache_does_not_cause_a_rebuild() {
        let f = Fixture::new();
        let hello = f.create_derived_artifact("hello");
        let actions = f.register_actions(vec![write_action(&f, &hello, "content")]);
        let cache = f.create_cache("cache");

        assert_eq!(f.build(&actions, &cache, &hello), BUILT);
        cache.save().unwrap();
        drop(cache);
        let cache = f.create_cache("cache");
        assert_eq!(f.build(&actions, &cache, &hello), NOT_REBUILT);
    }

    /// `SkyframeActionExecutor.checkForUnsoundDirectoryInputs` warns about a source directory
    /// input and `checkForUnsoundDirectoryOutput` fails an output that is a directory although it
    /// was not declared as one (Java texts).
    #[test]
    fn unsound_directory_input_and_output() {
        let f = Fixture::new();
        let dir = f.create_source_artifact("pkg/dir");
        std::fs::create_dir_all(format!("{}/pkg/dir", f.exec_root.get_path_string())).unwrap();
        let link = f.create_derived_artifact("pkg/link");
        let symlink: ActionRef = Arc::new(SymlinkAction::to_artifact(
            owner(),
            dir.clone(),
            link.clone(),
            None,
        ));
        let actions = f.register_actions(vec![symlink]);
        let cache = f.create_cache("cache");
        let (result, events) = f.evaluate(&actions, &cache, &link);
        assert!(result.has_error());
        let messages: Vec<(blaze_events::EventKind, String)> = events
            .get_events()
            .iter()
            .map(|e| (e.get_kind(), e.get_message().to_string()))
            .collect();
        assert!(
            messages.contains(&(
                blaze_events::EventKind::Warning,
                "input 'pkg/dir' of //pkg:t is a directory; dependency checking of directories \
                 is unsound"
                    .to_string()
            )),
            "{messages:?}"
        );
        assert!(
            messages.contains(&(
                blaze_events::EventKind::Error,
                "output 'pkg/link' of //pkg:t is a directory but was not declared as such"
                    .to_string()
            )),
            "{messages:?}"
        );
    }
}
