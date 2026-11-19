//! Port of `lib/remote/util/RxUtilsTest.java` (`blaze_remote::util::rx_utils`).
//!
//! PORT: a `SettableCompletable` is a future that waits for its emitter and records whether it
//! was dropped before it completed (`cancelled()`); a subscription (`.test()`) is the merged
//! future polled by hand. `Exception` errors other than `IOException` have no counterpart (every
//! transfer error is an `io::Error`; an unchecked exception is a panic), so the two tests about
//! them are ignored.

use std::future::Future;
use std::io;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::task::{Context, Poll, Waker};

use blaze_remote::common::bulk_transfer_exception::BulkTransferException;
use blaze_remote::common::errors;
use blaze_remote::runtime::get_from_future;
use blaze_remote::util::rx_utils::{merge_bulk_transfer, to_transfer_result};
use tokio::sync::oneshot;

/// `SettableCompletable`.
struct SettableCompletable {
    emitter: parking_lot::Mutex<Option<oneshot::Sender<io::Result<()>>>>,
    receiver: parking_lot::Mutex<Option<oneshot::Receiver<io::Result<()>>>>,
    cancelled: Arc<AtomicBool>,
    completed: Arc<AtomicBool>,
}

/// Sets `cancelled` when the transfer is dropped before it completed.
struct Cancellable {
    cancelled: Arc<AtomicBool>,
    completed: Arc<AtomicBool>,
}

impl Drop for Cancellable {
    fn drop(&mut self) {
        if !self.completed.load(Ordering::SeqCst) {
            self.cancelled.store(true, Ordering::SeqCst);
        }
    }
}

impl SettableCompletable {
    fn create() -> SettableCompletable {
        let (tx, rx) = oneshot::channel();
        SettableCompletable {
            emitter: parking_lot::Mutex::new(Some(tx)),
            receiver: parking_lot::Mutex::new(Some(rx)),
            cancelled: Arc::new(AtomicBool::new(false)),
            completed: Arc::new(AtomicBool::new(false)),
        }
    }

    /// The transfer (subscribing to the completable).
    fn transfer(&self) -> impl Future<Output = io::Result<()>> + Send + 'static {
        let rx = self.receiver.lock().take().expect("subscribed once");
        let guard = Cancellable {
            cancelled: self.cancelled.clone(),
            completed: self.completed.clone(),
        };
        async move {
            let _guard = guard;
            rx.await.expect("the emitter is kept")
        }
    }

    fn set_complete(&self) {
        self.completed.store(true, Ordering::SeqCst);
        let _ = self.emitter.lock().take().unwrap().send(Ok(()));
    }

    fn set_error(&self, error: io::Error) {
        self.completed.store(true, Ordering::SeqCst);
        let _ = self.emitter.lock().take().unwrap().send(Err(error));
    }

    fn cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }
}

/// `TestObserver`: a future polled by hand.
struct TestObserver<T> {
    future: Pin<Box<dyn Future<Output = T> + Send>>,
    result: Option<T>,
}

impl<T> TestObserver<T> {
    fn test(future: impl Future<Output = T> + Send + 'static) -> TestObserver<T> {
        let mut ob = TestObserver {
            future: Box::pin(future),
            result: None,
        };
        ob.poll();
        ob
    }

    fn poll(&mut self) {
        if self.result.is_none()
            && let Poll::Ready(r) = self
                .future
                .as_mut()
                .poll(&mut Context::from_waker(Waker::noop()))
        {
            self.result = Some(r);
        }
    }

    /// The outcome, after the emitters have run.
    fn result(&mut self) -> Option<&T> {
        self.poll();
        self.result.as_ref()
    }
}

#[test]
fn to_transfer_result_on_complete_is_ok() {
    let transfer = SettableCompletable::create();
    let f = transfer.transfer();
    let mut ob = TestObserver::test(async move { to_transfer_result(f.await) });

    transfer.set_complete();

    let result = ob.result().expect("a value");
    assert!(result.is_ok());
    assert!(!result.is_error());
}

#[test]
fn to_transfer_result_on_io_exception_is_error() {
    let transfer = SettableCompletable::create();
    let f = transfer.transfer();
    let mut ob = TestObserver::test(async move { to_transfer_result(f.await) });

    transfer.set_error(io::Error::other("IO error"));

    let result = ob.result().expect("a value");
    assert!(!result.is_ok());
    assert!(result.is_error());
    assert_eq!(result.get_error().unwrap().to_string(), "IO error");
}

