//! Port of `lib/metrics/criticalpath/CriticalPathComputer.java`.
//!
//! PORT: Guava's `EventBus` calls the `@Subscribe` methods with the event objects; here each
//! subscriber method takes the parts of its event it reads (the action and the
//! `BlazeClock.nanoTime()` values), and the command's event-bus subscriber
//! (`blaze_buildtool`'s `ExecutionEventCollector`) unpacks the events. The times are the values
//! of an injectable clock, as in the Java tests.

use std::sync::Arc;
use std::sync::atomic::{AtomicI32, Ordering};

use blaze_actions::Artifact;
use blaze_actions::actions::action::ActionRef;
use blaze_exec::actions::spawn_metrics::SpawnMetrics;
use indexmap::IndexMap;
use parking_lot::Mutex;

use super::aggregated_critical_path::AggregatedCriticalPath;
use super::critical_path_component::CriticalPathComponent;
use crate::metrics::aggregated_spawn_metrics::AggregatedSpawnMetricsBuilder;

/// Number of top actions to record.
// Java: com/google/devtools/build/lib/metrics/criticalpath/CriticalPathComputer.java:63 CriticalPathComputer.SLOWEST_COMPONENTS_SIZE
pub const SLOWEST_COMPONENTS_SIZE: usize = 30;

// Java: com/google/devtools/build/lib/metrics/criticalpath/CriticalPathComputer.java:65 CriticalPathComputer.LARGEST_MEMORY_COMPONENTS_SIZE
const LARGEST_MEMORY_COMPONENTS_SIZE: usize = 20;
// Java: com/google/devtools/build/lib/metrics/criticalpath/CriticalPathComputer.java:66 CriticalPathComputer.LARGEST_INPUT_SIZE_COMPONENTS_SIZE
const LARGEST_INPUT_SIZE_COMPONENTS_SIZE: usize = 20;
// Java: com/google/devtools/build/lib/metrics/criticalpath/CriticalPathComputer.java:67 CriticalPathComputer.LARGEST_INPUT_COUNT_COMPONENTS_SIZE
const LARGEST_INPUT_COUNT_COMPONENTS_SIZE: usize = 20;

/// `Actions.canBeShared(actionKeyContext, newAction, oldAction)`, supplied by the command (the
/// action key context is the executor's).
pub type CanBeShared = Arc<dyn Fn(&ActionRef, &ActionRef) -> bool + Send + Sync>;

/// Selects and returns the longer of two components (the first may be `null`).
// Java: com/google/devtools/build/lib/metrics/criticalpath/CriticalPathComputer.java:70 CriticalPathComputer.SELECT_LONGER_COMPONENT
fn select_longer_component(
    a: Option<Arc<CriticalPathComponent>>,
    b: Arc<CriticalPathComponent>,
) -> Arc<CriticalPathComponent> {
    match a {
        Some(a) if a.get_aggregated_elapsed_time() >= b.get_aggregated_elapsed_time() => a,
        _ => b,
    }
}

/// Java reference equality of two actions.
fn same_action(a: &ActionRef, b: &ActionRef) -> bool {
    std::ptr::addr_eq(Arc::as_ptr(a), Arc::as_ptr(b))
}

/// Computes the critical path in the action graph based on events published to the event bus.
///
/// After instantiation, this object needs to be registered on the event bus to work.
// Java: com/google/devtools/build/lib/metrics/criticalpath/CriticalPathComputer.java:58 CriticalPathComputer
pub struct CriticalPathComputer {
    id_generator: AtomicI32,
    /// outputArtifactToComponent is accessed from multiple event handlers.
    ///
    /// PORT: java-hash-order. Java's `ConcurrentHashMap` iteration order is observable only in
    /// the `getSlowestComponents`-style rankings among components with equal values (a log line
    /// and nothing that reaches output); insertion order is kept here.
    output_artifact_to_component: Mutex<IndexMap<Artifact, Arc<CriticalPathComponent>>>,
    can_be_shared: CanBeShared,
    /// Maximum critical path found.
    max_critical_path: Mutex<Option<Arc<CriticalPathComponent>>>,
}

