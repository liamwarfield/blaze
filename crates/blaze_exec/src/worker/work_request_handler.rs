//! Port of `lib/worker/WorkRequestHandler.java`: the library a persistent worker uses to read
//! `WorkRequest`s, run each in its own thread and write the `WorkResponse`s (singleplex and
//! multiplex workers, cancellation).
//!
//! At the pin no part of the Bazel server calls it: Bazel's own workers (JavaBuilder, the test
//! workers) link it. It is ported as the library a worker written in Rust would use.
//!
//! PORT: the two garbage-collection schedulers (`CpuTimeBasedGcScheduler`, `IdleGcScheduler`)
//! call `System.gc()`, which has no Rust counterpart; their builder settings are kept and have
//! no effect.
// PORT: skipped, JVM-only (System.gc): WorkRequestHandler.IdleGcScheduler, WorkRequestHandler.CpuTimeBasedGcScheduler
// PORT: skipped, JVM-only (System.gc): IdleGcScheduler.start, IdleGcScheduler.markActivity, IdleGcScheduler.maybeDoGc, IdleGcScheduler.stop
// PORT: skipped, JVM-only (System.gc): CpuTimeBasedGcScheduler.getCpuTime, CpuTimeBasedGcScheduler.maybePerformGc

use std::collections::HashMap;
use std::fmt::Write as _;
use std::io::{self, Write};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use blaze_protos::blaze::worker::{WorkRequest, WorkResponse};
use blaze_shell::thread_interrupt::{self, ThreadInterrupt};
use parking_lot::Mutex;

use super::error_message::JavaException;

/// Contains the logic for reading `WorkRequest`s and writing `WorkResponse`s.
///
/// PORT: the methods take `&self` (the implementations lock their streams): requests are read on
/// the handler's thread while responses are written from the request threads.
// Java: com/google/devtools/build/lib/worker/WorkRequestHandler.java:50 WorkRequestHandler.WorkerMessageProcessor
pub trait WorkerMessageProcessor: Send + Sync {
    /// Reads the next incoming request from this worker's stdin; `Ok(None)` where Java returns
    /// null (the end of the input).
    fn read_work_request(&self) -> io::Result<Option<WorkRequest>>;

    /// Writes the provided `WorkResponse` to this worker's stdout. This function is also
    /// responsible for flushing the stdout.
    fn write_work_response(&self, work_response: &WorkResponse) -> io::Result<()>;

    /// Clean up.
    fn close(&self) -> io::Result<()>;
}

/// What a work request callback can throw.
#[derive(Debug)]
pub enum WorkRequestError {
    /// `InterruptedException`.
    Interrupted(String),
    /// Any other `Exception` (its stack trace goes into the response's output).
    Exception(JavaException),
}

/// The callback of a work request: the request, and the `PrintWriter` where all error messages
/// and other user-oriented messages should be written to. Returns the exit code, or the
/// exception it throws.
pub type WorkRequestFn =
    dyn Fn(&WorkRequest, &mut String) -> Result<i32, JavaException> + Send + Sync;

/// The cancel callback: the request id and the thread handling the request.
pub type CancelFn = dyn Fn(i32, &RequestThread) + Send + Sync;

/// The thread that handles a request (Java's `Thread` of `RequestInfo`): its interrupt and
/// whether it is alive.
pub struct RequestThread {
    /// The thread's interrupt, once it runs; an interrupt that arrives before is kept.
    interrupt: Mutex<(Option<ThreadInterrupt>, bool)>,
    alive: AtomicBool,
}

impl RequestThread {
    /// A thread that has not started (Java's tests pass null).
    pub fn new() -> RequestThread {
        RequestThread {
            interrupt: Mutex::new((None, false)),
            alive: AtomicBool::new(false),
        }
    }

    /// Called on the thread itself when it starts.
    fn register_current(&self) {
        let current = ThreadInterrupt::current();
        let mut state = self.interrupt.lock();
        if state.1 {
            current.interrupt();
        }
        state.0 = Some(current);
    }

