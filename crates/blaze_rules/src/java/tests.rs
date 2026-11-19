//! Ported JUnit tests of the Java rule classes that analyse BUILD files with rules_java through
//! `BuildViewTestCase` (`scratch.file`, `getConfiguredTarget`, `getGeneratingActionForLabel`).
//! Blaze has no analysis test harness yet; the Java rules are compared end to end with the
//! oracle by `scripts/java/compare` instead.

/// Java: src/test/java/com/google/devtools/build/lib/rules/java/JavaCompileActionBuilderTest.java
mod java_compile_action_builder_test {
    // Java: JavaCompileActionBuilderTest.testClassdirIsInBlazeOut
    #[test]
    #[ignore = "needs BuildViewTestCase (analysis test harness with rules_java)"]
    fn test_classdir_is_in_blaze_out() {}

    // Java: JavaCompileActionBuilderTest.progressMessage
    #[test]
    #[ignore = "needs BuildViewTestCase (analysis test harness with rules_java)"]
    fn progress_message() {}

    // Java: JavaCompileActionBuilderTest.progressMessageWithSourceJars
    #[test]
    #[ignore = "needs BuildViewTestCase (analysis test harness with rules_java)"]
    fn progress_message_with_source_jars() {}

    // Java: JavaCompileActionBuilderTest.progressMessageAnnotationProcessors
    #[test]
    #[ignore = "needs BuildViewTestCase (analysis test harness with rules_java)"]
    fn progress_message_annotation_processors() {}

    // Java: JavaCompileActionBuilderTest.testLocale
    #[test]
    #[ignore = "needs BuildViewTestCase (analysis test harness with rules_java)"]
    fn test_locale() {}

    // Java: JavaCompileActionBuilderTest.testClasspathReduction
    #[test]
    #[ignore = "needs BuildViewTestCase (analysis test harness with rules_java)"]
    fn test_classpath_reduction() {}

    // Java: JavaCompileActionBuilderTest.testExtraArgsPropagated
    #[test]
    #[ignore = "needs BuildViewTestCase (analysis test harness with rules_java)"]
    fn test_extra_args_propagated() {}
}

/// Java: src/test/java/com/google/devtools/build/lib/rules/java/JavaRuntimeTest.java
mod java_runtime_test {
    // Java: JavaRuntimeTest.simple
    #[test]
    #[ignore = "needs BuildViewTestCase (analysis test harness with rules_java)"]
    fn simple() {}

    // Java: JavaRuntimeTest.absoluteJavaHomeWithSrcs
    #[test]
    #[ignore = "needs BuildViewTestCase (analysis test harness with rules_java)"]
    fn absolute_java_home_with_srcs() {}

    // Java: JavaRuntimeTest.absoluteJavaHomeWithJava
    #[test]
    #[ignore = "needs BuildViewTestCase (analysis test harness with rules_java)"]
    fn absolute_java_home_with_java() {}

    // Java: JavaRuntimeTest.binJavaPathName
    #[test]
    #[ignore = "needs BuildViewTestCase (analysis test harness with rules_java)"]
    fn bin_java_path_name() {}

    // Java: JavaRuntimeTest.absoluteJavaHome
    #[test]
    #[ignore = "needs BuildViewTestCase (analysis test harness with rules_java)"]
    fn absolute_java_home() {}

    // Java: JavaRuntimeTest.relativeJavaHome
    #[test]
    #[ignore = "needs BuildViewTestCase (analysis test harness with rules_java)"]
    fn relative_java_home() {}

    // Java: JavaRuntimeTest.testRuntimeAlias
    #[test]
    #[ignore = "needs BuildViewTestCase (analysis test harness with rules_java)"]
    fn test_runtime_alias() {}

    // Java: JavaRuntimeTest.javaHomeWithMakeVariables
    #[test]
    #[ignore = "needs BuildViewTestCase (analysis test harness with rules_java)"]
    fn java_home_with_make_variables() {}

    // Java: JavaRuntimeTest.javaHomeWithInvalidMakeVariables
    #[test]
    #[ignore = "needs BuildViewTestCase (analysis test harness with rules_java)"]
    fn java_home_with_invalid_make_variables() {}

    // Java: JavaRuntimeTest.makeVariables
    #[test]
    #[ignore = "needs BuildViewTestCase (analysis test harness with rules_java)"]
    fn make_variables() {}

    // Java: JavaRuntimeTest.noSrcs
    #[test]
    #[ignore = "needs BuildViewTestCase (analysis test harness with rules_java)"]
    fn no_srcs() {}

    // Java: JavaRuntimeTest.invalidJavaBase
    #[test]
    #[ignore = "needs BuildViewTestCase (analysis test harness with rules_java)"]
    fn invalid_java_base() {}

    // Java: JavaRuntimeTest.javaHomeGenerated
    #[test]
    #[ignore = "needs BuildViewTestCase (analysis test harness with rules_java)"]
    fn java_home_generated() {}

