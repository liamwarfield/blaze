//! Port of `query2/engine/SomeFunction.java`.

use blaze_protos::failure_details::query;

use crate::callback::{Callback, callback_fn};
use crate::query_environment::{
    Argument, ArgumentType, QueryEnvironment, QueryFunction, QueryFunctionBase,
};
use crate::query_exception::{QueryError, QueryException};
use crate::query_expression::QueryExpression;
use crate::query_expression_context::QueryExpressionContext;
use crate::query_task_future::{QueryEnvironmentFutures, QueryTaskFuture};

/// A some(x) filter expression, which returns certain number of arbitrary nodes in set x, or
/// fails if x is empty. An optional integer-literal second argument may be specified; it
/// specifies number of arbitrary nodes to be returned. If second argument is empty, return one
/// node.
///
/// ```text
/// expr ::= SOME '(' expr ')'
///        | SOME '(' expr ',' count ')'
/// ```
// Java: com/google/devtools/build/lib/query2/engine/SomeFunction.java:38 SomeFunction
#[derive(Debug, Default)]
pub struct SomeFunction;

impl SomeFunction {
    pub fn new() -> Self {
        SomeFunction
    }
}

impl QueryFunctionBase for SomeFunction {
    // Java: com/google/devtools/build/lib/query2/engine/SomeFunction.java:42 SomeFunction.getName
    fn get_name(&self) -> String {
        "some".to_owned()
    }

    // Java: com/google/devtools/build/lib/query2/engine/SomeFunction.java:47 SomeFunction.getMandatoryArguments
    fn get_mandatory_arguments(&self) -> i32 {
        1 // last argument is optional
    }

    // Java: com/google/devtools/build/lib/query2/engine/SomeFunction.java:52 SomeFunction.getArgumentTypes
    fn get_argument_types(&self) -> Vec<ArgumentType> {
        vec![ArgumentType::Expression, ArgumentType::Integer]
    }
}

impl<T: Clone + 'static> QueryFunction<T> for SomeFunction {
    // Java: com/google/devtools/build/lib/query2/engine/SomeFunction.java:57 SomeFunction.eval
    fn eval(
        &self,
        env: &dyn QueryEnvironment<T>,
        context: &QueryExpressionContext<T>,
        expression: &QueryExpression,
        args: &[Argument],
        callback: &dyn Callback<T>,
    ) -> QueryTaskFuture<()> {
        // Add a second optional integer parameter indicating return size.
        let result_max_size = if args.len() > 1 {
            args[1].get_integer()
        } else {
            1
        };

        // Since the callback will be executed multiple times, so we need to avoid some target
        // being added multiple times in different callback execution. So we need to have a state
        // variable (`targetsSet` below) to track which ones are already added in order to avoid
        // adding duplicates.
        let targets_set = env.create_thread_safe_mutable_set();
        let targets_set_lock = std::sync::Mutex::new(());

        let evaluate_expression = env.create_evaluate_expression(args[0].get_expression(), context);
        let query_task_future = evaluate_expression.eval(&callback_fn(|partial_result: &[T]| {
            if partial_result.is_empty() {
                return Ok(());
            }

            let mut should_cancel = false;
            {
                // synchronized (targetsSet)
                let _guard = targets_set_lock.lock().unwrap();
                let mut current = Vec::new();
                for next_target in partial_result {
                    if targets_set.size() >= result_max_size {
                        break;
                    }
                    if targets_set.add(next_target.clone()) {
                        current.push(next_target.clone());
                    }
                }

                if !current.is_empty() {
                    callback.process(&current)?;
                }

                if targets_set.size() >= result_max_size {
                    should_cancel = true;
                }
            }

            if should_cancel {
                let _unused = evaluate_expression.gracefully_cancel();
            }
            Ok(())
        }));

        env.when_succeeds_or_is_cancelled_call(&query_task_future, || {
            if evaluate_expression.is_ungracefully_cancelled() {
                return Err(QueryError::Cancellation);
            }

            if targets_set.is_empty() {
                return Err(QueryException::new(
                    Some(expression),
                    "argument set is empty",
                    query::Code::ArgumentsMissing,
                )
                .into());
            }
            Ok(())
        })
    }
}
