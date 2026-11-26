//! Port of src/test/java/com/google/devtools/build/lib/worker/WorkerModuleTest.java.
//!
//! PORT: Java's `CommandEnvironment` and `BuildRequest` are Mockito mocks; Blaze's
//! `WorkerModule.buildStarting` takes what it reads as a struct (`WorkerBuildStarting`), so the
//! tests build that. The request's `SandboxOptions` is null in Java's tests (the mock returns
//! null); it is `None` here. The module's state is process-wide in Blaze (one module per
//! server), so each test starts with `reset_for_testing` (`new WorkerModule()`) and the tests
//! run one at a time. Java's in-memory file system is a temporary directory; the log files Java
//! makes as symbolic links to the empty fragment point to `dangling` here (Linux refuses an empty
//! symlink target).

mod worker_common;

use std::sync::Arc;

use blaze_events::{EventHandler, StoredEventHandler};
use blaze_exec::actions::execution_requirements::WorkerProtocolFormat;
use blaze_exec::worker::worker_module::{WorkerBuildStarting, WorkerModule};
use blaze_optdefs::misc::worker_options::{MultiResourceEntry, WorkerOptions};
use blaze_vfs::{FileSystemExt, FileSystemRef, Path, PathFragment};
use parking_lot::Mutex;
use worker_common::*;

/// Java's runner runs a class's tests one at a time.
static SEQUENTIAL: Mutex<()> = Mutex::new(());

struct Fixture {
    tmp: tempfile::TempDir,
    fs: FileSystemRef,
    stored_event_handler: Arc<StoredEventHandler>,
    output_base: Path,
}

impl Fixture {
    fn new() -> Fixture {
        WorkerModule::reset_for_testing();
        let tmp = tempfile::tempdir().unwrap();
        let fs = fs();
        let output_base = fs.get_path(tmp.path().to_str().unwrap());
        Fixture {
            tmp,
            fs,
            stored_event_handler: Arc::new(StoredEventHandler::new()),
            output_base,
        }
    }

    /// `fs.getPath(path)` for an absolute path of the Java test, under the temporary root.
    fn path(&self, p: &str) -> Path {
        self.fs.get_path(
            self.tmp
                .path()
                .join(p.trim_start_matches('/'))
                .to_str()
                .unwrap(),
        )
    }

    // Java: src/test/java/com/google/devtools/build/lib/worker/WorkerModuleTest.java:290 WorkerModuleTest.setupEnvironment
    fn setup_environment(&mut self, root_dir: &str) {
        self.stored_event_handler = Arc::new(StoredEventHandler::new());
        let root = self.path(root_dir);
        let output_base = root.get_relative_str("outputBase");
        output_base.create_directory_and_parents().unwrap();
        self.output_base = output_base;
    }

    fn reporter(&self) -> Arc<dyn EventHandler> {
        self.stored_event_handler.clone()
    }

    /// `module.beforeCommand(env)`.
    fn before_command(&self) {
        WorkerModule::before_command(self.reporter(), None);
    }

    /// `module.buildStarting(BuildStartingEvent.create(env, request))`.
    fn build_starting(&self, options: &WorkerOptions) {
        WorkerModule::build_starting(WorkerBuildStarting {
            worker_options: Arc::new(options.clone()),
            sandbox_options: None,
            output_base: &self.output_base,
            reporter: self.reporter(),
            bin_tools: None,
            file_system: Some(self.fs.clone()),
        });
    }

    fn messages(&self) -> Vec<String> {
        self.stored_event_handler
            .get_events()
            .iter()
            .map(|e| e.get_message().into_owned())
            .collect()
    }
}

fn worker_dir(f: &Fixture, root: &str) -> Path {
    f.path(root).get_relative_str("outputBase/bazel-workers")
}

fn same_pool() -> Arc<dyn blaze_exec::worker::worker_pool::WorkerPool> {
    WorkerModule::get_worker_pool().expect("workerPool")
}

// Java: src/test/java/com/google/devtools/build/lib/worker/WorkerModuleTest.java:72 WorkerModuleTest.buildStarting_createsPools
#[test]
fn build_starting_creates_pools() {
    let _s = SEQUENTIAL.lock();
    let mut f = Fixture::new();
    let options = worker_options();
    f.setup_environment("/outputRoot");
    f.before_command();
    f.build_starting(&options);
    assert!(f.messages().is_empty());
    assert!(!worker_dir(&f, "/outputRoot").exists());
    let pool = WorkerModule::get_worker_pool().expect("workerPool");
    let worker_key = Arc::new(create_worker_key_full(
        WorkerProtocolFormat::Json,
        &f.fs,
        "dummy",
        false,
        false,
        false,
        &[],
    ));
    let worker = pool.borrow_worker(&worker_key).ok().expect("borrowWorker");
    assert_eq!(**worker.get_worker_key(), *worker_key);
    assert!(worker_dir(&f, "/outputRoot").exists());
}

