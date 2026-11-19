//! Port of `lib/worker/WorkerModule.java` and `lib/worker/WorkerSpawnStrategy.java`.
//!
//! Java's `WorkerModule` is one object per server: its worker pool, factory and tree deleter
//! outlive commands. The state here is process-wide for the same reason. Blaze has no persistent
//! server yet, so a process runs one command and the pool's workers end with it (through the
//! shutdown hooks, see [`shutdown_hooks`](super::shutdown_hooks)); with a server the same code
//! keeps them between commands, as Java does.

use std::sync::{Arc, LazyLock};

use blaze_events::{Event, EventHandler};
use blaze_optdefs::misc::sandbox_options::SandboxOptions;
use blaze_optdefs::misc::worker_options::WorkerOptions;
use blaze_vfs::Path;
use parking_lot::Mutex;

use super::singleplex_worker::WorkerSandboxOptions;
use super::worker_factory::WorkerFactory;
use super::worker_lifecycle_manager::WorkerLifecycleManager;
use super::worker_multiplexer::WorkerMultiplexerManager;
use super::worker_pool::{WorkerPool, WorkerPoolConfig, WorkerPoolImpl};
use super::worker_process_metrics::WorkerProcessMetricsCollector;
use super::worker_spawn_runner::WorkerSpawnRunner;
use crate::exec::abstract_spawn_strategy::AbstractSpawnStrategy;
use crate::exec::exec_command_environment::ExecCommandEnvironment;
use crate::exec::local::local_env_provider;
use crate::exec::runfiles_tree_updater::RunfilesTreeUpdater;
use crate::exec::spawn_strategy_registry::SpawnStrategyRegistryBuilder;
use crate::exec::tree_deleter::TreeDeleter;
use crate::sandbox::asynchronous_tree_deleter::{AsynchronousTreeDeleter, MOVED_TRASH_DIR};

// Java: com/google/devtools/build/lib/worker/WorkerModule.java:51 WorkerModule.STALE_TRASH
const STALE_TRASH: &str = "_stale_trash";

/// The module's fields.
#[derive(Default)]
struct WorkerModuleState {
    /// `env.getReporter()` of the current command (`this.env`).
    reporter: Option<Arc<dyn EventHandler>>,
    /// `env.getOptions().getOptions(WorkerOptions.class)` of the current command (read by
    /// `buildComplete`).
    options: Option<Arc<WorkerOptions>>,
    worker_factory: Option<Arc<WorkerFactory>>,
    tree_deleter: Option<Arc<AsynchronousTreeDeleter>>,
    config: Option<WorkerPoolConfig>,
    worker_pool: Option<Arc<dyn WorkerPool>>,
    worker_lifecycle_manager: Option<Arc<WorkerLifecycleManager>>,
    /// Whether `buildStarting` ran for the current command.
    build_started: bool,
}

static MODULE: LazyLock<Mutex<WorkerModuleState>> =
    LazyLock::new(|| Mutex::new(WorkerModuleState::default()));

/// What `buildStarting` reads from the command and the `BuildStartingEvent`.
pub struct WorkerBuildStarting<'a> {
    /// `event.request().getOptions(WorkerOptions.class)`.
    pub worker_options: Arc<WorkerOptions>,
    /// `event.request().getOptions(SandboxOptions.class)`.
    pub sandbox_options: Option<&'a SandboxOptions>,
    /// `env.getOutputBase()`.
    pub output_base: &'a Path,
    /// `env.getReporter()`.
    pub reporter: Arc<dyn EventHandler>,
}

/// A module that adds the WorkerActionContextProvider to the available action context
/// providers.
// Java: com/google/devtools/build/lib/worker/WorkerModule.java:49 WorkerModule
pub struct WorkerModule;

impl WorkerModule {
    // Java: com/google/devtools/build/lib/worker/WorkerModule.java:67 WorkerModule.beforeCommand
    pub fn before_command(
        reporter: Arc<dyn EventHandler>,
        worker_options: Option<Arc<WorkerOptions>>,
    ) {
        {
            let mut st = MODULE.lock();
            st.reporter = Some(reporter.clone());
            st.options = worker_options;
            st.build_started = false;
        }
        WorkerProcessMetricsCollector::instance().before_command();
        WorkerMultiplexerManager::before_command(Some(reporter));
    }

