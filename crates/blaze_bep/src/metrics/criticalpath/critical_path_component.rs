//! Port of `lib/metrics/criticalpath/CriticalPathComponent.java`.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use blaze_actions::Artifact;
use blaze_actions::actions::action::ActionRef;
use blaze_cmdline::Label;
use blaze_exec::actions::spawn_metrics::{ExecKind, SpawnMetrics};
use parking_lot::Mutex;

use crate::metrics::aggregated_spawn_metrics::AggregatedSpawnMetrics;
use blaze_events::java_format::java_format_fixed;

/// Empty metrics used to simplify handling of `phaseMaxMetrics`.
// Java: com/google/devtools/build/lib/metrics/criticalpath/CriticalPathComponent.java:38 CriticalPathComponent.EMPTY_PLACEHOLDER_METRICS
fn empty_placeholder_metrics() -> SpawnMetrics {
    // SpawnMetrics.Builder.forOtherExec().build()
    SpawnMetrics::for_exec(ExecKind::Other)
}

/// `Duration.ofNanos(nanos).toMillis()`: floor division (a `Duration` keeps whole seconds
/// rounded down and a non-negative nano part).
pub(crate) fn nanos_to_millis(nanos: i64) -> i64 {
    nanos.div_euclid(1_000_000)
}

struct State {
    // These two fields are values of BlazeClock.nanoTime() at the relevant points in time.
    start_nanos: i64,
    finish_nanos: i64,
    /// The longest aggregate runtime of this component and its critical path.
    aggregated_elapsed_time: i64,
    /// Spawn metrics for this action.
    phase_max_metrics: SpawnMetrics,
    total_spawn_metrics: AggregatedSpawnMetrics,
    longest_running_total_duration_in_ms: i32,
    phase_change: bool,
    /// Name of the runner used for the spawn.
    longest_phase_spawn_runner_name: Option<String>,
    /// Details about the runner used for the spawn.
    longest_phase_spawn_runner_subtype: Option<String>,
    /// Child with the maximum critical path.
    child: Option<Arc<CriticalPathComponent>>,
    /// Indication that there is at least one remote spawn metrics received.
    remote: bool,
}

/// A component of the graph over which the critical path is computed. This may be identical to
/// the action graph, but does not have to be - it may also take into account individual spawns
/// run as part of an action.
///
/// PORT: Java's `synchronized` methods and unsynchronized field reads become one mutex over the
/// mutable state; `isRunning` is Java's `volatile` field.
// Java: com/google/devtools/build/lib/metrics/criticalpath/CriticalPathComponent.java:35 CriticalPathComponent
pub struct CriticalPathComponent {
    state: Mutex<State>,
    is_running: AtomicBool,
    action: ActionRef,
    primary_output: Artifact,
    /// An unique identifier of the component for one build execution.
    id: i32,
}

impl std::fmt::Debug for CriticalPathComponent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.to_string())
    }
}

impl CriticalPathComponent {
    // Java: com/google/devtools/build/lib/metrics/criticalpath/CriticalPathComponent.java:75 CriticalPathComponent.<init>
    pub fn new(id: i32, action: ActionRef, start_nanos: i64) -> CriticalPathComponent {
        let primary_output = action.get_primary_output();
        CriticalPathComponent {
            state: Mutex::new(State {
                start_nanos,
                finish_nanos: 0,
                aggregated_elapsed_time: 0,
                phase_max_metrics: empty_placeholder_metrics(),
                total_spawn_metrics: AggregatedSpawnMetrics::empty(),
                longest_running_total_duration_in_ms: 0,
                phase_change: false,
                longest_phase_spawn_runner_name: None,
                longest_phase_spawn_runner_subtype: None,
                child: None,
                remote: false,
            }),
            is_running: AtomicBool::new(false),
            action,
            primary_output,
            id,
        }
    }