    /// `Thread.interrupt()`.
    pub fn interrupt(&self) {
        let mut state = self.interrupt.lock();
        match &state.0 {
            Some(thread) => thread.interrupt(),
            None => state.1 = true,
        }
    }

    /// `Thread.isAlive()`.
    pub fn is_alive(&self) -> bool {
        self.alive.load(Ordering::SeqCst)
    }
}

/// Holds information necessary to properly handle a request, especially for cancellation.
// Java: com/google/devtools/build/lib/worker/WorkRequestHandler.java:64 WorkRequestHandler.RequestInfo
pub struct RequestInfo {
    /// The thread handling the request.
    pub thread: Arc<RequestThread>,
    /// If true, we have received a cancel request for this request.
    cancelled: AtomicBool,
    /// The builder for the response to this request. Since only one response must be sent per
    /// request, this builder must be accessed through `take_builder()`, which zeroes this field
    /// and returns the builder.
    response_builder: Mutex<Option<WorkResponse>>,
}

impl RequestInfo {
    // Java: com/google/devtools/build/lib/worker/WorkRequestHandler.java:76 RequestInfo.<init>
    pub fn new(thread: Arc<RequestThread>) -> RequestInfo {
        RequestInfo {
            thread,
            cancelled: AtomicBool::new(false),
            response_builder: Mutex::new(Some(WorkResponse::default())),
        }
    }

    /// Sets whether this request has been cancelled.
    // Java: com/google/devtools/build/lib/worker/WorkRequestHandler.java:81 RequestInfo.setCancelled
    pub fn set_cancelled(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
    }

    /// Returns true if this request has been cancelled.
    // Java: com/google/devtools/build/lib/worker/WorkRequestHandler.java:86 RequestInfo.isCancelled
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }

    /// Returns the response builder. If called more than once on the same instance, subsequent
    /// calls will return `None`.
    // Java: com/google/devtools/build/lib/worker/WorkRequestHandler.java:94 RequestInfo.takeBuilder
    pub fn take_builder(&self) -> Option<WorkResponse> {
        self.response_builder.lock().take()
    }

    /// Adds `s` as output to when the response eventually gets built. Does nothing if the
    /// response has already been taken. There is no guarantee that the response hasn't already
    /// been taken, making this call a no-op. This may be called multiple times. No delimiters are
    /// added between strings from multiple calls.
    // Java: com/google/devtools/build/lib/worker/WorkRequestHandler.java:106 RequestInfo.addOutput
    pub fn add_output(&self, s: &str) {
        if let Some(builder) = self.response_builder.lock().as_mut() {
            builder.output.push_str(s);
        }
    }
}

/// The callback of each `WorkRequest`, which fails with `InterruptedException` when the thread
/// was interrupted while it ran.
// Java: com/google/devtools/build/lib/worker/WorkRequestHandler.java:226 WorkRequestHandler.WorkRequestCallback
pub struct WorkRequestCallback {
    callback: Arc<WorkRequestFn>,
}

impl WorkRequestCallback {
    // Java: com/google/devtools/build/lib/worker/WorkRequestHandler.java:236 WorkRequestCallback.<init>
    pub fn new(
        callback: impl Fn(&WorkRequest, &mut String) -> Result<i32, JavaException>
        + Send
        + Sync
        + 'static,
    ) -> WorkRequestCallback {
        WorkRequestCallback {
            callback: Arc::new(callback),
        }
    }

    /// The deprecated form: the callback gets the request's arguments.
    // Java: com/google/devtools/build/lib/worker/WorkRequestHandler.java:203 WorkRequestHandler.<init>
    pub fn from_arguments(
        callback: impl Fn(&[String], &mut String) -> Result<i32, JavaException> + Send + Sync + 'static,
    ) -> WorkRequestCallback {
        WorkRequestCallback::new(move |request, pw| callback(&request.arguments, pw))
    }

