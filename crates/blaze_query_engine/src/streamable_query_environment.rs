//! Port of `query2/engine/StreamableQueryEnvironment.java`.

use crate::callback::Callback;
use crate::query_environment::QueryEnvironment;
use crate::query_expression::QueryExpression;
use crate::query_expression_context::QueryExpressionContext;
use crate::query_task_future::QueryTaskFuture;

/// The environment of a Blaze query which supports predefined streaming operations.
///
/// Only `SkyQueryEnvironment` implements it in Bazel; Blaze's environments return `None` from
/// [`QueryEnvironment::as_streamable`].
// Java: com/google/devtools/build/lib/query2/engine/StreamableQueryEnvironment.java:21 StreamableQueryEnvironment
pub trait StreamableQueryEnvironment<T>: QueryEnvironment<T> {
    // Java: com/google/devtools/build/lib/query2/engine/StreamableQueryEnvironment.java:22 StreamableQueryEnvironment.getAllRdepsBoundedParallel
    fn get_all_rdeps_bounded_parallel(
        &self,
        expression: &QueryExpression,
        depth: i32,
        context: &QueryExpressionContext<T>,
        callback: &dyn Callback<T>,
    ) -> QueryTaskFuture<()>;

    // Java: com/google/devtools/build/lib/query2/engine/StreamableQueryEnvironment.java:28 StreamableQueryEnvironment.getAllRdepsUnboundedParallel
    fn get_all_rdeps_unbounded_parallel(
        &self,
        expression: &QueryExpression,
        context: &QueryExpressionContext<T>,
        callback: &dyn Callback<T>,
    ) -> QueryTaskFuture<()>;

    // Java: com/google/devtools/build/lib/query2/engine/StreamableQueryEnvironment.java:31 StreamableQueryEnvironment.getRdepsBoundedParallel
    fn get_rdeps_bounded_parallel(
        &self,
        expression: &QueryExpression,
        depth: i32,
        universe: &QueryExpression,
        context: &QueryExpressionContext<T>,
        callback: &dyn Callback<T>,
    ) -> QueryTaskFuture<()>;

    // Java: com/google/devtools/build/lib/query2/engine/StreamableQueryEnvironment.java:38 StreamableQueryEnvironment.getRdepsUnboundedParallel
    fn get_rdeps_unbounded_parallel(
        &self,
        expression: &QueryExpression,
        universe: &QueryExpression,
        context: &QueryExpressionContext<T>,
        callback: &dyn Callback<T>,
    ) -> QueryTaskFuture<()>;

    // Java: com/google/devtools/build/lib/query2/engine/StreamableQueryEnvironment.java:44 StreamableQueryEnvironment.getDepsUnboundedParallel
    fn get_deps_unbounded_parallel(
        &self,
        expression: &QueryExpression,
        context: &QueryExpressionContext<T>,
        callback: &dyn Callback<T>,
        caller: &QueryExpression,
    ) -> QueryTaskFuture<()>;

    // TODO(bazel-team): Make this parallel.
    // Java: com/google/devtools/build/lib/query2/engine/StreamableQueryEnvironment.java:51 StreamableQueryEnvironment.getDepsBounded
    fn get_deps_bounded(
        &self,
        expression: &QueryExpression,
        context: &QueryExpressionContext<T>,
        callback: &dyn Callback<T>,
        depth: i32,
        caller: &QueryExpression,
    ) -> QueryTaskFuture<()>;

    // Java: com/google/devtools/build/lib/query2/engine/StreamableQueryEnvironment.java:58 StreamableQueryEnvironment.somePath
    fn some_path(
        &self,
        from_expression: &QueryExpression,
        to_expression: &QueryExpression,
        context: &QueryExpressionContext<T>,
        callback: &dyn Callback<T>,
        caller: &QueryExpression,
    ) -> QueryTaskFuture<()>;

    // Java: com/google/devtools/build/lib/query2/engine/StreamableQueryEnvironment.java:65 StreamableQueryEnvironment.allPaths
    fn all_paths(
        &self,
        from_expression: &QueryExpression,
        to_expression: &QueryExpression,
        context: &QueryExpressionContext<T>,
        callback: &dyn Callback<T>,
        caller: &QueryExpression,
    ) -> QueryTaskFuture<()>;
}