impl CriticalPathComputer {
    /// PORT: Java takes the `ActionKeyContext` (for `Actions.canBeShared`) and the Skyframe
    /// graph (for `actionChangePruned`); the former is the `can_be_shared` callback, and the
    /// latter is the caller's (see [`CriticalPathComputer::action_change_pruned`]).
    // Java: com/google/devtools/build/lib/metrics/criticalpath/CriticalPathComputer.java:86 CriticalPathComputer.<init>
    pub fn new(can_be_shared: CanBeShared) -> CriticalPathComputer {
        CriticalPathComputer {
            id_generator: AtomicI32::new(0),
            output_artifact_to_component: Mutex::new(IndexMap::new()),
            can_be_shared,
            max_critical_path: Mutex::new(None),
        }
    }

    /// Creates a critical path component for an action.
    ///
    /// `relative_start_nanos` is the time when the action started to run in nanos. Only meant to
    /// be used for computing time differences.
    // Java: com/google/devtools/build/lib/metrics/criticalpath/CriticalPathComputer.java:98 CriticalPathComputer.createComponent
    fn create_component(
        &self,
        action: &ActionRef,
        relative_start_nanos: i64,
    ) -> CriticalPathComponent {
        CriticalPathComponent::new(
            self.id_generator.fetch_add(1, Ordering::SeqCst),
            action.clone(),
            relative_start_nanos,
        )
    }

    /// Return the critical path stats for the current command execution.
    ///
    /// This method allows us to calculate lazily the aggregate statistics of the critical path,
    /// avoiding the memory and cpu penalty for doing it for all the actions executed.
    // Java: com/google/devtools/build/lib/metrics/criticalpath/CriticalPathComputer.java:108 CriticalPathComputer.aggregate
    pub fn aggregate(&self) -> AggregatedCriticalPath {
        let Some(critical_path) = self.get_max_critical_path() else {
            return AggregatedCriticalPath::empty();
        };

        let mut components = Vec::new();
        let mut metrics_builder = AggregatedSpawnMetricsBuilder::new();
        let mut child = Some(critical_path.clone());

        while let Some(c) = child {
            let child_spawn_metrics = c.get_spawn_metrics();
            metrics_builder.add_durations_aggregated(&child_spawn_metrics);
            metrics_builder.add_non_durations_aggregated(&child_spawn_metrics);
            child = c.get_child();
            components.push(c);
        }

        AggregatedCriticalPath::new(
            critical_path.get_aggregated_elapsed_time(),
            metrics_builder.build(),
            components,
        )
    }

    // Java: com/google/devtools/build/lib/metrics/criticalpath/CriticalPathComputer.java:132 CriticalPathComputer.getCriticalPathComponentsMap
    pub fn get_critical_path_components_map(
        &self,
    ) -> IndexMap<Artifact, Arc<CriticalPathComponent>> {
        self.output_artifact_to_component.lock().clone()
    }

    /// Changes the phase of the action (`SpawnExecutedEvent.ChangePhase`; `primary_output` is
    /// `phase.getAction().getPrimaryOutput()`).
    // Java: com/google/devtools/build/lib/metrics/criticalpath/CriticalPathComputer.java:137 CriticalPathComputer.nextCriticalPathPhase
    pub fn next_critical_path_phase(&self, primary_output: &Artifact) {
        let stats = self
            .output_artifact_to_component
            .lock()
            .get(primary_output)
            .cloned();
        if let Some(stats) = stats {
            stats.change_phase();
        }
    }

    /// Adds spawn metrics to the action stats (`SpawnExecutedEvent`: the action's primary output,
    /// the spawn result's metrics, runner name and subtype, and `wasRemote()`).
    // Java: com/google/devtools/build/lib/metrics/criticalpath/CriticalPathComputer.java:148 CriticalPathComputer.spawnExecuted
    pub fn spawn_executed(
        &self,
        primary_output: Option<&Artifact>,
        metrics: &SpawnMetrics,
        runner_name: &str,
        runner_subtype: &str,
        was_remote: bool,
    ) {
        let Some(primary_output) = primary_output else {
            // Despite the documentation to the contrary, the SpawnIncludeScanner creates an
            // ActionExecutionMetadata instance that returns a null primary output. That said,
            // this class is incorrect wrt. multiple Spawns in a single action. See b/111583707.
            return;
        };
        let stats = self
            .output_artifact_to_component
            .lock()
            .get(primary_output)
            .cloned()
            .unwrap_or_else(|| panic!("NullPointerException"));

        stats.add_spawn_result(metrics, Some(runner_name), runner_subtype, was_remote);
    }

