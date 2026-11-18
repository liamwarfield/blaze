//! Port of `query2/engine/LabelsFunction.java`.

use crate::callback::{Callback, callback_fn};
use crate::query_environment::{
    Argument, ArgumentType, QueryEnvironment, QueryFunction, QueryFunctionBase,
};
use crate::query_expression::QueryExpression;
use crate::query_expression_context::QueryExpressionContext;
use crate::query_task_future::QueryTaskFuture;

/// A label(attr_name, argument) expression, which computes the set of targets whose labels
/// appear in the specified attribute of some rule in 'argument'.
///
/// `expr ::= LABELS '(' WORD ',' expr ')'`
///
/// Example:
///
/// ```text
///  labels(srcs, //foo)      The 'srcs' source files to the //foo rule.
/// ```
// Java: com/google/devtools/build/lib/query2/engine/LabelsFunction.java:36 LabelsFunction
#[derive(Debug, Default)]
pub struct LabelsFunction;

impl LabelsFunction {
    pub fn new() -> Self {
        LabelsFunction
    }
}

impl QueryFunctionBase for LabelsFunction {
    // Java: com/google/devtools/build/lib/query2/engine/LabelsFunction.java:41 LabelsFunction.getName
    fn get_name(&self) -> String {
        "labels".to_owned()
    }

    // Java: com/google/devtools/build/lib/query2/engine/LabelsFunction.java:46 LabelsFunction.getMandatoryArguments
    fn get_mandatory_arguments(&self) -> i32 {
        2
    }

    // Java: com/google/devtools/build/lib/query2/engine/LabelsFunction.java:51 LabelsFunction.getArgumentTypes
    fn get_argument_types(&self) -> Vec<ArgumentType> {
        vec![ArgumentType::Word, ArgumentType::Expression]
    }
}

impl<T: Clone + 'static> QueryFunction<T> for LabelsFunction {
    // Java: com/google/devtools/build/lib/query2/engine/LabelsFunction.java:56 LabelsFunction.eval
    fn eval(
        &self,
        env: &dyn QueryEnvironment<T>,
        context: &QueryExpressionContext<T>,
        expression: &QueryExpression,
        args: &[Argument],
        callback: &dyn Callback<T>,
    ) -> QueryTaskFuture<()> {
        let attr_name = args[0].get_word();
        let uniquifier = env.create_uniquifier();
        env.eval(
            args[1].get_expression(),
            context,
            &callback_fn(|partial_result: &[T]| {
                for input in partial_result {
                    if env.get_accessor().is_rule(input) {
                        let targets =
                            uniquifier.unique_all(&env.get_accessor().get_prerequisites(
                                expression,
                                input,
                                attr_name,
                                &format!(
                                    "in '{}' of rule {}: ",
                                    attr_name,
                                    env.get_accessor().get_label(input)
                                ),
                            )?)?;
                        let mut result = Vec::with_capacity(targets.len());
                        for target in &targets {
                            result.push(env.get_or_create(target));
                        }
                        callback.process(&result)?;
                    }
                }
                Ok(())
            }),
        )
    }
}
