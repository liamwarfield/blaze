//! Port of `runtime/TargetSummaryPublisher.java`: publishes a `TargetSummaryEvent` for each
//! top-level target once its completion, its top-level aspects' completions and (for tests) its
//! test summary are known.
//!
//! PORT: Java subscribes to the command's event bus and keys targets by `ConfiguredTargetKey`
//! (`AliasProvider.getDependencyLabel(target)` and the configuration key). This crate sits below
//! the analysis crates, so the subscriber (`blaze_runtime` commands/bep.rs) calls the methods
//! below with what each event carries: the target's dependency label and configuration checksum
//! as the key ([`TargetSummaryKey`]), its original label (`TargetSummaryEvent.create` reads
//! `target.getOriginalLabel()`), and whether it is an alias.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use blaze_cmdline::label::Label;
use blaze_protos::blaze::BlazeTestStatus;
use blaze_util::check_state;
use parking_lot::Mutex;

use crate::runtime::test_summary_event::TargetSummaryEvent;

/// `ConfiguredTargetKey` as the publisher uses it: the dependency label and the configuration
/// checksum (`None` for the null configuration).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct TargetSummaryKey {
    pub label: Label,
    pub configuration_checksum: Option<String>,
}

/// What the publisher reads of a top-level `ConfiguredTarget`.
#[derive(Clone, Debug)]
pub struct TargetSummaryTarget {
    /// `asKey(target)`.
    pub key: TargetSummaryKey,
    /// `target.getOriginalLabel()`.
    pub original_label: Label,
    /// `AliasProvider.isAlias(target)`.
    pub is_alias: bool,
}

/// Sink for the `TargetSummaryEvent`s the publisher posts (`eventBus.post`).
pub trait TargetSummarySink: Send + Sync {
    fn post(&self, event: TargetSummaryEvent);
}

/// Publishes `TargetSummaryEvent`s.
// Java: com/google/devtools/build/lib/runtime/TargetSummaryPublisher.java:50 TargetSummaryPublisher
pub struct TargetSummaryPublisher {
    event_bus: Arc<dyn TargetSummarySink>,
    /// `cmdEnv::withMergedAnalysisAndExecutionSourceOfTruth` (a supplier in Java: the value is
    /// set once the command knows it, see [`TargetSummaryPublisher::set_merged_skyframe_analysis_execution`]).
    merged_skyframe_analysis_execution: AtomicBool,
    /// Whether or not toplevel aspects are present, from `BuildStartingEvent`.
    has_aspects: AtomicBool,
    aggregators: Mutex<HashMap<TargetSummaryKey, Arc<TargetSummaryAggregator>>>,
    aspect_count_per_target: Mutex<HashMap<TargetSummaryKey, i32>>,
    /// The `!failed()` of each queued `AspectCompleteEvent`, by base target.
    queued_aspect_complete_events: Mutex<HashMap<TargetSummaryKey, Vec<bool>>>,
}

impl TargetSummaryPublisher {
    // Java: com/google/devtools/build/lib/runtime/TargetSummaryPublisher.java:66 TargetSummaryPublisher.<init>
    pub fn new(event_bus: Arc<dyn TargetSummarySink>) -> Self {
        TargetSummaryPublisher {
            event_bus,
            merged_skyframe_analysis_execution: AtomicBool::new(false),
            has_aspects: AtomicBool::new(false),
            aggregators: Mutex::new(HashMap::new()),
            aspect_count_per_target: Mutex::new(HashMap::new()),
            queued_aspect_complete_events: Mutex::new(HashMap::new()),
        }
    }

    /// The value of `mergedSkyframeAnalysisExecution.get()` from now on.
    pub fn set_merged_skyframe_analysis_execution(&self, value: bool) {
        self.merged_skyframe_analysis_execution
            .store(value, Ordering::SeqCst);
    }

