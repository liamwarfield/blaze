//! Port of src/test/java/com/google/devtools/build/lib/worker/WorkRequestHandlerTest.java.

use std::collections::VecDeque;
use std::io::{self, Write};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use blaze_exec::worker::error_message::JavaException;
use blaze_exec::worker::json_reader::JsonReader;
use blaze_exec::worker::json_worker_message_processor::JsonWorkerMessageProcessor;
use blaze_exec::worker::proto_worker_message_processor::{
    ProtoWorkerMessageProcessor, parse_delimited_work_request,
};
use blaze_exec::worker::recording_input_stream::{ByteArrayInputStream, InputStream};
use blaze_exec::worker::work_request_handler::{
    RequestInfo, RequestThread, WorkRequestCallback, WorkRequestHandler, WorkRequestHandlerBuilder,
    WorkerIO, WorkerMessageProcessor,
};
use blaze_exec::worker::worker_protocol::parse_delimited_work_response;
use blaze_protos::JavaMessage;
use blaze_protos::blaze::worker::{WorkRequest, WorkResponse};
use blaze_shell::thread_interrupt::{self, ThreadInterrupt};
use parking_lot::{Condvar, Mutex};

/// Tests that swap the process's standard descriptors (`WorkerIO.capture`, which
/// `processRequests` calls) run one at a time, each in a child process of its own: in this
/// process the test harness writes its progress lines to descriptor 1 from other threads, which
/// the capture would take in.
static STD_STREAMS: Mutex<()> = Mutex::new(());

const CHILD_ENV: &str = "BLAZE_WORK_REQUEST_HANDLER_TEST_CHILD";

