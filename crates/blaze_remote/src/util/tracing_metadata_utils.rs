//! Port of `lib/remote/util/TracingMetadataUtils.java` (the metadata construction; the client
//! interceptors live with the channel in `crate::grpc`).

use std::sync::OnceLock;

use blaze_exec::actions::ActionExecutionMetadata;

use crate::reapi::{RequestMetadata, ToolDetails};

static TOOL_VERSION: OnceLock<String> = OnceLock::new();

/// Sets `BlazeVersionInfo.instance().getVersion()` for this process (first call wins).
///
/// PORT: `BlazeVersionInfo` lives in `blaze_runtime`, above this crate; the remote module's
/// integrator passes the version in.
pub fn set_tool_version(version: &str) {
    let _ = TOOL_VERSION.set(version.to_string());
}

pub(crate) fn tool_version() -> &'static str {
    match TOOL_VERSION.get() {
        Some(v) => v,
        None => panic!(
            "IllegalStateException: BlazeVersionInfo is not set (call tracing_metadata_utils::set_tool_version)"
        ),
    }
}

/// The gRPC header carrying the binary `RequestMetadata`
/// (`ProtoUtils.keyForProto(RequestMetadata.getDefaultInstance())`).
// Java: com/google/devtools/build/lib/remote/util/TracingMetadataUtils.java:51 TracingMetadataUtils.METADATA_KEY
pub const METADATA_KEY: &str = "build.bazel.remote.execution.v2.requestmetadata-bin";

/// `buildMetadata(buildRequestId, commandId, actionId, actionMetadata)`.
// Java: com/google/devtools/build/lib/remote/util/TracingMetadataUtils.java:54 TracingMetadataUtils.buildMetadata
pub fn build_metadata(
    build_request_id: &str,
    command_id: &str,
    action_id: &str,
    action_metadata: Option<&dyn ActionExecutionMetadata>,
) -> RequestMetadata {
    build_metadata_from_parts(
        build_request_id,
        command_id,
        action_id,
        action_metadata.map(|m| m.get_mnemonic()).as_deref(),
        action_metadata
            .and_then(|m| m.get_owner().get_label().map(|l| l.get_canonical_form()))
            .as_deref(),
        action_metadata.and_then(|m| m.get_owner().get_configuration_checksum()),
    )
}

/// `buildMetadata(buildRequestId, commandId, actionId, mnemonic, label, configurationId)`.
// Java: com/google/devtools/build/lib/remote/util/TracingMetadataUtils.java:70 TracingMetadataUtils.buildMetadata
pub fn build_metadata_from_parts(
    build_request_id: &str,
    command_id: &str,
    action_id: &str,
    mnemonic: Option<&str>,
    label: Option<&str>,
    configuration_id: Option<&str>,
) -> RequestMetadata {
    let mut builder = RequestMetadata {
        correlated_invocations_id: build_request_id.to_string(),
        tool_invocation_id: command_id.to_string(),
        action_id: action_id.to_string(),
        tool_details: Some(ToolDetails {
            tool_name: "bazel".to_string(),
            tool_version: tool_version().to_string(),
        }),
        ..Default::default()
    };
    if let Some(mnemonic) = mnemonic {
        builder.action_mnemonic = mnemonic.to_string();
    }
    if let Some(label) = label {
        builder.target_id = label.to_string();
    }
    if let Some(configuration_id) = configuration_id {
        builder.configuration_id = configuration_id.to_string();
    }
    builder
}
