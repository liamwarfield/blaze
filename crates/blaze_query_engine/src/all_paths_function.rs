//! Port of `query2/engine/AllPathsFunction.java`.

use crate::callback::Callback;
use crate::query_environment::{
    Argument, ArgumentType, QueryEnvironment, QueryFunction, QueryFunctionBase,
    ThreadSafeMutableSet,
};
use crate::query_expression::QueryExpression;
use crate::query_expression_context::QueryExpressionContext;
use crate::query_task_future::{QueryEnvironmentFutures, QueryTaskFuture};
use crate::query_util;

/// Implementation of the `allpaths()` function.
// Java: com/google/devtools/build/lib/query2/engine/AllPathsFunction.java:35 AllPathsFunction
#[derive(Debug, Default)]
pub struct AllPathsFunction;

impl AllPathsFunction {
    pub fn new() -> Self {
        AllPathsFunction
    }

    /// Returns a (new, mutable, unordered) set containing the intersection of the two specified
    /// sets.
    ///
    /// PORT: java-hash-order. Java copies the intersection into a `java.util.HashSet<T>`, whose
    /// order follows `T.hashCode()`; for `Target` that is the identity hash code, so Java's order
    /// is not deterministic. This keeps the order of `Sets.intersection` (the smaller set's
    /// iteration order).
    // Java: com/google/devtools/build/lib/query2/engine/AllPathsFunction.java:119 AllPathsFunction.intersection
    fn intersection<T: Clone>(
        x: &dyn ThreadSafeMutableSet<T>,
        y: &dyn ThreadSafeMutableSet<T>,
    ) -> Vec<T> {
        if x.size() > y.size() {
            y.to_vec().into_iter().filter(|t| x.contains(t)).collect()
        } else {
            x.to_vec().into_iter().filter(|t| y.contains(t)).collect()
        }
    }
}

impl QueryFunctionBase for AllPathsFunction {
    // Java: com/google/devtools/build/lib/query2/engine/AllPathsFunction.java:40 AllPathsFunction.getName
    fn get_name(&self) -> String {
        "allpaths".to_owned()
    }

    // Java: com/google/devtools/build/lib/query2/engine/AllPathsFunction.java:45 AllPathsFunction.getMandatoryArguments
    fn get_mandatory_arguments(&self) -> i32 {
        2
    }

    // Java: com/google/devtools/build/lib/query2/engine/AllPathsFunction.java:50 AllPathsFunction.getArgumentTypes
    fn get_argument_types(&self) -> Vec<ArgumentType> {
        vec![ArgumentType::Expression, ArgumentType::Expression]
    }
}

impl<T: Clone + 'static> QueryFunction<T> for AllPathsFunction {
    // Java: com/google/devtools/build/lib/query2/engine/AllPathsFunction.java:55 AllPathsFunction.eval
    fn eval(
        &self,
        env: &dyn QueryEnvironment<T>,
        context: &QueryExpressionContext<T>,
        expression: &QueryExpression,
        args: &[Argument],
        callback: &dyn Callback<T>,
    ) -> QueryTaskFuture<()> {
        if let Some(streamable_env) = env.as_streamable() {
            return streamable_env.all_paths(
                args[0].get_expression(),
                args[1].get_expression(),
                context,
                callback,
                expression,
            );
        }

        let from_value_future = query_util::eval_all(env, context, args[0].get_expression());
        let to_value_future = query_util::eval_all(env, context, args[1].get_expression());

        if let Some(custom_env) = env.as_custom_function_query_environment() {
            return env.when_all_succeed_call(&[&from_value_future, &to_value_future], || {
                let from_value = from_value_future.get_if_successful();
                let to_value = to_value_future.get_if_successful();
                custom_env.all_paths(
                    &from_value.to_vec(),
                    &to_value.to_vec(),
                    expression,
                    callback,
                )?;
                Ok(())
            });
        }
        env.when_all_succeed_call(&[&from_value_future, &to_value_future], || {
            // Algorithm: compute "reachableFromX", the forward transitive closure of the "from"
            // set, then find the intersection of "reachableFromX" with the reverse transitive
            // closure of the "to" set. The reverse transitive closure and intersection operations
            // are interleaved for efficiency. "result" holds the intersection.

            let from_value = from_value_future.get_if_successful();
            let to_value = to_value_future.get_if_successful();

            env.build_transitive_closure(expression, &**from_value, None)?;

            let reachable_from_x = env.get_transitive_closure(&**from_value, context)?;
            let reachable = |t: &T| reachable_from_x.contains(t);
            let uniquifier = env.create_uniquifier();
            let result =
                uniquifier.unique_all(&Self::intersection(&*reachable_from_x, &**to_value))?;
            callback.process(&result)?;
            let mut worklist = result;
            while !worklist.is_empty() {
                let reverse_deps = env.get_reverse_deps(&worklist, context)?;
                let filtered: Vec<T> = reverse_deps.into_iter().filter(|t| reachable(t)).collect();
                worklist = uniquifier.unique_all(&filtered)?;
                callback.process(&worklist)?;
            }
            Ok(())
        })
    }
}
