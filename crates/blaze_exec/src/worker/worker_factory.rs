//! Port of `lib/worker/WorkerFactory.java`.

use std::collections::BTreeSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicI32, Ordering};

use blaze_events::{Event, EventHandler};
use blaze_optdefs::misc::worker_options::WorkerOptions;
use blaze_vfs::{Path, PathFragment};
use parking_lot::Mutex;

use super::error_message::ErrorMessage;
use super::singleplex_worker::{SingleplexWorker, WorkerSandboxOptions};
use super::worker::Worker;
use super::worker_key::WorkerKey;
use super::worker_multiplexer::WorkerMultiplexerManager;
use super::worker_process_status::{Status, worker_code_name};
use super::worker_proxy::WorkerProxy;
use crate::exec::tree_deleter::TreeDeleter;
use crate::sandbox::asynchronous_tree_deleter::AsynchronousTreeDeleter;
use crate::sandbox::cgroups::VirtualCgroupFactory;

// It's fine to use an AtomicInteger here (which is 32-bit), because it is only incremented when
// spawning a new worker, thus even under worst-case circumstances and buggy workers quitting
// after each action, this should never overflow.
// This starts at 1 to avoid hiding latent problems of multiplex workers not returning a
// request_id (which is indistinguishable from 0 in proto3).
// Java: com/google/devtools/build/lib/worker/WorkerFactory.java:43 WorkerFactory.pidCounter
static PID_COUNTER: AtomicI32 = AtomicI32::new(1);

/// `WorkerLoggingHelper.LogLevel`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LogLevel {
    Info,
    Warning,
}

/// This class simultaneously sends messages to a logger and an event reporter.
// Java: com/google/devtools/build/lib/worker/WorkerFactory.java:270 WorkerFactory.WorkerLoggingHelper.logMessage
fn log_message(reporter: Option<&Arc<dyn EventHandler>>, level: LogLevel, message: &str) {
    // PORT: skipped, tracing (the GoogleLogger half).
    match level {
        LogLevel::Info => {
            if let Some(reporter) = reporter {
                reporter.handle(&Event::info(message));
            }
        }
        LogLevel::Warning => {
            if let Some(reporter) = reporter {
                reporter.handle(&Event::warn(message));
            }
        }
    }
}

/// The methods of `WorkerFactory` that `WorkerPoolImpl` calls.
///
/// PORT: Java's pool holds the `WorkerFactory` class, which its tests replace by a Mockito spy;
/// the pool holds this trait so that the tests can do the same.
// Java: com/google/devtools/build/lib/worker/WorkerFactory.java:36 WorkerFactory
pub trait WorkerFactoryApi: Send + Sync {
    /// `create(key)`.
    fn create(&self, key: &Arc<WorkerKey>) -> std::io::Result<Arc<dyn Worker>>;
    /// `destroyWorker(key, worker)`.
    fn destroy_worker(&self, key: &WorkerKey, worker: &Arc<dyn Worker>);
    /// `validateWorker(key, worker)`.
    fn validate_worker(&self, key: &WorkerKey, worker: &Arc<dyn Worker>) -> bool;
}

impl WorkerFactoryApi for WorkerFactory {
    fn create(&self, key: &Arc<WorkerKey>) -> std::io::Result<Arc<dyn Worker>> {
        WorkerFactory::create(self, key)
    }

    fn destroy_worker(&self, key: &WorkerKey, worker: &Arc<dyn Worker>) {
        WorkerFactory::destroy_worker(self, key, worker)
    }

    fn validate_worker(&self, key: &WorkerKey, worker: &Arc<dyn Worker>) -> bool {
        WorkerFactory::validate_worker(self, key, worker)
    }
}

/// Factory used by the pool to create / destroy / validate worker processes.
// Java: com/google/devtools/build/lib/worker/WorkerFactory.java:36 WorkerFactory
pub struct WorkerFactory {
    worker_options: Arc<WorkerOptions>,
    worker_base_dir: Path,
    tree_deleter: Option<Arc<AsynchronousTreeDeleter>>,
    reporter: Mutex<Option<Arc<dyn EventHandler>>>,
    /// Options specific to hardened sandbox. Null if `--experimental_worker_sandbox_hardening` is
    /// not set.
    hardened_sandbox_options: Option<WorkerSandboxOptions>,
    /// `cgroupFactory`: the factory of the workers' cgroups, or null.
    cgroup_factory: Option<Arc<VirtualCgroupFactory>>,
}

impl WorkerFactory {
    // Java: com/google/devtools/build/lib/worker/WorkerFactory.java:59 WorkerFactory.<init>
    pub fn new(worker_base_dir: Path, worker_options: Arc<WorkerOptions>) -> WorkerFactory {
        WorkerFactory::with_options(worker_base_dir, worker_options, None, None)
    }

