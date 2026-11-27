//! Port of `lib/sandbox/DockerSandboxedSpawnRunner.java`.

use std::collections::HashMap;
use std::io;
use std::sync::{Arc, LazyLock};

use blaze_events::{Event, EventHandler};
use blaze_protos::failure_details::sandbox;
use blaze_shell::{Command, JavaFile, thread_interrupt};
use blaze_vfs::{Path, PathFragment};
use indexmap::{IndexMap, IndexSet};
use parking_lot::Mutex;

use super::abstract_sandbox_spawn_runner::{
    AbstractSandboxSpawnRunner, SandboxRunnerError, SandboxSpawnRunnerHooks,
};
use super::copying_sandboxed_spawn::CopyingSandboxedSpawn;
use super::docker_command_line_builder::DockerCommandLineBuilder;
use super::sandbox_helpers;
use super::sandboxed_spawn::SandboxedSpawn;
use crate::actions::{ExecException, Spawn, SpawnResult, UserExecException, spawns};
use crate::exec::exec_command_environment::ExecCommandEnvironment;
use crate::exec::local::local_env_provider::{self, LocalEnvProvider, PosixLocalEnvProvider};
use crate::exec::spawn_runner::{SpawnExecutionContext, SpawnRunner};
use crate::exec::tree_deleter::TreeDeleter;
use crate::runtime::process_wrapper::ProcessWrapper;

/// The name of the container image entry in the Platform proto (see
/// third_party/googleapis/devtools/remoteexecution/*/remote_execution.proto and
/// remote_default_exec_properties in
/// src/main/java/com/google/devtools/build/lib/remote/RemoteOptions.java)
// Java: com/google/devtools/build/lib/sandbox/DockerSandboxedSpawnRunner.java:72 DockerSandboxedSpawnRunner.CONTAINER_IMAGE_ENTRY_NAME
const CONTAINER_IMAGE_ENTRY_NAME: &str = "container-image";
// Java: com/google/devtools/build/lib/sandbox/DockerSandboxedSpawnRunner.java:73 DockerSandboxedSpawnRunner.DOCKER_IMAGE_PREFIX
const DOCKER_IMAGE_PREFIX: &str = "docker://";

/// `ConcurrentHashMap<String, String> imageMap`.
///
/// PORT: Java's `computeIfAbsent` blocks updates of the same hash bin while an image is built;
/// here one lock is held while an image is built, so image builds do not overlap.
// Java: com/google/devtools/build/lib/sandbox/DockerSandboxedSpawnRunner.java:151 DockerSandboxedSpawnRunner.imageMap
static IMAGE_MAP: LazyLock<Mutex<HashMap<String, String>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// `ProcessWrapper.fromCommandEnvironment(cmdEnv)`.
// Java: com/google/devtools/build/lib/runtime/ProcessWrapper.java:66 ProcessWrapper.fromCommandEnvironment
fn process_wrapper_from_command_environment(
    cmd_env: &ExecCommandEnvironment,
) -> Option<ProcessWrapper> {
    let options = &cmd_env.local_execution_options;
    ProcessWrapper::from_bin_tools(
        &cmd_env.bin_tools,
        Some(options.get_local_sigkill_grace_seconds()),
        options.process_wrapper_graceful_sigterm,
    )
}

/// Spawn runner that uses Docker to execute a local subprocess.
// Java: com/google/devtools/build/lib/sandbox/DockerSandboxedSpawnRunner.java:66 DockerSandboxedSpawnRunner
pub struct DockerSandboxedSpawnRunner {
    base: AbstractSandboxSpawnRunner,
    exec_root: Path,
    allow_network: bool,
    docker_client: Path,
    process_wrapper: ProcessWrapper,
    sandbox_base: Path,
    default_image: String,
    local_env_provider: PosixLocalEnvProvider,
    command_id: String,
    reporter: Arc<dyn EventHandler>,
    use_customized_images: bool,
    tree_deleter: Arc<dyn TreeDeleter>,
    uid: i32,
    gid: i32,
    /// `Collections.synchronizedSet(new HashSet<UUID>())`: the `UUID.toString()`s.
    containers_to_cleanup: Arc<Mutex<IndexSet<String>>>,
    client_env: IndexMap<String, String>,
    remote_options: Arc<blaze_optdefs::misc::remote_options::RemoteOptions>,
}