/// Runs `body` in a child process that runs only the test `name`; returns `false` in the
/// parent (after checking that the child passed) and `true` in the child.
fn in_child_process(name: &str) -> bool {
    if std::env::var_os(CHILD_ENV).is_some_and(|v| v == name) {
        return true;
    }
    let _guard = STD_STREAMS.lock();
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([name, "--exact", "--test-threads=1"])
        .env(CHILD_ENV, name)
        // A pipe, so that the child's stdin is not the empty stream capture() installs (Java's
        // test sets System.in to a fresh stream first).
        .stdin(std::process::Stdio::piped())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{name} failed in its child process:\n{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    false
}

/// `ByteArrayOutputStream` shared with the test.
#[derive(Clone, Default)]
struct SharedBuffer(Arc<Mutex<Vec<u8>>>);

impl Write for SharedBuffer {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.lock().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// `PipedOutputStream` / `PipedInputStream`: reads block until data arrives or the writer is
/// closed; an interrupt of the reading thread fails the read with `InterruptedIOException`.
#[derive(Clone, Default)]
struct Pipe(Arc<(Mutex<(VecDeque<u8>, bool)>, Condvar)>);

impl Pipe {
    fn close(&self) {
        self.0.0.lock().1 = true;
        self.0.1.notify_all();
    }
}

impl Write for Pipe {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.0.lock().0.extend(buf.iter().copied());
        self.0.1.notify_all();
        Ok(buf.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl InputStream for Pipe {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        let mut state = self.0.0.lock();
        loop {
            if !state.0.is_empty() {
                let n = buf.len().min(state.0.len());
                for b in buf.iter_mut().take(n) {
                    *b = state.0.pop_front().unwrap();
                }
                return Ok(n);
            }
            if state.1 {
                return Ok(0);
            }
            if thread_interrupt::interrupted() {
                return Err(JavaException::interrupted_io("").into_io_error());
            }
            self.0.1.wait_for(&mut state, Duration::from_millis(5));
        }
    }

    fn available(&mut self) -> io::Result<usize> {
        Ok(self.0.0.lock().0.len())
    }
}

/// `java.util.concurrent.Semaphore`, whose `acquire` is interruptible.
#[derive(Default)]
struct Semaphore(Mutex<i64>, Condvar);

impl Semaphore {
    fn release(&self, n: i64) {
        *self.0.lock() += n;
        self.1.notify_all();
    }

    fn acquire(&self, n: i64) -> Result<(), ()> {
        let mut permits = self.0.lock();
        loop {
            if thread_interrupt::interrupted() {
                return Err(());
            }
            if *permits >= n {
                *permits -= n;
                return Ok(());
            }
            self.1.wait_for(&mut permits, Duration::from_millis(5));
        }
    }

    fn available_permits(&self) -> i64 {
        *self.0.lock()
    }
}

fn args() -> Vec<String> {
    vec!["--sources".to_string(), "A.java".to_string()]
}

fn delimited(request: &WorkRequest) -> Vec<u8> {
    let mut buf = Vec::new();
    request.write_delimited_to(&mut buf);
    buf
}

fn response_from(bytes: Vec<u8>) -> WorkResponse {
    parse_delimited_work_response(&mut ByteArrayInputStream::new(bytes))
        .unwrap()
        .unwrap()
}

fn test_worker_io() -> WorkerIO {
    WorkerIO::new(Arc::new(Mutex::new(Vec::new())), Box::new(|| Ok(())))
}

fn empty_stderr() -> Box<dyn Write + Send> {
    Box::new(SharedBuffer::default())
}

/// `new RequestInfo(null)`: a request thread that never ran.
fn request_info() -> RequestInfo {
    RequestInfo::new(Arc::new(RequestThread::new()))
}

fn proto_handler(
    callback: impl Fn(&[String], &mut String) -> Result<i32, JavaException> + Send + Sync + 'static,
    out: SharedBuffer,
) -> WorkRequestHandler {
    WorkRequestHandler::new(
        callback,
        empty_stderr(),
        Arc::new(ProtoWorkerMessageProcessor::new(
            Box::new(ByteArrayInputStream::new(Vec::new())),
            Box::new(out),
        )),
    )
}

#[test]
fn test_normal_work_request() {
    let out = SharedBuffer::default();
    let handler = proto_handler(|_, _| Ok(1), out.clone());

    let request = WorkRequest {
        arguments: args(),
        ..Default::default()
    };
    handler
        .respond_to_request(&test_worker_io(), &request, &request_info())
        .unwrap();

    let response = response_from(out.0.lock().clone());
    assert_eq!(response.request_id, 0);
    assert_eq!(response.exit_code, 1);
    assert!(response.output.is_empty());
}

#[test]
fn test_multiplex_work_request() {
    let out = SharedBuffer::default();
    let handler = proto_handler(|_, _| Ok(0), out.clone());

    let request = WorkRequest {
        arguments: args(),
        request_id: 42,
        ..Default::default()
    };
    handler
        .respond_to_request(&test_worker_io(), &request, &request_info())
        .unwrap();

    let response = response_from(out.0.lock().clone());
    assert_eq!(response.request_id, 42);
    assert_eq!(response.exit_code, 0);
    assert!(response.output.is_empty());
}

/// A wrapper around a WorkerMessageProcessor that can be stopped by calling `stop()`.
struct StoppableWorkerMessageProcessor {
    delegate: Box<dyn WorkerMessageProcessor>,
    stop: AtomicBool,
    reader_thread: Mutex<Option<ThreadInterrupt>>,
}

impl StoppableWorkerMessageProcessor {
    fn new(delegate: Box<dyn WorkerMessageProcessor>) -> StoppableWorkerMessageProcessor {
        StoppableWorkerMessageProcessor {
            delegate,
            stop: AtomicBool::new(false),
            reader_thread: Mutex::new(None),
        }
    }

    fn stop(&self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(reader_thread) = self.reader_thread.lock().as_ref() {
            reader_thread.interrupt();
        }
    }

    fn interrupt_reader(&self) {
        self.reader_thread.lock().as_ref().unwrap().interrupt();
    }
}

impl WorkerMessageProcessor for StoppableWorkerMessageProcessor {
    fn read_work_request(&self) -> io::Result<Option<WorkRequest>> {
        *self.reader_thread.lock() = Some(ThreadInterrupt::current());
        if self.stop.load(Ordering::SeqCst) {
            return Ok(None);
        }
        match self.delegate.read_work_request() {
            // Being interrupted is only an error if we didn't ask for it.
            Err(e)
                if JavaException::is_interrupted_io_exception(&e)
                    && self.stop.load(Ordering::SeqCst) =>
            {
                Ok(None)
            }
            other => other,
        }
    }

    fn write_work_response(&self, work_response: &WorkResponse) -> io::Result<()> {
        self.delegate.write_work_response(work_response)
    }

    fn close(&self) -> io::Result<()> {
        self.delegate.close()
    }
}

fn stoppable(src: &Pipe, dest: &Pipe) -> Arc<StoppableWorkerMessageProcessor> {
    Arc::new(StoppableWorkerMessageProcessor::new(Box::new(
        ProtoWorkerMessageProcessor::new(Box::new(src.clone()), Box::new(dest.clone())),
    )))
}

/// The threads of the requests, as the callbacks record them.
#[derive(Clone, Default)]
struct WorkerThreads(Arc<Mutex<Vec<(ThreadInterrupt, Arc<AtomicBool>)>>>);

impl WorkerThreads {
    /// Records the current thread; the returned flag is cleared when the callback returns.
    fn add_current(&self) -> Arc<AtomicBool> {
        let alive = Arc::new(AtomicBool::new(true));
        self.0
            .lock()
            .push((ThreadInterrupt::current(), alive.clone()));
        alive
    }

    fn len(&self) -> usize {
        self.0.lock().len()
    }

    fn is_alive(&self, i: usize) -> bool {
        self.0.lock()[i].1.load(Ordering::SeqCst)
    }
}

#[test]
fn test_multiplex_work_request_stops_threads_on_shutdown() {
    if !in_child_process("test_multiplex_work_request_stops_threads_on_shutdown") {
        return;
    }
    let src = Pipe::default();
    let dest = Pipe::default();

    // Work request threads release this when they have started.
    let started = Arc::new(Semaphore::default());
    // Work request threads wait forever on this, so we can see how they react to closed stdin.
    let eternity = Arc::new(Semaphore::default());
    // Released when the work request handler thread has noticed the closed stdin and
    // interrupted the work request threads.
    let stopped = Arc::new(Semaphore::default());
    let worker_threads = WorkerThreads::default();
    let message_processor = stoppable(&src, &dest);
    let handler = {
        let (started, eternity, worker_threads) =
            (started.clone(), eternity.clone(), worker_threads.clone());
        Arc::new(WorkRequestHandler::new(
            move |_, _| {
                // Each call to this runs in its own thread.
                let alive = worker_threads.add_current();
                started.release(1);
                if eternity.acquire(1).is_err() {
                    // This blocks until the thread is interrupted at shutdown.
                    thread_interrupt::interrupt_current();
                }
                alive.store(false, Ordering::SeqCst);
                Ok(0)
            },
            empty_stderr(),
            message_processor.clone(),
        ))
    };

    {
        let (handler, stopped) = (handler.clone(), stopped.clone());
        std::thread::spawn(move || {
            handler.process_requests().expect("Unhandled exception");
            stopped.release(1);
        });
    }
    let mut src_writer = src.clone();
    for id in [42, 43] {
        src_writer
            .write_all(&delimited(&WorkRequest {
                arguments: args(),
                request_id: id,
                ..Default::default()
            }))
            .unwrap();
    }

    started.acquire(2).unwrap();
    assert_eq!(worker_threads.len(), 2);
    // Now both request threads are started, closing the input to the "worker" should shut it
    // down.
    src.close();
    stopped.acquire(1).unwrap();
    while worker_threads.is_alive(0) || worker_threads.is_alive(1) {
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(!worker_threads.is_alive(0));
    assert!(!worker_threads.is_alive(1));
}

#[test]
fn test_multiplex_work_request_stops_worker_on_exception() {
    if !in_child_process("test_multiplex_work_request_stops_worker_on_exception") {
        return;
    }
    let src = Pipe::default();
    let dest = Pipe::default();

    // Work request threads release this when they have started.
    let started = Arc::new(Semaphore::default());
    // One work request threads waits forever on this, so the second one can throw an exception
    let eternity = Arc::new(Semaphore::default());
    // Released when the work request handler thread has been stopped after a worker thread
    // died.
    let stopped = Arc::new(Semaphore::default());
    let worker_threads = WorkerThreads::default();
    let message_processor = stoppable(&src, &dest);
    let handler = {
        let (started, eternity, worker_threads, reader) = (
            started.clone(),
            eternity.clone(),
            worker_threads.clone(),
            message_processor.clone(),
        );
        Arc::new(WorkRequestHandler::new(
            move |_, _| {
                // Each call to this runs in its own thread.
                let alive = worker_threads.add_current();
                started.release(1);
                if worker_threads.len() < 2 {
                    if eternity.acquire(1).is_err() {
                        // This blocks forever.
                        thread_interrupt::interrupt_current();
                    }
                } else {
                    // This is triggered by the second WorkRequest. This causes the pipe under the
                    // hood to throw an InterruptedIOException, which the WorkRequestHandler's
                    // loop catches as an IOException of readWorkRequest(), stopping the worker.
                    reader.interrupt_reader();
                }
                alive.store(false, Ordering::SeqCst);
                Ok(0)
            },
            empty_stderr(),
            message_processor.clone(),
        ))
    };

    {
        let (handler, stopped) = (handler.clone(), stopped.clone());
        std::thread::spawn(move || {
            handler.process_requests().expect("Unhandled exception");
            stopped.release(1);
        });
    }
    let mut src_writer = src.clone();
    for id in [42, 43] {
        src_writer
            .write_all(&delimited(&WorkRequest {
                arguments: args(),
                request_id: id,
                ..Default::default()
            }))
            .unwrap();
    }

    started.acquire(2).unwrap();
    assert_eq!(worker_threads.len(), 2);
    stopped.acquire(1).unwrap();
    while worker_threads.is_alive(0) || worker_threads.is_alive(1) {
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(!worker_threads.is_alive(0));
    assert!(!worker_threads.is_alive(1));
}

#[test]
fn test_output() {
    let out = SharedBuffer::default();
    let handler = proto_handler(
        |_, err| {
            err.push_str("Failed!\n");
            Ok(1)
        },
        out.clone(),
    );

    let request = WorkRequest {
        arguments: args(),
        ..Default::default()
    };
    handler
        .respond_to_request(&test_worker_io(), &request, &request_info())
        .unwrap();

    let response = response_from(out.0.lock().clone());
    assert_eq!(response.request_id, 0);
    assert_eq!(response.exit_code, 1);
    assert!(response.output.contains("Failed!"));
}

#[test]
fn test_exception() {
    let out = SharedBuffer::default();
    let handler = proto_handler(
        |_, _| {
            Err(JavaException::new(
                "java.lang.RuntimeException",
                "Exploded!",
            ))
        },
        out.clone(),
    );

    let request = WorkRequest {
        arguments: args(),
        ..Default::default()
    };
    handler
        .respond_to_request(&test_worker_io(), &request, &request_info())
        .unwrap();

    let response = response_from(out.0.lock().clone());
    assert_eq!(response.request_id, 0);
    assert_eq!(response.exit_code, 1);
    assert!(
        response
            .output
            .starts_with("java.lang.RuntimeException: Exploded!")
    );
}

/// `runRequestHandlerThread`: runs the handler, waits for its requests, then releases `done`
/// and waits for `finish`.
fn run_request_handler_thread(
    done: Arc<Semaphore>,
    handler: Arc<WorkRequestHandler>,
    finish: Arc<Semaphore>,
    failures: Arc<Mutex<Vec<String>>>,
) {
    // This thread just makes sure the WorkRequestHandler does work asynchronously.
    std::thread::spawn(move || {
        match handler.process_requests() {
            Ok(()) => {
                while !handler.active_requests.lock().is_empty() {
                    std::thread::sleep(Duration::from_millis(1));
                }
            }
            Err(_) => failures
                .lock()
                .push("Unexpected I/O error talking to worker thread".to_string()),
        }
        done.release(1);
        let _ = finish.acquire(1);
    });
}

fn write_request(src: &Pipe, request_id: i32, cancel: bool) {
    let mut src = src.clone();
    src.write_all(&delimited(&WorkRequest {
        request_id,
        cancel,
        ..Default::default()
    }))
    .unwrap();
}

fn read_response(dest: &Pipe) -> WorkResponse {
    let mut dest = dest.clone();
    parse_delimited_work_response(&mut dest).unwrap().unwrap()
}

#[test]
fn test_cancel_request_exactly_one_response_sent() {
    if !in_child_process("test_cancel_request_exactly_one_response_sent") {
        return;
    }
    let handler_called = Arc::new(AtomicBool::new(false));
    let cancel_called = Arc::new(AtomicBool::new(false));
    let src = Pipe::default();
    let dest = Pipe::default();
    let done = Arc::new(Semaphore::default());
    let finish = Arc::new(Semaphore::default());
    let failures = Arc::new(Mutex::new(Vec::new()));

    let message_processor = stoppable(&src, &dest);
    let handler = {
        let (handler_called, cancel_called) = (handler_called.clone(), cancel_called.clone());
        Arc::new(
            WorkRequestHandlerBuilder::new(
                WorkRequestCallback::from_arguments(move |_, err| {
                    handler_called.store(true, Ordering::SeqCst);
                    err.push_str("Such work! Much progress! Wow!\n");
                    Ok(1)
                }),
                empty_stderr(),
                message_processor.clone(),
            )
            .set_cancel_callback(move |_, _| cancel_called.store(true, Ordering::SeqCst))
            .build(),
        )
    };

    run_request_handler_thread(done.clone(), handler, finish.clone(), failures.clone());
    write_request(&src, 42, false);
    write_request(&src, 42, true);
    let response = read_response(&dest);
    message_processor.stop();
    done.acquire(1).unwrap();

    assert!(handler_called.load(Ordering::SeqCst) || cancel_called.load(Ordering::SeqCst));
    assert_eq!(response.request_id, 42);
    if response.was_cancelled {
        assert!(response.output.is_empty());
        assert_eq!(response.exit_code, 0);
    } else {
        assert!(
            response
                .output
                .starts_with("Such work! Much progress! Wow!")
        );
        assert_eq!(response.exit_code, 1);
    }

    // Checks that nothing more was sent.
    assert_eq!(dest.clone().available().unwrap(), 0);
    finish.release(1);

    // Checks that there weren't other unexpected failures.
    assert!(failures.lock().is_empty());
}

#[test]
fn test_cancel_request_sends_response_when_done() {
    if !in_child_process("test_cancel_request_sends_response_when_done") {
        return;
    }
    let wait_for_cancel = Arc::new(Semaphore::default());
    let handler_called = Arc::new(Semaphore::default());
    let cancel_called = Arc::new(Semaphore::default());
    let src = Pipe::default();
    let dest = Pipe::default();
    let done = Arc::new(Semaphore::default());
    let request_done = Arc::new(Semaphore::default());
    let finish = Arc::new(Semaphore::default());
    let failures = Arc::new(Mutex::new(Vec::new()));

    let message_processor = stoppable(&src, &dest);
    // We force the regular handling to not finish until after we have read the cancel
    // response, to avoid flakiness.
    let handler = {
        let (handler_called, wait_for_cancel, request_done, failures, cancel_called) = (
            handler_called.clone(),
            wait_for_cancel.clone(),
            request_done.clone(),
            failures.clone(),
            cancel_called.clone(),
        );
        Arc::new(
            WorkRequestHandlerBuilder::new(
                WorkRequestCallback::from_arguments(move |_, _| {
                    // This handler waits until the main thread has sent a cancel request.
                    handler_called.release(2);
                    if wait_for_cancel.acquire(1).is_err() {
                        failures
                            .lock()
                            .push("Unexpected interrupt waiting for cancel request".to_string());
                    }
                    request_done.release(1);
                    Ok(0)
                }),
                empty_stderr(),
                message_processor.clone(),
            )
            .set_cancel_callback(move |_, _| cancel_called.release(1))
            .build(),
        )
    };

    run_request_handler_thread(done.clone(), handler, finish.clone(), failures.clone());
    write_request(&src, 42, false);
    // Make sure the handler is called before sending the cancel request, or we might process
    // the cancellation entirely before that.
    handler_called.acquire(1).unwrap();
    write_request(&src, 42, true);
    cancel_called.acquire(1).unwrap();
    wait_for_cancel.release(1);
    // Give the other request a chance to process, so we can check that no other response is
    // sent
    request_done.acquire(1).unwrap();
    message_processor.stop();
    done.acquire(1).unwrap();

    let response = read_response(&dest);
    assert_eq!(handler_called.available_permits(), 1); // Released 2, one was acquired
    assert_eq!(cancel_called.available_permits(), 0);
    assert_eq!(response.request_id, 42);
    assert!(response.output.is_empty());
    assert!(response.was_cancelled);

    // Checks that nothing more was sent.
    assert_eq!(dest.clone().available().unwrap(), 0);
    src.close();
    finish.release(1);

    // Checks that there weren't other unexpected failures.
    assert!(failures.lock().is_empty());
}

#[test]
fn test_cancel_request_no_double_cancel_response() {
    if !in_child_process("test_cancel_request_no_double_cancel_response") {
        return;
    }
    let wait_for_cancel = Arc::new(Semaphore::default());
    let cancel_called = Arc::new(Semaphore::default());
    let src = Pipe::default();
    let dest = Pipe::default();
    let done = Arc::new(Semaphore::default());
    let requests_done = Arc::new(Semaphore::default());
    let finish = Arc::new(Semaphore::default());
    let failures = Arc::new(Mutex::new(Vec::new()));

    // We force the regular handling to not finish until after we have read the cancel
    // response, to avoid flakiness.
    let message_processor = stoppable(&src, &dest);
    let handler = {
        let (wait_for_cancel, requests_done, failures, cancel_called) = (
            wait_for_cancel.clone(),
            requests_done.clone(),
            failures.clone(),
            cancel_called.clone(),
        );
        Arc::new(
            WorkRequestHandlerBuilder::new(
                WorkRequestCallback::from_arguments(move |_, _| {
                    if wait_for_cancel.acquire(1).is_err() {
                        failures
                            .lock()
                            .push("Unexpected interrupt waiting for cancel request".to_string());
                    }
                    requests_done.release(1);
                    Ok(0)
                }),
                empty_stderr(),
                message_processor.clone(),
            )
            .set_cancel_callback(move |_, _| cancel_called.release(1))
            .build(),
        )
    };

    run_request_handler_thread(done.clone(), handler, finish.clone(), failures.clone());
    write_request(&src, 42, false);
    write_request(&src, 42, true);
    write_request(&src, 42, true);
    cancel_called.acquire(1).unwrap();
    wait_for_cancel.release(1);
    requests_done.acquire(1).unwrap();
    message_processor.stop();
    done.acquire(1).unwrap();

    let response = read_response(&dest);
    assert!(cancel_called.available_permits() < 2);
    assert_eq!(response.request_id, 42);
    assert!(response.output.is_empty());
    assert!(response.was_cancelled);

    // Checks that nothing more was sent.
    assert_eq!(dest.clone().available().unwrap(), 0);
    src.close();
    finish.release(1);

    // Checks that there weren't other unexpected failures.
    assert!(failures.lock().is_empty());
}

#[test]
fn test_cancel_request_sends_no_response_when_already_sent() {
    if !in_child_process("test_cancel_request_sends_no_response_when_already_sent") {
        return;
    }
    let handler_called = Arc::new(Semaphore::default());
    let src = Pipe::default();
    let dest = Pipe::default();
    let done = Arc::new(Semaphore::default());
    let finish = Arc::new(Semaphore::default());
    let failures = Arc::new(Mutex::new(Vec::new()));

    // We force the cancel request to not happen until after we have read the normal response,
    // to avoid flakiness.
    let message_processor = stoppable(&src, &dest);
    let handler = {
        let handler_called = handler_called.clone();
        Arc::new(
            WorkRequestHandlerBuilder::new(
                WorkRequestCallback::from_arguments(move |_, err| {
                    handler_called.release(1);
                    err.push_str("Such work! Much progress! Wow!\n");
                    Ok(2)
                }),
                empty_stderr(),
                message_processor.clone(),
            )
            .set_cancel_callback(|_, _| {})
            .build(),
        )
    };

    run_request_handler_thread(done.clone(), handler, finish.clone(), failures.clone());
    write_request(&src, 42, false);
    let response = read_response(&dest);
    write_request(&src, 42, true);
    message_processor.stop();
    done.acquire(1).unwrap();

    assert_eq!(handler_called.available_permits(), 1);
    assert_eq!(response.request_id, 42);
    assert!(!response.was_cancelled);
    assert_eq!(response.exit_code, 2);
    assert!(
        response
            .output
            .starts_with("Such work! Much progress! Wow!")
    );

    // Checks that nothing more was sent.
    assert_eq!(dest.clone().available().unwrap(), 0);
    src.close();
    finish.release(1);

    // Checks that there weren't other unexpected failures.
    assert!(failures.lock().is_empty());
}

#[test]
fn test_work_request_handler_with_work_request_callback() {
    let out = SharedBuffer::default();
    let callback = WorkRequestCallback::new(|request, _| Ok(request.arguments.len() as i32));
    let handler = WorkRequestHandlerBuilder::new(
        callback,
        empty_stderr(),
        Arc::new(ProtoWorkerMessageProcessor::new(
            Box::new(ByteArrayInputStream::new(Vec::new())),
            Box::new(out.clone()),
        )),
    )
    .build();

    let request = WorkRequest {
        arguments: vec!["--sources".to_string(), "B.java".to_string()],
        ..Default::default()
    };
    handler
        .respond_to_request(&test_worker_io(), &request, &request_info())
        .unwrap();

    let response = response_from(out.0.lock().clone());
    assert_eq!(response.request_id, 0);
    assert_eq!(response.exit_code, 2);
    assert!(response.output.is_empty());
}

/// `(st_dev, st_ino)` of a descriptor: which file it is.
fn file_identity(fd: i32) -> (u64, u64) {
    // SAFETY: fstat writes into the struct.
    unsafe {
        let mut st: libc::stat = std::mem::zeroed();
        assert_eq!(libc::fstat(fd, &mut st), 0);
        (st.st_dev, st.st_ino)
    }
}

/// PORT: Java compares the `System` stream objects; Blaze compares the files the descriptors
/// name.
#[test]
fn test_worker_io_does_wrap_system_streams() {
    if !in_child_process("test_worker_io_does_wrap_system_streams") {
        return;
    }
    let original = [file_identity(0), file_identity(1), file_identity(2)];
    let io = WorkerIO::capture();
    // Assert that the WorkerIO returns the original streams and the standard descriptors have
    // been swapped out.
    let wrapped = [file_identity(0), file_identity(1), file_identity(2)];
    let originals = [
        file_identity(io.get_original_input_stream().unwrap()),
        file_identity(io.get_original_output_stream().unwrap()),
        file_identity(io.get_original_error_stream().unwrap()),
    ];
    io.close().unwrap();
    assert_eq!(originals, original);
    for i in 0..3 {
        assert_ne!(wrapped[i], original[i]);
    }
    // close() puts the originals back.
    assert_eq!(
        [file_identity(0), file_identity(1), file_identity(2)],
        original
    );
}

/// Writes straight to a descriptor (the test harness captures Rust's `print!` itself).
fn write_fd(fd: i32, s: &str) {
    // SAFETY: plain write of a buffer we own.
    let n = unsafe { libc::write(fd, s.as_ptr().cast(), s.len()) };
    assert_eq!(n, s.len() as isize);
}

#[test]
fn test_worker_io_does_capture_standard_out_and_error_streams() {
    if !in_child_process("test_worker_io_does_capture_standard_out_and_error_streams") {
        return;
    }
    let io = WorkerIO::capture();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        // Assert that nothing has been captured in the new instance
        assert!(io.read_captured_as_utf8_string().unwrap().is_empty());

        // Assert that the standard out/error stream redirect to our own streams
        write_fd(1, "This is a standard out message!");
        write_fd(2, "This is a standard error message!");
        assert_eq!(
            io.read_captured_as_utf8_string().unwrap(),
            "This is a standard out message!This is a standard error message!"
        );

        // Assert that readCapturedAsUtf8String calls reset on the captured stream after a read
        assert!(io.read_captured_as_utf8_string().unwrap().is_empty());

        write_fd(1, "out 1");
        write_fd(2, "err 1");
        write_fd(1, "out 2");
        write_fd(2, "err 2");
        assert_eq!(
            io.read_captured_as_utf8_string().unwrap(),
            "out 1err 1out 2err 2"
        );
        assert!(io.read_captured_as_utf8_string().unwrap().is_empty());
    }));
    io.close().unwrap();
    if let Err(payload) = result {
        std::panic::resume_unwind(payload);
    }
}

// Blaze's own: the worker side of the protocols reads what the server side writes.

#[test]
fn proto_message_processor_reads_requests_until_the_end_of_input() {
    let mut input = Vec::new();
    for id in [1, 2] {
        input.extend(delimited(&WorkRequest {
            arguments: args(),
            request_id: id,
            ..Default::default()
        }));
    }
    let processor = ProtoWorkerMessageProcessor::new(
        Box::new(ByteArrayInputStream::new(input)),
        empty_stderr(),
    );
    assert_eq!(
        processor.read_work_request().unwrap().unwrap().request_id,
        1
    );
    assert_eq!(
        processor.read_work_request().unwrap().unwrap().request_id,
        2
    );
    assert!(processor.read_work_request().unwrap().is_none());
    let mut truncated = ByteArrayInputStream::new(vec![10, 1]);
    let e = parse_delimited_work_request(&mut truncated).unwrap_err();
    assert!(
        JavaException::from_io_error(&e)
            .to_string()
            .starts_with("com.google.protobuf.InvalidProtocolBufferException: While parsing")
    );
}

#[test]
fn json_message_processor_reads_requests_and_writes_responses() {
    let input = br#"{"arguments":["a","b"],"inputs":[{"path":"p","digest":"d","x":1}],"requestId":3,"verbosity":11,"sandboxDir":"sd","unknown":[1]}"#;
    let out = SharedBuffer::default();
    let processor = JsonWorkerMessageProcessor::new(
        JsonReader::new(Box::new(ByteArrayInputStream::new(input.to_vec()))),
        Box::new(out.clone()),
    );
    let request = processor.read_work_request().unwrap().unwrap();
    assert_eq!(request.arguments, vec!["a", "b"]);
    assert_eq!(request.inputs.len(), 1);
    assert_eq!(request.inputs[0].path, "p");
    assert_eq!(request.inputs[0].digest, b"d");
    assert_eq!(request.request_id, 3);
    assert_eq!(request.verbosity, 11);
    assert_eq!(request.sandbox_dir, "sd");
    // After the document, Gson's lenient reader peeks END_DOCUMENT and beginObject fails.
    let e = processor.read_work_request().unwrap_err();
    assert!(
        JavaException::from_io_error(&e).to_string().starts_with(
            "java.io.IOException: java.lang.IllegalStateException: Expected BEGIN_OBJECT but was \
             END_DOCUMENT"
        ),
        "{}",
        JavaException::from_io_error(&e)
    );

    processor
        .write_work_response(&WorkResponse {
            exit_code: 1,
            output: "o".to_string(),
            request_id: 3,
            was_cancelled: false,
        })
        .unwrap();
    // alwaysPrintFieldsWithNoPresence, no whitespace, no newline.
    assert_eq!(
        String::from_utf8(out.0.lock().clone()).unwrap(),
        r#"{"exitCode":1,"output":"o","requestId":3,"wasCancelled":false}"#
    );
}