    /// Record the elapsed time in case the new duration is greater. This method could be called
    /// multiple times in the following cases:
    ///
    /// 1. Shared actions run concurrently, and the one that really gets executed takes more time
    ///    to send the finish event and the one that was a cache hit manages to send the event
    ///    before.
    /// 2. An action gets rewound, and is later reattempted.
    ///
    /// In both these cases we overwrite the components' times if the later call specifies a
    /// greater duration.
    ///
    /// In the former case the logic is known to be incorrect, as other actions that depend on
    /// this action will not necessarily use the correct getElapsedTimeNanos(). But we do not want
    /// to block action execution because of this. So in certain conditions we might see another
    /// path as the critical path.
    ///
    /// In addition, in the case of sequential spawns, Aggregate the last phase's duration values
    /// with the total spawn metrics. To make sure not to add the last phase's duration multiple
    /// times, only add if there is duration and reset the phase metrics once it has been
    /// aggregated.
    // Java: com/google/devtools/build/lib/metrics/criticalpath/CriticalPathComponent.java:102 CriticalPathComponent.finishActionExecution
    pub fn finish_action_execution(
        &self,
        start_nanos: i64,
        finish_nanos: i64,
        finalize_reason: &str,
    ) {
        let mut st = self.state.lock();
        let is_running = self.is_running.load(Ordering::SeqCst);
        if is_running || finish_nanos - start_nanos > self.get_elapsed_time_nanos_locked(&st) {
            st.start_nanos = start_nanos;
            st.finish_nanos = finish_nanos;
            // In case aggregatedElapsedTime was never set (such as a leaf node with no
            // depedencies) with #addDepInfo, we want to set it here in which case the elapsed
            // time is just the run time of this component.
            st.aggregated_elapsed_time = st
                .aggregated_elapsed_time
                .max(st.finish_nanos - st.start_nanos);
            self.is_running.store(false, Ordering::SeqCst);
            if st.longest_phase_spawn_runner_name.is_none() && !finalize_reason.is_empty() {
                // This is probably not the best way to do it in face of getting called multiple
                // times.
                st.longest_phase_spawn_runner_name = Some(finalize_reason.to_owned());
                st.longest_phase_spawn_runner_subtype = Some(String::new());
                st.longest_running_total_duration_in_ms =
                    nanos_to_millis(st.finish_nanos - st.start_nanos) as i32;
            }
        }

        // If the phaseMaxMetrics has Duration, then we want to aggregate it to the total.
        if !st.phase_max_metrics.is_empty() {
            st.total_spawn_metrics = st
                .total_spawn_metrics
                .sum_durations_max_other(&st.phase_max_metrics);
            st.phase_max_metrics = empty_placeholder_metrics();
        }
    }

    /// PORT: Java compares references ("the keys in the CriticalPathComputer are exactly the
    /// values returned from action.getPrimaryOutput()"); the map's keys are unique by `equals`,
    /// so `==` selects the same entries.
    // Java: com/google/devtools/build/lib/metrics/criticalpath/CriticalPathComponent.java:130 CriticalPathComponent.isPrimaryOutput
    pub fn is_primary_output(&self, possible_primary_output: &Artifact) -> bool {
        *possible_primary_output == self.primary_output
    }

    /// The action for which we are storing the stat.
    // Java: com/google/devtools/build/lib/metrics/criticalpath/CriticalPathComponent.java:137 CriticalPathComponent.getAction
    pub fn get_action(&self) -> &ActionRef {
        &self.action
    }

    /// This is called by `CriticalPathComputer.actionStarted` to start running the action. The
    /// three scenarios where this would occur is:
    ///
    /// 1. A new CriticalPathComponent is created and should start running.
    /// 2. A CriticalPathComponent has been created with discover inputs and beginning to execute.
    /// 3. An action was rewound and starts again.
    // Java: com/google/devtools/build/lib/metrics/criticalpath/CriticalPathComponent.java:151 CriticalPathComponent.startRunning
    pub fn start_running(&self) {
        self.is_running.store(true, Ordering::SeqCst);
    }

    // Java: com/google/devtools/build/lib/metrics/criticalpath/CriticalPathComponent.java:155 CriticalPathComponent.isRunning
    pub fn is_running(&self) -> bool {
        self.is_running.load(Ordering::SeqCst)
    }

    // Java: com/google/devtools/build/lib/metrics/criticalpath/CriticalPathComponent.java:159 CriticalPathComponent.prettyPrintAction
    pub fn pretty_print_action(&self) -> String {
        self.action.pretty_print()
    }

    // Java: com/google/devtools/build/lib/metrics/criticalpath/CriticalPathComponent.java:164 CriticalPathComponent.getOwner
    pub fn get_owner(&self) -> Option<Label> {
        self.action.get_owner().get_label().cloned()
    }

    // Java: com/google/devtools/build/lib/metrics/criticalpath/CriticalPathComponent.java:172 CriticalPathComponent.getMnemonic
    pub fn get_mnemonic(&self) -> String {
        self.action.get_mnemonic()
    }

    /// An unique identifier of the component for one build execution.
    // Java: com/google/devtools/build/lib/metrics/criticalpath/CriticalPathComponent.java:177 CriticalPathComponent.getId
    pub fn get_id(&self) -> i32 {
        self.id
    }

