//! Port of `query2/engine/SiblingsFunction.java`.

use std::collections::HashSet;
use std::sync::Mutex;

use crate::callback::{Callback, callback_fn};
use crate::query_environment::{
    Argument, ArgumentType, QueryEnvironment, QueryFunction, QueryFunctionBase,
};
use crate::query_expression::QueryExpression;
use crate::query_expression_context::QueryExpressionContext;
use crate::query_task_future::QueryTaskFuture;

/// A "siblings" query expression, which computes all of the targets in all of the packages of all
/// the targets to which the argument evaluates.
///
/// `expr ::= SIBLINGS '(' expr ')'`
// Java: com/google/devtools/build/lib/query2/engine/SiblingsFunction.java:32 SiblingsFunction
#[derive(Debug, Default)]
pub struct SiblingsFunction;

impl SiblingsFunction {
    pub fn new() -> Self {
        SiblingsFunction
    }
}

impl QueryFunctionBase for SiblingsFunction {
    // Java: com/google/devtools/build/lib/query2/engine/SiblingsFunction.java:34 SiblingsFunction.getName
    fn get_name(&self) -> String {
        "siblings".to_owned()
    }

    // Java: com/google/devtools/build/lib/query2/engine/SiblingsFunction.java:39 SiblingsFunction.getMandatoryArguments
    fn get_mandatory_arguments(&self) -> i32 {
        1
    }

    // Java: com/google/devtools/build/lib/query2/engine/SiblingsFunction.java:44 SiblingsFunction.getArgumentTypes
    fn get_argument_types(&self) -> Vec<ArgumentType> {
        vec![ArgumentType::Expression]
    }
}

impl<T: Clone + 'static> QueryFunction<T> for SiblingsFunction {
    // Java: com/google/devtools/build/lib/query2/engine/SiblingsFunction.java:49 SiblingsFunction.eval
    fn eval(
        &self,
        env: &dyn QueryEnvironment<T>,
        context: &QueryExpressionContext<T>,
        _expression: &QueryExpression,
        args: &[Argument],
        callback: &dyn Callback<T>,
    ) -> QueryTaskFuture<()> {
        let target_accessor = env.get_accessor();
        let package_names: Mutex<HashSet<String>> = Mutex::new(HashSet::new());
        env.eval(
            args[0].get_expression(),
            context,
            &callback_fn(|partial_result: &[T]| {
                for target in partial_result {
                    let added = package_names
                        .lock()
                        .unwrap()
                        .insert(target_accessor.get_package(target));
                    if added {
                        callback.process(&env.get_sibling_targets_in_package(target)?)?;
                    }
                }
                Ok(())
            }),
        )
    }
}
