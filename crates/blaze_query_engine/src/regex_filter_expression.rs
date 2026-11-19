//! Port of `query2/engine/RegexFilterExpression.java`.

use blaze_protos::failure_details::query;

use crate::callback::{Callback, callback_fn};
use crate::query_environment::{Argument, QueryEnvironment, QueryFunctionBase};
use crate::query_exception::QueryException;
use crate::query_expression::QueryExpression;
use crate::query_expression_context::QueryExpressionContext;
use crate::query_task_future::{QueryEnvironmentFutures, QueryTaskFuture};

/// Java `Pattern.compile(pattern)`: `blaze_util::java_regex` (Java's dialect on the `regex`
/// crate; its module docs list what is `unported!`). The error is
/// `PatternSyntaxException.getMessage()`. Pattern and inputs are internal strings, as in Java: `é`
/// in a pattern is the two chars U+00C3 U+00A9 and matches the same two chars of a label.
pub fn compile_java_pattern(pattern: &str) -> Result<blaze_util::java_regex::Pattern, String> {
    blaze_util::java_regex::Pattern::compile(pattern, 0)
}

/// An abstract class that provides generic regex filter expression. Actual expression are
/// implemented by the subclasses.
///
/// PORT: Java's abstract class is this trait; a subclass implements it and its
/// `QueryFunction::eval` calls [`regex_filter_expression_eval`], its
/// `get_expression_to_filter_index` calls [`regex_filter_expression_get_expression_to_filter_index`].
// Java: com/google/devtools/build/lib/query2/engine/RegexFilterExpression.java:33 RegexFilterExpression
pub trait RegexFilterExpression<T>: QueryFunctionBase {
    /// Java's `protected final boolean invert`.
    fn invert_flag(&self) -> bool;

    /// Returns string for the given target that must be matched against pattern. May return
    /// `None`, in which case matching is guaranteed to fail.
    // Java: com/google/devtools/build/lib/query2/engine/RegexFilterExpression.java:87 RegexFilterExpression.getFilterString
    fn get_filter_string(
        &self,
        env: &dyn QueryEnvironment<T>,
        args: &[Argument],
        target: &T,
    ) -> Option<String>;

    /// Returns a list of strings for the given target that must be matched against pattern. The
    /// filter matches if *any* of these strings matches.
    ///
    /// Unless subclasses have an explicit reason to override this method, it's fine to keep the
    /// default implementation that just delegates to `get_filter_string`. Overriding this method
    /// is useful for subclasses that want to match against a universe of possible values. For
    /// example, with configurable attributes, an attribute might have different values depending
    /// on the build configuration. One may wish the filter to match if *any* of those values
    /// matches.
    // Java: com/google/devtools/build/lib/query2/engine/RegexFilterExpression.java:101 RegexFilterExpression.getFilterStrings
    fn get_filter_strings(
        &self,
        env: &dyn QueryEnvironment<T>,
        args: &[Argument],
        target: &T,
    ) -> Vec<String> {
        match self.get_filter_string(env, args, target) {
            None => vec![],
            Some(filter_string) => vec![filter_string],
        }
    }

    // Java: com/google/devtools/build/lib/query2/engine/RegexFilterExpression.java:107 RegexFilterExpression.getPattern
    fn get_pattern(&self, args: &[Argument]) -> String;
}

// Java: com/google/devtools/build/lib/query2/engine/RegexFilterExpression.java:79 RegexFilterExpression.getExpressionToFilterIndex
pub fn regex_filter_expression_get_expression_to_filter_index<T>(
    this: &(impl RegexFilterExpression<T> + ?Sized),
) -> i32 {
    this.get_mandatory_arguments() - 1
}

// Java: com/google/devtools/build/lib/query2/engine/RegexFilterExpression.java:41 RegexFilterExpression.eval
pub fn regex_filter_expression_eval<T: Clone + 'static>(
    this: &(impl RegexFilterExpression<T> + ?Sized),
    env: &dyn QueryEnvironment<T>,
    context: &QueryExpressionContext<T>,
    expression: &QueryExpression,
    args: &[Argument],
    callback: &dyn Callback<T>,
) -> QueryTaskFuture<()> {
    let raw_pattern = this.get_pattern(args);
    let compiled_pattern = match compile_java_pattern(&raw_pattern) {
        Ok(p) => p,
        Err(e) => {
            return env.immediate_failed_future(QueryException::new(
                Some(expression),
                format!(
                    "illegal '{}' pattern regexp '{}': {}",
                    this.get_name(),
                    raw_pattern,
                    e
                ),
                query::Code::SyntaxError,
            ));
        }
    };

    let invert = this.invert_flag();
    // Note that Patttern#matcher is thread-safe and so this Predicate can safely be used
    // concurrently.
    let match_filter = |target: &T| -> bool {
        for s in this.get_filter_strings(env, args, target) {
            if compiled_pattern.is_match(&s) {
                return !invert;
            }
        }
        invert
    };

    let index = regex_filter_expression_get_expression_to_filter_index::<T>(this) as usize;
    env.eval(
        args[index].get_expression(),
        context,
        &filtered_callback(callback, &match_filter),
    )
}

/// Java `RegexFilterExpression.FilteredCallback`.
///
/// Consume as much of the iterable as possible here. The parent callback may be synchronized, so
/// we can avoid calling it at all if everything gets filtered out.
///
/// PORT: Java hands the parent a lazily filtered `InProgressIterable`; the predicate is pure, so
/// passing the filtered batch is equivalent.
// Java: com/google/devtools/build/lib/query2/engine/RegexFilterExpression.java:109 RegexFilterExpression.FilteredCallback
fn filtered_callback<'a, T: Clone>(
    parent_callback: &'a dyn Callback<T>,
    retain_if_true: &'a dyn Fn(&T) -> bool,
) -> impl Fn(&[T]) -> Result<(), crate::query_exception::QueryError> + 'a {
    callback_fn(move |partial_result: &[T]| {
        let mut it = partial_result.iter();
        while let Some(next) = it.next() {
            if retain_if_true(next) {
                let mut rest = vec![next.clone()];
                rest.extend(it.filter(|t| retain_if_true(*t)).cloned());
                parent_callback.process(&rest)?;
                break;
            }
        }
        Ok(())
    })
}
