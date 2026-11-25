//! Port of `src/test/java/com/google/devtools/build/lib/rules/java/JavaStarlarkApiTest.java`, on
//! the harness's mock `rules_java`.
//!
//! The Java class is parameterized over three target platforms (linux, darwin, windows); the
//! parameter is read only by its `useConfiguration` override. The tests ported here do not call
//! `useConfiguration`, so each runs once (the three Java runs are the same). Bodies transcribed by
//! the converter of `docs/analysis-harness.md` (expectations character for character).

use crate::testing::BuildViewTestCase;

/// `@Before setupMyInfo`.
// Java: JavaStarlarkApiTest.setupMyInfo
fn new_test() -> BuildViewTestCase {
    let t = BuildViewTestCase::new();
    t.scratch
        .file("myinfo/myinfo.bzl", &["MyInfo = provider()"]);
    t.scratch.file("myinfo/BUILD", &[]);
    t
}

/// `JavaTestUtil.writeBuildFileForJavaToolchain(scratch)`.
// Java: JavaTestUtil.writeBuildFileForJavaToolchain
fn write_build_file_for_java_toolchain(t: &BuildViewTestCase) {
    t.scratch
        .file("java/com/google/test/turbine_canary_deploy.jar", &[]);
    t.scratch.file("java/com/google/test/turbine_graal", &[]);
    t.scratch.file("java/com/google/test/tzdata.jar", &[]);
    t.scratch.overwrite_file(
        "java/com/google/test/BUILD",
        &[
            "load('@rules_java//java:defs.bzl', 'java_runtime', 'java_toolchain')",
            "java_toolchain(name = 'toolchain',",
            "    source_version = '6',",
            "    target_version = '6',",
            "    bootclasspath = ['rt.jar'],",
            "    xlint = ['toto'],",
            "    javacopts =['-Xmaxerrs 500'],",
            "    compatible_javacopts = {",
            "        'appengine': ['-XDappengineCompatible'],",
            "        'android': ['-XDandroidCompatible'],",
            "    },",
            "    tools = [':javac_canary.jar'],",
            "    javabuilder = [':JavaBuilder_deploy.jar'],",
            "    jacocorunner = ':jacocorunner.jar',",
            "    header_compiler = [':turbine_canary_deploy.jar'],",
            "    header_compiler_direct = [':turbine_graal'],",
            "    singlejar = ['singlejar'],",
            "    ijar = ['ijar'],",
            "    genclass = ['GenClass_deploy.jar'],",
            "    timezone_data = 'tzdata.jar',",
            "    java_runtime = ':jvm-k8'",
            ")",
            "java_runtime(",
            "    name = 'jvm-k8',",
            "    srcs = [",
            "        'k8/a', ",
            "        'k8/b',",
            "    ], ",
            "    java_home = 'k8',",
            ")",
            "toolchain(",
            "    name = 'java_toolchain',",
            "    toolchain = ':toolchain',",
            "    toolchain_type = '@bazel_tools//tools/jdk:toolchain_type',",
            ")",
        ],
    );
}

