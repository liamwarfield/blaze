//! Port of `query2/engine/QueryException.java`, plus the error types the engine's `throws`
//! clauses map to.

use std::fmt;
use std::sync::Arc;

use blaze_protos::failure_details::{
    ActionQuery, ConfigurableQuery, FailureDetail, Query, action_query, configurable_query,
    failure_detail::Category, query,
};

use crate::query_expression::QueryExpression;

/// Exception indicating a failure in Blaze query, aquery, or cquery.
// Java: com/google/devtools/build/lib/query2/engine/QueryException.java:24 QueryException
#[derive(Clone)]
pub struct QueryException {
    message: String,
    /// The subexpression for which evaluation failed, or `None` if the failure occurred during
    /// lexing/parsing.
    expression: Option<QueryExpression>,
    failure_detail: FailureDetail,
    cause: Option<Arc<dyn std::error::Error + Send + Sync>>,
}

impl QueryException {
    /// Returns a better error message for the query.
    // Java: com/google/devtools/build/lib/query2/engine/QueryException.java:27 QueryException.describeFailedQuery
    pub(crate) fn describe_failed_query(e: &QueryException, toplevel: &QueryExpression) -> String {
        let bad_query = e.get_failed_expression();
        match bad_query {
            None => format!("Evaluation failed: {}", e.get_message()),
            Some(bad_query) => {
                if bad_query.ptr_eq(toplevel) {
                    format!(
                        "Evaluation of query \"{}\" failed: {}",
                        toplevel.to_trunctated_string(),
                        e.get_message()
                    )
                } else {
                    format!(
                        "Evaluation of subquery \"{}\" failed (did you want to use --keep_going?): {}",
                        bad_query.to_trunctated_string(),
                        e.get_message()
                    )
                }
            }
        }
    }

    /// Java `QueryException(QueryException e, QueryExpression toplevel)`.
    // Java: com/google/devtools/build/lib/query2/engine/QueryException.java:43 QueryException.<init>
    pub fn wrap_toplevel(e: QueryException, toplevel: &QueryExpression) -> QueryException {
        QueryException {
            message: Self::describe_failed_query(&e, toplevel),
            expression: None,
            failure_detail: e.get_failure_detail().clone(),
            cause: Some(Arc::new(e)),
        }
    }

    /// Java `QueryException(QueryExpression, String, Throwable, FailureDetail)` and
    /// `QueryException(String, Throwable, FailureDetail)` (with `expression == None`).
    // Java: com/google/devtools/build/lib/query2/engine/QueryException.java:49 QueryException.<init>
    pub fn with_cause(
        expression: Option<&QueryExpression>,
        message: impl Into<String>,
        cause: Arc<dyn std::error::Error + Send + Sync>,
        failure_detail: FailureDetail,
    ) -> QueryException {
        QueryException {
            message: message.into(),
            expression: expression.cloned(),
            failure_detail,
            cause: Some(cause),
        }
    }

    /// Java `QueryException(QueryExpression, String, FailureDetail)` and
    /// `QueryException(String, FailureDetail)` (with `expression == None`).
    // Java: com/google/devtools/build/lib/query2/engine/QueryException.java:56 QueryException.<init>
    pub fn with_failure_detail(
        expression: Option<&QueryExpression>,
        message: impl Into<String>,
        failure_detail: FailureDetail,
    ) -> QueryException {
        QueryException {
            message: message.into(),
            expression: expression.cloned(),
            failure_detail,
            cause: None,
        }
    }

    /// Java `QueryException(QueryExpression, String, Query.Code)` and
    /// `QueryException(String, Query.Code)` (with `expression == None`).
    // Java: com/google/devtools/build/lib/query2/engine/QueryException.java:62 QueryException.<init>
    pub fn new(
        expression: Option<&QueryExpression>,
        message: impl Into<String>,
        query_code: query::Code,
    ) -> QueryException {
        let message = message.into();
        let failure_detail = FailureDetail {
            message: message.clone(),
            category: Some(Category::Query(Query {
                code: query_code as i32,
            })),
        };
        Self::with_failure_detail(expression, message, failure_detail)
    }