    // Java: com/google/devtools/build/lib/worker/WorkerModule.java:75 WorkerModule.cleanStarting
    pub fn clean_starting(options: &WorkerOptions) {
        let mut st = MODULE.lock();
        if st.worker_pool.is_some() {
            let reporter = st.reporter.clone();
            if let Some(f) = &st.worker_factory {
                f.set_reporter(if options.worker_verbose {
                    reporter
                } else {
                    None
                });
            }
            WorkerModule::shutdown_pool(
                &mut st,
                "Clean command is running, shutting down worker pool...",
                /* always_log= */ false,
                options.worker_verbose,
            );
        }
    }

    /// Handles updating worker factories and pools when a build starts. If either the workerDir
    /// or the sandboxing flag has changed, we need to recreate the factory, and we clear out logs
    /// at the same time. If options affecting the pools have changed, we just change the pools.
    // Java: com/google/devtools/build/lib/worker/WorkerModule.java:92 WorkerModule.buildStarting
    pub fn build_starting(event: WorkerBuildStarting<'_>) {
        let mut st = MODULE.lock();
        st.build_started = true;
        st.reporter = Some(event.reporter.clone());
        st.options = Some(event.worker_options.clone());
        let options = event.worker_options;
        let env_reporter = event.reporter;
        if let Some(f) = &st.worker_factory {
            f.set_reporter(if options.worker_verbose {
                Some(env_reporter.clone())
            } else {
                None
            });
        }
        // env.getRuntime().getProductName() + "-workers"
        let worker_dir = event.output_base.get_relative_str("bazel-workers");
        let worker_sandbox_options: Option<WorkerSandboxOptions> = if options.sandbox_hardening {
            // LinuxSandboxUtil.getLinuxSandbox(workspace) and the sandbox options; the hardened
            // sandbox needs cgroups (see SandboxedWorker).
            blaze_util::unported!(
                "WorkerModule.buildStarting (--experimental_worker_sandbox_hardening)"
            )
        } else {
            None
        };
        let trash_base = worker_dir.get_relative_str(MOVED_TRASH_DIR);
        if st.tree_deleter.is_none() {
            let tree_deleter = Arc::new(AsynchronousTreeDeleter::new(trash_base.clone()));
            st.tree_deleter = Some(tree_deleter.clone());
            if trash_base.exists() {
                WorkerModule::remove_stale_trash(
                    &*env_reporter,
                    &worker_dir,
                    &trash_base,
                    &*tree_deleter,
                );
            }
        }
        // VirtualCgroupFactory("worker_", VirtualCgroup.getInstance(), sandboxHardening ? limits :
        // {}, useCgroupsOnLinux) on Linux with --incompatible_use_new_cgroup_implementation (the
        // default).
        // PORT: as in LinuxSandboxedSpawnRunner, VirtualCgroup.getInstance() (the per-server
        // `blaze_<pid>_spawns.slice`) is not created. Without hardening and without
        // --experimental_worker_use_cgroups_on_linux (both fail loudly here) the factory has no
        // limits and alwaysCreate is false, so VirtualCgroupFactory.create returns
        // VirtualCgroup.NULL for every worker: no cgroup is made, NULL.exists() is false, and the
        // metrics collector reads cgroups only with useCgroupsOnLinux. A null factory behaves the
        // same.
        let _ = event
            .sandbox_options
            .map(|o| o.use_new_cgroup_implementation);

        let new_worker_factory = Arc::new(WorkerFactory::with_options(
            worker_dir.clone(),
            options.clone(),
            worker_sandbox_options,
            st.tree_deleter.clone(),
        ));
        let factory_changed = match &st.worker_factory {
            None => true,
            Some(f) => **f != *new_worker_factory,
        };
        if factory_changed {
            if worker_dir.exists() {
                // Clean out old log files.
                match worker_dir.get_directory_entries() {
                    Ok(entries) => {
                        for log_file in entries {
                            if log_file.get_base_name().ends_with(".log")
                                && let Err(e) = log_file.delete()
                            {
                                env_reporter.handle(&Event::warn(&format!(
                                    "Could not delete old worker log '{}': {}",
                                    log_file.get_path_string(),
                                    e
                                )));
                            }
                        }
                    }
                    Err(e) => {
                        env_reporter.handle(&Event::warn(&format!(
                            "Could not delete old worker logs in '{}': {}",
                            worker_dir.get_path_string(),
                            e
                        )));
                    }
                }
            }

            WorkerModule::shutdown_pool(
                &mut st,
                "Worker factory configuration has changed, restarting worker pool...",
                /* always_log= */ true,
                options.worker_verbose,
            );
            new_worker_factory.set_reporter(if options.worker_verbose {
                Some(env_reporter.clone())
            } else {
                None
            });
            st.worker_factory = Some(new_worker_factory);
        }

        let new_config = WorkerPoolConfig::new(
            options.worker_max_instances.clone(),
            options.worker_max_multiplex_instances.clone(),
        );

        // If the config changed compared to the last run, we have to create a new pool.
        if st.config.as_ref() != Some(&new_config) {
            WorkerModule::shutdown_pool(
                &mut st,
                "Worker pool configuration has changed, restarting worker pool...",
                /* always_log= */ true,
                options.worker_verbose,
            );
        }

        if st.worker_pool.is_none() {
            let factory = st.worker_factory.clone().expect("workerFactory");
            st.worker_pool = Some(Arc::new(WorkerPoolImpl::new(factory, &new_config)));
            st.config = Some(new_config);
            // If workerPool is restarted then we should recreate metrics.
            WorkerProcessMetricsCollector::instance().clear();
        }

        // Override the flag value if we can't actually use cgroups so that we at least fallback
        // to ps.
        if options.use_cgroups_on_linux {
            // CgroupsInfo.isSupported() / VirtualCgroup.getInstance().memory().
            blaze_util::unported!(
                "CgroupsInfo.isSupported (--experimental_worker_use_cgroups_on_linux)"
            );
        }
        WorkerProcessMetricsCollector::instance().set_use_cgroups_on_linux(false);

        // Start collecting after a pool is defined
        let manager = Arc::new(WorkerLifecycleManager::new(
            st.worker_pool.clone().unwrap(),
            &options,
            Some(env_reporter.clone()),
        ));
        manager.start();
        st.worker_lifecycle_manager = Some(manager);

        // Reset the pool at the beginning of each build.
        st.worker_pool.as_ref().unwrap().reset();
    }

