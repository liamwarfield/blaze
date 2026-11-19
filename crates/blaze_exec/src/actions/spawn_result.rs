//! Port of `lib/actions/SpawnResult.java` (`SimpleSpawnResult` and its `Builder`).

use std::fmt;
use std::time::SystemTime;

use blaze_protos::failure_details::FailureDetail;
use blaze_protos::tools::protos::Digest;
use blaze_shell::{ExecutionStatistics, TerminationStatus};
use blaze_vfs::Path;

use super::action_input::ActionInputRef;
use super::spawn_metrics::SpawnMetrics;

/// `SpawnResult.POSIX_TIMEOUT_EXIT_CODE`: SIGNAL_BASE (128) + SIGALRM (14).
// Java: com/google/devtools/build/lib/actions/SpawnResult.java:40 SpawnResult.POSIX_TIMEOUT_EXIT_CODE
pub const POSIX_TIMEOUT_EXIT_CODE: i32 = 128 + 14;

/// `SpawnResult.Status`.
// Java: com/google/devtools/build/lib/actions/SpawnResult.java:43 SpawnResult.Status
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Status {
    Success,
    NonZeroExit,
    Timeout,
    OutOfMemory,
    ExecutionFailed,
    ExecutionFailedCatastrophically,
    ExecutionDenied,
    ExecutionDeniedCatastrophically,
    RemoteCacheFailed,
}

impl Status {
    /// `isConsideredUserError()`.
    pub fn is_considered_user_error(self) -> bool {
        matches!(
            self,
            Status::NonZeroExit
                | Status::Timeout
                | Status::OutOfMemory
                | Status::ExecutionDenied
                | Status::ExecutionDeniedCatastrophically
        )
    }

    /// `Enum.name()` / `toString()`.
    pub fn name(self) -> &'static str {
        match self {
            Status::Success => "SUCCESS",
            Status::NonZeroExit => "NON_ZERO_EXIT",
            Status::Timeout => "TIMEOUT",
            Status::OutOfMemory => "OUT_OF_MEMORY",
            Status::ExecutionFailed => "EXECUTION_FAILED",
            Status::ExecutionFailedCatastrophically => "EXECUTION_FAILED_CATASTROPHICALLY",
            Status::ExecutionDenied => "EXECUTION_DENIED",
            Status::ExecutionDeniedCatastrophically => "EXECUTION_DENIED_CATASTROPHICALLY",
            Status::RemoteCacheFailed => "REMOTE_CACHE_FAILED",
        }
    }
}

