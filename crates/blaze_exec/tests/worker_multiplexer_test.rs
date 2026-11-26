//! Port of src/test/java/com/google/devtools/build/lib/worker/WorkerMultiplexerTest.java.
//!
//! PORT: Java uses an `InMemoryFileSystem` and a `PipedInputStream`; these tests use the real file
//! system (paths under a temporary directory where files are touched) and an OS pipe. The
//! `WorkerMultiplexerManager` is process-wide and `tearDown` resets it, so the tests take a lock
//! (JUnit runs one test at a time).

mod worker_common;

use std::collections::BTreeSet;
use std::io::Write;
use std::sync::{Arc, Mutex, MutexGuard};

use blaze_exec::actions::execution_requirements::WorkerProtocolFormat;
use blaze_exec::sandbox::sandbox_helpers::{SandboxInputs, SandboxOutputs};
use blaze_exec::worker::worker::Worker;
use blaze_exec::worker::worker_key::WorkerKey;
use blaze_exec::worker::worker_multiplexer::{WorkerMultiplexer, WorkerMultiplexerManager};
use blaze_exec::worker::worker_proxy::WorkerProxy;
use blaze_protos::JavaMessage;
use blaze_protos::blaze::worker::{WorkRequest, WorkResponse};
use blaze_vfs::{FileSystemExt, FileSystemRef, Path};
use indexmap::IndexMap;
use worker_common::*;

static SERIAL: Mutex<()> = Mutex::new(());

struct Fixture {
    _serial: MutexGuard<'static, ()>,
    _tmp: tempfile::TempDir,
    file_system: FileSystemRef,
    test_root: Path,
    log_path: Path,
}

impl Fixture {
    // Java: src/test/java/com/google/devtools/build/lib/worker/WorkerMultiplexerTest.java:47 WorkerMultiplexerTest.setUp
    fn set_up() -> Fixture {
        let serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = tempfile::tempdir().unwrap();
        let file_system = fs();
        let test_root = file_system.get_path(tmp.path().to_str().unwrap());
        let log_path = test_root.get_relative_str("tmp/logs4");
        log_path.create_directory_and_parents().unwrap();
        Fixture {
            _serial: serial,
            _tmp: tmp,
            file_system,
            test_root,
            log_path,
        }
    }
}

impl Drop for Fixture {
    // Java: src/test/java/com/google/devtools/build/lib/worker/WorkerMultiplexerTest.java:54 WorkerMultiplexerTest.tearDown
    fn drop(&mut self) {
        WorkerMultiplexerManager::reset_for_testing();
    }
}

/// `WorkerTestUtils.createWorkerKey(fileSystem, mnemonic, proxied, args...)`.
// Java: src/test/java/com/google/devtools/build/lib/worker/WorkerTestUtils.java:84 WorkerTestUtils.createWorkerKey
fn create_worker_key_proxied(
    fs: &FileSystemRef,
    mnemonic: &str,
    proxied: bool,
    args: &[&str],
) -> Arc<WorkerKey> {
    Arc::new(create_worker_key_full(
        WorkerProtocolFormat::Proto,
        fs,
        mnemonic,
        proxied,
        /* sandboxed= */ false,
        /* dynamic= */ false,
        args,
    ))
}

/// `new PipedInputStream()` + `new PipedOutputStream(serverInputStream)` +
/// `multiplexer.setProcessFactory(params -> new FakeSubprocess(serverInputStream))`.
fn fake_process(multiplexer: &WorkerMultiplexer) -> std::io::PipeWriter {
    let (server_input_stream, worker_output_stream) = std::io::pipe().unwrap();
    let fake = FakeSubprocess::with_stream(Box::new(server_input_stream));
    multiplexer.set_process_factory(Arc::new(FakeSubprocessFactory(fake)));
    worker_output_stream
}

fn prepare_execution(worker: &WorkerProxy) {
    // worker.prepareExecution(null, null, null, ImmutableMap.of())
    worker
        .prepare_execution(
            &SandboxInputs::get_empty_inputs(),
            &SandboxOutputs::get_empty_instance(),
            &BTreeSet::new(),
            &IndexMap::new(),
        )
        .unwrap();
}

