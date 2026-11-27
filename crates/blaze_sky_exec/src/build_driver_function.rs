//! `BuildDriverFunction`: drives the analysis and execution of a top-level target (Skymeld).

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::sync::Arc;

use blaze_events::Postable;
use blaze_skyframe::{ComputeError, Environment, SkyKey};
use parking_lot::Mutex;

use crate::analysis_seam::{AnalysisSeam, ConfiguredAspectView, ConfiguredTargetView};
use crate::events::{
    SomeExecutionStartedEvent, TestAnalyzedEvent, TopLevelEntityAnalysisConcludedEvent,
    TopLevelTargetPendingExecutionEvent, TopLevelTargetReadyForSymlinkPlanting,
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
// Java: com/google/devtools/build/lib/skyframe/BuildDriverValue.java:28 BuildDriverValue.getWrappedSkyValue
// Java: com/google/devtools/build/lib/skyframe/BuildDriverValue.java:32 BuildDriverValue.isSkipped
// (the getters are the public fields `top_level_sky_value` and `skipped`)
#[derive(Clone, Debug)]
pub struct BuildDriverValue {
    /// `getLastEvaluatedTopLevelSkyValue()`.
    pub top_level_sky_value: TopLevelSkyValue,
    pub skipped: bool,
    /// Whether this is an `ExclusiveTestBuildDriverValue` (Java's subclass, meant to represent
    /// an exclusive test); its `getExclusiveTestConfiguredTarget()` is the configured target of
    /// `top_level_sky_value`.
    // Java: com/google/devtools/build/lib/skyframe/ExclusiveTestBuildDriverValue.java:20 ExclusiveTestBuildDriverValue
    // Java: com/google/devtools/build/lib/skyframe/ExclusiveTestBuildDriverValue.java:31 ExclusiveTestBuildDriverValue.getExclusiveTestConfiguredTarget
    pub exclusive_test: bool,
}

/// The types of the `TopLevelStatusEvents` this function posts.
// Java: com/google/devtools/build/lib/skyframe/TopLevelStatusEvents.java:241 TopLevelStatusEvents.Type
///
/// `TopLevelStatusEventWithType.getType()` is the value each post passes with its event.
// Java: com/google/devtools/build/lib/skyframe/TopLevelStatusEvents.java:38 TopLevelStatusEvents.TopLevelStatusEventWithType.getType
// Java: com/google/devtools/build/lib/skyframe/TopLevelStatusEvents.java:80 TopLevelStatusEvents.TopLevelTargetReadyForSymlinkPlanting.getType
// Java: com/google/devtools/build/lib/skyframe/TopLevelStatusEvents.java:139 TopLevelStatusEvents.TopLevelTargetPendingExecutionEvent.getType
// Java: com/google/devtools/build/lib/skyframe/TopLevelStatusEvents.java:162 TopLevelStatusEvents.SomeExecutionStartedEvent.getType
// Java: com/google/devtools/build/lib/skyframe/TopLevelStatusEvents.java:202 TopLevelStatusEvents.TestAnalyzedEvent.getType
// Java: com/google/devtools/build/lib/skyframe/TopLevelStatusEvents.java:55 TopLevelStatusEvents.TopLevelTargetAnalyzedEvent.getType
// Java: com/google/devtools/build/lib/skyframe/TopLevelStatusEvents.java:97 TopLevelStatusEvents.TopLevelTargetSkippedEvent.getType
// Java: com/google/devtools/build/lib/skyframe/TopLevelStatusEvents.java:220 TopLevelStatusEvents.AspectAnalyzedEvent.getType
// Java: com/google/devtools/build/lib/skyframe/TopLevelStatusEvents.java:118 TopLevelStatusEvents.TopLevelEntityAnalysisConcludedEvent.getType
// PORT: skipped, posted by the execution progress receiver, not through this function's posted types: TopLevelStatusEvents.TopLevelTargetBuiltEvent.getType
// PORT: skipped, the built aspects of the build result are read from the evaluation (blaze_runtime top_level_aspect_built), not collected from events: TopLevelStatusEvents.AspectBuiltEvent.aspectKey TopLevelStatusEvents.AspectBuiltEvent.create TopLevelStatusEvents.AspectBuiltEvent.getType
// PORT: skipped, the private constructor of a holder class: TopLevelStatusEvents.<init>
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[allow(non_camel_case_types)]
enum TopLevelStatusEventsType {
    TOP_LEVEL_TARGET_CONFIGURED,
    TOP_LEVEL_TARGET_ANALYZED,
    TOP_LEVEL_TARGET_READY_FOR_SYMLINK_PLANTING,
    TOP_LEVEL_TARGET_SKIPPED,
    TOP_LEVEL_ENTITY_ANALYSIS_CONCLUDED,
    TOP_LEVEL_TARGET_PENDING_EXECUTION,
    SOME_EXECUTION_STARTED,
    TEST_ANALYZED,
    ASPECT_ANALYZED,
}

/// The function's per-build state: `keyToPostedEvents` and `checkedForConflicts`, which Java
/// resets after each `evaluateBuildDriverKeys` (`SkyframeExecutor.resetBuildDriverFunction`).
///
/// PORT: the function object outlives a build (the kept graph keeps the functions), so the
/// state lives in the build arena, which the server releases after every command.
#[derive(Default)]
struct BuildDriverStates {
    /// Java: `keyToPostedEvents`. A function can run more than once for a key (after a
    /// dependency went missing, and once more in error bubbling); each event is posted once.
    key_to_posted_events: Mutex<HashMap<BuildDriverKey, HashSet<TopLevelStatusEventsType>>>,
    /// Java: `checkedForConflicts`: we only check for action conflict once per BuildDriverKey.
    checked_for_conflicts: Mutex<HashSet<BuildDriverKey>>,
}

impl blaze_util::build_arena::ArenaSlot for BuildDriverStates {
    const RELEASE_ORDER: blaze_util::build_arena::ReleaseOrder =
        blaze_util::build_arena::ReleaseOrder::State;
}

/// Drives the analysis and execution of an `ActionLookupKey`, which is wrapped inside a
/// `BuildDriverKey`.
// Java: com/google/devtools/build/lib/skyframe/BuildDriverFunction.java:104 BuildDriverFunction
pub struct BuildDriverFunction {
    seam: Arc<dyn AnalysisSeam>,
}

impl BuildDriverFunction {
    pub fn new(seam: Arc<dyn AnalysisSeam>) -> BuildDriverFunction {
        BuildDriverFunction { seam }
    }

    fn states() -> Arc<BuildDriverStates> {
        blaze_util::build_arena::slot::<BuildDriverStates>()
    }

    /// `resetStates()`: the function's states start over (Java calls it after each
    /// `evaluateBuildDriverKeys`; the build arena also drops them after the command).
    // Java: com/google/devtools/build/lib/skyframe/BuildDriverFunction.java:493 BuildDriverFunction.resetStates
    pub fn reset_states() {
        let states = Self::states();
        states.key_to_posted_events.lock().clear();
        states.checked_for_conflicts.lock().clear();
    }

    // Java: com/google/devtools/build/lib/skyframe/BuildDriverFunction.java:503 BuildDriverFunction.postEventIfNecessary
    fn post_event_if_necessary(
        &self,
        key: &BuildDriverKey,
        env: &Environment<'_>,
        type_: TopLevelStatusEventsType,
        event: Arc<dyn Postable>,
    ) {
        if self.mark_event_posted(key, type_) {
            env.listener().post(event);
        }
    }

    /// `postedEventsTypes.contains(type)`.
    fn event_posted(&self, key: &BuildDriverKey, type_: TopLevelStatusEventsType) -> bool {
        Self::states()
            .key_to_posted_events
            .lock()
            .get(key)
            .is_some_and(|posted| posted.contains(&type_))
    }

    /// `postedEventsTypes.add(type)`: whether it was newly added.
    fn mark_event_posted(&self, key: &BuildDriverKey, type_: TopLevelStatusEventsType) -> bool {
        Self::states()
            .key_to_posted_events
            .lock()
            .entry(key.clone())
            .or_default()
            .insert(type_)
    }

    /// Sends out a signal that no more analysis work will be done on this top level
    /// target/aspect.
    ///
    /// Only do so in --keep_going mode. This is consistent with the legacy behavior where the
    /// analysis phase isn't considered "finished" if there's an error in --nokeep_going mode.
    // Java: com/google/devtools/build/lib/skyframe/BuildDriverFunction.java:398 BuildDriverFunction.signalAnalysisConclusionIfKeepGoing
    fn signal_analysis_conclusion_if_keep_going(
        &self,
        env: &Environment<'_>,
        key: &BuildDriverKey,
        success: bool,
    ) {
        if key.keep_going {
            self.post_analysis_concluded(env, key, success);
        }
    }

    /// `postEventIfNecessary(..., TopLevelEntityAnalysisConcludedEvent.create(key, success))`.
    fn post_analysis_concluded(&self, env: &Environment<'_>, key: &BuildDriverKey, success: bool) {
        self.post_event_if_necessary(
            key,
            env,
            TopLevelStatusEventsType::TOP_LEVEL_ENTITY_ANALYSIS_CONCLUDED,
            Arc::new(TopLevelEntityAnalysisConcludedEvent {
                analyzed_top_level_key: key.clone(),
                succeeded: success,
            }),
        );
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
        let states = Self::states();
        states.checked_for_conflicts.lock().remove(key);
        states.key_to_posted_events.lock().remove(key);
    }

    /// From the ConfiguredTarget/Aspect keys, get the top-level artifacts. Then evaluate them
    /// together with the appropriate CompletionFunctions. This is the bridge between the
    /// conceptual analysis & execution phases.
    ///
    /// The analysis steps (the announcements, the conflict check, the compatibility check, the
    /// test type resolver, `TopLevelEntityAnalysisConcludedEvent`) run in a build with merged
    /// analysis and execution ([`AnalysisSeam::is_merged_analysis_and_execution`]); the path
    /// without it evaluates these keys after its analysis announced and checked every target,
    /// and the key carries the resolved test type.
    ///
    /// PORT: Java keeps `State` (`testType`, `checkedForCompatibility`,
    /// `checkedForPlatformCompatibility`) across Skyframe restarts (`env.getState`); the blocking
    /// engine runs the function once, so they are locals.
    // Java: com/google/devtools/build/lib/skyframe/BuildDriverFunction.java:155 BuildDriverFunction.compute
    // Java: com/google/devtools/build/lib/skyframe/BuildDriverFunction.java:723 BuildDriverFunction.TestTypeResolver.determineTestType
    // Java: com/google/devtools/build/lib/skyframe/BuildDriverFunction.java:512 BuildDriverFunction.isTest
    // PORT: skipped, Bazel passes BuildDriverFunction.AdditionalPostAnalysisDepsRequestedAndAvailable.NO_OP (SkyframeExecutor): BuildDriverFunction.AdditionalPostAnalysisDepsRequestedAndAvailable.request
    pub fn compute(
        &self,
        key: &BuildDriverKey,
        env: &mut Environment<'_>,
    ) -> Result<Option<BuildDriverValue>, ComputeError> {
        // Register a dependency on the BUILD_ID. We do this to make sure BuildDriverFunction is
        // reevaluated every build.
        // Java: com/google/devtools/build/lib/skyframe/BuildDriverFunction.java:165 BuildDriverFunction.compute
        blaze_sky_loading::precomputed::BUILD_ID.get(env)?;
        let merged = self.seam.is_merged_analysis_and_execution();

        // Why SkyValue and not ActionLookupValue? The evaluation of some ActionLookupKey can
        // result in classes that don't implement ActionLookupValue (e.g. ConfiguredTargetKey ->
        // NonRuleConfiguredTargetValue).
        let top_level_sky_value = match self.seam.get_top_level_value_or_throw(key, env)? {
            Ok(value) => value,
            Err(e) => {
                if merged {
                    self.signal_analysis_conclusion_if_keep_going(
                        env, key, /* success= */ false,
                    );
                }
                // BuildDriverFunctionException.ofConfiguredTargetOrAspectEval(e): PERSISTENT.
                // Java: com/google/devtools/build/lib/skyframe/BuildDriverFunction.java:713 BuildDriverFunction.BuildDriverFunctionException.ofConfiguredTargetOrAspectEval
                return Err(blaze_skyframe::SkyFunctionException::from_cause(
                    e,
                    blaze_skyframe::Transience::Persistent,
                )
                .into());
            }
        };
        let Some(top_level_sky_value) = top_level_sky_value else {
            return Ok(None);
        };
        if env.values_missing() {
            return Ok(None);
        }

        if merged {
            // At this point, the target is considered "analyzed". It's important that this event
            // is sent before the TopLevelEntityAnalysisConcludedEvent: when the last of the
            // analysis work is concluded, we need to have the complete list of analyzed targets
            // ready in BuildResultListener.
            match &top_level_sky_value {
                TopLevelSkyValue::ConfiguredTarget(configured_target) => {
                    // Java: com/google/devtools/build/lib/skyframe/BuildDriverFunction.java:415 BuildDriverFunction.announceTopLevelConfiguredTargetAnalyzed
                    let post_target_configured = self.mark_event_posted(
                        key,
                        TopLevelStatusEventsType::TOP_LEVEL_TARGET_CONFIGURED,
                    );
                    let Some(analyzed_event) =
                        self.seam.announce_top_level_configured_target_analyzed(
                            configured_target,
                            post_target_configured,
                            env,
                        )?
                    else {
                        return Ok(None);
                    };
                    self.post_event_if_necessary(
                        key,
                        env,
                        TopLevelStatusEventsType::TOP_LEVEL_TARGET_ANALYZED,
                        analyzed_event,
                    );
                }
                TopLevelSkyValue::TopLevelAspects(_) => {
                    // Java: com/google/devtools/build/lib/skyframe/BuildDriverFunction.java:449 BuildDriverFunction.announceTopLevelAspectAnalyzed
                    if self.mark_event_posted(key, TopLevelStatusEventsType::ASPECT_ANALYZED)
                        && self
                            .seam
                            .announce_top_level_aspect_analyzed(key, env)?
                            .is_none()
                    {
                        return Ok(None);
                    }
                }
            }

            // We only check for action conflict once per BuildDriverKey.
            // Java: com/google/devtools/build/lib/skyframe/BuildDriverFunction.java:194 BuildDriverFunction.compute
            if self.seam.should_check_for_conflict_with_traversal()
                && Self::states()
                    .checked_for_conflicts
                    .lock()
                    .insert(key.clone())
            {
                let _c = blaze_util::profiler::Profiler::instance().profile(
                    blaze_util::profiler::ProfilerTask::INFO,
                    "BuildDriverFunction.checkActionConflicts",
                );
                if let Err(e) = self.seam.check_action_conflicts(key) {
                    // The analysis technically succeeded, even though the target/aspect can't be
                    // executed.
                    self.signal_analysis_conclusion_if_keep_going(
                        env, key, /* success= */ true,
                    );
                    return Err(e);
                }
            }
        }

        match top_level_sky_value {
            TopLevelSkyValue::ConfiguredTarget(configured_target) => {
                self.compute_configured_target(key, configured_target, merged, env)
            }
            TopLevelSkyValue::TopLevelAspects(top_level_aspects) => {
                self.compute_top_level_aspects(key, top_level_aspects, merged, env)
            }
        }
    }

    /// The part of `compute` for a `ConfiguredTargetValue`.
    // Java: com/google/devtools/build/lib/skyframe/BuildDriverFunction.java:219 BuildDriverFunction.compute
    fn compute_configured_target(
        &self,
        key: &BuildDriverKey,
        configured_target: Arc<dyn ConfiguredTargetView>,
        merged: bool,
        env: &mut Environment<'_>,
    ) -> Result<Option<BuildDriverValue>, ComputeError> {
        // state.testType = testTypeResolver.get().determineTestType(configuredTarget)
        let test_type = if merged {
            self.seam.determine_test_type(&configured_target, env)
        } else {
            key.test_type
        };
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

        if merged {
            // Java: com/google/devtools/build/lib/skyframe/BuildDriverFunction.java:254 BuildDriverFunction.compute
            let mut checked_for_platform_compatibility = false;
            match self.seam.is_configured_target_compatible(
                &configured_target,
                key.explicitly_requested,
                key.skip_incompatible_explicit_targets,
                &mut checked_for_platform_compatibility,
                env,
            ) {
                Ok(None) => return Ok(None),
                Ok(Some(true)) => {}
                Ok(Some(false)) => {
                    self.post_event_if_necessary(
                        key,
                        env,
                        TopLevelStatusEventsType::TOP_LEVEL_TARGET_SKIPPED,
                        self.seam.top_level_target_skipped_event(&configured_target),
                    );
                    // We still record analyzed but skipped tests, as this information is needed
                    // for the result summary.
                    if test_type.is_test() {
                        self.post_test_analyzed(
                            key,
                            &configured_target,
                            env,
                            /* is_skipped= */ true,
                        );
                    }
                    // Only send the event now to include the compatibility check in the
                    // measurement for time spent on analysis work.
                    self.post_analysis_concluded(env, key, /* success= */ true);
                    // We consider the evaluation of this BuildDriverKey successful at this point,
                    // even when the target is skipped.
                    self.remove_states_for_key(key);
                    return Ok(Some(BuildDriverValue {
                        top_level_sky_value: TopLevelSkyValue::ConfiguredTarget(configured_target),
                        skipped: true,
                        exclusive_test: false,
                    }));
                }
                Err(e) => {
                    // The analysis of the target technically succeeded, just that it was
                    // incompatible and can't be executed.
                    self.signal_analysis_conclusion_if_keep_going(
                        env, key, /* success= */ true,
                    );
                    return Err(e);
                }
            }

            self.post_analysis_concluded(env, key, /* success= */ true);
        }

        // Java: com/google/devtools/build/lib/skyframe/BuildDriverFunction.java:598 BuildDriverFunction.requestConfiguredTargetExecution
        self.post_event_if_necessary(
            key,
            env,
            TopLevelStatusEventsType::TOP_LEVEL_TARGET_PENDING_EXECUTION,
            Arc::new(TopLevelTargetPendingExecutionEvent {
                label: configured_target.get_label(),
                is_test: test_type.is_test(),
            }),
        );
        let mut keys_to_request: Vec<SkyKey> = self
            .add_extra_actions_if_requested(
                configured_target.get_extra_action_artifacts_provider(),
                key.extra_action_top_level_only,
            )
            .iter()
            .map(crate::keys::artifact_key)
            .collect();
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
            self.post_test_analyzed(key, &configured_target, env, /* is_skipped= */ false);
        }
        let completion_key: SkyKey = match test_type {
            TestType::NOT_TEST => SkyKey::new(TargetCompletionKey {
                action_lookup_key: key.action_lookup_key.clone(),
                output_groups: key.output_groups.clone(),
                will_test: false,
            }),
            // Only run non-exclusive tests here. Exclusive tests need to be run sequentially
            // later.
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

    /// `postEventIfNecessary(..., TestAnalyzedEvent.create(configuredTarget,
    /// buildConfigurationValue, isSkipped))`.
    fn post_test_analyzed(
        &self,
        key: &BuildDriverKey,
        configured_target: &Arc<dyn ConfiguredTargetView>,
        env: &Environment<'_>,
        is_skipped: bool,
    ) {
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
                is_skipped,
            }),
        );
    }

    /// The part of `compute` for a `TopLevelAspectsValue` (the `else` branch of
    /// `topLevelSkyValue instanceof ConfiguredTargetValue`).
    // Java: com/google/devtools/build/lib/skyframe/BuildDriverFunction.java:316 BuildDriverFunction.compute
    fn compute_top_level_aspects(
        &self,
        key: &BuildDriverKey,
        top_level_aspects: Vec<Arc<dyn ConfiguredAspectView>>,
        merged: bool,
        env: &mut Environment<'_>,
    ) -> Result<Option<BuildDriverValue>, ComputeError> {
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

        // Send the AspectAnalyzedEvents first to make sure the BuildResultListener is up-to-date
        // before signaling that the analysis of this top level aspect has concluded.
        // Java: com/google/devtools/build/lib/skyframe/BuildDriverFunction.java:362 BuildDriverFunction.compute
        if merged {
            self.post_analysis_concluded(env, key, /* success= */ true);
        }

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
        self.remove_states_for_key(key);
        Ok(Some(BuildDriverValue {
            top_level_sky_value: TopLevelSkyValue::TopLevelAspects(top_level_aspects),
            skipped: false,
            exclusive_test: false,
        }))
    }
}
