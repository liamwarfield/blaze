//! `BuildDriverFunction`: drives the analysis and execution of a top-level target (Skymeld).

use std::sync::Arc;

use blaze_skyframe::{ComputeError, Environment, SkyKey};

use crate::analysis_seam::{AnalysisSeam, ConfiguredTargetView};
use crate::events::{TestAnalyzedEvent, TopLevelTargetPendingExecutionEvent};
use crate::keys::{BuildDriverKey, TargetCompletionKey, TestCompletionKey, TestType};

/// The value of a [`BuildDriverKey`]: the analyzed top-level target and whether it was skipped.
// Java: com/google/devtools/build/lib/skyframe/BuildDriverValue.java:24 BuildDriverValue
#[derive(Clone, Debug)]
pub struct BuildDriverValue {
    pub configured_target: Arc<dyn ConfiguredTargetView>,
    pub skipped: bool,
}

/// `BuildDriverFunction.compute` for configured targets.
// Java: com/google/devtools/build/lib/skyframe/BuildDriverFunction.java:155 BuildDriverFunction.compute
// PORT: BUILD_ID, action conflict checks (checkActionConflicts), target compatibility
// (isConfiguredTargetCompatible: incompatible targets are skipped), extra actions, top-level
// aspects and the TopLevelStatusEvents other than TopLevelTargetPendingExecutionEvent are the
// analysis side's (blaze_sky_top) and are not ported here.
pub fn compute_build_driver(
    seam: &dyn AnalysisSeam,
    key: &BuildDriverKey,
    env: &mut Environment<'_>,
) -> Result<Option<BuildDriverValue>, ComputeError> {
    let Some(configured_target) = seam.get_configured_target(&key.action_lookup_key, env)? else {
        return Ok(None);
    };
    if env.values_missing() {
        return Ok(None);
    }
    let test_type = key.test_type;
    env.listener()
        .post(Arc::new(TopLevelTargetPendingExecutionEvent {
            label: configured_target.get_label(),
            is_test: test_type.is_test(),
        }));
    // Java: com/google/devtools/build/lib/skyframe/BuildDriverFunction.java:590 BuildDriverFunction.requestConfiguredTargetExecution
    if test_type != TestType::NOT_TEST {
        // Java: com/google/devtools/build/lib/skyframe/BuildDriverFunction.java:615 BuildDriverFunction.requestConfiguredTargetExecution
        env.listener().post(Arc::new(TestAnalyzedEvent {
            configured_target: configured_target.clone(),
            configuration_checksum: configured_target
                .get_configuration_checksum()
                .expect("NullPointerException: buildConfigurationValue"),
            configuration_mnemonic: configured_target
                .get_configuration_mnemonic()
                .expect("NullPointerException: buildConfigurationValue"),
            is_skipped: false,
        }));
    }
    let completion_key: SkyKey = match test_type {
        TestType::NOT_TEST => SkyKey::new(TargetCompletionKey {
            action_lookup_key: key.action_lookup_key.clone(),
            output_groups: key.output_groups.clone(),
            will_test: false,
        }),
        TestType::PARALLEL => SkyKey::new(TestCompletionKey {
            configured_target_key: key.action_lookup_key.clone(),
            output_groups: key.output_groups.clone(),
            exclusive_testing: false,
        }),
        // Exclusive tests will be run with sequential Skyframe evaluations afterwards.
        TestType::EXCLUSIVE | TestType::EXCLUSIVE_IF_LOCAL => SkyKey::new(TargetCompletionKey {
            action_lookup_key: key.action_lookup_key.clone(),
            output_groups: key.output_groups.clone(),
            will_test: true,
        }),
    };
    // Java: com/google/devtools/build/lib/skyframe/BuildDriverFunction.java:650 BuildDriverFunction.declareDependenciesAndCheckValues
    let result = env.get_values_and_exceptions(std::slice::from_ref(&completion_key))?;
    let _ = result.get(&completion_key);
    if env.values_missing() {
        return Ok(None);
    }
    Ok(Some(BuildDriverValue {
        configured_target,
        skipped: false,
    }))
}
