//! Port of `QueryEnvironment.QueryTaskFuture`, `QueryTaskCallable`, `QueryTaskAsyncCallable` and
//! the future-composition methods of `QueryEnvironment` (`immediateSuccessfulFuture`, `execute`,
//! `whenSucceedsCall`, `whenAllSucceed`, `transformAsync`, ...).
//!
//! PORT: immediate futures. Every query environment Blaze ports (`BlazeQueryEnvironment`,
//! `ConfiguredTargetQueryEnvironment`, `ActionGraphQueryEnvironment`, all subclasses of
//! `AbstractBlazeQueryEnvironment`) evaluates on the calling thread with Guava's
//! `directExecutor()`, so every `QueryTaskFuture` is already complete when it is returned.
//! A [`QueryTaskFuture`] is therefore a completed outcome (success, failure or cancellation), and
//! the composition methods (an extension trait on every [`QueryEnvironment`]) are the ports of
//! `AbstractBlazeQueryEnvironment`'s implementations together with the Guava semantics they rely
//! on:
//!
//! - `Futures.whenAllSucceed(fs).call(c)`: the first input (in order) that failed makes the result
//!   fail with its cause; the first that was cancelled cancels the result; otherwise `c` runs. A
//!   callable that throws fails the result with that exception, except that a thrown
//!   `CancellationException` cancels it (`CombinedFuture` `afterRanInterruptiblyFailure`).
//! - `getChecked` turns cancellation into `InterruptedException`; `transformAsync`'s fast path
//!   turns a failed-with-`InterruptedException` input into a cancelled future.
//!
//! The ported query code keeps Java's structure: expressions build futures with
//! `env.eval(..)`/`env.execute(..)` and combine them with `env.when_succeeds_call(..)` etc.

use crate::query_environment::QueryEnvironment;
use crate::query_exception::{InterruptedException, QueryError, QueryException};

#[derive(Debug)]
enum State<R> {
    Success(R),
    /// A failed future: Java's `ExecutionException` cause (a `QueryException` or an
    /// `InterruptedException`).
    Failed(QueryError),
    Cancelled,
}

/// An asynchronous computation of part of a query evaluation.
///
/// This class is deliberately opaque; the only ways to compose/use `QueryTaskFuture` instances
/// are the helper methods like [`QueryEnvironmentFutures::when_succeeds_call`]. A crucial
/// consequence of this is there is no way for a `QueryExpression` or `QueryFunction`
/// implementation to block on the result of a `QueryTaskFuture`. This eliminates a large class of
/// deadlocks by design!
///
/// See the module documentation for the immediate-future port.
// Java: com/google/devtools/build/lib/query2/engine/QueryEnvironment.java:379 QueryEnvironment.QueryTaskFuture
#[derive(Debug)]
#[must_use]
pub struct QueryTaskFuture<R> {
    state: State<R>,
}

/// The outcome of a `QueryTaskFuture<?>`, independent of its value type.
#[derive(Debug, Clone, Copy)]
pub enum QueryTaskFutureOutcome<'a> {
    Success,
    Failed(&'a QueryError),
    Cancelled,
}

/// A `QueryTaskFuture<?>`: any future, as `whenAllSucceed` and friends accept.
pub trait AnyQueryTaskFuture {
    fn outcome(&self) -> QueryTaskFutureOutcome<'_>;
}

impl<R> AnyQueryTaskFuture for QueryTaskFuture<R> {
    fn outcome(&self) -> QueryTaskFutureOutcome<'_> {
        match &self.state {
            State::Success(_) => QueryTaskFutureOutcome::Success,
            State::Failed(e) => QueryTaskFutureOutcome::Failed(e),
            State::Cancelled => QueryTaskFutureOutcome::Cancelled,
        }
    }
}

impl<R> QueryTaskFuture<R> {
    fn success(value: R) -> Self {
        QueryTaskFuture {
            state: State::Success(value),
        }
    }