    /// `new WorkerFactory(workerBaseDir, workerOptions, hardenedSandboxOptions, treeDeleter,
    /// null)`; the cgroup factory is given with [`WorkerFactory::with_cgroup_factory`].
    // Java: com/google/devtools/build/lib/worker/WorkerFactory.java:63 WorkerFactory.<init>
    pub fn with_options(
        worker_base_dir: Path,
        worker_options: Arc<WorkerOptions>,
        hardened_sandbox_options: Option<WorkerSandboxOptions>,
        tree_deleter: Option<Arc<AsynchronousTreeDeleter>>,
    ) -> WorkerFactory {
        WorkerFactory {
            worker_options,
            worker_base_dir,
            tree_deleter,
            reporter: Mutex::new(None),
            hardened_sandbox_options,
            cgroup_factory: None,
        }
    }

    /// The `cgroupFactory` constructor argument (not part of `equals`, as in Java).
    pub fn with_cgroup_factory(
        mut self,
        cgroup_factory: Option<Arc<VirtualCgroupFactory>>,
    ) -> Self {
        self.cgroup_factory = cgroup_factory;
        self
    }

    // Java: com/google/devtools/build/lib/worker/WorkerFactory.java:76 WorkerFactory.setReporter
    pub fn set_reporter(&self, reporter: Option<Arc<dyn EventHandler>>) {
        *self.reporter.lock() = reporter;
    }

    fn reporter(&self) -> Option<Arc<dyn EventHandler>> {
        self.reporter.lock().clone()
    }

    /// `treeDeleter`, null without one (the workers fail when they use it).
    fn tree_deleter(&self) -> Option<Arc<dyn TreeDeleter>> {
        self.tree_deleter
            .as_ref()
            .map(|t| t.clone() as Arc<dyn TreeDeleter>)
    }

    // Java: com/google/devtools/build/lib/worker/WorkerFactory.java:80 WorkerFactory.create
    pub fn create(&self, key: &Arc<WorkerKey>) -> std::io::Result<Arc<dyn Worker>> {
        let worker_id = PID_COUNTER.fetch_add(1, Ordering::SeqCst);
        let work_type_name = key.get_worker_type_name();
        if !self.worker_base_dir.is_directory() {
            self.worker_base_dir.create_directory_and_parents()?;
            if let Some(tree_deleter) = &self.tree_deleter {
                tree_deleter.get_trash_base().create_directory()?;
            }
        }
        let log_file = self.worker_base_dir.get_relative_str(&format!(
            "{}-{}-{}.log",
            work_type_name,
            worker_id,
            key.get_mnemonic()
        ));

        let worker: Arc<dyn Worker> = if key.is_sandboxed() {
            if key.is_multiplex() {
                let worker_multiplexer = WorkerMultiplexerManager::get_instance(key, &log_file);
                let multiplexer_id = worker_multiplexer.get_multiplexer_id();
                let work_dir = self.get_multiplex_sandboxed_worker_path(key, multiplexer_id);
                let proxy = WorkerProxy::new_sandboxed(
                    key.clone(),
                    worker_id,
                    worker_multiplexer.get_log_file().clone(),
                    worker_multiplexer.clone(),
                    work_dir.clone(),
                    self.tree_deleter(),
                );
                worker_multiplexer.set_work_dir(work_dir);
                Arc::new(proxy)
            } else {
                let work_dir = self.get_sandboxed_worker_path(key, worker_id);
                Arc::new(
                    SingleplexWorker::new_sandboxed(
                        key.clone(),
                        worker_id,
                        work_dir,
                        log_file,
                        self.worker_options.clone(),
                        self.hardened_sandbox_options.as_ref(),
                        self.tree_deleter(),
                        key.use_in_memory_tracking(),
                    )
                    .with_cgroup_factory(self.cgroup_factory.clone()),
                )
            }
        } else if key.is_multiplex() {
            let worker_multiplexer = WorkerMultiplexerManager::get_instance(key, &log_file);
            Arc::new(WorkerProxy::new(
                key.clone(),
                worker_id,
                worker_multiplexer.get_log_file().clone(),
                worker_multiplexer,
                key.get_exec_root().clone(),
            ))
        } else {
            Arc::new(
                SingleplexWorker::new(
                    key.clone(),
                    worker_id,
                    key.get_exec_root().clone(),
                    log_file,
                    self.worker_options.clone(),
                )
                .with_cgroup_factory(self.cgroup_factory.clone()),
            )
        };

        let msg = format!(
            "Created new {} {} {} {} (id {}, key hash {}), logging to {}",
            if key.is_sandboxed() {
                "sandboxed"
            } else {
                "non-sandboxed"
            },
            if key.is_multiplex() {
                "multiplex"
            } else {
                "singleplex"
            },
            key.get_mnemonic(),
            work_type_name,
            worker_id,
            key.hash_code(),
            worker.get_log_file()
        );
        log_message(self.reporter().as_ref(), LogLevel::Info, &msg);
        Ok(worker)
    }