    // Java: com/google/devtools/build/lib/worker/WorkRequestHandler.java:240 WorkRequestCallback.apply
    pub fn apply(
        &self,
        work_request: &WorkRequest,
        print_writer: &mut String,
    ) -> Result<i32, WorkRequestError> {
        let result =
            (self.callback)(work_request, print_writer).map_err(WorkRequestError::Exception)?;
        if thread_interrupt::interrupted() {
            return Err(WorkRequestError::Interrupted(format!(
                "Work request interrupted: {}",
                work_request.request_id
            )));
        }
        Ok(result)
    }
}

/// Builds a [`WorkRequestHandler`].
// Java: com/google/devtools/build/lib/worker/WorkRequestHandler.java:252 WorkRequestHandler.WorkRequestHandlerBuilder
pub struct WorkRequestHandlerBuilder {
    callback: WorkRequestCallback,
    stderr: Box<dyn Write + Send>,
    message_processor: Arc<dyn WorkerMessageProcessor>,
    cpu_usage_before_gc: Duration,
    cancel_callback: Option<Arc<CancelFn>>,
    idle_time_before_gc: Duration,
}

impl WorkRequestHandlerBuilder {
    /// Creates a `WorkRequestHandlerBuilder` for the given callback; `stderr` is where log
    /// messages should be written to, typically the process' stderr; `message_processor` parses
    /// `WorkRequest`s from the server and writes `WorkResponse`s to the server.
    // Java: com/google/devtools/build/lib/worker/WorkRequestHandler.java:295 WorkRequestHandlerBuilder.<init>
    pub fn new(
        callback: WorkRequestCallback,
        stderr: Box<dyn Write + Send>,
        message_processor: Arc<dyn WorkerMessageProcessor>,
    ) -> WorkRequestHandlerBuilder {
        WorkRequestHandlerBuilder {
            callback,
            stderr,
            message_processor,
            cpu_usage_before_gc: Duration::ZERO,
            cancel_callback: None,
            idle_time_before_gc: Duration::ZERO,
        }
    }

    /// Sets the minimum amount of CPU time between explicit garbage collection calls. Pass
    /// `Duration::ZERO` to not do explicit garbage collection (the default).
    /// PORT: no effect (see the module documentation).
    // Java: com/google/devtools/build/lib/worker/WorkRequestHandler.java:309 WorkRequestHandlerBuilder.setCpuUsageBeforeGc
    pub fn set_cpu_usage_before_gc(mut self, cpu_usage_before_gc: Duration) -> Self {
        self.cpu_usage_before_gc = cpu_usage_before_gc;
        self
    }

    /// Sets a callback will be called when a cancellation message has been received. The
    /// callback will be call with the request ID and the thread executing the request.
    // Java: com/google/devtools/build/lib/worker/WorkRequestHandler.java:320 WorkRequestHandlerBuilder.setCancelCallback
    pub fn set_cancel_callback(
        mut self,
        cancel_callback: impl Fn(i32, &RequestThread) + Send + Sync + 'static,
    ) -> Self {
        self.cancel_callback = Some(Arc::new(cancel_callback));
        self
    }

    /// Sets the time without any work that should elapse before forcing a GC.
    /// PORT: no effect (see the module documentation).
    // Java: com/google/devtools/build/lib/worker/WorkRequestHandler.java:327 WorkRequestHandlerBuilder.setIdleTimeBeforeGc
    pub fn set_idle_time_before_gc(mut self, idle_time_before_gc: Duration) -> Self {
        self.idle_time_before_gc = idle_time_before_gc;
        self
    }

