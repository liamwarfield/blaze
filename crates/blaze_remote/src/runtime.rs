//! The executor the remote package's futures run on.
//!
//! Java uses Guava `ListenableFuture`s and RxJava on Netty event loops plus a few executor
//! services; the port uses Rust futures on one process-wide tokio runtime. A Java method that
//! returns `ListenableFuture<T>` is an `async fn` returning `io::Result<T>`; a blocking Java
//! method (`getFromFuture(...)`, `waitForBulkTransfer(...)`) blocks the calling thread with
//! [`get_from_future`]. Blocking entry points are never called from inside a future.

use std::future::Future;
use std::io;
use std::sync::OnceLock;

use tokio::runtime::{Builder, Runtime};

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

/// Returns the result of a future, blocking the calling thread.
///
/// PORT: Java's cancel-on-interrupt is the drop of the future when the thread is interrupted;
/// Blaze has no thread interruption, an interrupted build surfaces as an
/// `io::ErrorKind::Interrupted` error from the future itself.
// Java: com/google/devtools/build/lib/remote/util/Utils.java:112 Utils.getFromFuture
pub fn get_from_future<T>(f: impl Future<Output = io::Result<T>>) -> io::Result<T> {
    check_not_in_runtime();
    runtime().block_on(f)
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