fn request(id: i32) -> WorkRequest {
    WorkRequest {
        request_id: id,
        ..Default::default()
    }
}

fn write_response(out: &mut impl Write, id: i32) {
    let response = WorkResponse {
        request_id: id,
        ..Default::default()
    };
    let mut buf = Vec::new();
    response.write_delimited_to(&mut buf);
    out.write_all(&buf).unwrap();
}

fn new_proxy(
    key: &Arc<WorkerKey>,
    id: i32,
    log_path: &Path,
    multiplexer: &Arc<WorkerMultiplexer>,
) -> Arc<WorkerProxy> {
    Arc::new(WorkerProxy::new(
        key.clone(),
        id,
        log_path.clone(),
        multiplexer.clone(),
        key.get_exec_root().clone(),
    ))
}

fn get_response_async(
    worker: &Arc<WorkerProxy>,
    id: i32,
) -> std::thread::JoinHandle<Option<WorkResponse>> {
    let worker = worker.clone();
    std::thread::spawn(move || worker.get_response(id).unwrap())
}

// Java: src/test/java/com/google/devtools/build/lib/worker/WorkerMultiplexerTest.java:59 WorkerMultiplexerTest.testGetResponse_noOutstandingRequests
#[test]
fn test_get_response_no_outstanding_requests() {
    let f = Fixture::set_up();
    let worker_key = create_worker_key_proxied(&f.file_system, "test1", true, &["fakeBinary"]);
    let multiplexer = WorkerMultiplexerManager::get_instance(&worker_key, &f.log_path);

    let mut worker_output_stream = fake_process(&multiplexer);

    let request1 = request(1);
    let worker = new_proxy(&worker_key, 2, &f.log_path, &multiplexer);
    prepare_execution(&worker);
    worker.put_request(&request1).unwrap();
    write_response(&mut worker_output_stream, 1);
    worker_output_stream.flush().unwrap();
    let response = worker.get_response(1).unwrap().unwrap();
    assert_eq!(response.request_id, 1);
    // Can't get the same response twice - but the responseChecker is gone, so it just returns
    // null
    assert!(multiplexer.get_response(1).unwrap().is_none());
    assert!(multiplexer.no_outstanding_requests());
}

// Java: src/test/java/com/google/devtools/build/lib/worker/WorkerMultiplexerTest.java:83 WorkerMultiplexerTest.testGetResponse_basicConcurrency
#[test]
fn test_get_response_basic_concurrency() {
    let f = Fixture::set_up();
    let worker_key = create_worker_key_proxied(&f.file_system, "test2", true, &["fakeBinary"]);
    let multiplexer = WorkerMultiplexerManager::get_instance(&worker_key, &f.log_path);

    let mut worker_output_stream = fake_process(&multiplexer);

    let worker1 = new_proxy(&worker_key, 1, &f.log_path, &multiplexer);
    prepare_execution(&worker1);
    worker1.put_request(&request(3)).unwrap();

    let worker2 = new_proxy(&worker_key, 2, &f.log_path, &multiplexer);
    prepare_execution(&worker2);
    worker2.put_request(&request(42)).unwrap();

    let response1 = get_response_async(&worker1, 3);
    let response2 = get_response_async(&worker2, 42);

    // Responses can arrive out of order
    write_response(&mut worker_output_stream, 42);
    write_response(&mut worker_output_stream, 3);
    worker_output_stream.flush().unwrap();

    assert_eq!(response1.join().unwrap().unwrap().request_id, 3);
    assert_eq!(response2.join().unwrap().unwrap().request_id, 42);
    assert!(multiplexer.no_outstanding_requests());
}

