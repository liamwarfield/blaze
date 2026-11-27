//! Port of `lib/sandbox/LinuxSandboxCommandLineBuilder.java`.

use blaze_util::java_duration::JavaDuration;

use blaze_vfs::{Path, PathFragment};
use indexmap::{IndexMap, IndexSet};

use crate::actions::execution_requirements;

/// The possibilities for creating a network namespace in the sandbox
/// (`LinuxSandboxCommandLineBuilder.NetworkNamespace`).
// Java: com/google/devtools/build/lib/sandbox/LinuxSandboxCommandLineBuilder.java:312 LinuxSandboxCommandLineBuilder.NetworkNamespace
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum NetworkNamespace {
    /// No network namespace will be created, sandboxed processes can access the network freely.
    NoNetns,
    /// A fresh network namespace will be created.
    Netns,
    /// A fresh network namespace will be created, and a loopback device will be set up in it.
    NetnsWithLoopback,
}

/// A builder class for constructing the full command line to run a command using the
/// `linux-sandbox` tool.
// Java: com/google/devtools/build/lib/sandbox/LinuxSandboxCommandLineBuilder.java:38 LinuxSandboxCommandLineBuilder
#[derive(Clone, Debug)]
pub struct LinuxSandboxCommandLineBuilder {
    linux_sandbox_path: Path,
    hermetic_sandbox_path: Option<Path>,
    working_directory: Option<Path>,
    timeout: Option<JavaDuration>,
    kill_delay: Option<JavaDuration>,
    persistent_process: bool,
    stdout_path: Option<Path>,
    stderr_path: Option<Path>,
    writable_files_and_directories: IndexSet<Path>,
    tmpfs_directories: IndexSet<PathFragment>,
    bind_mounts: IndexMap<Path, Path>,
    statistics_path: Option<Path>,
    use_fake_hostname: bool,
    create_network_namespace: NetworkNamespace,
    use_fake_root: bool,
    use_fake_username: bool,
    enable_pseudoterminal: bool,
    sandbox_debug_path: Option<String>,
    sigint_sends_sigterm: bool,
    /// `Set<java.nio.file.Path>`, as path strings.
    cgroups_dirs: IndexSet<String>,
}

impl LinuxSandboxCommandLineBuilder {
    /// Returns a new command line builder for the `linux-sandbox` tool.
    // Java: com/google/devtools/build/lib/sandbox/LinuxSandboxCommandLineBuilder.java:65 LinuxSandboxCommandLineBuilder.commandLineBuilder
    pub fn command_line_builder(linux_sandbox_path: Path) -> LinuxSandboxCommandLineBuilder {
        LinuxSandboxCommandLineBuilder {
            linux_sandbox_path,
            hermetic_sandbox_path: None,
            working_directory: None,
            timeout: None,
            kill_delay: None,
            persistent_process: false,
            stdout_path: None,
            stderr_path: None,
            writable_files_and_directories: IndexSet::new(),
            tmpfs_directories: IndexSet::new(),
            bind_mounts: IndexMap::new(),
            statistics_path: None,
            use_fake_hostname: false,
            create_network_namespace: NetworkNamespace::NoNetns,
            use_fake_root: false,
            use_fake_username: false,
            enable_pseudoterminal: false,
            sandbox_debug_path: None,
            sigint_sends_sigterm: false,
            cgroups_dirs: IndexSet::new(),
        }
    }

    /// Sets the sandbox path to chroot to, required for the hermetic linux sandbox to figure out
    /// where the working directory is.
    // Java: com/google/devtools/build/lib/sandbox/LinuxSandboxCommandLineBuilder.java:73 LinuxSandboxCommandLineBuilder.setHermeticSandboxPath
    pub fn set_hermetic_sandbox_path(&mut self, sandbox_path: Path) -> &mut Self {
        self.hermetic_sandbox_path = Some(sandbox_path);
        self
    }

    /// Sets the working directory to use, if any.
    // Java: com/google/devtools/build/lib/sandbox/LinuxSandboxCommandLineBuilder.java:80 LinuxSandboxCommandLineBuilder.setWorkingDirectory
    pub fn set_working_directory(&mut self, working_directory: Path) -> &mut Self {
        self.working_directory = Some(working_directory);
        self
    }

