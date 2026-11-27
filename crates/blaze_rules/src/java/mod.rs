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
// PORT: skipped, no implementation under src/main/java at the pin (see above): JavaAnnotationProcessingApi.usesAnnotationProcessing
// PORT: skipped, no implementation under src/main/java at the pin (see above): JavaAnnotationProcessingApi.getGenClassJar
// PORT: skipped, no implementation under src/main/java at the pin (see above): JavaAnnotationProcessingApi.getGenSourceJar
// PORT: skipped, no implementation under src/main/java at the pin (see above): JavaAnnotationProcessingApi.getTransitiveGenClassJarsForStarlark
// PORT: skipped, no implementation under src/main/java at the pin (see above): JavaAnnotationProcessingApi.getTransitiveGenSourceJarsForStarlark
// PORT: skipped, no implementation under src/main/java at the pin (see above): JavaAnnotationProcessingApi.getProcessorClasspathForStarlark
// PORT: skipped, no implementation under src/main/java at the pin (see above): JavaAnnotationProcessingApi.getProcessorClassNamesList
// PORT: skipped, no implementation under src/main/java at the pin (see above): JavaCompilationInfoProviderApi.getJavacOptsStarlark
// PORT: skipped, no implementation under src/main/java at the pin (see above): JavaCompilationInfoProviderApi.getRuntimeClasspath
// PORT: skipped, no implementation under src/main/java at the pin (see above): JavaCompilationInfoProviderApi.getCompilationClasspath
// PORT: skipped, no implementation under src/main/java at the pin (see above): JavaCompilationInfoProviderApi.getBootClasspathList
// PORT: skipped, no implementation under src/main/java at the pin (see above): JavaInfoApi.isNeverlink
// PORT: skipped, no implementation under src/main/java at the pin (see above): JavaInfoApi.getTransitiveRuntimeJars
// PORT: skipped, no implementation under src/main/java at the pin (see above): JavaInfoApi.getTransitiveCompileTimeJars
// PORT: skipped, no implementation under src/main/java at the pin (see above): JavaInfoApi.getCompileTimeJars
// PORT: skipped, no implementation under src/main/java at the pin (see above): JavaInfoApi.getFullCompileTimeJars
// PORT: skipped, no implementation under src/main/java at the pin (see above): JavaInfoApi.headerCompilationDirectDeps
// PORT: skipped, no implementation under src/main/java at the pin (see above): JavaInfoApi.getSourceJars
// PORT: skipped, no implementation under src/main/java at the pin (see above): JavaInfoApi.getOutputJars
// PORT: skipped, no implementation under src/main/java at the pin (see above): JavaInfoApi.getGenJarsProvider
// PORT: skipped, no implementation under src/main/java at the pin (see above): JavaInfoApi.getCompilationInfoProvider
// PORT: skipped, no implementation under src/main/java at the pin (see above): JavaInfoApi.getRuntimeOutputJars
// PORT: skipped, no implementation under src/main/java at the pin (see above): JavaInfoApi.getTransitiveSourceJars
// PORT: skipped, no implementation under src/main/java at the pin (see above): JavaInfoApi.getTransitiveNativeLibrariesForStarlark
// PORT: skipped, no implementation under src/main/java at the pin (see above): JavaInfoApi.getCcLinkParamInfo
// PORT: skipped, no implementation under src/main/java at the pin (see above): JavaInfoApi.getJavaModuleFlagsInfo
// PORT: skipped, no implementation under src/main/java at the pin (see above): JavaInfoApi.getTransitiveFullCompileJars
// PORT: skipped, no implementation under src/main/java at the pin (see above): JavaInfoApi.getCompileTimeJavaDependencies
// PORT: skipped, no implementation under src/main/java at the pin (see above): JavaInfoApi.getJavaConstraintsStarlark
// PORT: skipped, no implementation under src/main/java at the pin (see above): JavaModuleFlagsProviderApi.getAddExports
// PORT: skipped, no implementation under src/main/java at the pin (see above): JavaModuleFlagsProviderApi.getAddOpens
// PORT: skipped, no implementation under src/main/java at the pin (see above): JavaRuleOutputJarsProviderApi.getJavaOutputs
// PORT: skipped, no implementation under src/main/java at the pin (see above): JavaRuleOutputJarsProviderApi.getJdeps
// PORT: skipped, no implementation under src/main/java at the pin (see above): JavaRuleOutputJarsProviderApi.getNativeHeaders
// PORT: skipped, no implementation under src/main/java at the pin (see above): JavaRuntimeInfoApi.javaHome
// PORT: skipped, no implementation under src/main/java at the pin (see above): JavaRuntimeInfoApi.javaBinaryExecPath
// PORT: skipped, no implementation under src/main/java at the pin (see above): JavaRuntimeInfoApi.javaHomeRunfilesPath
// PORT: skipped, no implementation under src/main/java at the pin (see above): JavaRuntimeInfoApi.javaBinaryRunfilesPath
// PORT: skipped, no implementation under src/main/java at the pin (see above): JavaRuntimeInfoApi.starlarkJavaBaseInputs
// PORT: skipped, no implementation under src/main/java at the pin (see above): JavaRuntimeInfoApi.starlarkHermeticInputs
// PORT: skipped, no implementation under src/main/java at the pin (see above): JavaRuntimeInfoApi.libCtSym
// PORT: skipped, no implementation under src/main/java at the pin (see above): JavaRuntimeInfoApi.libModules
// PORT: skipped, no implementation under src/main/java at the pin (see above): JavaRuntimeInfoApi.defaultCDS
// PORT: skipped, no implementation under src/main/java at the pin (see above): JavaRuntimeInfoApi.starlarkHermeticStaticLibs
// PORT: skipped, no implementation under src/main/java at the pin (see above): JavaRuntimeInfoApi.version
// PORT: skipped, no implementation under src/main/java at the pin (see above): JavaToolchainStarlarkApiProviderApi.getSourceVersion
// PORT: skipped, no implementation under src/main/java at the pin (see above): JavaToolchainStarlarkApiProviderApi.getTargetVersion
// PORT: skipped, no implementation under src/main/java at the pin (see above): JavaToolchainStarlarkApiProviderApi.getToolchainLabel
// PORT: skipped, no implementation under src/main/java at the pin (see above): JavaToolchainStarlarkApiProviderApi.getSingleJar
// PORT: skipped, no implementation under src/main/java at the pin (see above): JavaToolchainStarlarkApiProviderApi.getStarlarkBootclasspath
// PORT: skipped, no implementation under src/main/java at the pin (see above): JavaToolchainStarlarkApiProviderApi.getStarlarkJvmOptions
// PORT: skipped, no implementation under src/main/java at the pin (see above): JavaToolchainStarlarkApiProviderApi.getIjar
// PORT: skipped, no implementation under src/main/java at the pin (see above): JavaToolchainStarlarkApiProviderApi.getJacocoRunner
// PORT: skipped, no implementation under src/main/java at the pin (see above): JavaToolchainStarlarkApiProviderApi.getStarlarkTools
// PORT: skipped, no implementation under src/main/java at the pin (see above): JavaToolchainStarlarkApiProviderApi.getJavaRuntime
// PORT: skipped, no implementation under src/main/java at the pin (see above): JavaToolchainStarlarkApiProviderApi.getProguardAllowlister

use std::sync::Arc;

use blaze_analysis::configured_rule_class_provider::ConfiguredRuleClassProviderBuilder;

/// Registers the Java rules' native configured target factories under their Java factory class
/// simple names (the names `blaze_packages::native_rules::java_rules` records).
// Java: com/google/devtools/build/lib/bazel/rules/JavaRules.java:38 JavaRules.init
pub fn register(builder: &mut ConfiguredRuleClassProviderBuilder) {
    builder.add_factory(
        "JavaPluginsFlagAlias",
        Arc::new(java_plugins_flag_alias::JavaPluginsFlagAlias),
    );
}

#[cfg(test)]
mod tests;