    /// Extracts how many aspect completions per target to expect. This must happen before
    /// `populateTargets`.
    ///
    /// This excludes --exec_aspects.
    // Java: com/google/devtools/build/lib/runtime/TargetSummaryPublisher.java:80 TargetSummaryPublisher.buildStarting
    pub fn build_starting(&self, request_has_aspects: bool) {
        self.has_aspects
            .store(request_has_aspects, Ordering::SeqCst);
    }

    /// Reports the correct number of top-level aspects that were analyzed for a given configured
    /// target.
    // Java: com/google/devtools/build/lib/runtime/TargetSummaryPublisher.java:97 TargetSummaryPublisher.toplevelAspectsIdentified
    pub fn toplevel_aspects_identified(
        &self,
        target_key: TargetSummaryKey,
        num_top_level_aspects: i32,
    ) {
        self.aspect_count_per_target
            .lock()
            .insert(target_key.clone(), num_top_level_aspects);
        let aggregator = self.aggregators.lock().get(&target_key).cloned();
        if let Some(aggregator) = aggregator {
            // We may have already set the expected aspect completions if this method is racing
            // with #populateTarget(). This is safe because we guarantee that we set the same
            // value, so that no aspect completions have happened when we double-set the expected
            // aspect completions.
            aggregator.set_expect_aspect_completions(num_top_level_aspects);
        }
    }

    /// Populates the target summary map as soon as test filtering is complete. This is the
    /// earliest at which the final set of targets to build and test is known. This must happen
    /// after `buildStarting`.
    // Java: com/google/devtools/build/lib/runtime/TargetSummaryPublisher.java:115 TargetSummaryPublisher.populateTargets
    pub fn populate_targets(
        &self,
        targets: &[TargetSummaryTarget],
        test_targets: &[TargetSummaryKey],
        skipped_tests: &[TargetSummaryKey],
    ) {
        for target in targets {
            if skipped_tests.contains(&target.key) {
                // Skipped tests aren't built, and won't receive completion events, so we ignore
                // them. Note we'll still get (and ignore) a TestSummary event, but that event
                // isn't published to BEP.
                continue;
            }
            let new_aggregator = self.create_aggregator_for_target(
                /* is_test= */ test_targets.contains(&target.key),
                target,
            );
            if let Some(count) = self.aspect_count_per_target.lock().get(&target.key) {
                new_aggregator.set_expect_aspect_completions(*count);
            }
            let mut aggregators = self.aggregators.lock();
            let old_aggregator = aggregators.get(&target.key).cloned();
            check_state!(
                old_aggregator.is_none(),
                "target: {:?}, values: {:?} {:?}",
                target.key,
                old_aggregator.map(|a| a.target.key.clone()),
                new_aggregator.target.key
            );
            aggregators.insert(target.key.clone(), new_aggregator);
        }
    }

    /// Populates the aggregator for a particular top level target, including test targets.
    ///
    /// With skymeld, the corresponding AspectCompleteEvents may arrive before the aggregator is
    /// set up. We therefore need to put those events in a queue and resolve them when the
    /// aggregator becomes available.
    // Java: com/google/devtools/build/lib/runtime/TargetSummaryPublisher.java:149 TargetSummaryPublisher.populateTarget
    pub fn populate_target(&self, target: &TargetSummaryTarget, is_test: bool) {
        let mut aggregators = self.aggregators.lock();
        let new_aggregator = self.create_aggregator_for_target(is_test, target);
        if let Some(count) = self.aspect_count_per_target.lock().get(&target.key) {
            new_aggregator.set_expect_aspect_completions(*count);
        }
        let old_aggregator = aggregators.get(&target.key).cloned();
        check_state!(
            old_aggregator.is_none(),
            "target: {:?}, values: {:?} {:?}",
            target.key,
            old_aggregator.map(|a| a.target.key.clone()),
            new_aggregator.target.key
        );
        aggregators.insert(target.key.clone(), new_aggregator.clone());
        if let Some(queued) = self
            .queued_aspect_complete_events
            .lock()
            .remove(&target.key)
        {
            for success in queued {
                new_aggregator.add_aspect_completion_event(success);
            }
        }
    }

