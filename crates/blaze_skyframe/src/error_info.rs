//! Port of `ErrorInfo`.

use crate::{CycleInfo, SkyError, SkyFunctionException, SkyKey};
use blaze_util::check_argument;
use std::fmt;

/// Information about why a SkyValue failed to evaluate successfully.
///
/// PORT: Java's `ErrorInfoWithValue` subclass is the `has_value` flag.
///
/// Root causes are not part of `ErrorInfo` at the pin (Bazel 9.3.2); domain exceptions carry them
/// (`ConfiguredValueCreationException.getRootCauses`, `ActionExecutionException.getRootCauses`).
// Java: com/google/devtools/build/skyframe/ErrorInfo.java:29 ErrorInfo
#[derive(Clone)]
pub struct ErrorInfo {
    exception: Option<SkyError>,
    cycles: Vec<CycleInfo>,
    is_directly_transient: bool,
    is_transitively_transient: bool,
    is_catastrophic: bool,
    has_value: bool,
}

impl ErrorInfo {
    /// Create an ErrorInfo from a `ReifiedSkyFunctionException`.
    // Java: com/google/devtools/build/skyframe/ErrorInfoManager.java:22 ErrorInfoManager.fromException
    // Java: com/google/devtools/build/skyframe/ErrorInfoManager.java:45 ErrorInfoManager.UseChildErrorInfoIfNecessary.fromException
    // Java: com/google/devtools/build/skyframe/ErrorInfo.java:31 ErrorInfo.fromException
    pub fn from_exception(
        sky_function_exception: &SkyFunctionException,
        is_transitively_transient: bool,
    ) -> ErrorInfo {
        let root_cause_exception = sky_function_exception.get_cause().clone();
        ErrorInfo::new(
            Some(root_cause_exception),
            /* cycles= */ vec![],
            sky_function_exception.is_transient(),
            is_transitively_transient || sky_function_exception.is_transient(),
            sky_function_exception.is_catastrophic(),
        )
    }

    /// Create an ErrorInfo from a [`CycleInfo`].
    // Java: com/google/devtools/build/skyframe/ErrorInfo.java:43 ErrorInfo.fromCycle
    pub fn from_cycle(cycle_info: CycleInfo) -> ErrorInfo {
        ErrorInfo::new(
            /* exception= */ None,
            vec![cycle_info],
            /* is_directly_transient= */ false,
            /* is_transitively_transient= */ false,
            /* is_catastrophic= */ false,
        )
    }

    /// A wrapper that indicates that there's a value associated with the ErrorInfo's SkyKey.
    // Java: com/google/devtools/build/skyframe/ErrorInfo.java:53 ErrorInfo.withValue
    pub fn with_value(wrapped: &ErrorInfo) -> ErrorInfo {
        ErrorInfo {
            has_value: true,
            ..ErrorInfo::new(
                wrapped.exception.clone(),
                wrapped.cycles.clone(),
                wrapped.is_directly_transient,
                wrapped.is_transitively_transient,
                wrapped.is_catastrophic,
            )
        }
    }

