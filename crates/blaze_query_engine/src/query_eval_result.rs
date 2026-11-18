//! Port of `query2/engine/QueryEvalResult.java`.
//!
//! PORT: `DigraphQueryEvalResult<T>` (a `QueryEvalResult` plus a `lib/graph` `Digraph<T>`) is
//! ported with the `Digraph` in blaze_query, next to `BlazeQueryEnvironment`, which builds it.

use std::fmt;

use crate::detailed_exit_code::DetailedExitCode;

/// Information about the query evaluation, like if it was successful and number of elements
/// returned.
// Java: com/google/devtools/build/lib/query2/engine/QueryEvalResult.java:26 QueryEvalResult
#[derive(Debug, Clone, PartialEq)]
pub struct QueryEvalResult {
    success: bool,
    empty: bool,
    detailed_exit_code: DetailedExitCode,
}

impl QueryEvalResult {
    // Java: com/google/devtools/build/lib/query2/engine/QueryEvalResult.java:32 QueryEvalResult.<init>
    pub fn new(success: bool, empty: bool, detailed_exit_code: DetailedExitCode) -> Self {
        blaze_util::check_argument!(
            !success || detailed_exit_code.is_success(),
            "successful query evaluations should not have non-success exit codes. \
             detailedExitCode={:?}",
            detailed_exit_code
        );
        QueryEvalResult {
            success,
            empty,
            detailed_exit_code,
        }
    }

    // Java: com/google/devtools/build/lib/query2/engine/QueryEvalResult.java:43 QueryEvalResult.success
    pub fn success(empty: bool) -> Self {
        Self::new(true, empty, DetailedExitCode::success())
    }

    // Java: com/google/devtools/build/lib/query2/engine/QueryEvalResult.java:47 QueryEvalResult.failure
    pub fn failure(empty: bool, detailed_exit_code: DetailedExitCode) -> Self {
        Self::new(false, empty, detailed_exit_code)
    }

    /// Whether the query was successful. This can only be false if the query was run with
    /// `keep_going`, otherwise evaluation will throw a `QueryException`.
    // Java: com/google/devtools/build/lib/query2/engine/QueryEvalResult.java:55 QueryEvalResult.getSuccess
    pub fn get_success(&self) -> bool {
        self.success
    }

    /// True if the query did not return any result;
    // Java: com/google/devtools/build/lib/query2/engine/QueryEvalResult.java:60 QueryEvalResult.isEmpty
    pub fn is_empty(&self) -> bool {
        self.empty
    }

    /// Returns `DetailedExitCode::success()` if successful, and otherwise a `DetailedExitCode`
    /// describing the failure if unsuccessful.
    // Java: com/google/devtools/build/lib/query2/engine/QueryEvalResult.java:68 QueryEvalResult.getDetailedExitCode
    pub fn get_detailed_exit_code(&self) -> &DetailedExitCode {
        &self.detailed_exit_code
    }
}

impl fmt::Display for QueryEvalResult {
    // Java: com/google/devtools/build/lib/query2/engine/QueryEvalResult.java:73 QueryEvalResult.toString
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}, empty = {}",
            if self.success {
                "Successful"
            } else {
                "Unsuccessful"
            },
            self.empty
        )
    }
}
