//! Port of `lib/metrics/criticalpath/AggregatedCriticalPath.java`.

use std::sync::Arc;

use blaze_events::java_format::java_format_fixed;

use super::critical_path_component::{CriticalPathComponent, nanos_to_millis};
use crate::metrics::aggregated_spawn_metrics::AggregatedSpawnMetrics;

/// Aggregates all the critical path components in one object. This allows us to easily access
/// the components data and have a proper toString().
// Java: com/google/devtools/build/lib/metrics/criticalpath/AggregatedCriticalPath.java:28 AggregatedCriticalPath
#[derive(Clone, Debug)]
pub struct AggregatedCriticalPath {
    /// `totalTime`, in nanoseconds.
    total_time: i64,
    aggregated_spawn_metrics: AggregatedSpawnMetrics,
    critical_path_components: Vec<Arc<CriticalPathComponent>>,
}

impl AggregatedCriticalPath {
    /// `AggregatedCriticalPath.EMPTY`.
    // Java: com/google/devtools/build/lib/metrics/criticalpath/AggregatedCriticalPath.java:30 AggregatedCriticalPath.EMPTY
    pub fn empty() -> AggregatedCriticalPath {
        AggregatedCriticalPath::new(0, AggregatedSpawnMetrics::empty(), Vec::new())
    }

    /// Java's `criticalPath != AggregatedCriticalPath.EMPTY` (reference comparison).
    ///
    /// PORT: the computer returns `EMPTY` exactly when no component finished; an aggregate made
    /// from components always has at least one.
    pub fn is_empty_instance(&self) -> bool {
        self.critical_path_components.is_empty()
            && self.total_time == 0
            && self.aggregated_spawn_metrics == AggregatedSpawnMetrics::empty()
    }

    // Java: com/google/devtools/build/lib/metrics/criticalpath/AggregatedCriticalPath.java:37 AggregatedCriticalPath.<init>
    pub fn new(
        total_time: i64,
        aggregated_spawn_metrics: AggregatedSpawnMetrics,
        critical_path_components: Vec<Arc<CriticalPathComponent>>,
    ) -> AggregatedCriticalPath {
        AggregatedCriticalPath {
            total_time,
            aggregated_spawn_metrics,
            critical_path_components,
        }
    }

    /// Total wall time spent running the critical path actions, in nanoseconds.
    // Java: com/google/devtools/build/lib/metrics/criticalpath/AggregatedCriticalPath.java:47 AggregatedCriticalPath.getAggregatedElapsedTime
    pub fn get_aggregated_elapsed_time(&self) -> i64 {
        self.total_time
    }

    // Java: com/google/devtools/build/lib/metrics/criticalpath/AggregatedCriticalPath.java:51 AggregatedCriticalPath.getSpawnMetrics
    pub fn get_spawn_metrics(&self) -> &AggregatedSpawnMetrics {
        &self.aggregated_spawn_metrics
    }

    /// Returns a list of all the component stats for the critical path.
    // Java: com/google/devtools/build/lib/metrics/criticalpath/AggregatedCriticalPath.java:56 AggregatedCriticalPath.components
    pub fn components(&self) -> &[Arc<CriticalPathComponent>] {
        &self.critical_path_components
    }

    // Java: com/google/devtools/build/lib/metrics/criticalpath/AggregatedCriticalPath.java:60 AggregatedCriticalPath.getNewStringSummary
    pub fn get_new_string_summary(&self) -> String {
        let execution_wall_time_in_ms = self
            .aggregated_spawn_metrics
            .get_total_duration(|m| m.execution_wall_time_in_ms);
        let overhead_time_in_ms = self
            .aggregated_spawn_metrics
            .get_total_duration(|m| m.total_time_in_ms)
            .wrapping_sub(execution_wall_time_in_ms);
        format!(
            "Execution critical path {}s (setup {}s, action wall time {}s)",
            java_format_fixed(nanos_to_millis(self.total_time) as f64 / 1000.0, 2),
            java_format_fixed(overhead_time_in_ms as f64 / 1000.0, 2),
            java_format_fixed(execution_wall_time_in_ms as f64 / 1000.0, 2)
        )
    }

    // Java: com/google/devtools/build/lib/metrics/criticalpath/AggregatedCriticalPath.java:79 AggregatedCriticalPath.toString
    fn to_string_with(&self, summary: bool, remote: bool) -> String {
        let mut sb = String::from("Critical Path: ");
        sb.push_str(&java_format_fixed(
            nanos_to_millis(self.total_time) as f64 / 1000.0,
            2,
        ));
        sb.push('s');
        if remote {
            sb.push_str(", ");
            sb.push_str(
                &self
                    .get_spawn_metrics()
                    .to_string_with_total(nanos_to_millis(self.total_time), summary),
            );
        }
        if summary || self.critical_path_components.is_empty() {
            return sb;
        }
        sb.push_str("\n  ");
        let components: Vec<String> = self
            .critical_path_components
            .iter()
            .map(|c| c.to_string())
            .collect();
        sb.push_str(&components.join("\n  "));
        sb
    }

    /// Returns a summary version of the critical path stats that omits stats that are not useful
    /// to the user.
    // Java: com/google/devtools/build/lib/metrics/criticalpath/AggregatedCriticalPath.java:99 AggregatedCriticalPath.toStringSummary
    pub fn to_string_summary(&self) -> String {
        self.to_string_with(true, true)
    }

    /// Same as toStringSummary but also omits remote stats. This is to be used in Bazel because
    /// currently the Remote stats are not calculated correctly.
    // Java: com/google/devtools/build/lib/metrics/criticalpath/AggregatedCriticalPath.java:107 AggregatedCriticalPath.toStringSummaryNoRemote
    pub fn to_string_summary_no_remote(&self) -> String {
        self.to_string_with(true, false)
    }
}

// Java: com/google/devtools/build/lib/metrics/criticalpath/AggregatedCriticalPath.java:74 AggregatedCriticalPath.toString
impl std::fmt::Display for AggregatedCriticalPath {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.to_string_with(false, true))
    }
}
