//! Port of `com.google.devtools.build.lib.remote` (see docs/remote.md).
#![allow(clippy::too_many_arguments, clippy::new_without_default)]

pub mod abstract_action_input_prefetcher;
pub mod api_version;
pub mod authandtls;
pub mod byte_stream_uploader;
pub mod chunker;
pub mod circuitbreaker;
pub mod client_api_version;
pub mod combined_cache;
pub mod combined_cache_client_factory;
pub mod common;
pub mod disk;
pub mod execute_retrier;
pub mod execution_status_exception;
pub mod grpc;
pub mod grpc_cache_client;
pub mod grpc_remote_executor;
pub mod hooks;
pub mod logging;
pub mod merkletree;
pub mod platform_utils;
pub mod reapi;
pub mod remote_action;
pub mod remote_action_input_fetcher;
pub mod remote_execution_cache;
// Java still reads and writes the deprecated REAPI fields (output_files, platform, ...).
#[allow(deprecated)]
pub mod remote_execution_service;
// Java still reads and writes the deprecated REAPI fields (output_files, platform, ...).
#[allow(deprecated)]
pub mod remote_execution_service_download;
pub mod remote_grpc_service_config;
pub mod remote_module;
pub mod remote_output_checker;
// Java still writes the deprecated `Command.platform`.
#[allow(deprecated)]
pub mod remote_repository_remote_executor;
pub mod remote_retrier;
pub mod remote_retrier_utils;
pub mod remote_server_capabilities;
pub mod remote_spawn_cache;
pub mod remote_spawn_runner;
pub mod remote_spawn_strategy;
pub mod retrier;
pub mod runtime;
pub mod scrubber;
pub mod store;
pub mod upload_manifest;
pub mod util;
pub mod zstd;
