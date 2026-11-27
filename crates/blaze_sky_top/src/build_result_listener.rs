//! Port of `skyframe/BuildResultListener.java`.

use std::sync::Arc;

use blaze_analysis::aspect_key_creator::AspectKey;
use blaze_analysis::aspect_value::AspectValue;
use blaze_analysis::configured_target::ConfiguredTarget;
use blaze_sky_analysis::top_level_status_events::{
    TopLevelTargetAnalyzedEvent, TopLevelTargetSkippedEvent,
};
use blaze_sky_exec::events::{TestAnalyzedEvent, downcast_event};
use indexmap::IndexMap;
use parking_lot::Mutex;

use crate::analysis_phase_runner::AspectAnalyzedEvent;
use crate::execution_seam::SkyframeConfiguredTargetView;

/// Listens to the various status events of the top level targets/aspects.
///
/// WARNING: For consistency, the getter methods should only be used after the execution phase is
/// finished.
///
/// PORT: the built targets and aspects (`TopLevelTargetBuiltEvent`, `AspectBuiltEvent`) are read
/// from the evaluation by the build command (`top_level_target_built`, `top_level_aspect_built`
/// in blaze_runtime commands/build.rs); their part of the class is not here.
///
/// PORT: java-hash-order. Java's sets are `ConcurrentHashMap.newKeySet()` over identity hash
/// codes, so the order of the result lines of several targets changes from run to run; here it is
/// the order of the events.
// Java: com/google/devtools/build/lib/skyframe/BuildResultListener.java:42 BuildResultListener
// PORT: skipped, read from the evaluation (see above): BuildResultListener.addBuiltTarget BuildResultListener.addBuiltAspect BuildResultListener.getBuiltTargets BuildResultListener.getBuiltAspects
#[derive(Default)]
pub struct BuildResultListener {
    // Also includes test targets.
    analyzed_targets: Mutex<Vec<ConfiguredTarget>>,
    analyzed_tests: Mutex<Vec<ConfiguredTarget>>,
    analyzed_aspects: Mutex<IndexMap<AspectKey, Arc<AspectValue>>>,
    // Also includes test targets.
    skipped_targets: Mutex<Vec<ConfiguredTarget>>,
    /// PORT: not in Java's class. The `BuildDriverKey`s of the merged evaluation, from which
    /// the build command reads the built targets and aspects in the graph (see the type).
    top_level_keys: Mutex<Vec<blaze_sky_exec::keys::BuildDriverKey>>,
}

impl BuildResultListener {
    /// `Set.add`: a configured target is one instance per lookup key.
    fn add(set: &Mutex<Vec<ConfiguredTarget>>, target: &ConfiguredTarget) {
        let mut set = set.lock();
        if !set
            .iter()
            .any(|t| t.get_lookup_key() == target.get_lookup_key())
        {
            set.push(target.clone());
        }
    }

    // Java: com/google/devtools/build/lib/skyframe/BuildResultListener.java:55 BuildResultListener.addAnalyzedTarget
    pub fn add_analyzed_target(&self, event: &TopLevelTargetAnalyzedEvent) {
        Self::add(&self.analyzed_targets, &event.configured_target);
    }

    /// PORT: the event carries the execution phase's view of the target; the analysis crates'
    /// view holds the `ConfiguredTarget`.
    // Java: com/google/devtools/build/lib/skyframe/BuildResultListener.java:61 BuildResultListener.addAnalyzedTest
    pub fn add_analyzed_test(&self, event: &TestAnalyzedEvent) {
        let view = event
            .configured_target
            .as_any()
            .downcast_ref::<SkyframeConfiguredTargetView>()
            .unwrap_or_else(|| {
                panic!(
                    "ClassCastException: {:?} is not a ConfiguredTarget",
                    event.configured_target
                )
            });
        Self::add(&self.analyzed_tests, view.get_configured_target());
    }

    // Java: com/google/devtools/build/lib/skyframe/BuildResultListener.java:67 BuildResultListener.addAnalyzedAspect
    pub fn add_analyzed_aspect(&self, event: &AspectAnalyzedEvent) {
        self.analyzed_aspects
            .lock()
            .insert(event.aspect_key.clone(), event.configured_aspect.clone());
    }

    // Java: com/google/devtools/build/lib/skyframe/BuildResultListener.java:73 BuildResultListener.addSkippedTarget
    pub fn add_skipped_target(&self, event: &TopLevelTargetSkippedEvent) {
        Self::add(&self.skipped_targets, &event.configured_target);
    }

    // Java: com/google/devtools/build/lib/skyframe/BuildResultListener.java:89 BuildResultListener.getAnalyzedTargets
    pub fn get_analyzed_targets(&self) -> Vec<ConfiguredTarget> {
        self.analyzed_targets.lock().clone()
    }

    // Java: com/google/devtools/build/lib/skyframe/BuildResultListener.java:93 BuildResultListener.getAnalyzedTests
    pub fn get_analyzed_tests(&self) -> Vec<ConfiguredTarget> {
        self.analyzed_tests.lock().clone()
    }

    // Java: com/google/devtools/build/lib/skyframe/BuildResultListener.java:97 BuildResultListener.getAnalyzedAspects
    pub fn get_analyzed_aspects(&self) -> IndexMap<AspectKey, Arc<AspectValue>> {
        self.analyzed_aspects.lock().clone()
    }

    /// See the field.
    pub fn set_top_level_keys(&self, keys: Vec<blaze_sky_exec::keys::BuildDriverKey>) {
        *self.top_level_keys.lock() = keys;
    }

    /// See the field.
    pub fn get_top_level_keys(&self) -> Vec<blaze_sky_exec::keys::BuildDriverKey> {
        self.top_level_keys.lock().clone()
    }

    // Java: com/google/devtools/build/lib/skyframe/BuildResultListener.java:101 BuildResultListener.getSkippedTargets
    pub fn get_skipped_targets(&self) -> Vec<ConfiguredTarget> {
        self.skipped_targets.lock().clone()
    }
}

/// `@Subscribe`: the listener's subscriptions on the command's event bus.
impl blaze_events::EventBus for BuildResultListener {
    fn post(&self, event: Arc<dyn blaze_events::Postable>) {
        if let Some(event) = downcast_event::<TopLevelTargetAnalyzedEvent>(&event) {
            self.add_analyzed_target(event);
        } else if let Some(event) = downcast_event::<TestAnalyzedEvent>(&event) {
            self.add_analyzed_test(event);
        } else if let Some(event) = downcast_event::<AspectAnalyzedEvent>(&event) {
            self.add_analyzed_aspect(event);
        } else if let Some(event) = downcast_event::<TopLevelTargetSkippedEvent>(&event) {
            self.add_skipped_target(event);
        }
    }
}
