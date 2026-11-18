//! Port of `com.google.devtools.build.lib.runtime` (W1 skeleton): `BlazeRuntime`,
//! `BlazeCommandDispatcher`, directories, the command registry, and the `version`, `info`,
//! `clean` and `shutdown` commands.

#![allow(
    clippy::too_many_arguments,
    clippy::new_without_default,
    non_camel_case_types
)]

pub mod command;
pub mod command_environment;
pub mod directories;
pub mod dispatcher;
pub mod exit_code;
pub mod info_item;
pub mod module;
pub mod options_result;
pub mod runtime;
pub mod workspace;

pub mod bazel_modules;
pub mod command_registry;
pub mod commands;
pub mod fragment_options;
pub mod local_resources;
pub mod server_log;
pub mod version_info;
