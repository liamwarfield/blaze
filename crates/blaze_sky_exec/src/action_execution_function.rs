//! `ActionExecutionFunction`: the `ACTION_EXECUTION` SkyFunction.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Instant;

use blaze_actions::actions::action::{Action, ActionRef};
use blaze_actions::{ActionLookupData, Artifact};
use blaze_cmdline::PackageIdentifier;
use blaze_collect::{NestedSet, NestedSetBuilder, Order};
use blaze_exec::actions::action_adapters::artifact_input;
use blaze_exec::actions::action_cache_checker::{
    InterruptedException, PackageRootException, PackageRootExceptionOrInterrupted,
    PackageRootResolver, Token,
};
use blaze_exec::actions::action_execution::{FailedActionInfo, LabelCause, RootCause};
use blaze_exec::actions::{
    ActionExecutionException, ActionInputRef, FileArtifactValue, TreeArtifactValue,
};
use blaze_packages::exceptions::{NoSuchPackageException, NoSuchPackageKind};
use blaze_protos::failure_details::{
    FailureDetail, IncludeScanning, PackageLoading, execution, failure_detail, include_scanning,
    package_loading,
};
use blaze_sky_loading::containing_package_lookup::ContainingPackageLookupKey;
use blaze_sky_loading::io::IOException;
use blaze_sky_loading::precomputed::STARLARK_SEMANTICS;
use blaze_skyframe::{
    ComputeError, Environment, Interrupted, Reset, SkyFunctionException, SkyKey, SkyValue,
    SkyframeLookupResult, Transience,
};
use blaze_util::check_state;
use blaze_util::profiler::{Profiler, ProfilerTask};
use blaze_vfs::{PathFragment, Root};
use indexmap::IndexSet;

use crate::action_execution_value::ActionExecutionValue;
use crate::analysis_seam::AnalysisSeam;
use crate::artifact_function::{
    ArtifactValue, InputDepException, InputDepValue, MissingArtifactValue, SourceArtifactException,
    execution_failure, get_input_dep_or_throw,
};
use crate::completion_function::create_source_error_code_and_message_of;
use crate::events::{ActionCompletionEvent, ErrorTiming};
use crate::keys::{
    ActionExecutionKey, ArtifactKey, ArtifactNestedSetKey, ArtifactNestedSetValue, artifact_key,
};
use crate::skyframe_action_executor::{
    ActionInputMetadata, ContainingPackageRootLookup, ExecutionFailure,
    LostInputsActionExecutionException, SkyframeActionExecutor,
};

/// `ActionExecutionFunction`: executes an action once all its inputs are available.
// Java: com/google/devtools/build/lib/skyframe/ActionExecutionFunction.java:137 ActionExecutionFunction
pub struct ActionExecutionFunction {
    pub skyframe_action_executor: Arc<SkyframeActionExecutor>,
    pub seam: Arc<dyn AnalysisSeam>,
}

/// `ActionUtils.getActionForLookupData`.
// PORT: skipped, ActionUtils.ActionUtils: the private constructor of a static utility class.
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

/// An `ExecutionFailure` as the SkyFunction's error: an `ActionExecutionFunctionException` or an
/// interrupt.
/// Determines whether the given action needs to depend on the build ID.
///
/// PORT: Java's `action instanceof NotifyOnActionCacheHit` is a `TestRunnerAction` or an
/// executable action that answers `is_notify_on_action_cache_hit` (`CoverageReportAction`);
/// every action reaching `ACTION_EXECUTION` is an `Action`.
// Java: com/google/devtools/build/lib/actions/Actions.java:61 Actions.dependsOnBuildId
fn depends_on_build_id(action: &ActionRef) -> bool {
    // Volatile build actions may need to execute even if none of their known inputs have
    // changed. Depending on the build ID ensures that these actions have a chance to execute.
    if blaze_exec::analysis::test::as_test_runner_action(action.as_ref()).is_some()
        || blaze_exec::actions::action_execution::as_executable(action.as_ref())
            .is_some_and(|executable| executable.is_notify_on_action_cache_hit())
    {
        return true;
    }
    action.is_volatile()
}

fn to_compute_error(e: ExecutionFailure) -> ComputeError {
    match e {
        ExecutionFailure::Exception(e) => action_execution_function_exception(e).into(),
        ExecutionFailure::Interrupted => ComputeError::Interrupted(Interrupted),
        // Lost inputs reach ActionExecutionFunction.handleLostInputs only from executing the
        // action (computeInternal's catch), never from the steps that use this conversion.
        ExecutionFailure::LostInputs(e) => panic!(
            "IllegalStateException: unexpected lost inputs: {}",
            e.exception.get_message()
        ),
    }
}

/// `new ActionExecutionFunctionException(e)`: used to declare all the exception types that can be
/// wrapped in the exception thrown by `ActionExecutionFunction.compute`.
// Java: com/google/devtools/build/lib/skyframe/ActionExecutionFunction.java:1314 ActionExecutionFunction.ActionExecutionFunctionException
fn action_execution_function_exception(e: ActionExecutionException) -> SkyFunctionException {
    // We conservatively assume that the error is transient. We don't have enough information to
    // distinguish non-transient errors (e.g. compilation error from a deterministic compiler)
    // from transient ones (e.g. IO error).
    // isCatastrophic(): actionException.isCatastrophe().
    let catastrophic = e.is_catastrophe();
    SkyFunctionException {
        catastrophic,
        ..SkyFunctionException::new(e, Transience::Transient)
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
        lookup.find_package_roots_for_files(exec_paths, &mut env)
    }
}

/// The Skyframe lookups of `PackageRootResolverWithEnvironment.findPackageRootsForFiles` after its
/// `checkState`: `PrecomputedValue.STARLARK_SEMANTICS` and one `CONTAINING_PACKAGE_LOOKUP` per
/// package discovered from the exec paths. The [`ContainingPackageRootLookup`] the build gives
/// the executor (`ExecutionTool`).
// Java: com/google/devtools/build/lib/skyframe/ActionExecutionFunction.java:591 ActionExecutionFunction.PackageRootResolverWithEnvironment
pub struct SkyframeContainingPackageRootLookup;

/// The exceptions `findPackageRootsForFiles` catches from `ContainingPackageLookupValue`s.
enum ContainingPackageLookupException {
    /// `BuildFileNotFoundException`.
    BuildFileNotFound(NoSuchPackageException),
    /// `InconsistentFilesystemException`.
    InconsistentFilesystem(IOException),
}

impl ContainingPackageRootLookup for SkyframeContainingPackageRootLookup {
    // Java: com/google/devtools/build/lib/skyframe/ActionExecutionFunction.java:602 PackageRootResolverWithEnvironment.findPackageRootsForFiles
    fn find_package_roots_for_files(
        &self,
        exec_paths: &[PathFragment],
        env: &mut Environment<'_>,
    ) -> Result<Option<HashMap<PathFragment, Option<Root>>>, PackageRootExceptionOrInterrupted>
    {
        let starlark_semantics = match STARLARK_SEMANTICS.get(env) {
            Ok(Some(s)) => s,
            Ok(None) => return Ok(None),
            Err(Interrupted) => return Err(InterruptedException.into()),
        };

        let sibling_repository_layout = starlark_semantics.experimental_sibling_repository_layout;

        // Create SkyKeys list based on execPaths.
        let mut dep_keys: HashMap<PathFragment, ContainingPackageLookupKey> = HashMap::new();
        // PORT: request order. Java requests `depKeys.values()` (HashMap order); the keys are
        // requested here in the order of `execPaths` (each once).
        let mut requested: IndexSet<ContainingPackageLookupKey> = IndexSet::new();
        for path in exec_paths {
            let parent = path
                .get_parent_directory()
                .expect("Must pass in files, not root directory");
            blaze_util::check_argument!(!parent.is_absolute(), "{}", path);
            let pkg_id =
                PackageIdentifier::discover_from_exec_path(path, true, sibling_repository_layout);
            if let Some(pkg_id) = pkg_id {
                let dep_key = ContainingPackageLookupKey::key(pkg_id);
                requested.insert(dep_key.clone());
                dep_keys.insert(path.clone(), dep_key);
            }
        }

        let request: Vec<SkyKey> = requested.into_iter().map(SkyKey::new).collect();
        let values = env
            .get_values_and_exceptions(&request)
            .map_err(|Interrupted| InterruptedException)?;
        let mut result: HashMap<PathFragment, Option<Root>> = HashMap::new();
        for path in exec_paths {
            let Some(dep_key) = dep_keys.get(path) else {
                continue;
            };
            let value = match values.get_or_throw_with(dep_key, |e| {
                if let Some(nspe) = e.downcast_ref::<NoSuchPackageException>()
                    && nspe.kind() == NoSuchPackageKind::BuildFileNotFound
                {
                    return Some(ContainingPackageLookupException::BuildFileNotFound(
                        nspe.clone(),
                    ));
                }
                e.downcast_ref::<IOException>()
                    .filter(|io| io.is_inconsistent_filesystem())
                    .map(|io| ContainingPackageLookupException::InconsistentFilesystem(io.clone()))
            }) {
                Ok(value) => value,
                Err(ContainingPackageLookupException::BuildFileNotFound(e)) => {
                    return Err(package_root_exception_for_build_file_not_found(path, &e).into());
                }
                Err(ContainingPackageLookupException::InconsistentFilesystem(e)) => {
                    return Err(package_root_exception_for_inconsistent_filesystem(path, &e).into());
                }
            };
            match value {
                Some(value) if value.has_containing_package() => {
                    // We have found corresponding root for current execPath.
                    result.insert(
                        path.clone(),
                        Some(value.get_containing_package_root().clone()),
                    );
                }
                _ => {
                    // We haven't found corresponding root for current execPath.
                    result.insert(path.clone(), None);
                }
            }
        }
        Ok(if env.values_missing() {
            None
        } else {
            Some(result)
        })
    }
}

/// `PackageRootException.create(execPath, BuildFileNotFoundException)`.
// Java: com/google/devtools/build/lib/actions/PackageRootResolver.java:62 PackageRootException.create
pub fn package_root_exception_for_build_file_not_found(
    exec_path: &PathFragment,
    e: &NoSuchPackageException,
) -> PackageRootException {
    let detailed_exit_code = e.get_detailed_exit_code().to_util();
    let failure_detail = detailed_exit_code
        .get_failure_detail()
        .expect("a NoSuchPackageException's DetailedExitCode has a FailureDetail")
        .clone();
    // Java: failureDetail.getPackageLoading().getCode() (the default instance's code when the
    // category is another one).
    let package_loading_code = match &failure_detail.category {
        Some(failure_detail::Category::PackageLoading(PackageLoading { code })) => *code,
        _ => 0,
    };
    let code = if e.has_explicit_detailed_exit_code() {
        if blaze_util::DetailedExitCode::get_exit_code_of(&failure_detail)
            .is_infrastructure_failure()
        {
            include_scanning::Code::SystemPackageLoadFailure
        } else {
            include_scanning::Code::UserPackageLoadFailure
        }
    } else {
        include_scanning::Code::UndifferentiatedPackageLoadFailure
    };
    // PORT: skipped, bug reports (BugReport.sendBugReport when the code is
    // PACKAGE_LOADING_UNKNOWN).
    PackageRootException::new(
        exec_path,
        IncludeScanning {
            code: code as i32,
            package_loading_code,
        },
        &e.get_message(),
    )
}

