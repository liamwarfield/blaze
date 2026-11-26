//! Port of src/test/java/com/google/devtools/build/lib/worker/SandboxedWorkerProxyTest.java.
//!
//! PORT: Java uses an `InMemoryFileSystem`; these tests use the real file system under a fresh
//! temporary directory. Java's anonymous `WorkerMultiplexer` subclass overriding
//! `createProcess` is `override_create_process_for_testing`. Java casts `factory.create(key)` to
//! `SandboxedWorkerProxy`; here the `Arc<dyn Worker>` is kept and `as_worker_proxy` borrows it.

mod worker_common;

use std::sync::Arc;

use blaze_exec::actions::execution_requirements::{self as er, WorkerProtocolFormat};
use blaze_exec::worker::worker::Worker;
use blaze_exec::worker::worker_factory::WorkerFactory;
use blaze_exec::worker::worker_key::WorkerKey;
use blaze_exec::worker::worker_multiplexer::{WorkerMultiplexer, WorkerMultiplexerManager};
use blaze_exec::worker::worker_proxy::WorkerProxy;
use blaze_protos::blaze::worker::WorkRequest;
use blaze_vfs::{DigestHashFunction, FileSystemExt, Path, UnixFileSystem};
use indexmap::IndexMap;
use worker_common::sandbox_helper::{SandboxHelper, read_content};
use worker_common::*;

struct Fixture {
    _tmp: tempfile::TempDir,
    global_exec_root: Path,
    worker_base_dir: Path,
    global_output_base: Path,
}

// Java: src/test/java/com/google/devtools/build/lib/worker/SandboxedWorkerProxyTest.java:53 SandboxedWorkerProxyTest.setUp
fn set_up() -> Fixture {
    let tmp = tempfile::tempdir().unwrap();
    let fs = UnixFileSystem::create(DigestHashFunction::Sha256);
    let test_root = fs.get_path(tmp.path().to_str().unwrap());

    let global_output_base = test_root.get_child("outputbase");
    let global_exec_root = global_output_base.get_child("execroot");
    global_exec_root.create_directory_and_parents().unwrap();

    let worker_base_dir = test_root.get_relative_str("bazel-workers");
    worker_base_dir.create_directory_and_parents().unwrap();
    Fixture {
        _tmp: tmp,
        global_exec_root,
        worker_base_dir,
        global_output_base,
    }
}

/// `ImmutableMap.copyOf(System.getenv())`.
fn client_env() -> IndexMap<String, String> {
    std::env::vars().collect()
}

fn proxy(worker: &Arc<dyn Worker>) -> &WorkerProxy {
    worker.as_worker_proxy().expect("SandboxedWorkerProxy")
}

fn multiplex_sandboxed_key(f: &Fixture, mnemonic: &str) -> (Arc<WorkerKey>, WorkerFactory) {
    let mut req = exec_requirements_builder(mnemonic);
    req.insert(
        er::SUPPORTS_MULTIPLEX_SANDBOXING.to_string(),
        "1".to_string(),
    );
    let spawn = create_spawn(&[], req);

    let mut options = worker_options();
    options.worker_multiplex = true;
    options.multiplex_sandboxing = true;

    let key = Arc::new(create_worker_key_from_options(
        WorkerProtocolFormat::Proto,
        &f.global_output_base,
        &options,
        true,
        &spawn,
        &["worker.sh"],
    ));
    let factory = WorkerFactory::new(f.worker_base_dir.clone(), Arc::new(options));
    (key, factory)
}

fn work_dir_of(f: &Fixture, mnemonic: &str, multiplexer_id: i32) -> Path {
    f.worker_base_dir
        .get_child(&format!(
            "{mnemonic}-multiplex-worker-{multiplexer_id}-workdir"
        ))
        .get_child("execroot")
}

// Java: src/test/java/com/google/devtools/build/lib/worker/SandboxedWorkerProxyTest.java:230 SandboxedWorkerProxyTest.createSandboxedWorkerProxies
fn create_sandboxed_worker_proxies(
    f: &Fixture,
    mnemonic: &str,
    num_proxies_to_create: usize,
) -> Vec<Arc<dyn Worker>> {
    let (key, factory) = multiplex_sandboxed_key(f, mnemonic);
    assert!(num_proxies_to_create > 0);
    (0..num_proxies_to_create)
        .map(|_| factory.create(&key).unwrap())
        .collect()
}

// Java: src/test/java/com/google/devtools/build/lib/worker/SandboxedWorkerProxyTest.java:256 SandboxedWorkerProxyTest.createFakedSandboxedWorkerProxy
fn create_faked_sandboxed_worker_proxy(f: &Fixture) -> Arc<dyn Worker> {
    let (key, factory) = multiplex_sandboxed_key(f, "Mnem");
    let multiplexer = WorkerMultiplexer::new(
        f.global_exec_root.get_child("testWorker.log"),
        key.clone(),
        0,
    );
    multiplexer.override_create_process_for_testing(Arc::new(|| {
        // new FakeSubprocess(new PipedInputStream())
        Arc::new(FakeSubprocessHandle(FakeSubprocess::new(Vec::new())))
            as Arc<dyn blaze_shell::Subprocess>
    }));
    WorkerMultiplexerManager::inject_for_testing(&key, multiplexer);
    factory.create(&key).unwrap()
}

