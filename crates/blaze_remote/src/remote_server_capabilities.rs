//! Port of `lib/remote/RemoteServerCapabilities.java`.

use std::io;
use std::sync::Arc;

use blaze_optdefs::misc::remote_options::{ChunkingFunctionValue, RemoteOptions};

use crate::client_api_version::ClientApiVersion;
use crate::common::RemoteActionExecutionContext;
use crate::grpc::channel::io_result;
use crate::grpc::{CallCredentials, CallOptions, ReferenceCountedChannel, methods};
use crate::reapi::{
    GetCapabilitiesRequest, PriorityCapabilities, ServerCapabilities, compressor, digest_function,
};
use crate::remote_retrier::RemoteRetrier;
use crate::util::tracing_metadata_utils;

/// Fetches the ServerCapabilities of the remote execution/cache server.
// Java: com/google/devtools/build/lib/remote/RemoteServerCapabilities.java:40 RemoteServerCapabilities
pub struct RemoteServerCapabilities {
    build_request_id: String,
    command_id: String,
    instance_name: Option<String>,
    call_credentials: Option<Arc<dyn CallCredentials>>,
    retrier: Arc<RemoteRetrier>,
}

impl RemoteServerCapabilities {
    // Java: com/google/devtools/build/lib/remote/RemoteServerCapabilities.java:47 RemoteServerCapabilities.<init>
    pub fn new(
        build_request_id: &str,
        command_id: &str,
        instance_name: Option<&str>,
        call_credentials: Option<Arc<dyn CallCredentials>>,
        retrier: Arc<RemoteRetrier>,
    ) -> RemoteServerCapabilities {
        RemoteServerCapabilities {
            build_request_id: build_request_id.to_string(),
            command_id: command_id.to_string(),
            instance_name: instance_name.map(str::to_string),
            call_credentials,
            retrier,
        }
    }

    /// The decoration of `CapabilitiesGrpc.newFutureStub(channel)`: the request metadata and the
    /// call credentials (no network time interceptor, as in Java).
    // Java: com/google/devtools/build/lib/remote/RemoteServerCapabilities.java:60 RemoteServerCapabilities.capabilitiesFutureStub
    fn capabilities_future_stub(&self, context: &RemoteActionExecutionContext<'_>) -> CallOptions {
        CallOptions::new()
            .with_request_metadata(context.get_request_metadata().clone())
            .with_call_credentials(self.call_credentials.clone())
    }

    /// Fetches the capabilities of the server behind `channel`.
    ///
    /// PORT: Java takes the `ManagedChannel` of one connection; the port's connection is the
    /// [`ReferenceCountedChannel`] itself (crate::grpc::channel).
    // Java: com/google/devtools/build/lib/remote/RemoteServerCapabilities.java:68 RemoteServerCapabilities.get
    pub async fn get(&self, channel: &ReferenceCountedChannel) -> io::Result<ServerCapabilities> {
        let metadata = tracing_metadata_utils::build_metadata(
            &self.build_request_id,
            &self.command_id,
            "capabilities",
            None,
        );
        let context = RemoteActionExecutionContext::create(metadata);
        let request = match &self.instance_name {
            None => GetCapabilitiesRequest::default(),
            Some(instance_name) => GetCapabilitiesRequest {
                instance_name: instance_name.clone(),
            },
        };
        self.retrier
            .execute_async(|| async {
                io_result(
                    channel
                        .unary::<GetCapabilitiesRequest, ServerCapabilities>(
                            methods::GET_CAPABILITIES,
                            request.clone(),
                            self.capabilities_future_stub(&context),
                        )
                        .await,
                )
            })
            .await
    }
}

/// The result of [`check_client_server_compatibility`].
// Java: com/google/devtools/build/lib/remote/RemoteServerCapabilities.java:80 RemoteServerCapabilities.ClientServerCompatibilityStatus
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientServerCompatibilityStatus {
    warnings: Vec<String>,
    errors: Vec<String>,
}

impl ClientServerCompatibilityStatus {
    // Java: com/google/devtools/build/lib/remote/RemoteServerCapabilities.java:107 ClientServerCompatibilityStatus.isOk
    pub fn is_ok(&self) -> bool {
        self.warnings.is_empty() && self.errors.is_empty()
    }

    // Java: com/google/devtools/build/lib/remote/RemoteServerCapabilities.java:111 ClientServerCompatibilityStatus.getWarnings
    pub fn get_warnings(&self) -> &[String] {
        &self.warnings
    }

    // Java: com/google/devtools/build/lib/remote/RemoteServerCapabilities.java:115 ClientServerCompatibilityStatus.getErrors
    pub fn get_errors(&self) -> &[String] {
        &self.errors
    }
}

