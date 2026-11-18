//! Port of `query2/engine/RdepsFunction.java`.

use crate::all_rdeps_function::AllRdepsFunction;
use crate::callback::Callback;
use crate::query_environment::{
    Argument, ArgumentType, QueryEnvironment, QueryFunction, QueryFunctionBase,
};
use crate::query_exception::QueryError;
use crate::query_expression::QueryExpression;
use crate::query_expression_context::QueryExpressionContext;
use crate::query_task_future::{QueryEnvironmentFutures, QueryTaskFuture};
use crate::query_util;

/// An "rdeps" query expression, which computes the reverse dependencies of the argument within
/// the transitive closure of the universe. An optional integer-literal third argument may be
/// specified; its value bounds the search from the arguments.
///
/// ```text
/// expr ::= RDEPS '(' expr ',' expr ')'
///        | RDEPS '(' expr ',' expr ',' WORD ')'
/// ```
///
/// PORT: Java's `RdepsFunction extends AllRdepsFunction`; the inherited `getMandatoryArguments`
/// and `getArgumentTypes` are reached through an `AllRdepsFunction` value.
// Java: com/google/devtools/build/lib/query2/engine/RdepsFunction.java:36 RdepsFunction
#[derive(Debug, Default)]
pub struct RdepsFunction {
    super_: AllRdepsFunction,
}

impl RdepsFunction {
    // Java: com/google/devtools/build/lib/query2/engine/RdepsFunction.java:37 RdepsFunction.<init>
    pub fn new() -> Self {
        RdepsFunction {
            super_: AllRdepsFunction::new(),
        }
    }

    /// Compute the transitive closure of the universe, then breadth-first search from the
    /// argument towards the universe while staying within the transitive closure.
    // Java: com/google/devtools/build/lib/query2/engine/RdepsFunction.java:82 RdepsFunction.evalWithBoundedDepth
    fn eval_with_bounded_depth<T: Clone + 'static>(
        env: &dyn QueryEnvironment<T>,
        rdeps_function_expression_for_error_messages: &QueryExpression,
        context: &QueryExpressionContext<T>,
        argument_expression: &QueryExpression,
        depth: Option<i32>,
        universe_expression: &QueryExpression,
        callback: &dyn Callback<T>,
    ) -> QueryTaskFuture<()> {
        let universe_value_future = query_util::eval_all(env, context, universe_expression);

        if let Some(custom_env) = env.as_custom_function_query_environment() {
            let from_value_future = query_util::eval_all(env, context, argument_expression);
            return env.when_all_succeed_call(
                &[&from_value_future, &universe_value_future],
                || {
                    let from_value = from_value_future.get_if_successful();
                    let universe_value = universe_value_future.get_if_successful();
                    custom_env.rdeps(
                        &from_value.to_vec(),
                        &universe_value.to_vec(),
                        depth,
                        rdeps_function_expression_for_error_messages,
                        callback,
                    )?;
                    Ok(())
                },
            );
        }

        let eval_in_universe_async_function = |universe_value: std::sync::Arc<
            dyn crate::query_environment::ThreadSafeMutableSet<T>,
        >| {
            let universe = match (|| -> Result<_, QueryError> {
                env.build_transitive_closure(
                    rdeps_function_expression_for_error_messages,
                    &*universe_value,
                    None,
                )?;
                Ok(env.get_transitive_closure(&*universe_value, context)?)
            })() {
                Ok(universe) => universe,
                Err(QueryError::Interrupted(_)) => return env.immediate_cancelled_future(),
                Err(QueryError::Query(e)) => return env.immediate_failed_future(e),
                Err(QueryError::Cancellation) => {
                    panic!("java.util.concurrent.CancellationException")
                }
            };
            // Predicates.in(set)
            let universe_predicate = |t: &T| universe.contains(t);

            AllRdepsFunction::eval_in_universe(
                env,
                argument_expression,
                &universe_predicate,
                context,
                callback,
                depth,
            )
        };
        env.transform_async(universe_value_future, eval_in_universe_async_function)
    }
}

impl QueryFunctionBase for RdepsFunction {
    // Java: com/google/devtools/build/lib/query2/engine/RdepsFunction.java:40 RdepsFunction.getName
    fn get_name(&self) -> String {
        "rdeps".to_owned()
    }

    // Java: com/google/devtools/build/lib/query2/engine/RdepsFunction.java:45 RdepsFunction.getMandatoryArguments
    fn get_mandatory_arguments(&self) -> i32 {
        self.super_.get_mandatory_arguments() + 1 // +1 for the universe.
    }

    // Java: com/google/devtools/build/lib/query2/engine/RdepsFunction.java:50 RdepsFunction.getArgumentTypes
    fn get_argument_types(&self) -> Vec<ArgumentType> {
        let mut types = vec![ArgumentType::Expression];
        types.extend(self.super_.get_argument_types());
        types
    }
}

impl<T: Clone + 'static> QueryFunction<T> for RdepsFunction {
    // Java: com/google/devtools/build/lib/query2/engine/RdepsFunction.java:56 RdepsFunction.eval
    fn eval(
        &self,
        env: &dyn QueryEnvironment<T>,
        context: &QueryExpressionContext<T>,
        expression: &QueryExpression,
        args: &[Argument],
        callback: &dyn Callback<T>,
    ) -> QueryTaskFuture<()> {
        let depth = if args.len() == 2 {
            None
        } else {
            Some(args[2].get_integer())
        };
        let universe_expression = args[0].get_expression();
        let argument_expression = args[1].get_expression();
        if let Some(streamable_env) = env.as_streamable() {
            match depth {
                Some(depth) => streamable_env.get_rdeps_bounded_parallel(
                    argument_expression,
                    depth,
                    universe_expression,
                    context,
                    callback,
                ),
                None => streamable_env.get_rdeps_unbounded_parallel(
                    argument_expression,
                    universe_expression,
                    context,
                    callback,
                ),
            }
        } else {
            Self::eval_with_bounded_depth(
                env,
                expression,
                context,
                argument_expression,
                depth,
                universe_expression,
                callback,
            )
        }
    }
}
