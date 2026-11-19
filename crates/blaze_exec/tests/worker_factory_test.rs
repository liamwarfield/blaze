//! Ports of src/test/java/com/google/devtools/build/lib/worker/WorkerFactoryTest.java and
//! WorkerMultiplexerManagerTest.java (both use the process-wide WorkerMultiplexerManager, so they
//! share one binary and run one at a time).

mod worker_common;

use std::collections::BTreeMap;
use std::sync::Arc;

use blaze_exec::actions::execution_requirements::WorkerProtocolFormat;
use blaze_exec::worker::worker::Worker;
use blaze_exec::worker::worker_factory::WorkerFactory;
use blaze_exec::worker::worker_key::WorkerKey;
use blaze_exec::worker::worker_multiplexer::WorkerMultiplexerManager;
use blaze_exec::worker::worker_process_status::Status;
use blaze_vfs::{FileSystemExt, FileSystemRef, Path};
use indexmap::IndexMap;
use parking_lot::Mutex;
use worker_common::*;

/// The tests use the process-wide multiplexer manager.
static SERIAL: Mutex<()> = Mutex::new(());

struct Dir {
    _tmp: tempfile::TempDir,
    fs: FileSystemRef,
    root: String,
}

impl Dir {
    fn new() -> Dir {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().to_str().unwrap().to_string();
        Dir {
            _tmp: tmp,
            fs: fs(),
            root,
        }
    }
    /// `fs.getPath(p)` with `p` absolute in the Java test's in-memory file system.
    fn path(&self, p: &str) -> Path {
        self.fs.get_path(&format!("{}{}", self.root, p))
    }
}

// Java: src/test/java/com/google/devtools/build/lib/worker/WorkerFactoryTest.java:131 WorkerFactoryTest.createWorkerKey
fn create_worker_key(
    dir: &Dir,
    must_be_sandboxed: bool,
    multiplex: bool,
    args: &[&str],
) -> Arc<WorkerKey> {
    Arc::new(WorkerKey::new(
        args.iter().map(|s| s.to_string()).collect(),
        IndexMap::new(),
        dir.path("/outputbase/execroot/workspace"),
        "dummy".to_string(),
        vec![0, 0, 0, 0],
        BTreeMap::new(),
        must_be_sandboxed,
        false,
        multiplex,
        false,
        WorkerProtocolFormat::Proto,
    ))
}

fn factory(dir: &Dir) -> WorkerFactory {
    let mut options = worker_options();
    options.worker_sandboxing = false;
    WorkerFactory::with_options(
        dir.path("/outputbase/bazel-workers"),
        Arc::new(options),
        None,
        Some(Arc::new(
            blaze_exec::sandbox::asynchronous_tree_deleter::AsynchronousTreeDeleter::new(
                dir.path("/outputbase/bazel-workers/_moved_trash_dir"),
            ),
        )),
    )
}

/// Regression test for b/64689608: The execroot of the sandboxed worker process must end with
/// the workspace name, just like the normal execroot does.
#[test]
fn sandboxed_worker_path_ends_with_workspace_name() {
    let _s = SERIAL.lock();
    let dir = Dir::new();
    let worker_factory = factory(&dir);
    let worker_key = create_worker_key(&dir, true, false, &[]);
    let sandboxed_worker_path = worker_factory.get_sandboxed_worker_path(&worker_key, 1);
    assert_eq!(sandboxed_worker_path.get_base_name(), "workspace");
    WorkerMultiplexerManager::reset_for_testing();
}

/// WorkerFactory should create correct worker type based on WorkerKey.
#[test]
fn worker_creation_type_check() {
    let _s = SERIAL.lock();
    let dir = Dir::new();
    let worker_factory = factory(&dir);
    let sandboxed_worker_key = create_worker_key(&dir, true, false, &[]);
    let sandboxed_worker = worker_factory.create(&sandboxed_worker_key).unwrap();
    // SandboxedWorker.class
    assert!(sandboxed_worker.is_sandboxed());
    assert!(!sandboxed_worker.to_string().contains("proxy"));

    let non_proxied_worker_key = create_worker_key(&dir, false, false, &[]);
    let non_proxied_worker = worker_factory.create(&non_proxied_worker_key).unwrap();
    // SingleplexWorker.class
    assert!(!non_proxied_worker.is_sandboxed());
    assert!(!non_proxied_worker.to_string().contains("proxy"));

    let proxied_worker_key = create_worker_key(&dir, false, true, &[]);
    let proxied_worker = worker_factory.create(&proxied_worker_key).unwrap();
    // If proxied = true, WorkerProxy is created along with a WorkerMultiplexer.
    // Destroy WorkerMultiplexer to avoid unexpected behavior in WorkerMultiplexerManagerTest.
    WorkerMultiplexerManager::remove_instance(&proxied_worker_key).unwrap();
    // WorkerProxy.class
    assert!(!proxied_worker.is_sandboxed());
    assert!(proxied_worker.to_string().contains("proxy worker"));
    WorkerMultiplexerManager::reset_for_testing();
}

/// Proxied workers with the same WorkerKey should share the log file.
#[test]
fn test_multiplex_workers_share_logfiles() {
    let _s = SERIAL.lock();
    let dir = Dir::new();
    let worker_factory = factory(&dir);
    let worker_key1 = create_worker_key(&dir, false, true, &["arg1"]);
    let proxied_worker1a = worker_factory.create(&worker_key1).unwrap();
    let proxied_worker1b = worker_factory.create(&worker_key1).unwrap();
    let worker_key2 = create_worker_key(&dir, false, true, &["arg2"]);
    let proxied_worker2 = worker_factory.create(&worker_key2).unwrap();
    assert_eq!(
        proxied_worker1a.get_log_file(),
        proxied_worker1b.get_log_file()
    );
    assert_ne!(
        proxied_worker1a.get_log_file(),
        proxied_worker2.get_log_file()
    );
    WorkerMultiplexerManager::reset_for_testing();
}

