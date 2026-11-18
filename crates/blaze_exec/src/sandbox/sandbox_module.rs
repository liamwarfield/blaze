//! Port of `lib/sandbox/SandboxModule.java`: the module that provides the sandboxed spawn
//! strategies.

use std::io;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use blaze_events::Event;
use blaze_optdefs::misc::sandbox_options::SandboxOptions;
use blaze_options::TriState;
use blaze_protos::failure_details::{FailureDetail, failure_detail};
use blaze_shell::{JavaFile, SubprocessBuilder, SubprocessBuilderStreamAction};
use blaze_util::Fingerprint;
use blaze_vfs::{FileSystemExt, Path, PathFragment};
use indexmap::IndexMap;

use super::asynchronous_tree_deleter::{AsynchronousTreeDeleter, MOVED_TRASH_DIR};
use super::linux_sandboxed_spawn_runner::LinuxSandboxedSpawnRunner;
use super::linux_sandboxed_strategy::LinuxSandboxedStrategy;
use super::process_wrapper_sandboxed_spawn_runner::ProcessWrapperSandboxedSpawnRunner;
use super::process_wrapper_sandboxed_strategy::ProcessWrapperSandboxedStrategy;
use super::sandbox_helpers::{INACCESSIBLE_HELPER_DIR, INACCESSIBLE_HELPER_FILE};
use super::sandbox_stash::{self, SandboxStash};
use super::synchronous_tree_deleter::SynchronousTreeDeleter;
use crate::exec::exec_command_environment::ExecCommandEnvironment;
use crate::exec::spawn_runner::SpawnRunner;
use crate::exec::spawn_strategy_registry::SpawnStrategyRegistryBuilder;
use crate::exec::tree_deleter::TreeDeleter;

// Java: com/google/devtools/build/lib/sandbox/SandboxModule.java:60 SandboxModule.MAC_INDEX_FILE
const MAC_INDEX_FILE: &str = ".DS_Store";

// Java: com/google/devtools/build/lib/sandbox/SandboxModule.java:62 SandboxModule.SANDBOX_BASE_PERSISTENT_DIRS
const SANDBOX_BASE_PERSISTENT_DIRS: [&str; 4] = [
    MAC_INDEX_FILE,
    sandbox_stash::SANDBOX_STASH_BASE,
    sandbox_stash::TEMPORARY_SANDBOX_STASH_BASE,
    MOVED_TRASH_DIR,
];

/// `BlazeRuntime.getProductName()`.
// PORT: Blaze reports Bazel's product name (docs/decisions.md, version impersonation); the
// command environment seam does not carry it.
const PRODUCT_NAME: &str = "bazel";

/// Tracks whether we are issuing the very first build within this Bazel server instance.
// Java: com/google/devtools/build/lib/sandbox/SandboxModule.java:72 SandboxModule.firstBuild
static FIRST_BUILD: AtomicBool = AtomicBool::new(true);

/// The tree deleter in use, and whether it is the asynchronous one (Java `instanceof` checks).
#[derive(Clone)]
enum TreeDeleterKind {
    Synchronous(Arc<SynchronousTreeDeleter>),
    Asynchronous(Arc<AsynchronousTreeDeleter>),
}

impl TreeDeleterKind {
    fn as_tree_deleter(&self) -> Arc<dyn TreeDeleter> {
        match self {
            TreeDeleterKind::Synchronous(d) => d.clone(),
            TreeDeleterKind::Asynchronous(d) => d.clone(),
        }
    }
}

/// This module provides the Sandbox spawn strategy.
// Java: com/google/devtools/build/lib/sandbox/SandboxModule.java:58 SandboxModule
pub struct SandboxModule {
    /// Environment for the running command.
    env: Option<ExecCommandEnvironment>,
    /// Path to the location of the sandboxes.
    sandbox_base: Option<Path>,
    /// Collection of spawn runner instantiated during the executor setup.
    ///
    /// We need this information to clean up the heavy subdirectories of the sandbox base on build
    /// completion but to avoid wiping the whole sandbox base itself, which could be problematic
    /// across builds.
    spawn_runners: Vec<Arc<dyn SpawnRunner>>,
    /// Handler to process expensive tree deletions, potentially outside of the critical path.
    ///
    /// Sandboxing creates one separate tree for each action, and this tree is used to run the
    /// action commands in. These trees are disjoint for all actions and have unique identifiers.
    /// Therefore, there is no need for their deletion (which can be very expensive) to happen in
    /// the critical path -- so if the user so wishes, we process those deletions asynchronously.
    tree_deleter: Option<TreeDeleterKind>,
    /// Whether to remove the sandbox worker directories after a build or not. Useful for
    /// debugging to inspect the state of files on failures.
    should_cleanup_sandbox_base: bool,
}

