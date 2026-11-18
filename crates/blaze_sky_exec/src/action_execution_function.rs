//! `ActionExecutionFunction`: the `ACTION_EXECUTION` SkyFunction.

use std::sync::Arc;
use std::time::Instant;

use blaze_actions::actions::action::{Action, ActionRef};
use blaze_actions::{ActionLookupData, Artifact};
use blaze_collect::{NestedSet, NestedSetBuilder, Order};
use blaze_exec::actions::ActionExecutionException;
use blaze_exec::actions::action_execution::{ActionFailed, FailedActionInfo};
use blaze_protos::failure_details::execution;
use blaze_skyframe::{
    ComputeError, Environment, SkyFunctionException, SkyKey, SkyValue, Transience,
};
use indexmap::IndexSet;

use crate::action_execution_value::ActionExecutionValue;
use crate::analysis_seam::AnalysisSeam;
use crate::artifact_function::{ArtifactValue, MissingArtifactValue, SourceArtifactException};
use crate::keys::{ActionExecutionKey, ArtifactKey, ArtifactNestedSetKey, artifact_key};
use crate::skyframe_action_executor::{ActionInputMetadata, SkyframeActionExecutor};

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
        // holds the client environment of this single build.

        // If two actions are shared and the first one executes, when the second one goes to
        // execute, we should detect that and short-circuit.
        let previous_execution = self
            .skyframe_action_executor
            .probe_action_execution(action.as_ref());

        // Java: com/google/devtools/build/lib/skyframe/ActionExecutionFunction.java:543 ActionExecutionFunction.collectInputs
        if !action.inputs_known() {
            // PORT: actions that discover inputs use the action cache's cached inputs
            // (getActionCachedInputs) before discovery; not ported.
            blaze_util::unported!("ActionExecutionFunction.collectInputs (input discovery)");
        }
        let all_inputs: NestedSet<Artifact> = {
            let mut b = NestedSetBuilder::new_builder(Order::STABLE_ORDER);
            b.add_transitive(&action.get_inputs());
            b.build()
        };

        // PORT: ActionInputCollectedEvent (BEP/metrics only) is not posted.
        let input_dep_keys = get_input_dep_keys(&all_inputs, &action.get_scheduling_dependencies());
        let input_deps_result = env.get_values_and_exceptions(&input_dep_keys)?;

        let mut input_map = None;
        if !previous_execution {
            // Do we actually need to find our metadata?
            match self.check_inputs(
                env,
                action.as_ref(),
                &input_deps_result,
                &all_inputs,
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

        let action_start_time = Instant::now();
        let input_metadata = Arc::new(input_map.unwrap_or_else(|| {
            ActionInputMetadata::new(self.skyframe_action_executor.get_exec_root().clone())
        }));
        // PORT: the action cache check (checkActionCache / updateActionCache) is not wired yet;
        // every action executes. Input discovery (discoverInputs) is unported (see above).
        let result = self.skyframe_action_executor.execute_action(
            env.listener(),
            action,
            input_metadata,
            action_start_time,
            action_lookup_data,
        );
        match result {
            Ok(v) => Ok(Some(v)),
            // In this case we do not report the error to the action reporter because we have
            // already done it in SkyframeActionExecutor.reportErrorIfNotAbortingMode() method.
            Err(e) => Err(SkyFunctionException::new(e, Transience::Persistent).into()),
        }
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
        let mandatory: IndexSet<Artifact> = action
            .get_mandatory_inputs()
            .to_list()
            .iter()
            .cloned()
            .collect();
        // Java: com/google/devtools/build/lib/skyframe/ActionExecutionFunction.java:923 ActionExecutionFunction.makeMandatoryInputPredicate
        let is_mandatory = |a: &Artifact| !a.is_source_artifact() || mandatory.contains(a);

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

        let mut input_map =
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
            add_to_map(&mut input_map, input, &value);
        }
        // After accumulating the inputs, we might find some mandatory artifact with
        // SourceFileInErrorArtifactValue.
        if let Err(e) = handler.maybe_throw_exception() {
            return Ok(Err(e));
        }
        Ok(Ok(Some(input_map)))
    }
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
fn add_to_map(input_map: &mut ActionInputMetadata, key: &Artifact, value: &InputValue) {
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
                    input_map.put_runfiles_metadata(key, r.clone());
                }
                return;
            }
            if key.is_tree_artifact() {
                if let Some(t) = action_value.get_tree_artifact_value(key) {
                    input_map.put_tree_artifact(key, t.clone());
                }
                return;
            }
            if let Some(f) = action_value.get_existing_file_artifact_value(key) {
                input_map.put(key, f.clone());
            }
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
