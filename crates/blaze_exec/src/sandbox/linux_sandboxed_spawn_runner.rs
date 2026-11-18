//! Port of `lib/sandbox/LinuxSandboxedSpawnRunner.java`.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::io;
use std::sync::Arc;
use std::time::Duration;

use blaze_optdefs::misc::sandbox_options::SandboxOptions;
use blaze_shell::{Command, JavaFile, thread_interrupt};
use blaze_vfs::{FileSystemExt, FileSystemRef, Path, PathFragment, Symlinks};
use indexmap::{IndexMap, IndexSet};
use parking_lot::Mutex;

use super::abstract_sandbox_spawn_runner::{
    AbstractSandboxSpawnRunner, SandboxRunnerError, SandboxSpawnRunnerHooks,
};
use super::hardlinked_sandboxed_spawn::HardlinkedSandboxedSpawn;
use super::linux_sandbox_command_line_builder::{LinuxSandboxCommandLineBuilder, NetworkNamespace};
use super::linux_sandbox_util;
use super::sandbox_helpers;
use super::sandboxed_spawn::SandboxedSpawn;
use super::symlinked_sandboxed_spawn::SymlinkedSandboxedSpawn;
use crate::actions::{
    ExecException, FileContentsProxy, Spawn, SpawnResult, execution_requirements, spawns,
};
use crate::exec::exec_command_environment::ExecCommandEnvironment;
use crate::exec::local::local_env_provider::{LocalEnvProvider, PosixLocalEnvProvider};
use crate::exec::spawn_runner::{SpawnExecutionContext, SpawnRunner};
use crate::exec::tree_deleter::TreeDeleter;

/// Since checking if sandbox is supported is expensive, we remember what we've checked.
// Java: com/google/devtools/build/lib/sandbox/LinuxSandboxedSpawnRunner.java:75 LinuxSandboxedSpawnRunner.isSupportedMap
static IS_SUPPORTED_MAP: Mutex<Option<HashMap<String, bool>>> = Mutex::new(None);

/// Spawn runner that uses linux sandboxing APIs to execute a local subprocess.
// Java: com/google/devtools/build/lib/sandbox/LinuxSandboxedSpawnRunner.java:73 LinuxSandboxedSpawnRunner
pub struct LinuxSandboxedSpawnRunner {
    base: AbstractSandboxSpawnRunner,
    file_system: FileSystemRef,
    exec_root: Path,
    allow_network: bool,
    linux_sandbox: Path,
    sandbox_base: Path,
    inaccessible_helper_file: Option<Path>,
    inaccessible_helper_dir: Option<Path>,
    local_env_provider: PosixLocalEnvProvider,
    timeout_kill_delay: Duration,
    tree_deleter: Arc<dyn TreeDeleter>,
    slash_tmp: Path,
    known_paths_to_mount_under_hermetic_tmp: IndexSet<Path>,
    /// `cgroupFactory`: `Some(defaultLimits)` when it exists.
    cgroup_factory_default_limits: Option<Vec<(String, f64)>>,
}

impl LinuxSandboxedSpawnRunner {
    /// Returns whether the linux sandbox is supported on the local machine by running a small
    /// command in it.
    // Java: com/google/devtools/build/lib/sandbox/LinuxSandboxedSpawnRunner.java:83 LinuxSandboxedSpawnRunner.isSupported
    pub fn is_supported(cmd_env: &ExecCommandEnvironment) -> bool {
        // PORT: skipped, the OS.getCurrent() != OS.LINUX check (Linux only).
        if !linux_sandbox_util::is_supported(&cmd_env.bin_tools) {
            return false;
        }
        let linux_sandbox = linux_sandbox_util::get_linux_sandbox(&cmd_env.bin_tools)
            .expect("checked by isSupported");
        let mut map = IS_SUPPORTED_MAP.lock();
        let map = map.get_or_insert_with(HashMap::new);
        if let Some(is_supported) = map.get(linux_sandbox.get_path_string()) {
            return *is_supported;
        }
        let is_supported = compute_is_supported(cmd_env, &linux_sandbox);
        map.insert(linux_sandbox.get_path_string().to_string(), is_supported);
        is_supported
    }