/// `ClientServerCompatibilityStatus.Builder`.
// Java: com/google/devtools/build/lib/remote/RemoteServerCapabilities.java:90 ClientServerCompatibilityStatus.Builder
#[derive(Debug, Default)]
pub struct ClientServerCompatibilityStatusBuilder {
    warnings: Vec<String>,
    errors: Vec<String>,
}

impl ClientServerCompatibilityStatusBuilder {
    /// `new ClientServerCompatibilityStatus.Builder()`.
    pub fn new() -> ClientServerCompatibilityStatusBuilder {
        ClientServerCompatibilityStatusBuilder::default()
    }

    // Java: com/google/devtools/build/lib/remote/RemoteServerCapabilities.java:94 ClientServerCompatibilityStatus.Builder.addWarning
    pub fn add_warning(&mut self, message: impl Into<String>) {
        self.warnings.push(message.into());
    }

    // Java: com/google/devtools/build/lib/remote/RemoteServerCapabilities.java:98 ClientServerCompatibilityStatus.Builder.addError
    pub fn add_error(&mut self, message: impl Into<String>) {
        self.errors.push(message.into());
    }

    // Java: com/google/devtools/build/lib/remote/RemoteServerCapabilities.java:102 ClientServerCompatibilityStatus.Builder.build
    pub fn build(self) -> ClientServerCompatibilityStatus {
        ClientServerCompatibilityStatus {
            warnings: self.warnings,
            errors: self.errors,
        }
    }
}

// Java: com/google/devtools/build/lib/remote/RemoteServerCapabilities.java:120 RemoteServerCapabilities.checkPriorityInRange
fn check_priority_in_range(
    priority: i32,
    option_name: &str,
    pr_cap: &PriorityCapabilities,
    result: &mut ClientServerCompatibilityStatusBuilder,
) {
    if priority != 0 {
        let mut found = false;
        let mut range_builder = String::new();
        for pr in &pr_cap.priorities {
            range_builder.push_str(&format!("{}-{},", pr.min_priority, pr.max_priority));
            if pr.min_priority <= priority && priority <= pr.max_priority {
                found = true;
                break;
            }
        }
        if !found {
            let mut range = range_builder;
            if !range.is_empty() {
                range.truncate(range.len() - 1);
            }
            result.add_error(format!(
                "--{option_name} {priority} is outside of server supported range {range}."
            ));
        }
    }
}

/// Which capabilities of the server the current build needs.
// Java: com/google/devtools/build/lib/remote/RemoteServerCapabilities.java:147 RemoteServerCapabilities.ServerCapabilitiesRequirement
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ServerCapabilitiesRequirement {
    /// `NONE`.
    None,
    /// `CACHE`.
    Cache,
    /// `EXECUTION`.
    Execution,
    /// `EXECUTION_AND_CACHE`.
    ExecutionAndCache,
}

/// `String.valueOf(DigestFunction.Value)` for the number of a `DigestFunction.Value` field: the
/// name of the value, `UNRECOGNIZED` for a number this client does not know.
fn digest_function_name(value: i32) -> &'static str {
    match digest_function::Value::try_from(value) {
        Ok(value) => value.as_str_name(),
        Err(_) => "UNRECOGNIZED",
    }
}

/// `String.valueOf(List<DigestFunction.Value>)`: `[A, B]`.
fn digest_functions_list(values: &[i32]) -> String {
    let names: Vec<&str> = values.iter().map(|v| digest_function_name(*v)).collect();
    format!("[{}]", names.join(", "))
}

