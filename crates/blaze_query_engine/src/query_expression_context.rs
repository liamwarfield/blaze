//! Port of `query2/engine/QueryExpressionContext.java`.

use std::sync::Arc;

use indexmap::IndexMap;

use crate::query_environment::ThreadSafeMutableSet;

/// An immutable context, including variable bindings for variables introduced by
/// `LetExpression`s.
///
/// PORT: `extraGlobalDeps()` (an `ImmutableSetMultimap<SkyKey, SkyKey>`, always empty here and
/// only overridden by cquery's context subclass) is not ported; cquery adds it with its context.
// Java: com/google/devtools/build/lib/query2/engine/QueryExpressionContext.java:30 QueryExpressionContext
pub struct QueryExpressionContext<T> {
    context: IndexMap<String, Arc<dyn ThreadSafeMutableSet<T>>>,
}

impl<T> Clone for QueryExpressionContext<T> {
    fn clone(&self) -> Self {
        QueryExpressionContext {
            context: self.context.clone(),
        }
    }
}

impl<T> QueryExpressionContext<T> {
    /// Returns the value bound to the specified variable given by `name`, or `None` if there is
    /// no such binding.
    // Java: com/google/devtools/build/lib/query2/engine/QueryExpressionContext.java:42 QueryExpressionContext.get
    pub fn get(&self, name: &str) -> Option<&Arc<dyn ThreadSafeMutableSet<T>>> {
        self.context.get(name)
    }

    /// Returns a `QueryExpressionContext` with no variables defined.
    // Java: com/google/devtools/build/lib/query2/engine/QueryExpressionContext.java:47 QueryExpressionContext.empty
    pub fn empty() -> Self {
        QueryExpressionContext {
            context: IndexMap::new(),
        }
    }

    /// Returns a `QueryExpressionContext` that has all the same bindings as the given
    /// `variableContext` and also the binding of `name` to `value`.
    // Java: com/google/devtools/build/lib/query2/engine/QueryExpressionContext.java:54 QueryExpressionContext.with
    pub fn with(&self, name: &str, value: Arc<dyn ThreadSafeMutableSet<T>>) -> Self {
        QueryExpressionContext {
            context: self.with_new_variable(name, value),
        }
    }

    // Java: com/google/devtools/build/lib/query2/engine/QueryExpressionContext.java:58 QueryExpressionContext.withNewVariable
    pub fn with_new_variable(
        &self,
        name: &str,
        value: Arc<dyn ThreadSafeMutableSet<T>>,
    ) -> IndexMap<String, Arc<dyn ThreadSafeMutableSet<T>>> {
        let mut new_context_builder = IndexMap::new();
        for (key, entry) in &self.context {
            if key != name {
                // The binding of 'name' to 'value' should override any existing binding of name
                // in 'variableContext'. These are the semantics we want in order for nested
                // let-expressions to have the semantics we want.
                new_context_builder.insert(key.clone(), entry.clone());
            }
        }
        new_context_builder.insert(name.to_owned(), value);
        new_context_builder
    }

    /// The variable names bound in this context, in binding order.
    pub fn variable_names(&self) -> impl Iterator<Item = &str> {
        self.context.keys().map(|k| k.as_str())
    }
}