    /// Creates a sandboxed spawn runner that uses the `linux-sandbox` tool.
    ///
    /// `sandbox_base`: path to the sandbox base directory; `inaccessible_helper_file`: path to a
    /// file that is (already) inaccessible; `inaccessible_helper_dir`: path to a directory that is
    /// (already) inaccessible; `timeout_kill_delay`: an additional grace period before killing
    /// timing out commands.
    // Java: com/google/devtools/build/lib/sandbox/LinuxSandboxedSpawnRunner.java:150 LinuxSandboxedSpawnRunner.<init>
    pub fn new(
        cmd_env: &ExecCommandEnvironment,
        sandbox_base: Path,
        inaccessible_helper_file: Option<Path>,
        inaccessible_helper_dir: Option<Path>,
        timeout_kill_delay: Duration,
        tree_deleter: Arc<dyn TreeDeleter>,
    ) -> LinuxSandboxedSpawnRunner {
        let base = AbstractSandboxSpawnRunner::new(cmd_env);
        let sandbox_options = &cmd_env.sandbox_options;
        // Java: new VirtualCgroupFactory("sandbox_", VirtualCgroup.getInstance(),
        // getSandboxOptions().getLimits(), /* alwaysCreate= */ false).
        // PORT: VirtualCgroup.getInstance() (which creates the per-server
        // blaze_<pid>_spawns.slice cgroup) is not ported; the factory only records its default
        // limits. As long as no limit applies, VirtualCgroupFactory.create returns
        // VirtualCgroup.NULL (no -C flag), which is what the port does; any limit is unported!.
        let cgroup_factory_default_limits = if sandbox_options.use_new_cgroup_implementation {
            Some(sandbox_options.get_limits())
        } else {
            None
        };
        let file_system = cmd_env.file_system.clone();
        let slash_tmp = file_system.get_path("/tmp");
        let mut runner = LinuxSandboxedSpawnRunner {
            base,
            file_system: file_system.clone(),
            exec_root: cmd_env.exec_root.clone(),
            allow_network: sandbox_helpers::should_allow_network(&cmd_env.test_arguments),
            linux_sandbox: linux_sandbox_util::get_linux_sandbox(&cmd_env.bin_tools)
                .expect("NullPointerException: linux-sandbox is not an embedded tool"),
            sandbox_base,
            inaccessible_helper_file,
            inaccessible_helper_dir,
            timeout_kill_delay,
            local_env_provider: PosixLocalEnvProvider::new(cmd_env.client_env.clone()),
            tree_deleter,
            slash_tmp,
            known_paths_to_mount_under_hermetic_tmp: IndexSet::new(),
            cgroup_factory_default_limits,
        };
        runner.known_paths_to_mount_under_hermetic_tmp =
            runner.collect_paths_to_mount_under_hermetic_tmp(cmd_env);
        runner
    }