impl fmt::Display for Status {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// The result of a spawn execution (`SpawnResult.SimpleSpawnResult`).
// Java: com/google/devtools/build/lib/actions/SpawnResult.java:286 SpawnResult.SimpleSpawnResult
#[derive(Clone, Debug)]
pub struct SpawnResult {
    exit_code: i32,
    status: Status,
    failure_detail: Option<FailureDetail>,
    executor_host_name: Option<String>,
    runner_name: String,
    runner_subtype: String,
    spawn_metrics: SpawnMetrics,
    start_time: Option<SystemTime>,
    wall_time_in_ms: i32,
    user_time_in_ms: i32,
    system_time_in_ms: i32,
    num_block_output_operations: Option<i64>,
    num_block_input_operations: Option<i64>,
    num_involuntary_context_switches: Option<i64>,
    memory_kb: Option<i64>,
    cache_hit: bool,
    failure_message: String,
    in_memory_output_file: Option<ActionInputRef>,
    in_memory_contents: Option<Vec<u8>>,
    /// Set by [`SpawnResult::delegate_with_in_memory_output`]: `getInMemoryOutput(output)` then
    /// compares `output.getExecPathString()` with this path instead of the output.
    in_memory_output_exec_path: Option<String>,
    remote: bool,
    digest: Option<Digest>,
}

impl SpawnResult {
    /// `setupSuccess()`.
    pub fn setup_success(&self) -> bool {
        matches!(
            self.status,
            Status::Success | Status::NonZeroExit | Status::Timeout | Status::OutOfMemory
        )
    }
    /// `isCatastrophe()`.
    pub fn is_catastrophe(&self) -> bool {
        matches!(
            self.status,
            Status::ExecutionFailedCatastrophically | Status::ExecutionDeniedCatastrophically
        )
    }
    /// `status()`.
    pub fn status(&self) -> Status {
        self.status
    }
    /// `exitCode()`.
    pub fn exit_code(&self) -> i32 {
        self.exit_code
    }
    /// `failureDetail()`.
    pub fn failure_detail(&self) -> Option<&FailureDetail> {
        self.failure_detail.as_ref()
    }
    /// `getExecutorHostName()`.
    pub fn get_executor_host_name(&self) -> Option<&str> {
        self.executor_host_name.as_deref()
    }
    /// `getRunnerName()`.
    pub fn get_runner_name(&self) -> &str {
        &self.runner_name
    }
    /// `getRunnerSubtype()`.
    pub fn get_runner_subtype(&self) -> &str {
        &self.runner_subtype
    }
    /// `getMetrics()`.
    pub fn get_metrics(&self) -> &SpawnMetrics {
        &self.spawn_metrics
    }
    /// `getStartTime()`.
    pub fn get_start_time(&self) -> Option<SystemTime> {
        self.start_time
    }
    /// `getWallTimeInMs()`.
    pub fn get_wall_time_in_ms(&self) -> i32 {
        self.wall_time_in_ms
    }
    /// `getUserTimeInMs()`.
    pub fn get_user_time_in_ms(&self) -> i32 {
        self.user_time_in_ms
    }
    /// `getSystemTimeInMs()`.
    pub fn get_system_time_in_ms(&self) -> i32 {
        self.system_time_in_ms
    }
    /// `getNumBlockOutputOperations()`.
    pub fn get_num_block_output_operations(&self) -> Option<i64> {
        self.num_block_output_operations
    }
    /// `getNumBlockInputOperations()`.
    pub fn get_num_block_input_operations(&self) -> Option<i64> {
        self.num_block_input_operations
    }
    /// `getNumInvoluntaryContextSwitches()`.
    pub fn get_num_involuntary_context_switches(&self) -> Option<i64> {
        self.num_involuntary_context_switches
    }
    /// `getMemoryInKb()`.
    pub fn get_memory_in_kb(&self) -> Option<i64> {
        self.memory_kb
    }
    /// `isCacheHit()`.
    pub fn is_cache_hit(&self) -> bool {
        self.cache_hit
    }
    /// `getFailureMessage()`.
    pub fn get_failure_message(&self) -> &str {
        &self.failure_message
    }
    /// `getInMemoryOutput(ActionInput)`.
    pub fn get_in_memory_output(&self, output: &ActionInputRef) -> Option<&[u8]> {
        if let Some(exec_path) = &self.in_memory_output_exec_path {
            return if output.get_exec_path_string() == *exec_path {
                self.in_memory_contents.as_deref()
            } else {
                None
            };
        }
        match &self.in_memory_output_file {
            Some(f) if f == output => self.in_memory_contents.as_deref(),
            _ => None,
        }
    }

    /// `new SpawnResult.DelegateSpawnResult(this) { getRunnerName() { return runnerName; }
    /// getMetrics() { return metrics; } }`: this result with another runner name and metrics.
    ///
    /// PORT: `SpawnResult` is immutable, so a copy with the two overridden getters behaves as
    /// the delegate does.
    // Java: com/google/devtools/build/lib/actions/SpawnResult.java:482 SpawnResult.DelegateSpawnResult
    pub fn delegate_with_runner_name_and_metrics(
        &self,
        runner_name: &str,
        metrics: SpawnMetrics,
    ) -> SpawnResult {
        let mut delegate = self.clone();
        delegate.runner_name = runner_name.to_string();
        delegate.spawn_metrics = metrics;
        delegate
    }

    /// `new SpawnResult.DelegateSpawnResult(this) { getInMemoryOutput(output) { return
    /// output.getExecPathString().equals(execPath) ? content : null; } }`.
    ///
    /// PORT: a copy with the overridden getter (see
    /// [`SpawnResult::delegate_with_runner_name_and_metrics`]).
    // Java: com/google/devtools/build/lib/actions/SpawnResult.java:482 SpawnResult.DelegateSpawnResult
    pub fn delegate_with_in_memory_output(
        &self,
        exec_path: String,
        content: Option<Vec<u8>>,
    ) -> SpawnResult {
        let mut delegate = self.clone();
        delegate.in_memory_output_exec_path = Some(exec_path);
        delegate.in_memory_contents = content;
        delegate
    }
    /// `wasRemote()`.
    pub fn was_remote(&self) -> bool {
        self.remote
    }
    /// `getDigest()`.
    pub fn get_digest(&self) -> Option<&Digest> {
        self.digest.as_ref()
    }

