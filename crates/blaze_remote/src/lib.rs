//! Port of `com.google.devtools.build.lib.remote` (see docs/remote.md).
#![allow(clippy::too_many_arguments, clippy::new_without_default)]

#[cfg(feature = "wip")]
pub mod api_version;
#[cfg(feature = "wip")]
pub mod authandtls;
#[cfg(feature = "wip")]
pub mod byte_stream_uploader;
#[cfg(feature = "wip")]
pub mod chunker;
#[cfg(feature = "wip")]
pub mod circuitbreaker;
#[cfg(feature = "wip")]
pub mod client_api_version;
#[cfg(feature = "wip")]
pub mod combined_cache;
pub mod common;
#[cfg(feature = "wip")]
pub mod disk;
#[cfg(feature = "wip")]
pub mod execute_retrier;
#[cfg(feature = "wip")]
pub mod execution_status_exception;
pub mod grpc;
#[cfg(feature = "wip")]
pub mod grpc_cache_client;
pub mod hooks;
#[cfg(feature = "wip")]
pub mod logging;
#[cfg(feature = "wip")]
pub mod merkletree;
#[cfg(feature = "wip")]
pub mod platform_utils;
pub mod reapi;
#[cfg(feature = "wip")]
pub mod remote_action;
#[cfg(feature = "wip")]
pub mod remote_execution_cache;
#[cfg(feature = "wip")]
// Java still reads and writes the deprecated REAPI fields (output_files, platform, ...).
#[allow(deprecated)]
pub mod remote_execution_service;
#[cfg(feature = "wip")]
// Java still reads and writes the deprecated REAPI fields (output_files, platform, ...).
#[allow(deprecated)]
pub mod remote_execution_service_download;
#[cfg(feature = "wip")]
pub mod remote_output_checker;
#[cfg(feature = "wip")]
pub mod remote_retrier;
#[cfg(feature = "wip")]
pub mod remote_retrier_utils;
#[cfg(feature = "wip")]
pub mod remote_server_capabilities;
#[cfg(feature = "wip")]
pub mod retrier;
pub mod runtime;
#[cfg(feature = "wip")]
pub mod scrubber;
pub mod store;
#[cfg(feature = "wip")]
pub mod upload_manifest;
pub mod util;
#[cfg(feature = "wip")]
pub mod zstd;