    fn failed(e: QueryError) -> Self {
        match e {
            // A CancellationException cancels the future instead of failing it.
            QueryError::Cancellation => Self::cancelled(),
            e => QueryTaskFuture {
                state: State::Failed(e),
            },
        }
    }

    fn cancelled() -> Self {
        QueryTaskFuture {
            state: State::Cancelled,
        }
    }

    /// If this `QueryTaskFuture`'s encapsulated computation is currently complete and
    /// successful, returns the result. This method is intended to be used in combination with
    /// `whenSucceedsCall`.
    ///
    /// Panics (Java `IllegalStateException`) if the future failed or was cancelled.
    // Java: com/google/devtools/build/lib/query2/common/AbstractBlazeQueryEnvironment.java:620 QueryTaskFutureImpl.getIfSuccessful
    pub fn get_if_successful(&self) -> &R {
        match &self.state {
            State::Success(v) => v,
            State::Failed(e) => {
                panic!("IllegalStateException: java.util.concurrent.ExecutionException: {e}")
            }
            State::Cancelled => {
                panic!("IllegalStateException: java.util.concurrent.CancellationException")
            }
        }
    }

    /// `getIfSuccessful` that moves the value out.
    pub fn into_if_successful(self) -> R {
        match self.state {
            State::Success(v) => v,
            State::Failed(e) => {
                panic!("IllegalStateException: java.util.concurrent.ExecutionException: {e}")
            }
            State::Cancelled => {
                panic!("IllegalStateException: java.util.concurrent.CancellationException")
            }
        }
    }

    /// Java `QueryTaskFutureImpl.getChecked`: the value, the failure's `QueryException` or
    /// `InterruptedException`, or `InterruptedException` for a cancelled future.
    // Java: com/google/devtools/build/lib/query2/common/AbstractBlazeQueryEnvironment.java:628 QueryTaskFutureImpl.getChecked
    pub fn get_checked(self) -> Result<R, QueryError> {
        match self.state {
            State::Success(v) => Ok(v),
            State::Cancelled => Err(QueryError::Interrupted(InterruptedException::new())),
            State::Failed(e) => Err(e),
        }
    }

    /// Java `Future.isDone()`: always true for an immediate future.
    pub fn is_done(&self) -> bool {
        true
    }

    /// Java `Future.isCancelled()`.
    pub fn is_cancelled(&self) -> bool {
        matches!(self.state, State::Cancelled)
    }

    /// Java's frequent `try { ...; return immediateSuccessfulFuture(v); } catch (QueryException e)
    /// { return immediateFailedFuture(e); } catch (InterruptedException e) { return
    /// immediateCancelledFuture(); }`.
    ///
    /// A `CancellationException` (unchecked in Java) is not caught there; it propagates as a
    /// panic.
    pub fn from_caught(result: Result<R, QueryError>) -> Self {
        match result {
            Ok(v) => Self::success(v),
            Err(QueryError::Query(e)) => Self::failed(QueryError::Query(e)),
            Err(QueryError::Interrupted(_)) => Self::cancelled(),
            Err(QueryError::Cancellation) => panic!("java.util.concurrent.CancellationException"),
        }
    }
}

/// Collects `&dyn AnyQueryTaskFuture` views of a list of futures (Java passes
/// `Iterable<? extends QueryTaskFuture<?>>`).
pub fn as_any_futures<R>(futures: &[QueryTaskFuture<R>]) -> Vec<&dyn AnyQueryTaskFuture> {
    futures
        .iter()
        .map(|f| f as &dyn AnyQueryTaskFuture)
        .collect()
}