    /// An action can run multiple spawns. Those calls can be sequential or parallel. If action is
    /// a sequence of calls we aggregate the SpawnMetrics of all the SpawnResults. If there are
    /// multiples of the same action run in parallel, we keep the maximum runtime SpawnMetrics. We
    /// will also set the longestPhaseSpawnRunnerName to the longest running spawn runner name
    /// across all phases if it exists.
    // Java: com/google/devtools/build/lib/metrics/criticalpath/CriticalPathComponent.java:188 CriticalPathComponent.addSpawnResult
    pub fn add_spawn_result(
        &self,
        metrics: &SpawnMetrics,
        runner_name: Option<&str>,
        runner_subtype: &str,
        was_remote: bool,
    ) {
        let mut st = self.state.lock();
        // Mark this component as having remote components if _any_ spawn result contributing
        // to it contains meaningful remote metrics. Subsequent non-remote spawns in an action
        // must not reset this flag.
        if was_remote {
            st.remote = true;
        }
        if st.phase_change {
            if !st.phase_max_metrics.is_empty() {
                st.total_spawn_metrics = st
                    .total_spawn_metrics
                    .sum_durations_max_other(&st.phase_max_metrics);
            }
            st.phase_max_metrics = metrics.clone();
            st.phase_change = false;
        } else if metrics.total_time_in_ms > st.phase_max_metrics.total_time_in_ms {
            st.phase_max_metrics = metrics.clone();
        }

        if let Some(runner_name) = runner_name
            && metrics.total_time_in_ms > st.longest_running_total_duration_in_ms
        {
            st.longest_phase_spawn_runner_name = Some(runner_name.to_owned());
            st.longest_phase_spawn_runner_subtype = Some(runner_subtype.to_owned());
            st.longest_running_total_duration_in_ms = metrics.total_time_in_ms;
        }
    }

    /// Set the phaseChange flag as true so we will aggregate incoming spawnMetrics.
    // Java: com/google/devtools/build/lib/metrics/criticalpath/CriticalPathComponent.java:214 CriticalPathComponent.changePhase
    pub fn change_phase(&self) {
        self.state.lock().phase_change = true;
    }

    /// Returns total spawn metrics of the maximum (longest running) spawn metrics of all phases
    /// for the execution of the action.
    // Java: com/google/devtools/build/lib/metrics/criticalpath/CriticalPathComponent.java:222 CriticalPathComponent.getSpawnMetrics
    pub fn get_spawn_metrics(&self) -> AggregatedSpawnMetrics {
        self.state.lock().total_spawn_metrics.clone()
    }

    /// Returns name of the maximum runner used for the finished spawn which took most time, null
    /// if no spawns have finished for this action (either there are no spawns or we asked before
    /// any have finished).
    // Java: com/google/devtools/build/lib/metrics/criticalpath/CriticalPathComponent.java:232 CriticalPathComponent.getLongestPhaseSpawnRunnerName
    pub fn get_longest_phase_spawn_runner_name(&self) -> Option<String> {
        self.state.lock().longest_phase_spawn_runner_name.clone()
    }

    /// Like getLongestPhaseSpawnRunnerName(), but returns the runner details.
    // Java: com/google/devtools/build/lib/metrics/criticalpath/CriticalPathComponent.java:238 CriticalPathComponent.getLongestPhaseSpawnRunnerSubtype
    pub fn get_longest_phase_spawn_runner_subtype(&self) -> Option<String> {
        self.state.lock().longest_phase_spawn_runner_subtype.clone()
    }

    /// Updates the child component if the union of the new dependency component runtime and the
    /// current component runtime is greater than the union of the current child runtime and
    /// current component runtime. The caller should ensure the dependency component is not
    /// running.
    ///
    /// PORT: the dependency's fields are read before this component's lock is taken (Java reads
    /// them without holding the dependency's monitor).
    // Java: com/google/devtools/build/lib/metrics/criticalpath/CriticalPathComponent.java:248 CriticalPathComponent.addDepInfo
    pub fn add_dep_info(
        self: &Arc<Self>,
        dep: &Arc<CriticalPathComponent>,
        component_finish_nanos: i64,
    ) {
        let (dep_aggregated_elapsed_time, dep_finish_nanos) = if Arc::ptr_eq(self, dep) {
            // PORT: Java's monitor is re-entrant; a self dependency reads its own fields.
            let st = self.state.lock();
            (st.aggregated_elapsed_time, st.finish_nanos)
        } else {
            let dep_state = dep.state.lock();
            (dep_state.aggregated_elapsed_time, dep_state.finish_nanos)
        };
        let mut st = self.state.lock();
        let current_elapsed_time = component_finish_nanos - st.start_nanos;
        let mut aggregated_elapsed_time = dep_aggregated_elapsed_time + current_elapsed_time;
        // This corrects the overlapping run time.
        if dep_finish_nanos > st.start_nanos {
            aggregated_elapsed_time -= dep_finish_nanos - st.start_nanos;
        }
        if st.child.is_none() || aggregated_elapsed_time > st.aggregated_elapsed_time {
            st.aggregated_elapsed_time = aggregated_elapsed_time;
            st.child = Some(dep.clone());
        }
    }