    /// Creates a TargetSummaryAggregator for the given target.
    // Java: com/google/devtools/build/lib/runtime/TargetSummaryPublisher.java:177 TargetSummaryPublisher.createAggregatorForTarget
    fn create_aggregator_for_target(
        &self,
        is_test: bool,
        target: &TargetSummaryTarget,
    ) -> Arc<TargetSummaryAggregator> {
        // We want target summaries for alias targets, but note they don't receive test summaries.
        Arc::new(TargetSummaryAggregator::new(
            target.clone(),
            is_test && !target.is_alias,
            self.has_aspects.load(Ordering::SeqCst),
            self.event_bus.clone(),
        ))
    }

    /// `buildCompleteEvent`: sets overall build success of every target not yet published.
    /// `actual_targets` and `successful_targets` are `result.getActualTargets()` and
    /// `getSuccessfulTargets()` (`None` when the result has none); `successful_aspects` the base
    /// target keys of `getSuccessfulAspects()`.
    // Java: com/google/devtools/build/lib/runtime/TargetSummaryPublisher.java:185 TargetSummaryPublisher.buildCompleteEvent
    pub fn build_complete_event(
        &self,
        actual_targets: Option<&[TargetSummaryKey]>,
        successful_targets: Option<&[TargetSummaryKey]>,
        successful_aspects: &[TargetSummaryKey],
    ) {
        let (Some(actual_targets), Some(successful_targets)) = (actual_targets, successful_targets)
        else {
            return;
        };

        // Count out how many aspects have succeeded for each target
        let mut aspect_successes: HashMap<&TargetSummaryKey, i32> = HashMap::new();
        for key in successful_aspects {
            *aspect_successes.entry(key).or_insert(0) += 1;
        }

        // Now go through all targets and set overall build success. This is a backstop against
        // missing {Target|Aspect}Completed events (e.g., due to interruption or failing fast after
        // failures).
        for target_key in actual_targets {
            // If we have not seen the ToplevelAspectsIdentifiedEvent for a target, and we are
            // expecting aspects, then we know the target failed to build all its aspects as we
            // never even analyzed any aspects. Set expectedAspectSuccesses to Integer.MAX_VALUE
            // to make clear: we failed.
            let expected_aspect_successes = self
                .aspect_count_per_target
                .lock()
                .get(target_key)
                .copied()
                .unwrap_or(if self.has_aspects.load(Ordering::SeqCst) {
                    i32::MAX
                } else {
                    0
                });
            let aggregator = self.aggregators.lock().get(target_key).cloned();
            if let Some(aggregator) = aggregator
                && !aggregator.published.load(Ordering::SeqCst)
            {
                // Overall success means all aspects were successful and the target didn't fail
                // to build
                let successful_aspect_count =
                    aspect_successes.get(target_key).copied().unwrap_or(0);
                check_state!(
                    successful_aspect_count <= expected_aspect_successes,
                    "for target {:?} got {} successful aspects, expected at most {}",
                    target_key,
                    successful_aspect_count,
                    expected_aspect_successes
                );
                aggregator.set_overall_build_success(
                    successful_targets.contains(target_key)
                        && successful_aspect_count == expected_aspect_successes,
                );
            }
        }
    }

    // Java: com/google/devtools/build/lib/runtime/TargetSummaryPublisher.java:229 TargetSummaryPublisher.targetComplete
    pub fn target_complete(&self, key: &TargetSummaryKey, failed: bool) {
        let aggregator = self.aggregators.lock().get(key).cloned();
        if let Some(aggregator) = aggregator
            && !aggregator.published.load(Ordering::SeqCst)
        {
            aggregator.add_completion_event(!failed);
        }
    }