    // Java: JavaRuntimeTest.javaRuntimeVersion_isAccessibleByNativeCode
    #[test]
    #[ignore = "needs BuildViewTestCase (analysis test harness with rules_java)"]
    fn java_runtime_version_is_accessible_by_native_code() {}
}

/// Java: src/test/java/com/google/devtools/build/lib/rules/java/JavaStarlarkApiTest.java
mod java_starlark_api_test {
    // Java: JavaStarlarkApiTest.javaInfoConstructorWithNeverlink
    #[test]
    #[ignore = "needs BuildViewTestCase (analysis test harness with rules_java)"]
    fn java_info_constructor_with_neverlink() {}

    // Java: JavaStarlarkApiTest.javaCommonMergeWithNeverlink
    #[test]
    #[ignore = "needs BuildViewTestCase (analysis test harness with rules_java)"]
    fn java_common_merge_with_neverlink() {}

    // Java: JavaStarlarkApiTest.javaCommonCompileWithNeverlink
    #[test]
    #[ignore = "needs BuildViewTestCase (analysis test harness with rules_java)"]
    fn java_common_compile_with_neverlink() {}

    // Java: JavaStarlarkApiTest.javaCommonCompile_nativeLibrariesPropagate
    #[test]
    #[ignore = "needs BuildViewTestCase (analysis test harness with rules_java)"]
    fn java_common_compile_native_libraries_propagate() {}

    // Java: JavaStarlarkApiTest.javaCommonCompile_directNativeLibraries
    #[test]
    #[ignore = "needs BuildViewTestCase (analysis test harness with rules_java)"]
    fn java_common_compile_direct_native_libraries() {}

    // Java: JavaStarlarkApiTest.strictDepsEnabled
    #[test]
    #[ignore = "needs BuildViewTestCase (analysis test harness with rules_java)"]
    fn strict_deps_enabled() {}

    // Java: JavaStarlarkApiTest.strictDepsDisabled
    #[test]
    #[ignore = "needs BuildViewTestCase (analysis test harness with rules_java)"]
    fn strict_deps_disabled() {}

    // Java: JavaStarlarkApiTest.strictJavaDepsFlagExposed_default
    #[test]
    #[ignore = "needs BuildViewTestCase (analysis test harness with rules_java)"]
    fn strict_java_deps_flag_exposed_default() {}

    // Java: JavaStarlarkApiTest.strictJavaDepsFlagExposed_error
    #[test]
    #[ignore = "needs BuildViewTestCase (analysis test harness with rules_java)"]
    fn strict_java_deps_flag_exposed_error() {}

    // Java: JavaStarlarkApiTest.mergeRuntimeOutputJarsTest
    #[test]
    #[ignore = "needs BuildViewTestCase (analysis test harness with rules_java)"]
    fn merge_runtime_output_jars_test() {}

    // Java: JavaStarlarkApiTest.javaToolchainFlag_default
    #[test]
    #[ignore = "needs BuildViewTestCase (analysis test harness with rules_java)"]
    fn java_toolchain_flag_default() {}

    // Java: JavaStarlarkApiTest.javaToolchainFlag_set
    #[test]
    #[ignore = "needs BuildViewTestCase (analysis test harness with rules_java)"]
    fn java_toolchain_flag_set() {}

    // Java: JavaStarlarkApiTest.testCompileOutputJarHasManifestProto
    #[test]
    #[ignore = "needs BuildViewTestCase (analysis test harness with rules_java)"]
    fn test_compile_output_jar_has_manifest_proto() {}

    // Java: JavaStarlarkApiTest.testCompileWithNeverlinkDeps
    #[test]
    #[ignore = "needs BuildViewTestCase (analysis test harness with rules_java)"]
    fn test_compile_with_neverlink_deps() {}

    // Java: JavaStarlarkApiTest.testCompileOutputJarNotInRuntimePathWithoutAnySourcesDefined
    #[test]
    #[ignore = "needs BuildViewTestCase (analysis test harness with rules_java)"]
    fn test_compile_output_jar_not_in_runtime_path_without_any_sources_defined() {}

    // Java: JavaStarlarkApiTest.testConfiguredTargetToolchain
    #[test]
    #[ignore = "needs BuildViewTestCase (analysis test harness with rules_java)"]
    fn test_configured_target_toolchain() {}

    // Java: JavaStarlarkApiTest.defaultJavacOpts
    #[test]
    #[ignore = "needs BuildViewTestCase (analysis test harness with rules_java)"]
    fn default_javac_opts() {}

    // Java: JavaStarlarkApiTest.defaultJavacOpts_asDepset
    #[test]
    #[ignore = "needs BuildViewTestCase (analysis test harness with rules_java)"]
    fn default_javac_opts_as_depset() {}