    // Java: com/google/devtools/build/lib/worker/WorkerModule.java:200 WorkerModule.removeStaleTrash
    fn remove_stale_trash(
        reporter: &dyn EventHandler,
        worker_dir: &Path,
        trash_base: &Path,
        tree_deleter: &AsynchronousTreeDeleter,
    ) {
        let result: std::io::Result<()> = (|| {
            // The AsynchronousTreeDeleter relies on a counter for naming directories that will be
            // moved out of the way before being deleted asynchronously. If there is trash on disk
            // from a previous bazel server instance, the dirs will have names not synced with the
            // counter, therefore we may run the risk of moving a directory in this server instance
            // to a path of an existing directory. To solve this we rename the trash directory that
            // was on disk, create a new empty trash directory and delete the old trash via the
            // AsynchronousTreeDeleter. Before deletion the stale trash will be moved to a
            // directory named `0` under MOVED_TRASH_DIR.
            let stale_trash = trash_base
                .get_parent_directory()
                .expect("trash base has a parent")
                .get_child(STALE_TRASH);
            trash_base.rename_to(&stale_trash)?;
            trash_base.create_directory()?;
            tree_deleter.delete_tree(&stale_trash)
        })();
        if let Err(e) = result {
            reporter.handle(&Event::error(&format!(
                "Could not trash dir in '{}': {}",
                worker_dir.get_path_string(),
                e
            )));
        }
    }