    // Java: com/google/devtools/build/lib/worker/WorkerFactory.java:148 WorkerFactory.getSandboxedWorkerPath
    pub fn get_sandboxed_worker_path(&self, key: &WorkerKey, worker_id: i32) -> Path {
        let workspace_name = key.get_exec_root().get_base_name().to_string();
        self.worker_base_dir
            .get_relative_str(&format!(
                "{}-{}-{}",
                key.get_worker_type_name(),
                worker_id,
                key.get_mnemonic()
            ))
            .get_relative_str(&workspace_name)
    }

    // Java: com/google/devtools/build/lib/worker/WorkerFactory.java:155 WorkerFactory.getMultiplexSandboxedWorkerPath
    pub fn get_multiplex_sandboxed_worker_path(
        &self,
        key: &WorkerKey,
        multiplexer_id: i32,
    ) -> Path {
        let workspace_name = key.get_exec_root().get_base_name().to_string();
        self.worker_base_dir
            .get_relative_str(&format!(
                "{}-{}-{}-workdir",
                key.get_mnemonic(),
                key.get_worker_type_name(),
                multiplexer_id
            ))
            .get_relative_str(&workspace_name)
    }

    // Java: com/google/devtools/build/lib/worker/WorkerFactory.java:163 WorkerFactory.destroyWorker
    pub fn destroy_worker(&self, key: &WorkerKey, worker: &Arc<dyn Worker>) {
        let worker_id = worker.get_worker_id();
        let mut worker_failure_code = String::new();
        if let Some(code) = worker.get_status().get_worker_code() {
            worker_failure_code = format!("(code: {})", worker_code_name(code));
        }
        let msg = format!(
            "Destroying {} {} (id {}, key hash {}) with cause: {} {}\n",
            key.get_mnemonic(),
            key.get_worker_type_name(),
            worker_id,
            key.hash_code(),
            worker.get_status().get(),
            worker_failure_code
        );
        log_message(self.reporter().as_ref(), LogLevel::Info, &msg);
        worker.destroy();
    }

    // Java: com/google/devtools/build/lib/worker/WorkerFactory.java:183 WorkerFactory.validateWorker
    pub fn validate_worker(&self, key: &WorkerKey, worker: &Arc<dyn Worker>) -> bool {
        // Status is invalid if the status is either killed or pending killed.
        if !worker.get_status().is_valid() {
            return false;
        }
        if let Some(exit_value) = worker.get_exit_value() {
            // At this point, the worker factory has no idea what caused the process to be killed -
            // so we set the status to be KILLED_UNKNOWN.
            worker
                .get_status()
                .maybe_update_status(Status::KilledUnknown);
            if worker.died_unexpectedly() {
                let msg = format!(
                    "{} {} (id {}) has unexpectedly died with exit code {}.",
                    key.get_mnemonic(),
                    key.get_worker_type_name(),
                    worker.get_worker_id(),
                    exit_value
                );
                let error_message = ErrorMessage::builder()
                    .message(&msg)
                    .log_file(worker.get_log_file())
                    .log_size_limit(4096)
                    .build();
                log_message(
                    self.reporter().as_ref(),
                    LogLevel::Warning,
                    &error_message.to_string(),
                );
            }
            return false;
        }
        let files_changed =
            key.get_worker_files_combined_hash() != worker.get_worker_files_combined_hash();

        if files_changed {
            let mut msg = format!(
                "{} {} (id {}) can no longer be used, because its files have changed on disk:",
                key.get_mnemonic(),
                key.get_worker_type_name(),
                worker.get_worker_id()
            );
            let mut files: BTreeSet<PathFragment> = BTreeSet::new();
            files.extend(key.get_worker_files_with_digests().keys().cloned());
            files.extend(worker.get_worker_files_with_digests().keys().cloned());
            for file in &files {
                let old_digest = worker.get_worker_files_with_digests().get(file);
                let new_digest = key.get_worker_files_with_digests().get(file);
                if old_digest != new_digest {
                    msg.push('\n');
                    msg.push_str(file.get_path_string());
                    msg.push_str(": ");
                    msg.push_str(&hex_string_for_debugging(old_digest));
                    msg.push_str(" -> ");
                    msg.push_str(&hex_string_for_debugging(new_digest));
                }
            }

            log_message(self.reporter().as_ref(), LogLevel::Warning, &msg);
        }

        !files_changed
    }
}

// Java: com/google/devtools/build/lib/worker/WorkerFactory.java:233 WorkerFactory.hexStringForDebugging
fn hex_string_for_debugging(bytes: Option<&Vec<u8>>) -> String {
    match bytes {
        Some(b) => hex::encode(b),
        None => "<none>".to_string(),
    }
}

impl PartialEq for WorkerFactory {
    // Java: com/google/devtools/build/lib/worker/WorkerFactory.java:238 WorkerFactory.equals
    fn eq(&self, that: &WorkerFactory) -> bool {
        self.worker_base_dir == that.worker_base_dir
            && self.worker_options.use_cgroups_on_linux == that.worker_options.use_cgroups_on_linux
            && self.hardened_sandbox_options == that.hardened_sandbox_options
    }
}
