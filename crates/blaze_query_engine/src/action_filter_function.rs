//! Ports of `query2/engine/ActionFilterFunction.java` and its subclasses `InputsFunction.java`,
//! `OutputsFunction.java` and `MnemonicFunction.java` (aquery's filter functions).

use blaze_protos::failure_details::action_query;

use crate::callback::Callback;
use crate::query_environment::{
    Argument, ArgumentType, QueryEnvironment, QueryFunction, QueryFunctionBase,
};
use crate::query_exception::QueryException;
use crate::query_expression::QueryExpression;
use crate::query_expression_context::QueryExpressionContext;
use crate::query_task_future::{QueryEnvironmentFutures, QueryTaskFuture};
use crate::regex_filter_expression::compile_java_pattern;

/// A function that carries out filtering on the action graph.
///
/// PORT: Java's abstract class; each subclass implements `QueryFunctionBase::get_name` and
/// `get_argument_types`, takes [`ACTION_FILTER_FUNCTION_MANDATORY_ARGUMENTS`] and evaluates with
/// [`action_filter_function_eval`].
// Java: com/google/devtools/build/lib/query2/engine/ActionFilterFunction.java:25 ActionFilterFunction
pub trait ActionFilterFunction: QueryFunctionBase {}

/// The number of mandatory args was set to 1 to accommodate the --skyframe_state flag. For all
/// other purposes, the _actual_ number of mandatory args is 2.
// Java: com/google/devtools/build/lib/query2/engine/ActionFilterFunction.java:28 ActionFilterFunction.getMandatoryArguments
pub const ACTION_FILTER_FUNCTION_MANDATORY_ARGUMENTS: i32 = 1;

// Java: com/google/devtools/build/lib/query2/engine/ActionFilterFunction.java:35 ActionFilterFunction.eval
pub fn action_filter_function_eval<T: Clone + 'static>(
    this: &(impl ActionFilterFunction + ?Sized),
    env: &dyn QueryEnvironment<T>,
    context: &QueryExpressionContext<T>,
    expression: &QueryExpression,
    args: &[Argument],
    callback: &dyn Callback<T>,
) -> QueryTaskFuture<()> {
    let filter_pattern = args[0].get_word();
    if let Err(e) = compile_java_pattern(filter_pattern) {
        return env.immediate_failed_future(QueryException::with_action_query_code(
            Some(expression),
            format!(
                "Illegal '{}' pattern regexp '{}': {}",
                this.get_name(),
                filter_pattern,
                e
            ),
            action_query::Code::IllegalPatternSyntax,
        ));
    }

    // The 2nd argument can only be empty in the case of --skyframe_state.
    if args.len() != 2 {
        return env.immediate_failed_future(QueryException::with_action_query_code(
            Some(expression),
            "aquery filter functions (inputs, outputs, mnemonics) must have exactly 2 arguments,\
             except when --skyframe_state is used.",
            action_query::Code::IncorrectArguments,
        ));
    }

    // Do nothing, pass the expression along
    env.eval(args[1].get_expression(), context, callback)
}

macro_rules! action_filter_function {
    ($(#[$doc:meta])* $name:ident, $const_name:ident, $value:literal, $java:literal) => {
        $(#[$doc])*
        #[derive(Debug, Default)]
        pub struct $name;

        impl $name {
            pub const $const_name: &'static str = $value;

            pub fn new() -> Self {
                $name
            }
        }

        impl ActionFilterFunction for $name {}

        impl QueryFunctionBase for $name {
            fn get_name(&self) -> String {
                Self::$const_name.to_owned()
            }

            fn get_mandatory_arguments(&self) -> i32 {
                ACTION_FILTER_FUNCTION_MANDATORY_ARGUMENTS
            }

            fn get_argument_types(&self) -> Vec<ArgumentType> {
                vec![ArgumentType::Word, ArgumentType::Expression]
            }
        }

        impl<T: Clone + 'static> QueryFunction<T> for $name {
            fn eval(
                &self,
                env: &dyn QueryEnvironment<T>,
                context: &QueryExpressionContext<T>,
                expression: &QueryExpression,
                args: &[Argument],
                callback: &dyn Callback<T>,
            ) -> QueryTaskFuture<()> {
                action_filter_function_eval(self, env, context, expression, args, callback)
            }
        }
    };
}

action_filter_function!(
    /// A label(pattern, argument) filter expression, which computes the set of resulting actions
    /// of argument whose inputs matches the regexp 'pattern'. The pattern follows
    /// java.util.regex format.
    ///
    /// `expr ::= INPUTS '(' WORD ',' expr ')'`
    // Java: com/google/devtools/build/lib/query2/engine/InputsFunction.java:34 InputsFunction
    InputsFunction,
    INPUTS,
    "inputs",
    "InputsFunction"
);

action_filter_function!(
    /// A label(pattern, argument) filter expression, which computes the set of resulting actions
    /// of argument whose outputs matches the regexp 'pattern'. The pattern follows
    /// java.util.regex format.
    ///
    /// `expr ::= OUTPUTS '(' WORD ',' expr ')'`
    // Java: com/google/devtools/build/lib/query2/engine/OutputsFunction.java:34 OutputsFunction
    OutputsFunction,
    OUTPUTS,
    "outputs",
    "OutputsFunction"
);

action_filter_function!(
    /// A label(pattern, argument) filter expression, which computes the set of resulting actions
    /// of argument whose mnemonics matches the regexp 'pattern'. The pattern follows
    /// java.util.regex format.
    ///
    /// `expr ::= MNEMONIC '(' WORD ',' expr ')'`
    // Java: com/google/devtools/build/lib/query2/engine/MnemonicFunction.java:35 MnemonicFunction
    MnemonicFunction,
    MNEMONIC,
    "mnemonic",
    "MnemonicFunction"
);