impl DockerSandboxedSpawnRunner {
    /// Returns whether the darwin sandbox is supported on the local machine by running docker
    /// info. This is expensive, and we have also reports of docker hanging for a long time!
    ///
    /// An `Err` is Java's `InterruptedException` (a [`sandbox_helpers::interrupted_error`]).
    // Java: com/google/devtools/build/lib/sandbox/DockerSandboxedSpawnRunner.java:79 DockerSandboxedSpawnRunner.isSupported
    pub fn is_supported(
        cmd_env: &ExecCommandEnvironment,
        docker_client: &Path,
    ) -> io::Result<bool> {
        let verbose = cmd_env.sandbox_options.docker_verbose;

        if process_wrapper_from_command_environment(cmd_env).is_none() {
            if verbose {
                cmd_env.reporter.handle(&Event::error(
                    "Docker sandboxing is disabled because ProcessWrapper is not supported. This should never happen - is your Bazel binary corrupted?",
                ));
            }
            return Ok(false);
        }

        // On Linux we need to know the UID and GID that we're running as, because otherwise
        // Docker will create files as 'root' and we can't move them to the execRoot.
        // PORT: ProcessUtils.getuid/getgid are libc calls here; the UnsatisfiedLinkError of a
        // Bazel without JNI libraries cannot happen.

        let cmd = Command::with_env(
            vec![
                docker_client.get_path_string().to_string(),
                "info".to_string(),
            ],
            Some(cmd_env.client_env.clone()),
            Some(JavaFile::new(cmd_env.exec_root.get_path_string())),
            &cmd_env.client_env,
        );
        // PORT: Java streams the output to null streams; it is captured and dropped here.
        if let Err(e) = cmd.execute() {
            let Some(command_exception) = e.as_command_exception() else {
                // The InterruptedException leaves isSupported.
                return Err(sandbox_helpers::interrupted_error(None));
            };
            if verbose {
                // `Throwables.getStackTraceAsString(e)`: the exception's `toString()`, then its
                // stack frames.
                // PORT: the stack frames ("\tat com.google...") are Java's; Blaze has none.
                let class = match command_exception {
                    blaze_shell::CommandException::ExecFailed(_) => "ExecFailedException",
                    blaze_shell::CommandException::AbnormalTermination(_) => {
                        "AbnormalTerminationException"
                    }
                    blaze_shell::CommandException::BadExitStatus(_) => "BadExitStatusException",
                };
                cmd_env.reporter.handle(&Event::error(format!(
                    "Docker sandboxing is disabled, because running 'docker info' failed: com.google.devtools.build.lib.shell.{class}: {command_exception}"
                )));
            }
            return Ok(false);
        }

        if verbose {
            cmd_env
                .reporter
                .handle(&Event::info("Docker sandboxing is supported"));
        }

        Ok(true)
    }

