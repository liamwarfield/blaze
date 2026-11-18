//! Port of `query2/engine/TargetLiteral.java`.

use std::fmt;

use blaze_protos::failure_details::query;

use crate::callback::Callback;
use crate::let_expression::LetExpression;
use crate::lexer;
use crate::query_environment::QueryEnvironment;
use crate::query_exception::QueryException;
use crate::query_expression::QueryExpression;
use crate::query_expression_context::QueryExpressionContext;
use crate::query_task_future::{QueryEnvironmentFutures, QueryTaskFuture};

/// A literal set of targets, using 'blaze build' syntax. Or, a reference to a variable name. (The
/// syntax of the string "pattern" determines which.)
///
/// TODO(bazel-team): Perhaps we should distinguish NAME from WORD in the parser, based on the
/// characters in it? Also, perhaps we should not allow NAMEs to be quoted like WORDs can be.
///
/// `expr ::= NAME | WORD`
// Java: com/google/devtools/build/lib/query2/engine/TargetLiteral.java:32 TargetLiteral
#[derive(Debug)]
pub struct TargetLiteral {
    pattern: String,
}

impl TargetLiteral {
    // Java: com/google/devtools/build/lib/query2/engine/TargetLiteral.java:36 TargetLiteral.<init>
    pub fn new(pattern: impl Into<String>) -> TargetLiteral {
        TargetLiteral {
            pattern: pattern.into(),
        }
    }

    // Java: com/google/devtools/build/lib/query2/engine/TargetLiteral.java:40 TargetLiteral.getPattern
    pub fn get_pattern(&self) -> &str {
        &self.pattern
    }

    // Java: com/google/devtools/build/lib/query2/engine/TargetLiteral.java:44 TargetLiteral.isVariableReference
    pub fn is_variable_reference(&self) -> bool {
        LetExpression::is_valid_var_reference(&self.pattern)
    }

    /// `this` is the `QueryExpression` wrapping this node (the failed expression of errors).
    // Java: com/google/devtools/build/lib/query2/engine/TargetLiteral.java:48 TargetLiteral.evalVarReference
    fn eval_var_reference<T: Clone + 'static>(
        &self,
        this: &QueryExpression,
        env: &dyn QueryEnvironment<T>,
        context: &QueryExpressionContext<T>,
        callback: &dyn Callback<T>,
    ) -> QueryTaskFuture<()> {
        let var_name = LetExpression::get_name_from_reference(&self.pattern);
        let value = match context.get(var_name) {
            Some(value) => value,
            None => {
                return env.immediate_failed_future(QueryException::new(
                    Some(this),
                    format!("undefined variable '{var_name}'"),
                    query::Code::VariableUndefined,
                ));
            }
        };
        QueryTaskFuture::from_caught(callback.process(&value.to_vec()))
    }

    /// `this` is the `QueryExpression` wrapping this node.
    // Java: com/google/devtools/build/lib/query2/engine/TargetLiteral.java:68 TargetLiteral.eval
    pub fn eval<T: Clone + 'static>(
        &self,
        this: &QueryExpression,
        env: &dyn QueryEnvironment<T>,
        context: &QueryExpressionContext<T>,
        callback: &dyn Callback<T>,
    ) -> QueryTaskFuture<()> {
        if self.is_variable_reference() {
            self.eval_var_reference(this, env, context, callback)
        } else {
            env.get_targets_matching_pattern(this, &self.pattern, callback)
        }
    }

    // Java: com/google/devtools/build/lib/query2/engine/TargetLiteral.java:78 TargetLiteral.collectTargetPatterns
    pub fn collect_target_patterns(&self, literals: &mut dyn FnMut(String)) {
        if !self.is_variable_reference() {
            literals(self.pattern.clone());
        }
    }
}

impl fmt::Display for TargetLiteral {
    // Java: com/google/devtools/build/lib/query2/engine/TargetLiteral.java:90 TargetLiteral.toString
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // The character matching has to be in sync with LabelValidator#PUNCTUATION_REQUIRING_QUOTING
        // except for the special characters that Lexer#scanWord *\/@.-_:$~ consider to be a word.
        let pattern = &self.pattern;
        let needs_quoting = lexer::is_reserved_word(pattern)
            || pattern.is_empty()
            || pattern.starts_with('-')
            || pattern.starts_with('*')
            || pattern.chars().any(|c| " \"#&'()+,;<=>?[]{|}".contains(c));

        if !needs_quoting {
            return f.write_str(pattern);
        }

        // If the word requires quoting, we want to quote the word such that the quoting character
        // does not lex the result differently if the result toString is fed back into the parser.
        // For example: If the following Java string that requires quoting is set("foo"), and we
        // quote the Java string with double quotes, "set("foo")" and feed it back into the lexer,
        // the lexer will parse the following word set(. So in this case we want to quote it with
        // single quotes such that it would look like 'set("foo")'. In the case that we find both
        // single quote and double quote, we would fail and use either of the two quotes.
        let quote = if pattern.contains('"') { '\'' } else { '"' };
        write!(f, "{quote}{pattern}{quote}")
    }
}
