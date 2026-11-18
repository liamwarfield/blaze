//! Port of `EvaluationResult`.

use crate::{ErrorInfo, Key, SkyError, SkyKey, SkyValue};
use blaze_util::check_state;
use std::fmt;
use std::sync::Arc;

/// The result of a Skyframe `evaluate` call. Will contain all the successfully evaluated values,
/// retrievable through [`EvaluationResult::get`]. As well, the [`ErrorInfo`] for the first value
/// that failed to evaluate (in the non-keep-going case), or any remaining values that failed to
/// evaluate (in the keep-going case) will be retrievable.
///
/// A node can never be successfully evaluated and fail to evaluate. Thus, if `get` returns
/// non-null for some key, there is no stored error for that key, and vice versa.
///
/// PORT: Java's result and error maps are `HashMap`s whose order callers do not rely on; here
/// they are vectors in the order the roots were requested. The walkable graph is the evaluator
/// itself (`Evaluator` implements [`crate::WalkableGraph`]).
// Java: com/google/devtools/build/skyframe/EvaluationResult.java:40 EvaluationResult
pub struct EvaluationResult {
    pub values: Vec<(SkyKey, SkyValue)>,
    pub errors: Vec<(SkyKey, ErrorInfo)>,
    /// Catastrophic error encountered during evaluation, if any.
    pub catastrophe: Option<SkyError>,
}

impl EvaluationResult {
    /// Get a successfully evaluated value.
    // Java: com/google/devtools/build/skyframe/EvaluationResult.java:66 EvaluationResult.get
    pub fn get<K: Key>(&self, key: &K) -> Option<Arc<K::Value>> {
        self.values
            .iter()
            .find(|(k, _)| k.downcast::<K>() == Some(key))
            .and_then(|(_, v)| v.clone().downcast::<K::Value>().ok())
    }

    /// Untyped [`EvaluationResult::get`].
    pub fn get_dyn(&self, key: &SkyKey) -> Option<&SkyValue> {
        self.values.iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }

    /// Whether or not the eval successfully evaluated all requested values. True iff
    /// [`EvaluationResult::get_catastrophe`] or [`EvaluationResult::get_error`] returns non-null.
    // Java: com/google/devtools/build/skyframe/EvaluationResult.java:75 EvaluationResult.hasError
    pub fn has_error(&self) -> bool {
        self.catastrophe.is_some() || !self.errors.is_empty()
    }

    // Java: com/google/devtools/build/skyframe/EvaluationResult.java:84 EvaluationResult.getCatastrophe
    pub fn get_catastrophe(&self) -> Option<&SkyError> {
        self.catastrophe.as_ref()
    }

    /// Returns the errors. Note that the SkyKey is not necessarily the cause of the error -- it is
    /// just the value that was being evaluated when the error was discovered.
    // Java: com/google/devtools/build/skyframe/EvaluationResult.java:101 EvaluationResult.errorMap
    pub fn error_map(&self) -> &[(SkyKey, ErrorInfo)] {
        &self.errors
    }

    /// Returns the [`ErrorInfo`] for `key`, if it is in error.
    // Java: com/google/devtools/build/skyframe/EvaluationResult.java:106 EvaluationResult.getError
    pub fn get_error(&self, key: &SkyKey) -> Option<&ErrorInfo> {
        self.errors.iter().find(|(k, _)| k == key).map(|(_, e)| e)
    }

    /// Returns some error info (Java: `getError()`, the first entry of the error map).
    // Java: com/google/devtools/build/skyframe/EvaluationResult.java:114 EvaluationResult.getError
    pub fn get_first_error(&self) -> Option<&ErrorInfo> {
        self.errors.first().map(|(_, e)| e)
    }

    /// Names of all values that were successfully evaluated.
    // Java: com/google/devtools/build/skyframe/EvaluationResult.java:122 EvaluationResult.keyNames
    pub fn key_names(&self) -> Vec<SkyKey> {
        self.values.iter().map(|(k, _)| k.clone()).collect()
    }