    /// Creates a sandboxed spawn runner that uses Docker.
    ///
    /// `docker_client`: path to the `docker` executable. `sandbox_base`: path to the sandbox base
    /// directory. `default_image`: the Docker image to use if the platform doesn't specify one.
    /// `use_customized_images`: whether to use customized images for execution.
    /// `tree_deleter`: scheduler for tree deletions.
    ///
    /// PORT: Java registers the runner on the command's event bus for `CommandCompleteEvent`;
    /// `SandboxModule.afterCommand` calls [`DockerSandboxedSpawnRunner::command_complete`].
    // Java: com/google/devtools/build/lib/sandbox/DockerSandboxedSpawnRunner.java:181 DockerSandboxedSpawnRunner.<init>
    pub fn new(
        cmd_env: &ExecCommandEnvironment,
        docker_client: Path,
        sandbox_base: Path,
        default_image: String,
        use_customized_images: bool,
        tree_deleter: Arc<dyn TreeDeleter>,
    ) -> DockerSandboxedSpawnRunner {
        // OS.getCurrent() == OS.LINUX.
        // Java: com/google/devtools/build/lib/unix/ProcessUtils.java:41 ProcessUtils.getuid
        // Java: com/google/devtools/build/lib/unix/ProcessUtils.java:34 ProcessUtils.getgid
        // PORT: skipped, ProcessUtils.ProcessUtils: the private constructor of a static utility class.
        // SAFETY: plain getuid(2) / getgid(2).
        let (uid, gid) = unsafe { (libc::getuid() as i32, libc::getgid() as i32) };
        DockerSandboxedSpawnRunner {
            base: AbstractSandboxSpawnRunner::new(cmd_env),
            exec_root: cmd_env.exec_root.clone(),
            allow_network: sandbox_helpers::should_allow_network(&cmd_env.test_arguments),
            docker_client,
            process_wrapper: process_wrapper_from_command_environment(cmd_env)
                .expect("NullPointerException: process-wrapper is not an embedded tool"),
            sandbox_base,
            default_image,
            local_env_provider: local_env_provider::for_current_os(&cmd_env.client_env),
            command_id: cmd_env.command_id.clone(),
            reporter: cmd_env.reporter.clone(),
            use_customized_images,
            tree_deleter,
            uid,
            gid,
            containers_to_cleanup: Arc::new(Mutex::new(IndexSet::new())),
            client_env: cmd_env.client_env.clone(),
            remote_options: cmd_env.remote_options.clone(),
        }
    }

    // Java: com/google/devtools/build/lib/sandbox/DockerSandboxedSpawnRunner.java:213 DockerSandboxedSpawnRunner.prepareSpawn
    fn prepare_spawn(
        &self,
        spawn: &dyn Spawn,
        context: &dyn SpawnExecutionContext,
    ) -> Result<Box<dyn SandboxedSpawn>, SandboxRunnerError> {
        // Each invocation of "exec" gets its own sandbox base, execroot and temporary directory.
        let sandbox_path = self
            .sandbox_base
            .get_relative_str(&self.get_name())
            .get_relative_str(&context.get_id().to_string());
        sandbox_path
            .get_parent_directory()
            .expect("sandboxPath has a parent")
            .create_directory()?;
        sandbox_path.create_directory()?;

        // b/64689608: The execroot of the sandboxed process must end with the workspace name,
        // just like the normal execroot does.
        let sandbox_exec_root = sandbox_path
            .get_relative_str("execroot")
            .get_relative_str(self.exec_root.get_base_name());
        sandbox_exec_root
            .get_parent_directory()
            .expect("sandboxExecRoot has a parent")
            .create_directory()?;
        sandbox_exec_root.create_directory()?;

        let environment = self.local_env_provider.rewrite_local_env(
            spawn.get_environment(),
            &self.base.bin_tools,
            "/tmp",
        )?;

        let inputs = sandbox_helpers::process_input_files(
            &context.get_input_mapping(
                &PathFragment::empty_fragment(),
                /* will_access_repeatedly= */ true,
            ),
            &self.exec_root,
        )?;
        let outputs = sandbox_helpers::get_outputs(spawn);

        let timeout = context.get_timeout();

        let uuid = uuid::Uuid::new_v4().to_string();

        let base_image_name = self
            .docker_container_from_spawn(spawn)?
            .unwrap_or_else(|| self.default_image.clone());
        if base_image_name.is_empty() {
            return Err(UserExecException::new(sandbox_helpers::create_failure_detail(
                &format!(
                    "Cannot execute {} mnemonic with Docker, because no image could be found in the remote_execution_properties of the platform and no default image was set via --experimental_docker_image",
                    spawn.get_mnemonic()
                ),
                sandbox::Code::NoDockerImage,
            ))
            .into());
        }

        let customized_image_name = self.get_or_create_customized_image(&base_image_name)?;

        let sandbox_options = self.base.get_sandbox_options();
        let mut cmd_line = DockerCommandLineBuilder::new();
        cmd_line
            .set_process_wrapper(self.process_wrapper.clone())
            .set_docker_client(self.docker_client.clone())
            .set_image_name(customized_image_name)
            .set_command_arguments(spawn.get_arguments().to_vec())
            .set_sandbox_exec_root(sandbox_exec_root.clone())
            .set_additional_mounts(sandbox_options.sandbox_additional_mounts.clone())
            .set_privileged(sandbox_options.docker_privileged)
            .set_environment_variables(environment.clone())
            .set_create_network_namespace(
                !(self.allow_network
                    || spawns::requires_network(
                        spawn,
                        sandbox_options.default_sandbox_allow_network,
                    )),
            )
            .set_command_id(self.command_id.clone())
            .set_uuid(uuid.clone());
        // If uid / gid are -1, we are on an operating system that doesn't require us to set them
        // on the Docker invocation. If they're 0, it means we are running as root and don't need
        // to set them.
        if self.uid > 0 {
            cmd_line.set_uid(self.uid);
        }
        if self.gid > 0 {
            cmd_line.set_gid(self.gid);
        }
        if !timeout.is_zero() {
            cmd_line.set_timeout(timeout);
        }

        // If we were interrupted, it is possible that "docker run" gets killed in exactly the
        // moment between the create and the start call, leaving behind a container that is
        // created but never ran. This means that Docker won't automatically clean it up (as --rm
        // only affects the start phase and has no effect on the create phase of "docker run").
        // We register the container UUID for cleanup, but remove the UUID if the process ran
        // successfully.
        self.containers_to_cleanup.lock().insert(uuid.clone());
        let containers_to_cleanup = self.containers_to_cleanup.clone();
        Ok(Box::new(CopyingSandboxedSpawn::new(
            sandbox_path,
            sandbox_exec_root,
            cmd_line.build(),
            self.client_env.clone(),
            inputs,
            outputs,
            IndexSet::new(),
            self.tree_deleter.clone(),
            /* sandbox_debug_path= */ None,
            /* statistics_path= */ None,
            Box::new(move || {
                containers_to_cleanup.lock().shift_remove(&uuid);
            }),
            &spawn.get_mnemonic(),
        )))
    }