    // Java: com/google/devtools/build/lib/sandbox/LinuxSandboxedSpawnRunner.java:181 LinuxSandboxedSpawnRunner.collectPathsToMountUnderHermeticTmp
    fn collect_paths_to_mount_under_hermetic_tmp(
        &self,
        cmd_env: &ExecCommandEnvironment,
    ) -> IndexSet<Path> {
        // If any path managed or tracked by Bazel is under /tmp, it needs to be explicitly mounted
        // into the sandbox when using hermetic /tmp. We attempt to collect an over-approximation
        // of these paths, as the main goal of hermetic /tmp is to avoid inheriting any direct or
        // well-known children of /tmp from the host.
        // TODO(bazel-team): Review all flags whose path may have to be considered here.
        let mut candidates: Vec<Path> =
            vec![self.sandbox_base.clone(), cmd_env.output_base.clone()];
        candidates.extend(cmd_env.repo_contents_cache_path.iter().cloned());
        candidates.extend(cmd_env.package_path_entries.iter().cloned());
        candidates
            .into_iter()
            .filter(|p| p.starts_with(&self.slash_tmp))
            // For any path /tmp/dir1/dir2 we encounter, we instead mount /tmp/dir1 (first two
            // path segments). This is necessary to gracefully handle an edge case:
            // - A workspace contains a subdirectory (e.g. examples) that is itself a workspace.
            // - The child workspace brings in the parent workspace as a local_repository with
            //   an up-level reference.
            // - The parent workspace is checked out under /tmp.
            // In this scenario, the parent workspace's external source root points to the parent
            // workspace's source directory under /tmp, but this directory is neither under the
            // output base nor on the package path. While it would be possible to track the
            // external roots of all inputs and mount their entire symlink chain, this would be
            // very invasive to do in the face of resolved symlink artifacts (and impossible with
            // unresolved symlinks).
            // Instead, by mounting the direct children of /tmp that are parents of the source
            // roots, we attempt to cover all reasonable cases in which repositories symlink
            // paths relative to themselves and workspaces are checked out into subdirectories of
            // /tmp. All explicit references to paths under /tmp must be handled by the user via
            // --sandbox_add_mount_pair.
            .map(|p| {
                let fragment = p.as_fragment();
                p.get_file_system().get_path_fragment(
                    &fragment.sub_fragment(0, std::cmp::min(2, fragment.segment_count())),
                )
            })
            .collect()
    }

    // Java: com/google/devtools/build/lib/sandbox/LinuxSandboxedSpawnRunner.java:218 LinuxSandboxedSpawnRunner.useHermeticTmp
    fn use_hermetic_tmp(&self) -> bool {
        let options = self.get_sandbox_options();
        if options.use_hermetic {
            // The hermetic sandbox is, well, already hermetic. Also, it creates an empty /tmp by
            // default so nothing needs to be done to achieve a /tmp that is also hermetic.
            return false;
        }

        let tmp_explicitly_bind_mounted = options
            .sandbox_additional_mounts
            .iter()
            .any(|(key, _)| key == "/tmp");
        if tmp_explicitly_bind_mounted {
            // An explicit mount on /tmp is an explicit way to make it non-hermetic.
            return false;
        }

        if self
            .known_paths_to_mount_under_hermetic_tmp
            .contains(&self.slash_tmp)
        {
            // /tmp as a package path entry or output base seems very unlikely to work, but the
            // bind mounting logic is not prepared for it and we don't want to crash, so just
            // disable hermetic tmp in this case.
            return false;
        }

        if options
            .sandbox_tmpfs_path
            .iter()
            .any(|p| &p.0 == self.slash_tmp.as_fragment())
        {
            // A tmpfs path under /tmp is as hermetic as "hermetic /tmp".
            return false;
        }

        true
    }

    fn get_sandbox_options(&self) -> &SandboxOptions {
        self.base.get_sandbox_options()
    }