impl SandboxModule {
    pub fn new() -> SandboxModule {
        SandboxModule {
            env: None,
            sandbox_base: None,
            spawn_runners: Vec::new(),
            tree_deleter: None,
            should_cleanup_sandbox_base: false,
        }
    }

    /// Computes the path to the sandbox base tree for the given running command.
    // Java: com/google/devtools/build/lib/sandbox/SandboxModule.java:113 SandboxModule.computeSandboxBase
    pub fn compute_sandbox_base(
        options: &SandboxOptions,
        env: &ExecCommandEnvironment,
    ) -> io::Result<Path> {
        if options.sandbox_base.is_empty() {
            Ok(env.output_base.get_relative_str("sandbox"))
        } else {
            let dir_name = format!(
                "{}-sandbox.{}",
                PRODUCT_NAME,
                Fingerprint::get_hex_digest(env.output_base.get_path_string())
            );
            let file_system = &env.file_system;
            // PORT: skipped, darwin only (no symlink resolution on macOS).
            let resolved_sandbox_base = file_system
                .get_path(&options.sandbox_base)
                .resolve_symbolic_links()?;
            Ok(resolved_sandbox_base.get_relative_str(&dir_name))
        }
    }

    /// `beforeCommand(env)`.
    // Java: com/google/devtools/build/lib/sandbox/SandboxModule.java:134 SandboxModule.beforeCommand
    pub fn before_command(&mut self, env: &ExecCommandEnvironment) {
        // We can't assert that env is null because the Blaze runtime does not guarantee that
        // afterCommand() will be called if the command fails due to, e.g. a syntax error.
        self.env = Some(env.clone());
        // PORT: event bus registration dropped; `clean_starting` is called directly.

        // Don't attempt cleanup unless the executor is initialized.
        self.should_cleanup_sandbox_base = false;
    }

    /// `registerSpawnStrategies(registryBuilder, env)`.
    ///
    /// On error the caller throws Java's `AbruptExitException` with
    /// [`initialization_failure_detail`].
    // Java: com/google/devtools/build/lib/sandbox/SandboxModule.java:145 SandboxModule.registerSpawnStrategies
    pub fn register_spawn_strategies(
        &mut self,
        registry_builder: &mut SpawnStrategyRegistryBuilder,
        env: &ExecCommandEnvironment,
    ) -> io::Result<()> {
        // PORT: Java requires beforeCommand to have set `env` (checkNotNull); callers that do not
        // call `before_command` get the same state here.
        if self.env.is_none() {
            self.before_command(env);
        }
        self.setup(env, registry_builder)
    }

    // Java: com/google/devtools/build/lib/sandbox/SandboxModule.java:174 SandboxModule.shouldUseWindowsSandbox
    fn should_use_windows_sandbox(
        requested: TriState,
        binary: &PathFragment,
        client_env: &IndexMap<String, String>,
    ) -> io::Result<bool> {
        Ok(match requested {
            TriState::AUTO => windows_sandbox_is_available(binary, client_env),
            TriState::NO => false,
            TriState::YES => {
                if !windows_sandbox_is_available(binary, client_env) {
                    return Err(io::Error::other(format!(
                        "windows-sandbox explicitly requested but \"{}\" could not be found or is not valid",
                        binary
                    )));
                }
                true
            }
        })
    }