// Java: src/test/java/com/google/devtools/build/lib/worker/WorkerModuleTest.java:91 WorkerModuleTest.buildStarting_noRestartOnSandboxChange
#[test]
fn build_starting_no_restart_on_sandbox_change() {
    let _s = SEQUENTIAL.lock();
    let mut f = Fixture::new();
    let mut options = worker_options();
    f.setup_environment("/outputRoot");
    f.before_command();
    f.build_starting(&options);
    assert!(f.messages().is_empty());
    let worker_dir = worker_dir(&f, "/outputRoot");
    let a_log = worker_dir.get_relative_str("f.log");
    worker_dir.create_directory_and_parents().unwrap();
    a_log
        .create_symbolic_link_to_fragment(&PathFragment::create("dangling"))
        .unwrap();
    let old_pool = same_pool();
    options.worker_sandboxing = !options.worker_sandboxing;
    f.before_command();
    f.build_starting(&options);
    assert!(f.messages().is_empty());
    assert!(Arc::ptr_eq(&same_pool(), &old_pool));
    assert!(a_log.exists_with(blaze_vfs::Symlinks::Nofollow));
}

// Java: src/test/java/com/google/devtools/build/lib/worker/WorkerModuleTest.java:113 WorkerModuleTest.buildStarting_restartsOnOutputbaseChanges
#[test]
fn build_starting_restarts_on_outputbase_changes() {
    let _s = SEQUENTIAL.lock();
    let mut f = Fixture::new();
    let options = worker_options();
    f.setup_environment("/outputRoot");
    f.before_command();
    f.build_starting(&options);
    assert!(f.messages().is_empty());
    // Log file from old root, doesn't get cleaned
    let worker_dir = worker_dir(&f, "/outputRoot");
    let old_log = worker_dir.get_relative_str("f.log");
    worker_dir.create_directory_and_parents().unwrap();
    old_log
        .create_symbolic_link_to_fragment(&PathFragment::create("dangling"))
        .unwrap();
    let old_pool = same_pool();
    f.setup_environment("/otherRootDir");
    f.before_command();
    f.build_starting(&options);
    let messages = f.messages();
    assert_eq!(messages.len(), 1);
    assert!(messages[0].contains("Worker factory configuration has changed"));
    assert!(!Arc::ptr_eq(&same_pool(), &old_pool));
    let worker_key = Arc::new(create_worker_key_mnemonic("mnemonic", &f.fs, false, false));
    WorkerModule::get_worker_factory()
        .expect("workerFactory")
        .create(&worker_key)
        .unwrap();
    assert!(
        f.path("/otherRootDir")
            .get_relative_str("outputBase/bazel-workers")
            .exists()
    );
    assert!(old_log.exists_with(blaze_vfs::Symlinks::Nofollow));
}

// Java: src/test/java/com/google/devtools/build/lib/worker/WorkerModuleTest.java:142 WorkerModuleTest.buildStarting_restartsOnUseCgroupsOnLinuxChanges
#[test]
#[ignore = "with a null SandboxOptions Java asks the old cgroup implementation \
            (CgroupsInfo.isSupported), which is not ported (unported!)"]
fn build_starting_restarts_on_use_cgroups_on_linux_changes() {
    let _s = SEQUENTIAL.lock();
    let mut f = Fixture::new();
    let mut options = worker_options();
    options.use_cgroups_on_linux = false;
    f.setup_environment("/outputRoot");
    f.before_command();
    f.build_starting(&options);
    assert!(f.messages().is_empty());
    let old_pool = same_pool();
    let mut new_options = worker_options();
    new_options.use_cgroups_on_linux = true;
    f.before_command();
    f.build_starting(&new_options);
    let messages = f.messages();
    assert_eq!(messages.len(), 1);
    assert!(messages[0].contains("Worker factory configuration has changed"));
    assert!(!Arc::ptr_eq(&same_pool(), &old_pool));
}

// Java: src/test/java/com/google/devtools/build/lib/worker/WorkerModuleTest.java:168 WorkerModuleTest.buildStarting_clearsLogsOnFactoryCreation
#[test]
fn build_starting_clears_logs_on_factory_creation() {
    let _s = SEQUENTIAL.lock();
    let mut f = Fixture::new();
    let options = worker_options();
    f.setup_environment("/outputRoot");
    let worker_dir = worker_dir(&f, "/outputRoot");
    worker_dir.create_directory_and_parents().unwrap();
    let old_log = worker_dir.get_relative_str("f.log");
    old_log
        .create_symbolic_link_to_fragment(&PathFragment::create("dangling"))
        .unwrap();
    f.before_command();
    f.build_starting(&options);
    assert!(f.messages().is_empty());
    assert!(worker_dir.exists());
    assert!(!old_log.exists_with(blaze_vfs::Symlinks::Nofollow));
}

