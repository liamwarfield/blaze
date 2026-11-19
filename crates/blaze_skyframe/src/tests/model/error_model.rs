//! `ErrorInfo` reduced to what the model compares.

use super::spec::{ErrClass, NodeId};

/// Java `ErrorInfo` (ErrorInfo.java) as the model needs it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MError {
    /// The node whose exception is the representative one (`ErrorInfo.getException`), `None`
    /// for a pure cycle error.
    pub exception: Option<NodeId>,
    pub transitively_transient: bool,
    /// `!getCycleInfo().isEmpty()`.
    pub cycle: bool,
    /// `ErrorInfo.hasValue()` (Java `ErrorInfoWithValue`).
    pub has_value: bool,
}

impl MError {
    // Java: com/google/devtools/build/skyframe/ErrorInfo.java:31 ErrorInfo.fromException
    pub fn from_exception(origin: NodeId, transitively_transient: bool) -> MError {
        MError {
            exception: Some(origin),
            transitively_transient,
            cycle: false,
            has_value: false,
        }
    }

    // Java: com/google/devtools/build/skyframe/ErrorInfo.java:43 ErrorInfo.fromCycle
    pub fn from_cycle() -> MError {
        MError {
            exception: None,
            transitively_transient: false,
            cycle: true,
            has_value: false,
        }
    }

    // Java: com/google/devtools/build/skyframe/ErrorInfo.java:53 ErrorInfo.withValue
    pub fn with_value(self) -> MError {
        MError {
            has_value: true,
            ..self
        }
    }

    /// Child errors in the order the function requested them (`childErrorInfos` is a
    /// `LinkedHashSet` filled by `processDepValue`, SkyFunctionEnvironment.java:612-660).
    // Java: com/google/devtools/build/skyframe/ErrorInfo.java:63 ErrorInfo.fromChildErrors
    pub fn from_child_errors(child_errors: &[MError]) -> MError {
        assert!(!child_errors.is_empty(), "childErrors may not be empty");
        let mut representative: Option<NodeId> = None;
        let mut representative_came_with_value = false;
        let mut transient = false;
        let mut cycle = false;
        for child in child_errors {
            // Child errors that come with a value indicates that the error was somehow
            // tolerated. Priorities should be given to those without values. Otherwise, choose
            // the first error.
            if representative.is_none()
                || (representative_came_with_value && child.exception.is_some())
            {
                representative = child.exception;
                representative_came_with_value = child.has_value;
            }
            cycle |= child.cycle;
            transient |= child.transitively_transient;
        }
        MError {
            exception: representative,
            transitively_transient: transient,
            cycle,
            has_value: false,
        }
    }

    pub fn class(&self) -> ErrClass {
        if self.cycle {
            ErrClass::Cycle
        } else {
            ErrClass::Exception {
                origin: self
                    .exception
                    .expect("an error without cycles has an exception"),
                transient: self.transitively_transient,
            }
        }
    }
}
