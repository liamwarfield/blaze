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

/// Runs `test` (a test of this file) in a child process with `BLAZE_SIGNAL_TEST_CHILD` set, so
/// that it can end its process; returns how the child ended.
fn run_in_child(test: &str) -> std::process::ExitStatus {
    std::process::Command::new(std::env::current_exe().unwrap())
        .args([test, "--exact", "--nocapture", "--test-threads=1"])
        .env("BLAZE_SIGNAL_TEST_CHILD", test)
        .stdout(std::process::Stdio::null())
        .status()
        .unwrap()
}

fn is_child(test: &str) -> bool {
    std::env::var("BLAZE_SIGNAL_TEST_CHILD").as_deref() == Ok(test)
}

fn install_handler() {
    signal_handler::install(
        "Bazel",
        &Path::new("/tmp/output_base"),
        std::process::id() as i32,
        &Path::new("/tmp/output_base/server/jvm.out"),
    );
}

/// Review 2026-11-19 round 3, P1: with the handler installed, a `writev` on a socket whose peer
/// has closed and reset the connection (what a remote cache or BES endpoint does to a writing
/// client) fails with EPIPE or ECONNRESET and does not end the process, neither at once nor
/// through `propagate_signal_or_exit`: the command's exit code is the process's. In Bazel the
/// socket belongs to the server, a JVM, which ignores SIGPIPE.
#[test]
fn writev_to_a_reset_peer_does_not_kill_the_process() {
    const NAME: &str = "writev_to_a_reset_peer_does_not_kill_the_process";
    if !is_child(NAME) {
        use std::os::unix::process::ExitStatusExt;
        let status = run_in_child(NAME);
        assert_eq!(
            status.signal(),
            None,
            "child killed by a signal: {status:?}"
        );
        assert_eq!(status.code(), Some(34));
        return;
    }
    use std::os::fd::AsRawFd;
    install_handler();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let client = std::net::TcpStream::connect(listener.local_addr().unwrap()).unwrap();
    let (server, _) = listener.accept().unwrap();
    // FIN, then RST (SO_LINGER {1, 0}): the client is in CLOSE_WAIT when the RST arrives, so its
    // next write fails with EPIPE, which raises SIGPIPE unless the signal is ignored.
    server.shutdown(std::net::Shutdown::Write).unwrap();
    let linger = libc::linger {
        l_onoff: 1,
        l_linger: 0,
    };
    // SAFETY: setsockopt on a valid socket with a valid `linger`.
    let r = unsafe {
        libc::setsockopt(
            server.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_LINGER,
            (&raw const linger).cast(),
            std::mem::size_of::<libc::linger>() as libc::socklen_t,
        )
    };
    assert_eq!(r, 0);
    drop(server);
    std::thread::sleep(Duration::from_millis(100));
    let data = [0u8; 4096];
    let iov = [
        libc::iovec {
            iov_base: data.as_ptr() as *mut libc::c_void,
            iov_len: data.len(),
        },
        libc::iovec {
            iov_base: data.as_ptr() as *mut libc::c_void,
            iov_len: data.len(),
        },
    ];
    let mut epipe = false;
    for _ in 0..10 {
        // SAFETY: writev on a valid socket with two valid buffers.
        let n = unsafe { libc::writev(client.as_raw_fd(), iov.as_ptr(), 2) };
        if n < 0 && std::io::Error::last_os_error().raw_os_error() == Some(libc::EPIPE) {
            epipe = true;
        }
    }
    assert!(epipe, "no write failed with EPIPE");
    std::process::exit(signal_handler::propagate_signal_or_exit(34));
}

/// An EPIPE on stdout is Bazel's client's SIGPIPE: `note_broken_pipe` (called at the failed
/// write) makes `propagate_signal_or_exit` end the process by SIGPIPE, as
/// `SignalHandler::PropagateSignalOrExit` does after the handler recorded the signal.
#[test]
fn epipe_on_stdout_ends_the_process_with_sigpipe() {
    const NAME: &str = "epipe_on_stdout_ends_the_process_with_sigpipe";
    if !is_child(NAME) {
        use std::os::unix::process::ExitStatusExt;
        let status = run_in_child(NAME);
        assert_eq!(status.signal(), Some(libc::SIGPIPE), "{status:?}");
        return;
    }
    install_handler();
    let (reader, writer) = std::io::pipe().unwrap();
    drop(reader);
    use std::os::fd::AsRawFd;
    // SAFETY: dup2 of a valid descriptor onto stdout; write(2) of a valid buffer.
    let n = unsafe {
        assert_eq!(
            libc::dup2(writer.as_raw_fd(), libc::STDOUT_FILENO),
            libc::STDOUT_FILENO
        );
        libc::write(libc::STDOUT_FILENO, b"x".as_ptr().cast(), 1)
    };
    assert_eq!(n, -1);
    assert_eq!(
        std::io::Error::last_os_error().raw_os_error(),
        Some(libc::EPIPE)
    );
    // The write did not end the process (SIGPIPE is ignored) and the exit code is still the
    // command's until the failed write is reported.
    assert_eq!(signal_handler::propagate_signal_or_exit(0), 0);
    signal_handler::note_broken_pipe();
    signal_handler::propagate_signal_or_exit(0);
    unreachable!("propagate_signal_or_exit returned after a broken pipe");
}

/// Without the handler (batch mode: Bazel's process is then the JVM, which ignores SIGPIPE) a
/// broken pipe is not recorded and the exit code is the command's.
#[test]
fn broken_pipe_before_install_is_not_recorded() {
    const NAME: &str = "broken_pipe_before_install_is_not_recorded";
    if !is_child(NAME) {
        use std::os::unix::process::ExitStatusExt;
        let status = run_in_child(NAME);
        assert_eq!(status.signal(), None, "{status:?}");
        assert_eq!(status.code(), Some(36));
        return;
    }
    signal_handler::note_broken_pipe();
    std::process::exit(signal_handler::propagate_signal_or_exit(36));
}
