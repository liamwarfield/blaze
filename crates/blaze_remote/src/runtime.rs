//! The executor the remote package's futures run on.
//!
//! Java uses Guava `ListenableFuture`s and RxJava on Netty event loops plus a few executor
//! services; the port uses Rust futures on one process-wide tokio runtime. A Java method that
//! returns `ListenableFuture<T>` is an `async fn` returning `io::Result<T>`; a blocking Java
//! method (`getFromFuture(...)`, `waitForBulkTransfer(...)`) blocks the calling thread with
//! [`get_from_future`]. Blocking entry points are never called from inside a future.

use std::future::Future;
use std::io;
use std::sync::{Arc, OnceLock};

use tokio::runtime::{Builder, Runtime};

use crate::common::errors;

/// The runtime (Java: the Netty event loop group of the gRPC channels and
/// `RemoteModule.executorService`).
pub fn runtime() -> &'static Runtime {
    static RUNTIME: OnceLock<Runtime> = OnceLock::new();
    RUNTIME.get_or_init(|| {
        Builder::new_multi_thread()
            .enable_all()
            .thread_name("remote-executor")
            .build()
            .expect("cannot start the remote executor runtime")
    })
}

/// Returns the result of a future, blocking the calling thread (`getFromFuture(f,
/// cancelOnInterrupt=true)`).
///
/// When the calling thread is interrupted (`blaze_shell::thread_interrupt`: the thread's own
/// status or the one it adopted) before or while it waits, the future is cancelled and the call
/// fails with [`errors::interrupted`](crate::common::errors::interrupted), and the interrupt is
/// consumed as Java's `Future.get` consumes it (it throws `InterruptedException`, which clears the
/// flag).
///
/// PORT: Java's `f.cancel(true)` is the drop of the future: dropping a tonic call cancels the RPC
/// (an `Execute` stream, a ByteStream read or write) and every future it awaits.
// Java: com/google/devtools/build/lib/remote/util/Utils.java:97 Utils.getFromFuture
// Java: com/google/devtools/build/lib/remote/util/Utils.java:108 Utils.getFromFuture
pub fn get_from_future<T>(f: impl Future<Output = io::Result<T>>) -> io::Result<T> {
    match block_on_interruptibly(f) {
        Some(result) => result,
        // catch (InterruptedException e): `f.cancel(true)` (the future was dropped) and the
        // exception rethrown.
        None => Err(errors::interrupted()),
    }
}

/// Runs `f` to completion on the calling thread, or until the thread is interrupted
/// (`blaze_shell::thread_interrupt`: its own status or the one it adopted), before or while it
/// waits. `None` is the interruption: `f` was dropped (cancelled) and the interrupt consumed, as
/// a Java wait that throws `InterruptedException` clears the flag.
pub fn block_on_interruptibly<T>(f: impl Future<Output = T>) -> Option<T> {
    check_not_in_runtime();
    // `f.get()` throws InterruptedException at once when the flag is set (and clears it).
    if blaze_shell::thread_interrupt::interrupted() {
        // `f` is dropped unpolled.
        return None;
    }
    let notify = Arc::new(tokio::sync::Notify::new());
    let listener_notify = notify.clone();
    // The listener runs on the interrupting thread: `notify_one` stores a permit when the waiter
    // is not polling yet, so an interrupt between the check above and the wait is not lost.
    let result = blaze_shell::thread_interrupt::with_interrupt_listener(
        Arc::new(move || listener_notify.notify_one()),
        || {
            runtime().block_on(async {
                tokio::select! {
                    biased;
                    result = f => Some(result),
                    () = notify.notified() => None,
                }
            })
        },
    );
    if result.is_none() {
        // The wait threw InterruptedException, which cleared the flag.
        blaze_shell::thread_interrupt::interrupted();
    }
    result
}

