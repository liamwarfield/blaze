//! `BuildDriverFunction`: drives the analysis and execution of a top-level target (Skymeld).

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::sync::Arc;

use blaze_events::Postable;
use blaze_skyframe::{ComputeError, Environment, SkyKey};
use parking_lot::Mutex;

use crate::analysis_seam::{AnalysisSeam, ConfiguredAspectView, ConfiguredTargetView};
use crate::events::{TestAnalyzedEvent, TopLevelTargetPendingExecutionEvent};
use crate::keys::{
    AspectCompletionKey, BuildDriverKey, TargetCompletionKey, TestCompletionKey, TestType,
};

/// The analyzed top-level value of a [`BuildDriverKey`]: a configured target or the aspects of a
/// `TopLevelAspectsValue`.
#[derive(Clone)]
pub enum TopLevelSkyValue {
    /// A `ConfiguredTargetValue`.
    ConfiguredTarget(Arc<dyn ConfiguredTargetView>),
    /// A `TopLevelAspectsValue` (`getTopLevelAspectsMap()`, in order).
    TopLevelAspects(Vec<Arc<dyn ConfiguredAspectView>>),
}

impl fmt::Debug for TopLevelSkyValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TopLevelSkyValue::ConfiguredTarget(ct) => write!(f, "{ct:?}"),
            TopLevelSkyValue::TopLevelAspects(aspects) => {
                let keys: Vec<String> = aspects
                    .iter()
                    .map(|a| format!("{}, aspect {}", a.get_label(), a.get_aspect_class_name()))
                    .collect();
                write!(f, "TopLevelAspectsValue{keys:?}")
            }
        }
    }
}

/// The value of a [`BuildDriverKey`]: the analyzed top-level value and whether it was skipped.
// Java: com/google/devtools/build/lib/skyframe/BuildDriverValue.java:24 BuildDriverValue
#[derive(Clone, Debug)]
pub struct BuildDriverValue {
    /// `getLastEvaluatedTopLevelSkyValue()`.
    pub top_level_sky_value: TopLevelSkyValue,
    pub skipped: bool,
}

/// The types of the `TopLevelStatusEvents` this function posts.
// Java: com/google/devtools/build/lib/skyframe/TopLevelStatusEvents.java:241 TopLevelStatusEvents.Type
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[allow(non_camel_case_types)]
enum TopLevelStatusEventsType {
    TOP_LEVEL_TARGET_PENDING_EXECUTION,
    TEST_ANALYZED,
}

/// Drives the analysis and execution of an `ActionLookupKey`, which is wrapped inside a
/// `BuildDriverKey`.
// Java: com/google/devtools/build/lib/skyframe/BuildDriverFunction.java:104 BuildDriverFunction
pub struct BuildDriverFunction {
    seam: Arc<dyn AnalysisSeam>,
    /// Java: `keyToPostedEvents`. A function can run more than once for a key (after a
    /// dependency went missing, and once more in error bubbling); each event is posted once.
    key_to_posted_events: Mutex<HashMap<BuildDriverKey, HashSet<TopLevelStatusEventsType>>>,
}

impl BuildDriverFunction {
    pub fn new(seam: Arc<dyn AnalysisSeam>) -> BuildDriverFunction {
        BuildDriverFunction {
            seam,
            key_to_posted_events: Mutex::new(HashMap::new()),
        }
    }

    // Java: com/google/devtools/build/lib/skyframe/BuildDriverFunction.java:503 BuildDriverFunction.postEventIfNecessary
    fn post_event_if_necessary(
        &self,
        key: &BuildDriverKey,
        env: &Environment<'_>,
        type_: TopLevelStatusEventsType,
        event: Arc<dyn Postable>,
    ) {
        let newly_added = self
            .key_to_posted_events
            .lock()
            .entry(key.clone())
            .or_default()
            .insert(type_);
        if newly_added {
            env.listener().post(event);
        }
    }

    // Java: com/google/devtools/build/lib/skyframe/BuildDriverFunction.java:498 BuildDriverFunction.removeStatesForKey
    fn remove_states_for_key(&self, key: &BuildDriverKey) {
        self.key_to_posted_events.lock().remove(key);
    }

