//! Port of `query2/engine/BinaryOperatorExpression.java`.

use std::fmt;
use std::sync::Arc;

use crate::callback::{Callback, callback_fn};
use crate::lexer::TokenKind;
use crate::query_environment::{QueryEnvironment, ThreadSafeMutableSet};
use crate::query_expression::QueryExpression;
use crate::query_expression_context::QueryExpressionContext;
use crate::query_task_future::{QueryEnvironmentFutures, QueryTaskFuture, as_any_futures};
use crate::query_util;

/// A binary algebraic set operation.
///
/// ```text
/// expr ::= expr (INTERSECT expr)+
///        | expr ('^' expr)+
///        | expr (UNION expr)+
///        | expr ('+' expr)+
///        | expr (EXCEPT expr)+
///        | expr ('-' expr)+
/// ```
// Java: com/google/devtools/build/lib/query2/engine/BinaryOperatorExpression.java:39 BinaryOperatorExpression
pub struct BinaryOperatorExpression {
    /// ::= INTERSECT/CARET | UNION/PLUS | EXCEPT/MINUS
    operator: TokenKind,
    operands: Vec<QueryExpression>,
}

impl BinaryOperatorExpression {
    // Java: com/google/devtools/build/lib/query2/engine/BinaryOperatorExpression.java:44 BinaryOperatorExpression.<init>
    pub fn new(operator: TokenKind, operands: Vec<QueryExpression>) -> BinaryOperatorExpression {
        blaze_util::check_state!(operands.len() > 1);
        BinaryOperatorExpression { operator, operands }
    }

    // Java: com/google/devtools/build/lib/query2/engine/BinaryOperatorExpression.java:50 BinaryOperatorExpression.getOperator
    pub fn get_operator(&self) -> TokenKind {
        self.operator
    }

    // Java: com/google/devtools/build/lib/query2/engine/BinaryOperatorExpression.java:54 BinaryOperatorExpression.getOperands
    pub fn get_operands(&self) -> &[QueryExpression] {
        &self.operands
    }

    // Java: com/google/devtools/build/lib/query2/engine/BinaryOperatorExpression.java:59 BinaryOperatorExpression.eval
    pub fn eval<T: Clone + 'static>(
        &self,
        env: &dyn QueryEnvironment<T>,
        context: &QueryExpressionContext<T>,
        callback: &dyn Callback<T>,
    ) -> QueryTaskFuture<()> {
        match self.operator {
            TokenKind::Plus | TokenKind::Union => {
                Self::eval_plus(&self.operands, env, context, callback)
            }
            TokenKind::Minus | TokenKind::Except => {
                Self::eval_minus(&self.operands, env, context, callback)
            }
            TokenKind::Intersect | TokenKind::Caret => self.eval_intersect(env, context, callback),
            other => panic!("IllegalStateException: {}", other.name()),
        }
    }

    /// Evaluates an expression of the form "e1 + e2 + ... + eK" by evaluating all the
    /// subexpressions separately.
    ///
    /// N.B. `operands.len()` may be `1`.
    // Java: com/google/devtools/build/lib/query2/engine/BinaryOperatorExpression.java:75 BinaryOperatorExpression.evalPlus
    fn eval_plus<T: Clone + 'static>(
        operands: &[QueryExpression],
        env: &dyn QueryEnvironment<T>,
        context: &QueryExpressionContext<T>,
        callback: &dyn Callback<T>,
    ) -> QueryTaskFuture<()> {
        let mut query_tasks = Vec::with_capacity(operands.len());
        for operand in operands {
            query_tasks.push(env.eval(operand, context, callback));
        }
        env.when_all_succeed(&as_any_futures(&query_tasks))
    }

    /// Evaluates an expression of the form "e1 - e2 - ... - eK" by noting its equivalence to
    /// "e1 - (e2 + ... + eK)" and evaluating the subexpressions on the right-hand-side
    /// separately.
    // Java: com/google/devtools/build/lib/query2/engine/BinaryOperatorExpression.java:91 BinaryOperatorExpression.evalMinus
    fn eval_minus<T: Clone + 'static>(
        operands: &[QueryExpression],
        env: &dyn QueryEnvironment<T>,
        context: &QueryExpressionContext<T>,
        callback: &dyn Callback<T>,
    ) -> QueryTaskFuture<()> {
        let lhs_value_future = query_util::eval_all(env, context, &operands[0]);
        let subtract_async_function = |lhs_value: Arc<dyn ThreadSafeMutableSet<T>>| {
            let thread_safe_lhs_value = lhs_value;
            let subtraction_callback = callback_fn(|partial_result: &[T]| {
                for target in partial_result {
                    thread_safe_lhs_value.remove(target);
                }
                Ok(())
            });
            let rhs_evaluated_future =
                Self::eval_plus(&operands[1..], env, context, &subtraction_callback);
            env.when_succeeds_call(&rhs_evaluated_future, || {
                callback.process(&thread_safe_lhs_value.to_vec())
            })
        };
        env.transform_async(lhs_value_future, subtract_async_function)
    }

    // Java: com/google/devtools/build/lib/query2/engine/BinaryOperatorExpression.java:125 BinaryOperatorExpression.evalIntersect
    fn eval_intersect<T: Clone + 'static>(
        &self,
        env: &dyn QueryEnvironment<T>,
        context: &QueryExpressionContext<T>,
        callback: &dyn Callback<T>,
    ) -> QueryTaskFuture<()> {
        // For each right-hand side operand, intersection cannot be performed in a streaming
        // manner; the entire result of that operand is needed. So, in order to avoid pinning too
        // much in memory at once, we process each right-hand side operand one at a time and throw
        // away that operand's result.
        // TODO(bazel-team): Consider keeping just the name / label of the right-hand side results
        // instead of the potentially heavy-weight instances of type T. This would let us process
        // all right-hand side operands in parallel without worrying about memory usage.
        let mut rolling_result_future = query_util::eval_all(env, context, &self.operands[0]);
        for index in 1..self.operands.len() {
            let eval_operand_and_intersect_async_function =
                |rolling_result: Arc<dyn ThreadSafeMutableSet<T>>| {
                    let rhs_operand_value_future =
                        query_util::eval_all(env, context, &self.operands[index]);
                    env.when_succeeds_call(&rhs_operand_value_future, || {
                        rolling_result.retain_all(&**rhs_operand_value_future.get_if_successful());
                        Ok(rolling_result)
                    })
                };
            rolling_result_future = env.transform_async(
                rolling_result_future,
                eval_operand_and_intersect_async_function,
            );
        }
        let result_future = rolling_result_future;
        env.when_succeeds_call(&result_future, || {
            callback.process(&result_future.get_if_successful().to_vec())
        })
    }

    // Java: com/google/devtools/build/lib/query2/engine/BinaryOperatorExpression.java:172 BinaryOperatorExpression.collectTargetPatterns
    pub fn collect_target_patterns(&self, literals: &mut dyn FnMut(String)) {
        for sub_expression in &self.operands {
            sub_expression.collect_target_patterns(literals);
        }
    }
}

impl fmt::Display for BinaryOperatorExpression {
    // Java: com/google/devtools/build/lib/query2/engine/BinaryOperatorExpression.java:184 BinaryOperatorExpression.toString
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "({}", self.operands[0])?;
        for expr in &self.operands[1..] {
            write!(f, " {} {}", self.operator.get_pretty_name(), expr)?;
        }
        f.write_str(")")
    }
}
