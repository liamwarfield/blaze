//! Port of `query2/engine/SomePathFunction.java`.

use crate::callback::Callback;
use crate::query_environment::{
    Argument, ArgumentType, QueryEnvironment, QueryFunction, QueryFunctionBase,
};
use crate::query_expression::QueryExpression;
use crate::query_expression_context::QueryExpressionContext;
use crate::query_task_future::{QueryEnvironmentFutures, QueryTaskFuture};
use crate::query_util;

/// A somepath(x, y) query expression, which computes the set of nodes on some arbitrary path from
/// a target in set x to a target in set y.
///
/// `expr ::= SOMEPATH '(' expr ',' expr ')'`
// Java: com/google/devtools/build/lib/query2/engine/SomePathFunction.java:36 SomePathFunction
#[derive(Debug, Default)]
pub struct SomePathFunction;

impl SomePathFunction {
    pub fn new() -> Self {
        SomePathFunction
    }
}

impl QueryFunctionBase for SomePathFunction {
    // Java: com/google/devtools/build/lib/query2/engine/SomePathFunction.java:41 SomePathFunction.getName
    fn get_name(&self) -> String {
        "somepath".to_owned()
    }

    // Java: com/google/devtools/build/lib/query2/engine/SomePathFunction.java:46 SomePathFunction.getMandatoryArguments
    fn get_mandatory_arguments(&self) -> i32 {
        2
    }

    // Java: com/google/devtools/build/lib/query2/engine/SomePathFunction.java:51 SomePathFunction.getArgumentTypes
    fn get_argument_types(&self) -> Vec<ArgumentType> {
        vec![ArgumentType::Expression, ArgumentType::Expression]
    }
}

impl<T: Clone + 'static> QueryFunction<T> for SomePathFunction {
    // Java: com/google/devtools/build/lib/query2/engine/SomePathFunction.java:56 SomePathFunction.eval
    fn eval(
        &self,
        env: &dyn QueryEnvironment<T>,
        context: &QueryExpressionContext<T>,
        expression: &QueryExpression,
        args: &[Argument],
        callback: &dyn Callback<T>,
    ) -> QueryTaskFuture<()> {
        if let Some(streamable_env) = env.as_streamable() {
            return streamable_env.some_path(
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
                custom_env.some_path(
                    &from_value.to_vec(),
                    &to_value.to_vec(),
                    expression,
                    callback,
                )?;
                Ok(())
            });
        }
        env.when_all_succeed_call(&[&from_value_future, &to_value_future], || {
            // Implementation strategy: for each x in "from", compute its forward transitive
            // closure. If it intersects "to", then do a path search from x to an arbitrary node
            // in the intersection, and return the path. This avoids computing the full transitive
            // closure of "from" in some cases.

            let from_value = from_value_future.get_if_successful();
            let to_value = to_value_future.get_if_successful();

            env.build_transitive_closure(expression, &**from_value, None)?;

            for x in from_value.to_vec() {
                // TODO(b/122548314): if x was already seen as part of a previous node's tc, we
                // should skip it here. That's subsumed by the TODO below.
                let x_set = env.create_thread_safe_mutable_set();
                x_set.add(x.clone());
                // TODO(b/122548314): this transitive closure building should stop at any nodes
                // that have already been visited.
                let xtc = env.get_transitive_closure(&*x_set, context)?;
                // Sets.intersection(a, b) iterates a and keeps the elements b contains.
                let result: Option<T> = if xtc.size() > to_value.size() {
                    to_value.to_vec().into_iter().find(|t| xtc.contains(t))
                } else {
                    xtc.to_vec().into_iter().find(|t| to_value.contains(t))
                };
                if let Some(first) = result {
                    callback.process(&env.get_nodes_on_path(&x, &first, context)?)?;
                    return Ok(());
                }
            }
            callback.process(&[])?;
            Ok(())
        })
    }
}