    /// `getDetailMessage(message, catastrophe, forciblyRunRemotely)`: e.g. `"(Exit 1): <msg>"`.
    // Java: com/google/devtools/build/lib/actions/SpawnResult.java:446 SimpleSpawnResult.getDetailMessage
    pub fn get_detail_message(
        &self,
        message: &str,
        _catastrophe: bool,
        forcibly_run_remotely: bool,
    ) -> String {
        let status = TerminationStatus::new(self.exit_code, self.status == Status::Timeout);
        let reason = format!("({})", status.to_short_string()); // e.g. "(Exit 1)"
        let mut explanation = if message.is_empty() {
            String::new()
        } else {
            format!(": {message}")
        };
        if self.status == Status::Timeout {
            if self.wall_time_in_ms != 0 {
                explanation.push_str(&format!(
                    " (failed due to timeout after {:.2} seconds.)",
                    self.wall_time_in_ms as f64 / 1000.0
                ));
            } else {
                explanation.push_str(" (failed due to timeout.)");
            }
        } else if self.status == Status::OutOfMemory {
            explanation.push_str(" (Remote action was terminated due to Out of Memory.)");
        }
        if self.status != Status::Timeout && forcibly_run_remotely {
            explanation.push_str(
                " Action tagged as local was forcibly run remotely and failed - it's possible that the action simply doesn't work remotely",
            );
        }
        reason + &explanation
    }
}

/// `SpawnResult.Builder`.
// Java: com/google/devtools/build/lib/actions/SpawnResult.java:616 SpawnResult.Builder
#[derive(Clone, Debug, Default)]
pub struct SpawnResultBuilder {
    exit_code: i32,
    status: Option<Status>,
    failure_detail: Option<FailureDetail>,
    executor_host_name: Option<String>,
    runner_name: String,
    runner_subtype: String,
    spawn_metrics: Option<SpawnMetrics>,
    start_time: Option<SystemTime>,
    wall_time_in_ms: i32,
    user_time_in_ms: i32,
    system_time_in_ms: i32,
    num_block_output_operations: Option<i64>,
    num_block_input_operations: Option<i64>,
    num_involuntary_context_switches: Option<i64>,
    memory_in_kb: Option<i64>,
    cache_hit: bool,
    failure_message: String,
    in_memory_output_file: Option<ActionInputRef>,
    in_memory_contents: Option<Vec<u8>>,
    remote: bool,
    digest: Option<Digest>,
}

impl SpawnResultBuilder {
    /// `new SpawnResult.Builder()`.
    pub fn new() -> SpawnResultBuilder {
        SpawnResultBuilder::default()
    }

    /// `build()`.
    // Java: com/google/devtools/build/lib/actions/SpawnResult.java:642 SpawnResult.Builder.build
    pub fn build(&self) -> SpawnResult {
        blaze_util::check_argument!(!self.runner_name.is_empty());
        let status = self.status.expect("NullPointerException: status");
        match status {
            Status::Success => {
                blaze_util::check_argument!(self.exit_code == 0, "{}", self.exit_code);
                blaze_util::check_argument!(
                    self.failure_detail.is_none(),
                    "{:?}",
                    self.failure_detail
                );
            }
            _ => {
                if status == Status::Timeout {
                    blaze_util::check_argument!(
                        self.exit_code == POSIX_TIMEOUT_EXIT_CODE,
                        "{}",
                        self.exit_code
                    );
                }
                blaze_util::check_argument!(
                    self.exit_code != 0,
                    "Failed spawn with status {} had exit code 0 ({} {:?})",
                    status,
                    self.failure_message,
                    self.failure_detail
                );
                blaze_util::check_argument!(
                    self.failure_detail.is_some(),
                    "Failed spawn with status {} and exit code {} had no failure detail ({})",
                    status,
                    self.exit_code,
                    self.failure_message
                );
                // PORT: the BugReport for a system error with a BUILD_FAILURE exit code is not
                // ported (it only logs).
            }
        }
        let mut metrics = self
            .spawn_metrics
            .clone()
            .unwrap_or_else(|| SpawnMetrics::for_local_execution(self.wall_time_in_ms));
        if let Some(kb) = self.memory_in_kb {
            metrics.measured_memory_peak_bytes = kb * 1024;
        }
        SpawnResult {
            exit_code: self.exit_code,
            status,
            failure_detail: self.failure_detail.clone(),
            executor_host_name: self.executor_host_name.clone(),
            runner_name: self.runner_name.clone(),
            runner_subtype: self.runner_subtype.clone(),
            spawn_metrics: metrics,
            start_time: self.start_time,
            wall_time_in_ms: self.wall_time_in_ms,
            user_time_in_ms: self.user_time_in_ms,
            system_time_in_ms: self.system_time_in_ms,
            num_block_output_operations: self.num_block_output_operations,
            num_block_input_operations: self.num_block_input_operations,
            num_involuntary_context_switches: self.num_involuntary_context_switches,
            memory_kb: self.memory_in_kb,
            cache_hit: self.cache_hit,
            failure_message: self.failure_message.clone(),
            in_memory_output_file: self.in_memory_output_file.clone(),
            in_memory_contents: self.in_memory_contents.clone(),
            in_memory_output_exec_path: None,
            remote: self.remote,
            digest: self.digest.clone(),
        }
    }