    // Java: com/google/devtools/build/lib/sandbox/SandboxModule.java:192 SandboxModule.setup
    fn setup(
        &mut self,
        cmd_env: &ExecCommandEnvironment,
        builder: &mut SpawnStrategyRegistryBuilder,
    ) -> io::Result<()> {
        let env = self
            .env
            .clone()
            .expect("env not initialized; was beforeCommand called?");
        let options: Arc<SandboxOptions> = env.sandbox_options.clone();
        let sandbox_base = SandboxModule::compute_sandbox_base(&options, &env)?;
        self.sandbox_base = Some(sandbox_base.clone());
        let trash_base = sandbox_base.get_relative_str(MOVED_TRASH_DIR);

        // Do not remove the sandbox base when --sandbox_debug was specified so that people can
        // check out the contents of the generated sandbox directories.
        self.should_cleanup_sandbox_base = !options.sandbox_debug;

        // If there happens to be any live tree deleter from a previous build and it's different
        // than the one we want now, leave it alone (i.e. don't attempt to wait for pending
        // deletions). Its deletions shouldn't overlap any new directories we create during this
        // build (because the identifiers in the subdirectories will be different).
        if options.async_tree_delete_idle_threads == 0 {
            if !matches!(self.tree_deleter, Some(TreeDeleterKind::Synchronous(_))) {
                self.tree_deleter = Some(TreeDeleterKind::Synchronous(Arc::new(
                    SynchronousTreeDeleter,
                )));
            }
        } else {
            let reuse = matches!(
                &self.tree_deleter,
                Some(TreeDeleterKind::Asynchronous(d)) if d.get_trash_base() == &trash_base
            );
            if !reuse {
                if let Some(d) = &self.tree_deleter {
                    d.as_tree_deleter().shutdown();
                }
                self.tree_deleter = Some(TreeDeleterKind::Asynchronous(Arc::new(
                    AsynchronousTreeDeleter::new(trash_base.clone()),
                )));
                FIRST_BUILD.store(true, Ordering::SeqCst);
            }
        }
        let tree_deleter = self
            .tree_deleter
            .as_ref()
            .expect("set above")
            .as_tree_deleter();
        // PORT: CommandEnvironment.getWorkspaceName() is the exec root's base name.
        SandboxStash::initialize(
            env.exec_root.get_base_name(),
            &sandbox_base,
            &options,
            &*tree_deleter,
        );

        // SpawnExecutionPolicy#getId returns unique base directories for each sandboxed action
        // during the life of a Bazel server instance so we don't need to worry about stale
        // directories from previous builds. However, on the very first build of an instance of
        // the server, we must wipe old contents to avoid reusing stale directories.
        if FIRST_BUILD.load(Ordering::SeqCst) && sandbox_base.exists() {
            let cleaned: io::Result<()> = (|| {
                if trash_base.exists() {
                    // Delete stale trash from a previous server instance.
                    let stale_trash = get_stale_trash_dir(&trash_base);
                    trash_base.rename_to(&stale_trash)?;
                    trash_base.create_directory()?;
                    tree_deleter.delete_tree(&stale_trash)?;
                } else {
                    trash_base.create_directory()?;
                }
                // We can delete other dirs asynchronously (if the flag is on).
                for entry in sandbox_base.get_directory_entries()? {
                    if entry.get_base_name() == MOVED_TRASH_DIR {
                        continue;
                    }
                    if entry.get_base_name() == INACCESSIBLE_HELPER_DIR {
                        entry.delete_tree()?;
                    } else if entry.is_directory() {
                        tree_deleter.delete_tree(&entry)?;
                    } else {
                        entry.delete()?;
                    }
                }
                Ok(())
            })();
            if cleaned.is_err() {
                // We have observed asynchronous deletion failing when running Bazel under Docker,
                // see #21719. Different RUN commands with `bazel build` will write to different
                // layers in the docker image. The overlay filesystem is different and the renaming
                // of the directories that we need to do for asynchronous deletion will fail. When
                // that happens we fall back to synchronous deletion here.
                sandbox_base.delete_tree()?;
            }
        }
        FIRST_BUILD.store(false, Ordering::SeqCst);
        sandbox_base.create_directory_and_parents()?;
        trash_base.create_directory()?;

        let windows_sandbox_path = PathFragment::create(&options.windows_sandbox_path);
        let windows_sandbox_supported = SandboxModule::should_use_windows_sandbox(
            options.use_windows_sandbox,
            &windows_sandbox_path,
            &cmd_env.client_env,
        )?;

        let timeout_kill_delay: Duration = cmd_env
            .local_execution_options
            .get_local_sigkill_grace_seconds();

        let process_wrapper_supported = ProcessWrapperSandboxedSpawnRunner::is_supported(cmd_env);
        let linux_sandbox_supported = LinuxSandboxedSpawnRunner::is_supported(cmd_env);
        // PORT: skipped, darwin only (DarwinSandboxedSpawnRunner.isSupported is false on Linux).
        let darwin_sandbox_supported = false;

        let execution_options = cmd_env.execution_options.clone();
        // This works on most platforms, but isn't the best choice, so we put it first and let
        // later platform-specific sandboxing strategies become the default.
        if process_wrapper_supported {
            let spawn_runner: Arc<dyn SpawnRunner> =
                Arc::new(ProcessWrapperSandboxedSpawnRunner::new(
                    cmd_env,
                    sandbox_base.clone(),
                    tree_deleter.clone(),
                ));
            self.spawn_runners.push(spawn_runner.clone());
            builder.register_strategy(
                Arc::new(ProcessWrapperSandboxedStrategy::new(
                    cmd_env.exec_root.clone(),
                    spawn_runner,
                    execution_options.clone(),
                )),
                &["sandboxed", "processwrapper-sandbox"],
            );
        }

        if options.enable_docker_sandbox {
            // This strategy uses Docker to execute spawns. It should work on all platforms that
            // support Docker.
            blaze_util::unported!(
                "DockerSandboxedSpawnRunner",
                "--experimental_enable_docker_sandbox"
            );
        } else if options.docker_verbose {
            cmd_env.reporter.handle(&Event::info(
                "Docker sandboxing disabled. Use the '--experimental_enable_docker_sandbox' command line option to enable it",
            ));
        }

        // This is the preferred sandboxing strategy on Linux.
        if linux_sandbox_supported {
            let spawn_runner: Arc<dyn SpawnRunner> = Arc::new(LinuxSandboxedStrategy::create(
                cmd_env,
                &sandbox_base,
                timeout_kill_delay,
                tree_deleter.clone(),
                &options,
            ));
            self.spawn_runners.push(spawn_runner.clone());
            builder.register_strategy(
                Arc::new(LinuxSandboxedStrategy::new(
                    cmd_env.exec_root.clone(),
                    spawn_runner,
                    execution_options.clone(),
                )),
                &["sandboxed", "linux-sandbox"],
            );
        }

        // This is the preferred sandboxing strategy on macOS.
        // PORT: skipped, darwin only (DarwinSandboxedSpawnRunner).

        if windows_sandbox_supported {
            blaze_util::unported!("WindowsSandboxedSpawnRunner", "windows only");
        }

        if process_wrapper_supported
            || linux_sandbox_supported
            || darwin_sandbox_supported
            || windows_sandbox_supported
        {
            // This makes the "sandboxed" strategy the default Spawn strategy, unless it is
            // overridden by a later BlazeModule.
            builder.set_default_strategies(&["sandboxed".to_string()]);
        }
        Ok(())
    }