/// Guava `Futures.whenAllSucceed(futures)`: the first non-successful input in order decides.
fn first_unsuccessful(futures: &[&dyn AnyQueryTaskFuture]) -> Option<QueryTaskFuture<()>> {
    for f in futures {
        match f.outcome() {
            QueryTaskFutureOutcome::Success => {}
            QueryTaskFutureOutcome::Failed(e) => return Some(QueryTaskFuture::failed(e.clone())),
            QueryTaskFutureOutcome::Cancelled => return Some(QueryTaskFuture::cancelled()),
        }
    }
    None
}

fn rethrow<R>(f: QueryTaskFuture<()>) -> QueryTaskFuture<R> {
    match f.state {
        State::Success(()) => unreachable!(),
        State::Failed(e) => QueryTaskFuture::failed(e),
        State::Cancelled => QueryTaskFuture::cancelled(),
    }
}

/// Runs a `QueryTaskCallable` inside a Guava combined future: its exception fails the future,
/// except `CancellationException`, which cancels it.
fn run_callable<R>(callable: impl FnOnce() -> Result<R, QueryError>) -> QueryTaskFuture<R> {
    match callable() {
        Ok(v) => QueryTaskFuture::success(v),
        Err(e) => QueryTaskFuture::failed(e),
    }
}

/// The future-composition methods of Java's `QueryEnvironment`, available on every query
/// environment (including `dyn QueryEnvironment<T>`).
///
/// A `QueryTaskCallable<R>` is `FnOnce() -> Result<R, QueryError>`; a `QueryTaskAsyncCallable<R>`
/// is `FnOnce() -> QueryTaskFuture<R>`.
pub trait QueryEnvironmentFutures<T> {
    /// Returns a `QueryTaskFuture` representing the successful computation of `value`.
    // Java: com/google/devtools/build/lib/query2/engine/QueryEnvironment.java:402 QueryEnvironment.immediateSuccessfulFuture
    fn immediate_successful_future<R>(&self, value: R) -> QueryTaskFuture<R> {
        QueryTaskFuture::success(value)
    }

    /// Returns a `QueryTaskFuture` representing a computation that was unsuccessful because of
    /// `e`.
    // Java: com/google/devtools/build/lib/query2/engine/QueryEnvironment.java:411 QueryEnvironment.immediateFailedFuture
    fn immediate_failed_future<R>(&self, e: QueryException) -> QueryTaskFuture<R> {
        QueryTaskFuture::failed(QueryError::Query(e))
    }

    /// Returns a `QueryTaskFuture` representing a cancelled computation.
    // Java: com/google/devtools/build/lib/query2/engine/QueryEnvironment.java:419 QueryEnvironment.immediateCancelledFuture
    fn immediate_cancelled_future<R>(&self) -> QueryTaskFuture<R> {
        QueryTaskFuture::cancelled()
    }

    /// Returns a `QueryTaskFuture` representing the given computation `callable` being performed
    /// asynchronously.
    // Java: com/google/devtools/build/lib/query2/common/AbstractBlazeQueryEnvironment.java:270 AbstractBlazeQueryEnvironment.execute
    fn execute<R>(&self, callable: impl FnOnce() -> Result<R, QueryError>) -> QueryTaskFuture<R> {
        QueryTaskFuture::from_caught(callable())
    }

    /// Returns a `QueryTaskFuture` representing both the given `callable` being performed
    /// asynchronously and also the returned `QueryTaskFuture` returned therein being completed.
    // Java: com/google/devtools/build/lib/query2/common/AbstractBlazeQueryEnvironment.java:281 AbstractBlazeQueryEnvironment.executeAsync
    fn execute_async<R>(
        &self,
        callable: impl FnOnce() -> QueryTaskFuture<R>,
    ) -> QueryTaskFuture<R> {
        callable()
    }

