//! Port of `query2/engine/VisibleFunction.java`.

use std::sync::Arc;

use crate::callback::{Callback, callback_fn};
use crate::query_environment::{
    Argument, ArgumentType, FilteringQueryFunction, QueryEnvironment, QueryFunction,
    QueryFunctionBase, ThreadSafeMutableSet,
};
use crate::query_exception::QueryError;
use crate::query_expression::QueryExpression;
use crate::query_expression_context::QueryExpressionContext;
use crate::query_task_future::{QueryEnvironmentFutures, QueryTaskFuture};
use crate::query_util;

/// A visible(x, y) query expression, which computes the subset of nodes in y that are visible
/// from all nodes in x.
///
/// `expr ::= VISIBILE '(' expr ',' expr ')'`
///
/// Example: return targets from the package //bar/baz that are visible to //foo.
///
/// ```text
/// visible(//foo, //bar/baz:*)
/// ```
// Java: com/google/devtools/build/lib/query2/engine/VisibleFunction.java:39 VisibleFunction
#[derive(Debug, Default, Clone, Copy)]
pub struct VisibleFunction {
    invert: bool,
}

impl VisibleFunction {
    // Java: com/google/devtools/build/lib/query2/engine/VisibleFunction.java:43 VisibleFunction.<init>
    pub fn new() -> Self {
        VisibleFunction { invert: false }
    }

    /// Returns true if `target` is visible to all targets in `to_set`.
    // Java: com/google/devtools/build/lib/query2/engine/VisibleFunction.java:101 VisibleFunction.visibleToAll
    fn visible_to_all<T: Clone>(
        caller: &QueryExpression,
        env: &dyn QueryEnvironment<T>,
        to_set: &dyn ThreadSafeMutableSet<T>,
        target: &T,
    ) -> Result<bool, QueryError> {
        for to in to_set.to_vec() {
            if !Self::visible(caller, env, &to, target)? {
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// Returns true if the target `from` is visible to the target `to`.
    // Java: com/google/devtools/build/lib/query2/engine/VisibleFunction.java:113 VisibleFunction.visible
    pub fn visible<T>(
        caller: &QueryExpression,
        env: &dyn QueryEnvironment<T>,
        to: &T,
        from: &T,
    ) -> Result<bool, QueryError> {
        let visible_packages = env.get_accessor().get_visibility(caller, from)?;
        for spec in &visible_packages {
            if spec.contains(to) {
                return Ok(true);
            }
        }
        Ok(false)
    }
}

impl QueryFunctionBase for VisibleFunction {
    // Java: com/google/devtools/build/lib/query2/engine/VisibleFunction.java:57 VisibleFunction.getName
    fn get_name(&self) -> String {
        format!("{}visible", if self.invert { "no" } else { "" })
    }

    // Java: com/google/devtools/build/lib/query2/engine/VisibleFunction.java:62 VisibleFunction.getMandatoryArguments
    fn get_mandatory_arguments(&self) -> i32 {
        2
    }

    // Java: com/google/devtools/build/lib/query2/engine/VisibleFunction.java:72 VisibleFunction.getArgumentTypes
    fn get_argument_types(&self) -> Vec<ArgumentType> {
        vec![ArgumentType::Expression, ArgumentType::Expression]
    }
}

impl<T: Clone + 'static> QueryFunction<T> for VisibleFunction {
    // Java: com/google/devtools/build/lib/query2/engine/VisibleFunction.java:77 VisibleFunction.eval
    fn eval(
        &self,
        env: &dyn QueryEnvironment<T>,
        context: &QueryExpressionContext<T>,
        expression: &QueryExpression,
        args: &[Argument],
        callback: &dyn Callback<T>,
    ) -> QueryTaskFuture<()> {
        let to_set_future = query_util::eval_all(env, context, args[0].get_expression());
        let invert = self.invert;
        let compute_visible_nodes_async_function = |to_set: Arc<dyn ThreadSafeMutableSet<T>>| {
            env.eval(
                args[1].get_expression(),
                context,
                &callback_fn(|partial_result: &[T]| {
                    for t in partial_result {
                        if invert ^ Self::visible_to_all(expression, env, &*to_set, t)? {
                            callback.process(std::slice::from_ref(t))?;
                        }
                    }
                    Ok(())
                }),
            )
        };
        env.transform_async(to_set_future, compute_visible_nodes_async_function)
    }

    fn as_filtering_function(&self) -> Option<&dyn FilteringQueryFunction<T>> {
        Some(self)
    }
}

impl<T: Clone + 'static> FilteringQueryFunction<T> for VisibleFunction {
    // Java: com/google/devtools/build/lib/query2/engine/VisibleFunction.java:52 VisibleFunction.invert
    fn invert(&self) -> Arc<dyn FilteringQueryFunction<T>> {
        Arc::new(VisibleFunction {
            invert: !self.invert,
        })
    }

    // Java: com/google/devtools/build/lib/query2/engine/VisibleFunction.java:67 VisibleFunction.getExpressionToFilterIndex
    fn get_expression_to_filter_index(&self) -> i32 {
        1
    }
}