    /// Sets the timeout for the command run using the `linux-sandbox` tool.
    // Java: com/google/devtools/build/lib/sandbox/LinuxSandboxCommandLineBuilder.java:87 LinuxSandboxCommandLineBuilder.setTimeout
    pub fn set_timeout(&mut self, timeout: impl Into<JavaDuration>) -> &mut Self {
        self.timeout = Some(timeout.into());
        self
    }

    /// Sets the kill delay for commands run using the `linux-sandbox` tool that exceed their
    /// timeout.
    // Java: com/google/devtools/build/lib/sandbox/LinuxSandboxCommandLineBuilder.java:97 LinuxSandboxCommandLineBuilder.setKillDelay
    pub fn set_kill_delay(&mut self, kill_delay: impl Into<JavaDuration>) -> &mut Self {
        self.kill_delay = Some(kill_delay.into());
        self
    }

    // Java: com/google/devtools/build/lib/sandbox/LinuxSandboxCommandLineBuilder.java:103 LinuxSandboxCommandLineBuilder.setPersistentProcess
    pub fn set_persistent_process(&mut self, persistent_process: bool) -> &mut Self {
        self.persistent_process = persistent_process;
        self
    }

    /// Sets the path to use for redirecting stdout, if any.
    // Java: com/google/devtools/build/lib/sandbox/LinuxSandboxCommandLineBuilder.java:110 LinuxSandboxCommandLineBuilder.setStdoutPath
    pub fn set_stdout_path(&mut self, stdout_path: Path) -> &mut Self {
        self.stdout_path = Some(stdout_path);
        self
    }

    /// Sets the path to use for redirecting stderr, if any.
    // Java: com/google/devtools/build/lib/sandbox/LinuxSandboxCommandLineBuilder.java:117 LinuxSandboxCommandLineBuilder.setStderrPath
    pub fn set_stderr_path(&mut self, stderr_path: Path) -> &mut Self {
        self.stderr_path = Some(stderr_path);
        self
    }

    /// Sets the files or directories to make writable for the sandboxed process, if any.
    // Java: com/google/devtools/build/lib/sandbox/LinuxSandboxCommandLineBuilder.java:124 LinuxSandboxCommandLineBuilder.setWritableFilesAndDirectories
    pub fn set_writable_files_and_directories(
        &mut self,
        writable_files_and_directories: IndexSet<Path>,
    ) -> &mut Self {
        self.writable_files_and_directories = writable_files_and_directories;
        self
    }

    // Java: com/google/devtools/build/lib/sandbox/LinuxSandboxCommandLineBuilder.java:132 LinuxSandboxCommandLineBuilder.addWritablePath
    pub fn add_writable_path(&mut self, writable_path: Path) -> &mut Self {
        self.writable_files_and_directories.insert(writable_path);
        self
    }

    /// Sets the directories where to mount an empty tmpfs, if any.
    // Java: com/google/devtools/build/lib/sandbox/LinuxSandboxCommandLineBuilder.java:141 LinuxSandboxCommandLineBuilder.setTmpfsDirectories
    pub fn set_tmpfs_directories(
        &mut self,
        tmpfs_directories: IndexSet<PathFragment>,
    ) -> &mut Self {
        self.tmpfs_directories = tmpfs_directories;
        self
    }

    /// Sets the sources and targets of files or directories to explicitly bind-mount in the
    /// sandbox, if any (target -> source).
    // Java: com/google/devtools/build/lib/sandbox/LinuxSandboxCommandLineBuilder.java:152 LinuxSandboxCommandLineBuilder.setBindMounts
    pub fn set_bind_mounts(&mut self, bind_mounts: IndexMap<Path, Path>) -> &mut Self {
        self.bind_mounts = bind_mounts;
        self
    }

