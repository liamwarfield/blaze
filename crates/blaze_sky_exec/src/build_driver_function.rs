//! `BuildDriverFunction`: drives the analysis and execution of a top-level target (Skymeld).

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::sync::Arc;

use blaze_events::Postable;
use blaze_skyframe::{ComputeError, Environment, SkyKey};
use parking_lot::Mutex;

use crate::analysis_seam::{AnalysisSeam, ConfiguredAspectView, ConfiguredTargetView};
use crate::events::{
    SomeExecutionStartedEvent, TestAnalyzedEvent, TopLevelTargetPendingExecutionEvent,
    TopLevelTargetReadyForSymlinkPlanting,
};
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
    /// Whether this is an `ExclusiveTestBuildDriverValue` (Java's subclass, meant to represent
    /// an exclusive test); its `getExclusiveTestConfiguredTarget()` is the configured target of
    /// `top_level_sky_value`.
    // Java: com/google/devtools/build/lib/skyframe/ExclusiveTestBuildDriverValue.java:20 ExclusiveTestBuildDriverValue
    pub exclusive_test: bool,
}

/// The types of the `TopLevelStatusEvents` this function posts.
// Java: com/google/devtools/build/lib/skyframe/TopLevelStatusEvents.java:241 TopLevelStatusEvents.Type
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[allow(non_camel_case_types)]
enum TopLevelStatusEventsType {
    TOP_LEVEL_TARGET_PENDING_EXECUTION,
    TOP_LEVEL_TARGET_READY_FOR_SYMLINK_PLANTING,
    SOME_EXECUTION_STARTED,
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

    /// `postedEventsTypes.contains(type)`.
    fn event_posted(&self, key: &BuildDriverKey, type_: TopLevelStatusEventsType) -> bool {
        self.key_to_posted_events
            .lock()
            .get(key)
            .is_some_and(|posted| posted.contains(&type_))
    }

    /// `postedEventsTypes.add(type)`.
    fn mark_event_posted(&self, key: &BuildDriverKey, type_: TopLevelStatusEventsType) {
        self.key_to_posted_events
            .lock()
            .entry(key.clone())
            .or_default()
            .insert(type_);
    }

    /// The extra action artifacts to build (`ImmutableSet.Builder<Artifact>`: no duplicates,
    /// first occurrence order).
    // Java: com/google/devtools/build/lib/skyframe/BuildDriverFunction.java:672 BuildDriverFunction.addExtraActionsIfRequested
    fn add_extra_actions_if_requested(
        &self,
        provider: Option<(
            blaze_collect::NestedSet<blaze_actions::Artifact>,
            blaze_collect::NestedSet<blaze_actions::Artifact>,
        )>,
        extra_action_top_level_only: bool,
    ) -> indexmap::IndexSet<blaze_actions::Artifact> {
        let mut builder = indexmap::IndexSet::new();
        if let Some((extra_action_artifacts, transitive_extra_action_artifacts)) = provider {
            let artifacts = if extra_action_top_level_only {
                extra_action_artifacts.to_list()
            } else {
                transitive_extra_action_artifacts.to_list()
            };
            // Java: com/google/devtools/build/lib/skyframe/BuildDriverFunction.java:685 BuildDriverFunction.addArtifactsToBuilder
            for artifact in artifacts.iter() {
                let owner_label = artifact
                    .get_owner_label()
                    .map(|l| l.to_string())
                    .unwrap_or_else(|| "null".to_owned());
                if self.seam.is_extra_action_owner_included(&owner_label) {
                    builder.insert(artifact.clone());
                }
            }
        }
        builder
    }

    // Java: com/google/devtools/build/lib/skyframe/BuildDriverFunction.java:498 BuildDriverFunction.removeStatesForKey
    fn remove_states_for_key(&self, key: &BuildDriverKey) {
        self.key_to_posted_events.lock().remove(key);
    }

