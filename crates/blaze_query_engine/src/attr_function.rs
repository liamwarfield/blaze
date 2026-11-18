//! Port of `query2/engine/AttrFunction.java`.

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

/// An attr(attribute, pattern, argument) filter expression, which computes the set of subset of
/// nodes in 'argument' which correspond to rules with defined attribute 'attribute' with attribute
/// value matching the unanchored regexp 'pattern'. For boolean attributes, the attribute value
/// will be formatted as '0' (False) or '1' (True) for backwards compatibility purposes. For list
/// attributes, the attribute value will be defined as a usual List.toString() representation
/// (using '[' as first character, ']' as last character and ", " as a delimiter between multiple
/// values). Also, all label-based attributes will use fully-qualified label names instead of
/// original value specified in the BUILD file.
///
/// `expr ::= ATTR '(' ATTRNAME ',' WORD ',' expr ')'`
///
/// Examples
///
/// ```text
/// attr(linkshared,1,//project/...)    find all rules under in the //project/... that
///                                 have attribute linkshared set to 1.
/// ```
// Java: com/google/devtools/build/lib/query2/engine/AttrFunction.java:41 AttrFunction
#[derive(Debug, Clone, Copy)]
pub struct AttrFunction {
    invert: bool,
}

impl AttrFunction {
    // Java: com/google/devtools/build/lib/query2/engine/AttrFunction.java:42 AttrFunction.<init>
    pub fn new() -> Self {
        AttrFunction { invert: false }
    }
}

impl QueryFunctionBase for AttrFunction {
    // Java: com/google/devtools/build/lib/query2/engine/AttrFunction.java:56 AttrFunction.getName
    fn get_name(&self) -> String {
        format!("{}attr", if self.invert { "no" } else { "" })
    }

    // Java: com/google/devtools/build/lib/query2/engine/AttrFunction.java:66 AttrFunction.getMandatoryArguments
    fn get_mandatory_arguments(&self) -> i32 {
        3
    }

    // Java: com/google/devtools/build/lib/query2/engine/AttrFunction.java:71 AttrFunction.getArgumentTypes
    fn get_argument_types(&self) -> Vec<ArgumentType> {
        vec![
            ArgumentType::Word,
            ArgumentType::Word,
            ArgumentType::Expression,
        ]
    }
}

impl<T: Clone + 'static> RegexFilterExpression<T> for AttrFunction {
    fn invert_flag(&self) -> bool {
        self.invert
    }

    // Java: com/google/devtools/build/lib/query2/engine/AttrFunction.java:61 AttrFunction.getPattern
    fn get_pattern(&self, args: &[Argument]) -> String {
        args[1].get_word().to_owned()
    }

    // Java: com/google/devtools/build/lib/query2/engine/AttrFunction.java:76 AttrFunction.getFilterString
    fn get_filter_string(
        &self,
        _env: &dyn QueryEnvironment<T>,
        _args: &[Argument],
        _target: &T,
    ) -> Option<String> {
        panic!(
            "IllegalStateException: The 'attr' regex filter gets its match values directly from \
             getFilterStrings"
        )
    }

    // Java: com/google/devtools/build/lib/query2/engine/AttrFunction.java:82 AttrFunction.getFilterStrings
    fn get_filter_strings(
        &self,
        env: &dyn QueryEnvironment<T>,
        args: &[Argument],
        target: &T,
    ) -> Vec<String> {
        if env.get_accessor().is_rule(target) {
            return env
                .get_accessor()
                .get_attr_as_string(target, args[0].get_word());
        }
        vec![]
    }
}

impl<T: Clone + 'static> QueryFunction<T> for AttrFunction {
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

impl<T: Clone + 'static> FilteringQueryFunction<T> for AttrFunction {
    // Java: com/google/devtools/build/lib/query2/engine/AttrFunction.java:51 AttrFunction.invert
    fn invert(&self) -> Arc<dyn FilteringQueryFunction<T>> {
        Arc::new(AttrFunction {
            invert: !self.invert,
        })
    }

    fn get_expression_to_filter_index(&self) -> i32 {
        regex_filter_expression_get_expression_to_filter_index::<T>(self)
    }
}
