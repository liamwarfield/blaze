//! Port of `com.google.devtools.build.lib.runtime` (W1 skeleton): `BlazeRuntime`,
//! `BlazeCommandDispatcher`, directories, the command registry, and the `version`, `info`,
//! `clean` and `shutdown` commands.

#![allow(
    clippy::too_many_arguments,
    clippy::new_without_default,
    non_camel_case_types
)]
// Ported signatures and Java's control flow win (PORTING.md "Style"): maps keyed by `Artifact`
// (Java `Map<Artifact, ...>`), exceptions carried by value, Java method names (`from_*`), Java's
// branch structure.
#![allow(
    clippy::mutable_key_type,
    clippy::result_large_err,
    clippy::large_enum_variant,
    clippy::type_complexity,
    clippy::wrong_self_convention,
    clippy::collapsible_if,
    clippy::collapsible_match,
    clippy::if_same_then_else
)]

pub mod blaze_command_utils;
pub mod bugreport;
pub mod command;
pub mod command_environment;
pub mod custom_exit_code_publisher;
mod command_name_cache_impl;
pub mod directories;
pub mod dispatcher;
pub mod exit_code;
pub mod info_item;
pub mod interrupt;
pub mod jvm_signals;
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
pub mod starlark_options_parser;
mod unported_options;
pub mod version_info;

#[cfg(test)]
mod dispatcher_tests;
