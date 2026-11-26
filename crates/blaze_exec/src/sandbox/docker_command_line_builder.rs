//! Port of `lib/sandbox/DockerCommandLineBuilder.java`.

use std::time::Duration;

use blaze_vfs::{Path, PathFragment};
use indexmap::IndexMap;

use crate::runtime::process_wrapper::ProcessWrapper;

// Java: com/google/devtools/build/lib/sandbox/DockerCommandLineBuilder.java:32 DockerCommandLineBuilder
#[derive(Default)]
pub struct DockerCommandLineBuilder {
    process_wrapper: Option<ProcessWrapper>,
    docker_client: Option<Path>,
    image_name: Option<String>,
    command_arguments: Option<Vec<String>>,
    sandbox_exec_root: Option<Path>,
    environment_variables: Option<IndexMap<String, String>>,
    timeout: Option<Duration>,
    create_network_namespace: bool,
    /// `UUID.toString()`.
    uuid: Option<String>,
    uid: i32,
    gid: i32,
    command_id: Option<String>,
    privileged: bool,
    additional_mounts: Option<Vec<(String, String)>>,
}

impl DockerCommandLineBuilder {
    pub fn new() -> DockerCommandLineBuilder {
        DockerCommandLineBuilder::default()
    }

    // Java: com/google/devtools/build/lib/sandbox/DockerCommandLineBuilder.java:47 DockerCommandLineBuilder.setProcessWrapper
    pub fn set_process_wrapper(&mut self, process_wrapper: ProcessWrapper) -> &mut Self {
        self.process_wrapper = Some(process_wrapper);
        self
    }

    // Java: com/google/devtools/build/lib/sandbox/DockerCommandLineBuilder.java:53 DockerCommandLineBuilder.setDockerClient
    pub fn set_docker_client(&mut self, docker_client: Path) -> &mut Self {
        self.docker_client = Some(docker_client);
        self
    }

    // Java: com/google/devtools/build/lib/sandbox/DockerCommandLineBuilder.java:59 DockerCommandLineBuilder.setImageName
    pub fn set_image_name(&mut self, image_name: String) -> &mut Self {
        self.image_name = Some(image_name);
        self
    }

    // Java: com/google/devtools/build/lib/sandbox/DockerCommandLineBuilder.java:65 DockerCommandLineBuilder.setCommandArguments
    pub fn set_command_arguments(&mut self, command_arguments: Vec<String>) -> &mut Self {
        self.command_arguments = Some(command_arguments);
        self
    }

    // Java: com/google/devtools/build/lib/sandbox/DockerCommandLineBuilder.java:71 DockerCommandLineBuilder.setSandboxExecRoot
    pub fn set_sandbox_exec_root(&mut self, sandbox_exec_root: Path) -> &mut Self {
        self.sandbox_exec_root = Some(sandbox_exec_root);
        self
    }

    // Java: com/google/devtools/build/lib/sandbox/DockerCommandLineBuilder.java:77 DockerCommandLineBuilder.setEnvironmentVariables
    pub fn set_environment_variables(
        &mut self,
        environment_variables: IndexMap<String, String>,
    ) -> &mut Self {
        self.environment_variables = Some(environment_variables);
        self
    }

    // Java: com/google/devtools/build/lib/sandbox/DockerCommandLineBuilder.java:84 DockerCommandLineBuilder.setTimeout
    pub fn set_timeout(&mut self, timeout: Duration) -> &mut Self {
        self.timeout = Some(timeout);
        self
    }

    // Java: com/google/devtools/build/lib/sandbox/DockerCommandLineBuilder.java:90 DockerCommandLineBuilder.setCreateNetworkNamespace
    pub fn set_create_network_namespace(&mut self, create_network_namespace: bool) -> &mut Self {
        self.create_network_namespace = create_network_namespace;
        self
    }

    // Java: com/google/devtools/build/lib/sandbox/DockerCommandLineBuilder.java:96 DockerCommandLineBuilder.setUuid
    pub fn set_uuid(&mut self, uuid: String) -> &mut Self {
        self.uuid = Some(uuid);
        self
    }

    // Java: com/google/devtools/build/lib/sandbox/DockerCommandLineBuilder.java:102 DockerCommandLineBuilder.setUid
    pub fn set_uid(&mut self, uid: i32) -> &mut Self {
        self.uid = uid;
        self
    }

    // Java: com/google/devtools/build/lib/sandbox/DockerCommandLineBuilder.java:108 DockerCommandLineBuilder.setGid
    pub fn set_gid(&mut self, gid: i32) -> &mut Self {
        self.gid = gid;
        self
    }