    /// `prepareSpawn(spawn, context)`.
    // Java: com/google/devtools/build/lib/sandbox/LinuxSandboxedSpawnRunner.java:249 LinuxSandboxedSpawnRunner.prepareSpawn
    pub fn prepare_spawn(
        &self,
        spawn: &dyn Spawn,
        context: &dyn SpawnExecutionContext,
    ) -> Result<Box<dyn SandboxedSpawn>, SandboxRunnerError> {
        // Each invocation of "exec" gets its own sandbox base.
        // Note that the value returned by context.getId() is only unique inside one given
        // SpawnRunner, so we have to prefix our name to turn it into a globally unique value.
        let sandbox_path = self
            .sandbox_base
            .get_relative_str(&self.get_name())
            .get_relative_str(&context.get_id().to_string());

        // b/64689608: The execroot of the sandboxed process must end with the workspace name, just
        // like the normal execroot does.
        let workspace_name = self.exec_root.get_base_name();
        let sandbox_exec_root = sandbox_path
            .get_relative_str("execroot")
            .get_relative_str(workspace_name);
        sandbox_exec_root.create_directory_and_parents()?;

        let inputs = sandbox_helpers::process_input_files(
            &context.get_input_mapping(
                &PathFragment::empty_fragment(),
                /* will_access_repeatedly= */ true,
            ),
            &self.exec_root,
        )?;

        let environment = self.local_env_provider.rewrite_local_env(
            spawn.get_environment(),
            &self.base.bin_tools,
            "/tmp",
        )?;
        let writable_dirs = self.get_writable_dirs(&sandbox_exec_root, &environment)?;

        let mut sandbox_tmp: Option<Path> = None;
        let mut paths_under_tmp_to_mount: IndexSet<Path> = IndexSet::new();
        if self.use_hermetic_tmp() {
            // Special paths under /tmp are treated exactly like a user mount under /tmp to ensure
            // that they are visible at the same path after mounting the hermetic tmp.
            paths_under_tmp_to_mount = self.known_paths_to_mount_under_hermetic_tmp.clone();

            // The initially empty directory that will be mounted as /tmp in the sandbox.
            let tmp = sandbox_path.get_relative_str("_hermetic_tmp");
            tmp.create_directory_and_parents()?;

            let fragments: Vec<PathFragment> = self
                .get_sandbox_options()
                .sandbox_tmpfs_path
                .iter()
                .map(|p| p.0.clone())
                .chain(writable_dirs.iter().map(|p| p.as_fragment().clone()))
                .collect();
            for path_fragment in fragments {
                let path = self.file_system.get_path_fragment(&path_fragment);
                if path.starts_with(&self.slash_tmp) {
                    // tmpfs mount points and writable dirs must exist, which is usually the user's
                    // responsibility. But if the user requests a path mount under /tmp, we have to
                    // create it under the sandbox tmp directory.
                    tmp.get_relative(&path.relative_to(&self.slash_tmp))
                        .create_directory_and_parents()?;
                }
            }
            sandbox_tmp = Some(tmp);
        }

        let outputs = sandbox_helpers::get_outputs(spawn);
        let timeout = context.get_timeout();
        let sandbox_options = self.get_sandbox_options();

        let create_network_namespace = !(self.allow_network
            || spawns::requires_network(spawn, sandbox_options.default_sandbox_allow_network));
        let mut command_line_builder =
            LinuxSandboxCommandLineBuilder::command_line_builder(self.linux_sandbox.clone());
        command_line_builder
            .add_execution_info(spawn.get_execution_info())
            .set_writable_files_and_directories(writable_dirs.clone())
            .set_tmpfs_directories(
                sandbox_options
                    .sandbox_tmpfs_path
                    .iter()
                    .map(|p| p.0.clone())
                    .collect(),
            )
            .set_bind_mounts(self.prepare_and_get_bind_mounts(
                &sandbox_exec_root,
                sandbox_tmp.as_ref(),
                &paths_under_tmp_to_mount,
            )?)
            .set_use_fake_hostname(sandbox_options.sandbox_fake_hostname)
            .set_enable_pseudoterminal(sandbox_options.sandbox_explicit_pseudoterminal)
            .set_create_network_namespace(if create_network_namespace {
                self.get_network_namespace()
            } else {
                NetworkNamespace::NoNetns
            })
            .set_kill_delay(self.timeout_kill_delay);

        let mut sandbox_debug_path: Option<Path> = None;
        if sandbox_options.sandbox_debug {
            let p = sandbox_path.get_relative_str("debug.out");
            command_line_builder.set_sandbox_debug_path(p.get_path_string().to_string());
            sandbox_debug_path = Some(p);
        }

        if let Some(default_limits) = &self.cgroup_factory_default_limits {
            // Java: com/google/devtools/build/lib/sandbox/LinuxSandboxedSpawnRunner.java:322 LinuxSandboxedSpawnRunner.prepareSpawn
            let spawn_resource_limits: Vec<(String, f64)> = if sandbox_options
                .enforce_resources
                .matches(&spawn.get_mnemonic())
            {
                spawn
                    .get_local_resources()
                    .map_err(|e| io::Error::other(e.to_string()))?
                    .get_resources()
                    .iter()
                    .map(|(k, v)| (k.clone(), *v))
                    .collect()
            } else {
                Vec::new()
            };
            // Java: com/google/devtools/build/lib/sandbox/cgroups/VirtualCgroupFactory.java:57 VirtualCgroupFactory.create
            let get = |m: &[(String, f64)], k: &str| m.iter().find(|(n, _)| n == k).map(|e| e.1);
            let no_limits = default_limits.is_empty() && spawn_resource_limits.is_empty();
            let cpu_limit = get(&spawn_resource_limits, "cpu")
                .unwrap_or_else(|| get(default_limits, "cpu").unwrap_or(0.0));
            let memory_limit = (get(&spawn_resource_limits, "memory")
                .unwrap_or_else(|| get(default_limits, "memory").unwrap_or(0.0))
                * 1024.0
                * 1024.0) as i64;
            if !(no_limits || (cpu_limit == 0.0 && memory_limit == 0)) {
                blaze_util::unported!(
                    "VirtualCgroupFactory.create",
                    "a sandbox cgroup limit applies to {}",
                    spawn.get_mnemonic()
                );
            }
            // VirtualCgroup.NULL: no cgroup directories.
        } else if sandbox_options.memory_limit_mb > 0 {
            // We put the sandbox inside a unique subdirectory using the context's ID. This ID is
            // unique per spawn run by this spawn runner.
            blaze_util::unported!(
                "CgroupsInfo.getBlazeSpawnsCgroup",
                "--experimental_sandbox_memory_limit_mb"
            );
        }

        if !timeout.is_zero() {
            command_line_builder.set_timeout(timeout);
        }
        if spawn
            .get_execution_info()
            .contains_key(execution_requirements::REQUIRES_FAKEROOT)
        {
            command_line_builder.set_use_fake_root(true);
        } else if sandbox_options.sandbox_fake_username {
            command_line_builder.set_use_fake_username(true);
        }
        let statistics_path = sandbox_path.get_relative_str("stats.out");
        command_line_builder.set_statistics_path(statistics_path.clone());
        if sandbox_options.use_hermetic {
            command_line_builder.set_hermetic_sandbox_path(sandbox_path.clone());
            Ok(Box::new(HardlinkedSandboxedSpawn::new(
                sandbox_path,
                sandbox_exec_root,
                command_line_builder.build_for_command(spawn.get_arguments()),
                environment,
                inputs,
                outputs,
                writable_dirs,
                self.tree_deleter.clone(),
                sandbox_debug_path,
                Some(statistics_path),
                sandbox_options.sandbox_debug,
                self.make_interactive_debug_arguments(&command_line_builder, sandbox_options),
                &spawn.get_mnemonic(),
            )))
        } else {
            Ok(Box::new(SymlinkedSandboxedSpawn::new(
                sandbox_path,
                sandbox_exec_root,
                command_line_builder.build_for_command(spawn.get_arguments()),
                environment,
                inputs,
                outputs,
                writable_dirs,
                self.tree_deleter.clone(),
                sandbox_debug_path,
                Some(statistics_path),
                self.make_interactive_debug_arguments(&command_line_builder, sandbox_options),
                &spawn.get_mnemonic(),
                spawn.get_target_label(),
            )))
        }
    }