    // Java: com/google/devtools/build/lib/sandbox/DockerSandboxedSpawnRunner.java:307 DockerSandboxedSpawnRunner.getOrCreateCustomizedImage
    fn get_or_create_customized_image(
        &self,
        base_image: &str,
    ) -> Result<String, SandboxRunnerError> {
        // TODO(philwo) docker run implicitly does a docker pull if the image does not exist
        // locally. Pulling an image can take a long time and a user might not be aware of that.
        // We could check if the image exists locally (docker images -q name:tag) and if not, do a
        // docker pull and notify the user in a similar way as when we download a http_archive.
        //
        // This is mostly relevant for the case where we don't build a customized image, as that
        // prints a message when it runs.

        if !self.use_customized_images {
            return Ok(base_image.to_string());
        }

        // If we're running as root, we can skip this step, as it's safe to assume that every
        // image already has a built-in root user and group.
        if self.uid == 0 && self.gid == 0 {
            return Ok(base_image.to_string());
        }

        // We only need to create a customized image, if we're running on Linux, as Docker on
        // macOS and Windows doesn't map users from the host into the container anyway.
        // (OS.getCurrent() == OS.LINUX.)

        let mut image_map = IMAGE_MAP.lock();
        if let Some(result) = image_map.get(base_image) {
            return Ok(result.clone());
        }
        let image = base_image;
        self.reporter.handle(&Event::info(&format!(
            "Preparing Docker image {image} for use..."
        )));
        let work_dir = PathFragment::create("/execroot")
            .get_relative_str(self.exec_root.get_base_name())
            .get_path_string()
            .to_string();
        let mut dockerfile = String::new();
        dockerfile.push_str("ARG image\n");
        dockerfile.push_str("FROM $image\n");
        dockerfile.push_str("ARG work_dir\n");
        dockerfile.push_str("RUN mkdir --parents $work_dir\n"); // could this be a VOLUME?
        dockerfile.push_str("ARG group_name\n");
        dockerfile.push_str("ARG gid\n");
        dockerfile.push_str("RUN groupadd --non-unique --gid $gid $group_name\n");
        dockerfile.push_str("ARG user_name\n");
        dockerfile.push_str("ARG uid\n");
        dockerfile.push_str(
            "RUN useradd --non-unique --no-log-init --create-home --gid $gid --home-dir $work_dir --no-user-group --uid $uid $user_name\n",
        ); // we've already created home above?
        // if we create home with useradd it'd already have the right ownership
        dockerfile.push_str("RUN chown --recursive $uid:$gid $work_dir\n");
        dockerfile.push_str("USER $user_name:$group_name\n");
        dockerfile.push_str("ENV HOME $work_dir\n");
        dockerfile.push_str("ENV USER $user_name\n");
        dockerfile.push_str("WORKDIR $work_dir\n");
        let result = self.execute_command(
            vec![
                self.docker_client.get_path_string().to_string(),
                "build".to_string(),
                "--build-arg".to_string(),
                format!("image={image}"),
                "--build-arg".to_string(),
                format!("work_dir={work_dir}"),
                "--build-arg".to_string(),
                format!("group_name={}", "bazelbuild"),
                "--build-arg".to_string(),
                format!("gid={}", self.gid),
                "--build-arg".to_string(),
                format!("user_name={}", "bazelbuild"),
                "--build-arg".to_string(),
                format!("uid={}", self.uid),
                "-q".to_string(),
                "-".to_string(),
            ],
            // Charset.defaultCharset(): UTF-8.
            dockerfile.as_bytes(),
        )?;
        image_map.insert(base_image.to_string(), result.clone());
        Ok(result)
    }