    /// Returns the list of components using the most memory.
    // Java: com/google/devtools/build/lib/metrics/criticalpath/CriticalPathComputer.java:171 CriticalPathComputer.getLargestMemoryComponents
    pub fn get_largest_memory_components(&self) -> Vec<Arc<CriticalPathComponent>> {
        greatest(self.unique_actions(), LARGEST_MEMORY_COMPONENTS_SIZE, |c| {
            c.get_spawn_metrics()
                .get_max_non_duration(0, |m| m.memory_estimate_bytes)
        })
    }

    /// Returns the list of components with the largest input sizes.
    // Java: com/google/devtools/build/lib/metrics/criticalpath/CriticalPathComputer.java:182 CriticalPathComputer.getLargestInputSizeComponents
    pub fn get_largest_input_size_components(&self) -> Vec<Arc<CriticalPathComponent>> {
        greatest(
            self.unique_actions(),
            LARGEST_INPUT_SIZE_COMPONENTS_SIZE,
            |c| {
                c.get_spawn_metrics()
                    .get_max_non_duration(0, |m| m.input_bytes)
            },
        )
    }

    /// Returns the list of components with the largest input counts.
    // Java: com/google/devtools/build/lib/metrics/criticalpath/CriticalPathComputer.java:192 CriticalPathComputer.getLargestInputCountComponents
    pub fn get_largest_input_count_components(&self) -> Vec<Arc<CriticalPathComponent>> {
        greatest(
            self.unique_actions(),
            LARGEST_INPUT_COUNT_COMPONENTS_SIZE,
            |c| {
                c.get_spawn_metrics()
                    .get_max_non_duration(0, |m| m.input_files)
            },
        )
    }

    /// Returns the list of slowest components.
    // Java: com/google/devtools/build/lib/metrics/criticalpath/CriticalPathComputer.java:202 CriticalPathComputer.getSlowestComponents
    pub fn get_slowest_components(&self) -> Vec<Arc<CriticalPathComponent>> {
        greatest(self.unique_actions(), SLOWEST_COMPONENTS_SIZE, |c| {
            c.get_elapsed_time_nanos()
        })
    }

    // Java: com/google/devtools/build/lib/metrics/criticalpath/CriticalPathComputer.java:210 CriticalPathComputer.uniqueActions
    fn unique_actions(&self) -> Vec<Arc<CriticalPathComponent>> {
        self.output_artifact_to_component
            .lock()
            .iter()
            .filter(|(k, v)| v.is_primary_output(k))
            .map(|(_, v)| v.clone())
            .collect()
    }

    /// Creates a CriticalPathComponent and adds the duration of input discovery and changes
    /// phase (`DiscoveredInputsEvent`).
    // Java: com/google/devtools/build/lib/metrics/criticalpath/CriticalPathComputer.java:217 CriticalPathComputer.discoverInputs
    pub fn discover_inputs(
        &self,
        action: &ActionRef,
        start_time_nanos: i64,
        metrics: &SpawnMetrics,
    ) {
        let stats = self.try_add_component(self.create_component(action, start_time_nanos));
        stats.add_spawn_result(metrics, None, "", /* was_remote= */ false);
        stats.change_phase();
    }

    /// Record an action that has started to run. If the CriticalPathComponent has not been
    /// created, initialize it and then start running (`ActionStartedEvent`).
    // Java: com/google/devtools/build/lib/metrics/criticalpath/CriticalPathComputer.java:232 CriticalPathComputer.actionStarted
    pub fn action_started(&self, action: &ActionRef, nano_time_start: i64) {
        self.try_add_component(self.create_component(action, nano_time_start))
            .start_running();
    }

