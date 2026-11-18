//! Port of `query2/engine/QueryExpression.java`.

use std::fmt;
use std::sync::Arc;

use crate::binary_operator_expression::BinaryOperatorExpression;
use crate::callback::Callback;
use crate::function_expression::FunctionExpression;
use crate::let_expression::LetExpression;
use crate::query_environment::QueryEnvironment;
use crate::query_expression_context::QueryExpressionContext;
use crate::query_expression_visitor::QueryExpressionVisitor;
use crate::query_parser::QueryParser;
use crate::query_syntax_exception::QuerySyntaxException;
use crate::query_task_future::QueryTaskFuture;
use crate::set_expression::SetExpression;
use crate::target_literal::TargetLiteral;

/// Base class for expressions in the Blaze query language, revision 2.
///
/// All queries return a subgraph of the dependency graph, represented as a set of target nodes.
///
/// All queries must ensure that sufficient graph edges are created in the QueryEnvironment so that
/// all nodes in the result are correctly ordered according to the type of query. For example,
/// "deps" queries require that all the nodes in the transitive closure of its argument set are
/// correctly ordered w.r.t. each other; "somepath" queries require that the order of the nodes on
/// the resulting path are correctly ordered; algebraic set operations such as intersect and union
/// are inherently unordered.
///
/// PORT: Java's abstract class with five subclasses is a closed Rust enum of shared nodes. Java
/// compares expressions by reference (`QueryException.describeFailedQuery`,
/// `QueryExpressionMapper`); [`QueryExpression::ptr_eq`] compares the node pointers.
// Java: com/google/devtools/build/lib/query2/engine/QueryExpression.java:58 QueryExpression
#[derive(Clone)]
pub enum QueryExpression {
    TargetLiteral(Arc<TargetLiteral>),
    BinaryOperator(Arc<BinaryOperatorExpression>),
    Function(Arc<FunctionExpression>),
    Let(Arc<LetExpression>),
    Set(Arc<SetExpression>),
}

// Java: com/google/devtools/build/lib/query2/engine/QueryExpression.java:60 QueryExpression.MAX_QUERY_EXPRESSION_LOG_CHARS
const MAX_QUERY_EXPRESSION_LOG_CHARS: usize = 1000;

