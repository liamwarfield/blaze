//! Port of `lib/worker`: persistent workers (`--strategy=<Mnemonic>=worker`, the "worker"
//! spawn strategy), their pool, the proto and JSON wire protocols, sandboxed and multiplex
//! workers, and the worker metrics.

pub mod error_message;
pub mod json_reader;
pub mod json_worker_message_processor;
pub mod proto_worker_message_processor;
pub mod recording_input_stream;
pub mod shutdown_hooks;
pub mod singleplex_worker;
pub mod work_request_handler;
pub mod worker;
pub mod worker_exec_root;
pub mod worker_factory;
pub mod worker_files_hash;
pub mod worker_key;
pub mod worker_lifecycle_manager;
pub mod worker_module;
pub mod worker_multiplexer;
pub mod worker_parser;
pub mod worker_pool;
pub mod worker_process_metrics;
pub mod worker_process_status;
pub mod worker_protocol;
pub mod worker_proxy;
pub mod worker_spawn_runner;
