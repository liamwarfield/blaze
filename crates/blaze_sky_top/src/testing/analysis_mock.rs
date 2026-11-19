//! Port of `analysis/util/AnalysisMock.java` and `analysis/mock/BazelAnalysisMock.java`
//! (src/test/java), with `packages/util/MockPlatformSupport.java` and
//! `packages/util/MockGenruleSupport.java`: the mock client the analysis tests run in.
//!
//! The files are Java's, character for character (text blocks dedented as `javac` does). What
//! Java copies from the test's runfiles and resources (the real `rules_cc`, `rules_java`,
//! `rules_python`, `bazel_skylib` and protobuf sources, the `tools/jdk` and `tools/python` files
//! of the Bazel source, the mock `cc_toolchain_config.bzl`) comes from the prepared test
//! dependencies (`test_deps.rs`, `scripts/test-deps/prepare`). Not ported:
//!
//! - `ShellConfiguration.injectShellExecutableFinder` is a no-op here (Blaze's shell
//!   configuration is computed from the options).

use blaze_bzlmod_model::module::NonRegistryOverride;
use blaze_bzlmod_model::repo_spec::{AttributeValue, AttributeValues, LOCAL_REPOSITORY, RepoSpec};
use blaze_vfs::{Path, PathFragment};
use indexmap::IndexMap;

use crate::testing::mock_cc_support::BazelMockCcSupport;
use crate::testing::mock_java_python_support::{BazelMockJavaSupport, BazelMockPythonSupport};
use crate::testing::mock_tools_config::MockToolsConfig;
use crate::testing::test_deps::test_deps;

/// `TestConstants.PRODUCT_NAME`.
pub const PRODUCT_NAME: &str = "bazel";
/// `TestConstants.TOOLS_REPOSITORY_SCRATCH`.
pub const TOOLS_REPOSITORY_SCRATCH: &str = "embedded_tools/";
/// `TestConstants.CONSTRAINTS_PACKAGE_ROOT`.
pub const CONSTRAINTS_PACKAGE_ROOT: &str = "@platforms//";
/// `TestConstants.PLATFORMS_PATH`.
pub const PLATFORMS_PATH: &str = "embedded_tools/platforms";
/// `TestConstants.CONSTRAINTS_PATH`.
pub const CONSTRAINTS_PATH: &str = "platforms_workspace";
/// `TestConstants.PLATFORM_LABEL`.
pub const PLATFORM_LABEL: &str = "@platforms//host";
/// `TestConstants.PIII_PLATFORM_LABEL`.
pub const PIII_PLATFORM_LABEL: &str = "@platforms//host:piii";
/// `TestConstants.PRODUCT_SPECIFIC_FLAGS`.
// Java: com/google/devtools/build/lib/testutil/TestConstants.java:152 TestConstants.PRODUCT_SPECIFIC_FLAGS
pub const PRODUCT_SPECIFIC_FLAGS: &[&str] = &[
    "--platforms=@platforms//host",
    "--host_platform=@platforms//host",
    // TODO(#7849): Remove after flag flip.
    "--incompatible_use_toolchain_resolution_for_java_rules",
    "--incompatible_disable_select_on=cpu,crosstool_top,host_cpu",
];
/// `TestConstants.PRODUCT_SPECIFIC_BUILD_LANG_OPTIONS`.
// Java: com/google/devtools/build/lib/testutil/TestConstants.java:160 TestConstants.PRODUCT_SPECIFIC_BUILD_LANG_OPTIONS
pub const PRODUCT_SPECIFIC_BUILD_LANG_OPTIONS: &[&str] = &[
    // Don't apply autoloads in unit tests, because not all repos are available
    "--incompatible_autoload_externally=",
];

/// Java `BazelAnalysisMock` (the only `AnalysisMock` of Bazel's tests).
// Java: com/google/devtools/build/lib/analysis/mock/BazelAnalysisMock.java:51 BazelAnalysisMock
pub struct BazelAnalysisMock;