// Java: src/test/java/com/google/devtools/build/lib/worker/SandboxedWorkerProxyTest.java:66 SandboxedWorkerProxyTest.prepareExecution_createsFilesInSandbox
#[test]
fn prepare_execution_creates_files_in_sandbox() {
    let f = set_up();
    let worker = create_sandboxed_worker_proxies(&f, "Mnem", 1).remove(0);
    let proxy = proxy(&worker);
    let multiplexer_id = proxy.get_worker_multiplexer().get_multiplexer_id();
    let work_dir = work_dir_of(&f, "Mnem", multiplexer_id);
    let sandbox_dir = work_dir
        .get_child("__sandbox")
        .get_child(&proxy.get_worker_id().to_string())
        .get_child("execroot");
    let sandbox_helper = SandboxHelper::new(&f.global_exec_root, &work_dir)
        .add_and_create_input_file("anInputFile", "anInputFile", "Just stuff")
        // Worker files are expected to also be inputs.
        .add_input_file("worker.sh", Some("worker.sh"))
        .add_output("very/output.txt")
        .add_and_create_worker_file("worker.sh", "#!/bin/bash");

    // PipedInputStream with no writer: reading it fails at once, as an empty stream ends here.
    proxy
        .get_worker_multiplexer()
        .set_process_factory(Arc::new(FakeSubprocessFactory(FakeSubprocess::new(
            Vec::new(),
        ))));

    proxy
        .prepare_execution(
            &sandbox_helper.get_sandbox_inputs(),
            &sandbox_helper.get_sandbox_outputs(),
            &sandbox_helper.get_worker_files(),
            &client_env(),
        )
        .unwrap();

    assert!(work_dir.is_directory());
    assert!(work_dir.get_child("worker.sh").exists());
    assert!(work_dir.get_child("worker.sh").is_symbolic_link());
    assert!(sandbox_dir.is_directory());
    assert!(sandbox_dir.get_child("anInputFile").exists());
    assert!(sandbox_dir.get_child("anInputFile").is_symbolic_link());
    assert!(sandbox_dir.get_child("very").exists());
    assert!(sandbox_dir.get_child("very").is_directory());
}

// Java: src/test/java/com/google/devtools/build/lib/worker/SandboxedWorkerProxyTest.java:102 SandboxedWorkerProxyTest.putRequest_setsSandboxDir
#[test]
fn put_request_sets_sandbox_dir() {
    let f = set_up();
    let worker_arc = create_faked_sandboxed_worker_proxy(&f);
    let worker = proxy(&worker_arc);
    let multiplexer_id = worker.get_worker_multiplexer().get_multiplexer_id();
    let work_dir = work_dir_of(&f, "Mnem", multiplexer_id);
    let sandbox_helper = SandboxHelper::new(&f.global_exec_root, &work_dir)
        .add_and_create_input_file("anInputFile", "anInputFile", "Just stuff")
        .add_output("very/output.txt")
        .add_and_create_worker_file("worker.sh", "#!/bin/bash");
    worker
        .prepare_execution(
            &sandbox_helper.get_sandbox_inputs(),
            &sandbox_helper.get_sandbox_outputs(),
            &sandbox_helper.get_worker_files(),
            &client_env(),
        )
        .unwrap();
    worker
        .put_request(&WorkRequest {
            request_id: 2,
            ..Default::default()
        })
        .unwrap();
    assert!(worker.get_worker_multiplexer().pending_request_count() > 0);
    let actual_request = worker
        .get_worker_multiplexer()
        .take_pending_request_for_testing()
        .unwrap();
    assert_eq!(actual_request.request_id, 2);
    assert_eq!(
        actual_request.sandbox_dir,
        format!("__sandbox/{}/execroot", worker.get_worker_id())
    );
}