    /// `getWritableDirs(sandboxExecRoot, env)`.
    // Java: com/google/devtools/build/lib/sandbox/LinuxSandboxedSpawnRunner.java:391 LinuxSandboxedSpawnRunner.getWritableDirs
    pub fn get_writable_dirs(
        &self,
        sandbox_exec_root: &Path,
        env: &IndexMap<String, String>,
    ) -> io::Result<IndexSet<Path>> {
        let mut writable_dirs: BTreeSet<Path> = self
            .base
            .get_writable_dirs(sandbox_exec_root, env)?
            .into_iter()
            .collect();
        let fs = sandbox_exec_root.get_file_system();
        writable_dirs.insert(fs.get_path("/dev/shm").resolve_symbolic_links()?);
        writable_dirs.insert(fs.get_path("/tmp"));
        Ok(writable_dirs.into_iter().collect())
    }

    // Java: com/google/devtools/build/lib/sandbox/LinuxSandboxedSpawnRunner.java:400 LinuxSandboxedSpawnRunner.prepareAndGetBindMounts
    fn prepare_and_get_bind_mounts(
        &self,
        sandbox_exec_root: &Path,
        sandbox_tmp: Option<&Path>,
        paths_under_tmp_to_mount: &IndexSet<Path>,
    ) -> Result<IndexMap<Path, Path>, SandboxRunnerError> {
        let mut user_bind_mounts: BTreeMap<Path, Path> = BTreeMap::new();
        // ImmutableMap.Builder.putAll(...).buildKeepingLast()
        let mut additional_mounts: IndexMap<String, String> = IndexMap::new();
        for (key, value) in &self.get_sandbox_options().sandbox_additional_mounts {
            additional_mounts.insert(key.clone(), value.clone());
        }
        sandbox_helpers::mount_additional_paths(
            &additional_mounts,
            sandbox_exec_root,
            &mut user_bind_mounts,
        )?;

        let inaccessible_paths = self.base.get_inaccessible_paths();
        blaze_util::check_state!(
            inaccessible_paths.is_empty()
                || (self.inaccessible_helper_dir.is_some()
                    && self.inaccessible_helper_file.is_some())
        );
        for inaccessible_path in inaccessible_paths {
            if !inaccessible_path.exists() {
                // No need to make non-existent paths inaccessible (this would make the bind mount
                // fail).
                continue;
            }

            if inaccessible_path.is_directory_with(Symlinks::Nofollow) {
                user_bind_mounts.insert(
                    inaccessible_path.clone(),
                    self.inaccessible_helper_dir.clone().expect("checked above"),
                );
            } else {
                user_bind_mounts.insert(
                    inaccessible_path.clone(),
                    self.inaccessible_helper_file
                        .clone()
                        .expect("checked above"),
                );
            }
        }

        linux_sandbox_util::validate_bind_mounts(&user_bind_mounts)?;

        let Some(sandbox_tmp) = sandbox_tmp else {
            return Ok(user_bind_mounts.into_iter().collect());
        };

        let mut bind_mounts: BTreeMap<Path, Path> = BTreeMap::new();
        let entries: Vec<(Path, Path)> = user_bind_mounts
            .into_iter()
            .chain(
                paths_under_tmp_to_mount
                    .iter()
                    .map(|p| (p.clone(), p.clone())),
            )
            .collect();
        for (mount_point, content) in entries {
            let mut mount_point = mount_point;
            if mount_point.starts_with(&self.slash_tmp) {
                // sandboxTmp is null if /tmp is an explicit mount point.
                if mount_point == self.slash_tmp {
                    return Err(io::Error::other(
                        "Cannot mount /tmp explicitly with hermetic /tmp. Please file a bug at https://github.com/bazelbuild/bazel/issues/new/choose.",
                    )
                    .into());
                }
                // We need to rewrite the mount point to be under the sandbox tmp directory, which
                // will be mounted onto /tmp as the final mount.
                mount_point = sandbox_tmp.get_relative(&mount_point.relative_to(&self.slash_tmp));
                mount_point.create_directory_and_parents()?;
            }
            bind_mounts.insert(mount_point, content);
        }

        // Mount $SANDBOX/_hermetic_tmp at /tmp as the final mount.
        let mut result: IndexMap<Path, Path> = bind_mounts.into_iter().collect();
        // Java: ImmutableMap.Builder.buildOrThrow() fails on a duplicate key.
        blaze_util::check_argument!(
            !result.contains_key(&self.slash_tmp),
            "Multiple entries with same key: {}",
            self.slash_tmp
        );
        result.insert(self.slash_tmp.clone(), sandbox_tmp.clone());
        Ok(result)
    }

