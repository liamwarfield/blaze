//! Port of `query2/engine/LetExpression.java`.

use std::fmt;

use blaze_protos::failure_details::query;

use crate::callback::Callback;
use crate::query_environment::QueryEnvironment;
use crate::query_exception::QueryException;
use crate::query_expression::QueryExpression;
use crate::query_expression_context::QueryExpressionContext;
use crate::query_task_future::{QueryEnvironmentFutures, QueryTaskFuture};
use crate::query_util;

/// A let expression.
///
/// `expr ::= LET WORD = expr IN expr`
// Java: com/google/devtools/build/lib/query2/engine/LetExpression.java:28 LetExpression
pub struct LetExpression {
    var_name: String,
    var_expr: QueryExpression,
    body_expr: QueryExpression,
}

/// Java `VAR_NAME_PATTERN` = `[a-zA-Z_][a-zA-Z0-9_]*$`, matched against a whole string.
///
/// Variables names may be any legal identifier in the C programming language.
// Java: com/google/devtools/build/lib/query2/engine/LetExpression.java:30 LetExpression.VAR_NAME_PATTERN
fn matches_var_name(s: &str) -> bool {
    let bytes = s.as_bytes();
    match bytes.split_first() {
        None => false,
        Some((first, rest)) => {
            (first.is_ascii_alphabetic() || *first == b'_')
                && rest.iter().all(|b| b.is_ascii_alphanumeric() || *b == b'_')
        }
    }
}

impl LetExpression {
    /// Variable references are prepended with the "$" character. A variable named "x" is
    /// referenced as "$x" (Java `REF_PATTERN.matcher(varName).matches()`).
    // Java: com/google/devtools/build/lib/query2/engine/LetExpression.java:39 LetExpression.isValidVarReference
    pub fn is_valid_var_reference(var_name: &str) -> bool {
        match var_name.strip_prefix('$') {
            Some(rest) => matches_var_name(rest),
            None => false,
        }
    }

    // Java: com/google/devtools/build/lib/query2/engine/LetExpression.java:43 LetExpression.getNameFromReference
    pub fn get_name_from_reference(reference: &str) -> &str {
        // reference.substring(1): the reference starts with the one-byte '$'.
        &reference[1..]
    }

    // Java: com/google/devtools/build/lib/query2/engine/LetExpression.java:51 LetExpression.<init>
    pub fn new(
        var_name: impl Into<String>,
        var_expr: QueryExpression,
        body_expr: QueryExpression,
    ) -> LetExpression {
        LetExpression {
            var_name: var_name.into(),
            var_expr,
            body_expr,
        }
    }

    // Java: com/google/devtools/build/lib/query2/engine/LetExpression.java:57 LetExpression.getVarName
    pub fn get_var_name(&self) -> &str {
        &self.var_name
    }

    // Java: com/google/devtools/build/lib/query2/engine/LetExpression.java:61 LetExpression.getVarExpr
    pub fn get_var_expr(&self) -> &QueryExpression {
        &self.var_expr
    }

    // Java: com/google/devtools/build/lib/query2/engine/LetExpression.java:65 LetExpression.getBodyExpr
    pub fn get_body_expr(&self) -> &QueryExpression {
        &self.body_expr
    }

    /// `this` is the `QueryExpression` wrapping this node.
    // Java: com/google/devtools/build/lib/query2/engine/LetExpression.java:70 LetExpression.eval
    pub fn eval<T: Clone + 'static>(
        &self,
        this: &QueryExpression,
        env: &dyn QueryEnvironment<T>,
        context: &QueryExpressionContext<T>,
        callback: &dyn Callback<T>,
    ) -> QueryTaskFuture<()> {
        if !matches_var_name(&self.var_name) {
            return env.immediate_failed_future(QueryException::new(
                Some(this),
                format!(
                    "invalid variable name '{}' in let expression",
                    self.var_name
                ),
                query::Code::VariableNameInvalid,
            ));
        }
        let var_value_future = query_util::eval_all(env, context, &self.var_expr);
        let eval_body_async_function = |var_value| {
            let body_context = context.with(&self.var_name, var_value);
            env.eval(&self.body_expr, &body_context, callback)
        };
        env.transform_async(var_value_future, eval_body_async_function)
    }

    // Java: com/google/devtools/build/lib/query2/engine/LetExpression.java:92 LetExpression.collectTargetPatterns
    pub fn collect_target_patterns(&self, literals: &mut dyn FnMut(String)) {
        self.var_expr.collect_target_patterns(literals);
        self.body_expr.collect_target_patterns(literals);
    }
}

impl fmt::Display for LetExpression {
    // Java: com/google/devtools/build/lib/query2/engine/LetExpression.java:103 LetExpression.toString
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "let {} = {} in {}",
            self.var_name, self.var_expr, self.body_expr
        )
    }
}
