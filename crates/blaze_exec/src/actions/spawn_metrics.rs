//! Port of `lib/actions/SpawnMetrics.java`.

use std::collections::BTreeMap;
use std::fmt;

/// `SpawnMetrics.ExecKind`.
// Java: com/google/devtools/build/lib/actions/SpawnMetrics.java:52 SpawnMetrics.ExecKind
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum ExecKind {
    Remote,
    #[default]
    Local,
    Worker,
    Other,
}

impl fmt::Display for ExecKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            ExecKind::Remote => "Remote",
            ExecKind::Local => "Local",
            ExecKind::Worker => "Worker",
            ExecKind::Other => "Other",
        })
    }
}

/// Timing, size and memory statistics for a spawn execution. Fields are the Java builder's
/// fields; build with struct-update syntax from [`SpawnMetrics::for_exec`].
// Java: com/google/devtools/build/lib/actions/SpawnMetrics.java:31 SpawnMetrics
#[derive(Clone, Debug, PartialEq, Default)]
pub struct SpawnMetrics {
    pub exec_kind: ExecKind,
    pub total_time_in_ms: i32,
    pub parse_time_in_ms: i32,
    pub fetch_time_in_ms: i32,
    pub queue_time_in_ms: i32,
    pub upload_time_in_ms: i32,
    pub setup_time_in_ms: i32,
    pub execution_wall_time_in_ms: i32,
    pub process_outputs_time_in_ms: i32,
    pub network_time_in_ms: i32,
    /// `retryTimeInMs`: error code to time (Java `ImmutableMap<Integer, Integer>`).
    pub retry_time_in_ms: BTreeMap<i32, i32>,
    pub input_bytes: i64,
    pub input_files: i64,
    pub memory_estimate_bytes: i64,
    pub measured_memory_peak_bytes: i64,
    pub input_bytes_limit: i64,
    pub input_files_limit: i64,
    pub output_bytes_limit: i64,
    pub output_files_limit: i64,
    pub memory_bytes_limit: i64,
    pub time_limit_in_ms: i32,
}

const STATS_SHOW_THRESHOLD: f64 = 0.10;

impl SpawnMetrics {
    /// `Builder.forExec(kind).build()`: empty metrics of a kind.
    pub fn for_exec(exec_kind: ExecKind) -> SpawnMetrics {
        SpawnMetrics {
            exec_kind,
            ..SpawnMetrics::default()
        }
    }

    /// `forLocalExecution(wallTimeInMs)`.
    // Java: com/google/devtools/build/lib/actions/SpawnMetrics.java:94 SpawnMetrics.forLocalExecution
    pub fn for_local_execution(wall_time_in_ms: i32) -> SpawnMetrics {
        SpawnMetrics {
            exec_kind: ExecKind::Local,
            total_time_in_ms: wall_time_in_ms,
            execution_wall_time_in_ms: wall_time_in_ms,
            ..SpawnMetrics::default()
        }
    }

    /// `execKind()`.
    pub fn exec_kind(&self) -> ExecKind {
        self.exec_kind
    }
    /// `isEmpty()`.
    pub fn is_empty(&self) -> bool {
        self.total_time_in_ms == 0
    }
    /// `retryTimeInMs()`: the sum over error codes.
    pub fn retry_time_in_ms(&self) -> i32 {
        self.retry_time_in_ms
            .values()
            .copied()
            .fold(0, i32::wrapping_add)
    }
    /// `otherTimeInMs()`.
    pub fn other_time_in_ms(&self) -> i32 {
        self.total_time_in_ms
            - self.parse_time_in_ms
            - self.network_time_in_ms
            - self.queue_time_in_ms
            - self.upload_time_in_ms
            - self.setup_time_in_ms
            - self.execution_wall_time_in_ms
            - self.fetch_time_in_ms
            - self.retry_time_in_ms()
            - self.process_outputs_time_in_ms
    }

    /// `toString(int total, boolean summary)`.
    // Java: com/google/devtools/build/lib/actions/SpawnMetrics.java:107 SpawnMetrics.toString
    pub fn to_string_with_total(&self, total: i32, summary: bool) -> String {
        let mut sb = String::from("(");
        sb.push_str(&pretty_percentage(self.total_time_in_ms, total));
        sb.push_str(" of the time): [");
        let mut stats = Vec::new();
        add_stat(&mut stats, "parse", !summary, self.parse_time_in_ms, total);
        add_stat(&mut stats, "queue", true, self.queue_time_in_ms, total);
        add_stat(
            &mut stats,
            "network",
            !summary,
            self.network_time_in_ms,
            total,
        );
        add_stat(
            &mut stats,
            "upload",
            !summary,
            self.upload_time_in_ms,
            total,
        );
        add_stat(&mut stats, "setup", true, self.setup_time_in_ms, total);
        add_stat(
            &mut stats,
            "process",
            true,
            self.execution_wall_time_in_ms,
            total,
        );
        add_stat(&mut stats, "fetch", !summary, self.fetch_time_in_ms, total);
        add_stat(
            &mut stats,
            "retry",
            !summary,
            self.retry_time_in_ms(),
            total,
        );
        add_stat(
            &mut stats,
            "processOutputs",
            !summary,
            self.process_outputs_time_in_ms,
            total,
        );
        add_stat(
            &mut stats,
            "other",
            !summary,
            self.other_time_in_ms(),
            total,
        );
        if !summary {
            stats.push(format!("input files: {}", self.input_files));
            stats.push(format!("input bytes: {}", self.input_bytes));
            stats.push(format!("memory bytes: {}", self.memory_estimate_bytes));
        }
        sb.push_str(&stats.join(", "));
        sb.push(']');
        sb
    }
}

fn add_stat(strings: &mut Vec<String>, name: &str, force_show: bool, time: i32, total: i32) {
    if force_show || (total > 0 && ((time as f32 / total as f32) as f64) >= STATS_SHOW_THRESHOLD) {
        strings.push(format!("{}: {}", name, pretty_percentage(time, total)));
    }
}

fn pretty_percentage(duration: i32, total: i32) -> String {
    if total == 0 {
        return "N/A".to_string();
    }
    format!("{:.2}%", duration as f64 * 100.0 / total as f64)
}