/// `PackageRootException.create(execPath, InconsistentFilesystemException)`.
// Java: com/google/devtools/build/lib/actions/PackageRootResolver.java:86 PackageRootException.create
pub fn package_root_exception_for_inconsistent_filesystem(
    exec_path: &PathFragment,
    e: &IOException,
) -> PackageRootException {
    PackageRootException::new(
        exec_path,
        IncludeScanning {
            code: include_scanning::Code::SystemPackageLoadFailure as i32,
            package_loading_code: package_loading::Code::PersistentInconsistentFilesystemError
                as i32,
        },
        e.get_message(),
    )
}

/// What `ActionExecutionFunction.compute` returns: the action's value, or a `Reset` that rewinds
/// the actions that generate its lost inputs.
pub enum ActionExecutionFunctionValue {
    Value(ActionExecutionValue),
    Reset(Reset),
}

impl ActionExecutionFunctionValue {
    /// The `SkyValue` for the evaluator.
    pub fn into_sky_value(self) -> SkyValue {
        match self {
            ActionExecutionFunctionValue::Value(v) => Arc::new(v),
            ActionExecutionFunctionValue::Reset(r) => Arc::new(r),
        }
    }
}

/// Java's `UndoneInputsException` and the `ActionExecutionException` `checkInputs` throws.
enum CheckInputsFailure {
    Exception(ActionExecutionException),
    // Java: com/google/devtools/build/lib/skyframe/ActionExecutionFunction.java:1343 ActionExecutionFunction.UndoneInputsException
    UndoneInputs(IndexSet<Artifact>),
}

impl ActionExecutionFunction {
    // Java: com/google/devtools/build/lib/skyframe/ActionExecutionFunction.java:175 ActionExecutionFunction.compute
    pub fn compute(
        &self,
        key: &ActionExecutionKey,
        env: &mut Environment<'_>,
    ) -> Result<Option<ActionExecutionFunctionValue>, ComputeError> {
        let action_lookup_data = &key.0;
        // PORT: skipped, incremental (remote analysis caching, retrieveRemoteSkyValue: retrieval
        // is enabled only with a remote analysis cache service, which is Google-internal).
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
    ) -> Result<Option<ActionExecutionFunctionValue>, ComputeError> {
        if depends_on_build_id(action) {
            if blaze_sky_loading::precomputed::BUILD_ID.get(env)?.is_none() {
                return Ok(None);
            }
        }

        // Look up the parts of the environment that influence the action.
        // PORT: Java builds the action's client environment from these values; the executor
        // holds the command's client environment, whose values for these variables are the same,
        // and the action cache checker reads them from it (computeEffectiveEnvironment). The
        // dependencies are what invalidates the node when a variable changes.
        let client_environment_variables = action.get_client_environment_variables();
        if !client_environment_variables.is_empty() {
            let client_environment_variables_set: IndexSet<String> =
                client_environment_variables.into_iter().collect();
            let dep_keys: Vec<SkyKey> = client_environment_variables_set
                .iter()
                .map(|var| SkyKey::new(blaze_skyframe::precomputed::ClientEnvironmentKey::key(var)))
                .collect();
            env.get_values_and_exceptions(&dep_keys)?;
            if env.values_missing() {
                return Ok(None);
            }
        }

        // If two actions are shared and the first one executes, when the second one goes to
        // execute, we should detect that and short-circuit.
        let previous_execution = self
            .skyframe_action_executor
            .probe_action_execution(action.as_ref());

        // If this action was previously completed this build, then this evaluation must be
        // happening because of rewinding. Prevent any progress events from being published a
        // second time for this action; downstream consumers of action events reasonably don't
        // expect them.
        if !self
            .skyframe_action_executor
            .should_emit_progress_events(action.as_ref())
        {
            env.suppress_progress_events();
        }

        let all_inputs = match self.collect_inputs(action, env) {
            Ok(Some(all_inputs)) => all_inputs,
            // Missing deps.
            Ok(None) => return Ok(None),
            Err(e) => return Err(to_compute_error(e)),
        };
        let all_input_set = all_inputs.get_all_inputs();

        // PORT: skipped, Google-internal (ActionInputCollectedEvent has no subscriber in Bazel).
        let input_dep_keys =
            get_input_dep_keys(&all_input_set, &action.get_scheduling_dependencies());
        let input_dep_keys_list: Vec<SkyKey> = input_dep_keys.iter().cloned().collect();

        let input_deps_result = env.get_values_and_exceptions(&input_dep_keys_list)?;
        let mut checked_inputs = None;
        if !previous_execution {
            // Do we actually need to find our metadata?
            match self.check_inputs(
                env,
                action.as_ref(),
                &input_deps_result,
                &all_input_set,
                &input_dep_keys,
            )? {
                Ok(m) => checked_inputs = m,
                Err(CheckInputsFailure::Exception(e)) => {
                    return Err(action_execution_function_exception(e).into());
                }
                // Java: catch (UndoneInputsException e) in compute (not counted as an execution
                // error).
                // Java: com/google/devtools/build/lib/skyframe/ActionExecutionFunction.java:206 ActionExecutionFunction.compute
                Err(CheckInputsFailure::UndoneInputs(undone_inputs)) => {
                    let executor = &*self.skyframe_action_executor;
                    return Ok(Some(ActionExecutionFunctionValue::Reset(
                        executor
                            .action_rewind_strategy()
                            .patch_nested_set_graph_to_propagate_error(
                                executor,
                                action_lookup_data,
                                action,
                                &undone_inputs,
                                &input_dep_keys,
                            ),
                    )));
                }
            }
        }
        if env.values_missing() {
            // There was missing artifact metadata in the graph. Wait for it to be present.
            // We must check this and return here before attempting to establish any Skyframe
            // dependencies of the action; see establishSkyframeDependencies why.
            return Ok(None);
        }

        let mut state = InputDiscoveryState {
            all_inputs,
            // state.inputArtifactData = checkedInputs.actionInputMap, with the
            // ActionInputMetadataProvider, SkyframeInputMetadataProvider and the
            // DelegatingPairInputMetadataProvider of both over it (ActionInputMetadata). For a
            // shared action that another node executes there is no input metadata.
            input_artifact_data: Arc::new(
                checked_inputs
                    .unwrap_or_else(|| ActionInputMetadata::new(&self.skyframe_action_executor)),
            ),
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
            // That method prints the error in the top-level reporter and also dumps the recorded
            // StdErr for the action. Label can be null in the case of, e.g., the
            // SystemActionOwner (for build-info.txt).
            // Java: com/google/devtools/build/lib/skyframe/ActionExecutionFunction.java:365 ActionExecutionFunction.computeInternal
            Err(ExecutionFailure::Exception(e)) => {
                return Err(action_execution_function_exception(
                    ActionExecutionException::already_reported(e),
                )
                .into());
            }
            Err(e @ ExecutionFailure::Interrupted) => return Err(to_compute_error(e)),
            // Java: com/google/devtools/build/lib/skyframe/ActionExecutionFunction.java:347 ActionExecutionFunction.computeInternal
            Err(ExecutionFailure::LostInputs(e)) => {
                return Ok(self
                    .handle_lost_inputs(
                        *e,
                        action_lookup_data,
                        action,
                        action_start_time,
                        env,
                        &input_dep_keys,
                        &state,
                    )?
                    .map(ActionExecutionFunctionValue::Reset));
            }
        };

        if env.values_missing() {
            // This usually happens only for input-discovering actions.
            return Ok(None);
        }
        Ok(result.map(ActionExecutionFunctionValue::Value))
    }

    /// Cleans up state associated with the current action execution attempt and returns a
    /// `Reset` value which rewinds the actions that generate the lost inputs. `Ok(None)` if there
    /// were missing dependencies (not with `allowSkyframeRestarts()` false).
    // Java: com/google/devtools/build/lib/skyframe/ActionExecutionFunction.java:453 ActionExecutionFunction.handleLostInputs
    #[allow(clippy::too_many_arguments)]
    fn handle_lost_inputs(
        &self,
        e: LostInputsActionExecutionException,
        action_lookup_data: &ActionLookupData,
        action: &ActionRef,
        action_start_time: Instant,
        env: &mut Environment<'_>,
        input_dep_keys: &IndexSet<SkyKey>,
        state: &InputDiscoveryState,
    ) -> Result<Option<Reset>, ComputeError> {
        let executor = &*self.skyframe_action_executor;
        check_state!(
            e.is_primary_action(action_lookup_data),
            "Non-primary action handling lost inputs exception: {} {}",
            action_lookup_data,
            e.exception.get_message()
        );
        let lost_inputs = e
            .exception
            .get_lost_inputs()
            .expect("a LostInputsActionExecutionException")
            .clone();

        // inputDepKeys only contains keys in the initial, pre-input-discovery Skyframe request.
        // If the action discovers inputs, we must combine them with discovered input keys.
        let failed_action_deps: IndexSet<SkyKey> = if e.from_input_discovery {
            // The action failed during input discovery. We don't know the discovered inputs, so
            // just add keys of lost inputs in case any of them were discovered.
            let mut deps = input_dep_keys.clone();
            deps.extend(lost_inputs.entries().into_iter().map(|(_, input)| {
                artifact_key(
                    blaze_exec::actions::action_adapters::as_blaze_artifact(input)
                        .expect("(Artifact) input"),
                )
            }));
            deps
        } else if let Some(discovered_inputs) = &state.discovered_inputs {
            let mut deps = input_dep_keys.clone();
            deps.extend(discovered_inputs.to_list().iter().map(artifact_key));
            deps
        } else {
            input_dep_keys.clone()
        };

        let rewind_plan_result = executor
            .action_rewind_strategy()
            .prepare_rewind_plan_for_lost_inputs(
                executor,
                &*self.seam,
                action_lookup_data,
                action,
                &failed_action_deps,
                &lost_inputs,
                e.action_started_event_already_emitted,
                &state.input_artifact_data,
                env,
                action_start_time,
            );
        // finally: if (e.isActionStartedEventAlreadyEmitted() && rewindPlanResult == null)
        let post_completion_event_if_unsuccessful = |env: &mut Environment<'_>| {
            if e.action_started_event_already_emitted {
                // Rewinding was unsuccessful. SkyframeActionExecutor's ActionRunner didn't emit
                // an ActionCompletionEvent because it hoped rewinding would fix things. Because it
                // won't, this must emit one to compensate.
                env.listener().post(Arc::new(ActionCompletionEvent {
                    related_time_start: action_start_time,
                    finish_time: Instant::now(),
                    action: action.clone(),
                    action_lookup_data: action_lookup_data.clone(),
                }));
            }
        };
        match rewind_plan_result {
            Ok(Ok(rewind_plan_result)) => {
                Ok(rewind_plan_result.to_null_if_missing_dependencies_else_reset())
            }
            Ok(Err(compute_error)) => {
                post_completion_event_if_unsuccessful(env);
                executor.rewinding_failed(action.as_ref(), ExecutionFailure::Interrupted);
                Err(compute_error)
            }
            Err(rewinding_failed_exception) => {
                let reported = executor.process_and_get_exception_to_throw(
                    env.listener(),
                    &e.primary_output_path,
                    action,
                    ActionExecutionException::new(
                        rewinding_failed_exception.get_message().to_owned(),
                        Some(action.as_ref()),
                        /* catastrophe= */ false,
                        Some(rewinding_failed_exception.get_failure_detail()),
                    ),
                    &e.file_out_err,
                    ErrorTiming::AFTER_EXECUTION,
                );
                post_completion_event_if_unsuccessful(env);
                let already_reported = ActionExecutionException::already_reported(reported);
                executor.rewinding_failed(
                    action.as_ref(),
                    ExecutionFailure::Exception(already_reported.clone()),
                );
                Err(action_execution_function_exception(already_reported).into())
            }
        }
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
            let output_metadata_store = Arc::new(
                executor.create_output_metadata_store(action.as_ref(), &state.input_artifact_data),
            );
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

