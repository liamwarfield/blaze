//! `BlazeCommandResult` (from `lib/runtime`).
//!
//! PORT: Java's `com.google.devtools.build.lib.runtime.BlazeCommandResult` lives in this crate
//! (decision D3: `blaze_server` sits below `blaze_runtime` and `GrpcServerImpl` needs it);
//! `blaze_runtime` re-exports it.

use std::fmt;
use std::sync::Arc;

use blaze_protos::command_server::ExecRequest;
use blaze_protos::failure_details::FailureDetail;
use blaze_protos::google::protobuf::Any;
use blaze_util::exit_code::{DetailedExitCode, ExitCode};

use crate::idle_task::IdleTask;

/// The result of a Blaze command. It is usually an [`ExitCode`] with optional
/// [`FailureDetail`], but can be an instruction to the client to execute a particular binary for
/// "blaze run".
// Java: com/google/devtools/build/lib/runtime/BlazeCommandResult.java:36 BlazeCommandResult
#[derive(Clone)]
pub struct BlazeCommandResult {
    detailed_exit_code: DetailedExitCode,
    /// `execDescription`.
    exec_description: Option<ExecRequest>,
    response_extensions: Vec<Any>,
    shutdown: bool,
    idle_tasks: Vec<Arc<dyn IdleTask>>,
}

impl BlazeCommandResult {
    // Java: com/google/devtools/build/lib/runtime/BlazeCommandResult.java:44 BlazeCommandResult.<init>
    fn new_full(
        detailed_exit_code: DetailedExitCode,
        exec_description: Option<ExecRequest>,
        shutdown: bool,
        response_extensions: Vec<Any>,
        idle_tasks: Vec<Arc<dyn IdleTask>>,
    ) -> Self {
        BlazeCommandResult {
            detailed_exit_code,
            exec_description,
            shutdown,
            response_extensions,
            idle_tasks,
        }
    }

    // Java: com/google/devtools/build/lib/runtime/BlazeCommandResult.java:57 BlazeCommandResult.<init>
    fn new(
        detailed_exit_code: DetailedExitCode,
        exec_description: Option<ExecRequest>,
        shutdown: bool,
    ) -> Self {
        Self::new_full(
            detailed_exit_code,
            exec_description,
            shutdown,
            Vec::new(),
            Vec::new(),
        )
    }

    // Java: com/google/devtools/build/lib/runtime/BlazeCommandResult.java:62 BlazeCommandResult.getExitCode
    pub fn get_exit_code(&self) -> ExitCode {
        self.detailed_exit_code.get_exit_code()
    }

    // Java: com/google/devtools/build/lib/runtime/BlazeCommandResult.java:66 BlazeCommandResult.getDetailedExitCode
    pub fn get_detailed_exit_code(&self) -> &DetailedExitCode {
        &self.detailed_exit_code
    }

    // Java: com/google/devtools/build/lib/runtime/BlazeCommandResult.java:71 BlazeCommandResult.getFailureDetail
    pub fn get_failure_detail(&self) -> Option<&FailureDetail> {
        self.detailed_exit_code.get_failure_detail()
    }

    // Java: com/google/devtools/build/lib/runtime/BlazeCommandResult.java:75 BlazeCommandResult.shutdown
    pub fn shutdown(&self) -> bool {
        self.shutdown
    }

    // Java: com/google/devtools/build/lib/runtime/BlazeCommandResult.java:80 BlazeCommandResult.getExecRequest
    pub fn get_exec_request(&self) -> Option<&ExecRequest> {
        self.exec_description.as_ref()
    }

    // Java: com/google/devtools/build/lib/runtime/BlazeCommandResult.java:84 BlazeCommandResult.isSuccess
    pub fn is_success(&self) -> bool {
        self.detailed_exit_code.is_success()
    }

    // Java: com/google/devtools/build/lib/runtime/BlazeCommandResult.java:88 BlazeCommandResult.getResponseExtensions
    pub fn get_response_extensions(&self) -> &[Any] {
        &self.response_extensions
    }

    // Java: com/google/devtools/build/lib/runtime/BlazeCommandResult.java:92 BlazeCommandResult.getIdleTasks
    pub fn get_idle_tasks(&self) -> &[Arc<dyn IdleTask>] {
        &self.idle_tasks
    }