    /// Sets the path for writing execution statistics (e.g. resource usage).
    // Java: com/google/devtools/build/lib/sandbox/LinuxSandboxCommandLineBuilder.java:159 LinuxSandboxCommandLineBuilder.setStatisticsPath
    pub fn set_statistics_path(&mut self, statistics_path: Path) -> &mut Self {
        self.statistics_path = Some(statistics_path);
        self
    }

    /// Sets whether to use a fake 'localhost' hostname inside the sandbox.
    // Java: com/google/devtools/build/lib/sandbox/LinuxSandboxCommandLineBuilder.java:166 LinuxSandboxCommandLineBuilder.setUseFakeHostname
    pub fn set_use_fake_hostname(&mut self, use_fake_hostname: bool) -> &mut Self {
        self.use_fake_hostname = use_fake_hostname;
        self
    }

    /// Sets whether and how to create a new network namespace.
    // Java: com/google/devtools/build/lib/sandbox/LinuxSandboxCommandLineBuilder.java:173 LinuxSandboxCommandLineBuilder.setCreateNetworkNamespace
    pub fn set_create_network_namespace(
        &mut self,
        create_network_namespace: NetworkNamespace,
    ) -> &mut Self {
        self.create_network_namespace = create_network_namespace;
        self
    }

    /// Sets whether to pretend to be 'root' inside the namespace.
    // Java: com/google/devtools/build/lib/sandbox/LinuxSandboxCommandLineBuilder.java:181 LinuxSandboxCommandLineBuilder.setUseFakeRoot
    pub fn set_use_fake_root(&mut self, use_fake_root: bool) -> &mut Self {
        self.use_fake_root = use_fake_root;
        self
    }

    /// Sets whether to use a fake 'nobody' username inside the sandbox.
    // Java: com/google/devtools/build/lib/sandbox/LinuxSandboxCommandLineBuilder.java:188 LinuxSandboxCommandLineBuilder.setUseFakeUsername
    pub fn set_use_fake_username(&mut self, use_fake_username: bool) -> &mut Self {
        self.use_fake_username = use_fake_username;
        self
    }

    /// Sets whether to set group to 'tty' and make /dev/pts writable inside the sandbox in order
    /// to enable the use of pseudoterminals.
    // Java: com/google/devtools/build/lib/sandbox/LinuxSandboxCommandLineBuilder.java:198 LinuxSandboxCommandLineBuilder.setEnablePseudoterminal
    pub fn set_enable_pseudoterminal(&mut self, enable_pseudoterminal: bool) -> &mut Self {
        self.enable_pseudoterminal = enable_pseudoterminal;
        self
    }

    /// Sets the output path for sandbox debugging messages.
    // Java: com/google/devtools/build/lib/sandbox/LinuxSandboxCommandLineBuilder.java:205 LinuxSandboxCommandLineBuilder.setSandboxDebugPath
    pub fn set_sandbox_debug_path(&mut self, sandbox_debug_path: String) -> &mut Self {
        self.sandbox_debug_path = Some(sandbox_debug_path);
        self
    }

    /// Sets the directory to be used for cgroups. Cgroups can be used to set limits on resource
    /// usage of a subprocess tree, and to gather statistics. Requires cgroups v2 and systemd. This
    /// directory must be under `/sys/fs/cgroup` and the user running Bazel must have write
    /// permissions to this directory, its parent directory, and the cgroup directory for the Bazel
    /// process.
    // Java: com/google/devtools/build/lib/sandbox/LinuxSandboxCommandLineBuilder.java:217 LinuxSandboxCommandLineBuilder.setCgroupsDirs
    pub fn set_cgroups_dirs(&mut self, cgroups_dirs: IndexSet<String>) -> &mut Self {
        self.cgroups_dirs = cgroups_dirs;
        self
    }

    /// Incorporates settings from a spawn's execution info.
    // Java: com/google/devtools/build/lib/sandbox/LinuxSandboxCommandLineBuilder.java:225 LinuxSandboxCommandLineBuilder.addExecutionInfo
    pub fn add_execution_info(&mut self, execution_info: &IndexMap<String, String>) -> &mut Self {
        if execution_info.contains_key(execution_requirements::GRACEFUL_TERMINATION) {
            self.sigint_sends_sigterm = true;
        }
        self
    }