impl BazelAnalysisMock {
    // Java: com/google/devtools/build/lib/analysis/util/AnalysisMock.java:84 AnalysisMock.getProductName
    pub fn get_product_name(&self) -> &'static str {
        PRODUCT_NAME
    }

    /// This is called from test setup to create the mock directory layout needed to create the
    /// configuration.
    // Java: com/google/devtools/build/lib/analysis/util/AnalysisMock.java:103 AnalysisMock.setupMockClient
    pub fn setup_mock_client(&self, config: &MockToolsConfig) {
        self.setup_mock_client_internal(config);
        self.setup_mock_testing_rules(config);
    }

    // Java: com/google/devtools/build/lib/analysis/util/AnalysisMock.java:110 AnalysisMock.setupMockTestingRules
    pub fn setup_mock_testing_rules(&self, config: &MockToolsConfig) {
        config.create("test_defs/BUILD", &[]);
        config.create(
            "test_defs/foo_library.bzl",
            &[r#"def _impl(ctx):
  pass
foo_library = rule(
  implementation = _impl,
  attrs = {
    "srcs": attr.label_list(allow_files=True),
    "deps": attr.label_list(),
  },
)
"#],
        );
        config.create(
            "test_defs/foo_binary.bzl",
            &[r#"def _impl(ctx):
  symlink = ctx.actions.declare_file(ctx.label.name)
  ctx.actions.symlink(output = symlink, target_file = ctx.files.srcs[0],
    is_executable = True)
  files = depset(ctx.files.srcs)
  return [DefaultInfo(files = files, executable = symlink,
     runfiles = ctx.runfiles(transitive_files = files, collect_default = True))]
foo_binary = rule(
  implementation = _impl,
  executable = True,
  attrs = {
    "srcs": attr.label_list(allow_files=True),
    "deps": attr.label_list(),
    "data": attr.label_list(allow_files=True),
  },
)
"#],
        );
        config.create(
            "test_defs/foo_test.bzl",
            &[r#"def _impl(ctx):
  symlink = ctx.actions.declare_file(ctx.label.name)
  ctx.actions.symlink(output = symlink, target_file = ctx.files.srcs[0],
    is_executable = True)
  files = depset(ctx.files.srcs)
  return [DefaultInfo(files = files, executable = symlink,
     runfiles = ctx.runfiles(transitive_files = files, collect_default = True))]
foo_test = rule(
  implementation = _impl,
  test = True,
  attrs = {
    "srcs": attr.label_list(allow_files=True),
    "deps": attr.label_list(),
    "data": attr.label_list(allow_files=True),
  },
)
"#],
        );
    }

    // Java: com/google/devtools/build/lib/analysis/mock/BazelAnalysisMock.java:56 BazelAnalysisMock.setupMockClientInternal
    pub fn setup_mock_client_internal(&self, config: &MockToolsConfig) {
        config.create(
            "local_config_xcode_workspace/BUILD",
            &[
                "load('@build_bazel_apple_support//xcode:xcode_config.bzl', 'xcode_config')",
                "xcode_config(name = 'host_xcodes')",
            ],
        );
        MockObjcSupport::setup_xcode_rules(config);
        config.create(
            "local_config_xcode_workspace/MODULE.bazel",
            &["module(name = 'local_config_xcode')"],
        );
        config.create("third_party/protobuf/BUILD", &[]);
        config.create(
            "third_party/protobuf/MODULE.bazel",
            &["module(name='com_google_protobuf')"],
        );
        config.overwrite(
            "MODULE.bazel",
            &[
                "register_toolchains('@rules_java//java/toolchains/runtime:all')",
                "register_toolchains('@rules_java//java/toolchains/javac:all')",
                "register_toolchains('@bazel_tools//tools/cpp:all')",
                "register_toolchains('@bazel_tools//tools/jdk:all')",
                "register_toolchains('@bazel_tools//tools/python:autodetecting_toolchain')",
            ],
        );
        /* The rest of platforms is initialized in {@link MockPlatformSupport}. */
        config.create(
            "platforms_workspace/MODULE.bazel",
            &["module(name = 'platforms')"],
        );
        config.create(
            "build_bazel_apple_support/MODULE.bazel",
            &["module(name = 'build_bazel_apple_support')"],
        );
        config.create(
            "third_party/bazel_rules/rules_shell/MODULE.bazel",
            &["module(name = 'rules_shell')"],
        );

        // TODO: remove after figuring out https://github.com/bazelbuild/bazel/issues/22208
        config.create(
            ".bazelignore",
            &[
                "embedded_tools",
                "platforms_workspace",
                "rules_java_workspace",
                "rules_python_workspace",
                "third_party/protobuf",
                "proto_bazel_features_workspace",
                "bazel_features_workspace",
                "build_bazel_apple_support",
                "local_config_xcode_workspace",
                "third_party/bazel_rules/rules_cc",
                "third_party/bazel_rules/rules_shell",
            ],
        );

        // runfiles.rlocation("io_bazel/" + filename): a file of the pinned Bazel source.
        for filename in ["tools/jdk/java_toolchain_alias.bzl"] {
            let deps = test_deps();
            let content = deps.read_resource(&deps.resource(filename));
            config.create(&format!("embedded_tools/{filename}"), &[&content]);
        }
        config.create(
            "embedded_tools/tools/jdk/launcher_flag_alias.bzl",
            &[r#"load("@rules_cc//cc/common:cc_common.bzl", "cc_common")
load("@rules_cc//cc/common:cc_info.bzl", "CcInfo")

_providers = [CcInfo, cc_common.launcher_provider]

def _impl(ctx):
        if not ctx.attr._launcher:
            return None
        launcher = ctx.attr._launcher
        providers = [ctx.attr._launcher[p] for p in _providers]
        providers.append(DefaultInfo(
            files = launcher[DefaultInfo].files,
            runfiles = launcher[DefaultInfo].default_runfiles,
        ))
        return providers

launcher_flag_alias = rule(
        implementation = _impl,
        attrs = {
            "_launcher": attr.label(
                default = configuration_field(
                    fragment = "java",
                    name = "launcher",
                ),
                providers = _providers,
            ),
        },
)
"#],
        );
        config.create(
            "embedded_tools/tools/jdk/BUILD",
            &[r#"load("@rules_java//java:defs.bzl",
  "java_binary", "java_import", "java_toolchain", "java_runtime")
load(
        ":java_toolchain_alias.bzl",
        "java_host_runtime_alias",
        "java_runtime_alias",
        "java_toolchain_alias",
)
load(":launcher_flag_alias.bzl", "launcher_flag_alias")

package(default_visibility = ["//visibility:public"])

java_toolchain(
        name = "toolchain",
        bootclasspath = [":bootclasspath"],
        genclass = ["GenClass_deploy.jar"],
        header_compiler = ["turbine_deploy.jar"],
        header_compiler_direct = ["TurbineDirect_deploy.jar"],
        ijar = ["ijar"],
        jacocorunner = ":JacocoCoverage",
        java_runtime = "host_jdk",
        javabuilder = ["JavaBuilder_deploy.jar"],
        singlejar = ["singlejar"],
        source_version = "8",
        target_version = "8",
)

java_toolchain(
        name = "remote_toolchain",
        bootclasspath = [":bootclasspath"],
        genclass = ["GenClass_deploy.jar"],
        header_compiler = ["turbine_deploy.jar"],
        header_compiler_direct = ["TurbineDirect_deploy.jar"],
        ijar = ["ijar"],
        jacocorunner = ":JacocoCoverage",
        java_runtime = "host_jdk",
        javabuilder = ["JavaBuilder_deploy.jar"],
        singlejar = ["singlejar"],
        source_version = "8",
        target_version = "8",
)

java_import(
        name = "JacocoCoverageRunner",
        jars = ["JacocoCoverage_jarjar_deploy.jar"],
)

java_import(
        name = "proguard_import",
        jars = ["proguard_rt.jar"],
)

java_binary(
        name = "proguard",
        main_class = "proguard.Proguard",
        runtime_deps = [":proguard_import"],
)

java_import(
        name = "TestRunner",
        jars = ["TestRunner.jar"],
)

java_runtime(
        name = "jdk",
        srcs = [],
)

java_runtime(
        name = "host_jdk",
        srcs = [],
)

java_runtime(
        name = "remote_jdk11",
        srcs = [],
)

java_toolchain_alias(name = "current_java_toolchain")

java_runtime_alias(name = "current_java_runtime")

java_host_runtime_alias(name = "current_host_java_runtime")

filegroup(
        name = "bootclasspath",
        srcs = ["jdk/jre/lib/rt.jar"],
)

filegroup(
        name = "extdir",
        srcs = glob(
            ["jdk/jre/lib/ext/*"],
            allow_empty = True,
        ),
)

filegroup(
        name = "java",
        srcs = ["jdk/jre/bin/java"],
)

filegroup(
        name = "JacocoCoverage",
        srcs = ["JacocoCoverage_deploy.jar"],
)

exports_files([
        "JavaBuilder_deploy.jar",
        "singlejar",
        "TestRunner_deploy.jar",
        "ijar",
        "GenClass_deploy.jar",
        "turbine_deploy.jar",
        "TurbineDirect_deploy.jar",
        "proguard_allowlister.par",
])

toolchain_type(name = "toolchain_type")

toolchain_type(name = "runtime_toolchain_type")

toolchain(
        name = "dummy_java_toolchain",
        toolchain = ":toolchain",
        toolchain_type = ":toolchain_type",
)

toolchain(
        name = "dummy_java_runtime_toolchain",
        toolchain = ":jdk",
        toolchain_type = ":runtime_toolchain_type",
)

java_plugins_flag_alias(name = "java_plugins_flag_alias")

launcher_flag_alias(
        name = "launcher_flag_alias",
        visibility = ["//visibility:public"],
)
"#],
        );

        config.create(
            &format!("{CONSTRAINTS_PATH}/android/BUILD"),
            &[
                "package(default_visibility=['//visibility:public'])",
                "platform(",
                "  name = 'armeabi-v7a',",
                &format!("  parents = ['{PLATFORM_LABEL}'],"),
                "  constraint_values = [",
                &format!("    '{CONSTRAINTS_PACKAGE_ROOT}os:android',"),
                &format!("    '{CONSTRAINTS_PACKAGE_ROOT}cpu:armv7',"),
                "  ],",
                ")",
            ],
        );

        // Create the actual SDKs.
        config.create(
            "embedded_tools/src/tools/android/java/com/google/devtools/build/android/r8/BUILD",
            &["filegroup(name='r8', srcs = [])\n"],
        );
        config.create(
            "android_gmaven_r8/jar/BUILD",
            &["filegroup(name = \"jar\", srcs = [\"r8.jar\"])\nfilegroup(name = \"file\", srcs = [])\n"],
        );
        config.create("android_gmaven_r8/REPO.bazel", &[]);

        MockGenruleSupport::setup(config);

        config.create(
            "embedded_tools/tools/BUILD",
            &[&format!(
                "alias(name='host_platform',actual='{PLATFORM_LABEL}')"
            )],
        );
        // Contains a stripped down version of @bazel_tools//tools/test.
        config.create(
            "embedded_tools/tools/test/BUILD",
            &[
                r#"load(":default_test_toolchain.bzl", "bool_flag", "empty_toolchain")

toolchain_type(
    name = "default_test_toolchain_type",
)

empty_toolchain(name = "empty_toolchain")

bool_flag(
    name = "incompatible_use_default_test_toolchain",
    build_setting_default = True,
    visibility = ["//visibility:private"],
)

config_setting(
    name = "use_default_test_toolchain",
    values = {
        "use_target_platform_for_tests": "false",
    },
    flag_values = {
        ":incompatible_use_default_test_toolchain": "true",
    },
    visibility = ["//visibility:private"],
)

config_setting(
    name = "use_legacy_test_toolchain_due_to_use_target_platform_for_tests",
    values = {
        "use_target_platform_for_tests": "true",
    },
    visibility = ["//visibility:private"],
)

config_setting(
    name = "use_legacy_test_toolchain_due_to_incompatible_flag",
    flag_values = {
        ":incompatible_use_default_test_toolchain": "false",
    },
    visibility = ["//visibility:private"],
)

toolchain(
    name = "default_test_toolchain",
    toolchain_type = ":default_test_toolchain_type",
    use_target_platform_constraints = True,
    target_settings = [":use_default_test_toolchain"],
    toolchain = ":empty_toolchain",
    visibility = ["//visibility:private"],
)

toolchain(
    name = "legacy_test_toolchain",
    toolchain_type = ":default_test_toolchain_type",
    target_settings = [":use_legacy_test_toolchain_due_to_incompatible_flag"],
    toolchain = ":empty_toolchain",
    visibility = ["//visibility:private"],
)

toolchain(
    name = "legacy_test_toolchain_use_target_platform_for_tests",
    toolchain_type = ":default_test_toolchain_type",
    target_settings = [":use_legacy_test_toolchain_due_to_use_target_platform_for_tests"],
    toolchain = ":empty_toolchain",
    visibility = ["//visibility:private"],
)

filegroup(
    name = "runtime",
    srcs = [
        "test-setup.sh",
        "test-xml-generator.sh",
    ],
)

filegroup(
    name = "test_wrapper",
    srcs = ["test_wrapper_bin"],
)

filegroup(
    name = "xml_writer",
    srcs = ["xml_writer_bin"],
)

filegroup(
    name = "test_setup",
    srcs = ["test-setup.sh"],
)

filegroup(
    name = "test_xml_generator",
    srcs = ["test-xml-generator.sh"],
)

filegroup(
    name = "collect_coverage",
    srcs = ["collect_coverage.sh"],
)

filegroup(
    name = "collect_cc_coverage",
    srcs = ["collect_cc_coverage.sh"],
)

filegroup(
    name = "coverage_support",
    srcs = ["collect_coverage.sh"],
)

filegroup(
    name = "coverage_report_generator",
    srcs = ["coverage_report_generator.sh"],
)

filegroup(
    name = "lcov_merger",
    srcs = ["lcov_merger.sh"],
)
"#,
            ],
        );
        config.create(
            "embedded_tools/tools/test/default_test_toolchain.bzl",
            &[r#"visibility("private")

bool_flag = rule(
    implementation = lambda _: None,
    build_setting = config.bool(flag = True),
    doc = "A bool-typed build setting that can be set on the command line",
)

empty_toolchain = rule(
    implementation = lambda ctx: platform_common.ToolchainInfo(),
)
"#],
        );

        // Create fake, minimal implementations of test-setup.sh and test-xml-generator.sh for
        // test cases that actually execute tests. Does not support coverage, interruption,
        // signals, etc. For proper test execution support, the actual test-setup.sh will need to
        // be included in the Java test's runfiles and copied/symlinked into the MockToolsConfig's
        // workspace.
        config.create(
            "embedded_tools/tools/test/test-setup.sh",
            &[r#"#!/bin/bash
set -e
function is_absolute {
  [[ "$1" = /* ]] || [[ "$1" =~ ^[a-zA-Z]:[/\].* ]]
}
is_absolute "$TEST_SRCDIR" || TEST_SRCDIR="$PWD/$TEST_SRCDIR"
RUNFILES_MANIFEST_FILE="${TEST_SRCDIR}/MANIFEST"
cd ${TEST_SRCDIR}
function rlocation() {
  if is_absolute "$1" ; then
    # If the file path is already fully specified, simply return it.
    echo "$1"
  elif [[ -e "$TEST_SRCDIR/$1" ]]; then
    # If the file exists in the $TEST_SRCDIR then just use it.
    echo "$TEST_SRCDIR/$1"
  elif [[ -e "$RUNFILES_MANIFEST_FILE" ]]; then
    # If a runfiles manifest file exists then use it.
    echo "$(grep "^$1 " "$RUNFILES_MANIFEST_FILE" | sed 's/[^ ]* //')"
  fi
}

EXE="${1#./}"
shift

if is_absolute "$EXE"; then
  TEST_PATH="$EXE"
else
  TEST_PATH="$(rlocation $TEST_WORKSPACE/$EXE)"
fi
exec $TEST_PATH
"#],
        );
        config.chmod("embedded_tools/tools/test/test-setup.sh", 0o755);
        config.create(
            "embedded_tools/tools/test/test-xml-generator.sh",
            &["#!/bin/sh", "cp \"$1\" \"$2\""],
        );
        config.chmod("embedded_tools/tools/test/test-xml-generator.sh", 0o755);

        // Use an alias package group to allow for modification at the simpler path
        config.create(
            "embedded_tools/tools/allowlists/config_feature_flag/BUILD",
            &[r#"package_group(
    name = "config_feature_flag",
    includes = ["@@//tools/allowlists/config_feature_flag"],
)

package_group(
    name = "config_feature_flag_setter",
    includes = ["@@//tools/allowlists/config_feature_flag:config_feature_flag_setter"],
)
"#],
        );

        config.create(
            "tools/allowlists/config_feature_flag/BUILD",
            &[r#"package_group(
    name = "config_feature_flag",
    packages = ["public"],
)

package_group(
    name = "config_feature_flag_Setter",
    packages = ["public"],
)
"#],
        );
        config.create(
            "embedded_tools/tools/allowlists/initializer_allowlist/BUILD",
            &[r#"package_group(
    name = "initializer_allowlist",
    packages = [],
)
"#],
        );
        config.create(
            "embedded_tools/tools/allowlists/extend_rule_allowlist/BUILD",
            &[r#"package_group(
    name = "extend_rule_allowlist",
    packages = ["public"],
)
package_group(
    name = "extend_rule_api_allowlist",
    packages = [],
)
"#],
        );
        config.create(
            "embedded_tools/tools/allowlists/subrules_allowlist/BUILD",
            &[r#"package_group(
    name = "subrules_allowlist",
    packages = [],
)
"#],
        );

        config.create(
            "embedded_tools/tools/allowlists/android_binary_allowlist/BUILD",
            &[r#"package_group(
    name = "enable_starlark_dex_desugar_proguard",
    includes = [
    "@@//tools/allowlists/android_binary_allowlist:enable_starlark_dex_desugar_proguard",
    ],
)
"#],
        );
        config.create(
            "tools/allowlists/android_binary_allowlist/BUILD",
            &["package_group(name='enable_starlark_dex_desugar_proguard', packages=[])"],
        );

        config.create(
            "embedded_tools/tools/proto/BUILD",
            &[r#"package(default_visibility = ["//visibility:public"])

alias(
    name = "protoc",
    actual = "@com_google_protobuf//:protoc",
)

alias(
    name = "javalite_toolchain",
    actual = "@com_google_protobuf//:javalite_toolchain",
)

alias(
    name = "java_toolchain",
    actual = "@com_google_protobuf//:java_toolchain",
)

alias(
    name = "cc_toolchain",
    actual = "@com_google_protobuf//:cc_toolchain",
)
"#],
        );

        config.create(
            "embedded_tools/tools/zip/BUILD",
            &[
                r#"load("@bazel_tools//third_party/cc_rules/macros:defs.bzl", "cc_binary")

package(default_visibility = ["//visibility:public"])

exports_files(["precompile.py"])

cc_binary(
    name = "zipper",
    srcs = ["zip_main.cc"],
)

alias(
    name = "unzip_fdo",
    actual = ":zipper",
)
"#,
            ],
        );

        config.create(
            "embedded_tools/tools/launcher/BUILD",
            &[
                r#"load("@bazel_tools//third_party/cc_rules/macros:defs.bzl", "cc_binary")

package(default_visibility = ["//visibility:public"])

cc_binary(
    name = "launcher",
    srcs = ["launcher_main.cc"],
)

cc_binary(
    name = "launcher_maker",
    srcs = ["launcher_maker.cc"],
)
"#,
            ],
        );

        config.create(
            "embedded_tools/tools/def_parser/BUILD",
            &[r#"package(default_visibility = ["//visibility:public"])

filegroup(
    name = "def_parser",
    srcs = ["def_parser.exe"],
)
"#],
        );

        config.create(
            "embedded_tools/objcproto/BUILD",
            &[r#"package(default_visibility = ["//visibility:public"])

objc_library(
    name = "protobuf_lib",
    srcs = ["empty.m"],
    hdrs = ["include/header.h"],
    includes = ["include"],
)

exports_files(["well_known_type.proto"])

proto_library(
    name = "well_known_type_proto",
        srcs = ["well_known_type.proto"],
    )
"#],
        );
        config.create("embedded_tools/objcproto/empty.m", &[]);
        config.create("embedded_tools/objcproto/empty.cc", &[]);
        config.create("embedded_tools/objcproto/well_known_type.proto", &[]);

        // Copies bazel_skylib from real @bazel_skylib (needed by rules_python)
        config.copy_directory(
            &test_deps().repository("bazel_skylib"),
            "bazel_skylib_workspace",
            usize::MAX,
            true,
        );
        config.overwrite(
            "bazel_skylib_workspace/MODULE.bazel",
            &["module(name = 'bazel_skylib')"],
        );

        config.create(
            "embedded_tools/tools/allowlists/function_transition_allowlist/BUILD",
            &[r#"package_group(
    name = "function_transition_allowlist",
    packages = ["public"],
)
"#],
        );

        config.create(
            "embedded_tools/tools/allowlists/dormant_dependency_allowlist/BUILD",
            &[r#"package_group(
    name = "dormant_dependency_allowlist",
    packages = ["public"],
)
"#],
        );
        config.create(
            "bazel_features_workspace/MODULE.bazel",
            &["module(name = 'bazel_features')"],
        );
        config.create("bazel_features_workspace/BUILD", &[]);
        config.create(
            "bazel_features_workspace/features.bzl",
            &[r#"bazel_features = struct(
  rules = struct(
    _has_launcher_maker_toolchain = False,
  ),
)
"#],
        );

        config.create(
            "embedded_tools/tools/allowlists/materializer_rule_allowlist/BUILD",
            &[r#"package_group(
    name = "materializer_rule_allowlist",
    packages = ["public"],
)
package_group(
  name = 'materializer_rule_real_deps_allowlist',
  packages = ["public"],
)
"#],
        );

        MockProtoSupport::setup_workspace(config);
        MockPlatformSupport::setup(config);
        BazelMockCcSupport.setup(config);
        BazelMockJavaSupport.setup_rules_java(config);
        BazelMockPythonSupport.setup(config);
        // PORT: ShellConfiguration.injectShellExecutableFinder is a no-op here (module
        // documentation).
    }

    /// Creates a mock tools repository.
    // Java: com/google/devtools/build/lib/analysis/mock/BazelAnalysisMock.java:774 BazelAnalysisMock.setupMockToolsRepository
    pub fn setup_mock_tools_repository(&self, config: &MockToolsConfig) {
        config.create(
            "embedded_tools/MODULE.bazel",
            &["module(name='bazel_tools')\nregister_toolchains(\"//tools/test:all\")\n"],
        );
        config.create("embedded_tools/tools/build_defs/repo/BUILD", &[]);
        config.create(
            "embedded_tools/tools/build_defs/build_info/bazel_cc_build_info.bzl",
            &[r#"def _impl(ctx):
    volatile_file = ctx.actions.declare_file("volatile_file.h")
    non_volatile_file = ctx.actions.declare_file("non_volatile_file.h")
    redacted_file = ctx.actions.declare_file("redacted_file.h")
    ctx.actions.write(output = volatile_file, content = "")
    ctx.actions.write(output = non_volatile_file, content = "")
    ctx.actions.write(output = redacted_file, content = "")
    output_groups = {
        "non_redacted_build_info_files": depset([volatile_file, non_volatile_file]),
        "redacted_build_info_files": depset([redacted_file]),
    }
    return OutputGroupInfo(**output_groups)

bazel_cc_build_info = rule(implementation = _impl)
"#],
        );
        config.create(
            "embedded_tools/tools/build_defs/build_info/bazel_java_build_info.bzl",
            &[r#"def _impl(ctx):
    volatile_file = ctx.actions.declare_file("volatile_file.properties")
    non_volatile_file = ctx.actions.declare_file("non_volatile_file.properties")
    redacted_file = ctx.actions.declare_file("redacted_file.properties")
    ctx.actions.write(output = volatile_file, content = "")
    ctx.actions.write(output = non_volatile_file, content = "")
    ctx.actions.write(output = redacted_file, content = "")
    output_groups = {
        "non_redacted_build_info_files": depset([volatile_file, non_volatile_file]),
        "redacted_build_info_files": depset([redacted_file]),
    }
    return OutputGroupInfo(**output_groups)

bazel_java_build_info = rule(implementation = _impl)
"#],
        );
        config.create(
            "embedded_tools/tools/build_defs/build_info/BUILD",
            &[r#"load("//tools/build_defs/build_info:bazel_cc_build_info.bzl", "bazel_cc_build_info")
load("//tools/build_defs/build_info:bazel_java_build_info.bzl", "bazel_java_build_info")

bazel_cc_build_info(
    name = "cc_build_info",
    visibility = ["//visibility:public"],
)

bazel_java_build_info(
    name = "java_build_info",
    visibility = ["//visibility:public"],
)
"#],
        );
        config.create(
            "embedded_tools/tools/build_defs/repo/utils.bzl",
            &[r#"def maybe(repo_rule, name, **kwargs):
    if name not in native.existing_rules():
        repo_rule(name = name, **kwargs)
"#],
        );
        config.create(
            "embedded_tools/tools/build_defs/repo/http.bzl",
            &[r#"def http_archive(**kwargs):
    pass

def http_file(**kwargs):
    pass

def http_jar(**kwargs):
    pass
"#],
        );
        config.create(
            "embedded_tools/tools/build_defs/repo/local.bzl",
            &[r#"def _local_repository_impl(rctx):
  path = rctx.workspace_root.get_child(rctx.attr.path)
  rctx.symlink(path, ".")
local_repository = repository_rule(
  implementation = _local_repository_impl,
  attrs = {"path": attr.string()},
)

def new_local_repository(**kwargs):
    pass
"#],
        );
        config.create(
            "embedded_tools/tools/jdk/jdk_build_file.bzl",
            &["JDK_BUILD_TEMPLATE = ''"],
        );
        config.create(
            "embedded_tools/tools/jdk/local_java_repository.bzl",
            &["def local_java_repository(**kwargs):\n    pass\n"],
        );
        config.create(
            "embedded_tools/tools/jdk/remote_java_repository.bzl",
            &["def remote_java_repository(**kwargs):\n    pass\n"],
        );
        config.create(
            "embedded_tools/tools/cpp/cc_configure.bzl",
            &["def cc_configure(**kwargs):\n    pass\n"],
        );

        config.create("embedded_tools/tools/sh/BUILD", &[]);
        config.create("embedded_tools/tools/osx/BUILD", &[]);
        config.create(
            "embedded_tools/tools/osx/xcode_configure.bzl",
            &[r#"# no positional arguments for XCode
def xcode_configure(*args, **kwargs):
    pass
"#],
        );
        config.create("embedded_tools/bin/sh", &["def sh(**kwargs):", "  pass"]);
    }

    /// The built-in modules: every module of the mock, as a local path override (Java
    /// `LocalPathRepoSpecs.create(<working directory>/<path>)`).
    // Java: com/google/devtools/build/lib/analysis/mock/BazelAnalysisMock.java:865 BazelAnalysisMock.getBuiltinModules
    pub fn get_builtin_modules(
        &self,
        working_directory: &Path,
    ) -> IndexMap<String, NonRegistryOverride> {
        let module_name_to_path: [(&str, &str); 13] = [
            ("bazel_tools", "embedded_tools"),
            ("platforms", "platforms_workspace"),
            ("rules_java", "rules_java_workspace"),
            ("rules_python", "rules_python_workspace"),
            ("rules_python_internal", "rules_python_internal_workspace"),
            ("bazel_skylib", "bazel_skylib_workspace"),
            // for WORKSPACE compatibility use com_google_protobuf
            ("com_google_protobuf", "third_party/protobuf"),
            ("proto_bazel_features", "proto_bazel_features_workspace"),
            ("bazel_features", "bazel_features_workspace"),
            ("build_bazel_apple_support", "build_bazel_apple_support"),
            ("local_config_xcode", "local_config_xcode_workspace"),
            ("rules_cc", "third_party/bazel_rules/rules_cc"),
            ("rules_shell", "third_party/bazel_rules/rules_shell"),
        ];
        module_name_to_path
            .iter()
            .map(|(name, path)| {
                (
                    name.to_string(),
                    NonRegistryOverride::new(local_path_repo_spec(
                        working_directory
                            .get_relative(&PathFragment::create(path))
                            .get_path_string(),
                    )),
                )
            })
            .collect()
    }
}

/// `LocalPathRepoSpecs.create(path)`.
// Java: com/google/devtools/build/lib/bazel/bzlmod/LocalPathRepoSpecs.java:31 LocalPathRepoSpecs.create
pub fn local_path_repo_spec(path: &str) -> RepoSpec {
    RepoSpec::new(
        LOCAL_REPOSITORY.clone(),
        AttributeValues::create([("path", AttributeValue::String(path.to_string()))]),
    )
}

/// Java `MockGenruleSupport`: creates mock BUILD files required for the genrule rule.
// Java: com/google/devtools/build/lib/packages/util/MockGenruleSupport.java:20 MockGenruleSupport
pub struct MockGenruleSupport;

impl MockGenruleSupport {
    /// Sets up mocking support for genrules.
    // Java: com/google/devtools/build/lib/packages/util/MockGenruleSupport.java:22 MockGenruleSupport.setup
    pub fn setup(config: &MockToolsConfig) {
        config.create(
            "embedded_tools/tools/genrule/BUILD",
            &["exports_files(['genrule-setup.sh'])"],
        );
        config.create("embedded_tools/tools/genrule/genrule-setup.sh", &[]);
    }
}

/// Java `MockProtoSupport` (the `setupWorkspace` part).
pub struct MockProtoSupport;

impl MockProtoSupport {
    // Java: com/google/devtools/build/lib/packages/util/MockProtoSupport.java:450 MockProtoSupport.setupWorkspace
    pub fn setup_workspace(config: &MockToolsConfig) {
        // TODO - ilist@: Remove after Google proto_library doesn't depend on rules_proto
        config.create(
            "third_party/bazel_rules/rules_proto/proto/BUILD",
            &[r#"licenses(["notice"])

toolchain_type(
    name = "toolchain_type",
    visibility = ["//visibility:public"],
)
"#],
        );
        // TestConstants.PRODUCT_NAME.equals("bazel")
        // runfiles.rlocation("com_google_protobuf/bazel/BUILD.bazel"), its directory.
        config.copy_directory(
            &test_deps().repository("protobuf").join("bazel"),
            "third_party/protobuf/bazel",
            usize::MAX,
            false,
        );
        config.overwrite(
            "third_party/protobuf/BUILD",
            &[
                "filegroup(name = 'license')",
                "genrule(name='protoc_gen', cmd='', executable = True, outs = ['protoc'])",
            ],
        );
        config.overwrite(
            "proto_bazel_features_workspace/MODULE.bazel",
            &["module(name = 'proto_bazel_features')"],
        );
        // Overwritten to remove bazel7 toolchains from protobuf
        config.overwrite("third_party/protobuf/bazel/private/toolchains/BUILD.bazel", &["load('//bazel/toolchains:proto_toolchain.bzl', 'proto_toolchain')", "load('@com_google_protobuf//bazel/toolchains:proto_lang_toolchain.bzl', 'proto_lang_toolchain')", "proto_toolchain(name = 'protoc_sources',proto_compiler = '@bazel_tools//tools/proto:protoc')"]);
        config.overwrite(
            "third_party/protobuf/bazel/private/toolchains/prebuilt/authenticity.bzl",
            &[
                "def _authenticity_impl(ctx):",
                "    return [OutputGroupInfo(_validation = depset())]",
                "authenticity_rule = rule(implementation = _authenticity_impl)",
            ],
        );
        config.overwrite("third_party/protobuf/bazel/private/toolchains/prebuilt/BUILD.bazel", &["load(':authenticity.bzl', 'authenticity_rule')", "authenticity_rule(name = 'authenticity_validation', visibility = ['//visibility:public'])"]);
        config.overwrite("proto_bazel_features_workspace/BUILD", &[]);
        config.overwrite(
            "proto_bazel_features_workspace/features.bzl",
            &[
                "bazel_features = struct(",
                "  globals = struct(PackageSpecificationInfo = PackageSpecificationInfo),",
                "  proto = struct(starlark_proto_info = True),",
                "  cc = struct(protobuf_on_allowlist = True),",
                ")",
            ],
        );
        config.overwrite(
            "third_party/protobuf/src/google/protobuf/compiler/BUILD.bazel",
            &[
                r#"genrule(name = "protoc", cmd = '', outs = ["protoc_minimal"], executable = True)
"#,
            ],
        );
    }
}

/// Java `MockPlatformSupport`: mocks for platforms and toolchains.
// Java: com/google/devtools/build/lib/packages/util/MockPlatformSupport.java:21 MockPlatformSupport
pub struct MockPlatformSupport;

impl MockPlatformSupport {
    /// Adds mocks for basic exec and target platform.
    // Java: com/google/devtools/build/lib/packages/util/MockPlatformSupport.java:24 MockPlatformSupport.setup
    pub fn setup(config: &MockToolsConfig) {
        Self::setup_with(
            config,
            PLATFORMS_PATH,
            CONSTRAINTS_PACKAGE_ROOT,
            CONSTRAINTS_PATH,
        );
    }

    /// Adds mocks for basic exec and target platform.
    // Java: com/google/devtools/build/lib/packages/util/MockPlatformSupport.java:33 MockPlatformSupport.setup
    pub fn setup_with(
        config: &MockToolsConfig,
        platforms_path: &str,
        constraints_package_root: &str,
        constraints_path: &str,
    ) {
        let r = constraints_package_root;
        config.create(
            &format!("{constraints_path}/BUILD"),
            &[
                "package(default_visibility=['//visibility:public'])",
                "licenses(['notice'])",
            ],
        );
        config.create(
            &format!("{constraints_path}/env/BUILD"),
            &[
                "package(default_visibility=['//visibility:public'])",
                "licenses(['notice'])",
                "constraint_setting(name = 'target_environment')",
                "constraint_value(",
                "    name = 'device',",
                "    constraint_setting = ':target_environment',",
                ")",
                "constraint_value(",
                "    name = 'simulator',",
                "    constraint_setting = ':target_environment',",
                ")",
            ],
        );
        let mut cpu: Vec<String> = vec![
            "package(default_visibility=['//visibility:public'])".into(),
            "licenses(['notice'])".into(),
            "constraint_setting(name = 'cpu')".into(),
        ];
        for value in [
            "all", "x86_32", "x86_64", "ppc", "arm", "arm64", "arm64e", "arm64_32", "armv7",
            "armv7k", "aarch64", "riscv32", "riscv64", "s390x",
        ] {
            cpu.push("constraint_value(".into());
            cpu.push(format!("    name = '{value}',"));
            cpu.push("    constraint_setting = ':cpu',".into());
            cpu.push(")".into());
        }
        let cpu: Vec<&str> = cpu.iter().map(String::as_str).collect();
        config.create(&format!("{constraints_path}/cpu/BUILD"), &cpu);
        let mut os: Vec<String> = vec![
            "package(default_visibility=['//visibility:public'])".into(),
            "licenses(['notice'])".into(),
            "constraint_setting(name = 'os')".into(),
        ];
        for value in [
            "osx", "macos", "ios", "tvos", "visionos", "watchos", "android", "linux", "windows",
            "freebsd", "openbsd",
        ] {
            os.push("constraint_value(".into());
            os.push(format!("    name = '{value}',"));
            os.push("    constraint_setting = ':os',".into());
            os.push(")".into());
        }
        let os: Vec<&str> = os.iter().map(String::as_str).collect();
        config.create(&format!("{constraints_path}/os/BUILD"), &os);
        config.create(
            &format!("{platforms_path}/BUILD"),
            &[
                "package(default_visibility=['//visibility:public'])",
                "platform(",
                "    name = 'default_target',",
                "    constraint_values = [",
                // Regardless of the actual machine the tests are run on, hardcode everything to a
                // single default value for simplicity.
                &format!("        '{r}cpu:x86_64',"),
                &format!("        '{r}os:linux',"),
                "    ],",
                ")",
                "platform(",
                "    name = 'default_host',",
                "    constraint_values = [",
                &format!("        '{r}cpu:x86_64',"),
                &format!("        '{r}os:linux',"),
                "    ],",
                ")",
            ],
        );
        config.create(
            &format!("{constraints_path}/host/BUILD"),
            &[
                "package(default_visibility=['//visibility:public'])",
                "licenses(['notice'])",
                "platform(",
                "    name = 'host',",
                "    constraint_values = [",
                // Regardless of the actual machine the tests are run on, hardcode everything to a
                // single default value for simplicity.
                &format!("        '{r}cpu:x86_64',"),
                &format!("        '{r}os:linux',"),
                "    ],",
                ")",
                "platform(",
                "     name = 'piii',",
                "     constraint_values = [",
                &format!("        '{r}cpu:x86_32',"),
                &format!("        '{r}os:linux',"),
                "    ],",
                ")",
            ],
        );
        config.create(
            &format!("{constraints_path}/host/constraints.bzl"),
            &[
                "HOST_CONSTRAINTS = [",
                // Regardless of the actual machine the tests are run on, hardcode everything to a
                // single default value for simplicity.
                &format!("        '{r}cpu:x86_64',"),
                &format!("        '{r}os:linux',"),
                "    ]",
            ],
        );
        config.create(
            &format!("{constraints_path}/host/extension.bzl"),
            &["def host_platform_repo(**kwargs):", "    pass"],
        );

        config.create(
            "third_party/bazel_platforms/android/BUILD",
            &[
                "licenses(['notice'])",
                "package(default_visibility=['//visibility:public'])",
                "platform(",
                "  name = 'armeabi-v7a',",
                &format!("  parents = ['{PLATFORM_LABEL}'],"),
                "  constraint_values = [",
                &format!("    '{CONSTRAINTS_PACKAGE_ROOT}os:android',"),
                &format!("    '{CONSTRAINTS_PACKAGE_ROOT}cpu:armv7',"),
                "  ],",
                ")",
            ],
        );
    }

    /// Adds a mock K8 platform.
    // Java: com/google/devtools/build/lib/packages/util/MockPlatformSupport.java:229 MockPlatformSupport.addMockK8Platform
    pub fn add_mock_k8_platform(config: &MockToolsConfig, crosstool_package: &str) {
        config.create(
            "mock_platform/BUILD",
            &[
                "package(default_visibility=['//visibility:public'])",
                "constraint_setting(name = 'mock_setting')",
                "constraint_value(name = 'mock_value', constraint_setting = ':mock_setting')",
                "platform(",
                "   name = 'mock-k8-platform',",
                "   constraint_values = [':mock_value'],",
                ")",
                "toolchain(",
                "   name = 'toolchain_cc-compiler-k8',",
                "   toolchain_type = '@bazel_tools//tools/cpp:toolchain_type',",
                &format!("   toolchain = '{crosstool_package}:cc-compiler-k8-compiler',"),
                "   target_compatible_with = [':mock_value'],",
                ")",
            ],
        );
    }
}

/// Java `MockObjcSupport` (the `setupXcodeRules` part).
pub struct MockObjcSupport;

impl MockObjcSupport {
    // Java: com/google/devtools/build/lib/packages/util/MockObjcSupport.java:129 MockObjcSupport.setupXcodeRules
    pub fn setup_xcode_rules(config: &MockToolsConfig) {
        config.create("build_bazel_apple_support/xcode/BUILD", &[]);
        config.create("build_bazel_apple_support/xcode/xcode_version.bzl", &[r#"XcodeVersionRuleInfo = provider(fields = ["aliases", "label", "xcode_version_properties"])

def _xcode_version_properties_info_init(
            *,
            xcode_version,
            default_ios_sdk_version = "8.4",
            default_macos_sdk_version = "10.11",
            default_tvos_sdk_version = "9.0",
            default_watchos_sdk_version = "2.0",
            default_visionos_sdk_version = "1.0"):
        return {
            "xcode_version": xcode_version,
            "default_ios_sdk_version": default_ios_sdk_version,
            "default_macos_sdk_version": default_macos_sdk_version,
            "default_tvos_sdk_version": default_tvos_sdk_version,
            "default_watchos_sdk_version": default_watchos_sdk_version,
            "default_visionos_sdk_version": default_visionos_sdk_version,
        }

XcodeVersionPropertiesInfo, _new_xcode_version_properties_info = provider(
        fields = [
            "xcode_version",
            "default_ios_sdk_version",
            "default_macos_sdk_version",
            "default_tvos_sdk_version",
            "default_watchos_sdk_version",
            "default_visionos_sdk_version",
        ],
        init = _xcode_version_properties_info_init,
)

def _xcode_version_impl(ctx):
        xcode_version_properties = XcodeVersionPropertiesInfo(
            xcode_version = ctx.attr.version,
            default_ios_sdk_version = ctx.attr.default_ios_sdk_version,
            default_visionos_sdk_version = ctx.attr.default_visionos_sdk_version,
            default_watchos_sdk_version = ctx.attr.default_watchos_sdk_version,
            default_tvos_sdk_version = ctx.attr.default_tvos_sdk_version,
            default_macos_sdk_version = ctx.attr.default_macos_sdk_version,
        )
        return [
            xcode_version_properties,
            XcodeVersionRuleInfo(
                label = ctx.label,
                xcode_version_properties = xcode_version_properties,
                aliases = ctx.attr.aliases,
            ),
            DefaultInfo(runfiles = ctx.runfiles()),
        ]

xcode_version = rule(
        attrs = {
            "version": attr.string(mandatory = True),
            "aliases": attr.string_list(allow_empty = True, mandatory = False),
            "default_ios_sdk_version": attr.string(default = "8.4", mandatory = False),
            "default_visionos_sdk_version": attr.string(default = "1.0", mandatory = False),
            "default_watchos_sdk_version": attr.string(default = "2.0", mandatory = False),
            "default_tvos_sdk_version": attr.string(default = "9.0",  mandatory = False),
            "default_macos_sdk_version": attr.string(default = "10.11",  mandatory = False),
        },
        implementation = _xcode_version_impl,
)
"#]);
        config.create(
            "build_bazel_apple_support/xcode/available_xcodes.bzl",
            &[r#"load(":xcode_version.bzl", "XcodeVersionRuleInfo")
AvailableXcodesInfo = provider(fields = ["available_versions", "default_version"])

def _available_xcodes_impl(ctx):
        available_versions = [
            target[XcodeVersionRuleInfo]
            for target in ctx.attr.versions
        ]
        default_version = ctx.attr.default[XcodeVersionRuleInfo]

        return [
            AvailableXcodesInfo(
                available_versions = available_versions,
                default_version = default_version,
            ),
        ]

available_xcodes = rule(
        attrs = {
            "default": attr.label(
                doc = "The default Xcode version for this platform.",
                mandatory = True,
                providers = [[XcodeVersionRuleInfo]],
                flags = ["NONCONFIGURABLE"],
            ),
            "versions": attr.label_list(
                doc = "The Xcode versions that are available on this platform.",
                providers = [[XcodeVersionRuleInfo]],
                flags = ["NONCONFIGURABLE"],
            ),
        },
        implementation = _available_xcodes_impl,
        provides = [AvailableXcodesInfo],
)
"#],
        );
        config.create("build_bazel_apple_support/xcode/xcode_config.bzl", &[r#"load(":available_xcodes.bzl", "AvailableXcodesInfo")
load(":xcode_version.bzl", "XcodeVersionRuleInfo", "XcodeVersionPropertiesInfo")

XcodeVersionInfo = apple_common.XcodeVersionConfig
unavailable_xcode_message = "'bazel fetch --force --configure' (Bzlmod) or 'bazel sync --configure' (WORKSPACE)"

def _xcode_config_impl(ctx):
        apple_fragment = ctx.fragments.apple
        cpp_fragment = ctx.fragments.cpp

        explicit_default_version = ctx.attr.default[XcodeVersionRuleInfo] if ctx.attr.default else None
        explicit_versions = [
            target[XcodeVersionRuleInfo]
            for target in ctx.attr.versions
        ] if ctx.attr.versions else []
        remote_versions = [
            target
            for target in ctx.attr.remote_versions[AvailableXcodesInfo].available_versions
        ] if ctx.attr.remote_versions else []
        local_versions = [
            target
            for target in ctx.attr.local_versions[AvailableXcodesInfo].available_versions
        ] if ctx.attr.local_versions else []

        local_default_version = ctx.attr.local_versions[AvailableXcodesInfo].default_version if ctx.attr.local_versions else None
        xcode_version_properties = None
        availability = "unknown"

        if _use_available_xcodes(
            explicit_default_version,
            explicit_versions,
            local_versions,
            remote_versions,
        ):
            xcode_version_properties, availability = _resolve_xcode_from_local_and_remote(
                local_versions,
                remote_versions,
                apple_fragment.xcode_version_flag,
                apple_fragment.prefer_mutual_xcode,
                local_default_version,
            )
        else:
            xcode_version_properties = _resolve_explicitly_defined_version(
                explicit_versions,
                explicit_default_version,
                apple_fragment.xcode_version_flag,
            )
            availability = "UNKNOWN"

        ios_sdk_version = apple_fragment.ios_sdk_version_flag or _dotted_version_or_default(xcode_version_properties.default_ios_sdk_version, "8.4")
        macos_sdk_version = apple_fragment.macos_sdk_version_flag or _dotted_version_or_default(xcode_version_properties.default_macos_sdk_version, "10.11")
        tvos_sdk_version = apple_fragment.tvos_sdk_version_flag or _dotted_version_or_default(xcode_version_properties.default_tvos_sdk_version, "9.0")
        watchos_sdk_version = apple_fragment.watchos_sdk_version_flag or _dotted_version_or_default(xcode_version_properties.default_watchos_sdk_version, "2.0")
        visionos_sdk_version = _dotted_version_or_default(xcode_version_properties.default_visionos_sdk_version, "1.0")

        ios_minimum_os = apple_fragment.ios_minimum_os_flag or ios_sdk_version
        macos_minimum_os = apple_fragment.macos_minimum_os_flag or macos_sdk_version
        tvos_minimum_os = apple_fragment.tvos_minimum_os_flag or tvos_sdk_version
        watchos_minimum_os = apple_fragment.watchos_minimum_os_flag or watchos_sdk_version
        if cpp_fragment.minimum_os_version():
            visionos_minimum_os = apple_common.dotted_version(cpp_fragment.minimum_os_version())
        else:
            visionos_minimum_os = visionos_sdk_version

        xcode_versions = XcodeVersionInfo(
            ios_sdk_version = str(ios_sdk_version),
            ios_minimum_os_version = str(ios_minimum_os),
            visionos_sdk_version = str(visionos_sdk_version),
            visionos_minimum_os_version = str(visionos_minimum_os),
            watchos_sdk_version = str(watchos_sdk_version),
            watchos_minimum_os_version = str(watchos_minimum_os),
            tvos_sdk_version = str(tvos_sdk_version),
            tvos_minimum_os_version = str(tvos_minimum_os),
            macos_sdk_version = str(macos_sdk_version),
            macos_minimum_os_version = str(macos_minimum_os),
            xcode_version = xcode_version_properties.xcode_version,
            availability = availability,
            xcode_version_flag = apple_fragment.xcode_version_flag,
            include_xcode_execution_info = apple_fragment.include_xcode_exec_requirements,
        )
        return [
            DefaultInfo(runfiles = ctx.runfiles()),
            xcode_versions,
            xcode_version_properties,
        ]

xcode_config = rule(
        attrs = {
            "default": attr.label(
                flags = ["NONCONFIGURABLE"],
                providers = [[XcodeVersionRuleInfo]],
            ),
            "versions": attr.label_list(
                flags = ["NONCONFIGURABLE"],
                providers = [[XcodeVersionRuleInfo]],
            ),
            "remote_versions": attr.label(
                flags = ["NONCONFIGURABLE"],
                providers = [[AvailableXcodesInfo]],
            ),
            "local_versions": attr.label(
                flags = ["NONCONFIGURABLE"],
                providers = [[AvailableXcodesInfo]],
            ),
        },
        fragments = ["apple", "cpp"],
        implementation = _xcode_config_impl,
)

def _use_available_xcodes(explicit_default_version, explicit_versions, local_versions, remote_versions):
        if remote_versions:
            if explicit_versions:
                fail("'versions' may not be set if '[local,remote]_versions' is set.")
            if explicit_default_version:
                fail("'default' may not be set if '[local,remote]_versions' is set.")
            if not local_versions:
                fail("if 'remote_versions' are set, you must also set 'local_versions'")
            return True
        return False

def _duplicate_alias_error(alias, versions):
        labels_containing_alias = []
        for version in versions:
            if alias in version.aliases or (version.xcode_version_properties.xcode_version == alias):
                labels_containing_alias.append(str(version.label))
        return "'{}' is registered to multiple labels ({}) in a single xcode_config rule".format(
            alias,
            ", ".join(labels_containing_alias),
        )

def _aliases_to_xcode_version(versions):
        version_map = {}
        if not versions:
            return version_map
        for version in versions:
            for alias in version.aliases:
                if alias in version_map:
                    fail(_duplicate_alias_error(alias, versions))
                else:
                    version_map[alias] = version
            version_string = version.xcode_version_properties.xcode_version
            if version_string not in version.aliases:  # only add the version if it's not also an alias
                if version_string in version_map:
                    fail(_duplicate_alias_error(version_string, versions))
                else:
                    version_map[version_string] = version
        return version_map

def _resolve_xcode_from_local_and_remote(
            local_versions,
            remote_versions,
            xcode_version_flag,
            prefer_mutual_xcode,
            local_default_version):
        local_alias_to_version_map = _aliases_to_xcode_version(local_versions)
        remote_alias_to_version_map = _aliases_to_xcode_version(remote_versions)

        mutually_available_versions = {}
        for version in local_versions:
            version_string = version.xcode_version_properties.xcode_version
            if version_string in remote_alias_to_version_map:
                mutually_available_versions[version_string] = remote_alias_to_version_map[version_string]

        # We'd log an event here if we could!!
        if xcode_version_flag:
            remote_version_from_flag = remote_alias_to_version_map.get(xcode_version_flag)
            local_version_from_flag = local_alias_to_version_map.get(xcode_version_flag)
            availability = "BOTH"

            if remote_version_from_flag and local_version_from_flag:
                local_version_from_remote_versions = remote_alias_to_version_map.get(local_version_from_flag.xcode_version_properties.xcode_version)
                if local_version_from_remote_versions:
                    return remote_version_from_flag.xcode_version_properties, availability
                else:
                    fail(
                        ("Xcode version {0} was selected, either because --xcode_version was passed, or" +
                        " because xcode-select points to this version locally. This corresponds to" +
                        " local Xcode version {1}. That build of version {0} is not available" +
                        " remotely, but there is a different build of version {2}, which has" +
                        " version {2} and aliases {3}. You probably meant to use this version." +
                        " Please download it *and delete version {1}, then run `blaze shutdown`" +
                        " to continue using dynamic execution. If you really did intend to use" +
                        " local version {1}, please specify it fully with --xcode_version={1}.").format(
                            xcode_version_flag,
                            local_version_from_flag.xcode_version_properties.xcode_version,
                            remote_version_from_flag.xcode_version_properties.xcode_version,
                            remote_version_from_flag.aliases,
                        ),
                    )

            elif local_version_from_flag:
                error = (
                    " --xcode_version={} specified, but it is not available remotely. Actions " +
                    "requiring Xcode will be run locally, which could make your build slower."
                ).format(
                    xcode_version_flag,
                )
                if (mutually_available_versions):
                    error += " Consider using one of [{}].".format(
                        ", ".join([version for version in mutually_available_versions]),
                    )
                print(error)
                return local_version_from_flag.xcode_version_properties, "LOCAL"

            elif remote_version_from_flag:
                print(("--xcode_version={version} specified, but it is not available locally. " +
                      "Your build will fail if any actions require a local Xcode. " +
                      "If you believe you have '{version}' installed, try running {command}," +
                      "and then re-run your command. Locally available versions: {local_versions}. ")
                    .format(
                    version = xcode_version_flag,
                    command = unavailable_xcode_message,
                    local_versions = ", ".join([version for version in local_alias_to_version_map.keys()]),
                ))
                availability = "REMOTE"

                return remote_version_from_flag.xcode_version_properties, availability

            else:  # fail if we can't find any version to match
                fail(
                    ("--xcode_version={0} specified, but '{0}' is not an available Xcode version." +
                    " Locally available versions: [{2}]. Remotely available versions: [{3}]. If" +
                    " you believe you have '{0}' installed, try running {1}, and then" +
                    " re-run your command.").format(
                        xcode_version_flag,
                        unavailable_xcode_message,
                        ", ".join([version.xcode_version_properties.xcode_version for version in local_versions]),
                        ", ".join([version.xcode_version_properties.xcode_version for version in remote_versions]),
                    ),
                )

        # --xcode_version is not set
        availability = "UNKNOWN"
        local_version = None

        # If there aren't any mutually available versions, select the local default.
        if not mutually_available_versions:
            print(
                ("Using a local Xcode version, '{}', since there are no" +
                " remotely available Xcodes on this machine. Consider downloading one of the" +
                " remotely available Xcode versions ({}) in order to get the best build" +
                " performance.").format(local_default_version.xcode_version_properties.xcode_version, ", ".join([version.xcode_version_properties.xcode_version for version in remote_versions])),
            )
            local_version = local_default_version
            availability = "LOCAL"
        elif (local_default_version.xcode_version_properties.xcode_version in remote_alias_to_version_map):
            #  If the local default version is also available remotely, use it.
            availability = "BOTH"
            local_version = remote_alias_to_version_map.get(local_default_version.xcode_version_properties.xcode_version)
        else:
            # If an alias of the local default version is available remotely, use it.
            for version_number in local_default_version.aliases:
                if version_number in remote_alias_to_version_map:
                    availability = "BOTH"
                    local_version = remote_alias_to_version_map.get(version_number)
                    break

        if local_version:
            return local_version.xcode_version_properties, availability

        # The local default is not available remotely.
        if prefer_mutual_xcode:
            # If we prefer a mutually available version, the newest one.
            newest_version = "0.0"
            default_version = None
            for _, version in mutually_available_versions.items():
                if _compare_version_strings(version.xcode_version_properties.xcode_version, newest_version) > 0:
                    default_version = version
                    newest_version = default_version.xcode_version_properties.xcode_version

            return default_version.xcode_version_properties, "BOTH"
        else:
            # Use the local default
            return local_default_version.xcode_version_properties, "LOCAL"

def _compare_version_strings(first, second):
        return apple_common.dotted_version(first).compare_to(apple_common.dotted_version(second))

def _resolve_explicitly_defined_version(
            explicit_versions,
            explicit_default_version,
            xcode_version_flag):
        if explicit_default_version and explicit_default_version.label not in [
            version.label
            for version in explicit_versions
        ]:
            fail(
                "default label '{}' must be contained in versions attribute".format(explicit_default_version.label),
            )
        if not explicit_versions:
            if explicit_default_version:
                fail("default label must be contained in versions attribute")
            return XcodeVersionPropertiesInfo(xcode_version = None)

        if not explicit_default_version:
            fail("if any versions are specified, a default version must be specified")

        alias_to_versions = _aliases_to_xcode_version(explicit_versions)
        if xcode_version_flag:
            flag_version = alias_to_versions.get(str(xcode_version_flag))
            if flag_version:
                return flag_version.xcode_version_properties
            elif xcode_version_flag.startswith("/"):
                return XcodeVersionPropertiesInfo(xcode_version = xcode_version_flag)
            else:
                fail(
                    ("--xcode_version={0} specified, but '{0}' is not an available Xcode version. " +
                    "If you believe you have '{0}' installed, try running 'bazel shutdown', and then " +
                    "re-run your command.").format(xcode_version_flag),
                )

        return alias_to_versions.get(explicit_default_version.xcode_version_properties.xcode_version).xcode_version_properties

def _dotted_version_or_default(field, default):
        return apple_common.dotted_version(field) or default
"#]);
        config.create(
            "build_bazel_apple_support/xcode/xcode_config_alias.bzl",
            &[r#"load(":xcode_version.bzl", "XcodeVersionPropertiesInfo")

def _xcode_config_alias_impl(ctx):
        xcode_config = ctx.attr._xcode_config
        return [
            xcode_config[XcodeVersionPropertiesInfo],
            xcode_config[apple_common.XcodeVersionConfig],
        ]

xcode_config_alias = rule(
        attrs = {
            "_xcode_config": attr.label(
                default = configuration_field(
                    fragment = "apple",
                    name = "xcode_config_label",
                ),
            ),
        },
        fragments = ["apple"],
        implementation = _xcode_config_alias_impl,
)
"#],
        );
    }
}