    /// `cleanStarting(CleanStartingEvent)`.
    // Java: com/google/devtools/build/lib/sandbox/SandboxModule.java:389 SandboxModule.cleanStarting
    pub fn clean_starting(&mut self) {
        if let Some(sandbox_base) = &self.sandbox_base {
            let tree_deleter = self.tree_deleter.as_ref().map(|d| d.as_tree_deleter());
            SandboxStash::clean(tree_deleter.as_deref(), sandbox_base);
        }
    }

    /// `afterCommand()`.
    // Java: com/google/devtools/build/lib/sandbox/SandboxModule.java:436 SandboxModule.afterCommand
    pub fn after_command(&mut self) {
        let env = self
            .env
            .clone()
            .expect("env not initialized; was beforeCommand called?");

        let options = &env.sandbox_options;
        let async_tree_delete_threads = options.async_tree_delete_idle_threads;

        // If asynchronous deletions were requested, they may still be ongoing so let them be:
        // trying to delete the base tree synchronously could fail as we can race with those other
        // deletions, and scheduling an asynchronous deletion could race with future builds.
        if async_tree_delete_threads > 0
            && let Some(TreeDeleterKind::Asynchronous(d)) = &self.tree_deleter
        {
            d.set_threads(async_tree_delete_threads);
        }
        // `treeDeleter` might not be an AsynchronousTreeDeleter if the user changed the option but
        // then interrupted the build before the start of the execution phase. But that's OK, there
        // will be nothing new to delete. See #13240.

        if self.should_cleanup_sandbox_base {
            let sandbox_base = self
                .sandbox_base
                .clone()
                .expect("shouldCleanupSandboxBase implies sandboxBase has been set");
            let tree_deleter = self
                .tree_deleter
                .as_ref()
                .map(|d| d.as_tree_deleter())
                .expect("NullPointerException: treeDeleter");
            let cleaned: io::Result<()> = (|| {
                for spawn_runner in &self.spawn_runners {
                    spawn_runner.cleanup_sandbox_base(&sandbox_base, &*tree_deleter)?;
                    sandbox_base.get_child(&spawn_runner.get_name()).delete()?;
                }
                Ok(())
            })();
            if let Err(e) = cleaned {
                env.reporter.handle(&Event::warn(format!(
                    "Failed to delete contents of sandbox {}: {}",
                    sandbox_base,
                    java_io_exception_to_string(&e)
                )));
            }
            self.should_cleanup_sandbox_base = false;

            check_sandbox_base_top_only_contains_persistent_dirs(&sandbox_base);
            // We intentionally keep sandboxBase around, without resetting it to null, in case we
            // have asynchronous deletions going on. In that case, we'd still want to retry this
            // during shutdown.
        }

        self.spawn_runners.clear();

        // PORT: event bus unregistration dropped.
        self.env = None;
    }