// Java: src/test/java/com/google/devtools/build/lib/worker/WorkerMultiplexerTest.java:115 WorkerMultiplexerTest.testGetResponse_slowMultiplexer
#[test]
fn test_get_response_slow_multiplexer() {
    let f = Fixture::set_up();
    let worker_key = create_worker_key_proxied(&f.file_system, "test3", true, &["fakeBinary"]);
    let multiplexer = WorkerMultiplexerManager::get_instance(&worker_key, &f.log_path);

    let mut worker_output_stream = fake_process(&multiplexer);

    let worker1 = new_proxy(&worker_key, 1, &f.log_path, &multiplexer);
    prepare_execution(&worker1);
    worker1.put_request(&request(3)).unwrap();

    let worker2 = new_proxy(&worker_key, 2, &f.log_path, &multiplexer);
    prepare_execution(&worker2);
    worker2.put_request(&request(42)).unwrap();

    let started = Arc::new(std::sync::Barrier::new(3));
    let spawn = |worker: &Arc<WorkerProxy>, id: i32| {
        let worker = worker.clone();
        let started = started.clone();
        std::thread::spawn(move || {
            started.wait();
            worker.get_response(id).unwrap()
        })
    };
    let response1 = spawn(&worker1, 3);
    let response2 = spawn(&worker2, 42);

    // Makes sure both workers are waiting for responses before the multiplexer processes
    // anything.
    // PORT: Java polls `Thread.getState() == WAITING`, which Rust threads do not expose; the
    // test waits until both threads run and then gives them time to block on their semaphores.
    started.wait();
    std::thread::sleep(std::time::Duration::from_millis(200));

    // Responses can arrive out of order
    write_response(&mut worker_output_stream, 42);
    write_response(&mut worker_output_stream, 3);
    worker_output_stream.flush().unwrap();

    assert_eq!(response1.join().unwrap().unwrap().request_id, 3);
    assert_eq!(response2.join().unwrap().unwrap().request_id, 42);
    assert!(multiplexer.no_outstanding_requests());
}

// Java: src/test/java/com/google/devtools/build/lib/worker/WorkerMultiplexerTest.java:182 WorkerMultiplexerTest.testGetResponse_slowProxy
#[test]
fn test_get_response_slow_proxy() {
    let f = Fixture::set_up();
    let worker_key = create_worker_key_proxied(&f.file_system, "test4", true, &["fakeBinary"]);
    let multiplexer = WorkerMultiplexerManager::get_instance(&worker_key, &f.log_path);

    let mut worker_output_stream = fake_process(&multiplexer);

    let worker1 = new_proxy(&worker_key, 1, &f.log_path, &multiplexer);
    prepare_execution(&worker1);
    worker1.put_request(&request(3)).unwrap();

    let worker2 = new_proxy(&worker_key, 2, &f.log_path, &multiplexer);
    prepare_execution(&worker2);
    worker2.put_request(&request(42)).unwrap();

    // Responses can arrive out of order, and before the workerproxies are ready to get them.
    write_response(&mut worker_output_stream, 42);
    write_response(&mut worker_output_stream, 3);
    worker_output_stream.flush().unwrap();

    let response1 = get_response_async(&worker1, 3);
    let response2 = get_response_async(&worker2, 42);

    assert_eq!(response1.join().unwrap().unwrap().request_id, 3);
    assert_eq!(response2.join().unwrap().unwrap().request_id, 42);
    assert!(multiplexer.no_outstanding_requests());
}

// Java: src/test/java/com/google/devtools/build/lib/worker/WorkerMultiplexerTest.java:229 WorkerMultiplexerTest.workDir_destroyMultiplexer_successfullyDestroysWorkDir
#[test]
fn work_dir_destroy_multiplexer_successfully_destroys_work_dir() {
    let f = Fixture::set_up();

    let worker_key =
        create_worker_key_proxied(&f.file_system, "TestMnemonic", true, &["fakeBinary"]);
    let multiplexer = WorkerMultiplexerManager::get_instance(&worker_key, &f.log_path);

    // testRoot.getRelative("/tmp/workdir"): an absolute path, under the temporary root here.
    let work_dir = f.test_root.get_relative_str("tmp/workdir");
    work_dir.create_directory_and_parents().unwrap();
    assert!(work_dir.exists());

    multiplexer.set_work_dir(work_dir.clone());
    multiplexer.destroy_multiplexer();
    assert!(!work_dir.exists());
}