    // Java: JavaStarlarkApiTest.defaultJavacOpts_toolchainProvider
    #[test]
    #[ignore = "needs BuildViewTestCase (analysis test harness with rules_java)"]
    fn default_javac_opts_toolchain_provider() {}

    // Java: JavaStarlarkApiTest.testJavaRuntimeProviderFiles
    #[test]
    #[ignore = "needs BuildViewTestCase (analysis test harness with rules_java)"]
    fn test_java_runtime_provider_files() {}

    // Java: JavaStarlarkApiTest.testJavaLibraryCollectsCoverageDependenciesFromResources
    #[test]
    #[ignore = "needs BuildViewTestCase (analysis test harness with rules_java)"]
    fn test_java_library_collects_coverage_dependencies_from_resources() {}

    // Java: JavaStarlarkApiTest.testSkipAnnotationProcessing
    #[test]
    #[ignore = "needs BuildViewTestCase (analysis test harness with rules_java)"]
    fn test_skip_annotation_processing() {}

    // Java: JavaStarlarkApiTest.testCompileWithDisablingCompileJarIsPrivateApi
    #[test]
    #[ignore = "needs BuildViewTestCase (analysis test harness with rules_java)"]
    fn test_compile_with_disabling_compile_jar_is_private_api() {}

    // Java: JavaStarlarkApiTest.testCompileWithClasspathResourcesIsPrivateApi
    #[test]
    #[ignore = "needs BuildViewTestCase (analysis test harness with rules_java)"]
    fn test_compile_with_classpath_resources_is_private_api() {}

    // Java: JavaStarlarkApiTest.testInjectingRuleKindIsPrivateApi
    #[test]
    #[ignore = "needs BuildViewTestCase (analysis test harness with rules_java)"]
    fn test_injecting_rule_kind_is_private_api() {}

    // Java: JavaStarlarkApiTest.testEnableJSpecifyIsPrivateApi
    #[test]
    #[ignore = "needs BuildViewTestCase (analysis test harness with rules_java)"]
    fn test_enable_j_specify_is_private_api() {}

    // Java: JavaStarlarkApiTest.testMergeJavaOutputsIsPrivateApi
    #[test]
    #[ignore = "needs BuildViewTestCase (analysis test harness with rules_java)"]
    fn test_merge_java_outputs_is_private_api() {}

    // Java: JavaStarlarkApiTest.testMergeSourceJarsIsPrivateApi
    #[test]
    #[ignore = "needs BuildViewTestCase (analysis test harness with rules_java)"]
    fn test_merge_source_jars_is_private_api() {}

    // Java: JavaStarlarkApiTest.testCompileIncludeCompilationInfoIsPrivateApi
    #[test]
    #[ignore = "needs BuildViewTestCase (analysis test harness with rules_java)"]
    fn test_compile_include_compilation_info_is_private_api() {}

    // Java: JavaStarlarkApiTest.testCompileWithResourceJarsIsPrivateApi
    #[test]
    #[ignore = "needs BuildViewTestCase (analysis test harness with rules_java)"]
    fn test_compile_with_resource_jars_is_private_api() {}

    // Java: JavaStarlarkApiTest.testRunIjarWithOutputParameterIsPrivateApi
    #[test]
    #[ignore = "needs BuildViewTestCase (analysis test harness with rules_java)"]
    fn test_run_ijar_with_output_parameter_is_private_api() {}

    // Java: JavaStarlarkApiTest.testPackSourcesWithExternalResourceArtifact
    #[test]
    #[ignore = "needs BuildViewTestCase (analysis test harness with rules_java)"]
    fn test_pack_sources_with_external_resource_artifact() {}

    // Java: JavaStarlarkApiTest.mergeAddExports
    #[test]
    #[ignore = "needs BuildViewTestCase (analysis test harness with rules_java)"]
    fn merge_add_exports() {}

    // Java: JavaStarlarkApiTest.hermeticStaticLibs
    #[test]
    #[ignore = "needs BuildViewTestCase (analysis test harness with rules_java)"]
    fn hermetic_static_libs() {}

    // Java: JavaStarlarkApiTest.implicitLibCtSym
    #[test]
    #[ignore = "needs BuildViewTestCase (analysis test harness with rules_java)"]
    fn implicit_lib_ct_sym() {}

    // Java: JavaStarlarkApiTest.explicitLibCtSym
    #[test]
    #[ignore = "needs BuildViewTestCase (analysis test harness with rules_java)"]
    fn explicit_lib_ct_sym() {}

    // Java: JavaStarlarkApiTest.testProviderValidationPrintsProviderName
    #[test]
    #[ignore = "needs BuildViewTestCase (analysis test harness with rules_java)"]
    fn test_provider_validation_prints_provider_name() {}

    // Java: JavaStarlarkApiTest.testNativeJavaInfoPrintableType_isJavaInfo
    #[test]
    #[ignore = "needs BuildViewTestCase (analysis test harness with rules_java)"]
    fn test_native_java_info_printable_type_is_java_info() {}
}