    /// Returns a `WorkRequestHandler` instance with the values in this Builder.
    // Java: com/google/devtools/build/lib/worker/WorkRequestHandler.java:333 WorkRequestHandlerBuilder.build
    pub fn build(self) -> WorkRequestHandler {
        let _ = (self.cpu_usage_before_gc, self.idle_time_before_gc);
        WorkRequestHandler {
            active_requests: Mutex::new(HashMap::new()),
            callback: self.callback,
            stderr: Mutex::new(self.stderr),
            message_processor: self.message_processor,
            cancel_callback: self.cancel_callback,
            shutdown_worker: AtomicBool::new(false),
            write_lock: Mutex::new(()),
        }
    }
}

/// A helper class that handles WorkRequests (https://bazel.build/docs/persistent-workers),
/// including multiplex workers (https://bazel.build/docs/multiplex-worker).
// Java: com/google/devtools/build/lib/worker/WorkRequestHandler.java:48 WorkRequestHandler
pub struct WorkRequestHandler {
    /// Requests that are currently being processed. Visible for testing.
    pub active_requests: Mutex<HashMap<i32, Arc<RequestInfo>>>,
    /// The function to be called after each `WorkRequest` is read.
    callback: WorkRequestCallback,
    /// This worker's stderr.
    stderr: Mutex<Box<dyn Write + Send>>,
    pub message_processor: Arc<dyn WorkerMessageProcessor>,
    cancel_callback: Option<Arc<CancelFn>>,
    /// If set, this worker will stop handling requests and shut itself down. This can happen if
    /// something throws an `Error`.
    shutdown_worker: AtomicBool,
    /// `synchronized (this)` around writing a response.
    write_lock: Mutex<()>,
}

impl WorkRequestHandler {
    /// Creates a `WorkRequestHandler` that will call `callback` for each WorkRequest received
    /// (the deprecated constructors; the callback gets the request's arguments).
    // Java: com/google/devtools/build/lib/worker/WorkRequestHandler.java:158 WorkRequestHandler.<init>
    // Java: com/google/devtools/build/lib/worker/WorkRequestHandler.java:180 WorkRequestHandler.<init>
    // Java: com/google/devtools/build/lib/worker/WorkRequestHandler.java:197 WorkRequestHandler.<init>
    // Java: com/google/devtools/build/lib/worker/WorkRequestHandler.java:211 WorkRequestHandler.<init>
    pub fn new(
        callback: impl Fn(&[String], &mut String) -> Result<i32, JavaException> + Send + Sync + 'static,
        stderr: Box<dyn Write + Send>,
        message_processor: Arc<dyn WorkerMessageProcessor>,
    ) -> WorkRequestHandler {
        WorkRequestHandlerBuilder::new(
            WorkRequestCallback::from_arguments(callback),
            stderr,
            message_processor,
        )
        .build()
    }

    /// `stderr.println(s)`.
    fn println_stderr(&self, s: &str) {
        let mut stderr = self.stderr.lock();
        let _ = writeln!(stderr, "{s}");
        let _ = stderr.flush();
    }