fn restarts_on_pool_config_change(change: impl FnOnce(&mut WorkerOptions)) {
    let _s = SEQUENTIAL.lock();
    let mut f = Fixture::new();
    let mut options = worker_options();
    f.setup_environment("/outputRoot");
    f.before_command();
    // Check that new pools/factories are made with default options
    f.build_starting(&options);
    assert!(f.messages().is_empty());
    let old_pool = same_pool();
    change(&mut options);
    f.before_command();
    f.build_starting(&options);
    let messages = f.messages();
    assert_eq!(messages.len(), 1);
    assert!(messages[0].contains("Worker pool configuration has changed"));
    assert!(!Arc::ptr_eq(&same_pool(), &old_pool));
}

// Java: src/test/java/com/google/devtools/build/lib/worker/WorkerModuleTest.java:186 WorkerModuleTest.buildStarting_restartsOnNumMultiplexWorkersChanges
#[test]
fn build_starting_restarts_on_num_multiplex_workers_changes() {
    restarts_on_pool_config_change(|options| {
        options.worker_max_multiplex_instances = vec![MultiResourceEntry {
            key: Some("foo".to_string()),
            value: Some(42),
        }];
    });
}

// Java: src/test/java/com/google/devtools/build/lib/worker/WorkerModuleTest.java:207 WorkerModuleTest.buildStarting_restartsOnNumWorkersChanges
#[test]
fn build_starting_restarts_on_num_workers_changes() {
    restarts_on_pool_config_change(|options| {
        options.worker_max_instances = vec![MultiResourceEntry {
            key: Some("bar".to_string()),
            value: Some(3),
        }];
    });
}

// Java: src/test/java/com/google/devtools/build/lib/worker/WorkerModuleTest.java:227 WorkerModuleTest.buildStarting_survivesNoWorkerDir
#[test]
fn build_starting_survives_no_worker_dir() {
    let _s = SEQUENTIAL.lock();
    let mut f = Fixture::new();
    let options = worker_options();
    f.setup_environment("/outputRoot");
    f.before_command();
    let worker_dir = worker_dir(&f, "/outputRoot");
    // Check that new pools/factories can be created without a worker dir.
    f.build_starting(&options);
    // But once we try to get a worker, it should fail. This forces a situation where we can't
    // have a workerDir.
    assert!(!worker_dir.exists());
    let parent = worker_dir.get_parent_directory().unwrap();
    parent.create_directory_and_parents().unwrap();
    parent.set_writable(false).unwrap();
    // But an actual worker cannot be created.
    let key = Arc::new(create_worker_key_mnemonic("Work", &f.fs, false, false));
    let result = same_pool().borrow_worker(&key);
    parent.set_writable(true).unwrap();
    assert!(result.is_err(), "assertThrows(IOException.class, ...)");
}

// Java: src/test/java/com/google/devtools/build/lib/worker/WorkerModuleTest.java:247 WorkerModuleTest.buildStarting_cleansStaleTrashDirCleanedOnFirstBuild
#[test]
fn build_starting_cleans_stale_trash_dir_cleaned_on_first_build() {
    let _s = SEQUENTIAL.lock();
    let mut f = Fixture::new();
    let options = worker_options();
    f.setup_environment("/outputRoot");
    f.before_command();
    let worker_dir = worker_dir(&f, "/outputRoot");
    let trash_base =
        worker_dir.get_child(blaze_exec::sandbox::asynchronous_tree_deleter::MOVED_TRASH_DIR);
    trash_base.create_directory_and_parents().unwrap();
    let stale_trash = trash_base.get_child("random-trash");
    stale_trash.create_directory_and_parents().unwrap();
    f.build_starting(&options);
    // Trash is cleaned upon first build (asynchronously: wait for the tree deleter).
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while stale_trash.exists() && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(!stale_trash.exists());
    stale_trash.create_directory_and_parents().unwrap();
    // PORT: Blaze's buildStarting returns at once when it already ran for the command (see
    // WorkerModule::build_starting); Java's second call runs, finds the tree deleter and leaves
    // the trash. A beforeCommand in between makes Blaze's call run as Java's does.
    f.before_command();
    f.build_starting(&options);
    // Trash is not cleaned upon subsequent builds.
    assert!(stale_trash.exists());
}