    pub fn set_exit_code(&mut self, exit_code: i32) -> &mut Self {
        self.exit_code = exit_code;
        self
    }
    pub fn set_status(&mut self, status: Status) -> &mut Self {
        self.status = Some(status);
        self
    }
    pub fn set_failure_detail(&mut self, failure_detail: FailureDetail) -> &mut Self {
        self.failure_detail = Some(failure_detail);
        self
    }
    pub fn set_executor_hostname(&mut self, name: String) -> &mut Self {
        self.executor_host_name = Some(name);
        self
    }
    pub fn set_runner_name(&mut self, runner_name: &str) -> &mut Self {
        self.runner_name = runner_name.to_string();
        self
    }
    pub fn set_runner_subtype(&mut self, runner_subtype: &str) -> &mut Self {
        self.runner_subtype = runner_subtype.to_string();
        self
    }
    pub fn set_spawn_metrics(&mut self, metrics: SpawnMetrics) -> &mut Self {
        self.spawn_metrics = Some(metrics);
        self
    }
    pub fn set_start_time(&mut self, start_time: SystemTime) -> &mut Self {
        self.start_time = Some(start_time);
        self
    }
    pub fn set_wall_time_in_ms(&mut self, ms: i32) -> &mut Self {
        self.wall_time_in_ms = ms;
        self
    }
    pub fn set_user_time_in_ms(&mut self, ms: i32) -> &mut Self {
        self.user_time_in_ms = ms;
        self
    }
    pub fn set_system_time_in_ms(&mut self, ms: i32) -> &mut Self {
        self.system_time_in_ms = ms;
        self
    }
    pub fn set_num_block_output_operations(&mut self, n: i64) -> &mut Self {
        self.num_block_output_operations = Some(n);
        self
    }
    pub fn set_num_block_input_operations(&mut self, n: i64) -> &mut Self {
        self.num_block_input_operations = Some(n);
        self
    }
    pub fn set_num_involuntary_context_switches(&mut self, n: i64) -> &mut Self {
        self.num_involuntary_context_switches = Some(n);
        self
    }
    pub fn set_memory_in_kb(&mut self, kb: i64) -> &mut Self {
        self.memory_in_kb = Some(kb);
        self
    }
    pub fn set_cache_hit(&mut self, cache_hit: bool) -> &mut Self {
        self.cache_hit = cache_hit;
        self
    }
    pub fn set_failure_message(&mut self, failure_message: &str) -> &mut Self {
        self.failure_message = failure_message.to_string();
        self
    }
    pub fn set_in_memory_output(&mut self, output: ActionInputRef, contents: Vec<u8>) -> &mut Self {
        self.in_memory_output_file = Some(output);
        self.in_memory_contents = Some(contents);
        self
    }
    pub fn set_remote(&mut self, remote: bool) -> &mut Self {
        self.remote = remote;
        self
    }
    pub fn set_digest(&mut self, digest: Option<Digest>) -> &mut Self {
        self.digest = digest;
        self
    }

    /// `setResourceUsageFromProto(Path statisticsPath)`.
    // Java: com/google/devtools/build/lib/actions/SpawnResult.java:793 SpawnResult.Builder.setResourceUsageFromProto
    pub fn set_resource_usage_from_proto(
        &mut self,
        statistics_path: &Path,
    ) -> std::io::Result<&mut Self> {
        if let Some(ru) = ExecutionStatistics::get_resource_usage(statistics_path)? {
            self.set_user_time_in_ms(ru.get_user_execution_time().as_millis() as i32);
            self.set_system_time_in_ms(ru.get_system_execution_time().as_millis() as i32);
            self.set_num_block_output_operations(ru.get_block_output_operations());
            self.set_num_block_input_operations(ru.get_block_input_operations());
            self.set_num_involuntary_context_switches(ru.get_involuntary_context_switches());
            // PORT: skipped, darwin only (maxrss in bytes there).
            self.set_memory_in_kb(ru.get_maximum_resident_set_size());
        }
        Ok(self)
    }
}