    // Java: com/google/devtools/build/lib/runtime/BlazeCommandResult.java:107 BlazeCommandResult.shutdownOnSuccess
    pub fn shutdown_on_success() -> Self {
        Self::new(DetailedExitCode::success(), None, true)
    }

    /// PORT: Java takes a `Crash` (`lib/bugreport`, not ported) and uses only its
    /// `getDetailedExitCode()`; the caller passes that.
    // Java: com/google/devtools/build/lib/runtime/BlazeCommandResult.java:111 BlazeCommandResult.createShutdown
    pub fn create_shutdown(crash_detailed_exit_code: DetailedExitCode) -> Self {
        Self::new(crash_detailed_exit_code, None, true)
    }

    // Java: com/google/devtools/build/lib/runtime/BlazeCommandResult.java:115 BlazeCommandResult.success
    pub fn success() -> Self {
        Self::new(DetailedExitCode::success(), None, false)
    }

    // Java: com/google/devtools/build/lib/runtime/BlazeCommandResult.java:119 BlazeCommandResult.failureDetail
    pub fn failure_detail(failure_detail: FailureDetail) -> Self {
        Self::new(DetailedExitCode::of(failure_detail), None, false)
    }

    // Java: com/google/devtools/build/lib/runtime/BlazeCommandResult.java:123 BlazeCommandResult.detailedExitCode
    pub fn detailed_exit_code(detailed_exit_code: DetailedExitCode) -> Self {
        Self::new(detailed_exit_code, None, false)
    }

    // Java: com/google/devtools/build/lib/runtime/BlazeCommandResult.java:127 BlazeCommandResult.withResponseExtensions
    pub fn with_response_extensions(
        result: BlazeCommandResult,
        response_extensions: Vec<Any>,
    ) -> Self {
        Self::new_full(
            result.detailed_exit_code,
            result.exec_description,
            result.shutdown,
            response_extensions,
            result.idle_tasks,
        )
    }

    // Java: com/google/devtools/build/lib/runtime/BlazeCommandResult.java:137 BlazeCommandResult.withIdleTasks
    pub fn with_idle_tasks(result: BlazeCommandResult, idle_tasks: Vec<Arc<dyn IdleTask>>) -> Self {
        Self::new_full(
            result.detailed_exit_code,
            result.exec_description,
            result.shutdown,
            result.response_extensions,
            idle_tasks,
        )
    }

    // Java: com/google/devtools/build/lib/runtime/BlazeCommandResult.java:147 BlazeCommandResult.execute
    pub fn execute(exec_description: ExecRequest) -> Self {
        Self::new(DetailedExitCode::success(), Some(exec_description), false)
    }

    // Java: com/google/devtools/build/lib/runtime/BlazeCommandResult.java:152 BlazeCommandResult.execute
    pub fn execute_with_detailed_exit_code(
        exec_description: ExecRequest,
        detailed_exit_code: DetailedExitCode,
    ) -> Self {
        Self::new(detailed_exit_code, Some(exec_description), false)
    }
}

/// PORT: Java has no `equals` (identity). Value equality here compares every field, idle tasks
/// by identity, so that tests can compare results as `blaze_runtime`'s derived `PartialEq` did.
impl PartialEq for BlazeCommandResult {
    fn eq(&self, other: &Self) -> bool {
        self.detailed_exit_code == other.detailed_exit_code
            && self.exec_description == other.exec_description
            && self.response_extensions == other.response_extensions
            && self.shutdown == other.shutdown
            && self.idle_tasks.len() == other.idle_tasks.len()
            && self
                .idle_tasks
                .iter()
                .zip(&other.idle_tasks)
                .all(|(a, b)| Arc::ptr_eq(a, b))
    }
}

/// PORT: Java's `toString` (`MoreObjects.toStringHelper`) prints protos with their text format,
/// which no caller depends on; the fields are printed with Rust's `Debug`.
// Java: com/google/devtools/build/lib/runtime/BlazeCommandResult.java:97 BlazeCommandResult.toString
impl fmt::Debug for BlazeCommandResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BlazeCommandResult")
            .field("exitCode", &self.get_exit_code())
            .field("failureDetail", &self.get_failure_detail())
            .field("execDescription", &self.exec_description)
            .field("shutdown", &self.shutdown)
            .field("responseExtensions", &self.response_extensions)
            .finish()
    }
}