    /// `BuildDriverFunction.compute` for configured targets.
    // Java: com/google/devtools/build/lib/skyframe/BuildDriverFunction.java:155 BuildDriverFunction.compute
    // PORT: BUILD_ID, action conflict checks (checkActionConflicts), target compatibility
    // (isConfiguredTargetCompatible: incompatible targets are skipped), extra actions, top-level
    // aspects and the TopLevelStatusEvents other than TopLevelTargetPendingExecutionEvent are the
    // analysis side's (blaze_sky_top) and are not ported here.
    pub fn compute(
        &self,
        key: &BuildDriverKey,
        env: &mut Environment<'_>,
    ) -> Result<Option<BuildDriverValue>, ComputeError> {
        if key.is_top_level_aspect_driver() {
            return self.compute_top_level_aspects(key, env);
        }
        let Some(configured_target) = self
            .seam
            .get_configured_target(&key.action_lookup_key, env)?
        else {
            return Ok(None);
        };
        if env.values_missing() {
            return Ok(None);
        }
        let test_type = key.test_type;
        self.post_event_if_necessary(
            key,
            env,
            TopLevelStatusEventsType::TOP_LEVEL_TARGET_PENDING_EXECUTION,
            Arc::new(TopLevelTargetPendingExecutionEvent {
                label: configured_target.get_label(),
                is_test: test_type.is_test(),
            }),
        );
        // Java: com/google/devtools/build/lib/skyframe/BuildDriverFunction.java:590 BuildDriverFunction.requestConfiguredTargetExecution
        if test_type != TestType::NOT_TEST {
            // Java: com/google/devtools/build/lib/skyframe/BuildDriverFunction.java:615 BuildDriverFunction.requestConfiguredTargetExecution
            self.post_event_if_necessary(
                key,
                env,
                TopLevelStatusEventsType::TEST_ANALYZED,
                Arc::new(TestAnalyzedEvent {
                    configured_target: configured_target.clone(),
                    configuration_checksum: configured_target
                        .get_configuration_checksum()
                        .expect("NullPointerException: buildConfigurationValue"),
                    configuration_mnemonic: configured_target
                        .get_configuration_mnemonic()
                        .expect("NullPointerException: buildConfigurationValue"),
                    is_skipped: false,
                }),
            );
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
            TestType::EXCLUSIVE | TestType::EXCLUSIVE_IF_LOCAL => {
                SkyKey::new(TargetCompletionKey {
                    action_lookup_key: key.action_lookup_key.clone(),
                    output_groups: key.output_groups.clone(),
                    will_test: true,
                })
            }
        };
        // Java: com/google/devtools/build/lib/skyframe/BuildDriverFunction.java:650 BuildDriverFunction.declareDependenciesAndCheckValues
        let result = env.get_values_and_exceptions(std::slice::from_ref(&completion_key))?;
        let _ = result.get(&completion_key);
        if env.values_missing() {
            return Ok(None);
        }

        // If we get to this point, the execution of this target/aspect succeeded.
        self.remove_states_for_key(key);
        Ok(Some(BuildDriverValue {
            top_level_sky_value: TopLevelSkyValue::ConfiguredTarget(configured_target),
            skipped: false,
        }))
    }

    /// `BuildDriverFunction.compute` for a `TopLevelAspectsKey` (the `else` branch of
    /// `topLevelSkyValue instanceof ConfiguredTargetValue`).
    ///
    /// PORT: as for configured targets, BUILD_ID, the action conflict check, extra actions
    /// (`addExtraActionsIfRequested`), `TopLevelTargetReadyForSymlinkPlanting`,
    /// `TopLevelEntityAnalysisConcludedEvent`, `SomeExecutionStartedEvent` and
    /// `additionalPostAnalysisDepsRequestedAndAvailable` are not ported here; the
    /// `announceTopLevelAspectAnalyzed` events (`ToplevelAspectsIdentifiedEvent`,
    /// `AspectConfiguredEvent`, `AspectAnalyzedEvent`) are posted by the analysis side
    /// (blaze_sky_top `build_view`).
    // Java: com/google/devtools/build/lib/skyframe/BuildDriverFunction.java:155 BuildDriverFunction.compute
    fn compute_top_level_aspects(
        &self,
        key: &BuildDriverKey,
        env: &mut Environment<'_>,
    ) -> Result<Option<BuildDriverValue>, ComputeError> {
        let Some(top_level_aspects) = self
            .seam
            .get_top_level_aspects(&key.action_lookup_key, env)?
        else {
            return Ok(None);
        };
        if env.values_missing() {
            return Ok(None);
        }
        // Java: state.testType = NOT_TEST for a TopLevelAspectsValue.
        blaze_util::check_state!(key.test_type == TestType::NOT_TEST);

        // Java: com/google/devtools/build/lib/skyframe/BuildDriverFunction.java:323 BuildDriverFunction.compute
        let mut aspect_completion_keys: Vec<SkyKey> = Vec::new();
        // Do not trigger Skyframe restarts in this loop.
        for aspect in &top_level_aspects {
            aspect_completion_keys.push(SkyKey::new(AspectCompletionKey::create(
                aspect.get_aspect_key(),
                key.output_groups.clone(),
            )));
        }

        // Request the execution of the collected aspects.
        // Java: com/google/devtools/build/lib/skyframe/BuildDriverFunction.java:650 BuildDriverFunction.declareDependenciesAndCheckValues
        let result = env.get_values_and_exceptions(&aspect_completion_keys)?;
        for k in &aspect_completion_keys {
            if result.get(k).is_none() {
                break;
            }
        }
        if env.values_missing() {
            return Ok(None);
        }

        // If we get to this point, the execution of this target/aspect succeeded.
        self.remove_states_for_key(key);
        Ok(Some(BuildDriverValue {
            top_level_sky_value: TopLevelSkyValue::TopLevelAspects(top_level_aspects),
            skipped: false,
        }))
    }
}