    /// Create an ErrorInfo from a collection of existing errors.
    // Java: com/google/devtools/build/skyframe/ErrorInfo.java:63 ErrorInfo.fromChildErrors
    pub fn from_child_errors<'a>(
        current_value: &SkyKey,
        child_errors: impl IntoIterator<Item = &'a ErrorInfo>,
    ) -> ErrorInfo {
        let mut cycle_builder: Vec<CycleInfo> = vec![];
        let mut representative_exception: Option<SkyError> = None;
        let mut representative_exception_came_with_value = false;
        let mut is_transitively_transient = false;
        let mut is_catastrophic = false;
        let mut any = false;
        for child in child_errors {
            any = true;
            // Child errors that come with a value indicates that the error was somehow tolerated.
            // Priorities should be given to those without values.
            // Otherwise, choose the first error.
            if representative_exception.is_none()
                || (representative_exception_came_with_value && child.exception.is_some())
            {
                representative_exception = child.exception.clone();
                representative_exception_came_with_value = child.has_value();
            }
            cycle_builder.extend(CycleInfo::prepare_cycles(current_value, &child.cycles));
            is_transitively_transient |= child.is_transitively_transient;
            is_catastrophic |= child.is_catastrophic;
        }
        blaze_util::check_state!(any, "childErrors may not be empty {:?}", current_value);

        ErrorInfo::new(
            representative_exception,
            cycle_builder,
            /* is_directly_transient= */ false,
            is_transitively_transient,
            is_catastrophic,
        )
    }

    /// Whether the SkyKey of this ErrorInfo has a corresponding SkyValue.
    // Java: com/google/devtools/build/skyframe/ErrorInfo.java:96 ErrorInfo.hasValue
    pub(crate) fn has_value(&self) -> bool {
        self.has_value
    }

    // Java: com/google/devtools/build/skyframe/ErrorInfo.java:107 ErrorInfo.<init>
    pub fn new(
        exception: Option<SkyError>,
        cycles: Vec<CycleInfo>,
        is_directly_transient: bool,
        is_transitively_transient: bool,
        is_catastrophic: bool,
    ) -> ErrorInfo {
        let info = ErrorInfo {
            exception,
            cycles,
            is_directly_transient,
            is_transitively_transient,
            is_catastrophic,
            has_value: false,
        };
        // Guava's lenient formatting appends the unused argument as " [arg]".
        check_argument!(
            info.exception.is_some() || !info.cycles.is_empty(),
            "At least one of exception and cycles must be present [{:?}]",
            info
        );
        check_argument!(
            !info.is_directly_transient || info.is_transitively_transient,
            "Cannot be directly transient but not transitively transient [{:?}]",
            info
        );
        info
    }

    /// The exception thrown when building a value. May be null if value's only error is
    /// depending on a cycle.
    ///
    /// The exception is used for reporting and thus may ultimately be rethrown by the caller.
    /// As well, during a --nokeep_going evaluation, if an error value is encountered from an
    /// earlier --keep_going build, the exception to be thrown is taken from here.
    // Java: com/google/devtools/build/skyframe/ErrorInfo.java:193 ErrorInfo.getException
    pub fn get_exception(&self) -> Option<&SkyError> {
        self.exception.as_ref()
    }

    /// Any cycles found when building this value.
    ///
    /// If there are a large number of cycles, only a limited number are returned here.
    ///
    /// If this value has a child through which there are multiple paths to the same cycle, only
    /// one path is returned here. However, if there are multiple paths to the same cycle, each of
    /// which goes through a different child, each of them is returned here.
    // Java: com/google/devtools/build/skyframe/ErrorInfo.java:206 ErrorInfo.getCycleInfo
    pub fn get_cycle_info(&self) -> &[CycleInfo] {
        &self.cycles
    }

    /// Returns true iff the error is directly transient, i.e. if there was a transient error
    /// encountered during the computation itself.
    // Java: com/google/devtools/build/skyframe/ErrorInfo.java:217 ErrorInfo.isDirectlyTransient
    pub fn is_directly_transient(&self) -> bool {
        self.is_directly_transient
    }

    /// Returns true iff the error is transitively transient, i.e. if retrying the same computation
    /// could lead to a different result.
    // Java: com/google/devtools/build/skyframe/ErrorInfo.java:225 ErrorInfo.isTransitivelyTransient
    pub fn is_transitively_transient(&self) -> bool {
        self.is_transitively_transient
    }

    /// Returns true iff the error is catastrophic, i.e. it should halt even for a keepGoing
    /// update() call.
    // Java: com/google/devtools/build/skyframe/ErrorInfo.java:233 ErrorInfo.isCatastrophic
    pub fn is_catastrophic(&self) -> bool {
        self.is_catastrophic
    }

    /// Downcasts the exception to a concrete error type (Java: `instanceof` on `getException()`).
    pub fn exception_as<E: std::error::Error + 'static>(&self) -> Option<&E> {
        self.exception.as_ref().and_then(|e| e.downcast_ref::<E>())
    }
}

impl fmt::Debug for ErrorInfo {
    // Java: com/google/devtools/build/skyframe/ErrorInfo.java:175 ErrorInfo.toString
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = if self.has_value {
            "ErrorInfoWithValue"
        } else {
            "ErrorInfo"
        };
        write!(f, "{name}{{exception=")?;
        match &self.exception {
            // PORT: Java prints Throwable.toString ("<class name>: <message>"); Rust errors have no
            // class name, so the message alone is printed.
            Some(e) => write!(f, "{e}")?,
            None => f.write_str("null")?,
        }
        write!(
            f,
            ", cycles={:?}, isCatastrophic={}, isDirectlyTransient={}, isTransitivelyTransient={}}}",
            self.cycles,
            self.is_catastrophic,
            self.is_directly_transient,
            self.is_transitively_transient
        )
    }
}

#[cfg(test)]
mod tests {
    // Java: src/test/java/com/google/devtools/build/skyframe/ErrorInfoTest.java
    use super::*;
    use crate::{Key, SkyFunctionName, Transience};
    use std::sync::Arc;