    // Java: com/google/devtools/build/lib/sandbox/SandboxModule.java:477 SandboxModule.commonShutdown
    fn common_shutdown(&mut self) {
        // Try to clean up as much garbage as possible, if there happens to be any. This will delay
        // server termination but it's the nice thing to do. If the user gets impatient, they can
        // always kill us again.
        if let Some(d) = self.tree_deleter.take() {
            // Avoid potential reexecution if we crash.
            d.as_tree_deleter().shutdown();
        }

        SandboxStash::shutdown();
    }

    /// `blazeShutdown()`.
    // Java: com/google/devtools/build/lib/sandbox/SandboxModule.java:493 SandboxModule.blazeShutdown
    pub fn blaze_shutdown(&mut self) {
        self.common_shutdown();
    }

    /// `blazeShutdownOnCrash(exitCode)`.
    // Java: com/google/devtools/build/lib/sandbox/SandboxModule.java:498 SandboxModule.blazeShutdownOnCrash
    pub fn blaze_shutdown_on_crash(&mut self) {
        self.common_shutdown();
    }
}

/// The `FailureDetail` of the `AbruptExitException` that `registerSpawnStrategies` throws when
/// `setup` fails: `"Failed to initialize sandbox: <message>"`, `Sandbox.Code.INITIALIZATION_FAILURE`.
// Java: com/google/devtools/build/lib/sandbox/SandboxModule.java:152 SandboxModule.registerSpawnStrategies
pub fn initialization_failure_detail(e: &io::Error) -> FailureDetail {
    FailureDetail {
        message: format!("Failed to initialize sandbox: {}", e),
        category: Some(failure_detail::Category::Sandbox(
            blaze_protos::failure_details::Sandbox {
                code: blaze_protos::failure_details::sandbox::Code::InitializationFailure as i32,
            },
        )),
    }
}

/// `"" + e` for an `IOException`: `java.io.IOException: <message>`.
fn java_io_exception_to_string(e: &io::Error) -> String {
    // PORT: Java prints the exception class (FileNotFoundException, AccessDeniedException, ...);
    // the `io::Error` keeps only the message, so the base class name is used.
    format!("java.io.IOException: {}", e)
}