    /// `base_key` is `event.getAspectKey().getBaseConfiguredTargetKey()`.
    // Java: com/google/devtools/build/lib/runtime/TargetSummaryPublisher.java:238 TargetSummaryPublisher.aspectComplete
    pub fn aspect_complete(&self, base_key: &TargetSummaryKey, failed: bool) {
        let aggregator;
        // Prevent a race condition where #populateTarget finishes checking the
        // queuedAspectCompleteEvents before the entries are added by this method.
        {
            let aggregators = self.aggregators.lock();
            aggregator = aggregators.get(base_key).cloned();

            // With skymeld, the corresponding AspectCompleteEvents may arrive before the
            // aggregator is set up. We therefore need to put those events in a queue and resolve
            // them when the aggregator becomes available.
            if self
                .merged_skyframe_analysis_execution
                .load(Ordering::SeqCst)
                && aggregator.is_none()
            {
                self.queued_aspect_complete_events
                    .lock()
                    .entry(base_key.clone())
                    .or_default()
                    .push(!failed);
                return;
            }
        }

        if let Some(aggregator) = aggregator
            && !aggregator.published.load(Ordering::SeqCst)
        {
            aggregator.add_aspect_completion_event(!failed);
        }
    }

    // Java: com/google/devtools/build/lib/runtime/TargetSummaryPublisher.java:262 TargetSummaryPublisher.testSummaryEvent
    pub fn test_summary_event(&self, key: &TargetSummaryKey, status: BlazeTestStatus) {
        let aggregator = self.aggregators.lock().get(key).cloned();
        if let Some(aggregator) = aggregator
            && !aggregator.published.load(Ordering::SeqCst)
        {
            aggregator.set_test_summary(status);
        }
    }
}

/// The mutable state of a [`TargetSummaryAggregator`] (`@GuardedBy("this")`).
struct AggregatorState {
    /// Whether or not the target has completed being built, or if there was any build failure.
    target_completed: bool,
    /// Aspect completion events we're still waiting on (always 0 if `has_build_failure`).
    remaining_aspect_completions: i32,
    has_build_failure: bool,
    test_status: Option<BlazeTestStatus>,
}

// Java: com/google/devtools/build/lib/runtime/TargetSummaryPublisher.java:278 TargetSummaryPublisher.TargetSummaryAggregator
struct TargetSummaryAggregator {
    target: TargetSummaryTarget,
    expect_aspect_completions: bool,
    expect_test_summary: bool,
    /// Whether a TargetSummary for `target` has been published.
    published: AtomicBool,
    state: Mutex<AggregatorState>,
    event_bus: Arc<dyn TargetSummarySink>,
}

impl TargetSummaryAggregator {
    // Java: com/google/devtools/build/lib/runtime/TargetSummaryPublisher.java:303 TargetSummaryAggregator.<init>
    fn new(
        target: TargetSummaryTarget,
        expect_test_summary: bool,
        expect_aspect_completions: bool,
        event_bus: Arc<dyn TargetSummarySink>,
    ) -> Self {
        TargetSummaryAggregator {
            target,
            expect_test_summary,
            expect_aspect_completions,
            published: AtomicBool::new(false),
            state: Mutex::new(AggregatorState {
                target_completed: false,
                remaining_aspect_completions: -1,
                has_build_failure: false,
                test_status: None,
            }),
            event_bus,
        }
    }

    // Java: com/google/devtools/build/lib/runtime/TargetSummaryPublisher.java:312 TargetSummaryAggregator.setExpectAspectCompletions
    fn set_expect_aspect_completions(&self, new_remaining_aspect_completions: i32) {
        let mut st = self.state.lock();
        check_state!(
            self.expect_aspect_completions,
            "Cannot track aspects unless --aspects is set."
        );
        check_state!(
            st.remaining_aspect_completions < 0
                || st.remaining_aspect_completions == new_remaining_aspect_completions
                || st.has_build_failure,
            "Cannot call setExpectAspectCompletions() twice on a single target after aspect \
             completions have begun. Was {}, got {}.",
            st.remaining_aspect_completions,
            new_remaining_aspect_completions
        );
        // If we have already had a build failure (because the target failed) then we have set
        // remainingAspectCompletions = 0 and it should stay at zero.
        if st.has_build_failure {
            return;
        }
        st.remaining_aspect_completions = new_remaining_aspect_completions;
    }

