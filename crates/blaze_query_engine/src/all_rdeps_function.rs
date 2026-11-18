//! Port of `query2/engine/AllRdepsFunction.java`.

use crate::callback::{Callback, callback_fn};
use crate::query_environment::{
    Argument, ArgumentType, QueryEnvironment, QueryFunction, QueryFunctionBase, should_visit,
};
use crate::query_expression::QueryExpression;
use crate::query_expression_context::QueryExpressionContext;
use crate::query_task_future::QueryTaskFuture;

/// An "allrdeps" query expression, which computes the reverse dependencies of the argument within
/// the currently known universe. An optional integer-literal second argument may be specified;
/// its value bounds the search from the arguments.
///
/// ```text
/// expr ::= ALLRDEPS '(' expr ')'
///        | ALLRDEPS '(' expr ',' WORD ')'
/// ```
// Public because SkyQueryEnvironment needs to refer to it directly.
// Java: com/google/devtools/build/lib/query2/engine/AllRdepsFunction.java:37 AllRdepsFunction
#[derive(Debug, Default)]
pub struct AllRdepsFunction;

impl AllRdepsFunction {
    pub fn new() -> Self {
        AllRdepsFunction
    }

    /// Common non-parallel implementation of depth-bounded allrdeps/deps.
    // Java: com/google/devtools/build/lib/query2/engine/AllRdepsFunction.java:75 AllRdepsFunction.eval
    pub fn eval_in_universe<T: Clone + 'static>(
        env: &dyn QueryEnvironment<T>,
        expression: &QueryExpression,
        universe: &dyn Fn(&T) -> bool,
        context: &QueryExpressionContext<T>,
        callback: &dyn Callback<T>,
        depth: Option<i32>,
    ) -> QueryTaskFuture<()> {
        let min_depth_uniquifier = env.create_min_depth_uniquifier();
        env.eval(
            expression,
            context,
            &callback_fn(|partial_result: &[T]| {
                let mut current: Vec<T> = partial_result.to_vec();
                let mut i = 0;
                while should_visit(depth, {
                    let d = i;
                    i += 1;
                    d
                }) {
                    let mut next: Vec<T> = Vec::new();
                    // Restrict to nodes satisfying the universe predicate.
                    let current_in_universe: Vec<T> =
                        current.iter().filter(|t| universe(*t)).cloned().collect();
                    // Filter already visited nodes: if we see a node in a later round, then we
                    // don't need to visit it again, because the depth at which we see it must be
                    // greater than or equal to the last visit.
                    next.extend(
                        env.get_reverse_deps(
                            &min_depth_uniquifier.unique_at_depth_less_than_or_equal_to_all(
                                &current_in_universe,
                                i,
                            )?,
                            context,
                        )?,
                    );
                    callback.process(&current_in_universe)?;
                    if next.is_empty() {
                        // Exit when there are no more nodes to visit.
                        break;
                    }
                    current = next;
                }
                Ok(())
            }),
        )
    }
}

impl QueryFunctionBase for AllRdepsFunction {
    // Java: com/google/devtools/build/lib/query2/engine/AllRdepsFunction.java:40 AllRdepsFunction.getName
    fn get_name(&self) -> String {
        "allrdeps".to_owned()
    }

    // Java: com/google/devtools/build/lib/query2/engine/AllRdepsFunction.java:45 AllRdepsFunction.getMandatoryArguments
    fn get_mandatory_arguments(&self) -> i32 {
        1 // last argument is optional
    }

    // Java: com/google/devtools/build/lib/query2/engine/AllRdepsFunction.java:50 AllRdepsFunction.getArgumentTypes
    fn get_argument_types(&self) -> Vec<ArgumentType> {
        vec![ArgumentType::Expression, ArgumentType::Integer]
    }
}

impl<T: Clone + 'static> QueryFunction<T> for AllRdepsFunction {
    // Java: com/google/devtools/build/lib/query2/engine/AllRdepsFunction.java:55 AllRdepsFunction.eval
    fn eval(
        &self,
        env: &dyn QueryEnvironment<T>,
        context: &QueryExpressionContext<T>,
        _expression: &QueryExpression,
        args: &[Argument],
        callback: &dyn Callback<T>,
    ) -> QueryTaskFuture<()> {
        let max_depth = if args.len() == 1 {
            None
        } else {
            Some(args[1].get_integer())
        };
        let argument_expression = args[0].get_expression();
        if let Some(streamable_env) = env.as_streamable() {
            match max_depth {
                Some(max_depth) => streamable_env.get_all_rdeps_bounded_parallel(
                    argument_expression,
                    max_depth,
                    context,
                    callback,
                ),
                None => streamable_env.get_all_rdeps_unbounded_parallel(
                    argument_expression,
                    context,
                    callback,
                ),
            }
        } else {
            Self::eval_in_universe(
                env,
                argument_expression,
                &|_: &T| true,
                context,
                callback,
                max_depth,
            )
        }
    }
}
