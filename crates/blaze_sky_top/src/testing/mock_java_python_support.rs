//! Port of `analysis/util/AbstractMockJavaSupport.java` (`BAZEL`) and
//! `packages/util/BazelMockPythonSupport.java` / `rules/python/PythonTestUtils.getPyLoad`
//! (src/test/java): the mock Java and Python rules of the analysis tests.
//!
//! The real `rules_java` and `rules_python` sources and the `tools/python` files of the pinned
//! Bazel source come from the prepared test dependencies (`test_deps.rs`) where Java reads its
//! runfiles and test resources.

use crate::testing::mock_tools_config::MockToolsConfig;
use crate::testing::test_deps::test_deps;

/// `TestConstants.RULES_PYTHON_PACKAGE_ROOT`.
pub const RULES_PYTHON_PACKAGE_ROOT: &str = "@@rules_python+/";

/// `PythonTestUtils.getPyLoad(symbolName)`.
// Java: com/google/devtools/build/lib/rules/python/PythonTestUtils.java:53 PythonTestUtils.getPyLoad
pub fn get_py_load(symbol_name: &str) -> String {
    if RULES_PYTHON_PACKAGE_ROOT.is_empty() {
        return String::new();
    }
    let bzl_filename = match symbol_name {
        "PyInfo" => "py_info.bzl".to_string(),
        "PyRuntimeInfo" => "py_runtime_info.bzl".to_string(),
        _ => format!("{symbol_name}.bzl"),
    };
    format!("load('{RULES_PYTHON_PACKAGE_ROOT}/python:{bzl_filename}', '{symbol_name}')")
}

/// Java `AbstractMockJavaSupport.BAZEL`.
// Java: com/google/devtools/build/lib/analysis/util/AbstractMockJavaSupport.java:24 AbstractMockJavaSupport.BAZEL
pub struct BazelMockJavaSupport;

