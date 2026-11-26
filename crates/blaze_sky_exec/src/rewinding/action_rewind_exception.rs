//! Port of `lib/skyframe/rewinding/ActionRewindException.java`.

use blaze_protos::failure_details::{
    ActionRewinding, FailureDetail, Spawn, action_rewinding, failure_detail, spawn,
};
use std::fmt;

/// Exception thrown when action rewinding is not possible.
// Java: com/google/devtools/build/lib/skyframe/rewinding/ActionRewindException.java:26 ActionRewindException
#[derive(Clone, Debug)]
pub enum ActionRewindException {
    // Java: com/google/devtools/build/lib/skyframe/rewinding/ActionRewindException.java:33 GenericActionRewindException
    Generic {
        message: String,
        code: action_rewinding::Code,
    },
    // Java: com/google/devtools/build/lib/skyframe/rewinding/ActionRewindException.java:51 FallbackToBuildRewindingException
    FallbackToBuildRewinding { message: String },
}

impl ActionRewindException {
    // Java: com/google/devtools/build/lib/skyframe/rewinding/ActionRewindException.java:36 GenericActionRewindException.<init>
    pub(crate) fn generic(message: String, code: action_rewinding::Code) -> ActionRewindException {
        ActionRewindException::Generic { message, code }
    }

    /// `getMessage()`.
    pub fn get_message(&self) -> &str {
        match self {
            ActionRewindException::Generic { message, .. }
            | ActionRewindException::FallbackToBuildRewinding { message } => message,
        }
    }

    /// `getDetailedExitCode()`'s failure detail.
    // Java: com/google/devtools/build/lib/skyframe/rewinding/ActionRewindException.java:42 GenericActionRewindException.getDetailedExitCode
    // Java: com/google/devtools/build/lib/skyframe/rewinding/ActionRewindException.java:57 FallbackToBuildRewindingException.getDetailedExitCode
    pub fn get_failure_detail(&self) -> FailureDetail {
        match self {
            ActionRewindException::Generic { message, code } => FailureDetail {
                message: message.clone(),
                category: Some(failure_detail::Category::ActionRewinding(ActionRewinding {
                    code: *code as i32,
                })),
            },
            ActionRewindException::FallbackToBuildRewinding { message } => FailureDetail {
                message: message.clone(),
                category: Some(failure_detail::Category::Spawn(Spawn {
                    code: spawn::Code::RemoteCacheEvicted as i32,
                    ..Default::default()
                })),
            },
        }
    }
}

impl fmt::Display for ActionRewindException {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.get_message())
    }
}