    // Java: com/google/devtools/build/lib/sandbox/LinuxSandboxedSpawnRunner.java:478 LinuxSandboxedSpawnRunner.checkForConcurrentModifications
    fn check_for_concurrent_modifications(
        &self,
        context: &dyn SpawnExecutionContext,
    ) -> io::Result<()> {
        let input_mapping = context.get_input_mapping(
            &PathFragment::empty_fragment(),
            /* will_access_repeatedly= */ true,
        );
        for input in input_mapping.values() {
            if input.as_virtual_action_input().is_some() {
                // Virtual inputs are not existing in file system and can't be tampered with via
                // sandbox. No need to check them.
                continue;
            }

            let Some(metadata) = context
                .get_input_metadata_provider()
                .get_input_metadata(input)?
            else {
                // This can happen if we are executing a spawn in an action that has multiple
                // spawns and the output of one is the input of another. In this case, we assume
                // that no one modifies an output of the first spawn before the action is completed
                // (which requires the the completion of the second spawn, which happens after this
                // point is reached in the code)
                continue;
            };
            if !metadata.get_type().is_file() {
                // The hermetic sandbox creates hardlinks from files inside sandbox to files
                // outside sandbox. The content of the files outside the sandbox could have been
                // tampered with via the hardlinks. Therefore files are checked for modifications.
                // On the other hand, directories and unresolved symlinks are not represented as
                // hardlinks, and don't have to be checked.
                continue;
            }

            let path = self.exec_root.get_relative(input.get_exec_path());
            if was_modified_since_digest(metadata.get_contents_proxy().as_ref(), &path)? {
                return Err(io::Error::other(format!(
                    "input dependency {} was modified during execution.",
                    path
                )));
            }
        }
        Ok(())
    }

