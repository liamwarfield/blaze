//! Port of `query2/engine/ExecutablesFunction.java`.

use crate::callback::{Callback, callback_fn};
use crate::query_environment::{
    Argument, ArgumentType, QueryEnvironment, QueryFunction, QueryFunctionBase,
};
use crate::query_expression::QueryExpression;
use crate::query_expression_context::QueryExpressionContext;
use crate::query_task_future::QueryTaskFuture;

/// An executables(x) filter expression, which returns all the executables (not including tests)
/// in set x.
///
/// `expr ::= EXECUTABLES '(' expr ')'`
// Java: com/google/devtools/build/lib/query2/engine/ExecutablesFunction.java:32 ExecutablesFunction
#[derive(Debug, Default)]
pub struct ExecutablesFunction;

impl ExecutablesFunction {
    // Java: com/google/devtools/build/lib/query2/engine/ExecutablesFunction.java:34 ExecutablesFunction.<init>
    pub fn new() -> Self {
        ExecutablesFunction
    }
}

impl QueryFunctionBase for ExecutablesFunction {
    // Java: com/google/devtools/build/lib/query2/engine/ExecutablesFunction.java:37 ExecutablesFunction.getName
    fn get_name(&self) -> String {
        "executables".to_owned()
    }

    // Java: com/google/devtools/build/lib/query2/engine/ExecutablesFunction.java:42 ExecutablesFunction.getMandatoryArguments
    fn get_mandatory_arguments(&self) -> i32 {
        1
    }

    // Java: com/google/devtools/build/lib/query2/engine/ExecutablesFunction.java:47 ExecutablesFunction.getArgumentTypes
    fn get_argument_types(&self) -> Vec<ArgumentType> {
        vec![ArgumentType::Expression]
    }
}

impl<T: Clone + 'static> QueryFunction<T> for ExecutablesFunction {
    // Java: com/google/devtools/build/lib/query2/engine/ExecutablesFunction.java:52 ExecutablesFunction.eval
    fn eval(
        &self,
        env: &dyn QueryEnvironment<T>,
        context: &QueryExpressionContext<T>,
        _expression: &QueryExpression,
        args: &[Argument],
        callback: &dyn Callback<T>,
    ) -> QueryTaskFuture<()> {
        let accessor = env.get_accessor();

        env.eval(
            args[0].get_expression(),
            context,
            &callback_fn(|partial_result: &[T]| {
                let filtered: Vec<T> = partial_result
                    .iter()
                    .filter(|t| accessor.is_executable_non_test_rule(*t))
                    .cloned()
                    .collect();
                callback.process(&filtered)
            }),
        )
    }
}