    // Java: com/google/devtools/build/lib/sandbox/DockerSandboxedSpawnRunner.java:388 DockerSandboxedSpawnRunner.executeCommand
    fn execute_command(
        &self,
        cmd_line: Vec<String>,
        std_in: &[u8],
    ) -> Result<String, SandboxRunnerError> {
        // Docker might need the $HOME and $PATH variables in order to be able to use advanced
        // authentication mechanisms (e.g. for Google Cloud), thus we pass in the client env.
        let cmd = Command::with_env(
            cmd_line,
            Some(self.client_env.clone()),
            Some(JavaFile::new(self.exec_root.get_path_string())),
            &self.client_env,
        );
        let mut std_in = std_in;
        let result = cmd
            .execute_async_with_input(&mut std_in, Command::KILL_SUBPROCESS_ON_INTERRUPT)
            .map_err(blaze_shell::CommandExecutionError::from)
            .and_then(|mut f| f.get());
        match result {
            // ByteArrayOutputStream.toString() (UTF-8), then String.trim(): chars <= ' '.
            Ok(r) => Ok(String::from_utf8_lossy(&r.get_stdout())
                .trim_matches(|c: char| c <= ' ')
                .to_string()),
            Err(e) => match e.as_command_exception() {
                None => Err(ExecException::Interrupted.into()),
                Some(command_exception) => {
                    // `stdErr` is the accumulated stderr (ByteArrayOutputStream.toString()).
                    let std_err = match command_exception {
                        blaze_shell::CommandException::AbnormalTermination(a) => {
                            String::from_utf8_lossy(&a.get_result().get_stderr()).into_owned()
                        }
                        blaze_shell::CommandException::BadExitStatus(b) => {
                            String::from_utf8_lossy(&b.get_result().get_stderr()).into_owned()
                        }
                        // The process did not start: nothing was written to stderr.
                        blaze_shell::CommandException::ExecFailed(_) => String::new(),
                    };
                    let message = format!(
                        "Running command {} failed: {std_err}",
                        cmd.to_debug_string()
                    );
                    Err(
                        UserExecException::new(sandbox_helpers::create_failure_detail(
                            &message,
                            sandbox::Code::DockerCommandFailure,
                        ))
                        .into(),
                    )
                }
            },
        }
    }