/// WorkerFactory should create the base dir if needed and fail if that's impossible.
#[test]
fn test_create_creates_worker_directory() {
    let _s = SERIAL.lock();
    let dir = Dir::new();
    let worker_base_dir = dir.path("/outputbase/bazel-workers");
    let worker_factory = WorkerFactory::new(worker_base_dir.clone(), Arc::new(worker_options()));
    let sandboxed_worker_key = create_worker_key(&dir, true, false, &[]);
    assert!(!worker_base_dir.is_directory());
    worker_factory.create(&sandboxed_worker_key).unwrap();
    assert!(worker_base_dir.is_directory());

    worker_base_dir.delete().unwrap();
    worker_base_dir
        .create_symbolic_link(&worker_base_dir.get_relative_str("whatevs"))
        .unwrap();
    assert!(!worker_base_dir.is_directory());
    assert!(worker_factory.create(&sandboxed_worker_key).is_err());
    WorkerMultiplexerManager::reset_for_testing();
}

#[test]
fn test_doomed_worker_validation() {
    let _s = SERIAL.lock();
    let dir = Dir::new();
    let worker_factory = factory(&dir);
    let worker_key = create_worker_key(&dir, false, false, &["arg1"]);
    let worker: Arc<dyn Worker> = worker_factory.create(&worker_key).unwrap();

    assert!(worker_factory.validate_worker(&worker_key, &worker));

    worker
        .get_status()
        .maybe_update_status(Status::KilledDueToMemoryPressure);

    assert!(!worker_factory.validate_worker(&worker_key, &worker));
    WorkerMultiplexerManager::reset_for_testing();
}

// Java: src/test/java/com/google/devtools/build/lib/worker/WorkerMultiplexerManagerTest.java:44 WorkerMultiplexerManagerTest.instanceCreationRemovalTest
#[test]
fn instance_creation_removal_test() {
    let _s = SERIAL.lock();
    WorkerMultiplexerManager::reset_for_testing();
    let dir = Dir::new();
    let log_file = dir.path("/tmp/logFilePath");
    let key = |mnemonic: &str| {
        Arc::new(WorkerKey::new(
            Vec::new(),
            IndexMap::new(),
            dir.path("/execRoot"),
            mnemonic.to_string(),
            vec![0, 0, 0, 1],
            BTreeMap::new(),
            false,
            false,
            false,
            false,
            WorkerProtocolFormat::Proto,
        ))
    };
    // Create a WorkerProxy hash and request for a WorkerMultiplexer.
    let worker_key1 = key("mnemonic1");
    let wm1 = WorkerMultiplexerManager::get_instance(&worker_key1, &log_file);

    assert!(Arc::ptr_eq(
        &WorkerMultiplexerManager::get_multiplexer(&worker_key1).unwrap(),
        &wm1
    ));
    assert_eq!(
        WorkerMultiplexerManager::get_ref_count(&worker_key1).unwrap(),
        1
    );
    assert_eq!(WorkerMultiplexerManager::get_instance_count(), 1);

    // Create another WorkerProxy hash and request for a WorkerMultiplexer.
    let worker_key2 = key("mnemonic2");
    let wm2 = WorkerMultiplexerManager::get_instance(&worker_key2, &log_file);

    assert!(Arc::ptr_eq(
        &WorkerMultiplexerManager::get_multiplexer(&worker_key2).unwrap(),
        &wm2
    ));
    assert_eq!(
        WorkerMultiplexerManager::get_ref_count(&worker_key2).unwrap(),
        1
    );
    assert_eq!(WorkerMultiplexerManager::get_instance_count(), 2);

    // Use the same WorkerProxy hash, it shouldn't instantiate a new WorkerMultiplexer.
    let wm2_annex = WorkerMultiplexerManager::get_instance(&worker_key2, &log_file);

    assert!(Arc::ptr_eq(&wm2, &wm2_annex));
    assert_eq!(
        WorkerMultiplexerManager::get_ref_count(&worker_key2).unwrap(),
        2
    );
    assert_eq!(WorkerMultiplexerManager::get_instance_count(), 2);

    // Remove an instance. If reference count is larger than 0, instance shouldn't be destroyed.
    WorkerMultiplexerManager::remove_instance(&worker_key2).unwrap();

    assert_eq!(
        WorkerMultiplexerManager::get_ref_count(&worker_key2).unwrap(),
        1
    );
    assert_eq!(WorkerMultiplexerManager::get_instance_count(), 2);

    // Remove an instance. Reference count is down to 0, instance should be destroyed.
    WorkerMultiplexerManager::remove_instance(&worker_key2).unwrap();

    assert!(WorkerMultiplexerManager::get_multiplexer(&worker_key2).is_err());
    assert_eq!(WorkerMultiplexerManager::get_instance_count(), 1);

    // WorkerProxy hash not found.
    assert!(WorkerMultiplexerManager::remove_instance(&worker_key2).is_err());

    // Remove all the instances.
    WorkerMultiplexerManager::remove_instance(&worker_key1).unwrap();

    assert_eq!(WorkerMultiplexerManager::get_instance_count(), 0);
    WorkerMultiplexerManager::reset_for_testing();
}