    // Java: com/google/devtools/build/lib/metrics/criticalpath/CriticalPathComponent.java:262 CriticalPathComponent.getStartTimeNanos
    pub fn get_start_time_nanos(&self) -> i64 {
        self.state.lock().start_nanos
    }

    /// PORT: `NanosToMillisSinceEpochConverter` is a function from a `nanoTime()` value to
    /// milliseconds since the epoch.
    // Java: com/google/devtools/build/lib/metrics/criticalpath/CriticalPathComponent.java:266 CriticalPathComponent.getStartTimeMillisSinceEpoch
    pub fn get_start_time_millis_since_epoch(&self, converter: &dyn Fn(i64) -> i64) -> i64 {
        converter(self.get_start_time_nanos())
    }

    /// `getElapsedTime()` in nanoseconds (`Duration.ofNanos(getElapsedTimeNanos())`).
    // Java: com/google/devtools/build/lib/metrics/criticalpath/CriticalPathComponent.java:270 CriticalPathComponent.getElapsedTime
    pub fn get_elapsed_time(&self) -> i64 {
        self.get_elapsed_time_nanos()
    }

    // Java: com/google/devtools/build/lib/metrics/criticalpath/CriticalPathComponent.java:274 CriticalPathComponent.getElapsedTimeNanos
    pub fn get_elapsed_time_nanos(&self) -> i64 {
        let st = self.state.lock();
        self.get_elapsed_time_nanos_locked(&st)
    }

    fn get_elapsed_time_nanos_locked(&self, st: &State) -> i64 {
        if self.is_running.load(Ordering::SeqCst) {
            // It can happen that we're being asked to compute a critical path even though the
            // build was interrupted. In that case, we may not have gotten an action completion
            // event. We don't have access to the clock from here, so we have to return 0.
            // Note that the critical path never includes interrupted actions, so
            // getAggregatedElapsedTime does not get called in this state.
            // If we want the critical path to contain partially executed actions in a case of
            // interrupt, then we need to tell the critical path computer that the build was
            // interrupt, and let it artificially mark all such actions as done.
            return 0;
        }
        get_elapsed_time_nanos_no_check(st)
    }

    /// Returns the current critical path for the action, in nanoseconds.
    ///
    /// Critical path is defined as : action_execution_time + max(child_critical_path).
    // Java: com/google/devtools/build/lib/metrics/criticalpath/CriticalPathComponent.java:306 CriticalPathComponent.getAggregatedElapsedTime
    pub fn get_aggregated_elapsed_time(&self) -> i64 {
        self.state.lock().aggregated_elapsed_time
    }

    /// Get the child critical path component.
    ///
    /// The component dependency with the maximum total critical path time.
    // Java: com/google/devtools/build/lib/metrics/criticalpath/CriticalPathComponent.java:316 CriticalPathComponent.getChild
    pub fn get_child(&self) -> Option<Arc<CriticalPathComponent>> {
        self.state.lock().child.clone()
    }

    /// Returns a string representation of the action. Only for use in crash messages and the
    /// like.
    // Java: com/google/devtools/build/lib/metrics/criticalpath/CriticalPathComponent.java:321 CriticalPathComponent.getActionString
    fn get_action_string(&self) -> String {
        self.action.pretty_print()
    }
}

/// To be used only in debugging: skips state invariance checks to avoid crash-looping.
// Java: com/google/devtools/build/lib/metrics/criticalpath/CriticalPathComponent.java:296 CriticalPathComponent.getElapsedTimeNanosNoCheck
fn get_elapsed_time_nanos_no_check(st: &State) -> i64 {
    // The delta value may be negative, see note in {@link Clock#nanoTime}.
    0.max(st.finish_nanos - st.start_nanos)
}

/// Returns a user readable representation of the critical path stats with all the details.
// Java: com/google/devtools/build/lib/metrics/criticalpath/CriticalPathComponent.java:326 CriticalPathComponent.toString
impl std::fmt::Display for CriticalPathComponent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let (elapsed_no_check, remote) = {
            let st = self.state.lock();
            (get_elapsed_time_nanos_no_check(&st), st.remote)
        };
        let mut sb = String::new();
        let mut current_time = String::from("still running");
        if !self.is_running() {
            current_time = format!(
                "{}s",
                java_format_fixed(nanos_to_millis(elapsed_no_check) as f64 / 1000.0, 2)
            );
        }
        sb.push_str(&current_time);
        if remote {
            sb.push_str(", ");
            sb.push_str(
                &self
                    .get_spawn_metrics()
                    .to_string_with_total(nanos_to_millis(elapsed_no_check), false),
            );
        }
        sb.push(' ');
        sb.push_str(&self.get_action_string());
        f.write_str(&sb)
    }
}
