//! Port of `query2/engine/BuildFilesFunction.java`.

use std::collections::HashSet;
use std::sync::Mutex;

use crate::callback::{Callback, callback_fn};
use crate::query_environment::{
    Argument, ArgumentType, QueryEnvironment, QueryFunction, QueryFunctionBase,
};
use crate::query_expression::QueryExpression;
use crate::query_expression_context::QueryExpressionContext;
use crate::query_task_future::{QueryEnvironmentFutures, QueryTaskFuture};

/// A buildfiles(x) query expression, which computes the set of BUILD files and subincluded files
/// for each target in set x. The result is unordered. This operator is typically used for
/// determining what files or packages to check out.
///
/// `expr ::= BUILDFILES '(' expr ')'`
// Java: com/google/devtools/build/lib/query2/engine/BuildFilesFunction.java:35 BuildFilesFunction
#[derive(Debug, Default)]
pub struct BuildFilesFunction;

impl BuildFilesFunction {
    pub fn new() -> Self {
        BuildFilesFunction
    }
}

impl QueryFunctionBase for BuildFilesFunction {
    // Java: com/google/devtools/build/lib/query2/engine/BuildFilesFunction.java:40 BuildFilesFunction.getName
    fn get_name(&self) -> String {
        "buildfiles".to_owned()
    }

    // Java: com/google/devtools/build/lib/query2/engine/BuildFilesFunction.java:75 BuildFilesFunction.getMandatoryArguments
    fn get_mandatory_arguments(&self) -> i32 {
        1
    }

    // Java: com/google/devtools/build/lib/query2/engine/BuildFilesFunction.java:80 BuildFilesFunction.getArgumentTypes
    fn get_argument_types(&self) -> Vec<ArgumentType> {
        vec![ArgumentType::Expression]
    }
}

impl<T: Clone + 'static> QueryFunction<T> for BuildFilesFunction {
    // Java: com/google/devtools/build/lib/query2/engine/BuildFilesFunction.java:45 BuildFilesFunction.eval
    fn eval(
        &self,
        env: &dyn QueryEnvironment<T>,
        context: &QueryExpressionContext<T>,
        _expression: &QueryExpression,
        args: &[Argument],
        callback: &dyn Callback<T>,
    ) -> QueryTaskFuture<()> {
        let seen_packages = Mutex::new(HashSet::new());
        let seen_bzl_labels = Mutex::new(HashSet::new());
        let uniquifier = env.create_uniquifier();
        let helper = match env.get_transitive_load_files_helper() {
            Ok(helper) => helper,
            Err(e) => return env.immediate_failed_future(e),
        };
        env.eval(
            args[0].get_expression(),
            context,
            &callback_fn(|partial_result: &[T]| {
                env.transitive_load_files(
                    partial_result,
                    /* also_add_build_files= */ true,
                    &seen_packages,
                    &seen_bzl_labels,
                    &*uniquifier,
                    &*helper,
                    callback,
                )
            }),
        )
    }
}