    // Java: com/google/devtools/build/lib/sandbox/DockerSandboxedSpawnRunner.java:404 DockerSandboxedSpawnRunner.dockerContainerFromSpawn
    fn docker_container_from_spawn(
        &self,
        spawn: &dyn Spawn,
    ) -> Result<Option<String>, ExecException> {
        let platform =
            crate::exec::spawn_log_context::get_platform_proto(spawn, Some(&self.remote_options))?;

        let Some(properties) = platform else {
            return Ok(None);
        };
        let images: Vec<String> = properties
            .iter()
            .filter(|(name, _)| name == CONTAINER_IMAGE_ENTRY_NAME)
            .map(|(_, value)| value)
            .filter(|r| r.starts_with(DOCKER_IMAGE_PREFIX))
            .map(|r| r[DOCKER_IMAGE_PREFIX.len()..].to_string())
            .collect();
        if images.len() > 1 {
            // MoreCollectors.toOptional() throws IllegalArgumentException, rethrown unchecked.
            panic!(
                "java.lang.IllegalArgumentException: Platform {} contained multiple container-image entries, but only one is allowed.",
                spawn
                    .get_execution_platform()
                    .map_or("null".to_string(), |p| p.label.to_string())
            );
        }
        Ok(images.into_iter().next())
    }

    /// Remove all Docker containers that might be stuck in "Created" state and weren't
    /// automatically cleaned up by Docker itself.
    // Java: com/google/devtools/build/lib/sandbox/DockerSandboxedSpawnRunner.java:430 DockerSandboxedSpawnRunner.cleanup
    pub fn cleanup(&self) -> Result<(), ExecException> {
        let mut containers_to_cleanup = self.containers_to_cleanup.lock();
        if containers_to_cleanup.is_empty() {
            return Ok(());
        }

        let mut cmd_line = vec![
            self.docker_client.get_path_string().to_string(),
            "rm".to_string(),
            "-fv".to_string(),
        ];
        // PORT: Java iterates a HashSet of UUIDs (hash order); here the registration order.
        cmd_line.extend(containers_to_cleanup.iter().cloned());

        let cmd = Command::with_env(
            cmd_line,
            Some(self.client_env.clone()),
            Some(JavaFile::new(self.exec_root.get_path_string())),
            &self.client_env,
        );

        if let Err(e) = cmd.execute()
            && e.as_command_exception().is_none()
        {
            return Err(ExecException::Interrupted);
        }
        // A CommandException is to be expected, as not all UUIDs that we pass to "docker rm"
        // will still be alive when this method is called. However, it will successfully remove
        // all the containers that *are* still there, even when the command exits with an error.

        containers_to_cleanup.clear();
        Ok(())
    }

    /// `@Subscribe commandComplete(CommandCompleteEvent)`.
    // Java: com/google/devtools/build/lib/sandbox/DockerSandboxedSpawnRunner.java:461 DockerSandboxedSpawnRunner.commandComplete
    pub fn command_complete(&self) {
        if self.cleanup().is_err() {
            self.reporter.handle(&Event::error(
                "Interrupted while cleaning up docker sandbox",
            ));
            thread_interrupt::interrupt_current();
        }
    }
}

impl SandboxSpawnRunnerHooks for DockerSandboxedSpawnRunner {
    fn runner_name(&self) -> String {
        self.get_name()
    }

    fn prepare_spawn(
        &self,
        spawn: &dyn Spawn,
        context: &dyn SpawnExecutionContext,
    ) -> Result<Box<dyn SandboxedSpawn>, SandboxRunnerError> {
        DockerSandboxedSpawnRunner::prepare_spawn(self, spawn, context)
    }
}

impl SpawnRunner for DockerSandboxedSpawnRunner {
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

    // Java: com/google/devtools/build/lib/sandbox/DockerSandboxedSpawnRunner.java:471 DockerSandboxedSpawnRunner.getName
    fn get_name(&self) -> String {
        "docker".to_string()
    }

    fn cleanup_sandbox_base(
        &self,
        sandbox_base: &Path,
        tree_deleter: &dyn TreeDeleter,
    ) -> io::Result<()> {
        self.base
            .cleanup_sandbox_base(&self.get_name(), sandbox_base, tree_deleter)
    }
}