// Java: JavaStarlarkApiTest.testCompileWithDisablingCompileJarIsPrivateApi
#[test]
fn test_compile_with_disabling_compile_jar_is_private_api() {
    let mut t = new_test();
    write_build_file_for_java_toolchain(&t);
    t.scratch.file("foo/custom_rule.bzl", &["load('@rules_java//java:defs.bzl', 'java_common')", "def _impl(ctx):", "  java_common.compile(", "    ctx,", "    output = ctx.actions.declare_file('output.jar'),", "    java_toolchain = ctx.attr._java_toolchain[java_common.JavaToolchainInfo],", "    enable_compile_jar_action = False,", "  )", "  return []", "java_custom_library = rule(", "  implementation = _impl,", "  attrs = {", "    'srcs': attr.label_list(),", "    '_java_toolchain': attr.label(default = Label('//java/com/google/test:toolchain')),", "  },", "  toolchains = ['@@bazel_tools//tools/jdk:toolchain_type'],", "  fragments = ['java']", ")"]);
    t.scratch.file(
        "foo/BUILD",
        &[r#"load(":custom_rule.bzl", "java_custom_library")

java_custom_library(name = "custom")
"#],
    );
    t.remove_fail_fast_handler();
    t.get_configured_target("//foo:custom");
    t.assert_contains_event("got unexpected keyword argument: enable_compile_jar_action");
}

// Java: JavaStarlarkApiTest.testCompileWithClasspathResourcesIsPrivateApi
#[test]
fn test_compile_with_classpath_resources_is_private_api() {
    let mut t = new_test();
    write_build_file_for_java_toolchain(&t);
    t.scratch
        .file("foo/resource.txt", &["Totally real resource content"]);
    t.scratch.file("foo/custom_rule.bzl", &["load('@rules_java//java:defs.bzl', 'java_common')", "def _impl(ctx):", "  java_common.compile(", "    ctx,", "    output = ctx.actions.declare_file('output.jar'),", "    classpath_resources = ctx.files.classpath_resources,", "    java_toolchain = ctx.attr._java_toolchain[java_common.JavaToolchainInfo],", "  )", "  return []", "java_custom_library = rule(", "  implementation = _impl,", "  attrs = {", "    'srcs': attr.label_list(),", "    'classpath_resources': attr.label_list(allow_files = True),", "    '_java_toolchain': attr.label(default = Label('//java/com/google/test:toolchain')),", "  },", "  toolchains = ['@@bazel_tools//tools/jdk:toolchain_type'],", "  fragments = ['java']", ")"]);
    t.scratch.file(
        "foo/BUILD",
        &[r#"load(":custom_rule.bzl", "java_custom_library")

java_custom_library(
    name = "custom",
    classpath_resources = ["resource.txt"],
)
"#],
    );
    t.remove_fail_fast_handler();
    t.get_configured_target("//foo:custom");
    t.assert_contains_event("got unexpected keyword argument: classpath_resources");
}

// Java: JavaStarlarkApiTest.testInjectingRuleKindIsPrivateApi
#[test]
fn test_injecting_rule_kind_is_private_api() {
    let mut t = new_test();
    write_build_file_for_java_toolchain(&t);
    t.scratch.file("foo/custom_rule.bzl", &["load('@rules_java//java:defs.bzl', 'java_common')", "def _impl(ctx):", "  java_common.compile(", "    ctx,", "    output = ctx.actions.declare_file('output.jar'),", "    java_toolchain = ctx.attr._java_toolchain[java_common.JavaToolchainInfo],", "    injecting_rule_kind = 'example_rule_kind',", "  )", "  return []", "java_custom_library = rule(", "  implementation = _impl,", "  attrs = {", "    'srcs': attr.label_list(),", "    '_java_toolchain': attr.label(default = Label('//java/com/google/test:toolchain')),", "  },", "  toolchains = ['@@bazel_tools//tools/jdk:toolchain_type'],", "  fragments = ['java']", ")"]);
    t.scratch.file(
        "foo/BUILD",
        &[r#"load(":custom_rule.bzl", "java_custom_library")

java_custom_library(name = "custom")
"#],
    );
    t.remove_fail_fast_handler();
    t.get_configured_target("//foo:custom");
    t.assert_contains_event("got unexpected keyword argument: injecting_rule_kind");
}

// Java: JavaStarlarkApiTest.testEnableJSpecifyIsPrivateApi
#[test]
fn test_enable_j_specify_is_private_api() {
    let mut t = new_test();
    write_build_file_for_java_toolchain(&t);
    t.scratch.file("foo/custom_rule.bzl", &["load('@rules_java//java:defs.bzl', 'java_common')", "def _impl(ctx):", "  java_common.compile(", "    ctx,", "    output = ctx.actions.declare_file('output.jar'),", "    java_toolchain = ctx.attr._java_toolchain[java_common.JavaToolchainInfo],", "    enable_jspecify = False,", "  )", "  return []", "java_custom_library = rule(", "  implementation = _impl,", "  attrs = {", "    'srcs': attr.label_list(),", "    '_java_toolchain': attr.label(default = Label('//java/com/google/test:toolchain')),", "  },", "  toolchains = ['@@bazel_tools//tools/jdk:toolchain_type'],", "  fragments = ['java']", ")"]);
    t.scratch.file(
        "foo/BUILD",
        &[r#"load(":custom_rule.bzl", "java_custom_library")

java_custom_library(name = "custom")
"#],
    );
    t.remove_fail_fast_handler();
    t.get_configured_target("//foo:custom");
    t.assert_contains_event("got unexpected keyword argument: enable_jspecify");
}

// Java: JavaStarlarkApiTest.testMergeJavaOutputsIsPrivateApi
#[test]
fn test_merge_java_outputs_is_private_api() {
    let mut t = new_test();
    t.scratch.file(
        "foo/custom_rule.bzl",
        &[
            r#"load("@rules_java//java:defs.bzl", "java_common", "JavaInfo")
def _impl(ctx):
    output_jar = ctx.actions.declare_file("lib.jar")
    java_info = JavaInfo(output_jar = output_jar, compile_jar = None)
    java_common.merge(
        [java_info],
        merge_java_outputs = False,
    )
    return []

java_custom_library = rule(
    implementation = _impl,
    fragments = ["java"],
)
"#,
        ],
    );
    t.scratch.file(
        "foo/BUILD",
        &[r#"load(":custom_rule.bzl", "java_custom_library")

java_custom_library(name = "custom")
"#],
    );
    t.remove_fail_fast_handler();
    t.get_configured_target("//foo:custom");
    t.assert_contains_event("got unexpected keyword argument: merge_java_outputs");
}

// Java: JavaStarlarkApiTest.testMergeSourceJarsIsPrivateApi
#[test]
fn test_merge_source_jars_is_private_api() {
    let mut t = new_test();
    t.scratch.file(
        "foo/custom_rule.bzl",
        &[
            r#"load("@rules_java//java:defs.bzl", "java_common", "JavaInfo")
def _impl(ctx):
    output_jar = ctx.actions.declare_file("lib.jar")
    java_info = JavaInfo(output_jar = output_jar, compile_jar = None)
    java_common.merge(
        [java_info],
        merge_source_jars = False,
    )
    return []

java_custom_library = rule(
    implementation = _impl,
    fragments = ["java"],
)
"#,
        ],
    );
    t.scratch.file(
        "foo/BUILD",
        &[r#"load(":custom_rule.bzl", "java_custom_library")

java_custom_library(name = "custom")
"#],
    );
    t.remove_fail_fast_handler();
    t.get_configured_target("//foo:custom");
    t.assert_contains_event("got unexpected keyword argument: merge_source_jars");
}

// Java: JavaStarlarkApiTest.testCompileIncludeCompilationInfoIsPrivateApi
#[test]
fn test_compile_include_compilation_info_is_private_api() {
    let mut t = new_test();
    write_build_file_for_java_toolchain(&t);
    t.scratch.file("foo/custom_rule.bzl", &["load('@rules_java//java:defs.bzl', 'java_common')", "def _impl(ctx):", "  java_common.compile(", "    ctx,", "    output = ctx.actions.declare_file('output.jar'),", "    java_toolchain = ctx.attr._java_toolchain[java_common.JavaToolchainInfo],", "    include_compilation_info = False,", "  )", "  return []", "java_custom_library = rule(", "  implementation = _impl,", "  attrs = {", "    'srcs': attr.label_list(),", "    '_java_toolchain': attr.label(default = Label('//java/com/google/test:toolchain')),", "  },", "  toolchains = ['@@bazel_tools//tools/jdk:toolchain_type'],", "  fragments = ['java']", ")"]);
    t.scratch.file(
        "foo/BUILD",
        &[r#"load(":custom_rule.bzl", "java_custom_library")

java_custom_library(name = "custom")
"#],
    );
    t.remove_fail_fast_handler();
    t.get_configured_target("//foo:custom");
    t.assert_contains_event("got unexpected keyword argument: include_compilation_info");
}

// Java: JavaStarlarkApiTest.testCompileWithResourceJarsIsPrivateApi
#[test]
fn test_compile_with_resource_jars_is_private_api() {
    let mut t = new_test();
    write_build_file_for_java_toolchain(&t);
    t.scratch.file("foo/custom_rule.bzl", &["load('@rules_java//java:defs.bzl', 'java_common')", "def _impl(ctx):", "  java_toolchain = ctx.attr._java_toolchain[java_common.JavaToolchainInfo]", "  java_common.compile(", "    ctx,", "    output = ctx.actions.declare_file('output.jar'),", "    java_toolchain = java_toolchain,", "    resource_jars = ['foo.jar'],", "  )", "  return []", "java_custom_library = rule(", "  implementation = _impl,", "  attrs = {", "    'srcs': attr.label_list(),", "    '_java_toolchain': attr.label(default = Label('//java/com/google/test:toolchain')),", "  },", "  toolchains = ['@@bazel_tools//tools/jdk:toolchain_type'],", "  fragments = ['java']", ")"]);
    t.scratch.file(
        "foo/BUILD",
        &[r#"load(":custom_rule.bzl", "java_custom_library")

java_custom_library(name = "custom")
"#],
    );
    t.remove_fail_fast_handler();
    t.get_configured_target("//foo:custom");
    t.assert_contains_event("got unexpected keyword argument: resource_jars");
}

// Java: JavaStarlarkApiTest.testRunIjarWithOutputParameterIsPrivateApi
#[test]
fn test_run_ijar_with_output_parameter_is_private_api() {
    let mut t = new_test();
    write_build_file_for_java_toolchain(&t);
    t.scratch.file("foo/custom_rule.bzl", &["load('@rules_java//java:defs.bzl', 'java_common')", "def _impl(ctx):", "  java_toolchain = ctx.attr._java_toolchain[java_common.JavaToolchainInfo]", "  java_common.run_ijar(", "    ctx.actions,", "    jar = ctx.actions.declare_file('input.jar'),", "    output = ctx.actions.declare_file('output.jar'),", "    java_toolchain = java_toolchain,", "  )", "  return []", "java_custom_library = rule(", "  implementation = _impl,", "  attrs = {", "    '_java_toolchain': attr.label(default = Label('//java/com/google/test:toolchain')),", "  },", "  toolchains = ['@@bazel_tools//tools/jdk:toolchain_type'],", "  fragments = ['java']", ")"]);
    t.scratch.file(
        "foo/BUILD",
        &[r#"load(":custom_rule.bzl", "java_custom_library")

java_custom_library(name = "custom")
"#],
    );
    t.remove_fail_fast_handler();
    t.get_configured_target("//foo:custom");
    t.assert_contains_event("got unexpected keyword argument: output");
}

// Java: JavaStarlarkApiTest.testProviderValidationPrintsProviderName
#[test]
fn test_provider_validation_prints_provider_name() {
    let mut t = new_test();
    t.scratch.file(
        "foo/rule.bzl",
        &[
            r#"load("@rules_java//java/common:java_info.bzl", "JavaInfo")
load("@rules_cc//cc/common:cc_info.bzl", "CcInfo")

def _impl(ctx):
    cc_info = ctx.attr.dep[CcInfo]
    JavaInfo(output_jar = None, compile_jar = None, deps = [cc_info])
    return []

myrule = rule(
    implementation = _impl,
    attrs = {"dep": attr.label()},
    fragments = [],
)
"#,
        ],
    );
    t.scratch.file(
        "foo/BUILD",
        &[r#"load("@rules_cc//cc:cc_library.bzl", "cc_library")
load(":rule.bzl", "myrule")

cc_library(name = "cc_lib")

myrule(
    name = "myrule",
    dep = ":cc_lib",
)
"#],
    );
    t.remove_fail_fast_handler();
    t.get_configured_target("//foo:myrule");
    t.assert_contains_event("got element of type CcInfo, want JavaInfo");
}

// Java: JavaStarlarkApiTest.javaInfoConstructorWithNeverlink
#[test]
#[ignore = "needs the test-side JavaInfo wrappers (src/test/java/.../rules/java/JavaInfo.java, JavaCompilationArgsProvider, JavaRuleOutputJarsProvider, ...) and the parameterized platforms (JavaTestUtil.setupPlatform with MockCcSupport.setupCcToolchainConfigForCpu) in the harness"]
fn java_info_constructor_with_neverlink() {}

// Java: JavaStarlarkApiTest.javaCommonMergeWithNeverlink
#[test]
#[ignore = "needs the test-side JavaInfo wrappers (src/test/java/.../rules/java/JavaInfo.java, JavaCompilationArgsProvider, JavaRuleOutputJarsProvider, ...) and the parameterized platforms (JavaTestUtil.setupPlatform with MockCcSupport.setupCcToolchainConfigForCpu) in the harness"]
fn java_common_merge_with_neverlink() {}

// Java: JavaStarlarkApiTest.javaCommonCompileWithNeverlink
#[test]
#[ignore = "needs the test-side JavaInfo wrappers (src/test/java/.../rules/java/JavaInfo.java, JavaCompilationArgsProvider, JavaRuleOutputJarsProvider, ...) and the parameterized platforms (JavaTestUtil.setupPlatform with MockCcSupport.setupCcToolchainConfigForCpu) in the harness"]
fn java_common_compile_with_neverlink() {}

// Java: JavaStarlarkApiTest.javaCommonCompile_nativeLibrariesPropagate
#[test]
#[ignore = "needs the test-side JavaInfo wrappers (src/test/java/.../rules/java/JavaInfo.java, JavaCompilationArgsProvider, JavaRuleOutputJarsProvider, ...) and the parameterized platforms (JavaTestUtil.setupPlatform with MockCcSupport.setupCcToolchainConfigForCpu) in the harness"]
fn java_common_compile_native_libraries_propagate() {}

// Java: JavaStarlarkApiTest.javaCommonCompile_directNativeLibraries
#[test]
#[ignore = "needs the test-side JavaInfo wrappers (src/test/java/.../rules/java/JavaInfo.java, JavaCompilationArgsProvider, JavaRuleOutputJarsProvider, ...) and the parameterized platforms (JavaTestUtil.setupPlatform with MockCcSupport.setupCcToolchainConfigForCpu) in the harness"]
fn java_common_compile_direct_native_libraries() {}

// Java: JavaStarlarkApiTest.strictDepsEnabled
#[test]
#[ignore = "needs the test-side JavaInfo wrappers (src/test/java/.../rules/java/JavaInfo.java, JavaCompilationArgsProvider, JavaRuleOutputJarsProvider, ...) and the parameterized platforms (JavaTestUtil.setupPlatform with MockCcSupport.setupCcToolchainConfigForCpu) in the harness"]
fn strict_deps_enabled() {}

// Java: JavaStarlarkApiTest.strictDepsDisabled
#[test]
#[ignore = "needs the test-side JavaInfo wrappers (src/test/java/.../rules/java/JavaInfo.java, JavaCompilationArgsProvider, JavaRuleOutputJarsProvider, ...) and the parameterized platforms (JavaTestUtil.setupPlatform with MockCcSupport.setupCcToolchainConfigForCpu) in the harness"]
fn strict_deps_disabled() {}

// Java: JavaStarlarkApiTest.strictJavaDepsFlagExposed_default
#[test]
#[ignore = "needs the test-side JavaInfo wrappers (src/test/java/.../rules/java/JavaInfo.java, JavaCompilationArgsProvider, JavaRuleOutputJarsProvider, ...) and the parameterized platforms (JavaTestUtil.setupPlatform with MockCcSupport.setupCcToolchainConfigForCpu) in the harness"]
fn strict_java_deps_flag_exposed_default() {}

// Java: JavaStarlarkApiTest.strictJavaDepsFlagExposed_error
#[test]
#[ignore = "needs the test-side JavaInfo wrappers (src/test/java/.../rules/java/JavaInfo.java, JavaCompilationArgsProvider, JavaRuleOutputJarsProvider, ...) and the parameterized platforms (JavaTestUtil.setupPlatform with MockCcSupport.setupCcToolchainConfigForCpu) in the harness"]
fn strict_java_deps_flag_exposed_error() {}

// Java: JavaStarlarkApiTest.mergeRuntimeOutputJarsTest
#[test]
#[ignore = "needs the test-side JavaInfo wrappers (src/test/java/.../rules/java/JavaInfo.java, JavaCompilationArgsProvider, JavaRuleOutputJarsProvider, ...) and the parameterized platforms (JavaTestUtil.setupPlatform with MockCcSupport.setupCcToolchainConfigForCpu) in the harness"]
fn merge_runtime_output_jars_test() {}

// Java: JavaStarlarkApiTest.javaToolchainFlag_default
#[test]
#[ignore = "needs the test-side JavaInfo wrappers (src/test/java/.../rules/java/JavaInfo.java, JavaCompilationArgsProvider, JavaRuleOutputJarsProvider, ...) and the parameterized platforms (JavaTestUtil.setupPlatform with MockCcSupport.setupCcToolchainConfigForCpu) in the harness"]
fn java_toolchain_flag_default() {}

// Java: JavaStarlarkApiTest.javaToolchainFlag_set
#[test]
#[ignore = "needs the test-side JavaInfo wrappers (src/test/java/.../rules/java/JavaInfo.java, JavaCompilationArgsProvider, JavaRuleOutputJarsProvider, ...) and the parameterized platforms (JavaTestUtil.setupPlatform with MockCcSupport.setupCcToolchainConfigForCpu) in the harness"]
fn java_toolchain_flag_set() {}

// Java: JavaStarlarkApiTest.testCompileOutputJarHasManifestProto
#[test]
#[ignore = "needs the test-side JavaInfo wrappers (src/test/java/.../rules/java/JavaInfo.java, JavaCompilationArgsProvider, JavaRuleOutputJarsProvider, ...) and the parameterized platforms (JavaTestUtil.setupPlatform with MockCcSupport.setupCcToolchainConfigForCpu) in the harness"]
fn test_compile_output_jar_has_manifest_proto() {}

// Java: JavaStarlarkApiTest.testCompileWithNeverlinkDeps
#[test]
#[ignore = "needs the test-side JavaInfo wrappers (src/test/java/.../rules/java/JavaInfo.java, JavaCompilationArgsProvider, JavaRuleOutputJarsProvider, ...) and the parameterized platforms (JavaTestUtil.setupPlatform with MockCcSupport.setupCcToolchainConfigForCpu) in the harness"]
fn test_compile_with_neverlink_deps() {}

// Java: JavaStarlarkApiTest.testCompileOutputJarNotInRuntimePathWithoutAnySourcesDefined
#[test]
#[ignore = "needs the test-side JavaInfo wrappers (src/test/java/.../rules/java/JavaInfo.java, JavaCompilationArgsProvider, JavaRuleOutputJarsProvider, ...) and the parameterized platforms (JavaTestUtil.setupPlatform with MockCcSupport.setupCcToolchainConfigForCpu) in the harness"]
fn test_compile_output_jar_not_in_runtime_path_without_any_sources_defined() {}

// Java: JavaStarlarkApiTest.testConfiguredTargetToolchain
#[test]
#[ignore = "needs the test-side JavaInfo wrappers (src/test/java/.../rules/java/JavaInfo.java, JavaCompilationArgsProvider, JavaRuleOutputJarsProvider, ...) and the parameterized platforms (JavaTestUtil.setupPlatform with MockCcSupport.setupCcToolchainConfigForCpu) in the harness"]
fn test_configured_target_toolchain() {}

// Java: JavaStarlarkApiTest.defaultJavacOpts
#[test]
#[ignore = "needs the test-side JavaInfo wrappers (src/test/java/.../rules/java/JavaInfo.java, JavaCompilationArgsProvider, JavaRuleOutputJarsProvider, ...) and the parameterized platforms (JavaTestUtil.setupPlatform with MockCcSupport.setupCcToolchainConfigForCpu) in the harness"]
fn default_javac_opts() {}

// Java: JavaStarlarkApiTest.defaultJavacOpts_asDepset
#[test]
#[ignore = "needs the test-side JavaInfo wrappers (src/test/java/.../rules/java/JavaInfo.java, JavaCompilationArgsProvider, JavaRuleOutputJarsProvider, ...) and the parameterized platforms (JavaTestUtil.setupPlatform with MockCcSupport.setupCcToolchainConfigForCpu) in the harness"]
fn default_javac_opts_as_depset() {}

// Java: JavaStarlarkApiTest.defaultJavacOpts_toolchainProvider
#[test]
#[ignore = "needs the test-side JavaInfo wrappers (src/test/java/.../rules/java/JavaInfo.java, JavaCompilationArgsProvider, JavaRuleOutputJarsProvider, ...) and the parameterized platforms (JavaTestUtil.setupPlatform with MockCcSupport.setupCcToolchainConfigForCpu) in the harness"]
fn default_javac_opts_toolchain_provider() {}

// Java: JavaStarlarkApiTest.testJavaRuntimeProviderFiles
#[test]
#[ignore = "needs the test-side JavaInfo wrappers (src/test/java/.../rules/java/JavaInfo.java, JavaCompilationArgsProvider, JavaRuleOutputJarsProvider, ...) and the parameterized platforms (JavaTestUtil.setupPlatform with MockCcSupport.setupCcToolchainConfigForCpu) in the harness"]
fn test_java_runtime_provider_files() {}

// Java: JavaStarlarkApiTest.testJavaLibraryCollectsCoverageDependenciesFromResources
#[test]
#[ignore = "needs the test-side JavaInfo wrappers (src/test/java/.../rules/java/JavaInfo.java, JavaCompilationArgsProvider, JavaRuleOutputJarsProvider, ...) and the parameterized platforms (JavaTestUtil.setupPlatform with MockCcSupport.setupCcToolchainConfigForCpu) in the harness"]
fn test_java_library_collects_coverage_dependencies_from_resources() {}

// Java: JavaStarlarkApiTest.testSkipAnnotationProcessing
#[test]
#[ignore = "needs the test-side JavaInfo wrappers (src/test/java/.../rules/java/JavaInfo.java, JavaCompilationArgsProvider, JavaRuleOutputJarsProvider, ...) and the parameterized platforms (JavaTestUtil.setupPlatform with MockCcSupport.setupCcToolchainConfigForCpu) in the harness"]
fn test_skip_annotation_processing() {}

// Java: JavaStarlarkApiTest.testPackSourcesWithExternalResourceArtifact
#[test]
#[ignore = "needs the test-side JavaInfo wrappers (src/test/java/.../rules/java/JavaInfo.java, JavaCompilationArgsProvider, JavaRuleOutputJarsProvider, ...) and the parameterized platforms (JavaTestUtil.setupPlatform with MockCcSupport.setupCcToolchainConfigForCpu) in the harness"]
fn test_pack_sources_with_external_resource_artifact() {}

// Java: JavaStarlarkApiTest.mergeAddExports
#[test]
#[ignore = "needs the test-side JavaInfo wrappers (src/test/java/.../rules/java/JavaInfo.java, JavaCompilationArgsProvider, JavaRuleOutputJarsProvider, ...) and the parameterized platforms (JavaTestUtil.setupPlatform with MockCcSupport.setupCcToolchainConfigForCpu) in the harness"]
fn merge_add_exports() {}

// Java: JavaStarlarkApiTest.hermeticStaticLibs
#[test]
#[ignore = "needs the test-side JavaInfo wrappers (src/test/java/.../rules/java/JavaInfo.java, JavaCompilationArgsProvider, JavaRuleOutputJarsProvider, ...) and the parameterized platforms (JavaTestUtil.setupPlatform with MockCcSupport.setupCcToolchainConfigForCpu) in the harness"]
fn hermetic_static_libs() {}

// Java: JavaStarlarkApiTest.implicitLibCtSym
#[test]
#[ignore = "needs the test-side JavaInfo wrappers (src/test/java/.../rules/java/JavaInfo.java, JavaCompilationArgsProvider, JavaRuleOutputJarsProvider, ...) and the parameterized platforms (JavaTestUtil.setupPlatform with MockCcSupport.setupCcToolchainConfigForCpu) in the harness"]
fn implicit_lib_ct_sym() {}

// Java: JavaStarlarkApiTest.explicitLibCtSym
#[test]
#[ignore = "needs the test-side JavaInfo wrappers (src/test/java/.../rules/java/JavaInfo.java, JavaCompilationArgsProvider, JavaRuleOutputJarsProvider, ...) and the parameterized platforms (JavaTestUtil.setupPlatform with MockCcSupport.setupCcToolchainConfigForCpu) in the harness"]
fn explicit_lib_ct_sym() {}

// Java: JavaStarlarkApiTest.testNativeJavaInfoPrintableType_isJavaInfo
#[test]
#[ignore = "needs JavaInfo.EMPTY_JAVA_INFO_FOR_TESTING (the native JavaInfo is not ported; Blaze has only the Starlark JavaInfo of rules_java)"]
fn test_native_java_info_printable_type_is_java_info() {}
