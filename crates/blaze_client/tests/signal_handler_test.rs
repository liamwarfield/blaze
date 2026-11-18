//! `SignalHandler` (blaze_util_posix.cc) and `BlazeServer`'s cancel thread (blaze.cc): a SIGINT or
//! SIGTERM while the command runs cancels it (in-process: the server's canceller runs on the
//! cancel thread). One test: the handler's interrupt count is process-wide, and a third SIGINT
//! would end the test process.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use blaze_client::signal_handler;
use blaze_client::util::path::Path;

fn wait_for(count: &AtomicUsize, n: usize) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while count.load(Ordering::SeqCst) < n {
        assert!(Instant::now() < deadline, "cancel {n} not delivered");
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn sigint_and_sigterm_cancel_the_running_command() {
    signal_handler::install(
        "Bazel",
        &Path::new("/tmp/output_base"),
        std::process::id() as i32,
        &Path::new("/tmp/output_base/server/jvm.out"),
    );
    let cancels = Arc::new(AtomicUsize::new(0));
    let c = cancels.clone();
    let exit_code = signal_handler::with_cancel_thread(
        Arc::new(move || {
            c.fetch_add(1, Ordering::SeqCst);
        }),
        || {
            // SAFETY: raise(3) with a handled signal.
            unsafe { libc::raise(libc::SIGINT) };
            wait_for(&cancels, 1);
            // SAFETY: as above.
            unsafe { libc::raise(libc::SIGTERM) };
            wait_for(&cancels, 2);
            8
        },
    );
    assert_eq!(exit_code, 8);
    assert_eq!(cancels.load(Ordering::SeqCst), 2);
    // No SIGPIPE was received: the exit code is returned.
    assert_eq!(signal_handler::propagate_signal_or_exit(8), 8);

    // A command that panics still ends the cancel thread, so the panic reaches the caller.
    let r = std::panic::catch_unwind(|| {
        signal_handler::with_cancel_thread(Arc::new(|| {}), || -> i32 { panic!("unported") })
    });
    assert!(r.is_err());
}