// Java: src/test/java/com/google/devtools/build/lib/worker/SandboxedWorkerProxyTest.java:127 SandboxedWorkerProxyTest.finishExecution_copiesOutputs
#[test]
fn finish_execution_copies_outputs() {
    let f = set_up();
    let worker_arc = create_faked_sandboxed_worker_proxy(&f);
    let worker = proxy(&worker_arc);
    let multiplexer_id = worker.get_worker_multiplexer().get_multiplexer_id();
    let work_dir = work_dir_of(&f, "Mnem", multiplexer_id);
    let sandbox_helper = SandboxHelper::new(&f.global_exec_root, &work_dir)
        .add_and_create_input_file("anInputFile", "anInputFile", "Just stuff")
        .add_output("very/output.txt")
        .add_output("rootFile")
        .add_and_create_worker_file("worker.sh", "#!/bin/bash");
    worker
        .prepare_execution(
            &sandbox_helper.get_sandbox_inputs(),
            &sandbox_helper.get_sandbox_outputs(),
            &sandbox_helper.get_worker_files(),
            &client_env(),
        )
        .unwrap();
    worker
        .put_request(&WorkRequest {
            request_id: 2,
            ..Default::default()
        })
        .unwrap();
    let actual_request = worker
        .get_worker_multiplexer()
        .take_pending_request_for_testing()
        .unwrap();
    let request_sandbox_subdir = actual_request.sandbox_dir;

    // Pretend to do work.
    let sandbox_helper = sandbox_helper
        .create_exec_root_file(
            &format!("{request_sandbox_subdir}/very/output.txt"),
            "some output",
        )
        .create_exec_root_file("very/output.txt", "some wrongly placed output")
        .create_exec_root_file(
            &format!("{request_sandbox_subdir}/rootFile"),
            "some output in root",
        )
        .create_exec_root_file(
            &format!("{request_sandbox_subdir}/randomFile"),
            "some randomOutput",
        );

    worker
        .finish_execution(&f.global_exec_root, &sandbox_helper.get_sandbox_outputs())
        .unwrap();

    assert!(!f.global_exec_root.get_child("randomFile").exists());
    assert_eq!(
        read_content(&f.global_exec_root.get_child("rootFile")),
        "some output in root"
    );
    assert_eq!(
        read_content(&f.global_exec_root.get_child("very").get_child("output.txt")),
        "some output"
    );
}

// Java: src/test/java/com/google/devtools/build/lib/worker/SandboxedWorkerProxyTest.java:168 SandboxedWorkerProxyTest.differentProxiesSameMultiplexerHaveSameWorkDir
#[test]
fn different_proxies_same_multiplexer_have_same_work_dir() {
    let f = set_up();
    let proxies = create_sandboxed_worker_proxies(&f, "Mnem", 2);
    let proxy_one = proxy(&proxies[0]);
    let proxy_two = proxy(&proxies[1]);

    let multiplexer_id_proxy_one = proxy_one.get_worker_multiplexer().get_multiplexer_id();
    let expected_work_dir_proxy_one = work_dir_of(&f, "Mnem", multiplexer_id_proxy_one);

    let multiplexer_id_proxy_two = proxy_two.get_worker_multiplexer().get_multiplexer_id();
    let expected_work_dir_proxy_two = work_dir_of(&f, "Mnem", multiplexer_id_proxy_two);

    assert_eq!(proxy_one.get_work_dir(), proxy_two.get_work_dir());
    assert_eq!(proxy_one.get_work_dir(), &expected_work_dir_proxy_one);
    assert_eq!(proxy_two.get_work_dir(), &expected_work_dir_proxy_two);
}

// Java: src/test/java/com/google/devtools/build/lib/worker/SandboxedWorkerProxyTest.java:191 SandboxedWorkerProxyTest.differentProxiesDifferentMultiplexerSameMnemHaveDifferentWorkDirs
#[test]
fn different_proxies_different_multiplexer_same_mnem_have_different_work_dirs() {
    let f = set_up();
    let shared_mnemonic = "Mnem";

    // Create a proxy on the first multiplexer
    let (key, factory) = multiplex_sandboxed_key(&f, shared_mnemonic);

    let proxy_one_multiplexer_one_arc = factory.create(&key).unwrap();
    let proxy_one_multiplexer_one = proxy(&proxy_one_multiplexer_one_arc);
    let multiplexer_id_one = proxy_one_multiplexer_one
        .get_worker_multiplexer()
        .get_multiplexer_id();
    let expected_work_dir_one = work_dir_of(&f, shared_mnemonic, multiplexer_id_one);

    // Shut down the first multiplexer, so we get a different multiplexer for the next proxy
    WorkerMultiplexerManager::remove_instance(&key).unwrap();

    // Create a proxy on the second multiplexer
    let proxy_one_multiplexer_two_arc = factory.create(&key).unwrap();
    let proxy_one_multiplexer_two = proxy(&proxy_one_multiplexer_two_arc);
    let multiplexer_id_two = proxy_one_multiplexer_two
        .get_worker_multiplexer()
        .get_multiplexer_id();
    let expected_work_dir_two = work_dir_of(&f, shared_mnemonic, multiplexer_id_two);

    assert_ne!(
        proxy_one_multiplexer_one.get_work_dir(),
        proxy_one_multiplexer_two.get_work_dir()
    );
    assert_eq!(
        proxy_one_multiplexer_one.get_work_dir(),
        &expected_work_dir_one
    );
    assert_eq!(
        proxy_one_multiplexer_two.get_work_dir(),
        &expected_work_dir_two
    );
}