    /// Try to add the component to the map of critical path components. If there is an existing
    /// component for its primary output it uses that to update the rest of the outputs.
    ///
    /// Returns the component to be used for updating the time stats.
    // Java: com/google/devtools/build/lib/metrics/criticalpath/CriticalPathComputer.java:245 CriticalPathComputer.tryAddComponent
    fn try_add_component(
        &self,
        new_component: CriticalPathComponent,
    ) -> Arc<CriticalPathComponent> {
        let new_component = Arc::new(new_component);
        let new_action = new_component.get_action().clone();
        let primary_output = new_action.get_primary_output();
        // putIfAbsent
        let stored = {
            let mut map = self.output_artifact_to_component.lock();
            match map.get(&primary_output) {
                Some(existing) => Some(existing.clone()),
                None => {
                    map.insert(primary_output.clone(), new_component.clone());
                    None
                }
            }
        };
        let stored_component = match stored {
            Some(stored_component) => {
                let old_action = stored_component.get_action().clone();
                // TODO(b/120663721) Replace this fragile reference equality check with something
                // principled.
                // PORT: the lock is released before canBeShared (it computes action keys).
                if !same_action(&old_action, &new_action)
                    && !(self.can_be_shared)(&new_action, &old_action)
                {
                    // PORT: Java appends the actions' toString(); `Debug` stands in for it in
                    // this crash message.
                    panic!(
                        "IllegalStateException: Duplicate output artifact found for unsharable \
                         actions.This can happen if a previous event registered the action.\n\
                         Old action: {old_action:?}\n\nNew action: {new_action:?}\n\n\
                         Artifact: {primary_output}\n"
                    );
                }
                stored_component
            }
            None => new_component,
        };
        // Try to insert the existing component for the rest of the outputs even if we failed to
        // be the ones inserting the component so that at the end of this method we guarantee
        // that all the outputs have a component.
        for output in new_action.get_outputs() {
            if output == primary_output {
                continue;
            }
            let mut map = self.output_artifact_to_component.lock();
            let old = match map.get(&output) {
                Some(old) => Some(old.clone()),
                None => {
                    map.insert(output, stored_component.clone());
                    None
                }
            };
            // If two actions run concurrently maybe we find a component by primary output but we
            // are the first updating the rest of the outputs.
            if let Some(old) = old
                && !Arc::ptr_eq(&old, &stored_component)
            {
                panic!("IllegalStateException: Inconsistent state for {new_action:?}");
            }
        }
        stored_component
    }

    /// Record an action that was not executed because it was in the (disk) cache. This is needed
    /// so that we can calculate correctly the dependencies tree if we have some cached actions in
    /// the middle of the critical path (`CachedActionEvent`).
    // Java: com/google/devtools/build/lib/metrics/criticalpath/CriticalPathComputer.java:291 CriticalPathComputer.actionCached
    pub fn action_cached(&self, action: &ActionRef, nano_time_start: i64, nano_time_finish: i64) {
        let component = self.try_add_component(self.create_component(action, nano_time_start));
        self.finalize_action_stat(
            nano_time_start,
            nano_time_finish,
            action,
            &component,
            "action cache hit",
        );
    }

    /// Records the elapsed time stats for the action. For each input artifact, it finds the real
    /// dependent artifacts and records the critical path stats (`ActionCompletionEvent`).
    // Java: com/google/devtools/build/lib/metrics/criticalpath/CriticalPathComputer.java:305 CriticalPathComputer.actionComplete
    pub fn action_complete(
        &self,
        action: &ActionRef,
        relative_action_start_time_nanos: i64,
        finish_time_nanos: i64,
    ) {
        let component = self
            .output_artifact_to_component
            .lock()
            .get(&action.get_primary_output())
            .cloned()
            .unwrap_or_else(|| panic!("NullPointerException: {action:?}"));
        self.finalize_action_stat(
            relative_action_start_time_nanos,
            finish_time_nanos,
            action,
            &component,
            "",
        );
    }

    /// `ActionChangePrunedEvent`.
    ///
    /// PORT: Java looks the action up in the Skyframe graph
    /// (`Actions.getAction(graph, event.actionLookupData())`) and returns when there is no graph
    /// or the value is not an `Action`; the caller does that lookup and passes the result as
    /// `action`.
    // Java: com/google/devtools/build/lib/metrics/criticalpath/CriticalPathComputer.java:316 CriticalPathComputer.actionChangePruned
    pub fn action_change_pruned(&self, action: Option<&ActionRef>, finish_time_nanos: i64) {
        let Some(action) = action else {
            return;
        };

        let component = self.try_add_component(self.create_component(action, finish_time_nanos));
        self.finalize_action_stat(
            finish_time_nanos,
            finish_time_nanos,
            action,
            &component,
            "change pruned",
        );
    }