    /// Runs an infinite loop of reading `WorkRequest` from `in`, running the callback, then
    /// writing the corresponding `WorkResponse` to `out`. If there is an error reading or
    /// writing the requests or responses, it writes an error message on `err` and returns. If
    /// `in` reaches EOF, it also returns.
    ///
    /// This function also wraps the system streams in a [`WorkerIO`] instance that prevents the
    /// underlying tool from writing to stdout or reading from stdin, which would corrupt the
    /// worker communication protocol.
    // Java: com/google/devtools/build/lib/worker/WorkRequestHandler.java:351 WorkRequestHandler.processRequests
    pub fn process_requests(self: &Arc<Self>) -> io::Result<()> {
        // Wrap the system streams into a WorkerIO instance to prevent unexpected reads and writes
        // on stdin/stdout.
        let worker_io = Arc::new(WorkerIO::capture());

        let result = (|| -> io::Result<()> {
            while !self.shutdown_worker.load(Ordering::SeqCst) {
                let request = self.message_processor.read_work_request()?;
                let Some(request) = request else {
                    break;
                };
                if request.cancel {
                    self.respond_to_cancel_request(&request);
                } else {
                    self.start_response_thread(worker_io.clone(), request);
                }
            }
            Ok(())
        })();
        if let Err(e) = result {
            let e = JavaException::from_io_error(&e);
            self.println_stderr(&format!("Error reading next WorkRequest: {e}"));
            // e.printStackTrace(stderr)
            let mut stderr = self.stderr.lock();
            let _ = stderr.write_all(e.stack_trace_as_string().as_bytes());
            let _ = stderr.flush();
        }
        // finally
        // TODO(b/220878242): Give the outstanding requests a chance to send a "shutdown"
        // response, but also try to kill stuck threads. For now, we just interrupt the remaining
        // threads. We considered doing System.exit here, but that is hard to test and would deny
        // the callers of this method a chance to clean up. Instead, we initiate the cleanup of
        // our resources here and the caller can decide whether to wait for an orderly shutdown or
        // now.
        let active: Vec<Arc<RequestInfo>> = self.active_requests.lock().values().cloned().collect();
        for ri in active {
            if ri.thread.is_alive() {
                ri.thread.interrupt();
            }
        }

        // Unwrap the system streams placing the original streams back
        if let Err(e) = worker_io.close() {
            self.println_stderr(JavaException::from_io_error(&e).get_message());
        }
        Ok(())
    }