/// Compare the remote server capabilities with those requested by current execution.
// Java: com/google/devtools/build/lib/remote/RemoteServerCapabilities.java:155 RemoteServerCapabilities.checkClientServerCompatibility
pub fn check_client_server_compatibility(
    capabilities: &ServerCapabilities,
    remote_options: &RemoteOptions,
    digest_function: digest_function::Value,
    requirement: ServerCapabilitiesRequirement,
) -> ClientServerCompatibilityStatus {
    let mut result = ClientServerCompatibilityStatusBuilder::new();
    let should_check_execution_capabilities = requirement
        == ServerCapabilitiesRequirement::Execution
        || requirement == ServerCapabilitiesRequirement::ExecutionAndCache;
    let should_check_cache_capabilities = requirement == ServerCapabilitiesRequirement::Cache
        || requirement == ServerCapabilitiesRequirement::ExecutionAndCache;
    if !(should_check_cache_capabilities || should_check_execution_capabilities) {
        return result.build();
    }

    // Check API version.
    let st = ClientApiVersion::CURRENT.check_server_supported_versions(capabilities);
    if st.is_unsupported() {
        result.add_error(st.get_message());
    }
    if st.is_deprecated() {
        result.add_warning(st.get_message());
    }

    if should_check_execution_capabilities {
        // Check remote execution is enabled.
        // PORT: an unset message field reads as the default instance.
        let exec_cap = capabilities
            .execution_capabilities
            .clone()
            .unwrap_or_default();
        if !exec_cap.exec_enabled {
            result.add_error(
                "Remote execution is not supported by the remote server, or the current \
                 account is not authorized to use remote execution.",
            );
            return result.build(); // No point checking other execution fields.
        }

        // Check execution digest function. The protocol only later added
        // support for multiple digest functions for remote execution, so
        // check both the singular and repeated field.
        if exec_cap.digest_functions.is_empty()
            && exec_cap.digest_function != digest_function::Value::Unknown as i32
        {
            if exec_cap.digest_function != digest_function as i32 {
                result.add_error(format!(
                    "Cannot use hash function {} with remote execution. \
                     Server supported function is {}",
                    digest_function.as_str_name(),
                    digest_function_name(exec_cap.digest_function)
                ));
            }
        } else if !exec_cap
            .digest_functions
            .contains(&(digest_function as i32))
        {
            result.add_error(format!(
                "Cannot use hash function {} with remote execution. \
                 Server supported functions are: {}",
                digest_function.as_str_name(),
                digest_functions_list(&exec_cap.digest_functions)
            ));
        }

        // Check execution priority is in the supported range.
        check_priority_in_range(
            remote_options.remote_execution_priority,
            "remote_execution_priority",
            &exec_cap
                .execution_priority_capabilities
                .clone()
                .unwrap_or_default(),
            &mut result,
        );
    }

    if should_check_cache_capabilities {
        // Check cache digest function.
        let cache_cap = capabilities.cache_capabilities.clone().unwrap_or_default();
        if !cache_cap
            .digest_functions
            .contains(&(digest_function as i32))
        {
            result.add_error(format!(
                "Cannot use hash function {} with remote cache. \
                 Server supported functions are: {}",
                digest_function.as_str_name(),
                digest_functions_list(&cache_cap.digest_functions)
            ));
        }

        if remote_options.remote_upload_local_results
            && !cache_cap
                .action_cache_update_capabilities
                .clone()
                .unwrap_or_default()
                .update_enabled
        {
            result.add_warning(
                "--remote_upload_local_results is set, but the remote cache does not support uploading \
                 action results or the current account is not authorized to write local results \
                 to the remote cache.",
            );
        }

        if remote_options.cache_compression
            && !cache_cap
                .supported_compressors
                .contains(&(compressor::Value::Zstd as i32))
        {
            result.add_error(
                "--remote_cache_compression requested but remote does not support compression",
            );
        }

        if remote_options.experimental_remote_cache_chunking {
            if !cache_cap.split_blob_support {
                result.add_error(
                    "--experimental_remote_cache_chunking requested but remote does not support \
                     SplitBlob",
                );
            }
            if !cache_cap.splice_blob_support {
                result.add_error(
                    "--experimental_remote_cache_chunking requested but remote does not support \
                     SpliceBlob",
                );
            }
            match remote_options.experimental_remote_cache_chunking_function {
                ChunkingFunctionValue::AUTO => {
                    if cache_cap.fast_cdc_2020_params.is_none()
                        && cache_cap.rep_max_cdc_params.is_none()
                    {
                        result.add_error(
                            "--experimental_remote_cache_chunking requested but remote does not support \
                             any chunking algorithm supported by Bazel (FastCDC 2020 or RepMaxCDC)",
                        );
                    }
                }
                ChunkingFunctionValue::FAST_CDC_2020 => {
                    if cache_cap.fast_cdc_2020_params.is_none() {
                        result.add_error(
                            "--experimental_remote_cache_chunking requested but remote does not support \
                             FastCDC 2020 chunking algorithm",
                        );
                    }
                }
                ChunkingFunctionValue::REP_MAX_CDC => {
                    if cache_cap.rep_max_cdc_params.is_none() {
                        result.add_error(
                            "--experimental_remote_cache_chunking requested but remote does not support \
                             RepMaxCDC chunking algorithm",
                        );
                    }
                }
            }
        }

        // Check result cache priority is in the supported range.
        check_priority_in_range(
            remote_options.remote_result_cache_priority,
            "remote_result_cache_priority",
            &cache_cap
                .cache_priority_capabilities
                .clone()
                .unwrap_or_default(),
            &mut result,
        );
    }

    result.build()
}
