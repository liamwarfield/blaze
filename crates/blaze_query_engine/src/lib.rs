//! Port of com.google.devtools.build.lib.query2.engine: the query language, parser, functions and
//! evaluation over an abstract target type.
//!
//! Design (see the module docs for details):
//!
//! - [`QueryExpression`] is a closed enum of shared nodes (`TargetLiteral`,
//!   `BinaryOperatorExpression`, `FunctionExpression`, `LetExpression`, `SetExpression`); Java's
//!   reference equality is [`QueryExpression::ptr_eq`].
//! - [`QueryEnvironment<T>`] is an object-safe trait generic over the node type; functions take
//!   `&dyn QueryEnvironment<T>`. Its nested Java types keep their simple names.
//! - Futures are immediate ([`query_task_future`]): every Blaze environment completes them on the
//!   calling thread, as Bazel's `AbstractBlazeQueryEnvironment` does with a direct executor.
//! - `QueryFunction` splits into [`QueryFunctionBase`] (name, signature) and
//!   [`QueryFunction<T>`] (evaluation); parsed function expressions keep the environment's
//!   `QueryFunction<T>`.
//! - Errors: Java's `throws QueryException, InterruptedException` is [`QueryError`].

#![allow(
    clippy::too_many_arguments,
    clippy::new_without_default,
    clippy::type_complexity,
    clippy::large_enum_variant
)]

pub mod action_filter_function;
pub mod aggregating_query_expression_visitor;
pub mod all_paths_function;
pub mod all_rdeps_function;
pub mod attr_function;
pub mod binary_operator_expression;
pub mod build_files_function;
pub mod callback;
pub mod deps_function;
pub mod detailed_exit_code;
pub mod executables_function;
pub mod filter_function;
pub mod function_expression;
pub mod java_concurrent_hash_map;
pub mod key_extractor;
pub mod kind_function;
pub mod labels_function;
pub mod let_expression;
pub mod lexer;
pub mod load_files_function;
pub mod min_depth_uniquifier;
pub mod query_environment;
pub mod query_eval_result;
pub mod query_exception;
pub mod query_expression;
pub mod query_expression_context;
pub mod query_expression_mapper;
pub mod query_expression_visitor;
pub mod query_parser;
pub mod query_syntax_exception;
pub mod query_task_future;
pub mod query_util;
pub mod query_visibility;
pub mod rdeps_function;
pub mod regex_filter_expression;
pub mod same_pkg_direct_rdeps_function;
pub mod set_expression;
pub mod siblings_function;
pub mod some_function;
pub mod some_path_function;
pub mod streamable_query_environment;
pub mod target_literal;
pub mod tests_function;
pub mod total_weight_query_expression_visitor;
pub mod uniquifier;
pub mod visible_function;

pub use action_filter_function::{InputsFunction, MnemonicFunction, OutputsFunction};
pub use aggregating_query_expression_visitor::{
    AggregatingQueryExpressionVisitor, ContainsFunctionQueryExpressionVisitor,
};
pub use all_paths_function::AllPathsFunction;
pub use all_rdeps_function::AllRdepsFunction;
pub use attr_function::AttrFunction;
pub use binary_operator_expression::BinaryOperatorExpression;
pub use build_files_function::BuildFilesFunction;
pub use callback::{
    Callback, OutputFormatterCallback, OutputFormatterError,
    SynchronizedDelegatingOutputFormatterCallback, ThreadSafeOutputFormatterCallback, callback_fn,
};
pub use deps_function::DepsFunction;
pub use detailed_exit_code::DetailedExitCode;
pub use executables_function::ExecutablesFunction;
pub use filter_function::FilterFunction;
pub use function_expression::FunctionExpression;
pub use java_concurrent_hash_map::{JavaConcurrentHashMap, JavaHashCode};
pub use key_extractor::KeyExtractor;
pub use kind_function::KindFunction;
pub use labels_function::LabelsFunction;
pub use let_expression::LetExpression;
pub use lexer::{Lexer, Token, TokenKind};
pub use load_files_function::LoadFilesFunction;
pub use min_depth_uniquifier::MinDepthUniquifier;
pub use query_environment::{
    Argument, ArgumentType, CustomFunctionQueryEnvironment, EvaluateExpression,
    FilteringQueryFunction, QueryEnvironment, QueryFunction, QueryFunctionBase, Setting,
    TargetAccessor, TargetNotFoundException, ThreadSafeMutableSet, TransitiveLoadFilesHelper,
    default_query_functions,
};
pub use query_eval_result::QueryEvalResult;
pub use query_exception::{InterruptedException, QueryError, QueryException};
pub use query_expression::QueryExpression;
pub use query_expression_context::QueryExpressionContext;
pub use query_expression_mapper::QueryExpressionMapper;
pub use query_expression_visitor::QueryExpressionVisitor;
pub use query_parser::QueryParser;
pub use query_syntax_exception::{QuerySyntaxException, SkyframeRestartQueryException};
pub use query_task_future::{AnyQueryTaskFuture, QueryEnvironmentFutures, QueryTaskFuture};
pub use query_visibility::QueryVisibility;
pub use rdeps_function::RdepsFunction;
pub use same_pkg_direct_rdeps_function::SamePkgDirectRdepsFunction;
pub use set_expression::SetExpression;
pub use siblings_function::SiblingsFunction;
pub use some_function::SomeFunction;
pub use some_path_function::SomePathFunction;
pub use streamable_query_environment::StreamableQueryEnvironment;
pub use target_literal::TargetLiteral;
pub use tests_function::TestsFunction;
pub use total_weight_query_expression_visitor::TotalWeightQueryExpressionVisitor;
pub use uniquifier::Uniquifier;
pub use visible_function::VisibleFunction;
