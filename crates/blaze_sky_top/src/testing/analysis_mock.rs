//! Port of `analysis/util/AnalysisMock.java` and `analysis/mock/BazelAnalysisMock.java`
//! (src/test/java), with `packages/util/MockPlatformSupport.java` and
//! `packages/util/MockGenruleSupport.java`: the mock client the analysis tests run in.
//!
//! The files are Java's, character for character (text blocks dedented as `javac` does). What is
//! not ported, and why (every other file of the Java mock is written here):
//!
//! - The mock C++, Java, Python, Objective-C and protobuf toolchains
//!   (`MockCcSupport`/`BazelMockCcSupport`, `AbstractMockJavaSupport.setupRulesJava`,
//!   `BazelMockPythonSupport`, `MockObjcSupport.setupXcodeRules`, the protobuf half of
//!   `MockProtoSupport.setupWorkspace`) and the `@bazel_skylib` copy. Java copies the real
//!   `rules_cc`, `rules_java`, `rules_python`, `bazel_skylib` and `protobuf` sources from the
//!   test's runfiles into the workspace; a hermetic Rust test has no such runfiles. Only the
//!   `MODULE.bazel` files those supports write are created (every built-in module needs one for
//!   module resolution), so `@rules_cc`, `@rules_java`, `@rules_python`, ... resolve but are
//!   empty. Tests that use C++, Java, Python, Objective-C or proto rules stay ignored with the
//!   reason "needs the mock C++/Java/Python toolchains".
//! - For the same reason the root `MODULE.bazel` does not register the toolchains of those mocks
//!   (`@rules_java//java/toolchains/runtime:all`, `@rules_java//java/toolchains/javac:all`,
//!   `@bazel_tools//tools/cpp:all`, `@bazel_tools//tools/jdk:all`,
//!   `@bazel_tools//tools/python:autodetecting_toolchain`, and `MockProtoSupport`'s
//!   `//tools/proto/toolchains:all`): their packages do not exist here. The `@bazel_tools`
//!   module's own registration (`//tools/test:all`) is kept.
//! - `embedded_tools/tools/jdk/{BUILD,launcher_flag_alias.bzl,java_toolchain_alias.bzl}` load
//!   `@rules_java` / `@rules_cc` symbols and are part of the Java mock: left out.
//! - `ShellConfiguration.injectShellExecutableFinder` is a no-op here (Blaze's shell
//!   configuration is computed from the options).

use blaze_bzlmod_model::module::NonRegistryOverride;
use blaze_bzlmod_model::repo_spec::{AttributeValue, AttributeValues, LOCAL_REPOSITORY, RepoSpec};
use blaze_vfs::{Path, PathFragment};
use indexmap::IndexMap;

use crate::testing::mock_tools_config::MockToolsConfig;

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
        // PORT: MockObjcSupport.setupXcodeRules(config) is not ported (module documentation).
        config.create(
            "local_config_xcode_workspace/MODULE.bazel",
            &["module(name = 'local_config_xcode')"],
        );
        config.create("third_party/protobuf/BUILD", &[]);
        config.create(
            "third_party/protobuf/MODULE.bazel",
            &["module(name='com_google_protobuf')"],
        );
        // PORT: Java registers the toolchains of the C++, Java and Python mocks here
        // (register_toolchains of '@rules_java//java/toolchains/runtime:all',
        // '@rules_java//java/toolchains/javac:all', '@bazel_tools//tools/cpp:all',
        // '@bazel_tools//tools/jdk:all', '@bazel_tools//tools/python:autodetecting_toolchain');
        // those mocks are not ported (module documentation), so the file is empty.
        config.overwrite("MODULE.bazel", &[]);
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

        // PORT: tools/jdk/java_toolchain_alias.bzl (copied from runfiles),
        // embedded_tools/tools/jdk/launcher_flag_alias.bzl and embedded_tools/tools/jdk/BUILD
        // are part of the Java mock (module documentation).

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
        // PORT: the copy is not ported (module documentation); the module file is.
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
        // PORT: ccSupport().setup(config), javaSupport().setupRulesJava(config, ...) and
        // pySupport().setup(config): only the module files they write (module documentation).
        config.create(
            "third_party/bazel_rules/rules_cc/MODULE.bazel",
            &["module(name='rules_cc')"],
        );
        config.create(
            "rules_java_workspace/MODULE.bazel",
            &["module(name = 'rules_java')"],
        );
        config.overwrite(
            "rules_python_workspace/MODULE.bazel",
            &["module(name = 'rules_python')"],
        );
        config.create(
            "rules_python_internal_workspace/MODULE.bazel",
            &["module(name = 'rules_python_internal')"],
        );
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

/// The part of Java `MockProtoSupport.setupWorkspace` that does not copy protobuf sources from
/// the runfiles (module documentation).
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
        // PORT: the copy of com_google_protobuf/bazel and the protobuf BUILD overwrites are not
        // ported (module documentation).
        config.overwrite(
            "proto_bazel_features_workspace/MODULE.bazel",
            &["module(name = 'proto_bazel_features')"],
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
