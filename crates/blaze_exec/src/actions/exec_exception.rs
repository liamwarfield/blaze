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

    /// `this instanceof TestExecException`.
    pub fn is_test_exec_exception(&self) -> bool {
        matches!(self, ExecException::User(e) if e.test_exec_exception)
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
    /// This is a `TestExecException` (see [`test_exec_exception`]).
    ///
    /// [`test_exec_exception`]: crate::analysis::test::test_runner_action::test_exec_exception
    pub test_exec_exception: bool,
}

impl UserExecException {
    /// `new UserExecException(FailureDetail)`.
    pub fn new(failure_detail: FailureDetail) -> UserExecException {
        UserExecException {
            message: failure_detail.message.clone(),
            failure_detail,
            test_exec_exception: false,
        }
    }
    /// `new UserExecException(Throwable cause, FailureDetail)`.
    pub fn with_cause(cause: &dyn fmt::Display, failure_detail: FailureDetail) -> Self {
        UserExecException {
            message: message_with_cause(&failure_detail.message, Some(&cause.to_string())),
            failure_detail,
            test_exec_exception: false,
        }
    }
}

impl From<UserExecException> for ExecException {
    fn from(e: UserExecException) -> Self {
        ExecException::User(e)
    }
}

/// Port of `EnvironmentalExecException`.
///
/// PORT: also stands for `LostInputsExecException` (see [`LostInputsExecException`]), with
/// `lost_inputs` set: a new `ExecException` variant would change exhaustive matches in crates
/// this port does not own.
// Java: com/google/devtools/build/lib/actions/EnvironmentalExecException.java:30 EnvironmentalExecException
#[derive(Debug, Clone)]
pub struct EnvironmentalExecException {
    pub message: String,
    pub failure_detail: FailureDetail,
    /// `this instanceof LostInputsExecException`: its `getLostInputs()`.
    pub lost_inputs: Option<std::sync::Arc<LostArtifacts>>,
}

impl EnvironmentalExecException {
    /// `new EnvironmentalExecException(FailureDetail)`.
    pub fn new(failure_detail: FailureDetail) -> Self {
        EnvironmentalExecException {
            message: failure_detail.message.clone(),
            failure_detail,
            lost_inputs: None,
        }
    }
    /// `new EnvironmentalExecException(Exception cause, FailureDetail)`.
    pub fn with_cause(cause: &dyn fmt::Display, failure_detail: FailureDetail) -> Self {
        EnvironmentalExecException {
            message: message_with_cause(&failure_detail.message, Some(&cause.to_string())),
            failure_detail,
            lost_inputs: None,
        }
    }
}

/// `LostInputsExecException.LostArtifacts`: the lost inputs by digest (`hash/size`), an
/// `ImmutableSetMultimap<String, ActionInput>` (insertion order).
// Java: com/google/devtools/build/lib/actions/LostInputsExecException.java LostInputsExecException.LostArtifacts
#[derive(Debug, Clone, Default)]
pub struct LostArtifacts {
    /// `byDigest()`.
    pub by_digest: indexmap::IndexMap<String, indexmap::IndexSet<super::ActionInputRef>>,
}

impl LostArtifacts {
    /// `LostArtifacts.EMPTY`.
    // Java: com/google/devtools/build/lib/actions/ImportantOutputHandler.java:144 LostArtifacts.EMPTY
    pub fn empty() -> LostArtifacts {
        LostArtifacts::default()
    }

    /// `isEmpty()`.
    pub fn is_empty(&self) -> bool {
        self.by_digest.is_empty()
    }

    /// Throws [`LostInputsExecException`] if this instance is not empty.
    // Java: com/google/devtools/build/lib/actions/ImportantOutputHandler.java:155 LostArtifacts.throwIfNotEmpty
    pub fn throw_if_not_empty(&self) -> Result<(), ExecException> {
        if !self.is_empty() {
            return Err(LostInputsExecException::create(self.clone()));
        }
        Ok(())
    }

    /// The `(digest, input)` entries in multimap order (`lostInputs.entries()`).
    pub fn entries(&self) -> Vec<(&str, &super::ActionInputRef)> {
        self.by_digest
            .iter()
            .flat_map(|(digest, inputs)| inputs.iter().map(move |i| (digest.as_str(), i)))
            .collect()
    }
}

/// `LostInputsExecException`: an action's inputs were lost (a remote cache no longer has their
/// blobs). Built as an [`ExecException::Environmental`] whose `lost_inputs` is set.
// Java: com/google/devtools/build/lib/actions/LostInputsExecException.java:33 LostInputsExecException
pub struct LostInputsExecException;

impl LostInputsExecException {
    /// `new LostInputsExecException(lostInputs, cause)`: the message is `lost inputs with
    /// digests: ` + the digests joined by `,`; the failure detail is
    /// `Execution.ACTION_INPUT_LOST`.
    // Java: com/google/devtools/build/lib/actions/LostInputsExecException.java:43 LostInputsExecException.<init>
    // Java: com/google/devtools/build/lib/actions/LostInputsExecException.java:59 LostInputsExecException.getFailureDetail
    pub fn create(lost_inputs: LostArtifacts) -> ExecException {
        blaze_util::check_argument!(!lost_inputs.is_empty(), "No inputs were lost");
        let message = format!(
            "lost inputs with digests: {}",
            lost_inputs
                .by_digest
                .keys()
                .map(String::as_str)
                .collect::<Vec<_>>()
                .join(",")
        );
        let failure_detail = FailureDetail {
            message: message.clone(),
            category: Some(
                blaze_protos::failure_details::failure_detail::Category::Execution(
                    blaze_protos::failure_details::Execution {
                        code: blaze_protos::failure_details::execution::Code::ActionInputLost
                            as i32,
                    },
                ),
            ),
        };
        ExecException::Environmental(EnvironmentalExecException {
            message,
            failure_detail,
            lost_inputs: Some(std::sync::Arc::new(lost_inputs)),
        })
    }
}

impl ExecException {
    /// `this instanceof LostInputsExecException ? getLostInputs() : null`.
    pub fn get_lost_inputs(&self) -> Option<&std::sync::Arc<LostArtifacts>> {
        match self {
            ExecException::Environmental(e) => e.lost_inputs.as_ref(),
            _ => None,
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