    // Java: com/google/devtools/build/lib/runtime/TargetSummaryPublisher.java:332 TargetSummaryAggregator.addCompletionEvent
    fn add_completion_event(&self, success: bool) {
        let mut st = self.state.lock();
        if st.target_completed {
            return; // already published or still waiting on aspects or test summary
        }
        st.target_completed = true;
        if !success {
            st.remaining_aspect_completions = 0;
            st.has_build_failure = true;
        }
        self.publish_once_when_ready(&st);
    }

    // Java: com/google/devtools/build/lib/runtime/TargetSummaryPublisher.java:344 TargetSummaryAggregator.addAspectCompletionEvent
    fn add_aspect_completion_event(&self, success: bool) {
        let mut st = self.state.lock();
        if st.remaining_aspect_completions <= 0 {
            return; // already published or still waiting on target or test summary
        }
        if success {
            st.remaining_aspect_completions -= 1;
        } else {
            st.target_completed = true;
            st.remaining_aspect_completions = 0;
            st.has_build_failure = true;
        }
        self.publish_once_when_ready(&st);
    }

    // Java: com/google/devtools/build/lib/runtime/TargetSummaryPublisher.java:358 TargetSummaryAggregator.setTestSummary
    fn set_test_summary(&self, status: BlazeTestStatus) {
        let mut st = self.state.lock();
        if st.remaining_aspect_completions <= 0
            && st.target_completed
            && (!self.expect_test_summary || st.test_status.is_some())
        {
            return; // already published
        }
        st.test_status = Some(status);
        self.publish_once_when_ready(&st);
    }

    // Java: com/google/devtools/build/lib/runtime/TargetSummaryPublisher.java:368 TargetSummaryAggregator.setOverallBuildSuccess
    fn set_overall_build_success(&self, success: bool) {
        let mut st = self.state.lock();
        if st.remaining_aspect_completions <= 0 && st.target_completed {
            return; // already published or still waiting on test summary
        }
        st.target_completed = true;
        st.remaining_aspect_completions = 0;
        st.has_build_failure = !success;
        self.publish_once_when_ready(&st);
    }

    /// Publishes `TargetSummaryEvent` for `target` if `has_build_failure` or when we have any
    /// test status as well as all completions (`target_completed` and
    /// `remaining_aspect_completions` == 0).
    // Java: com/google/devtools/build/lib/runtime/TargetSummaryPublisher.java:384 TargetSummaryAggregator.publishOnceWhenReady
    fn publish_once_when_ready(&self, st: &AggregatorState) {
        let already_published = self.published.load(Ordering::SeqCst);
        let waiting_for_target_completion = !st.target_completed;
        let waiting_for_aspect_completions = st.remaining_aspect_completions > 0
            || (self.expect_aspect_completions && st.remaining_aspect_completions == -1);
        let waiting_for_test_status =
            !st.has_build_failure && self.expect_test_summary && st.test_status.is_none();
        if waiting_for_target_completion
            || waiting_for_aspect_completions
            || waiting_for_test_status
        {
            check_state!(
                !already_published,
                "Shouldn't have published yet: {:?}",
                self.target.key
            );
            return;
        }
        if already_published {
            return;
        }
        let event = TargetSummaryEvent::create(
            &self.target.original_label,
            self.target.key.configuration_checksum.as_deref(),
            !st.has_build_failure,
            self.expect_test_summary,
            st.test_status,
        );
        self.event_bus.post(event);

        self.published.store(true, Ordering::SeqCst);
    }
}
