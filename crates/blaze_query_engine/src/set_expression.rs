//! Port of `query2/engine/SetExpression.java`.

use std::fmt;
use std::sync::Arc;

use crate::callback::Callback;
use crate::query_environment::QueryEnvironment;
use crate::query_expression::QueryExpression;
use crate::query_expression_context::QueryExpressionContext;
use crate::query_task_future::{QueryEnvironmentFutures, QueryTaskFuture, as_any_futures};
use crate::target_literal::TargetLiteral;

/// A set(word, ..., word) expression, which computes the union of zero or more target patterns
/// separated by whitespace. This is intended to support the use-case in which a set of labels
/// written to a file by a previous query expression can be modified externally, then used as
/// input to another query, like so:
///
/// ```text
/// % blaze query 'somepath(foo, bar)' | grep ... | sed ... | awk ... >file
/// % blaze query "kind(qux_library, set($(<file)))"
/// ```
///
/// The grammar currently restricts the operands of set() to being zero or more words (target
/// patterns), with no intervening punctuation. In principle this could be extended to arbitrary
/// expressions without grammatical ambiguity, but this seems excessively general for now.
///
/// `expr ::= SET '(' WORD * ')'`
// Java: com/google/devtools/build/lib/query2/engine/SetExpression.java:41 SetExpression
pub struct SetExpression {
    words: Vec<Arc<TargetLiteral>>,
}

impl SetExpression {
    // Java: com/google/devtools/build/lib/query2/engine/SetExpression.java:45 SetExpression.<init>
    pub fn new(words: Vec<Arc<TargetLiteral>>) -> SetExpression {
        SetExpression { words }
    }

    // Java: com/google/devtools/build/lib/query2/engine/SetExpression.java:50 SetExpression.eval
    pub fn eval<T: Clone + 'static>(
        &self,
        env: &dyn QueryEnvironment<T>,
        context: &QueryExpressionContext<T>,
        callback: &dyn Callback<T>,
    ) -> QueryTaskFuture<()> {
        let mut query_tasks = Vec::with_capacity(self.words.len());
        for expr in &self.words {
            query_tasks.push(env.eval(
                &QueryExpression::TargetLiteral(expr.clone()),
                context,
                callback,
            ));
        }
        env.when_all_succeed(&as_any_futures(&query_tasks))
    }

    // Java: com/google/devtools/build/lib/query2/engine/SetExpression.java:60 SetExpression.collectTargetPatterns
    pub fn collect_target_patterns(&self, literals: &mut dyn FnMut(String)) {
        for expr in &self.words {
            expr.collect_target_patterns(literals);
        }
    }

    /// Gets the list of `TargetLiteral`s contained in the expression.
    // Java: com/google/devtools/build/lib/query2/engine/SetExpression.java:72 SetExpression.getWords
    pub fn get_words(&self) -> &[Arc<TargetLiteral>] {
        &self.words
    }
}

impl fmt::Display for SetExpression {
    // Java: com/google/devtools/build/lib/query2/engine/SetExpression.java:77 SetExpression.toString
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("set(")?;
        for (i, word) in self.words.iter().enumerate() {
            if i > 0 {
                f.write_str(" ")?;
            }
            write!(f, "{word}")?;
        }
        f.write_str(")")
    }
}