    /// `BuildDriverFunction.compute` for configured targets.
    // Java: com/google/devtools/build/lib/skyframe/BuildDriverFunction.java:155 BuildDriverFunction.compute
    // PORT: action conflict checks (checkActionConflicts), target compatibility
    // (isConfiguredTargetCompatible: incompatible targets are skipped), extra actions, top-level
    // aspects and the TopLevelStatusEvents other than TopLevelTargetPendingExecutionEvent,
    // TopLevelTargetReadyForSymlinkPlanting, SomeExecutionStartedEvent and TestAnalyzedEvent are
    // the
    // analysis side's (blaze_sky_top) and are not ported here.
    pub fn compute(
        &self,
        key: &BuildDriverKey,
        env: &mut Environment<'_>,
    ) -> Result<Option<BuildDriverValue>, ComputeError> {
        // Register a dependency on the BUILD_ID. We do this to make sure BuildDriverFunction is
        // reevaluated every build.
        // Java: com/google/devtools/build/lib/skyframe/BuildDriverFunction.java:165 BuildDriverFunction.compute
        blaze_sky_loading::precomputed::BUILD_ID.get(env)?;
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
        // It's possible that this code path is triggered AFTER the analysis cache clean up and
        // the transitive packages for package root resolution is already cleared. In such a
        // case, the symlinks should have already been planted.
        // Java: com/google/devtools/build/lib/skyframe/BuildDriverFunction.java:235 BuildDriverFunction.compute
        if let Some(transitive_packages_for_symlink_planting) =
            configured_target.get_transitive_packages()
        {
            self.post_event_if_necessary(
                key,
                env,
                TopLevelStatusEventsType::TOP_LEVEL_TARGET_READY_FOR_SYMLINK_PLANTING,
                Arc::new(TopLevelTargetReadyForSymlinkPlanting {
                    transitive_packages_for_symlink_planting,
                }),
            );
        }
        // Java: com/google/devtools/build/lib/skyframe/BuildDriverFunction.java:598 BuildDriverFunction.requestConfiguredTargetExecution
        let mut keys_to_request: Vec<SkyKey> = self
            .add_extra_actions_if_requested(
                configured_target.get_extra_action_artifacts_provider(),
                key.extra_action_top_level_only,
            )
            .iter()
            .map(crate::keys::artifact_key)
            .collect();
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
        // Java: com/google/devtools/build/lib/skyframe/BuildDriverFunction.java:605 BuildDriverFunction.requestConfiguredTargetExecution
        self.post_event_if_necessary(
            key,
            env,
            TopLevelStatusEventsType::SOME_EXECUTION_STARTED,
            Arc::new(SomeExecutionStartedEvent::create()),
        );
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
        keys_to_request.push(completion_key);
        // Java: com/google/devtools/build/lib/skyframe/BuildDriverFunction.java:650 BuildDriverFunction.declareDependenciesAndCheckValues
        let result = env.get_values_and_exceptions(&keys_to_request)?;
        for k in &keys_to_request {
            if result.get(k).is_none() {
                break;
            }
        }
        if env.values_missing() {
            return Ok(None);
        }

        // If we get to this point, the execution of this target/aspect succeeded.

        // Java: com/google/devtools/build/lib/skyframe/BuildDriverFunction.java:381 BuildDriverFunction.compute
        if test_type == TestType::EXCLUSIVE || test_type == TestType::EXCLUSIVE_IF_LOCAL {
            self.remove_states_for_key(key);
            // If an exclusive test was run at all, it wasn't skipped.
            return Ok(Some(BuildDriverValue {
                top_level_sky_value: TopLevelSkyValue::ConfiguredTarget(configured_target),
                skipped: false,
                exclusive_test: true,
            }));
        }

        self.remove_states_for_key(key);
        Ok(Some(BuildDriverValue {
            top_level_sky_value: TopLevelSkyValue::ConfiguredTarget(configured_target),
            skipped: false,
            exclusive_test: false,
        }))
    }

    /// `BuildDriverFunction.compute` for a `TopLevelAspectsKey` (the `else` branch of
    /// `topLevelSkyValue instanceof ConfiguredTargetValue`).
    ///
    /// PORT: as for configured targets, BUILD_ID, the action conflict check,
    /// `TopLevelEntityAnalysisConcludedEvent` and
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
        let mut artifacts_to_build: indexmap::IndexSet<blaze_actions::Artifact> =
            indexmap::IndexSet::new();
        let mut aspect_completion_keys: Vec<SkyKey> = Vec::new();
        // Do not trigger Skyframe restarts in this loop.
        let symlink_planting_posted = self.event_posted(
            key,
            TopLevelStatusEventsType::TOP_LEVEL_TARGET_READY_FOR_SYMLINK_PLANTING,
        );
        for aspect in &top_level_aspects {
            artifacts_to_build.extend(self.add_extra_actions_if_requested(
                aspect.get_extra_action_artifacts_provider(),
                key.extra_action_top_level_only,
            ));

            // It's possible that this code path is triggered AFTER the analysis cache clean up
            // and the transitive packages for package root resolution is already cleared. In
            // such a case, the symlinks should have already been planted.
            // Java: com/google/devtools/build/lib/skyframe/BuildDriverFunction.java:338 BuildDriverFunction.compute
            if let Some(transitive_packages_for_symlink_planting) = aspect.get_transitive_packages()
            {
                // This event should be sent out exactly once per aspect in this
                // BuildDriverKey, even with resets. We achieve this by marking the event type as
                // sent only after sending the event for all aspects, but must avoid triggering
                // Skyframe restarts while doing so.
                if !symlink_planting_posted {
                    env.listener()
                        .post(Arc::new(TopLevelTargetReadyForSymlinkPlanting {
                            transitive_packages_for_symlink_planting,
                        }));
                }
            }
            aspect_completion_keys.push(SkyKey::new(AspectCompletionKey::create(
                aspect.get_aspect_key(),
                key.output_groups.clone(),
            )));
        }
        // Java: postedEventsTypes.add(TOP_LEVEL_TARGET_READY_FOR_SYMLINK_PLANTING)
        self.mark_event_posted(
            key,
            TopLevelStatusEventsType::TOP_LEVEL_TARGET_READY_FOR_SYMLINK_PLANTING,
        );

        // Java: com/google/devtools/build/lib/skyframe/BuildDriverFunction.java:369 BuildDriverFunction.compute
        self.post_event_if_necessary(
            key,
            env,
            TopLevelStatusEventsType::SOME_EXECUTION_STARTED,
            Arc::new(SomeExecutionStartedEvent::create()),
        );
        // Request the execution of the collected aspects.
        // Java: com/google/devtools/build/lib/skyframe/BuildDriverFunction.java:650 BuildDriverFunction.declareDependenciesAndCheckValues
        let mut keys_to_request: Vec<SkyKey> = artifacts_to_build
            .iter()
            .map(crate::keys::artifact_key)
            .collect();
        keys_to_request.extend(aspect_completion_keys);
        let aspect_completion_keys = keys_to_request;
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
            exclusive_test: false,
        }))
    }
}