/// If there is anything other than SANDBOX_BASE_PERSISTENT_DIRS in sandboxBase when we hit this
/// precondition then there is a programming error somewhere (or I made a wrong assumption that
/// wasn't caught by any of our tests).
// Java: com/google/devtools/build/lib/sandbox/SandboxModule.java:400 SandboxModule.checkSandboxBaseTopOnlyContainsPersistentDirs
fn check_sandbox_base_top_only_contains_persistent_dirs(sandbox_base: &Path) {
    let checked: io::Result<()> = (|| {
        let mut directory_entries: Vec<String> = sandbox_base
            .get_directory_entries()?
            .iter()
            .map(|p| p.get_base_name().to_string())
            .collect();
        // If sandbox initialization failed in-between creating the inaccessible dir/file and
        // adding the Linux sandboxing strategy to spawnRunners, then the sandbox base will be in a
        // bad state. We check for that here and clean up.
        if directory_entries
            .iter()
            .any(|e| e == INACCESSIBLE_HELPER_DIR)
        {
            let inaccessible_helper_dir = sandbox_base.get_child(INACCESSIBLE_HELPER_DIR);
            inaccessible_helper_dir.chmod(0o700)?;
            remove_first(&mut directory_entries, INACCESSIBLE_HELPER_DIR);
            inaccessible_helper_dir.delete_tree()?;
        }
        if directory_entries
            .iter()
            .any(|e| e == INACCESSIBLE_HELPER_FILE)
        {
            let inaccessible_helper_file = sandbox_base.get_child(INACCESSIBLE_HELPER_FILE);
            remove_first(&mut directory_entries, INACCESSIBLE_HELPER_FILE);
            inaccessible_helper_file.delete()?;
        }

        if !directory_entries
            .iter()
            .all(|e| SANDBOX_BASE_PERSISTENT_DIRS.contains(&e.as_str()))
        {
            let mut message = String::from(
                "Found unexpected entries in sandbox base. Please report this in https://github.com/bazelbuild/bazel/issues.",
            );
            message.push_str(" The entries are: ");
            message.push_str(&directory_entries.join(", "));
            panic!("IllegalStateException: {}", message);
        }
        Ok(())
    })();
    // PORT: logging dropped (on error: "Failed to clean up sandbox base %s").
    let _ = checked;
}

/// `List.remove(Object)`: removes the first occurrence.
fn remove_first(list: &mut Vec<String>, value: &str) {
    if let Some(i) = list.iter().position(|e| e == value) {
        list.remove(i);
    }
}

// Java: com/google/devtools/build/lib/sandbox/SandboxModule.java:502 SandboxModule.getStaleTrashDir
fn get_stale_trash_dir(trash_base: &Path) -> Path {
    let parent = trash_base
        .get_parent_directory()
        .expect("NullPointerException: trashBase has no parent");
    let mut i = 0;
    loop {
        let exists = parent.get_child(&format!("stale-trash-{}", i)).exists();
        i += 1;
        if !exists {
            break;
        }
    }
    i -= 1;
    parent.get_child(&format!("stale-trash-{}", i))
}

/// `WindowsSandboxUtil.isAvailable(binary, clientEnv)`: runs `<binary> -h` and checks that it
/// exits with 0.
// Java: com/google/devtools/build/lib/sandbox/WindowsSandboxUtil.java:48 WindowsSandboxUtil.isAvailable
fn windows_sandbox_is_available(
    binary: &PathFragment,
    client_env: &IndexMap<String, String>,
) -> bool {
    let mut builder = SubprocessBuilder::new(client_env);
    builder.set_argv(vec![binary.get_path_string().to_string(), "-h".to_string()]);
    builder.set_stdout_action(SubprocessBuilderStreamAction::Stream);
    builder.set_redirect_error_stream(true);
    builder.set_working_directory(Some(JavaFile::new(".")));
    let process = match builder.start() {
        Ok(p) => p,
        Err(_e) => {
            // PORT: logging dropped ("Windows sandbox binary at %s seems to be missing").
            return false;
        }
    };

    let mut out_err_bytes: Vec<u8> = Vec::new();
    if let Some(mut input) = process.get_input_stream()
        && let Err(e) = io::Read::read_to_end(&mut input, &mut out_err_bytes)
    {
        out_err_bytes.extend_from_slice(format!("Failed to read stdout: {}", e).as_bytes());
    }

    process.wait_for_uninterruptibly();
    let exit_code = process.exit_value();
    // PORT: logging dropped (on non-zero exit: "Windows sandbox binary at %s returned non-zero
    // exit code %d and output %s").
    exit_code == 0
}
