//! Port of `lib/remote/RemoteRepositoryRemoteExecutor.java`, with the interface it implements,
//! `lib/runtime/RepositoryRemoteExecutor.java`.

use std::collections::{BTreeMap, HashSet};
use std::io;
use std::sync::Arc;
use std::time::Duration;

use blaze_optdefs::fragment::fragment_options::java_string_compare;
use blaze_protos::JavaMessage;
use blaze_vfs::{Path, PathFragment};
use indexmap::IndexMap;

use crate::combined_cache::CachedActionResult;
use crate::common::{
    ActionKey, NoOpOperationObserver, RemoteActionExecutionContext, RemoteExecutionClient,
};
use crate::merkletree::MerkleTreeComputer;
use crate::platform_utils;
use crate::reapi::{ActionResult, Command, Digest, ExecuteRequest, ExecuteResponse, command};
use crate::remote_execution_cache::RemoteExecutionCache;
use crate::runtime::get_from_future;
use crate::util::DigestUtil;
use crate::util::tracing_metadata_utils;
use crate::util::utils::build_action;

/// The result of a remotely executed command.
///
/// PORT: `RepositoryRemoteExecutor` lives in `lib/runtime`, which has no Rust crate that this
/// one may depend on; the interface and its result are defined here until it does.
// Java: com/google/devtools/build/lib/runtime/RepositoryRemoteExecutor.java:28 RepositoryRemoteExecutor.ExecutionResult
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutionResult {
    exit_code: i32,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

impl ExecutionResult {
    // Java: com/google/devtools/build/lib/runtime/RepositoryRemoteExecutor.java:34 RepositoryRemoteExecutor.ExecutionResult.<init>
    pub fn new(exit_code: i32, stdout: Vec<u8>, stderr: Vec<u8>) -> ExecutionResult {
        ExecutionResult {
            exit_code,
            stdout,
            stderr,
        }
    }

    // Java: com/google/devtools/build/lib/runtime/RepositoryRemoteExecutor.java:40 RepositoryRemoteExecutor.ExecutionResult.exitCode
    pub fn exit_code(&self) -> i32 {
        self.exit_code
    }

    // Java: com/google/devtools/build/lib/runtime/RepositoryRemoteExecutor.java:44 RepositoryRemoteExecutor.ExecutionResult.stdout
    pub fn stdout(&self) -> &[u8] {
        &self.stdout
    }

    // Java: com/google/devtools/build/lib/runtime/RepositoryRemoteExecutor.java:48 RepositoryRemoteExecutor.ExecutionResult.stderr
    pub fn stderr(&self) -> &[u8] {
        &self.stderr
    }
}

/// Interface to support remote execution in repository_ctx.execute().
///
/// PORT: see [`ExecutionResult`] for why the interface is defined here.
// Java: com/google/devtools/build/lib/runtime/RepositoryRemoteExecutor.java:25 RepositoryRemoteExecutor
pub trait RepositoryRemoteExecutor: Send + Sync {
    /// Execute a command remotely.
    ///
    /// * `arguments`: the command arguments.
    /// * `input_files`: the files to upload and stage for the command. The key describes where to
    ///   stage the file on the remote machine. The value is the path of the file on the host
    ///   machine (where Bazel is running).
    /// * `execution_properties`: the remote platform the command should run on.
    /// * `environment`: any environment variables that should be set in the command's
    ///   environment.
    /// * `working_directory`: the working directory to run the command under. `""` means that
    ///   the remote system should choose. (PORT: `None` is Java's `null`.)
    /// * `timeout`: execution timeout.
    // Java: com/google/devtools/build/lib/runtime/RepositoryRemoteExecutor.java:66 RepositoryRemoteExecutor.execute
    fn execute(
        &self,
        arguments: &[String],
        input_files: &BTreeMap<PathFragment, Path>,
        execution_properties: &IndexMap<String, String>,
        environment: &IndexMap<String, String>,
        working_directory: Option<&str>,
        timeout: Duration,
    ) -> io::Result<ExecutionResult>;
}

/// The remote package's implementation of [`RepositoryRemoteExecutor`].
// Java: com/google/devtools/build/lib/remote/RemoteRepositoryRemoteExecutor.java:54 RemoteRepositoryRemoteExecutor
pub struct RemoteRepositoryRemoteExecutor {
    remote_cache: Arc<RemoteExecutionCache>,
    remote_executor: Arc<dyn RemoteExecutionClient>,
    digest_util: DigestUtil,
    build_request_id: String,
    command_id: String,
    workspace_name: String,

    remote_instance_name: String,
    accept_cached: bool,

    /// PORT: `MerkleTreeComputer::new` takes the exec root that stands for the artifact path
    /// resolver (missing hook: SpawnExecutionContext.getPathResolver); Java's constructor has
    /// no such parameter. `buildForFiles` resolves its inputs by their own paths.
    exec_root: Path,
}

impl RemoteRepositoryRemoteExecutor {
    // Java: com/google/devtools/build/lib/remote/RemoteRepositoryRemoteExecutor.java:65 RemoteRepositoryRemoteExecutor.<init>
    pub fn new(
        remote_cache: Arc<RemoteExecutionCache>,
        remote_executor: Arc<dyn RemoteExecutionClient>,
        digest_util: DigestUtil,
        build_request_id: &str,
        command_id: &str,
        workspace_name: &str,
        remote_instance_name: &str,
        accept_cached: bool,
        exec_root: Path,
    ) -> RemoteRepositoryRemoteExecutor {
        RemoteRepositoryRemoteExecutor {
            remote_cache,
            remote_executor,
            digest_util,
            build_request_id: build_request_id.to_string(),
            command_id: command_id.to_string(),
            workspace_name: workspace_name.to_string(),
            remote_instance_name: remote_instance_name.to_string(),
            accept_cached,
            exec_root,
        }
    }

    // Java: com/google/devtools/build/lib/remote/RemoteRepositoryRemoteExecutor.java:83 RemoteRepositoryRemoteExecutor.downloadOutErr
    fn download_out_err(
        &self,
        context: &RemoteActionExecutionContext<'_>,
        result: &ActionResult,
    ) -> io::Result<ExecutionResult> {
        // PORT: skipped, profiling ("download stdout/stderr").
        let mut stdout: Vec<u8> = Vec::new();
        if !result.stdout_raw.is_empty() {
            stdout = result.stdout_raw.to_vec();
        } else if let Some(stdout_digest) = &result.stdout_digest {
            stdout = get_from_future(self.remote_cache.download_blob_named(
                context,
                "<stdout>",
                /* exec_path= */ None,
                stdout_digest,
            ))?;
        }

        let mut stderr: Vec<u8> = Vec::new();
        if !result.stderr_raw.is_empty() {
            stderr = result.stderr_raw.to_vec();
        } else if let Some(stderr_digest) = &result.stderr_digest {
            stderr = get_from_future(self.remote_cache.download_blob_named(
                context,
                "<stderr>",
                /* exec_path= */ None,
                stderr_digest,
            ))?;
        }

        Ok(ExecutionResult::new(result.exit_code, stdout, stderr))
    }
}

impl RepositoryRemoteExecutor for RemoteRepositoryRemoteExecutor {
    // Java: com/google/devtools/build/lib/remote/RemoteRepositoryRemoteExecutor.java:110 RemoteRepositoryRemoteExecutor.execute
    fn execute(
        &self,
        arguments: &[String],
        input_files: &BTreeMap<PathFragment, Path>,
        execution_properties: &IndexMap<String, String>,
        environment: &IndexMap<String, String>,
        working_directory: Option<&str>,
        timeout: Duration,
    ) -> io::Result<ExecutionResult> {
        let metadata = tracing_metadata_utils::build_metadata(
            &self.build_request_id,
            &self.command_id,
            "repository_rule",
            None,
        );
        let context = RemoteActionExecutionContext::create(metadata);

        let platform = platform_utils::build_platform_proto(execution_properties);

        let mut command_builder = Command {
            arguments: arguments.to_vec(),
            ..Default::default()
        };
        // Sorting the environment pairs by variable name.
        let mut variables: Vec<&String> = environment.keys().collect();
        variables.sort_by(|a, b| java_string_compare(a, b));
        for var in variables {
            command_builder
                .environment_variables
                .push(command::EnvironmentVariable {
                    name: var.clone(),
                    value: environment[var].clone(),
                });
        }
        if let Some(platform) = &platform {
            command_builder.platform = Some(platform.clone());
        }
        if let Some(working_directory) = working_directory {
            command_builder.working_directory = working_directory.to_string();
        }

        let command = command_builder;
        let command_hash = self.digest_util.compute_message(&command);
        let merkle_tree = MerkleTreeComputer::new(
            self.digest_util.clone(),
            /* remote_execution_cache= */ None,
            &self.build_request_id,
            &self.command_id,
            &self.workspace_name,
            self.exec_root.clone(),
        )
        .build_for_files(input_files)?;
        let action = build_action(
            command_hash.clone(),
            merkle_tree.digest().clone(),
            platform,
            timeout,
            self.accept_cached,
            /* salt= */ None,
        );
        let action_digest = self.digest_util.compute_message(&action);
        let action_key = ActionKey::new(action_digest.clone());
        // PORT: skipped, profiling ("check cache hit").
        let cached_action_result: Option<CachedActionResult> =
            self.remote_cache.download_action_result(
                &context,
                &action_key,
                /* inline_out_err= */ true,
                /* inline_output_files= */ &HashSet::new(),
            )?;
        let mut action_result: Option<ActionResult> = None;
        if let Some(cached_action_result) = &cached_action_result {
            action_result = Some(cached_action_result.action_result().clone());
        }
        if action_result
            .as_ref()
            .is_none_or(|action_result| action_result.exit_code != 0)
        {
            {
                // PORT: skipped, profiling ("upload missing inputs").
                // PORT: java-hash-order. Java fills a `HashMap<Digest, Message>`; its order only
                // decides the order of the digests in the FindMissingBlobs request, and the
                // insertion order is used here.
                let mut additional_inputs: IndexMap<Digest, bytes::Bytes> =
                    IndexMap::with_capacity(2);
                additional_inputs.insert(
                    action_digest.clone(),
                    bytes::Bytes::from(action.to_byte_array()),
                );
                additional_inputs.insert(
                    command_hash.clone(),
                    bytes::Bytes::from(command.to_byte_array()),
                );

                self.remote_cache
                    .ensure_inputs_present_with_additional_inputs(
                        &context,
                        &merkle_tree,
                        &additional_inputs,
                        /* force= */ true,
                        /* remote_path_resolver= */ None,
                    )?;
            }

            {
                // PORT: skipped, profiling ("execute remotely").
                let execute_request = ExecuteRequest {
                    action_digest: Some(action_digest),
                    instance_name: self.remote_instance_name.clone(),
                    digest_function: self.digest_util.get_digest_function() as i32,
                    skip_cache_lookup: !self.accept_cached,
                    ..Default::default()
                };

                let response: ExecuteResponse = self.remote_executor.execute_remotely(
                    &context,
                    execute_request,
                    &NoOpOperationObserver,
                )?;
                // `response.getResult()` is the default instance when the field is not set.
                action_result = Some(response.result.unwrap_or_default());
            }
        }
        self.download_out_err(&context, action_result.as_ref().expect("actionResult"))
    }
}