    pub(crate) fn builder() -> EvaluationResultBuilder {
        EvaluationResultBuilder {
            result: EvaluationResult {
                values: vec![],
                errors: vec![],
                catastrophe: None,
            },
        }
    }
}

impl fmt::Debug for EvaluationResult {
    // Java: com/google/devtools/build/skyframe/EvaluationResult.java:142 EvaluationResult.toString
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "EvaluationResult{{catastrophe=")?;
        match &self.catastrophe {
            Some(e) => write!(f, "{e}")?,
            None => f.write_str("null")?,
        }
        write!(f, ", errorMap={{")?;
        for (i, (k, e)) in self.errors.iter().enumerate() {
            if i > 0 {
                f.write_str(", ")?;
            }
            write!(f, "{k:?}={e:?}")?;
        }
        write!(f, "}}, resultMap={{")?;
        for (i, (k, _)) in self.values.iter().enumerate() {
            if i > 0 {
                f.write_str(", ")?;
            }
            // PORT: values are `dyn Any` and have no toString.
            write!(f, "{k:?}=<value>")?;
        }
        f.write_str("}}")
    }
}

/// Builder for [`EvaluationResult`].
// Java: com/google/devtools/build/skyframe/EvaluationResult.java:156 EvaluationResult.Builder
pub(crate) struct EvaluationResultBuilder {
    result: EvaluationResult,
}

impl EvaluationResultBuilder {
    /// Adds a value to the result. An error for this key must not already be present.
    // Java: com/google/devtools/build/skyframe/EvaluationResult.java:165 EvaluationResult.Builder.addResult
    pub(crate) fn add_result(&mut self, key: SkyKey, value: SkyValue) {
        check_state!(
            !self.result.errors.iter().any(|(k, _)| *k == key),
            "{:?} in both result and errors: {:?}",
            key,
            self.result.errors
        );
        self.result.values.push((key, value));
    }

    /// Adds an error to the result. A successful value for this key must not already be present.
    // Java: com/google/devtools/build/skyframe/EvaluationResult.java:179 EvaluationResult.Builder.addError
    pub(crate) fn add_error(&mut self, key: SkyKey, error: ErrorInfo) {
        check_state!(
            !self.result.values.iter().any(|(k, _)| *k == key),
            "{:?} in both result and errors: {:?}",
            error,
            self.result.key_names()
        );
        if error.is_catastrophic() {
            self.result.catastrophe = error.get_exception().cloned();
        }
        match self.result.errors.iter_mut().find(|(k, _)| *k == key) {
            Some(slot) => slot.1 = error,
            None => self.result.errors.push((key, error)),
        }
    }

    // Java: com/google/devtools/build/skyframe/EvaluationResult.java:215 EvaluationResult.Builder.maybeEnsureCatastrophe
    pub(crate) fn maybe_ensure_catastrophe(&mut self, has_catastrophe: bool) {
        if !has_catastrophe || self.result.catastrophe.is_some() {
            return;
        }
        for (_, error_info) in &self.result.errors {
            if let Some(e) = error_info.get_exception() {
                self.result.catastrophe = Some(e.clone());
                return;
            }
        }
        panic!(
            "IllegalStateException: Should have found exception in catastrophe: {:?}",
            self.result.errors
        );
    }

    // Java: com/google/devtools/build/skyframe/EvaluationResult.java:227 EvaluationResult.Builder.isEmpty
    pub(crate) fn is_empty(&self) -> bool {
        self.result.values.is_empty() && self.result.errors.is_empty()
    }

    pub(crate) fn has_error(&self) -> bool {
        self.result.has_error()
    }

    // Java: com/google/devtools/build/skyframe/EvaluationResult.java:205 EvaluationResult.Builder.build
    pub(crate) fn build(self) -> EvaluationResult {
        self.result
    }
}