    // Java: com/google/devtools/build/lib/sandbox/DockerCommandLineBuilder.java:114 DockerCommandLineBuilder.setCommandId
    pub fn set_command_id(&mut self, command_id: String) -> &mut Self {
        self.command_id = Some(command_id);
        self
    }

    // Java: com/google/devtools/build/lib/sandbox/DockerCommandLineBuilder.java:120 DockerCommandLineBuilder.setPrivileged
    pub fn set_privileged(&mut self, privileged: bool) -> &mut Self {
        self.privileged = privileged;
        self
    }

    // Java: com/google/devtools/build/lib/sandbox/DockerCommandLineBuilder.java:126 DockerCommandLineBuilder.setAdditionalMounts
    pub fn set_additional_mounts(&mut self, additional_mounts: Vec<(String, String)>) -> &mut Self {
        self.additional_mounts = Some(additional_mounts);
        self
    }

    // Java: com/google/devtools/build/lib/sandbox/DockerCommandLineBuilder.java:132 DockerCommandLineBuilder.build
    pub fn build(&self) -> Vec<String> {
        let sandbox_exec_root = self
            .sandbox_exec_root
            .as_ref()
            .expect("NullPointerException: sandboxExecRoot must be set");
        let image_name = self
            .image_name
            .as_ref()
            .expect("NullPointerException: imageName");
        blaze_util::check_state!(!image_name.is_empty(), "imageName must be set");
        let command_arguments = self
            .command_arguments
            .as_ref()
            .expect("NullPointerException: commandArguments");
        blaze_util::check_state!(
            !command_arguments.is_empty(),
            "commandArguments must be set"
        );

        let mut docker_cmd_line: Vec<String> = Vec::new();

        docker_cmd_line.push(
            self.docker_client
                .as_ref()
                .expect("NullPointerException: dockerClient")
                .get_path_string()
                .to_string(),
        );
        docker_cmd_line.push("run".to_string());
        docker_cmd_line.push("--rm".to_string());
        if self.create_network_namespace {
            docker_cmd_line.push("--network=none".to_string());
        } else {
            docker_cmd_line.push("--network=host".to_string());
        }
        if self.privileged {
            docker_cmd_line.push("--privileged".to_string());
        }
        for (k, v) in self
            .environment_variables
            .as_ref()
            .expect("NullPointerException: environmentVariables")
        {
            docker_cmd_line.push("-e".to_string());
            docker_cmd_line.push(format!("{k}={v}"));
        }
        let exec_root_inside_docker =
            PathFragment::create("/execroot/").get_relative_str(sandbox_exec_root.get_base_name());
        docker_cmd_line.push("-v".to_string());
        docker_cmd_line.push(format!(
            "{}:{}",
            sandbox_exec_root.get_path_string(),
            exec_root_inside_docker.get_path_string()
        ));
        docker_cmd_line.push("-w".to_string());
        docker_cmd_line.push(exec_root_inside_docker.get_path_string().to_string());

        for (mount_source, mount_target) in self
            .additional_mounts
            .as_ref()
            .expect("NullPointerException: additionalMounts")
        {
            docker_cmd_line.push("-v".to_string());
            docker_cmd_line.push(format!("{mount_source}:{mount_target}"));
        }

        let mut uid_gid_flag = String::new();
        if self.uid != 0 {
            uid_gid_flag.push_str(&self.uid.to_string());
        }
        if self.gid != 0 {
            uid_gid_flag.push(':');
            uid_gid_flag.push_str(&self.gid.to_string());
        }
        if !uid_gid_flag.is_empty() {
            docker_cmd_line.push("-u".to_string());
            docker_cmd_line.push(uid_gid_flag);
        }

        let command_id = self
            .command_id
            .as_ref()
            .expect("NullPointerException: commandId");
        if !command_id.is_empty() {
            docker_cmd_line.push("-l".to_string());
            docker_cmd_line.push(format!("command_id={command_id}"));
        }
        if let Some(uuid) = &self.uuid {
            docker_cmd_line.push("--name".to_string());
            docker_cmd_line.push(uuid.clone());
        }
        docker_cmd_line.push(image_name.clone());
        docker_cmd_line.extend(command_arguments.iter().cloned());

        let mut process_wrapper_cmd_line = self
            .process_wrapper
            .as_ref()
            .expect("NullPointerException: processWrapper")
            .command_line_builder(docker_cmd_line);
        if let Some(timeout) = self.timeout {
            process_wrapper_cmd_line.set_timeout(timeout);
        }
        process_wrapper_cmd_line.build()
    }
}
