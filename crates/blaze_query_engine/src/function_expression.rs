//! Port of `query2/engine/FunctionExpression.java`.

use std::any::Any;
use std::fmt;
use std::sync::Arc;

use crate::callback::Callback;
use crate::query_environment::{
    Argument, ArgumentType, QueryEnvironment, QueryFunction, QueryFunctionBase,
};
use crate::query_expression::QueryExpression;
use crate::query_expression_context::QueryExpressionContext;
use crate::query_task_future::QueryTaskFuture;

/// A query expression for user-defined query functions.
///
/// PORT: Java's `QueryFunction` is generic in its `eval` method; here the expression keeps the
/// function twice: as the `T`-independent [`QueryFunctionBase`] (`get_function`) and as the
/// environment's `Arc<dyn QueryFunction<T>>`, type-erased, which `eval` recovers for the
/// environment's `T` (see `query_environment` module documentation).
// Java: com/google/devtools/build/lib/query2/engine/FunctionExpression.java:32 FunctionExpression
pub struct FunctionExpression {
    function: Arc<dyn QueryFunctionBase>,
    /// An `Arc<dyn QueryFunction<T>>` for the `T` of the environment that parsed this.
    typed_function: Arc<dyn Any + Send + Sync>,
    args: Vec<Argument>,
}

impl FunctionExpression {
    // Java: com/google/devtools/build/lib/query2/engine/FunctionExpression.java:36 FunctionExpression.<init>
    pub fn new<T: 'static>(
        function: Arc<dyn QueryFunction<T>>,
        args: Vec<Argument>,
    ) -> FunctionExpression {
        FunctionExpression {
            function: function.clone(),
            typed_function: Arc::new(function),
            args,
        }
    }

    /// Java `new FunctionExpression(functionExpression.getFunction(), args)`: the same function
    /// with other arguments (as `QueryExpressionMapper` builds).
    pub fn with_args(&self, args: Vec<Argument>) -> FunctionExpression {
        FunctionExpression {
            function: self.function.clone(),
            typed_function: self.typed_function.clone(),
            args,
        }
    }

    // Java: com/google/devtools/build/lib/query2/engine/FunctionExpression.java:41 FunctionExpression.getFunction
    pub fn get_function(&self) -> &dyn QueryFunctionBase {
        &*self.function
    }

    /// The function as a `QueryFunction<T>` (for `asFilteringFunction()` and other `T`-specific
    /// uses). Panics if this expression was parsed by an environment of another node type.
    pub fn get_typed_function<T: 'static>(&self) -> &Arc<dyn QueryFunction<T>> {
        self.typed_function
            .downcast_ref::<Arc<dyn QueryFunction<T>>>()
            .unwrap_or_else(|| {
                panic!(
                    "IllegalStateException: function '{}' was parsed for another query \
                     environment type",
                    self.function.get_name()
                )
            })
    }

    // Java: com/google/devtools/build/lib/query2/engine/FunctionExpression.java:45 FunctionExpression.getArgs
    pub fn get_args(&self) -> &[Argument] {
        &self.args
    }

    /// `this` is the `QueryExpression` wrapping this node (the function's `expression`).
    // Java: com/google/devtools/build/lib/query2/engine/FunctionExpression.java:50 FunctionExpression.eval
    pub fn eval<T: Clone + 'static>(
        &self,
        this: &QueryExpression,
        env: &dyn QueryEnvironment<T>,
        context: &QueryExpressionContext<T>,
        callback: &dyn Callback<T>,
    ) -> QueryTaskFuture<()> {
        // PORT: Profiler.instance().profile("function.eval/" + name) skipped (profiling).
        self.get_typed_function::<T>()
            .eval(env, context, this, &self.args, callback)
    }

    // Java: com/google/devtools/build/lib/query2/engine/FunctionExpression.java:61 FunctionExpression.collectTargetPatterns
    pub fn collect_target_patterns(&self, literals: &mut dyn FnMut(String)) {
        for arg in &self.args {
            if arg.get_type() == ArgumentType::Expression {
                arg.get_expression().collect_target_patterns(literals);
            }
        }
    }
}

impl fmt::Display for FunctionExpression {
    // Java: com/google/devtools/build/lib/query2/engine/FunctionExpression.java:75 FunctionExpression.toString
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}(", self.function.get_name())?;
        for (i, arg) in self.args.iter().enumerate() {
            if i > 0 {
                f.write_str(", ")?;
            }
            write!(f, "{arg}")?;
        }
        f.write_str(")")
    }
}