impl BazelMockJavaSupport {
    // Java: com/google/devtools/build/lib/analysis/util/AbstractMockJavaSupport.java:27 AbstractMockJavaSupport.BAZEL.setupRulesJava
    pub fn setup_rules_java(&self, config: &MockToolsConfig) {
        config.create(
            "rules_java_workspace/MODULE.bazel",
            &["module(name = 'rules_java')"],
        );
        // runfilesResolver.apply("rules_java/java/defs.bzl"), two directories up.
        let rules_java_root = test_deps().repository("rules_java");
        config.copy_directory(
            &rules_java_root.join("java"),
            "rules_java_workspace/java",
            usize::MAX,
            true,
        );
        config.copy_tool(
            &rules_java_root.join("toolchains/java_toolchain_alias.bzl"),
            "rules_java_workspace/toolchains/java_toolchain_alias.bzl",
        );
        // Overwrite redirects to not have to use bazel_features / compatibility layer
        config.overwrite("rules_java_workspace/java/java_binary.bzl", &[r#"load("@rules_java//java/bazel/rules:bazel_java_binary_wrapper.bzl", _java_binary = "java_binary")
java_binary = _java_binary
"#]);
        config.overwrite("rules_java_workspace/java/java_import.bzl", &[r#"load("@rules_java//java/bazel/rules:bazel_java_import.bzl", _java_import = "java_import")
java_import = _java_import
"#]);
        config.overwrite("rules_java_workspace/java/java_library.bzl", &[r#"load("@rules_java//java/bazel/rules:bazel_java_library.bzl", _java_library = "java_library")
java_library = _java_library
"#]);
        config.overwrite("rules_java_workspace/java/java_plugin.bzl", &[r#"load("@rules_java//java/bazel/rules:bazel_java_plugin.bzl", _java_plugin = "java_plugin")
java_plugin = _java_plugin
"#]);
        config.overwrite("rules_java_workspace/java/java_test.bzl", &[r#"load("@rules_java//java/bazel/rules:bazel_java_test.bzl", _java_test = "java_test")
java_test = _java_test
"#]);
        config.overwrite(
            "rules_java_workspace/java/toolchains/java_package_configuration.bzl",
            &[
                r#"load("@rules_java//java/common/rules:java_package_configuration.bzl",
  _java_package_configuration = "java_package_configuration")
java_package_configuration = _java_package_configuration
"#,
            ],
        );
        config.overwrite(
            "rules_java_workspace/java/toolchains/java_runtime.bzl",
            &[r#"load("@rules_java//java/common/rules:java_runtime.bzl",
  _java_runtime = "java_runtime")
java_runtime = _java_runtime
"#],
        );
        config.overwrite(
            "rules_java_workspace/java/toolchains/java_toolchain.bzl",
            &[r#"load("@rules_java//java/common/rules:java_toolchain.bzl",
  _java_toolchain = "java_toolchain")
java_toolchain = _java_toolchain
"#],
        );
        config.overwrite(
            "rules_java_workspace/java/common/java_common.bzl",
            &[
                r#"load("@rules_java//java/private:java_common.bzl", _java_common = "java_common")
java_common = _java_common
"#,
            ],
        );
        config.overwrite(
            "rules_java_workspace/java/common/java_info.bzl",
            &[
                r#"load("@rules_java//java/private:java_info.bzl", _JavaInfo = "JavaInfo")
JavaInfo = _JavaInfo
"#,
            ],
        );
        config.overwrite("rules_java_workspace/java/common/java_plugin_info.bzl", &[r#"load("@rules_java//java/private:java_info.bzl", _JavaPluginInfo = "JavaPluginInfo")
JavaPluginInfo = _JavaPluginInfo
"#]);
        config.overwrite(
            "rules_java_workspace/java/private/proto_support.bzl",
            &[
                r#"load("@rules_java//java/private:java_common.bzl", "java_common")
def compile(*, injecting_rule_kind, enable_jspecify, include_compilation_info, **kwargs):
        return java_common.compile(**kwargs)
def merge(providers, *, merge_java_outputs = True, merge_source_jars = True):
        return java_common.merge(providers)
"#,
            ],
        );
        // mocks
        config.create("rules_java_workspace/toolchains/BUILD", &[]);
        config.create(
            "rules_java_workspace/toolchains/local_java_repository.bzl",
            &["def local_java_repository(**attrs):
        pass
"],
        );
        config.create(
            "rules_java_workspace/toolchains/jdk_build_file.bzl",
            &["JDK_BUILD_TEMPLATE = ''"],
        );
        config.overwrite(
            "rules_java_workspace/java/repositories.bzl",
            &[r#"def rules_java_dependencies():
        pass

def rules_java_toolchains():
        native.register_toolchains("//java/toolchains/runtime:all")
        native.register_toolchains("//java/toolchains/javac:all")
"#],
        );
        config.overwrite(
            "rules_java_workspace/java/toolchains/runtime/BUILD",
            &[r#"toolchain_type(name = "toolchain_type")

toolchain(
        name = "local_jdk",
        toolchain = "@bazel_tools//tools/jdk:jdk",
        toolchain_type = "@rules_java//java/toolchains/runtime:toolchain_type",
)
"#],
        );
        config.overwrite(
            "rules_java_workspace/java/toolchains/javac/BUILD",
            &[r#"toolchain_type(name = "toolchain_type")

toolchain(
        name = "javac_toolchain",
        toolchain = "@bazel_tools//tools/jdk:toolchain",
        toolchain_type = "@rules_java//java/toolchains/javac:toolchain_type",
)
"#],
        );
    }

    // Java: com/google/devtools/build/lib/analysis/util/AbstractMockJavaSupport.java:167 AbstractMockJavaSupport.BAZEL.getLoadStatementForRule
    pub fn get_load_statement_for_rule(&self, rule_name: &str) -> String {
        format!("load('@rules_java//java:{rule_name}.bzl', '{rule_name}')")
    }
}

/// Java `BazelMockPythonSupport`.
// Java: com/google/devtools/build/lib/packages/util/BazelMockPythonSupport.java:27 BazelMockPythonSupport
pub struct BazelMockPythonSupport;

impl BazelMockPythonSupport {
    // Java: com/google/devtools/build/lib/packages/util/BazelMockPythonSupport.java:33 BazelMockPythonSupport.addTool
    fn add_tool(config: &MockToolsConfig, tool_relative_path: &str) {
        let deps = test_deps();
        // TestConstants.BAZEL_REPO_PATH + toolRelativePath, read from the test's resources.
        let content = deps.read_resource(&deps.resource(tool_relative_path));
        config.create(&format!("embedded_tools/{tool_relative_path}"), &[&content]);
    }

    // Java: com/google/devtools/build/lib/packages/util/BazelMockPythonSupport.java:40 BazelMockPythonSupport.setup
    pub fn setup(&self, config: &MockToolsConfig) {
        Self::add_tool(config, "tools/python/python_version.bzl");
        Self::add_tool(config, "tools/python/srcs_version.bzl");
        Self::add_tool(config, "tools/python/toolchain.bzl");
        Self::add_tool(config, "tools/python/utils.bzl");
        Self::add_tool(config, "tools/python/python_bootstrap_template.txt");

        config.create(
            "embedded_tools/tools/python/BUILD",
            &[
                "package(default_visibility=['//visibility:public'])",
                &get_py_load("py_runtime"),
                &get_py_load("py_runtime_pair"),
                "load(':python_version.bzl', 'define_python_version_flag')",
                "define_python_version_flag(",
                "    name = 'python_version',",
                ")",
                "config_setting(",
                "    name = 'PY2',",
                "    flag_values = {':python_version': 'PY2'},",
                ")",
                "config_setting(",
                "    name = 'PY3',",
                "    flag_values = {':python_version': 'PY3'},",
                ")",
                "toolchain_type(name = 'toolchain_type')",
                "constraint_setting(name = 'py2_interpreter_path')",
                "constraint_setting(name = 'py3_interpreter_path')",
                "py_runtime(",
                "    name = 'py3_interpreter',",
                "    interpreter_path = '/usr/bin/mockpython3',",
                "    python_version = 'PY3',",
                ")",
                "py_runtime_pair(",
                "    name = 'default_py_runtime_pair',",
                "    py3_runtime = ':py3_interpreter',",
                ")",
                "toolchain(",
                "    # The Python workspace suffix looks to register a toolchain of this name.",
                "    name = 'autodetecting_toolchain',",
                "    toolchain = ':default_py_runtime_pair',",
                "    toolchain_type = ':toolchain_type',",
                ")",
                "exports_files(['precompile.py', 'python_bootstrap_template.txt'])",
            ],
        );

        // Copies and mock rules_python from real @rules_python
        // runfiles.rlocation("rules_python/python/defs.bzl"), two directories up.
        config.copy_directory(
            &test_deps().repository("rules_python"),
            "rules_python_workspace",
            usize::MAX,
            true,
        );

        config.overwrite("rules_python_workspace/python/BUILD", &["alias(name = 'toolchain_type', actual = '@bazel_tools//tools/python:toolchain_type')", "toolchain_type(name = 'exec_tools_toolchain_type')"]);
        config.overwrite(
            "rules_python_workspace/MODULE.bazel",
            &["module(name = 'rules_python')"],
        );
        config.overwrite("rules_python_workspace/python/private/BUILD", &["load('@bazel_skylib//rules:common_settings.bzl', 'bool_setting')", "filegroup(name = 'stage2_bootstrap_template', srcs = ['stage2_bootstrap_template.py'])", "filegroup(name = 'zip_main_template', srcs = ['zip_main_template.py'])", "filegroup(name = 'bootstrap_template', srcs = ['python_bootstrap_template.txt'])", "filegroup(name = 'site_init_template', srcs = ['site_init_template.py'])", "bool_setting(name = 'visible_for_testing', build_setting_default = False)"]);
        config.overwrite("rules_python_workspace/python/private/common/BUILD", &[]);
        config.overwrite("rules_python_workspace/python/config_settings/BUILD.bazel", &["load('@bazel_skylib//rules:common_settings.bzl', 'string_flag', 'bool_flag')", "load('@rules_python//python/private:flags.bzl', rp_string_flag = 'string_flag')", "string_flag(name = 'python_version', build_setting_default = '3.11')", "string_flag(name = 'precompile', build_setting_default = 'auto')", "string_flag(name = 'py_freethreaded', build_setting_default = 'no')", "string_flag(name = 'add_srcs_to_runfiles', build_setting_default = 'auto')", "string_flag(name = 'pyc_collection', build_setting_default = 'disabled')", "string_flag(name = 'precompile_source_retention', build_setting_default = 'auto')", "rp_string_flag(name = 'bootstrap_impl', build_setting_default = 'system_python', ", "    values = ['system_python'])", "string_flag(name = 'venvs_use_declare_symlink', build_setting_default = 'yes')", "string_flag(name = 'precompile_add_to_runfiles', build_setting_default = 'always')", "string_flag(name = 'exec_tools_toolchain', build_setting_default = 'yes')", "label_flag(name = 'pip_env_marker_config', build_setting_default = ':_pip_env_marker_default_config')", "string_flag(name = 'pip_whl', build_setting_default = 'auto')", "string_flag(name = 'pip_whl_glibc_version', build_setting_default = '')", "string_flag(name = 'pip_whl_muslc_version', build_setting_default = '')", "string_flag(name = 'pip_whl_osx_arch', build_setting_default = '')", "string_flag(name = 'pip_whl_osx_version', build_setting_default = '')", "string_flag(name = 'py_linux_libc', build_setting_default = '')", "string_flag(name = 'venvs_site_packages', build_setting_default = 'no')", "bool_flag(name = 'experimental_python_import_all_repositories', build_setting_default = True)", "bool_flag(name = 'build_python_zip', build_setting_default = False)", "bool_flag(name = 'incompatible_default_to_explicit_init_py', build_setting_default = True)", "string_flag(name = 'python_path', build_setting_default = 'python')"]);
        config.overwrite(
            "rules_python_workspace/python/private/python_bootstrap_template.txt",
            &[],
        );
        config.overwrite(
            "rules_python_workspace/tools/build_defs/python/private/BUILD",
            &[],
        );
        config.overwrite(
            "rules_python_workspace/tools/launcher/BUILD",
            &["filegroup(name = 'launcher')"],
        );
        config.overwrite(
            "rules_python_workspace/python/bin/BUILD",
            &["label_flag(name = 'python_src', build_setting_default = '//python:none')"],
        );
        config.create(
            "rules_python_internal_workspace/MODULE.bazel",
            &["module(name = 'rules_python_internal')"],
        );
        config.create("rules_python_internal_workspace/BUILD", &[]);
        config.create("rules_python_internal_workspace/rules_python_config.bzl", &["config = struct(enable_pystar = True, BuiltinPyInfo = None, BuiltinPyRuntimeInfo = None)"]);
        config.create("rules_python_internal_workspace/py_internal.bzl", &["load('@rules_python//tools/build_defs/python/private:py_internal_renamed.bzl', 'py_internal_renamed')", "py_internal_impl = py_internal_renamed"]);
        config.create(
            "rules_python_internal_workspace/extra_transition_settings.bzl",
            &["EXTRA_TRANSITION_SETTINGS = []"],
        );
    }
}