    /// Starts a thread for the given request.
    // Java: com/google/devtools/build/lib/worker/WorkRequestHandler.java:391 WorkRequestHandler.startResponseThread
    pub fn start_response_thread(self: &Arc<Self>, worker_io: Arc<WorkerIO>, request: WorkRequest) {
        let current_thread = ThreadInterrupt::current();
        let thread_name = if request.request_id > 0 {
            format!("multiplex-request-{}", request.request_id)
        } else {
            "singleplex-request".to_string()
        };
        // TODO(larsrc): See if this can be handled with a queue instead, without introducing
        // more race conditions.
        if request.request_id == 0 {
            while self
                .active_requests
                .lock()
                .contains_key(&request.request_id)
            {
                // b/194051480: Previous singleplex requests can still be in activeRequests for a
                // bit after the response has been sent. We need to wait for them to vanish.
                std::thread::sleep(Duration::from_millis(1));
                if thread_interrupt::interrupted() {
                    thread_interrupt::interrupt_current();
                    return;
                }
            }
        }
        let thread = Arc::new(RequestThread::new());
        {
            let mut active = self.active_requests.lock();
            if active.contains_key(&request.request_id) {
                // Kill worker since this shouldn't happen: server didn't follow the worker
                // protocol
                panic!(
                    "IllegalStateException: Request still active: {}",
                    request.request_id
                );
            }
            active.insert(
                request.request_id,
                Arc::new(RequestInfo::new(thread.clone())),
            );
        }
        thread.alive.store(true, Ordering::SeqCst);
        let this = self.clone();
        let body_thread = thread.clone();
        let spawned = std::thread::Builder::new()
            .name(thread_name)
            .spawn(move || {
                body_thread.register_current();
                let request_id = request.request_id;
                let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    let request_info = this.active_requests.lock().get(&request_id).cloned();
                    let Some(request_info) = request_info else {
                        // Already cancelled
                        return;
                    };
                    let result = this.respond_to_request(&worker_io, &request, &request_info);
                    if let Err(e) = result {
                        // IOExceptions here means a problem talking to the server, so we must shut
                        // down.
                        if this
                            .shutdown_worker
                            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
                            .is_err()
                        {
                            this.println_stderr(
                                "Error communicating with server, shutting down worker.",
                            );
                            let e = JavaException::from_io_error(&e);
                            let mut stderr = this.stderr.lock();
                            let _ = stderr.write_all(e.stack_trace_as_string().as_bytes());
                            let _ = stderr.flush();
                            drop(stderr);
                            current_thread.interrupt();
                        }
                    }
                    // finally
                    this.active_requests.lock().remove(&request_id);
                }));
                if let Err(payload) = outcome {
                    // The uncaught exception handler: shut down the worker in case of severe issues.
                    // (A Rust panic is Java's `Error`; Java catches the callback's `Exception`s.)
                    this.active_requests.lock().remove(&request_id);
                    if this
                        .shutdown_worker
                        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
                        .is_ok()
                    {
                        let message = payload
                            .downcast_ref::<String>()
                            .map(String::as_str)
                            .or_else(|| payload.downcast_ref::<&str>().copied())
                            .unwrap_or("");
                        this.println_stderr("Error thrown by worker thread, shutting down worker.");
                        this.println_stderr(message);
                        current_thread.interrupt();
                        std::process::exit(1);
                    }
                }
                body_thread.alive.store(false, Ordering::SeqCst);
            });
        if spawned.is_err() {
            thread.alive.store(false, Ordering::SeqCst);
            panic!("OutOfMemoryError: unable to create native thread");
        }
    }

    /// Handles and responds to the given `WorkRequest`.
    // Java: com/google/devtools/build/lib/worker/WorkRequestHandler.java:458 WorkRequestHandler.respondToRequest
    pub fn respond_to_request(
        &self,
        worker_io: &WorkerIO,
        request: &WorkRequest,
        request_info: &RequestInfo,
    ) -> io::Result<()> {
        let mut sw = String::new();
        let exit_code = match self.callback.apply(request, &mut sw) {
            Ok(exit_code) => exit_code,
            Err(WorkRequestError::Interrupted(_)) => 1,
            Err(WorkRequestError::Exception(e)) => {
                // e.printStackTrace(pw)
                sw.push_str(&e.stack_trace_as_string());
                1
            }
        };

        match worker_io.read_captured_as_utf8_string() {
            // Read out the captured string for the final WorkResponse output
            Ok(captured) => {
                let captured = java_trim(&captured);
                if !captured.is_empty() {
                    let _ = sw.write_str(captured);
                }
            }
            Err(e) => self.println_stderr(JavaException::from_io_error(&e).get_message()),
        }
        if let Some(mut builder) = request_info.take_builder() {
            builder.request_id = request.request_id;
            if request_info.is_cancelled() {
                builder.was_cancelled = true;
            } else {
                builder.output.push_str(&sw);
                builder.exit_code = exit_code;
            }
            let _guard = self.write_lock.lock();
            self.message_processor.write_work_response(&builder)?;
        }
        Ok(())
    }

    /// Handles cancelling an existing request, including sending a response if that is not done
    /// by the time `cancel_callback.accept` returns.
    // Java: com/google/devtools/build/lib/worker/WorkRequestHandler.java:503 WorkRequestHandler.respondToCancelRequest
    pub fn respond_to_cancel_request(&self, request: &WorkRequest) {
        // Theoretically, we could have gotten two singleplex requests, and we can't tell those
        // apart. However, that's a violation of the protocol, so we don't try to handle it (not
        // least because handling it would be quite error-prone).
        let ri = self
            .active_requests
            .lock()
            .get(&request.request_id)
            .cloned();

        let Some(ri) = ri else {
            return;
        };
        match &self.cancel_callback {
            None => {
                ri.set_cancelled();
                // This is either an error on the server side or a version mismatch between the
                // server setup and the binary. It's better to wait for the regular work to finish
                // instead of breaking the build, but we should inform the user about the bad
                // setup.
                ri.add_output(&format!(
                    "Cancellation request received for worker request {}, but this worker does \
                     not support cancellation.\n",
                    request.request_id
                ));
            }
            Some(cancel_callback) => {
                if ri.thread.is_alive() && !ri.is_cancelled() {
                    ri.set_cancelled();
                    let cancel_callback = cancel_callback.clone();
                    let request_id = request.request_id;
                    let thread = ri.thread.clone();
                    // Response will be sent from request thread once request handler returns.
                    // We can ignore any exceptions in cancel callback since it's best effort.
                    std::thread::spawn(move || {
                        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                            cancel_callback(request_id, &thread)
                        }));
                    });
                }
            }
        }
    }

    // Java: com/google/devtools/build/lib/worker/WorkRequestHandler.java:539 WorkRequestHandler.close
    pub fn close(&self) -> io::Result<()> {
        self.message_processor.close()
    }
}

