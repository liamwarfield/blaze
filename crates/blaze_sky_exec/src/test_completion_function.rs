//! `TestCompletionFunction`: the `TEST_COMPLETION` SkyFunction.

use blaze_exec::actions::ActionExecutionException;
use blaze_skyframe::{ComputeError, Environment, SkyKey};

use crate::analysis_seam::AnalysisSeam;
use crate::keys::{ActionExecutionKey, TargetCompletionKey, TestCompletionKey, artifact_key};

/// The (marker) value of a completed test target.
// Java: com/google/devtools/build/lib/skyframe/TestCompletionValue.java:33 TestCompletionValue
#[derive(Clone, Debug, Default)]
pub struct TestCompletionValue;

/// Hook for `postTestResultEventsForBuiltTestThatCouldNotBeRun`: posts `FAILED_TO_BUILD`
/// `TestAttempt`/`TestResult` events for a `TestRunnerAction` that could not run. Set by the
/// crate that owns `TestRunnerAction`.
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
                    // PORT: Java checks `detailedExitCode.getExitCode() == BUILD_FAILURE`; every
                    // ActionExecutionException a TestRunnerAction throws for a built test maps to
                    // BUILD_FAILURE except catastrophes.
                    if !e.is_catastrophe()
                        && let Some(report) = unrunnable
                    {
                        report(env, &k.0, &e);
                    } else {
                        return Ok(None);
                    }
                }
            }
        }
    }
    Ok(Some(TestCompletionValue))
}
