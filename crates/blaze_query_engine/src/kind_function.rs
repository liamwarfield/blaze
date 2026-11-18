//! Port of `query2/engine/KindFunction.java`.

use std::sync::Arc;

use crate::callback::Callback;
use crate::query_environment::{
    Argument, ArgumentType, FilteringQueryFunction, QueryEnvironment, QueryFunction,
    QueryFunctionBase,
};
use crate::query_expression::QueryExpression;
use crate::query_expression_context::QueryExpressionContext;
use crate::query_task_future::QueryTaskFuture;
use crate::regex_filter_expression::{
    RegexFilterExpression, regex_filter_expression_eval,
    regex_filter_expression_get_expression_to_filter_index,
};

/// A kind(pattern, argument) filter expression, which computes the set of subset of nodes in
/// 'argument' whose kind matches the unanchored regexp 'pattern'.
///
/// `expr ::= KIND '(' WORD ',' expr ')'`
///
/// Example patterns:
///
/// ```text
/// ' file'              Match all file targets.
/// 'source file'        Match all test source file targets.
/// 'generated file'     Match all test generated file targets.
/// ' rule'              Match all rule targets.
/// 'foo_*'              Match all rules starting with "foo_",
/// 'test'               Match all test (rule) targets.
/// ```
///
/// Note, the space before "file" is needed to prevent unwanted matches against (e.g.) "filegroup
/// rule".
// Java: com/google/devtools/build/lib/query2/engine/KindFunction.java:42 KindFunction
#[derive(Debug, Clone, Copy)]
pub struct KindFunction {
    invert: bool,
}

impl KindFunction {
    // Java: com/google/devtools/build/lib/query2/engine/KindFunction.java:43 KindFunction.<init>
    pub fn new() -> Self {
        KindFunction { invert: false }
    }
}

impl QueryFunctionBase for KindFunction {
    // Java: com/google/devtools/build/lib/query2/engine/KindFunction.java:57 KindFunction.getName
    fn get_name(&self) -> String {
        format!("{}kind", if self.invert { "no" } else { "" })
    }

    // Java: com/google/devtools/build/lib/query2/engine/KindFunction.java:67 KindFunction.getMandatoryArguments
    fn get_mandatory_arguments(&self) -> i32 {
        2
    }

    // Java: com/google/devtools/build/lib/query2/engine/KindFunction.java:72 KindFunction.getArgumentTypes
    fn get_argument_types(&self) -> Vec<ArgumentType> {
        vec![ArgumentType::Word, ArgumentType::Expression]
    }
}

impl<T: Clone + 'static> RegexFilterExpression<T> for KindFunction {
    fn invert_flag(&self) -> bool {
        self.invert
    }

    // Java: com/google/devtools/build/lib/query2/engine/KindFunction.java:62 KindFunction.getPattern
    fn get_pattern(&self, args: &[Argument]) -> String {
        args[0].get_word().to_owned()
    }

    // Java: com/google/devtools/build/lib/query2/engine/KindFunction.java:77 KindFunction.getFilterString
    fn get_filter_string(
        &self,
        env: &dyn QueryEnvironment<T>,
        _args: &[Argument],
        target: &T,
    ) -> Option<String> {
        Some(env.get_accessor().get_target_kind(target))
    }
}

impl<T: Clone + 'static> QueryFunction<T> for KindFunction {
    fn eval(
        &self,
        env: &dyn QueryEnvironment<T>,
        context: &QueryExpressionContext<T>,
        expression: &QueryExpression,
        args: &[Argument],
        callback: &dyn Callback<T>,
    ) -> QueryTaskFuture<()> {
        regex_filter_expression_eval::<T>(self, env, context, expression, args, callback)
    }

    fn as_filtering_function(&self) -> Option<&dyn FilteringQueryFunction<T>> {
        Some(self)
    }
}

impl<T: Clone + 'static> FilteringQueryFunction<T> for KindFunction {
    // Java: com/google/devtools/build/lib/query2/engine/KindFunction.java:52 KindFunction.invert
    fn invert(&self) -> Arc<dyn FilteringQueryFunction<T>> {
        Arc::new(KindFunction {
            invert: !self.invert,
        })
    }

    fn get_expression_to_filter_index(&self) -> i32 {
        regex_filter_expression_get_expression_to_filter_index::<T>(self)
    }
}