/// Java `String.trim()`: strips the chars `<= ' '` at both ends.
fn java_trim(s: &str) -> &str {
    s.trim_matches(|c: char| c <= ' ')
}

/// Where a [`WorkerIO`] captures the standard output and error.
enum Captured {
    /// An in-memory buffer (`ByteArrayOutputStream`), for tests.
    Buffer(Arc<Mutex<Vec<u8>>>),
    /// An anonymous file that file descriptors 1 and 2 point to.
    File(Mutex<std::fs::File>),
}

/// A wrapper class for the standard streams (stdin, stdout and stderr) that captures what the
/// tool writes to stdout and stderr, so that it does not corrupt the worker protocol on stdout.
///
/// PORT: Java replaces `System.in`, `System.out` and `System.err`. Rust has no replaceable
/// standard streams; [`WorkerIO::capture`] points file descriptors 0, 1 and 2 elsewhere (fd 0 at
/// `/dev/null`, an empty stream; fds 1 and 2 at an anonymous file) and keeps duplicates of the
/// original descriptors, which it restores on [`WorkerIO::close`]. A worker's message processor
/// must therefore write its responses through its own handle on the original stdout (opened
/// before `capture`), as Java's holds the original `System.out`.
// Java: com/google/devtools/build/lib/worker/WorkRequestHandler.java:636 WorkRequestHandler.WorkerIO
pub struct WorkerIO {
    original_input_stream: Option<OwnedFd>,
    original_output_stream: Option<OwnedFd>,
    original_error_stream: Option<OwnedFd>,
    captured_stream: Captured,
    restore: Mutex<Option<Box<dyn FnOnce() -> io::Result<()> + Send>>>,
}

impl WorkerIO {
    /// A `WorkerIO` that captures into `captured_stream`, with `restore` run on close (the
    /// constructor the tests use). The original streams are not descriptors here.
    // Java: com/google/devtools/build/lib/worker/WorkRequestHandler.java:651 WorkerIO.<init>
    pub fn new(
        captured_stream: Arc<Mutex<Vec<u8>>>,
        restore: Box<dyn FnOnce() -> io::Result<()> + Send>,
    ) -> WorkerIO {
        WorkerIO {
            original_input_stream: None,
            original_output_stream: None,
            original_error_stream: None,
            captured_stream: Captured::Buffer(captured_stream),
            restore: Mutex::new(Some(restore)),
        }
    }

