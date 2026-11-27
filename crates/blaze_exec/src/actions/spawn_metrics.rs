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
    // Java: com/google/devtools/build/lib/actions/SpawnMetrics.java:69 SpawnMetrics.ExecKind.toString
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
// The accessors, `toBuilder` (`clone`) and the builder's setters are the public fields: a
// `set*(Duration)` setter is the caller's `to_ms` (`Builder.toMs`) and an `add*` setter the
// caller's `+=` (`addRetryTimeInMs` merges into `retry_time_in_ms` by error code). The limits
// are zero unless set, as `build()` returning a plain `SpawnMetrics` (no `FullSpawnMetrics`)
// answers.
// Java: com/google/devtools/build/lib/actions/SpawnMetrics.java:176 SpawnMetrics.totalTimeInMs
// Java: com/google/devtools/build/lib/actions/SpawnMetrics.java:185 SpawnMetrics.networkTimeInMs
// Java: com/google/devtools/build/lib/actions/SpawnMetrics.java:190 SpawnMetrics.queueTimeInMs
// Java: com/google/devtools/build/lib/actions/SpawnMetrics.java:198 SpawnMetrics.uploadTimeInMs
// Java: com/google/devtools/build/lib/actions/SpawnMetrics.java:207 SpawnMetrics.setupTimeInMs
// Java: com/google/devtools/build/lib/actions/SpawnMetrics.java:212 SpawnMetrics.executionWallTimeInMs
// Java: com/google/devtools/build/lib/actions/SpawnMetrics.java:220 SpawnMetrics.parseTimeInMs
// Java: com/google/devtools/build/lib/actions/SpawnMetrics.java:225 SpawnMetrics.fetchTimeInMs
// Java: com/google/devtools/build/lib/actions/SpawnMetrics.java:235 SpawnMetrics.retryTimeByError
// Java: com/google/devtools/build/lib/actions/SpawnMetrics.java:240 SpawnMetrics.processOutputsTimeInMs
// Java: com/google/devtools/build/lib/actions/SpawnMetrics.java:262 SpawnMetrics.inputBytes
// Java: com/google/devtools/build/lib/actions/SpawnMetrics.java:267 SpawnMetrics.inputFiles
// Java: com/google/devtools/build/lib/actions/SpawnMetrics.java:272 SpawnMetrics.memoryEstimate
// Java: com/google/devtools/build/lib/actions/SpawnMetrics.java:277 SpawnMetrics.measuredMemoryPeak
// Java: com/google/devtools/build/lib/actions/SpawnMetrics.java:281 SpawnMetrics.toBuilder
// Java: com/google/devtools/build/lib/actions/SpawnMetrics.java:286 SpawnMetrics.inputBytesLimit
// Java: com/google/devtools/build/lib/actions/SpawnMetrics.java:291 SpawnMetrics.inputFilesLimit
// Java: com/google/devtools/build/lib/actions/SpawnMetrics.java:296 SpawnMetrics.outputBytesLimit
// Java: com/google/devtools/build/lib/actions/SpawnMetrics.java:301 SpawnMetrics.outputFilesLimit
// Java: com/google/devtools/build/lib/actions/SpawnMetrics.java:306 SpawnMetrics.memoryLimit
// Java: com/google/devtools/build/lib/actions/SpawnMetrics.java:311 SpawnMetrics.timeLimitInMs
// FullSpawnMetrics (the builder's build() when a limit is set) is the same struct with its limits.
// Java: com/google/devtools/build/lib/actions/FullSpawnMetrics.java:28 FullSpawnMetrics.<init>
// Java: com/google/devtools/build/lib/actions/FullSpawnMetrics.java:40 FullSpawnMetrics.inputBytesLimit
// Java: com/google/devtools/build/lib/actions/FullSpawnMetrics.java:46 FullSpawnMetrics.inputFilesLimit
// Java: com/google/devtools/build/lib/actions/FullSpawnMetrics.java:52 FullSpawnMetrics.outputBytesLimit
// Java: com/google/devtools/build/lib/actions/FullSpawnMetrics.java:58 FullSpawnMetrics.outputFilesLimit
// Java: com/google/devtools/build/lib/actions/FullSpawnMetrics.java:64 FullSpawnMetrics.memoryLimit
// Java: com/google/devtools/build/lib/actions/FullSpawnMetrics.java:70 FullSpawnMetrics.timeLimitInMs
// Java: com/google/devtools/build/lib/actions/SpawnMetrics.java:378 SpawnMetrics.Builder.setExecKind
// Java: com/google/devtools/build/lib/actions/SpawnMetrics.java:384 SpawnMetrics.Builder.setTotalTime
// Java: com/google/devtools/build/lib/actions/SpawnMetrics.java:389 SpawnMetrics.Builder.setTotalTimeInMs
// Java: com/google/devtools/build/lib/actions/SpawnMetrics.java:395 SpawnMetrics.Builder.setParseTime
// Java: com/google/devtools/build/lib/actions/SpawnMetrics.java:400 SpawnMetrics.Builder.setParseTimeInMs
// Java: com/google/devtools/build/lib/actions/SpawnMetrics.java:406 SpawnMetrics.Builder.setNetworkTime
// Java: com/google/devtools/build/lib/actions/SpawnMetrics.java:411 SpawnMetrics.Builder.setNetworkTimeInMs
// Java: com/google/devtools/build/lib/actions/SpawnMetrics.java:417 SpawnMetrics.Builder.setFetchTime
// Java: com/google/devtools/build/lib/actions/SpawnMetrics.java:422 SpawnMetrics.Builder.setFetchTimeInMs
// Java: com/google/devtools/build/lib/actions/SpawnMetrics.java:428 SpawnMetrics.Builder.setQueueTime
// Java: com/google/devtools/build/lib/actions/SpawnMetrics.java:433 SpawnMetrics.Builder.setQueueTimeInMs
// Java: com/google/devtools/build/lib/actions/SpawnMetrics.java:439 SpawnMetrics.Builder.addQueueTime
// Java: com/google/devtools/build/lib/actions/SpawnMetrics.java:444 SpawnMetrics.Builder.addQueueTimeInMs
// Java: com/google/devtools/build/lib/actions/SpawnMetrics.java:450 SpawnMetrics.Builder.setSetupTime
// Java: com/google/devtools/build/lib/actions/SpawnMetrics.java:455 SpawnMetrics.Builder.setSetupTimeInMs
// Java: com/google/devtools/build/lib/actions/SpawnMetrics.java:461 SpawnMetrics.Builder.addSetupTime
// Java: com/google/devtools/build/lib/actions/SpawnMetrics.java:466 SpawnMetrics.Builder.addSetupTimeInMs
// Java: com/google/devtools/build/lib/actions/SpawnMetrics.java:472 SpawnMetrics.Builder.setUploadTime
// Java: com/google/devtools/build/lib/actions/SpawnMetrics.java:477 SpawnMetrics.Builder.setUploadTimeInMs
// Java: com/google/devtools/build/lib/actions/SpawnMetrics.java:483 SpawnMetrics.Builder.setExecutionWallTime
// Java: com/google/devtools/build/lib/actions/SpawnMetrics.java:488 SpawnMetrics.Builder.setExecutionWallTimeInMs
// Java: com/google/devtools/build/lib/actions/SpawnMetrics.java:494 SpawnMetrics.Builder.addRetryTime
// Java: com/google/devtools/build/lib/actions/SpawnMetrics.java:499 SpawnMetrics.Builder.addRetryTimeInMs
// Java: com/google/devtools/build/lib/actions/SpawnMetrics.java:505 SpawnMetrics.Builder.setRetryTimeInMs
// Java: com/google/devtools/build/lib/actions/SpawnMetrics.java:511 SpawnMetrics.Builder.setProcessOutputsTime
// Java: com/google/devtools/build/lib/actions/SpawnMetrics.java:516 SpawnMetrics.Builder.setProcessOutputsTimeInMs
// Java: com/google/devtools/build/lib/actions/SpawnMetrics.java:522 SpawnMetrics.Builder.setInputBytes
// Java: com/google/devtools/build/lib/actions/SpawnMetrics.java:528 SpawnMetrics.Builder.setInputFiles
// Java: com/google/devtools/build/lib/actions/SpawnMetrics.java:534 SpawnMetrics.Builder.setMemoryEstimateBytes
// Java: com/google/devtools/build/lib/actions/SpawnMetrics.java:540 SpawnMetrics.Builder.setMeasuredMemoryPeakBytes
// Java: com/google/devtools/build/lib/actions/SpawnMetrics.java:546 SpawnMetrics.Builder.setInputBytesLimit
// Java: com/google/devtools/build/lib/actions/SpawnMetrics.java:552 SpawnMetrics.Builder.setInputFilesLimit
// Java: com/google/devtools/build/lib/actions/SpawnMetrics.java:558 SpawnMetrics.Builder.setOutputBytesLimit
// Java: com/google/devtools/build/lib/actions/SpawnMetrics.java:564 SpawnMetrics.Builder.setOutputFilesLimit
// Java: com/google/devtools/build/lib/actions/SpawnMetrics.java:570 SpawnMetrics.Builder.setMemoryBytesLimit
// Java: com/google/devtools/build/lib/actions/SpawnMetrics.java:576 SpawnMetrics.Builder.setTimeLimitInMs
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
    // Java: com/google/devtools/build/lib/actions/SpawnMetrics.java:355 SpawnMetrics.Builder.forExec
    // Java: com/google/devtools/build/lib/actions/SpawnMetrics.java:339 SpawnMetrics.Builder.forLocalExec
    // Java: com/google/devtools/build/lib/actions/SpawnMetrics.java:343 SpawnMetrics.Builder.forRemoteExec
    // Java: com/google/devtools/build/lib/actions/SpawnMetrics.java:347 SpawnMetrics.Builder.forWorkerExec
    // Java: com/google/devtools/build/lib/actions/SpawnMetrics.java:351 SpawnMetrics.Builder.forOtherExec
    // Java: com/google/devtools/build/lib/actions/SpawnMetrics.java:361 SpawnMetrics.Builder.Builder
    // Java: com/google/devtools/build/lib/actions/SpawnMetrics.java:363 SpawnMetrics.Builder.build
    // Java: com/google/devtools/build/lib/actions/SpawnMetrics.java:378 SpawnMetrics.Builder.setExecKind
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
    // Java: com/google/devtools/build/lib/actions/SpawnMetrics.java:163 SpawnMetrics.execKind
    pub fn exec_kind(&self) -> ExecKind {
        self.exec_kind
    }
    /// `isEmpty()`.
    // Java: com/google/devtools/build/lib/actions/SpawnMetrics.java:168 SpawnMetrics.isEmpty
    pub fn is_empty(&self) -> bool {
        self.total_time_in_ms == 0
    }
    /// `retryTimeInMs()`: the sum over error codes.
    // Java: com/google/devtools/build/lib/actions/SpawnMetrics.java:230 SpawnMetrics.retryTimeInMs
    pub fn retry_time_in_ms(&self) -> i32 {
        self.retry_time_in_ms
            .values()
            .copied()
            .fold(0, i32::wrapping_add)
    }
    /// `otherTimeInMs()`.
    // Java: com/google/devtools/build/lib/actions/SpawnMetrics.java:248 SpawnMetrics.otherTimeInMs
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

// Java: com/google/devtools/build/lib/actions/SpawnMetrics.java:137 SpawnMetrics.addStatToString
// Java: com/google/devtools/build/lib/actions/SpawnMetrics.java:144 SpawnMetrics.isAboveThreshold
fn add_stat(strings: &mut Vec<String>, name: &str, force_show: bool, time: i32, total: i32) {
    if force_show || (total > 0 && ((time as f32 / total as f32) as f64) >= STATS_SHOW_THRESHOLD) {
        strings.push(format!("{}: {}", name, pretty_percentage(time, total)));
    }
}

// Java: com/google/devtools/build/lib/actions/SpawnMetrics.java:153 SpawnMetrics.prettyPercentage
fn pretty_percentage(duration: i32, total: i32) -> String {
    if total == 0 {
        return "N/A".to_string();
    }
    format!("{:.2}%", duration as f64 * 100.0 / total as f64)
}
