//! Port of `com.google.devtools.build.lib.remote` (see docs/remote.md).
#![allow(clippy::too_many_arguments, clippy::new_without_default)]
pub mod abstract_action_input_prefetcher;
pub mod api_version;
pub mod authandtls;
pub mod byte_stream_build_event_artifact_uploader;
pub mod byte_stream_uploader;
pub mod cancellable_task;
pub mod chunked_blob_downloader;
pub mod chunked_blob_uploader;
pub mod chunker;
pub mod chunking;
pub mod circuitbreaker;
pub mod client_api_version;
pub mod combined_cache;
mod concurrent_artifact_path_trie;
pub mod remote_rewound_action_synchronizer;
pub mod combined_cache_client_factory;
pub mod common;
pub mod disk;
pub mod downloader;
pub mod execute_retrier;
pub mod execution_status_exception;
pub mod grpc;
pub mod grpc_cache_client;
pub mod grpc_remote_executor;
pub mod hooks;
pub mod http;
pub mod lease_service;
pub mod remote_lease_extension;
pub mod logging;
pub mod merkletree;
pub mod platform_utils;
pub mod reader_preferring_read_write_lock;
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
pub mod remote_external_overlay_file_system;
pub mod remote_important_output_handler;
pub mod remote_grpc_service_config;
pub mod remote_module;
pub mod remote_output_checker;
pub mod remote_repo_contents_cache_impl;
// Java still writes the deprecated `Command.platform`.
#[allow(deprecated)]
pub mod remote_repository_remote_executor;
pub mod remote_retrier;
pub mod repository_remote_helpers_factory_impl;
pub mod remote_retrier_utils;
pub mod remote_server_capabilities;
pub mod remote_spawn_cache;
pub mod remote_spawn_runner;
pub mod remote_spawn_strategy;
pub mod retrier;
pub mod runtime;
pub mod scrubber;
pub mod store;
pub mod text_format;
pub mod upload_manifest;
pub mod util;
pub mod zstd;