impl QueryExpression {
    /// Scan and parse the specified query expression.
    // Java: com/google/devtools/build/lib/query2/engine/QueryExpression.java:63 QueryExpression.parse
    pub fn parse<T: Clone + 'static>(
        query: &str,
        env: &dyn QueryEnvironment<T>,
    ) -> Result<QueryExpression, QuerySyntaxException> {
        QueryParser::parse(query, env)
    }

    /// Returns a `QueryTaskFuture` representing the asynchronous evaluation of this query in the
    /// specified environment, notifying the callback with a result. Note that it is allowed to
    /// notify the callback with partial results instead of just one final result.
    ///
    /// Failures resulting from evaluation of an ill-formed query cause QueryException to be
    /// thrown.
    ///
    /// The reporting of failures arising from errors in BUILD files depends on the --keep_going
    /// flag. If enabled (the default), then QueryException is thrown. If disabled, evaluation will
    /// stumble on to produce a (possibly inaccurate) result, but a result nonetheless.
    // Java: com/google/devtools/build/lib/query2/engine/QueryExpression.java:81 QueryExpression.eval
    pub fn eval<T: Clone + 'static>(
        &self,
        env: &dyn QueryEnvironment<T>,
        context: &QueryExpressionContext<T>,
        callback: &dyn Callback<T>,
    ) -> QueryTaskFuture<()> {
        match self {
            QueryExpression::TargetLiteral(e) => e.eval(self, env, context, callback),
            QueryExpression::BinaryOperator(e) => e.eval(env, context, callback),
            QueryExpression::Function(e) => e.eval(self, env, context, callback),
            QueryExpression::Let(e) => e.eval(self, env, context, callback),
            QueryExpression::Set(e) => e.eval(env, context, callback),
        }
    }

    /// Collects all target patterns that are referenced anywhere within this query expression and
    /// adds them to the given collection (Java `Collection.add`).
    // Java: com/google/devtools/build/lib/query2/engine/QueryExpression.java:88 QueryExpression.collectTargetPatterns
    pub fn collect_target_patterns(&self, literals: &mut dyn FnMut(String)) {
        match self {
            QueryExpression::TargetLiteral(e) => e.collect_target_patterns(literals),
            QueryExpression::BinaryOperator(e) => e.collect_target_patterns(literals),
            QueryExpression::Function(e) => e.collect_target_patterns(literals),
            QueryExpression::Let(e) => e.collect_target_patterns(literals),
            QueryExpression::Set(e) => e.collect_target_patterns(literals),
        }
    }

    /// Java `accept(visitor, context)`: dispatches to the visitor method of this node's class.
    // Java: com/google/devtools/build/lib/query2/engine/QueryExpression.java:91 QueryExpression.accept
    pub fn accept<R, C>(
        &self,
        visitor: &mut (impl QueryExpressionVisitor<R, C> + ?Sized),
        context: &C,
    ) -> R {
        match self {
            QueryExpression::TargetLiteral(e) => visitor.visit_target_literal(e, context),
            QueryExpression::BinaryOperator(e) => {
                visitor.visit_binary_operator_expression(e, context)
            }
            QueryExpression::Function(e) => visitor.visit_function_expression(e, context),
            QueryExpression::Let(e) => visitor.visit_let_expression(e, context),
            QueryExpression::Set(e) => visitor.visit_set_expression(e, context),
        }
    }

    /// Returns this query expression pretty-printed, and truncated to a max of 1000 characters.
    ///
    /// Helpful for preparing text for logging or human-readable display, because query
    /// expressions may be very long.
    // Java: com/google/devtools/build/lib/query2/engine/QueryExpression.java:106 QueryExpression.toTrunctatedString
    pub fn to_trunctated_string(&self) -> String {
        Self::truncate(&self.to_string())
    }

    /// Truncates the provided string to a max of 1000 characters, in the fashion of
    /// `toTrunctatedString()` (Guava `Ascii.truncate(expr, 1000, "[truncated]")`).
    ///
    /// PORT: StringEncoding identity. Java measures the internal-form string, one char per UTF-8
    /// byte, so lengths here are byte lengths. Where Java's cut would split a multi-byte
    /// character, this cut stops before that character.
    // Java: com/google/devtools/build/lib/query2/engine/QueryExpression.java:114 QueryExpression.truncate
    pub fn truncate(expr: &str) -> String {
        const INDICATOR: &str = "[truncated]";
        if expr.len() <= MAX_QUERY_EXPRESSION_LOG_CHARS {
            return expr.to_owned();
        }
        let mut end = MAX_QUERY_EXPRESSION_LOG_CHARS - INDICATOR.len();
        while !expr.is_char_boundary(end) {
            end -= 1;
        }
        let mut result = expr[..end].to_owned();
        result.push_str(INDICATOR);
        result
    }

    /// Checks if this QueryExpression has a SomePathFunction at its top level.
    // Java: com/google/devtools/build/lib/query2/engine/QueryExpression.java:119 QueryExpression.isTopLevelSomePathFunction
    pub fn is_top_level_some_path_function(&self) -> bool {
        match self {
            QueryExpression::Function(function_expression) => {
                "somepath" == function_expression.get_function().get_name()
            }
            _ => false,
        }
    }

    /// Java reference equality (`==`) between expressions.
    pub fn ptr_eq(&self, other: &QueryExpression) -> bool {
        match (self, other) {
            (QueryExpression::TargetLiteral(a), QueryExpression::TargetLiteral(b)) => {
                Arc::ptr_eq(a, b)
            }
            (QueryExpression::BinaryOperator(a), QueryExpression::BinaryOperator(b)) => {
                Arc::ptr_eq(a, b)
            }
            (QueryExpression::Function(a), QueryExpression::Function(b)) => Arc::ptr_eq(a, b),
            (QueryExpression::Let(a), QueryExpression::Let(b)) => Arc::ptr_eq(a, b),
            (QueryExpression::Set(a), QueryExpression::Set(b)) => Arc::ptr_eq(a, b),
            _ => false,
        }
    }

    /// The node's address, for identity-keyed maps (Java `System.identityHashCode` keys).
    pub fn identity(&self) -> usize {
        match self {
            QueryExpression::TargetLiteral(a) => Arc::as_ptr(a) as *const () as usize,
            QueryExpression::BinaryOperator(a) => Arc::as_ptr(a) as *const () as usize,
            QueryExpression::Function(a) => Arc::as_ptr(a) as *const () as usize,
            QueryExpression::Let(a) => Arc::as_ptr(a) as *const () as usize,
            QueryExpression::Set(a) => Arc::as_ptr(a) as *const () as usize,
        }
    }
}

impl From<Arc<TargetLiteral>> for QueryExpression {
    fn from(e: Arc<TargetLiteral>) -> Self {
        QueryExpression::TargetLiteral(e)
    }
}

impl From<TargetLiteral> for QueryExpression {
    fn from(e: TargetLiteral) -> Self {
        QueryExpression::TargetLiteral(Arc::new(e))
    }
}

impl From<BinaryOperatorExpression> for QueryExpression {
    fn from(e: BinaryOperatorExpression) -> Self {
        QueryExpression::BinaryOperator(Arc::new(e))
    }
}

impl From<FunctionExpression> for QueryExpression {
    fn from(e: FunctionExpression) -> Self {
        QueryExpression::Function(Arc::new(e))
    }
}

impl From<LetExpression> for QueryExpression {
    fn from(e: LetExpression) -> Self {
        QueryExpression::Let(Arc::new(e))
    }
}

impl From<SetExpression> for QueryExpression {
    fn from(e: SetExpression) -> Self {
        QueryExpression::Set(Arc::new(e))
    }
}

impl fmt::Display for QueryExpression {
    /// Returns this query expression pretty-printed.
    // Java: com/google/devtools/build/lib/query2/engine/QueryExpression.java:96 QueryExpression.toString
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            QueryExpression::TargetLiteral(e) => fmt::Display::fmt(&**e, f),
            QueryExpression::BinaryOperator(e) => fmt::Display::fmt(&**e, f),
            QueryExpression::Function(e) => fmt::Display::fmt(&**e, f),
            QueryExpression::Let(e) => fmt::Display::fmt(&**e, f),
            QueryExpression::Set(e) => fmt::Display::fmt(&**e, f),
        }
    }
}

impl fmt::Debug for QueryExpression {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}
