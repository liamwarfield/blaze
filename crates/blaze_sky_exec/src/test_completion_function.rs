//! `TestCompletionFunction`: the `TEST_COMPLETION` SkyFunction.

use std::sync::Arc;

use blaze_actions::ActionLookupData;
use blaze_exec::actions::ActionExecutionException;
use blaze_exec::analysis::test::as_test_runner_action;
use blaze_exec::analysis::test::test_attempt::TestAttempt;
use blaze_exec::analysis::test::test_result::TestResult;
use blaze_protos::blaze::{BlazeTestStatus, TestResultData};
use blaze_protos::failure_details::{execution, failure_detail};
use blaze_skyframe::{ComputeError, Environment, SkyKey};
use blaze_util::{DetailedExitCode, ExitCode};

use crate::analysis_seam::AnalysisSeam;
use crate::keys::{ActionExecutionKey, TargetCompletionKey, TestCompletionKey, artifact_key};

/// The (marker) value of a completed test target.
// Java: com/google/devtools/build/lib/skyframe/TestCompletionValue.java:33 TestCompletionValue
#[derive(Clone, Debug, Default)]
pub struct TestCompletionValue;

/// Hook for `postTestResultEventsForBuiltTestThatCouldNotBeRun`: posts `FAILED_TO_BUILD`
/// `TestAttempt`/`TestResult` events for a `TestRunnerAction` that could not run.
///
/// PORT: optional override; when none is given, [`compute_test_completion`] posts the events
/// itself through the seam and `blaze_exec::analysis::test::as_test_runner_action`.
pub type UnrunnableTestReporter = dyn Fn(&mut Environment<'_>, &blaze_actions::ActionLookupData, &ActionExecutionException)
    + Send
    + Sync;

/// `TestCompletionFunction.compute`.
// Java: com/google/devtools/build/lib/skyframe/TestCompletionFunction.java:50 TestCompletionFunction.compute
pub fn compute_test_completion(
    seam: &dyn AnalysisSeam,
    unrunnable: Option<&UnrunnableTestReporter>,
    key: &TestCompletionKey,
    env: &mut Environment<'_>,
) -> Result<Option<TestCompletionValue>, ComputeError> {
    let ct_key = &key.configured_target_key;
    if env
        .get_value(TargetCompletionKey {
            action_lookup_key: ct_key.clone(),
            output_groups: key.output_groups.clone(),
            will_test: true,
        })?
        .is_none()
    {
        return Ok(None);
    }
    let Some(ct) = seam.get_configured_target(ct_key, env)? else {
        return Ok(None);
    };
    let test_status_artifacts = ct
        .get_test_params()
        .map(|p| p.test_status_artifacts)
        .unwrap_or_default();
    if key.exclusive_testing {
        // Request test execution iteratively if testing exclusively.
        for test_artifact in &test_status_artifacts {
            env.get_value(ActionExecutionKey(
                test_artifact.get_generating_action_key().clone(),
            ))?;
            if env.values_missing() {
                return Ok(None);
            }
        }
    } else {
        let sky_keys: Vec<SkyKey> = test_status_artifacts.iter().map(artifact_key).collect();
        let result = env.get_values_and_exceptions(&sky_keys)?;
        if env.values_missing() {
            return Ok(None);
        }
        for action_key in &sky_keys {
            let k = action_key
                .downcast::<ActionExecutionKey>()
                .expect("test status artifacts are derived");
            match result
                .get_or_throw_with(k, |e| e.downcast_ref::<ActionExecutionException>().cloned())
            {
                Ok(Some(_)) => {}
                Ok(None) => return Ok(None),
                Err(e) => {
                    // Java: `detailedExitCode.getExitCode().equals(ExitCode.BUILD_FAILURE) &&
                    // ctValue instanceof ActionLookupValue` (a rule's ConfiguredTargetValue
                    // always is one).
                    if is_build_failure(&e) {
                        match unrunnable {
                            Some(report) => report(env, &k.0, &e),
                            None => {
                                if !post_test_result_events_for_built_test_that_could_not_be_run(
                                    seam, env, &k.0, &e,
                                )? {
                                    return Ok(None);
                                }
                            }
                        }
                    } else {
                        return Ok(None);
                    }
                }
            }
        }
    }
    Ok(Some(TestCompletionValue))
}

/// `e.getDetailedExitCode().getExitCode().equals(ExitCode.BUILD_FAILURE)`.
fn is_build_failure(e: &ActionExecutionException) -> bool {
    match e.get_failure_detail() {
        Some(failure_detail) => {
            DetailedExitCode::get_exit_code_of(failure_detail) == ExitCode::BUILD_FAILURE
        }
        // PORT: Java's ActionExecutionException always carries a DetailedExitCode; Blaze's may
        // lack one. Such an exception is BUILD_FAILURE unless it is a catastrophe (the
        // exit-code rule `ActionExecutionException` applies when it builds its own code).
        None => !e.is_catastrophe(),
    }
}

/// Posts events for test actions that could not run despite the fact that the test target built
/// successfully.
///
/// When we run this SkyFunction we will have already built the test executable and its inputs,
/// but we might be unable to run the test. The currently known scenarios where this happens are:
///
/// 1. A failure to build the exec-configured attributes providing inputs to the
///    `TestRunnerAction` such as `$test_runtime`, `$test_wrapper`, `test_setup_script` and others.
/// 2. The test strategy throws an `ExecException` prior to running the test, for example when
///    some sort of validation fails.
/// 3. The test action observes lost input(s) and initiates action rewinding, but the lost input(s)
///    fail to build. Note that this implies action nondeterminism, since the lost input(s) were
///    previously built successfully.
///
/// In these scenarios, we do not get to use any `TestStrategy` that is responsible for posting
/// `TestAttempt` and `TestResult` events. We need to post minimal events here indicating the test
/// `FAILED_TO_BUILD`.
///
/// PORT: Java reads the action from the `ConfiguredTargetValue` it already holds; here it comes
/// through the seam (`Ok(false)` when that value is missing).
// Java: com/google/devtools/build/lib/skyframe/TestCompletionFunction.java:146 TestCompletionFunction.postTestResultEventsForBuiltTestThatCouldNotBeRun
fn post_test_result_events_for_built_test_that_could_not_be_run(
    seam: &dyn AnalysisSeam,
    env: &mut Environment<'_>,
    action_key: &ActionLookupData,
    e: &ActionExecutionException,
) -> Result<bool, ComputeError> {
    let mut status = BlazeTestStatus::FailedToBuild;
    if let Some(failure_detail) = e.get_failure_detail()
        && let Some(failure_detail::Category::Execution(execution)) = &failure_detail.category
        && execution.code == execution::Code::ActionNotUpToDate as i32
    {
        status = BlazeTestStatus::NoStatus;
    }
    let Some(action_lookup_value) =
        seam.get_action_lookup_value(action_key.get_action_lookup_key(), env)?
    else {
        return Ok(false);
    };
    let action = action_lookup_value.get_action(action_key.get_action_index());
    let test_runner_action = as_test_runner_action(action.as_ref()).unwrap_or_else(|| {
        panic!(
            "ClassCastException: {} cannot be cast to TestRunnerAction",
            action.get_mnemonic()
        )
    });
    let mut test_data = TestResultData::default();
    test_data.set_status(status);
    env.listener()
        .post(Arc::new(TestAttempt::for_unstartable_test_result(
            test_runner_action.clone(),
            &test_data,
        )));
    env.listener().post(Arc::new(TestResult::new(
        test_runner_action,
        test_data,
        Vec::new(),
        /* cached= */ false,
        /* exec_root= */ None,
        e.get_failure_detail().cloned(),
    )));
    Ok(true)
}
