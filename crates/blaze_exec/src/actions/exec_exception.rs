//! Port of `ExecException` and its subclasses used by spawn execution (`UserExecException`,
//! `EnvironmentalExecException`, `exec/SpawnExecException`).

use std::fmt;

use blaze_protos::failure_details::FailureDetail;

use super::spawn_result::SpawnResult;

/// Port of `ExecException`: a failure while executing an action or spawn.
// Java: com/google/devtools/build/lib/actions/ExecException.java:51 ExecException
#[derive(Debug, Clone)]
pub enum ExecException {
    /// `UserExecException`: the user's fault (bad inputs, failing command).
    User(UserExecException),
    /// `EnvironmentalExecException`: the environment's fault (I/O, sandbox setup).
    Environmental(EnvironmentalExecException),
    /// `SpawnExecException`: the spawn ran and failed.
    Spawn(Box<SpawnExecException>),
    /// `InterruptedException` (propagated through the same `Result`).
    Interrupted,
}

impl ExecException {
    /// `getMessage()`.
    pub fn get_message(&self) -> String {
        match self {
            ExecException::User(e) => e.message.clone(),
            ExecException::Environmental(e) => e.message.clone(),
            ExecException::Spawn(e) => e.message.clone(),
            ExecException::Interrupted => "interrupted".to_string(),
        }
    }

    /// `isCatastrophic()`.
    pub fn is_catastrophic(&self) -> bool {
        match self {
            ExecException::Spawn(e) => e.catastrophe,
            _ => false,
        }
    }

    /// `getMessageForActionExecutionException()`.
    // Java: com/google/devtools/build/lib/actions/ExecException.java:96 ExecException.getMessageForActionExecutionException
    pub fn get_message_for_action_execution_exception(&self) -> String {
        match self {
            ExecException::Spawn(e) => e.get_message_for_action_execution_exception(),
            _ => self.get_message(),
        }
    }

    /// `getFailureDetail(String message)`.
    pub fn get_failure_detail(&self, message: &str) -> Option<FailureDetail> {
        match self {
            ExecException::User(e) => {
                let mut d = e.failure_detail.clone();
                d.message = message.to_string();
                Some(d)
            }
            ExecException::Environmental(e) => {
                let mut d = e.failure_detail.clone();
                d.message = message.to_string();
                Some(d)
            }
            ExecException::Spawn(e) => e.result.failure_detail().cloned(),
            ExecException::Interrupted => None,
        }
    }
}

impl fmt::Display for ExecException {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.get_message())
    }
}

impl std::error::Error for ExecException {}

/// `ExecException(String message, Throwable cause)`: `message + ": " + cause.getMessage()`.
// Java: com/google/devtools/build/lib/actions/ExecException.java:75 ExecException.<init>
fn message_with_cause(message: &str, cause: Option<&str>) -> String {
    match cause {
        None => message.to_string(),
        Some(c) => format!("{message}: {c}"),
    }
}

/// Port of `UserExecException`.
// Java: com/google/devtools/build/lib/actions/UserExecException.java:23 UserExecException
#[derive(Debug, Clone)]
pub struct UserExecException {
    pub message: String,
    pub failure_detail: FailureDetail,
}

impl UserExecException {
    /// `new UserExecException(FailureDetail)`.
    pub fn new(failure_detail: FailureDetail) -> UserExecException {
        UserExecException {
            message: failure_detail.message.clone(),
            failure_detail,
        }
    }
    /// `new UserExecException(Throwable cause, FailureDetail)`.
    pub fn with_cause(cause: &dyn fmt::Display, failure_detail: FailureDetail) -> Self {
        UserExecException {
            message: message_with_cause(&failure_detail.message, Some(&cause.to_string())),
            failure_detail,
        }
    }
}

impl From<UserExecException> for ExecException {
    fn from(e: UserExecException) -> Self {
        ExecException::User(e)
    }
}

/// Port of `EnvironmentalExecException`.
// Java: com/google/devtools/build/lib/actions/EnvironmentalExecException.java:30 EnvironmentalExecException
#[derive(Debug, Clone)]
pub struct EnvironmentalExecException {
    pub message: String,
    pub failure_detail: FailureDetail,
}

impl EnvironmentalExecException {
    /// `new EnvironmentalExecException(FailureDetail)`.
    pub fn new(failure_detail: FailureDetail) -> Self {
        EnvironmentalExecException {
            message: failure_detail.message.clone(),
            failure_detail,
        }
    }
    /// `new EnvironmentalExecException(Exception cause, FailureDetail)`.
    pub fn with_cause(cause: &dyn fmt::Display, failure_detail: FailureDetail) -> Self {
        EnvironmentalExecException {
            message: message_with_cause(&failure_detail.message, Some(&cause.to_string())),
            failure_detail,
        }
    }
}

impl From<EnvironmentalExecException> for ExecException {
    fn from(e: EnvironmentalExecException) -> Self {
        ExecException::Environmental(e)
    }
}

/// Port of `exec/SpawnExecException`: a spawn ran and did not succeed.
// Java: com/google/devtools/build/lib/exec/SpawnExecException.java:30 SpawnExecException
#[derive(Debug, Clone)]
pub struct SpawnExecException {
    pub message: String,
    pub result: SpawnResult,
    pub forcibly_run_remotely: bool,
    pub catastrophe: bool,
}

impl SpawnExecException {
    /// `new SpawnExecException(message, result, forciblyRunRemotely)`.
    // Java: com/google/devtools/build/lib/exec/SpawnExecException.java:35 SpawnExecException.<init>
    pub fn new(message: String, result: SpawnResult, forcibly_run_remotely: bool) -> Self {
        blaze_util::check_argument!(
            result.status() != super::spawn_result::Status::Success,
            "Can't create exception with successful spawn result."
        );
        SpawnExecException {
            message,
            catastrophe: result.is_catastrophe(),
            result,
            forcibly_run_remotely,
        }
    }

    /// `getSpawnResult()`.
    pub fn get_spawn_result(&self) -> &SpawnResult {
        &self.result
    }

    /// `hasTimedOut()`.
    pub fn has_timed_out(&self) -> bool {
        self.result.status() == super::spawn_result::Status::Timeout
    }

    /// `getMessageForActionExecutionException()`.
    // Java: com/google/devtools/build/lib/exec/SpawnExecException.java:64 SpawnExecException.getMessageForActionExecutionException
    pub fn get_message_for_action_execution_exception(&self) -> String {
        self.result
            .get_detail_message(&self.message, self.catastrophe, self.forcibly_run_remotely)
    }

    /// The message of the `ActionExecutionException` made from this exception
    /// (`toActionExecutionException`): `"<action.describe()> failed: <detail message>"`.
    // Java: com/google/devtools/build/lib/exec/SpawnExecException.java:75 SpawnExecException.toActionExecutionException
    pub fn to_action_execution_exception_message(&self, action_description: &str) -> String {
        format!(
            "{} failed: {}",
            action_description,
            self.get_message_for_action_execution_exception()
        )
    }
}

impl From<SpawnExecException> for ExecException {
    fn from(e: SpawnExecException) -> Self {
        ExecException::Spawn(Box::new(e))
    }
}