    #[derive(Debug, Clone, PartialEq, Eq, Hash)]
    struct K(&'static str);
    impl Key for K {
        type Value = ();
        const FUNCTION: SkyFunctionName = SkyFunctionName("K");
        // Java: SkyFunctionName.FOR_TESTING (createHermetic)
        const HERMETICITY: crate::FunctionHermeticity = crate::FunctionHermeticity::Hermetic;
    }

    #[derive(Debug)]
    struct IoException(&'static str);
    impl fmt::Display for IoException {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str(self.0)
        }
    }
    impl std::error::Error for IoException {}

    fn dummy(cause: SkyError, is_transient: bool, is_catastrophic: bool) -> SkyFunctionException {
        SkyFunctionException {
            cause,
            transience: if is_transient {
                Transience::Transient
            } else {
                Transience::Persistent
            },
            catastrophic: is_catastrophic,
        }
    }

    fn run_test_from_exception(is_directly_transient: bool, is_transitively_transient: bool) {
        let exception: SkyError = Arc::new(IoException("ehhhhh"));
        let dummy_exception = dummy(exception.clone(), is_directly_transient, false);
        let error_info = ErrorInfo::from_exception(&dummy_exception, is_transitively_transient);
        assert!(Arc::ptr_eq(error_info.get_exception().unwrap(), &exception));
        assert!(error_info.get_cycle_info().is_empty());
        assert_eq!(error_info.is_directly_transient(), is_directly_transient);
        assert_eq!(
            error_info.is_transitively_transient(),
            is_directly_transient || is_transitively_transient
        );
        assert!(!error_info.is_catastrophic());
    }

    #[test]
    fn test_from_exception_non_transient() {
        run_test_from_exception(false, false);
    }

    #[test]
    fn test_from_exception_directly_transient() {
        run_test_from_exception(true, false);
    }

    #[test]
    fn test_from_exception_transitively_transient() {
        run_test_from_exception(false, true);
    }

    #[test]
    fn test_from_exception_directly_and_transitively_transient() {
        run_test_from_exception(true, true);
    }

    #[test]
    fn test_from_cycle() {
        let cycle = CycleInfo::create_cycle_info(
            vec![SkyKey::new(K("PATH, 1234"))],
            vec![SkyKey::new(K("CYCLE, 4321"))],
        );
        let error_info = ErrorInfo::from_cycle(cycle);
        assert!(error_info.get_exception().is_none());
        assert!(!error_info.is_transitively_transient());
        assert!(!error_info.is_catastrophic());
    }

    #[test]
    fn test_from_child_errors() {
        let cycle = CycleInfo::create_cycle_info(
            vec![SkyKey::new(K("PATH, 1234"))],
            vec![SkyKey::new(K("CYCLE, 4321"))],
        );
        let cycle_error_info = ErrorInfo::from_cycle(cycle.clone());

        let exception1: SkyError = Arc::new(IoException("ehhhhh"));
        let exception_error_info1 =
            ErrorInfo::from_exception(&dummy(exception1.clone(), true, false), false);

        // N.B this ErrorInfo will be catastrophic.
        let exception2: SkyError = Arc::new(IoException("blahhhhh"));
        let exception_error_info2 =
            ErrorInfo::from_exception(&dummy(exception2, false, true), false);

        let current_key = SkyKey::new(K("CURRENT, 9876"));

        let error_info = ErrorInfo::from_child_errors(
            &current_key,
            [
                &cycle_error_info,
                &exception_error_info1,
                &exception_error_info2,
            ],
        );

        // For simplicity we test the current implementation detail that we choose the first
        // non-null exception that we encounter. This isn't necessarily a requirement of the
        // interface, but it makes the test convenient and is a way to document the current
        // behavior.
        assert!(Arc::ptr_eq(
            error_info.get_exception().unwrap(),
            &exception1
        ));

        assert_eq!(
            error_info.get_cycle_info(),
            &[CycleInfo::create_cycle_info(
                vec![current_key.clone(), cycle.get_path_to_cycle()[0].clone()],
                cycle.get_cycle().to_vec()
            )]
        );
        assert!(error_info.is_transitively_transient());
        assert!(error_info.is_catastrophic());
    }

    #[test]
    #[should_panic(expected = "At least one of exception and cycles must be present")]
    fn cannot_create_error_info_without_exception_or_cycle() {
        ErrorInfo::new(None, vec![], false, false, false);
    }

    #[test]
    #[should_panic(expected = "Cannot be directly transient but not transitively transient")]
    fn cannot_create_error_info_with_direct_transience_but_not_transitive_transience() {
        ErrorInfo::new(
            Some(Arc::new(IoException("e"))),
            vec![],
            /* is_directly_transient= */ true,
            /* is_transitively_transient= */ false,
            /* is_catastrophic= */ false,
        );
    }
}