#[test]
#[ignore = "no counterpart: every transfer error is an io::Error (an `Exception` that is not an IOException would be a panic)"]
fn to_transfer_result_on_other_error_propagate_error() {}

#[test]
fn merge_bulk_transfer_all_complete_complete() {
    let (transfer1, transfer2, transfer3) = (
        SettableCompletable::create(),
        SettableCompletable::create(),
        SettableCompletable::create(),
    );
    let mut ob = TestObserver::test(merge_bulk_transfer([
        transfer1.transfer(),
        transfer2.transfer(),
        transfer3.transfer(),
    ]));

    transfer1.set_complete();
    transfer2.set_complete();
    transfer3.set_complete();

    assert!(matches!(ob.result(), Some(Ok(()))));
}

#[test]
fn merge_bulk_transfer_has_pending_transfer_pending() {
    let (transfer1, transfer2, transfer3) = (
        SettableCompletable::create(),
        SettableCompletable::create(),
        SettableCompletable::create(),
    );
    let mut ob = TestObserver::test(merge_bulk_transfer([
        transfer1.transfer(),
        transfer2.transfer(),
        transfer3.transfer(),
    ]));

    transfer1.set_complete();
    transfer2.set_complete();

    // assertNotComplete(); assertNoErrors();
    assert!(ob.result().is_none());
}

#[test]
fn merge_bulk_transfer_on_io_errors_keep_other_transfers() {
    let (transfer1, transfer2, transfer3) = (
        SettableCompletable::create(),
        SettableCompletable::create(),
        SettableCompletable::create(),
    );
    let mut ob = TestObserver::test(merge_bulk_transfer([
        transfer1.transfer(),
        transfer2.transfer(),
        transfer3.transfer(),
    ]));

    transfer1.set_error(io::Error::other("IO error"));
    transfer2.set_complete();
    transfer3.set_complete();

    let e = ob.result().unwrap().as_ref().unwrap_err();
    assert!(errors::downcast_ref::<BulkTransferException>(e).is_some());
    assert!(!transfer2.cancelled());
    assert!(!transfer3.cancelled());
}

#[test]
fn merge_bulk_transfer_on_io_errors_wraps_io_errors_in_bulk_transfer_exceptions() {
    let (transfer1, transfer2, transfer3) = (
        SettableCompletable::create(),
        SettableCompletable::create(),
        SettableCompletable::create(),
    );
    let mut ob = TestObserver::test(merge_bulk_transfer([
        transfer1.transfer(),
        transfer2.transfer(),
        transfer3.transfer(),
    ]));

    transfer1.set_error(io::Error::other("IO error 1"));
    transfer2.set_error(io::Error::other("IO error 2"));
    transfer3.set_complete();

    let e = ob.result().unwrap().as_ref().unwrap_err();
    let bulk_transfer_exception =
        errors::downcast_ref::<BulkTransferException>(e).expect("BulkTransferException");
    // containsExactly(error1, error2): in any order.
    let mut suppressed: Vec<String> = bulk_transfer_exception
        .get_suppressed()
        .iter()
        .map(|e| e.to_string())
        .collect();
    suppressed.sort();
    assert_eq!(suppressed, vec!["IO error 1", "IO error 2"]);
}

#[test]
#[ignore = "no counterpart: every transfer error is an io::Error (an `Exception` that is not an IOException would be a panic)"]
fn merge_bulk_transfer_on_other_error_cancel_other_transfers() {}

#[test]
fn merge_bulk_transfer_on_interruption_cancel_other_transfers() {
    let (transfer1, transfer2, transfer3) = (
        SettableCompletable::create(),
        SettableCompletable::create(),
        SettableCompletable::create(),
    );

    blaze_shell::thread_interrupt::interrupt_current();
    // `blockingAwait()` is `getFromFuture` here.
    let error = get_from_future(merge_bulk_transfer([
        transfer1.transfer(),
        transfer2.transfer(),
        transfer3.transfer(),
    ]))
    .unwrap_err();

    assert!(errors::is_interrupted(&error));
    assert!(transfer1.cancelled());
    assert!(transfer2.cancelled());
    assert!(transfer3.cancelled());
}