    /// Wraps the standard streams so that stdin reads nothing and what is written to stdout and
    /// stderr is captured; the originals are kept and put back on [`WorkerIO::close`].
    // Java: com/google/devtools/build/lib/worker/WorkRequestHandler.java:670 WorkerIO.capture
    pub fn capture() -> WorkerIO {
        fn dup(fd: i32) -> OwnedFd {
            // SAFETY: dup returns a new descriptor we own, or -1.
            let new = unsafe { libc::fcntl(fd, libc::F_DUPFD_CLOEXEC, 3) };
            assert!(new >= 0, "dup({fd}): {}", io::Error::last_os_error());
            // SAFETY: `new` is a fresh descriptor owned by nobody else.
            unsafe { OwnedFd::from_raw_fd(new) }
        }
        fn dup2(from: i32, to: i32) {
            // SAFETY: plain syscall on descriptors we hold.
            let rc = unsafe { libc::dup2(from, to) };
            assert!(
                rc >= 0,
                "dup2({from}, {to}): {}",
                io::Error::last_os_error()
            );
        }
        // Flush what Rust buffered for the original streams.
        let _ = io::stdout().flush();
        let _ = io::stderr().flush();

        // Save the original streams
        let original_input_stream = dup(0);
        let original_output_stream = dup(1);
        let original_error_stream = dup(2);

        // Replace the original streams with our own instances
        // SAFETY: memfd_create returns a new descriptor we own, or -1.
        let captured = unsafe {
            let fd = libc::memfd_create(c"worker-io".as_ptr(), libc::MFD_CLOEXEC);
            assert!(fd >= 0, "memfd_create: {}", io::Error::last_os_error());
            std::fs::File::from_raw_fd(fd)
        };
        let empty_input = std::fs::File::open("/dev/null").expect("/dev/null");
        dup2(empty_input.as_raw_fd(), 0);
        dup2(captured.as_raw_fd(), 1);
        dup2(captured.as_raw_fd(), 2);

        let saved = [
            original_input_stream.as_raw_fd(),
            original_output_stream.as_raw_fd(),
            original_error_stream.as_raw_fd(),
        ];
        WorkerIO {
            original_input_stream: Some(original_input_stream),
            original_output_stream: Some(original_output_stream),
            original_error_stream: Some(original_error_stream),
            captured_stream: Captured::File(Mutex::new(captured)),
            restore: Mutex::new(Some(Box::new(move || {
                let _ = io::stdout().flush();
                let _ = io::stderr().flush();
                dup2(saved[0], 0);
                dup2(saved[1], 1);
                dup2(saved[2], 2);
                Ok(())
            }))),
        }
    }

    /// The descriptor of the original stdin (`None` for a test `WorkerIO`).
    // Java: com/google/devtools/build/lib/worker/WorkRequestHandler.java:705 WorkerIO.getOriginalInputStream
    pub fn get_original_input_stream(&self) -> Option<i32> {
        self.original_input_stream.as_ref().map(|fd| fd.as_raw_fd())
    }

    /// The descriptor of the original stdout (`None` for a test `WorkerIO`).
    // Java: com/google/devtools/build/lib/worker/WorkRequestHandler.java:710 WorkerIO.getOriginalOutputStream
    pub fn get_original_output_stream(&self) -> Option<i32> {
        self.original_output_stream
            .as_ref()
            .map(|fd| fd.as_raw_fd())
    }

    /// The descriptor of the original stderr (`None` for a test `WorkerIO`).
    // Java: com/google/devtools/build/lib/worker/WorkRequestHandler.java:715 WorkerIO.getOriginalErrorStream
    pub fn get_original_error_stream(&self) -> Option<i32> {
        self.original_error_stream.as_ref().map(|fd| fd.as_raw_fd())
    }

    /// Returns the captured outputs as a UTF-8 string, and resets the capture.
    // Java: com/google/devtools/build/lib/worker/WorkRequestHandler.java:726 WorkerIO.readCapturedAsUtf8String
    pub fn read_captured_as_utf8_string(&self) -> io::Result<String> {
        let bytes = match &self.captured_stream {
            Captured::Buffer(buffer) => std::mem::take(&mut *buffer.lock()),
            Captured::File(file) => {
                use std::io::{Read, Seek, SeekFrom};
                let _ = io::stdout().flush();
                let _ = io::stderr().flush();
                let mut file = file.lock();
                let mut bytes = Vec::new();
                file.seek(SeekFrom::Start(0))?;
                file.read_to_end(&mut bytes)?;
                file.set_len(0)?;
                file.seek(SeekFrom::Start(0))?;
                bytes
            }
        };
        // ByteArrayOutputStream.toString(UTF_8): malformed input becomes U+FFFD.
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    }

    // Java: com/google/devtools/build/lib/worker/WorkRequestHandler.java:734 WorkerIO.close
    pub fn close(&self) -> io::Result<()> {
        match self.restore.lock().take() {
            Some(restore) => restore(),
            None => Ok(()),
        }
    }
}