    // Java: com/google/devtools/build/lib/sandbox/LinuxSandboxedSpawnRunner.java:546 LinuxSandboxedSpawnRunner.makeInteractiveDebugArguments
    fn make_interactive_debug_arguments(
        &self,
        command_line_builder: &LinuxSandboxCommandLineBuilder,
        sandbox_options: &SandboxOptions,
    ) -> Option<Vec<String>> {
        if !sandbox_options.sandbox_debug {
            return None;
        }
        Some(command_line_builder.build_for_command(&["/bin/sh", "-i"]))
    }

    // Java: com/google/devtools/build/lib/sandbox/LinuxSandboxedSpawnRunner.java:554 LinuxSandboxedSpawnRunner.getNetworkNamespace
    fn get_network_namespace(&self) -> NetworkNamespace {
        if self.get_sandbox_options().sandbox_enable_loopback_device {
            return NetworkNamespace::NetnsWithLoopback;
        }
        NetworkNamespace::Netns
    }
}

// Java: com/google/devtools/build/lib/sandbox/LinuxSandboxedSpawnRunner.java:103 LinuxSandboxedSpawnRunner.computeIsSupported
fn compute_is_supported(cmd_env: &ExecCommandEnvironment, linux_sandbox: &Path) -> bool {
    let options = &cmd_env.local_execution_options;
    let linux_sandbox_argv =
        LinuxSandboxCommandLineBuilder::command_line_builder(linux_sandbox.clone())
            .set_timeout(options.get_local_sigkill_grace_seconds())
            .build_for_command(&["/bin/true"]);
    let env: IndexMap<String, String> = IndexMap::new();
    let exec_root = &cmd_env.exec_root;
    let cwd = JavaFile::new(exec_root.get_path_string());

    let cmd = Command::with_env(
        linux_sandbox_argv,
        Some(env),
        Some(cwd),
        &cmd_env.client_env,
    );
    // PORT: Java streams the output to null streams; it is captured and dropped here.
    match cmd.execute() {
        Ok(_) => true,
        Err(e) => {
            if e.as_command_exception().is_none() {
                // PORT: Java propagates the InterruptedException; the fixed `is_supported(env) ->
                // bool` API cannot, so the interrupt is re-asserted and the sandbox reported as
                // unsupported.
                thread_interrupt::interrupt_current();
            }
            // PORT: skipped, tracing (Java log) ("Checking for linux sandbox support failed: %s").
            false
        }
    }
}

