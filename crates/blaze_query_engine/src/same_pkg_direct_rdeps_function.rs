//! Port of `query2/engine/SamePkgDirectRdepsFunction.java`.

use crate::callback::{Callback, callback_fn};
use crate::query_environment::{
    Argument, ArgumentType, QueryEnvironment, QueryFunction, QueryFunctionBase,
};
use crate::query_expression::QueryExpression;
use crate::query_expression_context::QueryExpressionContext;
use crate::query_task_future::QueryTaskFuture;

/// A "same_pkg_direct_rdeps" query expression, which computes all of the targets in the same
/// package of the given targets which directly depend on them.
///
/// `expr ::= SAME_PKG_DIRECT_RDEPS '(' expr ')'`
// Java: com/google/devtools/build/lib/query2/engine/SamePkgDirectRdepsFunction.java:35 SamePkgDirectRdepsFunction
#[derive(Debug, Default)]
pub struct SamePkgDirectRdepsFunction;

// Java: com/google/devtools/build/lib/query2/engine/SamePkgDirectRdepsFunction.java:36 SamePkgDirectRdepsFunction.DEPTH_ONE
pub const DEPTH_ONE: Option<i32> = Some(1);

impl SamePkgDirectRdepsFunction {
    pub fn new() -> Self {
        SamePkgDirectRdepsFunction
    }
}

impl QueryFunctionBase for SamePkgDirectRdepsFunction {
    // Java: com/google/devtools/build/lib/query2/engine/SamePkgDirectRdepsFunction.java:39 SamePkgDirectRdepsFunction.getName
    fn get_name(&self) -> String {
        "same_pkg_direct_rdeps".to_owned()
    }

    // Java: com/google/devtools/build/lib/query2/engine/SamePkgDirectRdepsFunction.java:44 SamePkgDirectRdepsFunction.getMandatoryArguments
    fn get_mandatory_arguments(&self) -> i32 {
        1
    }

    // Java: com/google/devtools/build/lib/query2/engine/SamePkgDirectRdepsFunction.java:49 SamePkgDirectRdepsFunction.getArgumentTypes
    fn get_argument_types(&self) -> Vec<ArgumentType> {
        vec![ArgumentType::Expression]
    }
}

impl<T: Clone + 'static> QueryFunction<T> for SamePkgDirectRdepsFunction {
    // Java: com/google/devtools/build/lib/query2/engine/SamePkgDirectRdepsFunction.java:54 SamePkgDirectRdepsFunction.eval
    fn eval(
        &self,
        env: &dyn QueryEnvironment<T>,
        context: &QueryExpressionContext<T>,
        expression: &QueryExpression,
        args: &[Argument],
        callback: &dyn Callback<T>,
    ) -> QueryTaskFuture<()> {
        if let Some(custom_env) = env.as_custom_function_query_environment() {
            return env.eval(
                args[0].get_expression(),
                context,
                &callback_fn(|partial_result: &[T]| {
                    custom_env.same_pkg_direct_rdeps(partial_result, expression, callback)
                }),
            );
        }
        let uniquifier = env.create_uniquifier();
        env.eval(
            args[0].get_expression(),
            context,
            &callback_fn(|partial_result: &[T]| {
                for target in partial_result {
                    let siblings = env.create_thread_safe_mutable_set();
                    siblings.add_all(&env.get_sibling_targets_in_package(target)?);
                    env.build_transitive_closure(expression, &*siblings, DEPTH_ONE)?;
                    let rdeps = env.get_reverse_deps(std::slice::from_ref(target), context)?;
                    let filtered: Vec<T> =
                        rdeps.into_iter().filter(|t| siblings.contains(t)).collect();
                    callback.process(&uniquifier.unique_all(&filtered)?)?;
                }
                Ok(())
            }),
        )
    }
}
