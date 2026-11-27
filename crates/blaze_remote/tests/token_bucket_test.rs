//! Port of `lib/remote/grpc/TokenBucketTest.java`.
//!
//! PORT: `TestObserver` subscribes when it is made; here `acquire_token()` registers its waiter
//! when it is called, and `Observer::poll` looks at the future without waiting.
// Java: com/google/devtools/build/lib/remote/grpc/TokenBucketTest.java:31 TokenBucketTest

use std::future::Future;
use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};

use blaze_remote::grpc::connection_pool::TokenBucket;

/// `bucket.acquireToken().test()`.
struct Observer(
    Pin<Box<dyn Future<Output = io::Result<i32>> + Send>>,
    Option<io::Result<i32>>,
);

impl Observer {
    fn new(future: impl Future<Output = io::Result<i32>> + Send + 'static) -> Observer {
        Observer(Box::pin(future), None)
    }

    fn poll(&mut self) -> Option<&io::Result<i32>> {
        if self.1.is_none() {
            let mut cx = Context::from_waker(futures::task::noop_waker_ref());
            if let Poll::Ready(result) = self.0.as_mut().poll(&mut cx) {
                self.1 = Some(result);
            }
        }
        self.1.as_ref()
    }

    /// `assertValue(token).assertComplete()`.
    fn assert_value(&mut self, token: i32) {
        match self.poll() {
            Some(Ok(t)) => assert_eq!(*t, token),
            other => panic!("expected {token}, got {other:?}"),
        }
    }

    /// `assertEmpty()`.
    fn assert_empty(&mut self) {
        assert!(self.poll().is_none(), "{:?}", self.1);
    }

    /// `assertError(e -> e instanceof IllegalStateException && e.getMessage().contains("closed"))`.
    fn assert_closed_error(&mut self) {
        match self.poll() {
            Some(Err(e)) => {
                assert!(e.to_string().starts_with("IllegalStateException"), "{e}");
                assert!(e.to_string().contains("closed"), "{e}");
            }
            other => panic!("expected the closed error, got {other:?}"),
        }
    }
}

fn acquire(bucket: &TokenBucket<i32>) -> Observer {
    Observer::new(bucket.acquire_token())
}

#[test]
fn acquire_token_smoke() {
    let bucket = TokenBucket::<i32>::default();
    assert_eq!(bucket.size(), 0);
    bucket.add_token(0);
    assert_eq!(bucket.size(), 1);
    acquire(&bucket).assert_value(0);
    assert_eq!(bucket.size(), 0);
}

#[test]
fn acquire_token_release_initial_tokens() {
    let bucket = TokenBucket::new(vec![0]);
    assert_eq!(bucket.size(), 1);
    acquire(&bucket).assert_value(0);
    assert_eq!(bucket.size(), 0);
}

#[test]
fn acquire_token_multiple_initial_tokens_release_first_token() {
    let bucket = TokenBucket::new(vec![0, 1]);
    assert_eq!(bucket.size(), 2);
    acquire(&bucket).assert_value(0);
    assert_eq!(bucket.size(), 1);
}

#[test]
fn acquire_token_multiple_initial_tokens_release_second_token() {
    let bucket = TokenBucket::new(vec![0, 1]);
    assert_eq!(bucket.size(), 2);
    acquire(&bucket).assert_value(0);
    acquire(&bucket).assert_value(1);
    assert_eq!(bucket.size(), 0);
}

#[test]
fn acquire_token_release_token_to_previous_observer() {
    let bucket = TokenBucket::<i32>::default();
    let mut observer = acquire(&bucket);
    observer.assert_empty();
    bucket.add_token(0);
    observer.assert_value(0);
    assert_eq!(bucket.size(), 0);
}

#[test]
fn acquire_token_not_release_token_to_disposed_observer() {
    let bucket = TokenBucket::<i32>::default();
    let observer = acquire(&bucket);
    // observer.dispose()
    drop(observer);
    bucket.add_token(0);
    assert_eq!(bucket.size(), 1);
}

#[test]
fn acquire_token_dispose_after_token_acquired() {
    let bucket = TokenBucket::<i32>::default();
    let mut observer = acquire(&bucket);
    bucket.add_token(0);
    bucket.add_token(1);
    observer.assert_value(0);
    assert_eq!(bucket.size(), 1);
}

#[test]
fn acquire_token_multiple_observers_only_one_can_acquire() {
    let bucket = TokenBucket::<i32>::default();
    let mut observer1 = acquire(&bucket);
    let mut observer2 = acquire(&bucket);
    bucket.add_token(0);
    if observer1.poll().is_some() {
        observer1.assert_value(0);
        observer2.assert_empty();
        bucket.add_token(1);
        observer2.assert_value(1);
    } else {
        observer1.assert_empty();
        observer2.assert_value(0);
        bucket.add_token(1);
        observer1.assert_value(1);
    }
}

#[test]
fn acquire_token_re_subscription_wait_available_token() {
    let bucket = TokenBucket::<i32>::default();
    bucket.add_token(0);
    // Two subscriptions of one `Single`: two calls.
    let mut observer1 = acquire(&bucket);
    let mut observer2 = acquire(&bucket);
    observer1.assert_value(0);
    observer2.assert_empty();
}

#[test]
fn acquire_token_re_subscription_acquire_new_token() {
    let bucket = TokenBucket::<i32>::default();
    bucket.add_token(0);
    let mut observer1 = acquire(&bucket);
    let mut observer2 = acquire(&bucket);
    bucket.add_token(1);
    observer1.assert_value(0);
    observer2.assert_value(1);
}

#[test]
fn acquire_token_re_subscription_acquire_next_token() {
    let bucket = TokenBucket::<i32>::default();
    bucket.add_token(0);
    bucket.add_token(1);
    let mut observer1 = acquire(&bucket);
    let mut observer2 = acquire(&bucket);
    observer1.assert_value(0);
    observer2.assert_value(1);
}

#[test]
fn acquire_token_disposed_token_remains() {
    let bucket = TokenBucket::<i32>::default();
    let mut observer = acquire(&bucket);
    observer.assert_empty();
    drop(observer);
    bucket.add_token(0);
    assert_eq!(bucket.size(), 1);
}

#[test]
fn close_error_after_close() {
    let bucket = TokenBucket::<i32>::default();
    bucket.add_token(0);
    bucket.close();
    acquire(&bucket).assert_closed_error();
}

#[test]
fn close_error_previous_observers() {
    let bucket = TokenBucket::<i32>::default();
    let mut observer = acquire(&bucket);
    bucket.close();
    observer.assert_closed_error();
}