    /// Record that the failed rewound action is no longer running. The action may or may not
    /// start again later (`ActionRewoundEvent`).
    // Java: com/google/devtools/build/lib/metrics/criticalpath/CriticalPathComputer.java:337 CriticalPathComputer.actionRewound
    pub fn action_rewound(
        &self,
        failed_rewound_action: &ActionRef,
        relative_action_start_time_nanos: i64,
        relative_action_finish_time_nanos: i64,
    ) {
        let component = self
            .output_artifact_to_component
            .lock()
            .get(&failed_rewound_action.get_primary_output())
            .cloned()
            .unwrap_or_else(|| panic!("NullPointerException"));
        component.finish_action_execution(
            relative_action_start_time_nanos,
            relative_action_finish_time_nanos,
            "action rewound",
        );
    }

    /// Maximum critical path component found during the build.
    // Java: com/google/devtools/build/lib/metrics/criticalpath/CriticalPathComputer.java:350 CriticalPathComputer.getMaxCriticalPath
    pub fn get_max_critical_path(&self) -> Option<Arc<CriticalPathComponent>> {
        self.max_critical_path.lock().clone()
    }

    // Java: com/google/devtools/build/lib/metrics/criticalpath/CriticalPathComputer.java:354 CriticalPathComputer.finalizeActionStat
    fn finalize_action_stat(
        &self,
        start_time_nanos: i64,
        finish_time_nanos: i64,
        action: &ActionRef,
        component: &Arc<CriticalPathComponent>,
        finalize_reason: &str,
    ) {
        for input in action.get_inputs().to_list().iter() {
            self.add_artifact_dependency(component, input, finish_time_nanos);
        }
        // PORT: skipped, logging: the warning for a negative duration ("Negative duration time
        // for [...]") goes to the server log.
        component.finish_action_execution(start_time_nanos, finish_time_nanos, finalize_reason);
        // maxCriticalPath.accumulateAndGet(component, SELECT_LONGER_COMPONENT)
        let mut max = self.max_critical_path.lock();
        *max = Some(select_longer_component(max.take(), component.clone()));
    }

    /// If "input" is a generated artifact, link its critical path to the one we're building.
    // Java: com/google/devtools/build/lib/metrics/criticalpath/CriticalPathComputer.java:374 CriticalPathComputer.addArtifactDependency
    fn add_artifact_dependency(
        &self,
        action_stats: &Arc<CriticalPathComponent>,
        input: &Artifact,
        component_finish_nanos: i64,
    ) {
        let dep_component = self.output_artifact_to_component.lock().get(input).cloned();

        // Typically, the dep component should already be finished since its output was used as
        // an input for a just-completed action. However, we tolerate it still running for (a)
        // action rewinding and (b) the rare case that an action depending on a previously-cached
        // shared action sees a different shared action that is in the midst of being an action
        // cache hit.
        if let Some(dep_component) = dep_component
            && !dep_component.is_running()
        {
            action_stats.add_dep_info(&dep_component, component_finish_nanos);
        }
    }
}

/// `Comparators.greatest(k, Comparator.comparingLong(key))`: the `k` greatest elements, in
/// descending order.
///
/// PORT: Guava's order among equal keys follows its selection algorithm over the map's
/// iteration order; this keeps the iteration order (a stable sort).
fn greatest(
    components: Vec<Arc<CriticalPathComponent>>,
    k: usize,
    key: impl Fn(&CriticalPathComponent) -> i64,
) -> Vec<Arc<CriticalPathComponent>> {
    let mut keyed: Vec<(i64, Arc<CriticalPathComponent>)> =
        components.into_iter().map(|c| (key(&c), c)).collect();
    keyed.sort_by_key(|b| std::cmp::Reverse(b.0));
    keyed.truncate(k);
    keyed.into_iter().map(|(_, c)| c).collect()
}