    /// Builds the command line to invoke a specific command using the `linux-sandbox` tool.
    // Java: com/google/devtools/build/lib/sandbox/LinuxSandboxCommandLineBuilder.java:233 LinuxSandboxCommandLineBuilder.buildForCommand
    pub fn build_for_command<S: AsRef<str>>(&self, command_arguments: &[S]) -> Vec<String> {
        blaze_util::check_state!(
            !(self.use_fake_username && self.use_fake_root),
            "useFakeUsername and useFakeRoot are exclusive"
        );

        let mut command_line_builder: Vec<String> = Vec::new();

        command_line_builder.push(self.linux_sandbox_path.get_path_string().to_string());
        if let Some(working_directory) = &self.working_directory {
            command_line_builder.push("-W".to_string());
            command_line_builder.push(working_directory.get_path_string().to_string());
        }
        if let Some(timeout) = self.timeout {
            command_line_builder.push("-T".to_string());
            command_line_builder.push(timeout.to_seconds().to_string());
        }
        if let Some(kill_delay) = self.kill_delay {
            command_line_builder.push("-t".to_string());
            command_line_builder.push(kill_delay.to_seconds().to_string());
        }
        if let Some(stdout_path) = &self.stdout_path {
            command_line_builder.push("-l".to_string());
            command_line_builder.push(stdout_path.get_path_string().to_string());
        }
        if let Some(stderr_path) = &self.stderr_path {
            command_line_builder.push("-L".to_string());
            command_line_builder.push(stderr_path.get_path_string().to_string());
        }
        for writable_path in &self.writable_files_and_directories {
            command_line_builder.push("-w".to_string());
            command_line_builder.push(writable_path.get_path_string().to_string());
        }
        for tmpfs_path in &self.tmpfs_directories {
            command_line_builder.push("-e".to_string());
            command_line_builder.push(tmpfs_path.get_path_string().to_string());
        }
        for (bind_mount_target, bind_mount_source) in &self.bind_mounts {
            command_line_builder.push("-M".to_string());
            command_line_builder.push(bind_mount_source.get_path_string().to_string());
            // The file is mounted in a custom location inside the sandbox.
            if bind_mount_source != bind_mount_target {
                command_line_builder.push("-m".to_string());
                command_line_builder.push(bind_mount_target.get_path_string().to_string());
            }
        }
        if let Some(statistics_path) = &self.statistics_path {
            command_line_builder.push("-S".to_string());
            command_line_builder.push(statistics_path.get_path_string().to_string());
        }
        if let Some(hermetic_sandbox_path) = &self.hermetic_sandbox_path {
            command_line_builder.push("-h".to_string());
            command_line_builder.push(hermetic_sandbox_path.get_path_string().to_string());
        }
        if self.use_fake_hostname {
            command_line_builder.push("-H".to_string());
        }
        if self.create_network_namespace == NetworkNamespace::NetnsWithLoopback {
            command_line_builder.push("-N".to_string());
        } else if self.create_network_namespace == NetworkNamespace::Netns {
            command_line_builder.push("-n".to_string());
        }
        if self.use_fake_root {
            command_line_builder.push("-R".to_string());
        }
        if self.use_fake_username {
            command_line_builder.push("-U".to_string());
        }
        if self.enable_pseudoterminal {
            command_line_builder.push("-P".to_string());
        }
        if let Some(sandbox_debug_path) = &self.sandbox_debug_path {
            command_line_builder.push("-D".to_string());
            command_line_builder.push(sandbox_debug_path.clone());
        }
        if self.sigint_sends_sigterm {
            command_line_builder.push("-i".to_string());
        }
        if self.persistent_process {
            command_line_builder.push("-p".to_string());
        }
        for dir in &self.cgroups_dirs {
            command_line_builder.push("-C".to_string());
            command_line_builder.push(dir.clone());
        }
        command_line_builder.push("--".to_string());
        command_line_builder.extend(command_arguments.iter().map(|a| a.as_ref().to_string()));

        command_line_builder
    }
}
