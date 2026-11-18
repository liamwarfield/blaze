//! Port of `query2/engine/FilterFunction.java`.

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

/// A label(pattern, argument) filter expression, which computes the set of subset of nodes in
/// 'argument' whose label matches the unanchored regexp 'pattern'.
///
/// `expr ::= FILTER '(' WORD ',' expr ')'`
///
/// Example patterns:
///
/// ```text
/// '//third_party'      Match all targets in the //third_party/...
///                      (equivalent to 'intersect //third_party/...)
/// '\.jar$'              Match all *.jar targets.
/// ```
// Java: com/google/devtools/build/lib/query2/engine/FilterFunction.java:35 FilterFunction
#[derive(Debug, Clone, Copy)]
pub struct FilterFunction {
    invert: bool,
}

impl FilterFunction {
    // Java: com/google/devtools/build/lib/query2/engine/FilterFunction.java:36 FilterFunction.<init>
    pub fn new() -> Self {
        FilterFunction { invert: false }
    }
}

impl QueryFunctionBase for FilterFunction {
    // Java: com/google/devtools/build/lib/query2/engine/FilterFunction.java:50 FilterFunction.getName
    fn get_name(&self) -> String {
        format!("{}filter", if self.invert { "no" } else { "" })
    }

    // Java: com/google/devtools/build/lib/query2/engine/FilterFunction.java:60 FilterFunction.getMandatoryArguments
    fn get_mandatory_arguments(&self) -> i32 {
        2
    }

    // Java: com/google/devtools/build/lib/query2/engine/FilterFunction.java:65 FilterFunction.getArgumentTypes
    fn get_argument_types(&self) -> Vec<ArgumentType> {
        vec![ArgumentType::Word, ArgumentType::Expression]
    }
}

impl<T: Clone + 'static> RegexFilterExpression<T> for FilterFunction {
    fn invert_flag(&self) -> bool {
        self.invert
    }

    // Java: com/google/devtools/build/lib/query2/engine/FilterFunction.java:55 FilterFunction.getPattern
    fn get_pattern(&self, args: &[Argument]) -> String {
        args[0].get_word().to_owned()
    }

    // Java: com/google/devtools/build/lib/query2/engine/FilterFunction.java:70 FilterFunction.getFilterString
    fn get_filter_string(
        &self,
        env: &dyn QueryEnvironment<T>,
        _args: &[Argument],
        target: &T,
    ) -> Option<String> {
        Some(env.get_accessor().get_label(target))
    }
}

impl<T: Clone + 'static> QueryFunction<T> for FilterFunction {
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

impl<T: Clone + 'static> FilteringQueryFunction<T> for FilterFunction {
    // Java: com/google/devtools/build/lib/query2/engine/FilterFunction.java:45 FilterFunction.invert
    fn invert(&self) -> Arc<dyn FilteringQueryFunction<T>> {
        Arc::new(FilterFunction {
            invert: !self.invert,
        })
    }

    fn get_expression_to_filter_index(&self) -> i32 {
        regex_filter_expression_get_expression_to_filter_index::<T>(self)
    }
}