    /// Java `QueryException(QueryExpression, String, ActionQuery.Code)` and
    /// `QueryException(String, ActionQuery.Code)`.
    // Java: com/google/devtools/build/lib/query2/engine/QueryException.java:72 QueryException.<init>
    pub fn with_action_query_code(
        expression: Option<&QueryExpression>,
        message: impl Into<String>,
        action_query_code: action_query::Code,
    ) -> QueryException {
        let message = message.into();
        let failure_detail = FailureDetail {
            message: message.clone(),
            category: Some(Category::ActionQuery(ActionQuery {
                code: action_query_code as i32,
            })),
        };
        Self::with_failure_detail(expression, message, failure_detail)
    }

    /// Java `QueryException(QueryExpression, String, ConfigurableQuery.Code)` and
    /// `QueryException(String, ConfigurableQuery.Code)`.
    // Java: com/google/devtools/build/lib/query2/engine/QueryException.java:83 QueryException.<init>
    pub fn with_configurable_query_code(
        expression: Option<&QueryExpression>,
        message: impl Into<String>,
        configurable_query_code: configurable_query::Code,
    ) -> QueryException {
        let message = message.into();
        let failure_detail = FailureDetail {
            message: message.clone(),
            category: Some(Category::ConfigurableQuery(ConfigurableQuery {
                code: configurable_query_code as i32,
            })),
        };
        Self::with_failure_detail(expression, message, failure_detail)
    }

    /// Java `getMessage()`.
    pub fn get_message(&self) -> &str {
        &self.message
    }

    /// Java `getCause()`.
    pub fn get_cause(&self) -> Option<&Arc<dyn std::error::Error + Send + Sync>> {
        self.cause.as_ref()
    }

    /// Returns the subexpression for which evaluation failed, or null if the failure occurred
    /// during lexing/parsing.
    // Java: com/google/devtools/build/lib/query2/engine/QueryException.java:123 QueryException.getFailedExpression
    pub fn get_failed_expression(&self) -> Option<&QueryExpression> {
        self.expression.as_ref()
    }

    /// Returns a `FailureDetail` with a corresponding code of the query error.
    // Java: com/google/devtools/build/lib/query2/engine/QueryException.java:128 QueryException.getFailureDetail
    pub fn get_failure_detail(&self) -> &FailureDetail {
        &self.failure_detail
    }
}

impl fmt::Display for QueryException {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl fmt::Debug for QueryException {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("QueryException")
            .field("message", &self.message)
            .field(
                "expression",
                &self.expression.as_ref().map(|e| e.to_string()),
            )
            .field("failure_detail", &self.failure_detail)
            .finish()
    }
}

impl std::error::Error for QueryException {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.cause
            .as_ref()
            .map(|c| c.as_ref() as &(dyn std::error::Error + 'static))
    }
}

/// Java `InterruptedException` as thrown and caught by the query engine.
///
/// PORT: the engine does not depend on `blaze_skyframe`; callers convert `blaze_skyframe`'s
/// `Interrupted` into this type at the boundary.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InterruptedException {
    message: Option<String>,
}

impl InterruptedException {
    /// Java `new InterruptedException()`.
    pub fn new() -> InterruptedException {
        InterruptedException { message: None }
    }

    /// Java `new InterruptedException(message)`.
    pub fn with_message(message: impl Into<String>) -> InterruptedException {
        InterruptedException {
            message: Some(message.into()),
        }
    }

    /// Java `getMessage()` (null without a message).
    pub fn get_message(&self) -> Option<&str> {
        self.message.as_deref()
    }
}

impl fmt::Display for InterruptedException {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Java's Throwable.toString().
        match &self.message {
            Some(message) => write!(f, "java.lang.InterruptedException: {message}"),
            None => f.write_str("java.lang.InterruptedException"),
        }
    }
}

impl std::error::Error for InterruptedException {}

/// What a Java method declared `throws QueryException, InterruptedException` can throw.
///
/// `Cancellation` is Java's unchecked `java.util.concurrent.CancellationException`, which
/// `SomeFunction` throws from a `QueryTaskCallable` to cancel the resulting future. Code that does
/// not handle it propagates it, as Java propagates the unchecked exception.
#[derive(Debug, Clone, thiserror::Error)]
pub enum QueryError {
    #[error(transparent)]
    Query(#[from] QueryException),
    #[error(transparent)]
    Interrupted(#[from] InterruptedException),
    #[error("CancellationException")]
    Cancellation,
}
