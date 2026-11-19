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
pub mod java_plugins_flag_alias;
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

// PORT: not ported, dead in Bazel 9.3.2. These starlarkbuildapi/java interfaces describe the
// Java-native JavaInfo family that moved to Starlark (rules_java). No class under src/main/java
// implements any of them (checked with `grep -rl` over src/main/java at the pin): they are only
// referenced by each other and by JavaCommonApi's documentation-only default methods; their
// implementations (JavaInfo, JavaGenJarsProvider, JavaCompilationInfoProvider,
// JavaRuleOutputJarsProvider, JavaModuleFlagsProvider) live in src/test/java only, and
// JavaRuntimeInfoApi and JavaToolchainStarlarkApiProviderApi have no implementation at all.
//   com/google/devtools/build/lib/starlarkbuildapi/java/JavaInfoApi.java
//   com/google/devtools/build/lib/starlarkbuildapi/java/JavaAnnotationProcessingApi.java
//   com/google/devtools/build/lib/starlarkbuildapi/java/JavaCompilationInfoProviderApi.java
//   com/google/devtools/build/lib/starlarkbuildapi/java/JavaRuleOutputJarsProviderApi.java
//   com/google/devtools/build/lib/starlarkbuildapi/java/JavaRuntimeInfoApi.java
//   com/google/devtools/build/lib/starlarkbuildapi/java/JavaToolchainStarlarkApiProviderApi.java
//   com/google/devtools/build/lib/starlarkbuildapi/java/JavaModuleFlagsProviderApi.java

use std::sync::Arc;

use blaze_analysis::configured_rule_class_provider::ConfiguredRuleClassProviderBuilder;

/// Registers the Java rules' native configured target factories under their Java factory class
/// simple names (the names `blaze_packages::native_rules::java_rules` records).
// Java: com/google/devtools/build/lib/bazel/rules/JavaRules.java:71 JavaRules.init
pub fn register(builder: &mut ConfiguredRuleClassProviderBuilder) {
    builder.add_factory(
        "JavaPluginsFlagAlias",
        Arc::new(java_plugins_flag_alias::JavaPluginsFlagAlias),
    );
}

#[cfg(test)]
mod tests;
