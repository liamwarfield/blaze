//! Java rule natives (`rules/java`, `bazel/rules/java`): the core half of the Java rules, whose
//! rules themselves are Starlark (rules_java). `_builtins.internal.java_common_internal_do_not_use`
//! (`JavaStarlarkCommon`), the wrappers of rules_java's providers (`JavaToolchainProvider`,
//! `JavaRuntimeInfo`, `BootClassPathInfo`, `JavaPluginInfo`, ...), the compile actions
//! (`JavaCompileAction`, `JavaHeaderCompileAction`) with their builders and execution, and the
//! resource jar action.
//!
//! The `java` configuration fragment's Starlark value is in
//! `crate::fragments::java_configuration`.

pub mod boot_class_path_info;
pub mod java_compilation_helper;
pub mod java_compile_action;
pub mod java_compile_action_builder;
pub mod java_compile_action_context;
pub mod java_header_compile_action;
pub mod java_helper;
pub mod java_output;
pub mod java_package_configuration_provider;
pub mod java_plugin_info;
pub mod java_runtime_info;
pub mod java_semantics;
pub mod java_starlark_common;
pub mod java_target_attributes;
pub mod java_toolchain_provider;
pub mod java_toolchain_tool;
pub mod java_util;
pub mod provider_util;
pub mod resource_jar_action_builder;
pub mod starlark_info_wrapper;

pub use java_starlark_common::java_common_internal_value;

#[cfg(test)]
mod tests;