    /// `registerSpawnStrategies(registryBuilder, env)`.
    ///
    /// PORT: a command that did not post the `BuildStartingEvent` to this module (the execution
    /// replay harnesses) has no pool; `buildStarting` runs here then, with the command's options,
    /// as it would have before execution.
    // Java: com/google/devtools/build/lib/worker/WorkerModule.java:221 WorkerModule.registerSpawnStrategies
    pub fn register_spawn_strategies(
        registry_builder: &mut SpawnStrategyRegistryBuilder,
        env: &ExecCommandEnvironment,
    ) {
        let needs_start = {
            let st = MODULE.lock();
            st.worker_pool.is_none() || !st.build_started
        };
        if needs_start {
            WorkerModule::build_starting(WorkerBuildStarting {
                worker_options: env.worker_options.clone(),
                sandbox_options: Some(&env.sandbox_options),
                output_base: &env.output_base,
                reporter: env.reporter.clone(),
            });
        }
        let worker_pool = MODULE
            .lock()
            .worker_pool
            .clone()
            .expect("NullPointerException: workerPool");
        let local_env_provider = Arc::new(local_env_provider::for_current_os(&env.client_env));
        let spawn_runner = WorkerSpawnRunner::new(
            env.exec_root.clone(),
            worker_pool,
            env.reporter.clone(),
            local_env_provider,
            env.bin_tools.clone(),
            env.resource_manager.clone(),
            Arc::new(RunfilesTreeUpdater::for_command_environment(env)),
            env.worker_options.clone(),
            WorkerProcessMetricsCollector::instance(),
        );
        // Java: com/google/devtools/build/lib/worker/WorkerSpawnStrategy.java:30 WorkerSpawnStrategy.toString
        registry_builder.register_strategy(
            Arc::new(
                AbstractSpawnStrategy::new(
                    env.exec_root.clone(),
                    Arc::new(spawn_runner),
                    env.execution_options.clone(),
                    "worker",
                )
                .with_simple_class_name("WorkerSpawnStrategy"),
            ),
            &["worker"],
        );
    }

    // Java: com/google/devtools/build/lib/worker/WorkerModule.java:246 WorkerModule.buildComplete
    pub fn build_complete() {
        let mut st = MODULE.lock();
        if let Some(options) = st.options.clone()
            && options.worker_quit_after_build
        {
            WorkerModule::shutdown_pool(
                &mut st,
                "Build completed, shutting down worker pool...",
                /* always_log= */ false,
                options.worker_verbose,
            );
        }
        if let Some(manager) = st.worker_lifecycle_manager.take() {
            manager.stop_processing();
            manager.interrupt();
        }
        drop(st);
        WorkerProcessMetricsCollector::instance().clear_killed_worker_process_metrics();
    }

    /// Shuts down the worker pool and sets `workerPool` to null.
    // Java: com/google/devtools/build/lib/worker/WorkerModule.java:264 WorkerModule.shutdownPool
    fn shutdown_pool(
        st: &mut WorkerModuleState,
        reason: &str,
        always_log: bool,
        worker_verbose: bool,
    ) {
        blaze_util::check_argument!(!reason.is_empty());

        if let Some(pool) = st.worker_pool.take() {
            if (worker_verbose || always_log)
                && let Some(reporter) = &st.reporter
            {
                reporter.handle(&Event::info(reason));
            }
            pool.close();
        }
    }

    // Java: com/google/devtools/build/lib/worker/WorkerModule.java:278 WorkerModule.afterCommand
    pub fn after_command() {
        {
            let mut st = MODULE.lock();
            st.reporter = None;
            st.options = None;
            st.build_started = false;
            if let Some(f) = &st.worker_factory {
                f.set_reporter(None);
            }
        }
        WorkerMultiplexerManager::after_command();
    }

    // Java: com/google/devtools/build/lib/worker/WorkerModule.java:288 WorkerModule.getWorkerPoolConfig
    pub fn get_worker_pool_config() -> Option<WorkerPoolConfig> {
        MODULE.lock().config.clone()
    }

    /// `workerPool` (`@VisibleForTesting`).
    pub fn get_worker_pool() -> Option<Arc<dyn WorkerPool>> {
        MODULE.lock().worker_pool.clone()
    }
}