// Java: com/google/devtools/build/lib/sandbox/LinuxSandboxedSpawnRunner.java:514 LinuxSandboxedSpawnRunner.wasModifiedSinceDigest
fn was_modified_since_digest(proxy: Option<&FileContentsProxy>, path: &Path) -> io::Result<bool> {
    let Some(proxy) = proxy else {
        // Metadata is not available (likely because this is not a regular file).
        return Ok(false);
    };
    let stat = path.stat_if_found(Symlinks::Follow)?;
    Ok(match stat {
        None => true,
        Some(stat) => !stat.is_file() || proxy.is_modified(&FileContentsProxy::create(&stat)),
    })
}

impl SandboxSpawnRunnerHooks for LinuxSandboxedSpawnRunner {
    fn runner_name(&self) -> String {
        self.get_name()
    }

    fn prepare_spawn(
        &self,
        spawn: &dyn Spawn,
        context: &dyn SpawnExecutionContext,
    ) -> Result<Box<dyn SandboxedSpawn>, SandboxRunnerError> {
        LinuxSandboxedSpawnRunner::prepare_spawn(self, spawn, context)
    }

    // Java: com/google/devtools/build/lib/sandbox/LinuxSandboxedSpawnRunner.java:463 LinuxSandboxedSpawnRunner.verifyPostCondition
    fn verify_post_condition(
        &self,
        _original_spawn: &dyn Spawn,
        _sandbox: &dyn SandboxedSpawn,
        context: &dyn SpawnExecutionContext,
    ) -> io::Result<()> {
        if self.get_sandbox_options().use_hermetic {
            self.check_for_concurrent_modifications(context)?;
        }
        // We cannot leave the cgroups around and delete them only when we delete the sandboxes
        // because linux has a hard limit of 65535 memory controllers.
        // PORT: the cgroupFactory is never created (VirtualCgroupFactory is unported! in `new`).
        Ok(())
    }
}

impl SpawnRunner for LinuxSandboxedSpawnRunner {
    fn exec(
        &self,
        spawn: &dyn Spawn,
        context: &dyn SpawnExecutionContext,
    ) -> Result<SpawnResult, ExecException> {
        self.base.exec(self, spawn, context)
    }

    fn can_exec(&self, spawn: &dyn Spawn) -> bool {
        AbstractSandboxSpawnRunner::can_exec(spawn)
    }

    fn handles_caching(&self) -> bool {
        false
    }

    // Java: com/google/devtools/build/lib/sandbox/LinuxSandboxedSpawnRunner.java:386 LinuxSandboxedSpawnRunner.getName
    fn get_name(&self) -> String {
        "linux-sandbox".to_string()
    }

    // Java: com/google/devtools/build/lib/sandbox/LinuxSandboxedSpawnRunner.java:524 LinuxSandboxedSpawnRunner.cleanupSandboxBase
    fn cleanup_sandbox_base(
        &self,
        sandbox_base: &Path,
        tree_deleter: &dyn TreeDeleter,
    ) -> io::Result<()> {
        // PORT: `cgroupsDir` is never assigned in Java (always null). VirtualCgroup.deleteInstance()
        // is skipped: VirtualCgroup is not ported, and here it would only create and destroy the
        // empty per-server cgroup.
        // Delete the inaccessible files synchronously, bypassing the treeDeleter. They are only a
        // couple of files that can be deleted fast, and ensuring they are gone at the end of every
        // build avoids annoying permission denied errors if the user happens to run "rm -rf" on
        // the output base. (We have some tests that do that.)
        if let Some(dir) = &self.inaccessible_helper_dir
            && dir.exists()
        {
            dir.chmod(0o700)?;
            dir.delete_tree()?;
        }
        if let Some(file) = &self.inaccessible_helper_file
            && file.exists()
        {
            file.chmod(0o600)?;
            file.delete()?;
        }

        self.base
            .cleanup_sandbox_base(&self.get_name(), sandbox_base, tree_deleter)
    }
}