/// `Thread.sleep(millis)` / `TimeUnit.MILLISECONDS.sleep(millis)`: sleeps on the calling thread;
/// an interrupt before or during the sleep ends it with an `InterruptedException` (and clears
/// the flag).
// Java: java/lang/Thread.java Thread.sleep
pub fn sleep(duration: std::time::Duration) -> io::Result<()> {
    // The timer is created inside the runtime (a `Sleep` needs its time driver).
    match block_on_interruptibly(async move { tokio::time::sleep(duration).await }) {
        Some(()) => Ok(()),
        None => Err(errors::interrupted()),
    }
}

/// Blocks on a future that cannot fail.
pub fn block_on<T>(f: impl Future<Output = T>) -> T {
    check_not_in_runtime();
    runtime().block_on(f)
}

fn check_not_in_runtime() {
    blaze_util::check_state!(
        tokio::runtime::Handle::try_current().is_err(),
        "blocking remote call from inside the remote executor"
    );
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::Duration;

    use blaze_shell::thread_interrupt::{self, ThreadInterrupt};

    use super::*;

    /// Sets `dropped` when the future it guards is dropped (Java: the future is cancelled).
    struct DropFlag(Arc<AtomicBool>);

    impl Drop for DropFlag {
        fn drop(&mut self) {
            self.0.store(true, Ordering::SeqCst);
        }
    }

    #[test]
    fn get_from_future_returns_the_result() {
        assert_eq!(get_from_future(async { Ok(7) }).unwrap(), 7);
        let e = get_from_future(async { Err::<(), _>(io::Error::other("boom")) }).unwrap_err();
        assert_eq!(e.to_string(), "boom");
    }

    #[test]
    fn interrupt_while_waiting_cancels_the_future_and_throws_interrupted() {
        let dropped = Arc::new(AtomicBool::new(false));
        let guard = DropFlag(dropped.clone());
        let thread = ThreadInterrupt::current();
        let interrupter = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(200));
            thread.interrupt();
        });
        let result: io::Result<()> = get_from_future(async move {
            let _guard = guard;
            std::future::pending::<()>().await;
            Ok(())
        });
        interrupter.join().unwrap();
        let e = result.unwrap_err();
        assert!(errors::is_interrupted(&e), "{e:?}");
        // `f.cancel(true)`: the never-completing future was dropped.
        assert!(dropped.load(Ordering::SeqCst));
        // Future.get threw InterruptedException, which cleared the flag.
        assert!(!thread_interrupt::is_current_interrupted());
    }

    #[test]
    fn interrupted_before_the_call_throws_without_polling() {
        let polled = Arc::new(AtomicBool::new(false));
        let p = polled.clone();
        thread_interrupt::interrupt_current();
        let e = get_from_future(async move {
            p.store(true, Ordering::SeqCst);
            Ok(())
        })
        .unwrap_err();
        assert!(errors::is_interrupted(&e));
        assert!(!polled.load(Ordering::SeqCst));
        assert!(!thread_interrupt::is_current_interrupted());
    }

    #[test]
    fn the_interrupt_listener_is_unregistered_after_the_call() {
        get_from_future(async { Ok(()) }).unwrap();
        // A later interrupt of this thread only sets the flag: no listener of the finished call
        // remains to wake a wait (the next call sees the flag and throws at once).
        let thread = ThreadInterrupt::current();
        std::thread::spawn(move || thread.interrupt())
            .join()
            .unwrap();
        assert!(thread_interrupt::is_current_interrupted());
        let e = get_from_future(async { Ok(()) }).unwrap_err();
        assert!(errors::is_interrupted(&e));
        assert!(!thread_interrupt::is_current_interrupted());
        assert_eq!(get_from_future(async { Ok(1) }).unwrap(), 1);
    }

    #[test]
    fn an_adopting_thread_is_interrupted_with_the_adopted_status() {
        let worker = ThreadInterrupt::current();
        let w = worker.clone();
        let helper = std::thread::spawn(move || {
            let _adopted = w.adopt();
            get_from_future(std::future::pending::<io::Result<()>>())
        });
        std::thread::sleep(Duration::from_millis(200));
        worker.interrupt();
        let e = helper.join().unwrap().unwrap_err();
        assert!(errors::is_interrupted(&e));
        assert!(!thread_interrupt::is_current_interrupted());
    }
}
