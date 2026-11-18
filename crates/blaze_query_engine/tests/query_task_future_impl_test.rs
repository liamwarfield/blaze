//! Port of `src/test/java/com/google/devtools/build/lib/query2/common/QueryTaskFutureImplTest.java`
//! (the future semantics Blaze's environments share), plus checks of the Guava combinator
//! semantics `AbstractBlazeQueryEnvironment` relies on.
//!
//! PORT: Java completes a `SettableFuture` after creating the dependent future; Blaze's futures
//! are immediate, so the input future is complete when the dependent one is built.

mod support;

use std::panic::AssertUnwindSafe;

use blaze_protos::failure_details::action_query;
use blaze_query_engine::query_task_future::QueryTaskFutureOutcome;
use blaze_query_engine::{
    AnyQueryTaskFuture, QueryEnvironment, QueryEnvironmentFutures, QueryError, QueryException,
    QueryTaskFuture,
};
use support::MockEnvironment;

fn env() -> MockEnvironment {
    MockEnvironment { functions: vec![] }
}

fn as_env(env: &MockEnvironment) -> &dyn QueryEnvironment<()> {
    env
}

#[test]
fn when_succeeds_or_is_cancelled_call_input_future_success() {
    let env = env();
    let env = as_env(&env);
    let input_future: QueryTaskFuture<()> = env.immediate_successful_future(());

    let next_future =
        env.when_succeeds_or_is_cancelled_call(&input_future, || Ok("Callback Return"));

    assert_eq!(*next_future.get_if_successful(), "Callback Return");
}

#[test]
fn when_succeeds_or_cancels_call_input_future_cancels() {
    let env = env();
    let env = as_env(&env);
    let input_query_task_future_impl: QueryTaskFuture<()> = env.immediate_cancelled_future();

    let next_future = env.when_succeeds_or_is_cancelled_call(&input_query_task_future_impl, || {
        Ok("Callback Return")
    });

    assert_eq!(*next_future.get_if_successful(), "Callback Return");
}

#[test]
fn when_succeeds_or_cancels_call_input_future_fails() {
    let env = env();
    let env = as_env(&env);
    let query_exception = QueryException::with_action_query_code(
        None,
        "Deliberate failure",
        action_query::Code::ActionQueryUnknown,
    );
    let input_future: QueryTaskFuture<()> = env.immediate_failed_future(query_exception);

    let next_future =
        env.when_succeeds_or_is_cancelled_call(&input_future, || Ok("Callback Return"));

    match next_future.outcome() {
        QueryTaskFutureOutcome::Failed(QueryError::Query(cause)) => {
            assert!(cause.get_message().contains("Deliberate failure"));
        }
        other => panic!("expected a failed future, got {other:?}"),
    }

    let thrown_from_get_if_successful = std::panic::catch_unwind(AssertUnwindSafe(|| {
        let _ = next_future.get_if_successful();
    }))
    .unwrap_err();
    let message = thrown_from_get_if_successful
        .downcast_ref::<String>()
        .cloned()
        .unwrap_or_default();
    assert!(message.starts_with("IllegalStateException"), "{message}");
    assert!(message.contains("Deliberate failure"), "{message}");
}

#[test]
fn when_all_succeed_call_first_unsuccessful_input_decides() {
    let env = env();
    let env = as_env(&env);
    let ok: QueryTaskFuture<()> = env.immediate_successful_future(());
    let cancelled: QueryTaskFuture<()> = env.immediate_cancelled_future();
    let failed: QueryTaskFuture<()> =
        env.immediate_failed_future(QueryException::with_action_query_code(
            None,
            "failure",
            action_query::Code::ActionQueryUnknown,
        ));

    let mut ran = false;
    let f = env.when_all_succeed_call(&[&ok, &cancelled, &failed], || {
        ran = true;
        Ok(())
    });
    assert!(f.is_cancelled());
    assert!(!ran);

    let f = env.when_all_succeed(&[&ok, &failed, &cancelled]);
    assert!(matches!(
        f.outcome(),
        QueryTaskFutureOutcome::Failed(QueryError::Query(_))
    ));
}

#[test]
fn callable_cancellation_exception_cancels_the_future() {
    let env = env();
    let env = as_env(&env);
    let ok: QueryTaskFuture<()> = env.immediate_successful_future(());
    let f: QueryTaskFuture<()> =
        env.when_succeeds_or_is_cancelled_call(&ok, || Err(QueryError::Cancellation));
    assert!(f.is_cancelled());
    // getChecked turns cancellation into InterruptedException.
    assert!(matches!(f.get_checked(), Err(QueryError::Interrupted(_))));
}

#[test]
fn transform_async_turns_interrupted_failure_into_cancellation() {
    let env = env();
    let env = as_env(&env);
    let interrupted: QueryTaskFuture<()> = env.when_all_succeed_call(&[], || {
        Err(QueryError::Interrupted(
            blaze_query_engine::InterruptedException::new(),
        ))
    });
    assert!(matches!(
        interrupted.outcome(),
        QueryTaskFutureOutcome::Failed(QueryError::Interrupted(_))
    ));
    let mut ran = false;
    let f = env.transform_async(interrupted, |()| {
        ran = true;
        env.immediate_successful_future(())
    });
    assert!(f.is_cancelled());
    assert!(!ran);
}

#[test]
fn execute_turns_interrupted_into_cancellation() {
    let env = env();
    let env = as_env(&env);
    let f: QueryTaskFuture<()> = env.execute(|| {
        Err(QueryError::Interrupted(
            blaze_query_engine::InterruptedException::new(),
        ))
    });
    assert!(f.is_cancelled());
}