        // The output metadata store resolves its paths through the action file system when the
        // output service has one (ArtifactPathResolver.createPathResolver(actionFileSystem,
        // execRoot)).
        let output_metadata_store = Arc::new(
            executor.create_output_metadata_store(action.as_ref(), &state.input_artifact_data),
        );

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
            // Java: com/google/devtools/build/lib/skyframe/ActionExecutionFunction.java:729 ActionExecutionFunction.checkCacheAndExecuteIfNeeded
            let mut discovered_inputs_duration = std::time::Duration::ZERO;
            if state.discovered_inputs.is_none() {
                // PORT: action.prepareInputDiscovery() is not part of the ported Action protocol;
                // its only override (CppCompileAction) clears additional inputs left over from an
                // earlier build, and actions are created fresh for each build here.
                let discovered_inputs = {
                    // Java: com/google/devtools/build/lib/skyframe/ActionExecutionFunction.java:736 ActionExecutionFunction.checkCacheAndExecuteIfNeeded
                    let _c = Profiler::instance()
                        .profile(ProfilerTask::DISCOVER_INPUTS, "discoverInputs");
                    let _skyframe_allowed = state.input_artifact_data.with_skyframe_allowed();
                    // state.actionFileSystem: the one the output metadata store holds, which
                    // execution in this same call uses as well. PORT: Java keeps it in the
                    // SkyKeyComputeState across restarts; here a restart (missing deps) creates
                    // the store, and so the file system, again (see create_action_file_system).
                    // Java: com/google/devtools/build/lib/skyframe/ActionExecutionFunction.java:740 ActionExecutionFunction.checkCacheAndExecuteIfNeeded
                    executor.discover_inputs(
                        env.listener(),
                        action,
                        state.input_artifact_data.clone(),
                        output_metadata_store.get_action_file_system().cloned(),
                    )?
                };
                discovered_inputs_duration = action_start_time.elapsed();
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

            // When discover inputs completes, post an event with the duration values.
            // Java: com/google/devtools/build/lib/skyframe/ActionExecutionFunction.java:769 ActionExecutionFunction.checkCacheAndExecuteIfNeeded
            // SpawnMetrics.Builder.forOtherExec().setParseTime(d).setTotalTime(d): toMs is
            // `Ints.saturatedCast(duration.toMillis())`.
            let discovered_inputs_millis =
                i32::try_from(discovered_inputs_duration.as_millis()).unwrap_or(i32::MAX);
            env.listener()
                .post(Arc::new(crate::events::DiscoveredInputsEvent {
                    metrics: blaze_exec::actions::spawn_metrics::SpawnMetrics {
                        parse_time_in_ms: discovered_inputs_millis,
                        total_time_in_ms: discovered_inputs_millis,
                        ..blaze_exec::actions::spawn_metrics::SpawnMetrics::for_exec(
                            blaze_exec::actions::spawn_metrics::ExecKind::Other,
                        )
                    },
                    action: action.clone(),
                    start_time_nanos: action_start_time,
                }));
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
                        // Java: com/google/devtools/build/lib/skyframe/ActionExecutionFunction.java:503 ActionExecutionFunction.addDiscoveredInputs
                        return Err(ActionExecutionException::already_reported(
                            ActionExecutionException::new(
                                message,
                                Some(action_for_error),
                                false,
                                Some(code),
                            ),
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
                    input_data.put_tree_artifact(input, tree_value.clone());
                    if let Some((archived_tree_file_artifact, archived_file_value)) =
                        &tree_value.archived_representation
                    {
                        input_data.put_input(
                            archived_tree_file_artifact.clone(),
                            archived_file_value.clone(),
                        );
                    }
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

    /// Declares a dependency on all known inputs of the action. Throws an exception if any are
    /// known to be missing.
    ///
    /// Returns `Ok(Ok(None))` if `env.values_missing()` is true and no inputs result in
    /// `ActionExecutionException`s; `Ok(Err(e))` is Java's thrown `ActionExecutionException` or
    /// `UndoneInputsException`.
    // Java: com/google/devtools/build/lib/skyframe/ActionExecutionFunction.java:968 ActionExecutionFunction.checkInputs
    fn check_inputs(
        &self,
        env: &mut Environment<'_>,
        action: &dyn Action,
        input_deps_result: &SkyframeLookupResult,
        all_inputs: &NestedSet<Artifact>,
        input_dep_keys: &IndexSet<SkyKey>,
    ) -> Result<Result<Option<ActionInputMetadata>, CheckInputsFailure>, ComputeError> {
        let is_mandatory_input = make_mandatory_input_predicate(action);

        let mut action_execution_function_exception_handler =
            ActionExecutionFunctionExceptionHandler::new(
                all_inputs,
                input_deps_result,
                action,
                &is_mandatory_input,
                input_dep_keys,
                &self.skyframe_action_executor,
            );
        let has_missing_inputs = match action_execution_function_exception_handler
            .accumulate_and_maybe_throw_exceptions()
        {
            Ok(has_missing_inputs) => has_missing_inputs,
            Err(e) => return Ok(Err(CheckInputsFailure::Exception(e))),
        };

        if env.values_missing() {
            return Ok(Ok(None));
        }

        let all_inputs_list = all_inputs.to_list();

        // When there are no missing values or there was an error, we can start checking
        // individual files. We don't bother to optimize the error-ful case since it's rare.
        let input_artifact_data = ActionInputMetadata::new(&self.skyframe_action_executor);
        let mut undone_inputs: IndexSet<Artifact> = IndexSet::new();

        for input in all_inputs_list.iter() {
            let value = self.get_and_check_input_sky_value(
                env,
                input,
                input_dep_keys,
                input_deps_result,
                &is_mandatory_input,
                &mut action_execution_function_exception_handler,
            )?;

            if let Some(value) = value {
                add_to_map(&input_artifact_data, input, &value);
            } else if !has_missing_inputs && input.has_known_generating_action() {
                // Derived inputs are mandatory, but we did not detect any missing inputs. This is
                // only possible for indirect inputs (beneath an ArtifactNestedSetKey) when, between
                // the time the associated direct dependency ArtifactNestedSetKey completes
                // successfully and the call to lookupInput, the input's key was rewound and
                // completed with an error.
                undone_inputs.insert(input.clone());
            }
        }

        if !undone_inputs.is_empty() {
            return Ok(Err(CheckInputsFailure::UndoneInputs(undone_inputs)));
        }

        // If there were no errors, we don't go through the scheduling dependencies because the
        // only reason to do so is to find and report missing input source files.
        if has_missing_inputs {
            // We unwrap the nested set like in getInputDepKeys(); apparently, if we don't do
            // this, it's a significant memory use hit due to the memoized graph traversal in
            // NestedSet.
            let mut seen: HashSet<Artifact> = HashSet::new();
            let scheduling_dependencies = action.get_scheduling_dependencies();
            for input in scheduling_dependencies.get_leaves() {
                if !seen.insert(input.clone()) {
                    panic!("VerifyException");
                }
                self.get_and_check_input_sky_value(
                    env,
                    &input,
                    input_dep_keys,
                    input_deps_result,
                    &is_mandatory_input,
                    &mut action_execution_function_exception_handler,
                )?;
            }

            for non_leaf in scheduling_dependencies.get_non_leaves() {
                for input in non_leaf.to_list().iter() {
                    if seen.insert(input.clone()) {
                        self.get_and_check_input_sky_value(
                            env,
                            input,
                            input_dep_keys,
                            input_deps_result,
                            &is_mandatory_input,
                            &mut action_execution_function_exception_handler,
                        )?;
                    }
                }
            }
        }

        // After accumulating the inputs, we might find some mandatory artifact with
        // SourceFileInErrorArtifactValue.
        if let Err(e) = action_execution_function_exception_handler.maybe_throw_exception() {
            return Ok(Err(CheckInputsFailure::Exception(e)));
        }

        Ok(Ok(Some(input_artifact_data)))
    }

    // Java: com/google/devtools/build/lib/skyframe/ActionExecutionFunction.java:1084 ActionExecutionFunction.getAndCheckInputSkyValue
    fn get_and_check_input_sky_value(
        &self,
        env: &mut Environment<'_>,
        input: &Artifact,
        input_dep_keys: &IndexSet<SkyKey>,
        input_deps_result: &SkyframeLookupResult,
        is_mandatory_input: &dyn Fn(&Artifact) -> bool,
        action_execution_function_exception_handler: &mut ActionExecutionFunctionExceptionHandler<
            '_,
        >,
    ) -> Result<Option<InputValue>, ComputeError> {
        let Some(value) = lookup_input(input, input_dep_keys, input_deps_result, env)? else {
            // Undone mandatory inputs are only expected for generated artifacts when rewinding is
            // enabled. Returning null allows the caller to use UndoneInputsException to recover.
            check_state!(
                !is_mandatory_input(input)
                    || (input.has_known_generating_action()
                        && self.skyframe_action_executor.rewinding_enabled()),
                "Unexpected undone mandatory input: {}",
                input
            );
            return Ok(None);
        };
        if let InputValue::Artifact(ArtifactValue::Missing(missing)) = &value {
            if !is_mandatory_input(input) {
                return Ok(Some(InputValue::File(FileArtifactValue::MissingFileMarker)));
            }
            action_execution_function_exception_handler
                .accumulate_missing_file_artifact_value(input, missing);
            return Ok(None);
        }
        Ok(Some(value))
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
    result: &SkyframeLookupResult,
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

/// Evaluate the supplied input deps. Declare deps on known inputs to action. We do this
/// unconditionally to maintain our invariant of asking for the same deps each build.
///
/// PORT: the `consumedArtifactsTracker` (remote output bookkeeping) is null and
/// `state.topLevelFileArtifacts` belongs to it.
// Java: com/google/devtools/build/lib/skyframe/ActionExecutionFunction.java:403 ActionExecutionFunction.getInputDepKeys
fn get_input_dep_keys(
    all_inputs: &NestedSet<Artifact>,
    scheduling_dependencies: &NestedSet<Artifact>,
) -> IndexSet<SkyKey> {
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
    result
}

/// The `SkyValue` of an input: the value of `Artifact.key(input)`, or the
/// `FileArtifactValue.MISSING_FILE_MARKER` that stands for a missing non-mandatory input.
pub(crate) enum InputValue {
    Artifact(ArtifactValue),
    Action(Arc<ActionExecutionValue>),
    File(FileArtifactValue),
}

/// Looks up the value for an input without adding additional Skyframe dependencies.
///
/// If the input's `Artifact.key` is already a direct dependency, looks up its value in the
/// `Environment`. Otherwise, the input is assumed to be beneath an already-requested
/// `ArtifactNestedSetKey`.
// Java: com/google/devtools/build/lib/skyframe/ActionExecutionFunction.java:1128 ActionExecutionFunction.lookupInput
fn lookup_input(
    input: &Artifact,
    input_dep_keys: &IndexSet<SkyKey>,
    input_deps_result: &SkyframeLookupResult,
    env: &mut Environment<'_>,
) -> Result<Option<InputValue>, ComputeError> {
    let key = artifact_key(input);
    let value: Option<SkyValue> = if input_dep_keys.contains(&key) {
        // env.getLookupHandleForPreviouslyRequestedDeps().get(key)
        input_deps_result.get(&key)
    } else {
        // PORT: Java reads the done node without a dependency
        // (MemoizingEvaluator.getExistingEntryAtCurrentlyEvaluatingVersion; the node is beneath a
        // requested ArtifactNestedSetKey). The engine has no such read, so the key is requested:
        // it is done, and the extra edge is only used by incremental builds.
        env.get_value_dyn(&key)?
    };
    let Some(value) = value else {
        return Ok(None);
    };
    if key.downcast::<ArtifactKey>().is_some() {
        let v = value.downcast::<ArtifactValue>().expect("ARTIFACT value");
        return Ok(Some(InputValue::Artifact((*v).clone())));
    }
    let v = value
        .downcast::<ActionExecutionValue>()
        .expect("ACTION_EXECUTION value");
    Ok(Some(InputValue::Action(v)))
}

/// Adds a value obtained by an Artifact SkyValue lookup to the action input map.
// Java: com/google/devtools/build/lib/skyframe/ActionInputMapHelper.java:31 ActionInputMapHelper.addToMap
pub(crate) fn add_to_map(input_map: &ActionInputMetadata, key: &Artifact, value: &InputValue) {
    match value {
        InputValue::Artifact(ArtifactValue::Tree(tree_artifact_value)) => {
            expand_tree_artifact_and_populate_artifact_data(
                artifact_input(key),
                tree_artifact_value,
                input_map,
            );
        }
        InputValue::Action(action_execution_value) => {
            // Actions resulting from the expansion of an ActionTemplate consume only one of the
            // files in a tree artifact. However, the input prefetcher and the Linux sandbox
            // require access to the tree metadata to determine the prefetch location of a tree
            // artifact materialized as a symlink to another (cf.
            // TreeArtifactValue#getResolvedPath()).
            if key.is_child_of_declared_directory() {
                let tree_artifact = key
                    .get_parent()
                    .unwrap_or_else(|| panic!("NullPointerException: parent of {key}"));
                let tree_artifact_value = action_execution_value
                    .get_tree_artifact_value(tree_artifact)
                    .unwrap_or_else(|| panic!("NullPointerException: {tree_artifact}"));
                expand_tree_artifact_and_populate_artifact_data(
                    artifact_input(tree_artifact),
                    tree_artifact_value,
                    input_map,
                );
            }
            if key.is_fileset() {
                blaze_util::unported!(
                    "ActionInputMapHelper.addToMap (unreachable in Bazel: only the non-open-sourced fileset rule makes fileset artifacts, packages/OutputFile.java:123)",
                    "fileset input {}",
                    key
                );
            } else if key.is_runfiles_tree() {
                let runfiles_artifact_value = action_execution_value
                    .get_rich_artifact_data()
                    .unwrap_or_else(|| panic!("NullPointerException: rich artifact data of {key}"))
                    .clone();
                // Note: we don't expand the .runfiles/MANIFEST file into the inputs. The reason
                // for that being that the MANIFEST file contains absolute paths that don't work
                // with remote execution.
                // Instead, the way the SpawnInputExpander expands runfiles is via the Runfiles
                // class which contains all artifacts in the runfiles tree minus the MANIFEST file.
                runfiles_artifact_value.for_each_file(|artifact, metadata| {
                    input_map.put_input(artifact.clone(), metadata.clone());
                });
                runfiles_artifact_value.for_each_tree(|tree_artifact, metadata| {
                    expand_tree_artifact_and_populate_artifact_data(
                        tree_artifact.clone(),
                        metadata,
                        input_map,
                    );
                });
                // forEachFileset: a RunfilesArtifactValue holds no filesets (they are unported
                // where the value is built).

                // This is used for two purposes:
                // - To collect the RunfilesTree objects which the execution strategies need to
                //   expand the runfiles trees
                // - The action cache checker may want the digest of the aggregated runfiles tree
                input_map.put_runfiles_metadata(key, runfiles_artifact_value);
            } else {
                // Java: com/google/devtools/build/lib/skyframe/ActionExecutionValue.java:164 ActionExecutionValue.getExistingFileArtifactValue
                let metadata = action_execution_value
                    .get_existing_file_artifact_value(key)
                    .unwrap_or_else(|| {
                        panic!(
                            "NullPointerException: Missing artifact {} (generating action key {:?}) in {:?}",
                            key,
                            key.get_generating_action_key(),
                            action_execution_value
                        )
                    });
                input_map.put(key, metadata.clone());
            }
        }
        InputValue::Artifact(ArtifactValue::File(file_artifact_value)) => {
            input_map.put(key, file_artifact_value.clone());
        }
        InputValue::File(file_artifact_value) => {
            input_map.put(key, file_artifact_value.clone());
        }
        InputValue::Artifact(ArtifactValue::Missing(_)) => {
            panic!("IllegalStateException: Unexpected value MissingArtifactValue for {key}")
        }
    }
}

// Java: com/google/devtools/build/lib/skyframe/ActionInputMapHelper.java:107 ActionInputMapHelper.expandTreeArtifactAndPopulateArtifactData
fn expand_tree_artifact_and_populate_artifact_data(
    tree_artifact: ActionInputRef,
    value: &Arc<TreeArtifactValue>,
    input_map: &ActionInputMetadata,
) {
    input_map.put_tree_artifact_input(tree_artifact, value.clone());
    let Some((archived_tree_file_artifact, archived_file_value)) = &value.archived_representation
    else {
        return;
    };

    input_map.put_input(
        archived_tree_file_artifact.clone(),
        archived_file_value.clone(),
    );
}

// Java: com/google/devtools/build/lib/skyframe/ActionExecutionFunction.java:1160 ActionExecutionFunction.createLabelCauseNullOwnerOk
fn create_label_cause_null_owner_ok(
    input: &Artifact,
    failure_detail: &FailureDetail,
    action_label: Option<&blaze_cmdline::Label>,
) -> LabelCause {
    // bugReporter.logUnexpected("Unexpected exit code %s for generated artifact %s (%s)") for a
    // derived input: logging only.
    LabelCause {
        // MoreObjects.firstNonNull(input.getOwner(), actionLabel)
        label: input
            .get_owner()
            .or_else(|| action_label.cloned())
            .unwrap_or_else(|| panic!("NullPointerException: owner of {input} and action label")),
        failure_detail: failure_detail.clone(),
    }
}

/// Helper for the error-handling logic for `check_inputs`.
// Java: com/google/devtools/build/lib/skyframe/ActionExecutionFunction.java:1354 ActionExecutionFunction.ActionExecutionFunctionExceptionHandler
struct ActionExecutionFunctionExceptionHandler<'a> {
    /// The inputs of the memoized `skyKeyToDerivedArtifactSetForExceptions` supplier.
    all_inputs: &'a NestedSet<Artifact>,
    sky_key_to_derived_artifact_set_for_exceptions:
        std::cell::OnceCell<HashMap<SkyKey, IndexSet<Artifact>>>,
    input_deps_result: &'a SkyframeLookupResult,
    action: &'a dyn Action,
    is_mandatory_input: &'a dyn Fn(&Artifact) -> bool,
    input_dep_keys: &'a IndexSet<SkyKey>,
    skyframe_action_executor: &'a SkyframeActionExecutor,
    missing_artifact_causes: Vec<LabelCause>,
    /// `List<NestedSet<Cause>>`: the root causes of each failed input.
    transitive_causes: Vec<Vec<RootCause>>,
    first_action_execution_exception: Option<ActionExecutionException>,
}

impl<'a> ActionExecutionFunctionExceptionHandler<'a> {
    // Java: com/google/devtools/build/lib/skyframe/ActionExecutionFunction.java:1364 ActionExecutionFunctionExceptionHandler.<init>
    fn new(
        all_inputs: &'a NestedSet<Artifact>,
        input_deps_result: &'a SkyframeLookupResult,
        action: &'a dyn Action,
        is_mandatory_input: &'a dyn Fn(&Artifact) -> bool,
        input_dep_keys: &'a IndexSet<SkyKey>,
        skyframe_action_executor: &'a SkyframeActionExecutor,
    ) -> ActionExecutionFunctionExceptionHandler<'a> {
        ActionExecutionFunctionExceptionHandler {
            all_inputs,
            sky_key_to_derived_artifact_set_for_exceptions: std::cell::OnceCell::new(),
            input_deps_result,
            action,
            is_mandatory_input,
            input_dep_keys,
            skyframe_action_executor,
            missing_artifact_causes: Vec::new(),
            transitive_causes: Vec::new(),
            first_action_execution_exception: None,
        }
    }

    /// The inputs (and scheduling dependencies) of the action by the key of their generating
    /// action. Source and tree artifacts are their own keys and are left out.
    // Java: com/google/devtools/build/lib/skyframe/ActionExecutionFunction.java:977 ActionExecutionFunction.checkInputs (the memoized supplier)
    fn associated_inputs(&self, key: &SkyKey) -> IndexSet<Artifact> {
        self.sky_key_to_derived_artifact_set_for_exceptions
            .get_or_init(|| {
                let mut all_inputs_set: IndexSet<Artifact> = IndexSet::new();
                all_inputs_set.extend(self.all_inputs.to_list().iter().cloned());
                all_inputs_set.extend(
                    self.action
                        .get_scheduling_dependencies()
                        .to_list()
                        .iter()
                        .cloned(),
                );
                let mut sky_key_to_artifact_set: HashMap<SkyKey, IndexSet<Artifact>> =
                    HashMap::new();
                for input in all_inputs_set {
                    let key = artifact_key(&input);
                    // `key != input`: the key of the generating action.
                    if key.downcast::<ArtifactKey>().is_none() {
                        sky_key_to_artifact_set
                            .entry(key)
                            .or_default()
                            .insert(input);
                    }
                }
                sky_key_to_artifact_set
            })
            .get(key)
            .cloned()
            .unwrap_or_default()
    }

    /// Goes through the list of evaluated SkyKeys and handles any exception that arises, taking
    /// into account whether the corresponding artifact(s) is a mandatory input.
    ///
    /// `Err` if the eval of any mandatory artifact threw an exception; otherwise true if there is
    /// at least one input artifact that is missing.
    // Java: com/google/devtools/build/lib/skyframe/ActionExecutionFunction.java:1387 ActionExecutionFunctionExceptionHandler.accumulateAndMaybeThrowExceptions
    fn accumulate_and_maybe_throw_exceptions(&mut self) -> Result<bool, ActionExecutionException> {
        let mut some_inputs_missing = false;
        for key in self.input_dep_keys {
            match get_input_dep_or_throw(self.input_deps_result, key) {
                Ok(None) => continue,
                Ok(Some(InputDepValue::NestedSet(value))) => {
                    if value == ArtifactNestedSetValue::SOME_MISSING {
                        some_inputs_missing = true;
                    }
                    continue;
                }
                Ok(Some(InputDepValue::Artifact(value))) => {
                    if matches!(*value, ArtifactValue::Missing(_)) {
                        some_inputs_missing = true;
                    }
                }
                Ok(Some(InputDepValue::Action(_))) => {}
                Err(InputDepException::Source(e, _)) => {
                    self.handle_source_artifact_exception_from_skykey(key, &e);
                }
                Err(InputDepException::Action(e, _)) => {
                    self.handle_action_execution_exception_from_skykey(key, &e);
                }
                Err(InputDepException::NestedSetEval(e)) => {
                    for sky_key_and_exception in e.get_nested_exceptions().to_list().iter() {
                        let sky_key = sky_key_and_exception.get_first();
                        if let Some(input_exception) =
                            sky_key_and_exception.get_source_artifact_exception()
                        {
                            self.handle_source_artifact_exception_from_skykey(
                                sky_key,
                                input_exception,
                            );
                            continue;
                        }
                        let Some(input_exception) =
                            sky_key_and_exception.get_action_execution_exception()
                        else {
                            panic!(
                                "IllegalStateException: Unexpected exception type, key: {sky_key:?}"
                            );
                        };
                        self.handle_action_execution_exception_from_skykey(
                            sky_key,
                            input_exception,
                        );
                    }
                }
            }
        }
        self.maybe_throw_exception()?;
        Ok(some_inputs_missing)
    }

    // Java: com/google/devtools/build/lib/skyframe/ActionExecutionFunction.java:1437 ActionExecutionFunctionExceptionHandler.handleActionExecutionExceptionFromSkykey
    fn handle_action_execution_exception_from_skykey(
        &mut self,
        key: &SkyKey,
        e: &ActionExecutionException,
    ) {
        if let Some(ArtifactKey(artifact)) = key.downcast::<ArtifactKey>() {
            self.handle_action_execution_exception_per_artifact(artifact, e);
            return;
        }
        let associated_inputs = self.associated_inputs(key);
        if associated_inputs.is_empty() {
            // This can happen if an action prunes its inputs, e.g. the way StarlarkAction
            // implements unused_inputs_list. An input may no longer be present in getInputs(),
            // but its generating action could still be a Skyframe dependency because Skyframe
            // eagerly adds a dep group to a dirty node if all prior dep groups are clean. If the
            // pruned input is in error, it propagates during error bubbling, and we reach this
            // point.
            if self.first_action_execution_exception.is_none() {
                self.first_action_execution_exception = Some(e.clone());
                self.transitive_causes.push(e.get_root_causes().to_vec());
            }
        } else {
            for input in &associated_inputs {
                self.handle_action_execution_exception_per_artifact(input, e);
            }
        }
    }

    // Java: com/google/devtools/build/lib/skyframe/ActionExecutionFunction.java:1466 ActionExecutionFunctionExceptionHandler.handleSourceArtifactExceptionFromSkykey
    fn handle_source_artifact_exception_from_skykey(
        &mut self,
        key: &SkyKey,
        e: &SourceArtifactException,
    ) {
        let source_artifact = key
            .downcast::<ArtifactKey>()
            .map(|k| &k.0)
            .filter(|artifact| artifact.is_source_artifact());
        let Some(artifact) = source_artifact else {
            // bugReporter.logUnexpected("Unexpected SourceArtifactException for key: %s, %s").
            self.missing_artifact_causes.push(LabelCause {
                label: self
                    .action
                    .get_owner()
                    .get_label()
                    .cloned()
                    .unwrap_or_else(|| {
                        panic!(
                            "NullPointerException: label of the owner of {}",
                            self.action.pretty_print()
                        )
                    }),
                failure_detail: e.get_failure_detail().clone(),
            });
            return;
        };

        if (self.is_mandatory_input)(artifact) {
            self.missing_artifact_causes
                .push(create_label_cause_null_owner_ok(
                    artifact,
                    e.get_failure_detail(),
                    self.action.get_owner().get_label(),
                ));
        }
    }

    // Java: com/google/devtools/build/lib/skyframe/ActionExecutionFunction.java:1486 ActionExecutionFunctionExceptionHandler.accumulateMissingFileArtifactValue
    fn accumulate_missing_file_artifact_value(
        &mut self,
        input: &Artifact,
        value: &MissingArtifactValue,
    ) {
        // createLabelCause: a bug report if the input has no owner, then
        // createLabelCauseNullOwnerOk.
        self.missing_artifact_causes
            .push(create_label_cause_null_owner_ok(
                input,
                value.get_failure_detail(),
                self.action.get_owner().get_label(),
            ));
    }

    /// `Err` if there is any accumulated exception from the inputs.
    // Java: com/google/devtools/build/lib/skyframe/ActionExecutionFunction.java:1493 ActionExecutionFunctionExceptionHandler.maybeThrowException
    fn maybe_throw_exception(&mut self) -> Result<(), ActionExecutionException> {
        for missing_input in &self.missing_artifact_causes {
            self.skyframe_action_executor.print_error(
                missing_input.get_message(),
                self.action,
                None,
            );
        }
        // We need to rethrow the first exception because it can contain a useful error message.
        if let Some(first_action_execution_exception) = &self.first_action_execution_exception {
            if self.missing_artifact_causes.is_empty() && self.transitive_causes.len() == 1 {
                // In the case a single action failed, just propagate the exception upward. This
                // avoids having to copy the root causes to the upwards transitive closure.
                return Err(first_action_execution_exception.clone());
            }
            // NestedSetBuilder.<Cause>stableOrder().addAll(missingArtifactCauses), then
            // addTransitive of each transitive cause set; a cause in several sets is kept once,
            // as a NestedSet does.
            let mut all_causes: Vec<RootCause> = Vec::new();
            let missing = self
                .missing_artifact_causes
                .iter()
                .cloned()
                .map(RootCause::Label);
            for cause in missing.chain(self.transitive_causes.iter().flatten().cloned()) {
                if !all_causes.contains(&cause) {
                    all_causes.push(cause);
                }
            }
            // PORT: firstActionExecutionException.getCause() is kept as its message only by
            // blaze_exec's ActionExecutionException; with_root_causes does not take one.
            return Err(ActionExecutionException::with_root_causes(
                first_action_execution_exception.get_message().to_string(),
                Some(FailedActionInfo::of(self.action)),
                all_causes,
                first_action_execution_exception.is_catastrophe(),
                first_action_execution_exception
                    .get_failure_detail()
                    .cloned(),
            ));
        }

        if !self.missing_artifact_causes.is_empty() {
            return Err(self.throw_source_error_exception());
        }
        Ok(())
    }

    // Java: com/google/devtools/build/lib/skyframe/ActionExecutionFunction.java:1523 ActionExecutionFunctionExceptionHandler.handleActionExecutionExceptionPerArtifact
    fn handle_action_execution_exception_per_artifact(
        &mut self,
        input: &Artifact,
        e: &ActionExecutionException,
    ) {
        if (self.is_mandatory_input)(input) {
            // Prefer a catastrophic exception as the one we propagate.
            let replace = match &self.first_action_execution_exception {
                None => true,
                Some(first) => !first.is_catastrophe() && e.is_catastrophe(),
            };
            if replace {
                self.first_action_execution_exception = Some(e.clone());
            }
            self.transitive_causes.push(e.get_root_causes().to_vec());
        }
    }

    /// Called when there are no action execution errors (whose reporting hides missing sources),
    /// but there was at least one missing/io exception-triggering source artifact.
    // Java: com/google/devtools/build/lib/skyframe/ActionExecutionFunction.java:1584 ActionExecutionFunction.throwSourceErrorException
    fn throw_source_error_exception(&self) -> ActionExecutionException {
        let source_artifact_error_causes: Vec<&FailureDetail> = self
            .missing_artifact_causes
            .iter()
            .map(|cause| &cause.failure_detail)
            .collect();
        let (code, message) =
            create_source_error_code_and_message_of(&source_artifact_error_causes);
        // NestedSetBuilder.wrap(Order.STABLE_ORDER, sourceArtifactErrorCauses)
        let root_causes: Vec<RootCause> = self
            .missing_artifact_causes
            .iter()
            .cloned()
            .map(RootCause::Label)
            .collect();
        let ex = ActionExecutionException::with_root_causes(
            message,
            Some(FailedActionInfo::of(self.action)),
            root_causes,
            /* catastrophe= */ false,
            code.get_failure_detail().cloned(),
        );
        self.skyframe_action_executor
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
    use blaze_exec::actions::OutputChecker;
    use blaze_exec::actions::action_cache_checker::{
        ActionCacheChecker, ActionCacheCheckerCacheConfig, NoProxies, TrustAll, TrustLocalOnly,
    };
    use blaze_exec::actions::cache::{ActionCache, CompactPersistentActionCache};
    use blaze_exec::analysis::actions::file_write_action_context::FileWriteActionContext;
    use blaze_exec::exec::action_execution_context::{ActionInputPrefetcher, RemoteOutputTree};
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
            self.evaluate_with(actions, cache, output, &RemoteSetup::default())
        }

        /// [`Fixture::evaluate`] with the remote module's output service (see [`RemoteSetup`]).
        fn evaluate_with(
            &self,
            actions: &[ActionRef],
            cache: &Arc<dyn ActionCache>,
            output: &Artifact,
            remote: &RemoteSetup,
        ) -> (blaze_skyframe::EvaluationResult, Arc<StoredEventHandler>) {
            self.evaluate_roots_with(actions, cache, &[output], remote)
        }

        /// [`Fixture::evaluate_with`] for the generating actions of several outputs, in one
        /// evaluation (one build).
        fn evaluate_roots_with(
            &self,
            actions: &[ActionRef],
            cache: &Arc<dyn ActionCache>,
            outputs: &[&Artifact],
            remote: &RemoteSetup,
        ) -> (blaze_skyframe::EvaluationResult, Arc<StoredEventHandler>) {
            blaze_exec::analysis::actions::register();
            let mut contexts = ActionContexts::default();
            for (type_id, ctx) in blaze_exec::analysis::actions::default_action_contexts("_main") {
                contexts.register_raw(type_id, ctx);
            }
            if let Some(tree) = &remote.tree {
                contexts.register_raw(std::any::TypeId::of::<RemoteOutputTree>(), tree.clone());
                let prefetcher: Arc<dyn ActionInputPrefetcher> = remote
                    .prefetcher
                    .clone()
                    .unwrap_or_else(|| Arc::new(NoPrefetch));
                contexts.register_raw(
                    std::any::TypeId::of::<Arc<dyn ActionInputPrefetcher>>(),
                    Arc::new(prefetcher),
                );
            }
            if let Some(file_write) = &remote.file_write {
                contexts.register_raw(
                    std::any::TypeId::of::<Arc<dyn FileWriteActionContext>>(),
                    Arc::new(file_write.clone()),
                );
            }
            let events = Arc::new(StoredEventHandler::new());
            let checker = Arc::new(ActionCacheChecker::new(
                Some(cache.clone()),
                Arc::new(ArtifactFactoryResolver(None)),
                Box::new(|_: &dyn CacheCheckAction| true),
                Arc::new(NoProxies),
                Some(
                    ActionCacheCheckerCacheConfig::builder()
                        .set_enabled(true)
                        .set_store_output_metadata(remote.output_checker.is_some())
                        .build(),
                ),
            ));
            let executor = Arc::new(
                SkyframeActionExecutor::new(SkyframeActionExecutorConfig {
                    exec_root: self.exec_root.clone(),
                    action_out_dir: self
                        .exec_root
                        .get_parent_directory()
                        .unwrap()
                        .get_child("action_outs"),
                    external_directory: self
                        .exec_root
                        .get_parent_directory()
                        .unwrap()
                        .get_child("external"),
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
                    output_checker: remote.output_checker.clone(),
                })
                .with_lost_inputs_policy(remote.lost_inputs_policy.clone()),
            );
            let mut builder = EvaluatorBuilder::new();
            // SequencedSkyframeExecutor.getGraphInconsistencyReceiverForCommand (as
            // execution_phase::prepare_execution).
            if remote
                .lost_inputs_policy
                .as_ref()
                .is_some_and(|p| p.rewinding_enabled)
            {
                builder = builder.graph_inconsistency_receiver(Arc::new(
                    crate::rewinding::rewindable_graph_inconsistency_receiver::RewindableGraphInconsistencyReceiver::new(),
                ));
            }
            // Java's TimestampBuilderTestCase registers FILE_STATE and FILE with ARTIFACT, which
            // requests the FILE value of a source artifact.
            let _ = remote.loading;
            {
                let root = self.exec_root.get_parent_directory().unwrap();
                let config =
                    blaze_sky_loading::skyframe_executor::LoadingFunctionsConfig::for_bazel_default(
                        blaze_sky_loading::directories::LoadingDirectories {
                            workspace: self.exec_root.clone(),
                            output_base: root.get_child("output_base"),
                            install_base: root.get_child("install_base"),
                        },
                        Arc::new(NoRepos),
                    );
                builder = blaze_sky_loading::skyframe_executor::register_loading_functions(
                    builder, &config,
                );
                builder = blaze_sky_loading::skyframe_executor::inject_loading_precomputed(
                    builder,
                    &config,
                    blaze_starlark::StarlarkSemantics::from_flags(&Default::default()).unwrap(),
                    IndexMap::new(),
                );
                builder = blaze_sky_loading::precomputed::BUILD_ID.set(builder, String::new());
            }
            let evaluator = crate::register_execution_functions(
                builder,
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
            let roots: Vec<SkyKey> = outputs
                .iter()
                .map(|output| {
                    SkyKey::new(ActionExecutionKey(
                        output.get_generating_action_key().clone(),
                    ))
                })
                .collect();
            let result = evaluator.evaluate(&roots, &ctx).unwrap();
            (result, events)
        }
    }

    /// The remote module's part of a build: `RemoteOutputService` (`output_checker` is its
    /// `getOutputChecker()`, and it stores remote output metadata in the action cache) and the
    /// remote output tree of the build. The default is `LocalOutputService`.
    #[derive(Default)]
    struct RemoteSetup {
        output_checker: Option<Arc<dyn OutputChecker>>,
        tree: Option<Arc<RemoteOutputTree>>,
        /// The `FileWriteActionContext` of the build, when not the local one; with
        /// [`InjectingFileWrite`], file writes are remote cache hits that are not downloaded: the
        /// output's metadata is injected into the action file system (as
        /// `RemoteExecutionService.downloadOutputs` does) and no local file is written.
        file_write: Option<Arc<dyn FileWriteActionContext>>,
        /// `rewindingEnabled` / `invocationRetriesEnabled` and the `LostInputsEvent` handler.
        lost_inputs_policy: Option<blaze_exec::exec::executor_setup::LostInputsPolicy>,
        /// Registers the loading functions (FILE, DIRECTORY_LISTING, PACKAGE_LOOKUP, ...) with
        /// the exec root as the workspace: a source directory input needs them
        /// (RecursiveFilesystemTraversalFunction).
        loading: bool,
        /// The input fetcher of the action file systems (`ActionInputPrefetcher.NONE` when
        /// `None`).
        prefetcher: Option<Arc<dyn ActionInputPrefetcher>>,
    }

    /// No external repository (the loading functions of [`RemoteSetup::loading`]).
    struct NoRepos;

    impl blaze_sky_loading::repos::ExternalRepos for NoRepos {
        fn compute_repository_mapping(
            &self,
            _repository_name: &blaze_cmdline::RepositoryName,
            _env: &mut blaze_skyframe::Environment<'_>,
        ) -> Result<Option<blaze_sky_loading::repos::RepositoryMappingValue>, ComputeError>
        {
            Ok(Some(
                blaze_sky_loading::repos::RepositoryMappingValue::value_for_empty_root_module(),
            ))
        }

        fn compute_repository_directory(
            &self,
            repository_name: &blaze_cmdline::RepositoryName,
            _env: &mut blaze_skyframe::Environment<'_>,
        ) -> Result<Option<blaze_sky_loading::repos::RepositoryDirectoryValue>, ComputeError>
        {
            Ok(Some(
                blaze_sky_loading::repos::RepositoryDirectoryValue::Failure {
                    error_msg: format!("Repository '{}' is not defined", repository_name),
                },
            ))
        }
    }

    /// A `FileWriteActionContext` whose action loses an input: a `LostInputsExecException`
    /// for `lost` (a prefetch that found its blob missing in the remote cache).
    struct LosingFileWrite {
        lost: Artifact,
        digest: String,
    }

    impl FileWriteActionContext for LosingFileWrite {
        fn write_output_to_file_with_output(
            &self,
            _action: &dyn blaze_actions::actions::action::Action,
            _ctx: &dyn blaze_exec::exec::action_execution_context::ActionExecutionContext,
            _deterministic_writer: &dyn blaze_actions::analysis::actions::abstract_file_write_action::DeterministicWriter,
            _make_executable: bool,
            _is_remotable: bool,
            _output: &Artifact,
        ) -> Result<Vec<blaze_exec::actions::SpawnResult>, blaze_exec::actions::ExecException>
        {
            let mut lost = blaze_exec::actions::LostArtifacts::default();
            lost.by_digest
                .entry(self.digest.clone())
                .or_default()
                .insert(blaze_exec::actions::action_adapters::artifact_input(
                    &self.lost,
                ));
            Err(blaze_exec::actions::LostInputsExecException::create(lost))
        }
    }

    /// `ActionInputPrefetcher.NONE`.
    struct NoPrefetch;

    impl ActionInputPrefetcher for NoPrefetch {
        fn prefetch_files(
            &self,
            _action: &dyn blaze_exec::actions::ActionExecutionMetadata,
            _inputs: &[blaze_exec::actions::ActionInputRef],
            _metadata_provider: &dyn blaze_exec::actions::InputMetadataProvider,
        ) -> std::io::Result<()> {
            Ok(())
        }
    }

    /// A `FileWriteActionContext` whose writes are remote: the content's metadata is injected
    /// into the action file system instead of writing the file.
    struct InjectingFileWrite;

    /// A `FileWriteActionContext` for an action result that lists, besides the output of the
    /// action writing `injector`, an extra output file at `victim` (the exec path of another
    /// action's output) with a digest of its choice; every other write is local.
    struct ForgingFileWrite {
        injector: Artifact,
        victim: Artifact,
        local: Arc<dyn FileWriteActionContext>,
    }

    impl FileWriteActionContext for ForgingFileWrite {
        fn write_output_to_file_with_output(
            &self,
            action: &dyn blaze_actions::actions::action::Action,
            ctx: &dyn blaze_exec::exec::action_execution_context::ActionExecutionContext,
            deterministic_writer: &dyn blaze_actions::analysis::actions::abstract_file_write_action::DeterministicWriter,
            make_executable: bool,
            is_remotable: bool,
            output: &Artifact,
        ) -> Result<Vec<blaze_exec::actions::SpawnResult>, blaze_exec::actions::ExecException>
        {
            if output != &self.injector {
                return self.local.write_output_to_file_with_output(
                    action,
                    ctx,
                    deterministic_writer,
                    make_executable,
                    is_remotable,
                    output,
                );
            }
            let fs = ctx
                .get_remote_action_file_system()
                .expect("an action file system");
            let result = InjectingFileWrite.write_output_to_file_with_output(
                action,
                ctx,
                deterministic_writer,
                make_executable,
                is_remotable,
                output,
            );
            fs.inject_remote_file(
                self.victim.get_path().as_fragment(),
                vec![0xAB; 32],
                7,
                None,
                false,
            )
            .unwrap();
            result
        }
    }

    impl FileWriteActionContext for InjectingFileWrite {
        fn write_output_to_file_with_output(
            &self,
            _action: &dyn blaze_actions::actions::action::Action,
            ctx: &dyn blaze_exec::exec::action_execution_context::ActionExecutionContext,
            deterministic_writer: &dyn blaze_actions::analysis::actions::abstract_file_write_action::DeterministicWriter,
            _make_executable: bool,
            _is_remotable: bool,
            output: &Artifact,
        ) -> Result<Vec<blaze_exec::actions::SpawnResult>, blaze_exec::actions::ExecException>
        {
            use sha2::Digest as _;
            let mut content = Vec::new();
            deterministic_writer.write_to(&mut content).unwrap();
            ctx.get_remote_action_file_system()
                .expect("an action file system")
                .inject_remote_file(
                    output.get_path().as_fragment(),
                    sha2::Sha256::digest(&content).to_vec(),
                    content.len() as i64,
                    None,
                    false,
                )
                .unwrap();
            Ok(vec![])
        }
    }

    impl Fixture {
        /// Like [`Fixture::build`] with [`Fixture::evaluate_with`].
        fn build_with(
            &self,
            actions: &[ActionRef],
            cache: &Arc<dyn ActionCache>,
            output: &Artifact,
            remote: &RemoteSetup,
        ) -> BuildCounts {
            let (result, events) = self.evaluate_with(actions, cache, output, remote);
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
    }

    /// Build without the bytes, second build: the first build's output exists only remotely (its
    /// metadata came from the remote output tree and the local file is gone). With
    /// `RemoteOutputService` the action cache holds the remote metadata and the output checker
    /// trusts it, so the second build is an action cache hit; with `LocalOutputService`
    /// (`TRUST_LOCAL_ONLY`, no output metadata in the cache) the action runs again. Java:
    /// `ExecutionTool.createBuilder` (`setStoreOutputMetadata(outputService
    /// .shouldStoreRemoteOutputMetadataInActionCache())`) and `SkyframeActionExecutor
    /// .checkActionCache` (`outputService.getOutputChecker()`).
    #[test]
    fn remote_output_metadata_in_the_action_cache_makes_a_second_build_a_cache_hit() {
        for (output_checker, expected) in [
            (
                Some(Arc::new(TrustAll) as Arc<dyn OutputChecker>),
                NOT_REBUILT,
            ),
            (
                Some(Arc::new(TrustLocalOnly) as Arc<dyn OutputChecker>),
                BUILT,
            ),
            (None, BUILT),
        ] {
            let f = Fixture::new();
            let hello = f.create_derived_artifact("hello");
            let actions = f.register_actions(vec![write_action(&f, &hello, "content")]);
            let cache = f.create_cache("cache");

            // Build 1: the action is a remote cache hit whose output is not downloaded: its
            // metadata is injected into the action file system, which the output metadata
            // store consults before the local file system.
            let first = RemoteSetup {
                output_checker: output_checker.clone(),
                tree: Some(Arc::new(RemoteOutputTree::new())),
                file_write: Some(Arc::new(InjectingFileWrite)),
                lost_inputs_policy: None,
                loading: false,
                prefetcher: None,
            };
            assert_eq!(f.build_with(&actions, &cache, &hello, &first), BUILT);
            // Not downloaded: no local file.
            assert!(!hello.get_path().exists());

            // Build 2, a new command: an empty remote output tree.
            let second = RemoteSetup {
                output_checker: output_checker.clone(),
                tree: Some(Arc::new(RemoteOutputTree::new())),
                file_write: Some(Arc::new(InjectingFileWrite)),
                lost_inputs_policy: None,
                loading: false,
                prefetcher: None,
            };
            assert_eq!(
                f.build_with(&actions, &cache, &hello, &second),
                expected,
                "output checker {:?}",
                output_checker.as_ref().map(|_| ())
            );
        }
    }

    /// R7: an action result may name paths the action does not own. The injection goes into
    /// the injecting action's own `RemoteActionFileSystem` (Java: one per action), which only
    /// that action's output metadata store reads, and only for its declared outputs: another
    /// action that writes the forged path locally keeps the digest of its own output.
    #[test]
    fn injected_metadata_for_another_actions_output_does_not_replace_its_digest() {
        use sha2::Digest as _;
        let f = Fixture::new();
        let a = f.create_derived_artifact("a");
        let b = f.create_derived_artifact("b");
        let actions = f.register_actions(vec![
            write_action(&f, &a, "aaa"),
            write_action(&f, &b, "bbb"),
        ]);
        let cache = f.create_cache("cache");

        let local: Arc<dyn FileWriteActionContext> = {
            let mut found = None;
            for (type_id, ctx) in blaze_exec::analysis::actions::default_action_contexts("_main") {
                if type_id == std::any::TypeId::of::<Arc<dyn FileWriteActionContext>>() {
                    found = Some(
                        (*ctx
                            .downcast::<Arc<dyn FileWriteActionContext>>()
                            .ok()
                            .unwrap())
                        .clone(),
                    );
                }
            }
            found.unwrap()
        };
        let forging: Arc<dyn FileWriteActionContext> = Arc::new(ForgingFileWrite {
            injector: a.clone(),
            victim: b.clone(),
            local,
        });
        let remote = RemoteSetup {
            output_checker: Some(Arc::new(TrustAll) as Arc<dyn OutputChecker>),
            tree: Some(Arc::new(RemoteOutputTree::new())),
            file_write: Some(forging),
            lost_inputs_policy: None,
            loading: false,
            prefetcher: None,
        };
        let (result, _events) = f.evaluate_roots_with(&actions, &cache, &[&a, &b], &remote);
        assert!(!result.has_error(), "{:?}", result.error_map());
        let value_b = result
            .get(&ActionExecutionKey(b.get_generating_action_key().clone()))
            .unwrap();
        let metadata_b = value_b.get_existing_file_artifact_value(&b).unwrap();
        assert!(!metadata_b.is_remote(), "{metadata_b:?}");
        assert_eq!(
            metadata_b.get_digest().unwrap(),
            sha2::Sha256::digest(b"bbb").as_slice()
        );
        // The injecting action's own output is its remote one.
        let value_a = result
            .get(&ActionExecutionKey(a.get_generating_action_key().clone()))
            .unwrap();
        let metadata_a = value_a.get_existing_file_artifact_value(&a).unwrap();
        assert!(metadata_a.is_remote(), "{metadata_a:?}");
    }

    /// R6: an action that lost inputs without action rewinding. With invocation retries
    /// (`--experimental_remote_cache_eviction_retries` > 0) the failure is
    /// `FallbackToBuildRewindingException` (Spawn `REMOTE_CACHE_EVICTED`, exit code 39, which
    /// `BlazeCommandDispatcher` retries) and the lost digests reach the `LostInputsEvent`
    /// handler; without retries it is `GenericActionRewindException`
    /// (`LOST_INPUT_REWINDING_DISABLED`). Texts from the oracle (`scripts/remote/evict-probe`):
    /// "Linking brotli failed: Lost inputs no longer available remotely: _objs/brotli/brotli.pic.o
    /// (0f3d…/40520)" and "... Unexpected lost inputs (pass --rewind_lost_inputs to enable
    /// recovery): _objs/brotli/brotli.pic.o".
    #[test]
    fn lost_inputs_without_rewinding_fail_the_action_as_bazel_does() {
        for retries in [true, false] {
            let f = Fixture::new();
            let lost = f.create_derived_artifact("lost.o");
            let out = f.create_derived_artifact("out");
            let actions = f.register_actions(vec![write_action(&f, &out, "content")]);
            let cache = f.create_cache("cache");
            let seen: Arc<parking_lot::Mutex<Vec<String>>> = Arc::default();
            let seen_in_handler = seen.clone();
            let remote = RemoteSetup {
                output_checker: None,
                tree: Some(Arc::new(RemoteOutputTree::new())),
                file_write: Some(Arc::new(LosingFileWrite {
                    lost: lost.clone(),
                    digest: "0f3d/40520".to_string(),
                })),
                lost_inputs_policy: Some(blaze_exec::exec::executor_setup::LostInputsPolicy {
                    rewinding_enabled: false,
                    invocation_retries_enabled: retries,
                    on_lost_inputs: Arc::new(move |digests: &[String]| {
                        seen_in_handler.lock().extend(digests.iter().cloned())
                    }),
                }),
                loading: false,
                prefetcher: None,
            };
            let (result, events) = f.evaluate_with(&actions, &cache, &out, &remote);
            assert!(result.has_error());
            let errors: Vec<String> = events
                .get_events()
                .iter()
                .filter(|e| e.get_kind() == blaze_events::EventKind::Error)
                .map(|e| e.get_message().to_string())
                .collect();
            let expected = if retries {
                "Lost inputs no longer available remotely: lost.o (0f3d/40520)"
            } else {
                "Unexpected lost inputs (pass --rewind_lost_inputs to enable recovery): lost.o"
            };
            assert!(
                errors
                    .iter()
                    .any(|m| m.ends_with(&format!("failed: {expected}"))),
                "{errors:?}"
            );
            assert_eq!(
                *seen.lock(),
                if retries {
                    vec!["0f3d/40520".to_string()]
                } else {
                    vec![]
                }
            );
        }
    }

    /// A `FileWriteActionContext` that writes remotely ([`InjectingFileWrite`]) and counts the
    /// writes of `counted`.
    struct CountingFileWrite {
        counted: Artifact,
        writes: Arc<std::sync::atomic::AtomicUsize>,
    }

    impl FileWriteActionContext for CountingFileWrite {
        fn write_output_to_file_with_output(
            &self,
            action: &dyn blaze_actions::actions::action::Action,
            ctx: &dyn blaze_exec::exec::action_execution_context::ActionExecutionContext,
            deterministic_writer: &dyn blaze_actions::analysis::actions::abstract_file_write_action::DeterministicWriter,
            make_executable: bool,
            is_remotable: bool,
            output: &Artifact,
        ) -> Result<Vec<blaze_exec::actions::SpawnResult>, blaze_exec::actions::ExecException>
        {
            if output == &self.counted {
                self.writes
                    .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            }
            InjectingFileWrite.write_output_to_file_with_output(
                action,
                ctx,
                deterministic_writer,
                make_executable,
                is_remotable,
                output,
            )
        }
    }

    /// The remote input fetcher of [`FakeRemotePrefetcher`] that loses the input for its first
    /// `losses` prefetches and downloads it afterwards.
    struct LosingPrefetcher {
        losing: FakeRemotePrefetcher,
        downloading: FakeRemotePrefetcher,
        losses: usize,
        prefetches: std::sync::atomic::AtomicUsize,
    }

    impl ActionInputPrefetcher for LosingPrefetcher {
        fn prefetch_files(
            &self,
            action: &dyn blaze_exec::actions::ActionExecutionMetadata,
            inputs: &[blaze_exec::actions::ActionInputRef],
            metadata_provider: &dyn blaze_exec::actions::InputMetadataProvider,
        ) -> std::io::Result<()> {
            let attempt = self
                .prefetches
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if attempt < self.losses {
                self.losing
                    .prefetch_files(action, inputs, metadata_provider)
            } else {
                self.downloading
                    .prefetch_files(action, inputs, metadata_provider)
            }
        }
    }

    fn rewinding_policy(
        seen: Arc<parking_lot::Mutex<Vec<String>>>,
    ) -> blaze_exec::exec::executor_setup::LostInputsPolicy {
        blaze_exec::exec::executor_setup::LostInputsPolicy {
            rewinding_enabled: true,
            invocation_retries_enabled: true,
            on_lost_inputs: Arc::new(move |digests: &[String]| {
                seen.lock().extend(digests.iter().cloned())
            }),
        }
    }

    /// `RewindingTest.generatedHeaderRewound_lostInInputDiscovery`, with the fixture's
    /// discovering action in place of a C++ compile: the input `mod.map` (a remote cache hit that
    /// was not downloaded) is lost when discovery reads it. With `--rewind_lost_inputs` the
    /// action's node returns a `Reset` that rewinds the generating action of `mod.map`
    /// (`ActionRewindStrategy.prepareRewindPlanForLostInputs`), which runs again; the
    /// discovering action then reads the input and the build succeeds. No `LostInputsEvent` is
    /// posted (`checkRewindingEnabled` returns before posting when rewinding is enabled).
    #[test]
    fn generated_header_rewound_lost_in_input_discovery() {
        let f = Fixture::new();
        let lost_digest = "5b1d/6".to_string();
        let prefetcher = LosingPrefetcher {
            losing: FakeRemotePrefetcher {
                exec_root: f.exec_root.clone(),
                content: None,
                lost_digest: lost_digest.clone(),
            },
            downloading: FakeRemotePrefetcher {
                exec_root: f.exec_root.clone(),
                content: Some("a.pcm\n".to_string()),
                lost_digest: String::new(),
            },
            losses: 1,
            prefetches: std::sync::atomic::AtomicUsize::new(0),
        };
        let seen: Arc<parking_lot::Mutex<Vec<String>>> = Arc::default();
        let placeholder = FakeRemotePrefetcher {
            exec_root: f.exec_root.clone(),
            content: None,
            lost_digest: String::new(),
        };
        let (actions, modmap, out, read_content, mut remote) =
            discovery_fixture(&f, placeholder, Some(rewinding_policy(seen.clone())));
        remote.prefetcher = Some(Arc::new(prefetcher));
        let writes = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        remote.file_write = Some(Arc::new(CountingFileWrite {
            counted: modmap.clone(),
            writes: writes.clone(),
        }));
        let cache = f.create_cache("cache");
        let (result, events) = f.evaluate_with(&actions, &cache, &out, &remote);
        assert!(
            !result.has_error(),
            "{:?} {:?}",
            result.error_map(),
            events
                .get_events()
                .iter()
                .map(|e| e.get_message().to_string())
                .collect::<Vec<_>>()
        );
        assert_eq!(read_content.lock().as_deref(), Some("a.pcm\n"));
        assert!(out.get_path().exists());
        // The generating action of the lost input ran twice: once before, once rewound.
        assert_eq!(writes.load(std::sync::atomic::Ordering::SeqCst), 2);
        assert!(seen.lock().is_empty(), "{:?}", seen.lock());
        // No error was reported.
        assert!(
            !events
                .get_events()
                .iter()
                .any(|e| e.get_kind() == blaze_events::EventKind::Error),
            "{:?}",
            events
                .get_events()
                .iter()
                .map(|e| e.get_message().to_string())
                .collect::<Vec<_>>()
        );
    }

    /// `RewindingTest.ineffectiveRewindingResultsInLostInputTooManyTimes`: an input that is
    /// lost again after every rewind fails the action once it has been lost more than
    /// `ActionRewindStrategy.MAX_REPEATED_LOST_INPUTS` (20) times, with Java's message
    /// ("lost input too many times (#21) for the same action. lostInput: <input>, lostInput
    /// digest: <digest>, failedAction: <action>") and `LOST_INPUT_TOO_MANY_TIMES`.
    #[test]
    fn ineffective_rewinding_results_in_lost_input_too_many_times() {
        let f = Fixture::new();
        let prefetcher = FakeRemotePrefetcher {
            exec_root: f.exec_root.clone(),
            content: None,
            lost_digest: "5b1d/6".to_string(),
        };
        let seen: Arc<parking_lot::Mutex<Vec<String>>> = Arc::default();
        let (actions, modmap, out, _read_content, mut remote) =
            discovery_fixture(&f, prefetcher, Some(rewinding_policy(seen.clone())));
        let writes = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        remote.file_write = Some(Arc::new(CountingFileWrite {
            counted: modmap.clone(),
            writes: writes.clone(),
        }));
        let cache = f.create_cache("cache");
        let (result, events) = f.evaluate_with(&actions, &cache, &out, &remote);
        assert!(result.has_error());
        let errors: Vec<String> = events
            .get_events()
            .iter()
            .filter(|e| e.get_kind() == blaze_events::EventKind::Error)
            .map(|e| e.get_message().to_string())
            .collect();
        assert!(
            errors.iter().any(|m| m.contains(&format!(
                "lost input too many times (#{}) for the same action. lostInput: {}, lostInput \
                 digest: 5b1d/6, failedAction: ",
                crate::rewinding::action_rewind_strategy::MAX_REPEATED_LOST_INPUTS + 1,
                modmap
            ))),
            "{errors:?}"
        );
        // The first execution, then one per rewind before the last loss.
        assert_eq!(
            writes.load(std::sync::atomic::Ordering::SeqCst),
            crate::rewinding::action_rewind_strategy::MAX_REPEATED_LOST_INPUTS + 1
        );
        assert!(seen.lock().is_empty());
    }

    /// An input-discovering action whose discovery reads one input in process, as
    /// `CppCompileAction.discoverInputs` reads the modmap input (`FileSystemUtils
    /// .readLinesAsLatin1(actionExecutionContext.getInputPath(modmapInputFile))`, failing with
    /// "failed to read modmap input: <exec path>"); its execution writes `out` locally.
    #[derive(Debug)]
    struct ReadingDiscoveryAction {
        base: blaze_actions::actions::action::AbstractAction,
        read: Artifact,
        out: Artifact,
        discovered: parking_lot::Mutex<bool>,
        read_content: Arc<parking_lot::Mutex<Option<String>>>,
    }

    impl blaze_actions::Action for ReadingDiscoveryAction {
        fn abstract_action(&self) -> &blaze_actions::actions::action::AbstractAction {
            &self.base
        }
        fn as_any(&self) -> &dyn Any {
            self
        }
        fn get_mnemonic(&self) -> String {
            "ReadingDiscovery".to_string()
        }
        fn compute_key(
            &self,
            _action_key_context: &blaze_actions::actions::action_key_context::ActionKeyContext,
            _input_metadata_provider: Option<
                &dyn blaze_actions::actions::action::InputMetadataProvider,
            >,
            fp: &mut blaze_util::Fingerprint,
        ) -> Result<(), blaze_actions::actions::action::ComputeKeyError> {
            fp.add_string("ReadingDiscoveryAction");
            Ok(())
        }
        fn discovers_inputs(&self) -> bool {
            true
        }
        fn inputs_discovered(&self) -> bool {
            *self.discovered.lock()
        }
        fn set_inputs_discovered(&self, inputs_discovered: bool) {
            *self.discovered.lock() = inputs_discovered;
        }
        fn get_allowed_derived_inputs(&self) -> blaze_collect::NestedSet<Artifact> {
            self.get_inputs()
        }
    }

    impl blaze_exec::actions::action_execution::ExecutableAction for ReadingDiscoveryAction {
        fn discover_inputs(
            &self,
            ctx: &dyn blaze_exec::exec::action_execution_context::ActionExecutionContext,
        ) -> Result<
            Option<blaze_collect::NestedSet<Artifact>>,
            blaze_exec::actions::ActionExecutionException,
        > {
            use std::io::Read as _;
            let path = blaze_exec::analysis::actions::get_input_path(ctx, &self.read);
            let mut content = String::new();
            if path
                .get_input_stream()
                .and_then(|mut s| s.read_to_string(&mut content))
                .is_err()
            {
                return Err(blaze_exec::actions::ActionExecutionException::new(
                    format!(
                        "failed to read modmap input: {}",
                        self.read.get_exec_path_string()
                    ),
                    Some(self),
                    false,
                    None,
                ));
            }
            *self.read_content.lock() = Some(content);
            blaze_actions::Action::set_inputs_discovered(self, true);
            Ok(Some(blaze_actions::Action::get_inputs(self)))
        }
        fn execute(
            &self,
            _ctx: &dyn blaze_exec::exec::action_execution_context::ActionExecutionContext,
        ) -> Result<blaze_exec::actions::ActionResult, blaze_exec::actions::ActionExecutionException>
        {
            write_file(&self.out.get_path(), "compiled");
            Ok(blaze_exec::actions::ActionResult::empty())
        }
    }

    /// The remote input fetcher of the action file system: writes the content of a known
    /// input to its local path, or fails with a `BulkTransferException` whose cause is a
    /// missing blob (`CacheNotFoundException`) for the input.
    struct FakeRemotePrefetcher {
        exec_root: Path,
        content: Option<String>,
        lost_digest: String,
    }

    impl ActionInputPrefetcher for FakeRemotePrefetcher {
        fn prefetch_files(
            &self,
            _action: &dyn blaze_exec::actions::ActionExecutionMetadata,
            inputs: &[blaze_exec::actions::ActionInputRef],
            _metadata_provider: &dyn blaze_exec::actions::InputMetadataProvider,
        ) -> std::io::Result<()> {
            for input in inputs {
                match &self.content {
                    Some(content) => {
                        write_file(&self.exec_root.get_relative(input.get_exec_path()), content)
                    }
                    None => {
                        return Err(std::io::Error::other(
                            blaze_exec::exec::action_execution_context::BulkTransferError {
                                message: format!(
                                    "Missing digest: {} for {}",
                                    self.lost_digest,
                                    input.get_exec_path()
                                ),
                                causes: Some(vec![(
                                    self.lost_digest.clone(),
                                    Some(input.get_exec_path().clone()),
                                )]),
                            },
                        ));
                    }
                }
            }
            Ok(())
        }
    }

    /// Two actions in one build without the bytes: `mod.map` is a remote cache hit that is not
    /// downloaded, and the discovering action reads it during input discovery.
    fn discovery_fixture(
        f: &Fixture,
        prefetcher: FakeRemotePrefetcher,
        lost_inputs_policy: Option<blaze_exec::exec::executor_setup::LostInputsPolicy>,
    ) -> (
        Vec<ActionRef>,
        Artifact,
        Artifact,
        Arc<parking_lot::Mutex<Option<String>>>,
        RemoteSetup,
    ) {
        let modmap = f.create_derived_artifact("mod.map");
        let out = f.create_derived_artifact("mod.o");
        let read_content: Arc<parking_lot::Mutex<Option<String>>> = Arc::default();
        let discovering: ActionRef = Arc::new(ReadingDiscoveryAction {
            base: blaze_actions::actions::action::AbstractAction::new(
                owner(),
                blaze_collect::NestedSetBuilder::stable_order()
                    .add(modmap.clone())
                    .build(),
                [out.clone()],
            ),
            read: modmap.clone(),
            out: out.clone(),
            discovered: parking_lot::Mutex::new(false),
            read_content: read_content.clone(),
        });
        blaze_exec::actions::action_execution::register_executable_action::<ReadingDiscoveryAction>(
        );
        let actions = f.register_actions(vec![write_action(f, &modmap, "a.pcm\n"), discovering]);
        let remote = RemoteSetup {
            output_checker: None,
            tree: Some(Arc::new(RemoteOutputTree::new())),
            file_write: Some(Arc::new(InjectingFileWrite)),
            lost_inputs_policy,
            loading: false,
            prefetcher: Some(Arc::new(prefetcher)),
        };
        (actions, modmap, out, read_content, remote)
    }

    /// Review 2026-11-19 round 3 P2: input discovery runs with the action file system
    /// (`ActionExecutionFunction.java:331-332` creates `state.actionFileSystem` before
    /// discovery; `SkyframeActionExecutor.java:931-952` `forInputDiscovery(...,
    /// actionFileSystem, ...)` and `updateActionFileSystemContext`), so an input that exists
    /// only remotely is read through it (downloaded by the input fetcher) during discovery.
    #[test]
    fn input_discovery_reads_a_remote_only_input_through_the_action_file_system() {
        let f = Fixture::new();
        let prefetcher = FakeRemotePrefetcher {
            exec_root: f.exec_root.clone(),
            content: Some("a.pcm\n".to_string()),
            lost_digest: String::new(),
        };
        let (actions, modmap, out, read_content, remote) = discovery_fixture(&f, prefetcher, None);
        let cache = f.create_cache("cache");
        let (result, events) = f.evaluate_with(&actions, &cache, &out, &remote);
        assert!(
            !result.has_error(),
            "{:?} {:?}",
            result.error_map(),
            events
                .get_events()
                .iter()
                .map(|e| e.get_message().to_string())
                .collect::<Vec<_>>()
        );
        assert_eq!(read_content.lock().as_deref(), Some("a.pcm\n"));
        // Downloaded by the fetcher, not written by the (remote) write action.
        assert!(modmap.get_path().exists());
        assert!(out.get_path().exists());
    }

    /// Review 2026-11-19 round 3 P2: an input lost during discovery
    /// (`SkyframeActionExecutor.java:967-989`): the discovery failure ("failed to read modmap
    /// input") gives way to the lost inputs of the action file system, which fail the action as
    /// `--norewind_lost_inputs` prescribes (`ActionExecutionFunction.handleLostInputs` →
    /// `ActionRewindStrategy.checkRewindingEnabled`); the discovery error is not reported.
    #[test]
    fn input_lost_during_discovery_fails_as_without_rewinding() {
        let f = Fixture::new();
        let prefetcher = FakeRemotePrefetcher {
            exec_root: f.exec_root.clone(),
            content: None,
            lost_digest: "5b1d/6".to_string(),
        };
        let (actions, _modmap, out, read_content, remote) = discovery_fixture(
            &f,
            prefetcher,
            Some(blaze_exec::exec::executor_setup::LostInputsPolicy {
                rewinding_enabled: false,
                invocation_retries_enabled: false,
                on_lost_inputs: Arc::new(|_: &[String]| {}),
            }),
        );
        let cache = f.create_cache("cache");
        let (result, events) = f.evaluate_with(&actions, &cache, &out, &remote);
        assert!(result.has_error());
        assert!(read_content.lock().is_none());
        let errors: Vec<String> = events
            .get_events()
            .iter()
            .filter(|e| e.get_kind() == blaze_events::EventKind::Error)
            .map(|e| e.get_message().to_string())
            .collect();
        assert!(
            errors.iter().any(|m| m.ends_with(
                "failed: Unexpected lost inputs (pass --rewind_lost_inputs to enable recovery): \
                 mod.map"
            )),
            "{errors:?}"
        );
        assert!(
            !errors
                .iter()
                .any(|m| m.contains("failed to read modmap input")),
            "{errors:?}"
        );
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

    /// `SkyframeActionExecutor.checkForUnsoundDirectoryOutput` fails an output that is a
    /// directory although it was not declared as one (Java text, DirectoryArtifactWarningTest).
    /// `checkForUnsoundDirectoryInputs` does not warn about the source directory input: Bazel
    /// tracks source directories by default (TrackSourceDirectoriesFlag).
    ///
    /// PORT: Java's DirectoryArtifactWarningTest runs with
    /// `-DBAZEL_TRACK_SOURCE_DIRECTORIES=0` and expects the input warning ("input 'x/dir' of //x:x
    /// is a directory; dependency checking of directories is unsound"); Blaze has no JVM
    /// property, so the warning cannot be produced.
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
        let (result, events) = f.evaluate_with(
            &actions,
            &cache,
            &link,
            &RemoteSetup {
                loading: true,
                prefetcher: None,
                ..RemoteSetup::default()
            },
        );
        assert!(result.has_error());
        let messages: Vec<(blaze_events::EventKind, String)> = events
            .get_events()
            .iter()
            .map(|e| (e.get_kind(), e.get_message().to_string()))
            .collect();
        assert!(
            !messages.contains(&(
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
