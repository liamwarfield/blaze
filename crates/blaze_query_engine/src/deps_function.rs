//! Port of `query2/engine/DepsFunction.java`.

use crate::callback::{Callback, callback_fn};
use crate::query_environment::{
    Argument, ArgumentType, QueryEnvironment, QueryFunction, QueryFunctionBase, should_visit,
};
use crate::query_expression::QueryExpression;
use crate::query_expression_context::QueryExpressionContext;
use crate::query_task_future::QueryTaskFuture;

/// A "deps" query expression, which computes the dependencies of the argument. An optional
/// integer-literal second argument may be specified; its value bounds the search from the
/// arguments.
///
/// ```text
/// expr ::= DEPS '(' expr ')'
///        | DEPS '(' expr ',' WORD ')'
/// ```
// Java: com/google/devtools/build/lib/query2/engine/DepsFunction.java:37 DepsFunction
#[derive(Debug, Default)]
pub struct DepsFunction;

impl DepsFunction {
    pub fn new() -> Self {
        DepsFunction
    }
}

impl QueryFunctionBase for DepsFunction {
    // Java: com/google/devtools/build/lib/query2/engine/DepsFunction.java:41 DepsFunction.getName
    fn get_name(&self) -> String {
        "deps".to_owned()
    }

    // Java: com/google/devtools/build/lib/query2/engine/DepsFunction.java:46 DepsFunction.getMandatoryArguments
    fn get_mandatory_arguments(&self) -> i32 {
        1 // last argument is optional
    }

    // Java: com/google/devtools/build/lib/query2/engine/DepsFunction.java:51 DepsFunction.getArgumentTypes
    fn get_argument_types(&self) -> Vec<ArgumentType> {
        vec![ArgumentType::Expression, ArgumentType::Integer]
    }
}

impl<T: Clone + 'static> QueryFunction<T> for DepsFunction {
    /// Breadth-first search from the arguments.
    // Java: com/google/devtools/build/lib/query2/engine/DepsFunction.java:58 DepsFunction.eval
    fn eval(
        &self,
        env: &dyn QueryEnvironment<T>,
        context: &QueryExpressionContext<T>,
        expression: &QueryExpression,
        args: &[Argument],
        callback: &dyn Callback<T>,
    ) -> QueryTaskFuture<()> {
        let query_expression = args[0].get_expression();
        let max_depth = if args.len() > 1 {
            Some(args[1].get_integer())
        } else {
            None
        };
        if let Some(streamable_env) = env.as_streamable() {
            return match max_depth {
                Some(max_depth) => streamable_env.get_deps_bounded(
                    query_expression,
                    context,
                    callback,
                    max_depth,
                    expression,
                ),
                None => streamable_env.get_deps_unbounded_parallel(
                    query_expression,
                    context,
                    callback,
                    expression,
                ),
            };
        }

        if let Some(custom_env) = env.as_custom_function_query_environment() {
            // Not all expressions generate a single future (e.g. SetExpression), as such, we
            // should batch them here before the heavy blocking work is done in the callback to
            // deps.
            return custom_env.eval_with_batch(
                query_expression,
                context,
                &callback_fn(|result: &[T]| {
                    custom_env.deps(result, max_depth, expression, callback)
                }),
                /* batch= */ true,
            );
        }

        let min_depth_uniquifier = env.create_min_depth_uniquifier();
        env.eval(
            query_expression,
            context,
            &callback_fn(|partial_result: &[T]| {
                let mut current = env.create_thread_safe_mutable_set();
                current.add_all(partial_result);
                // PORT: Profiler "env.buildTransitiveClosure" skipped (profiling).
                env.build_transitive_closure(expression, &*current, max_depth)?;

                let mut i = 0;
                while should_visit(max_depth, {
                    let d = i;
                    i += 1;
                    d
                }) {
                    // Filter already visited nodes: if we see a node in a later round, then we
                    // don't need to visit it again, because the depth at which we see it at must
                    // be greater than or equal to the last visit.
                    let to_process = min_depth_uniquifier
                        .unique_at_depth_less_than_or_equal_to_all(&current.to_vec(), i)?;
                    callback.process(&to_process)?;
                    current = env.create_thread_safe_mutable_set();
                    // PORT: Profiler "env.getFwdDeps" skipped (profiling).
                    current.add_all(&env.get_fwd_deps(&to_process, context)?);
                    if current.is_empty() {
                        // Exit when there are no more nodes to visit.
                        break;
                    }
                }
                Ok(())
            }),
        )
    }
}