    /// Returns a `QueryTaskFuture` representing the given computation `callable` being performed
    /// after the successful completion of the computation encapsulated by the given `future` has
    /// completed successfully.
    // Java: com/google/devtools/build/lib/query2/common/AbstractBlazeQueryEnvironment.java:286 AbstractBlazeQueryEnvironment.whenSucceedsCall
    fn when_succeeds_call<R>(
        &self,
        future: &dyn AnyQueryTaskFuture,
        callable: impl FnOnce() -> Result<R, QueryError>,
    ) -> QueryTaskFuture<R> {
        self.when_all_succeed_call(&[future], callable)
    }

    /// Returns a `QueryTaskFuture` representing the successful completion of all the computations
    /// encapsulated by the given `futures`.
    // Java: com/google/devtools/build/lib/query2/common/AbstractBlazeQueryEnvironment.java:292 AbstractBlazeQueryEnvironment.whenAllSucceed
    fn when_all_succeed(&self, futures: &[&dyn AnyQueryTaskFuture]) -> QueryTaskFuture<()> {
        // Dummy.INSTANCE
        self.when_all_succeed_call(futures, || Ok(()))
    }

    /// Returns a `QueryTaskFuture` representing the given computation `callable` being performed
    /// after the successful completion of all the computations encapsulated by the given
    /// `futures`.
    // Java: com/google/devtools/build/lib/query2/common/AbstractBlazeQueryEnvironment.java:297 AbstractBlazeQueryEnvironment.whenAllSucceedCall
    fn when_all_succeed_call<R>(
        &self,
        futures: &[&dyn AnyQueryTaskFuture],
        callable: impl FnOnce() -> Result<R, QueryError>,
    ) -> QueryTaskFuture<R> {
        if let Some(unsuccessful) = first_unsuccessful(futures) {
            return rethrow(unsuccessful);
        }
        run_callable(callable)
    }

    /// Returns a `QueryTaskFuture` representing the given computation `callable` being performed
    /// after the successful completion or cancellation of the computations encapsulated by the
    /// given `future`.
    // Java: com/google/devtools/build/lib/query2/common/AbstractBlazeQueryEnvironment.java:571 QueryTaskFutureImpl.whenSucceedsOrIsCancelledCall
    fn when_succeeds_or_is_cancelled_call<R>(
        &self,
        future: &dyn AnyQueryTaskFuture,
        callable: impl FnOnce() -> Result<R, QueryError>,
    ) -> QueryTaskFuture<R> {
        match future.outcome() {
            // If the input future is cancelled, we are supposed to swallow the
            // `CancellationException` and proceed normally.
            QueryTaskFutureOutcome::Success | QueryTaskFutureOutcome::Cancelled => {
                run_callable(callable)
            }
            // future.get() throws ExecutionException; CombinedFuture fails with its cause.
            QueryTaskFutureOutcome::Failed(e) => QueryTaskFuture::failed(e.clone()),
        }
    }

    /// Returns a `QueryTaskFuture` representing the asynchronous application of the given
    /// `function` to the value produced by the computation encapsulated by the given `future`.
    // Java: com/google/devtools/build/lib/query2/common/AbstractBlazeQueryEnvironment.java:311 AbstractBlazeQueryEnvironment.transformAsync
    fn transform_async<T1, T2>(
        &self,
        future: QueryTaskFuture<T1>,
        function: impl FnOnce(T1) -> QueryTaskFuture<T2>,
    ) -> QueryTaskFuture<T2> {
        // Due to how our subclasses use single-threaded query engines, in practice futureImpl
        // will always already be done. Therefore this is a fast-path to make it harder to stack
        // overflow on deeply nested expressions whose evaluation involves #transformAsync.
        let t1 = match future.get_checked() {
            Ok(t1) => t1,
            Err(QueryError::Query(e)) => return self.immediate_failed_future(e),
            Err(QueryError::Interrupted(_)) => return self.immediate_cancelled_future(),
            Err(QueryError::Cancellation) => unreachable!("not stored in a future"),
        };
        function(t1)
    }
}

impl<T, E: QueryEnvironment<T> + ?Sized> QueryEnvironmentFutures<T> for E {}
