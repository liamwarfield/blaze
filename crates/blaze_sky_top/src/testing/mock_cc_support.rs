//! Port of `packages/util/MockCcSupport.java`, `packages/util/BazelMockCcSupport.java` and
//! `packages/util/Crosstool.java` (src/test/java): the mock C++ rules and toolchain of the
//! analysis tests.
//!
//! The real `rules_cc` sources come from the prepared test dependencies (`test_deps.rs`) where
//! Java reads its runfiles and test resources. Java's tests run on an in-memory file system, so
//! the real-file-system crosstool branches (`config.isRealFileSystem()`) are not taken.

use crate::testing::mock_tools_config::MockToolsConfig;
use crate::testing::test_deps::test_deps;

/// `TestConstants.TOOLS_REPOSITORY_SCRATCH`.
const TOOLS_REPOSITORY_SCRATCH: &str = "embedded_tools/";
/// `TestConstants.MOCK_LICENSE_SCRATCH`.
const MOCK_LICENSE_SCRATCH: &str = "third_party/rules_license/";
/// `TestConstants.CONSTRAINTS_PACKAGE_ROOT`.
const CONSTRAINTS_PACKAGE_ROOT: &str = "@platforms//";
/// `TestConstants.TOOLS_REPOSITORY` (`RepositoryName.BAZEL_TOOLS`, printed as `@bazel_tools`).
const TOOLS_REPOSITORY: &str = "@bazel_tools";
/// `CppRuleClasses.STATIC_LINK_CPP_RUNTIMES`.
const STATIC_LINK_CPP_RUNTIMES: &str = "static_link_cpp_runtimes";

/// `Crosstool.CROSSTOOL_BINARIES`.
const CROSSTOOL_BINARIES: &[&str] = &[
    "ar",
    "as",
    "compile",
    "dwp",
    "link",
    "objcopy",
    "llvm-profdata",
];

/// `BazelMockCcSupport.CROSSTOOL_ARCHS`.
const CROSSTOOL_ARCHS: &[&str] = &["piii", "k8", "armeabi-v7a", "ppc", "darwin_x86_64"];

/// Java `Crosstool.CcToolchainConfig`: the fields needed to generate the crosstool BUILD file.
// Java: com/google/devtools/build/lib/packages/util/Crosstool.java:45 Crosstool.CcToolchainConfig
#[derive(Clone, Debug)]
pub struct CcToolchainConfig {
    cpu: String,
    compiler: String,
    toolchain_identifier: String,
    host_system_name: String,
    target_system_name: String,
    abi_version: String,
    abi_libc_version: String,
    target_libc: String,
    builtin_sysroot: String,
    cc_target_os: String,
    features: Vec<String>,
    action_configs: Vec<String>,
    artifact_name_patterns: Vec<Vec<String>>,
    tool_paths: Vec<(String, String)>,
    cxx_builtin_include_directories: Vec<String>,
    make_variables: Vec<(String, String)>,
    toolchain_exec_constraints: Vec<String>,
    toolchain_target_constraints: Vec<String>,
}

/// Java `Crosstool.CcToolchainConfig.Builder`.
// Java: com/google/devtools/build/lib/packages/util/Crosstool.java:109 Crosstool.CcToolchainConfig.Builder
#[derive(Clone, Debug)]
pub struct CcToolchainConfigBuilder {
    config: CcToolchainConfig,
}

impl Default for CcToolchainConfigBuilder {
    fn default() -> Self {
        let default_constraints = vec![
            format!("{CONSTRAINTS_PACKAGE_ROOT}cpu:x86_64"),
            format!("{CONSTRAINTS_PACKAGE_ROOT}os:linux"),
        ];
        CcToolchainConfigBuilder {
            config: CcToolchainConfig {
                features: Vec::new(),
                action_configs: Vec::new(),
                artifact_name_patterns: Vec::new(),
                tool_paths: Vec::new(),
                builtin_sysroot: "/usr/grte/v1".to_string(),
                cxx_builtin_include_directories: Vec::new(),
                make_variables: Vec::new(),
                cc_target_os: String::new(),
                cpu: "k8".to_string(),
                compiler: "compiler".to_string(),
                toolchain_identifier: "mock-toolchain-k8".to_string(),
                host_system_name: "local".to_string(),
                target_system_name: "local".to_string(),
                target_libc: "local".to_string(),
                abi_version: "local".to_string(),
                abi_libc_version: "local".to_string(),
                toolchain_exec_constraints: default_constraints.clone(),
                toolchain_target_constraints: default_constraints,
            },
        }
    }
}

fn strings(values: &[&str]) -> Vec<String> {
    values.iter().map(|s| s.to_string()).collect()
}

impl CcToolchainConfigBuilder {
    pub fn with_cpu(mut self, cpu: &str) -> Self {
        self.config.cpu = cpu.to_string();
        self
    }
    pub fn with_compiler(mut self, compiler: &str) -> Self {
        self.config.compiler = compiler.to_string();
        self
    }
    pub fn with_toolchain_identifier(mut self, toolchain_identifier: &str) -> Self {
        self.config.toolchain_identifier = toolchain_identifier.to_string();
        self
    }
    pub fn with_host_system_name(mut self, host_system_name: &str) -> Self {
        self.config.host_system_name = host_system_name.to_string();
        self
    }
    pub fn with_target_system_name(mut self, target_system_name: &str) -> Self {
        self.config.target_system_name = target_system_name.to_string();
        self
    }
    pub fn with_target_libc(mut self, target_libc: &str) -> Self {
        self.config.target_libc = target_libc.to_string();
        self
    }
    pub fn with_abi_version(mut self, abi_version: &str) -> Self {
        self.config.abi_version = abi_version.to_string();
        self
    }
    pub fn with_abi_libc_version(mut self, abi_libc_version: &str) -> Self {
        self.config.abi_libc_version = abi_libc_version.to_string();
        self
    }
    pub fn with_features(mut self, features: &[&str]) -> Self {
        self.config.features = strings(features);
        self
    }
    pub fn with_action_configs(mut self, action_configs: &[&str]) -> Self {
        self.config.action_configs = strings(action_configs);
        self
    }
    // Java: com/google/devtools/build/lib/packages/util/Crosstool.java:196 Crosstool.CcToolchainConfig.Builder.withArtifactNamePatterns
    pub fn with_artifact_name_patterns(mut self, artifact_name_patterns: &[&[&str]]) -> Self {
        for pattern in artifact_name_patterns {
            blaze_util::check_argument!(
                pattern.len() == 3,
                "Artifact name pattern should have three attributes: category_name, prefix and \
                 extension"
            );
        }
        self.config.artifact_name_patterns =
            artifact_name_patterns.iter().map(|p| strings(p)).collect();
        self
    }
    pub fn with_tool_paths(mut self, tool_paths: &[(&str, &str)]) -> Self {
        self.config.tool_paths = tool_paths
            .iter()
            .map(|(a, b)| (a.to_string(), b.to_string()))
            .collect();
        self
    }
    pub fn with_sysroot(mut self, sysroot: &str) -> Self {
        self.config.builtin_sysroot = sysroot.to_string();
        self
    }
    pub fn with_cc_target_os(mut self, cc_target_os: &str) -> Self {
        self.config.cc_target_os = cc_target_os.to_string();
        self
    }
    pub fn with_cxx_builtin_include_directories(mut self, directories: &[&str]) -> Self {
        self.config.cxx_builtin_include_directories = strings(directories);
        self
    }
    pub fn with_make_variables(mut self, make_variables: &[(&str, &str)]) -> Self {
        self.config.make_variables = make_variables
            .iter()
            .map(|(a, b)| (a.to_string(), b.to_string()))
            .collect();
        self
    }
    pub fn with_toolchain_exec_constraints(mut self, exec_constraints: &[&str]) -> Self {
        self.config.toolchain_exec_constraints = strings(exec_constraints);
        self
    }
    pub fn with_toolchain_target_constraints(mut self, target_constraints: &[&str]) -> Self {
        self.config.toolchain_target_constraints = strings(target_constraints);
        self
    }
    pub fn build(self) -> CcToolchainConfig {
        self.config
    }
}

impl CcToolchainConfig {
    // Java: com/google/devtools/build/lib/packages/util/Crosstool.java:104 Crosstool.CcToolchainConfig.builder
    pub fn builder() -> CcToolchainConfigBuilder {
        CcToolchainConfigBuilder::default()
    }

    pub fn get_toolchain_identifier(&self) -> &str {
        &self.toolchain_identifier
    }

    pub fn get_target_cpu(&self) -> &str {
        &self.cpu
    }

    pub fn get_compiler(&self) -> &str {
        &self.compiler
    }

    // Java: com/google/devtools/build/lib/packages/util/Crosstool.java:284 Crosstool.CcToolchainConfig.formatConstraints
    fn format_constraints(type_: &str, constraints: &[String]) -> String {
        if constraints.is_empty() {
            return String::new();
        }
        let output = constraints
            .iter()
            .map(|constraint| format!("'{constraint}',"))
            .collect::<Vec<_>>()
            .join("\n");
        format!("{type_}_compatible_with = [\n{output}\n],")
    }

    // Java: com/google/devtools/build/lib/packages/util/Crosstool.java:297 Crosstool.CcToolchainConfig.getToolchainExecConstraints
    pub fn get_toolchain_exec_constraints(&self) -> String {
        Self::format_constraints("exec", &self.toolchain_exec_constraints)
    }

    // Java: com/google/devtools/build/lib/packages/util/Crosstool.java:301 Crosstool.CcToolchainConfig.getToolchainTargetConstraints
    pub fn get_toolchain_target_constraints(&self) -> String {
        let mut constraints = self.toolchain_target_constraints.clone();
        if constraints.is_empty() {
            if self.get_target_cpu() == "k8" {
                // Use default constraints
                constraints = vec![
                    format!("{CONSTRAINTS_PACKAGE_ROOT}cpu:x86_64"),
                    format!("{CONSTRAINTS_PACKAGE_ROOT}os:linux"),
                ];
            } else if self.get_target_cpu() == "darwin_x86_64" {
                constraints = vec![
                    format!("{CONSTRAINTS_PACKAGE_ROOT}cpu:x86_64"),
                    format!("{CONSTRAINTS_PACKAGE_ROOT}os:macos"),
                ];
            } else if self.get_target_cpu() == "darwin_arm64" {
                constraints = vec![
                    format!("{CONSTRAINTS_PACKAGE_ROOT}cpu:arm64"),
                    format!("{CONSTRAINTS_PACKAGE_ROOT}os:macos"),
                ];
            }
        }
        Self::format_constraints("target", &constraints)
    }

    // Java: com/google/devtools/build/lib/packages/util/Crosstool.java:325 Crosstool.CcToolchainConfig.hasStaticLinkCppRuntimesFeature
    pub fn has_static_link_cpp_runtimes_feature(&self) -> bool {
        self.features.iter().any(|f| f == STATIC_LINK_CPP_RUNTIMES)
    }

    // Java: com/google/devtools/build/lib/packages/util/Crosstool.java:329 Crosstool.CcToolchainConfig.getCcToolchainConfigForCpu
    pub fn get_cc_toolchain_config_for_cpu(cpu: &str) -> CcToolchainConfig {
        CcToolchainConfig {
            cpu: cpu.to_string(),
            compiler: format!("mock-compiler-for-{cpu}"),
            toolchain_identifier: format!("mock-llvm-toolchain-for-{cpu}"),
            host_system_name: format!("mock-system-name-for-{cpu}"),
            target_system_name: format!("mock-target-system-name-for-{cpu}"),
            abi_version: format!("mock-abi-version-for-{cpu}"),
            abi_libc_version: format!("mock-abi-libc-for-{cpu}"),
            target_libc: format!("mock-libc-for-{cpu}"),
            builtin_sysroot: String::new(),
            cc_target_os: String::new(),
            features: Vec::new(),
            action_configs: Vec::new(),
            artifact_name_patterns: Vec::new(),
            tool_paths: Vec::new(),
            cxx_builtin_include_directories: Vec::new(),
            make_variables: Vec::new(),
            toolchain_exec_constraints: Vec::new(),
            toolchain_target_constraints: Vec::new(),
        }
    }

    // Java: com/google/devtools/build/lib/packages/util/Crosstool.java:351 Crosstool.CcToolchainConfig.getDefaultCcToolchainConfig
    pub fn get_default_cc_toolchain_config() -> CcToolchainConfig {
        Self::get_cc_toolchain_config_for_cpu("k8")
    }

    // Java: com/google/devtools/build/lib/packages/util/Crosstool.java:355 Crosstool.CcToolchainConfig.getCcToolchainConfigRule
    pub fn get_cc_toolchain_config_rule(&self) -> String {
        let features_list: Vec<String> = self.features.iter().map(|f| format!("'{f}'")).collect();
        let action_configs_list: Vec<String> = self
            .action_configs
            .iter()
            .map(|c| format!("'{c}'"))
            .collect();
        let patterns_list: Vec<String> = self
            .artifact_name_patterns
            .iter()
            .map(|p| format!("'{}': ['{}', '{}']", p[0], p[1], p[2]))
            .collect();
        let tool_paths_list: Vec<String> = self
            .tool_paths
            .iter()
            .map(|(a, b)| format!("'{a}': '{b}'"))
            .collect();
        let directories_list: Vec<String> = self
            .cxx_builtin_include_directories
            .iter()
            .map(|d| format!("'{d}'"))
            .collect();
        let make_variables_list: Vec<String> = self
            .make_variables
            .iter()
            .map(|(a, b)| format!("'{a}': '{b}'"))
            .collect();
        [
            "cc_toolchain_config(".to_string(),
            format!("  name = '{}-{}_config',", self.cpu, self.compiler),
            format!("  toolchain_identifier = '{}',", self.toolchain_identifier),
            format!("  cpu = '{}',", self.cpu),
            format!("  compiler = '{}',", self.compiler),
            format!("  host_system_name = '{}',", self.host_system_name),
            format!("  target_system_name = '{}',", self.target_system_name),
            format!("  target_libc = '{}',", self.target_libc),
            format!("  abi_version = '{}',", self.abi_version),
            format!("  abi_libc_version = '{}',", self.abi_libc_version),
            format!("  feature_names = [{}],", features_list.join(",\n    ")),
            format!(
                "  action_configs = [{}],",
                action_configs_list.join(",\n    ")
            ),
            format!(
                "  artifact_name_patterns = {{{}}},",
                patterns_list.join(",\n    ")
            ),
            format!("  tool_paths = {{{}}},", tool_paths_list.join(",\n    ")),
            format!("  builtin_sysroot = '{}',", self.builtin_sysroot),
            format!("  cc_target_os = '{}',", self.cc_target_os),
            format!(
                "  cxx_builtin_include_directories = [{}],",
                directories_list.join(",\n    ")
            ),
            format!(
                "  make_variables = {{{}}},",
                make_variables_list.join(",\n    ")
            ),
            "  )".to_string(),
        ]
        .join("\n")
    }
}

/// Java `Crosstool`: writes the mock crosstool package.
// Java: com/google/devtools/build/lib/packages/util/Crosstool.java:37 Crosstool
pub struct Crosstool<'a> {
    config: &'a MockToolsConfig,
    crosstool_top: String,
    /// `crosstoolTopLabel.getPackageIdentifier().getCanonicalForm()`.
    crosstool_top_package: String,
    cc_toolchain_config_file_contents: String,
    archs: Vec<String>,
    supports_header_parsing: bool,
    cc_toolchain_config_list: Vec<CcToolchainConfig>,
}

impl<'a> Crosstool<'a> {
    // Java: com/google/devtools/build/lib/packages/util/Crosstool.java:422 Crosstool.<init>
    pub fn new(
        config: &'a MockToolsConfig,
        crosstool_top: &str,
        crosstool_top_package: &str,
    ) -> Self {
        Crosstool {
            config,
            crosstool_top: crosstool_top.to_string(),
            crosstool_top_package: crosstool_top_package.to_string(),
            cc_toolchain_config_file_contents: String::new(),
            archs: Vec::new(),
            supports_header_parsing: false,
            cc_toolchain_config_list: Vec::new(),
        }
    }

    pub fn set_cc_toolchain_file(mut self, contents: String) -> Self {
        self.cc_toolchain_config_file_contents = contents;
        self
    }

    pub fn set_supported_archs(mut self, archs: &[&str]) -> Self {
        self.archs = strings(archs);
        self
    }

    pub fn set_supports_header_parsing(mut self, supports_header_parsing: bool) -> Self {
        self.supports_header_parsing = supports_header_parsing;
        self
    }

    pub fn set_toolchain_configs(mut self, configs: Vec<CcToolchainConfig>) -> Self {
        self.cc_toolchain_config_list = configs;
        self
    }

    // Java: com/google/devtools/build/lib/packages/util/Crosstool.java:454 Crosstool.write
    pub fn write(self) {
        // PORT: java-hash-order. Java collects the runtime filegroups in a `HashSet<String>` and
        // joins it; the elements are kept in insertion order here and joined in the simulated
        // `HashSet` iteration order (`blaze_repo_fetch::java_hash`).
        let mut runtimes: indexmap::IndexSet<String> = indexmap::IndexSet::new();
        let mut compilation_tools = String::new();
        for compilation_tool in CROSSTOOL_BINARIES {
            let arch_targets: Vec<String> = self
                .archs
                .iter()
                .map(|arch| format!("{compilation_tool}-{arch}"))
                .collect();
            compilation_tools.push_str(&format!(
                "filegroup(name = '{}', srcs = ['{}'])\n",
                compilation_tool,
                arch_targets.join("', '")
            ));
            for arch_target in &arch_targets {
                compilation_tools.push_str(&format!(
                    "filegroup(name = '{arch_target}', srcs = [':everything-multilib'])\n"
                ));
            }
        }

        for toolchain in &self.cc_toolchain_config_list {
            let static_runtime_label =
                toolchain.has_static_link_cpp_runtimes_feature().then(|| {
                    format!(
                        "mock-static-runtimes-target-for-{}",
                        toolchain.get_toolchain_identifier()
                    )
                });
            let dynamic_runtime_label =
                toolchain.has_static_link_cpp_runtimes_feature().then(|| {
                    format!(
                        "mock-dynamic-runtimes-target-for-{}",
                        toolchain.get_toolchain_identifier()
                    )
                });
            if let Some(label) = &static_runtime_label {
                runtimes.insert(
                    [
                        "filegroup(".to_string(),
                        format!("  name = '{label}',"),
                        "  licenses = ['unencumbered'],".to_string(),
                        "  srcs = ['libstatic-runtime-lib-source.a'])".to_string(),
                        String::new(),
                    ]
                    .join("\n"),
                );
            }
            if let Some(label) = &dynamic_runtime_label {
                runtimes.insert(
                    [
                        "filegroup(".to_string(),
                        format!("  name = '{label}',"),
                        "  licenses = ['unencumbered'],".to_string(),
                        "  srcs = ['libdynamic-runtime-lib-source.so'])".to_string(),
                        String::new(),
                    ]
                    .join("\n"),
                );
            }

            // Generate cc_toolchain target
            let suffix = format!(
                "{}-{}",
                toolchain.get_target_cpu(),
                toolchain.get_compiler()
            );
            let cpu = toolchain.get_target_cpu();
            compilation_tools.push_str(
                &[
                    "toolchain(".to_string(),
                    format!("  name = 'cc-toolchain-{suffix}',"),
                    format!("  toolchain_type = '{TOOLS_REPOSITORY}//tools/cpp:toolchain_type',"),
                    format!("  toolchain = ':cc-compiler-{suffix}',"),
                    toolchain.get_toolchain_exec_constraints(),
                    toolchain.get_toolchain_target_constraints(),
                    ")".to_string(),
                    toolchain.get_cc_toolchain_config_rule(),
                    "cc_toolchain(".to_string(),
                    format!("  name = 'cc-compiler-{suffix}',"),
                    format!(
                        "  toolchain_identifier = '{}',",
                        toolchain.get_toolchain_identifier()
                    ),
                    format!("  toolchain_config = ':{suffix}_config',"),
                    "  output_licenses = ['unencumbered'],".to_string(),
                    "  module_map = 'crosstool.cppmap',".to_string(),
                    format!("  ar_files = 'ar-{cpu}',"),
                    format!("  as_files = 'as-{cpu}',"),
                    format!("  compiler_files = 'compile-{cpu}',"),
                    "  coverage_files = 'coverage-file',".to_string(),
                    format!("  dwp_files = 'dwp-{cpu}',"),
                    format!("  linker_files = 'link-{cpu}',"),
                    "  strip_files = ':every-file',".to_string(),
                    format!("  objcopy_files = 'objcopy-{cpu}',"),
                    "  all_files = ':every-file',".to_string(),
                    "  licenses = ['unencumbered'],".to_string(),
                    if self.supports_header_parsing {
                        "    supports_header_parsing = 1,".to_string()
                    } else {
                        String::new()
                    },
                    match &dynamic_runtime_label {
                        None => String::new(),
                        Some(l) => format!("    dynamic_runtime_lib = '{l}',"),
                    },
                    match &static_runtime_label {
                        None => String::new(),
                        Some(l) => format!("    static_runtime_lib = '{l}',"),
                    },
                    ")".to_string(),
                    String::new(),
                ]
                .join("\n"),
            );
        }

        let build = [
            "load('@rules_cc//cc/toolchains:cc_toolchain.bzl', 'cc_toolchain')".to_string(),
            "load('@rules_cc//cc/toolchains:cc_toolchain_alias.bzl', 'cc_toolchain_alias')"
                .to_string(),
            "package(default_visibility=['//visibility:public'])".to_string(),
            "licenses(['restricted'])".to_string(),
            String::new(),
            "load(':cc_toolchain_config.bzl', 'cc_toolchain_config')".to_string(),
            format!(
                "load('{TOOLS_REPOSITORY}//third_party/cc_rules/macros:defs.bzl', 'cc_library', \
                 'cc_toolchain')"
            ),
            "toolchain_type(name = 'toolchain_type')".to_string(),
            "toolchain_type(name = 'test_runner_toolchain_type')".to_string(),
            "cc_toolchain_alias(name = 'current_cc_toolchain')".to_string(),
            "cc_toolchain_alias(".to_string(),
            "    name = 'optional_current_cc_toolchain',".to_string(),
            "    mandatory = False,".to_string(),
            ")".to_string(),
            "alias(name = 'toolchain', actual = 'everything')".to_string(),
            "filegroup(name = 'everything-multilib',".to_string(),
            "          srcs = glob(['mock_version/**/*'],".to_string(),
            "              exclude_directories = 1),".to_string(),
            "          output_licenses = ['unencumbered'])".to_string(),
            String::new(),
            format!(
                "filegroup(name = 'every-file', srcs = ['{}'])",
                CROSSTOOL_BINARIES.join("', '")
            ),
            String::new(),
            compilation_tools,
            blaze_repo_fetch::java_hash::java_hash_set_order(runtimes).join("\n"),
            String::new(),
            "filegroup(".to_string(),
            "    name = 'interface_library_builder',".to_string(),
            "    srcs = ['build_interface_so'],".to_string(),
            ")".to_string(),
            // We add a :link_extra_lib target in case we need it.
            "cc_library(name = 'link_extra_lib', srcs = ['linkextra.cc'])".to_string(),
            // We add an empty :malloc target in case we need it.
            "cc_library(name = 'malloc')".to_string(),
            // Fake targets to get us through loading/analysis.
            "exports_files(['grep-includes', 'link_dynamic_library'])".to_string(),
            String::new(),
            "filegroup(".to_string(),
            "    name = 'aggregate-ddi',".to_string(),
            "    srcs = ['aggregate-ddi.sh'],".to_string(),
            ")".to_string(),
            String::new(),
            "filegroup(".to_string(),
            "    name = 'generate-modmap',".to_string(),
            "    srcs = ['generate-modmap.sh'],".to_string(),
            ")".to_string(),
        ]
        .join("\n");

        let top = &self.crosstool_top;
        self.config
            .create(&format!("{top}/mock_version/x86/bin/gcc"), &[]);
        self.config
            .create(&format!("{top}/mock_version/x86/bin/ld"), &[]);
        self.config.overwrite(&format!("{top}/BUILD"), &[&build]);
        self.config.overwrite(
            &format!("{top}/cc_toolchain_config.bzl"),
            &[&self.cc_toolchain_config_file_contents],
        );
        self.config
            .create(&format!("{top}/crosstool.cppmap"), &["module crosstool {}"]);
        self.config.append(
            "MODULE.bazel",
            &[&format!(
                "register_toolchains('{}:all')",
                self.crosstool_top_package
            )],
        );
        // Empty files to satisfy fake targets.
        self.config.create(&format!("{top}/grep-includes"), &[]);
        self.config
            .create(&format!("{top}/build_interface_so"), &[]);
        self.config
            .create(&format!("{top}/link_dynamic_library"), &[]);
        self.config.create(&format!("{top}/aggregate-ddi.sh"), &[]);
        self.config
            .create(&format!("{top}/generate-modmap.sh"), &[]);
    }
}

/// Java `BazelMockCcSupport` (the `MockCcSupport` of Bazel's tests), with the inherited
/// `MockCcSupport` methods.
// Java: com/google/devtools/build/lib/packages/util/BazelMockCcSupport.java:31 BazelMockCcSupport
pub struct BazelMockCcSupport;

impl BazelMockCcSupport {
    /// `getMockCrosstoolPath()`.
    pub fn get_mock_crosstool_path(&self) -> &'static str {
        "embedded_tools/tools/cpp/"
    }

    /// `getMockCrosstoolLabel().getPackageIdentifier().getCanonicalForm()` of
    /// `@bazel_tools//tools/cpp:toolchain`.
    fn get_mock_crosstool_package(&self) -> &'static str {
        "@@bazel_tools//tools/cpp"
    }

    // Java: com/google/devtools/build/lib/packages/util/MockCcSupport.java:185 MockCcSupport.setupCcToolchainConfigForCpu
    pub fn setup_cc_toolchain_config_for_cpu(&self, config: &MockToolsConfig, cpus: &[&str]) {
        let mut configs = vec![CcToolchainConfig::get_default_cc_toolchain_config()];
        for cpu in cpus {
            configs.push(CcToolchainConfig::get_cc_toolchain_config_for_cpu(cpu));
        }
        self.setup_cc_toolchain_config_list(config, configs);
    }

    // Java: com/google/devtools/build/lib/packages/util/MockCcSupport.java:200 MockCcSupport.setupCcToolchainConfig
    pub fn setup_cc_toolchain_config(&self, config: &MockToolsConfig) {
        self.setup_cc_toolchain_config_with(config, CcToolchainConfig::builder());
    }

    // Java: com/google/devtools/build/lib/packages/util/MockCcSupport.java:205 MockCcSupport.setupCcToolchainConfig
    pub fn setup_cc_toolchain_config_with(
        &self,
        config: &MockToolsConfig,
        cc_toolchain_config: CcToolchainConfigBuilder,
    ) {
        self.setup_cc_toolchain_config_list(config, vec![cc_toolchain_config.build()]);
    }

    // Java: com/google/devtools/build/lib/packages/util/MockCcSupport.java:215 MockCcSupport.setupCcToolchainConfig
    pub fn setup_cc_toolchain_config_list(
        &self,
        config: &MockToolsConfig,
        cc_toolchain_configs: Vec<CcToolchainConfig>,
    ) {
        if config.is_real_file_system() && self.should_use_real_file_system_crosstool() {
            blaze_util::unported!("MockCcSupport.writeToolchainsForRealFilesystemTools");
        }
        Crosstool::new(
            config,
            self.get_mock_crosstool_path(),
            self.get_mock_crosstool_package(),
        )
        .set_cc_toolchain_file(self.read_cc_toolchain_config_file())
        .set_supported_archs(CROSSTOOL_ARCHS)
        .set_toolchain_configs(cc_toolchain_configs)
        .set_supports_header_parsing(true)
        .write();
    }

    /// `shouldUseRealFileSystemCrosstool()`: `OS.getCurrent() == OS.LINUX`.
    fn should_use_real_file_system_crosstool(&self) -> bool {
        true
    }

    // Java: com/google/devtools/build/lib/packages/util/MockCcSupport.java:376 MockCcSupport.readCcToolchainConfigFile
    fn read_cc_toolchain_config_file(&self) -> String {
        let deps = test_deps();
        deps.read_resource(&deps.resource(
            "src/test/java/com/google/devtools/build/lib/analysis/mock/cc_toolchain_config.bzl",
        ))
    }

    /// `ResourceLoader.readFromResources(TestConstants.RULES_CC_REPOSITORY_EXECROOT + path)`: a
    /// file of the real rules_cc.
    fn read_rules_cc_file(&self, path: &str) -> Option<String> {
        let deps = test_deps();
        let file = deps.repository("rules_cc").join(path);
        file.is_file().then(|| deps.read_resource(&file))
    }

    // Java: com/google/devtools/build/lib/packages/util/MockCcSupport.java:260 MockCcSupport.setupRulesCc
    fn setup_rules_cc(&self, config: &MockToolsConfig) {
        for path in [
            "cc/BUILD",
            "cc/defs.bzl",
            "cc/action_names.bzl",
            "cc/cc_binary.bzl",
            "cc/cc_import.bzl",
            "cc/cc_library.bzl",
            "cc/cc_test.bzl",
            "cc/cc_toolchain_config_lib.bzl",
            "cc/objc_import.bzl",
            "cc/objc_library.bzl",
            "cc/common/BUILD",
            "cc/common/cc_common.bzl",
            "cc/find_cc_toolchain.bzl",
            "cc/toolchains/BUILD",
            "cc/toolchains/cc_toolchain.bzl",
            "cc/toolchains/cc_toolchain_alias.bzl",
            "cc/toolchains/cc_toolchain_config_info.bzl",
            "cc/toolchain_utils.bzl",
            "cc/private/rules_impl/BUILD",
        ] {
            let content = self.read_rules_cc_file(path).unwrap_or_else(|| {
                panic!("RuntimeException: Couldn't read rules_cc file from {path}")
            });
            config.overwrite(
                &format!("third_party/bazel_rules/rules_cc/{path}"),
                &[&content],
            );
        }
        // We handle these files separately and ignore errors since only native.bzl will be found
        // for bazel, but the other two for blaze
        // TODO: once rules_cc is released and updated in bazel, move this into the set above
        for path in [
            "cc/private/rules_impl/native.bzl",
            "cc/private/rules_impl/native_cc_common.bzl",
            "cc/private/rules_impl/native_providers.bzl",
        ] {
            if let Some(content) = self.read_rules_cc_file(path) {
                config.overwrite(
                    &format!("third_party/bazel_rules/rules_cc/{path}"),
                    &[&content],
                );
            }
        }

        let lib = self
            .read_rules_cc_file("cc/cc_toolchain_config_lib.bzl")
            .expect("cc/cc_toolchain_config_lib.bzl of rules_cc");
        config.overwrite(
            &format!("{TOOLS_REPOSITORY_SCRATCH}tools/cpp/cc_toolchain_config_lib.bzl"),
            &[&lib],
        );
        let action_names = self
            .read_rules_cc_file("cc/action_names.bzl")
            .expect("cc/action_names.bzl of rules_cc");
        config.overwrite(
            &format!("{TOOLS_REPOSITORY_SCRATCH}tools/build_defs/cc/action_names.bzl"),
            &[&action_names],
        );
        // TestConstants.RULES_CC_REPOSITORY_EXECROOT + "BUILD"
        config.create(
            "external/rules_cc+/BUILD",
            &["genrule(name='license', cmd='exit 0', outs=['dummy_license'])"],
        );
        config.create(
            &format!("{TOOLS_REPOSITORY_SCRATCH}tools/build_defs/cc/BUILD"),
            &[],
        );
        config.append(&format!("{TOOLS_REPOSITORY_SCRATCH}tools/cpp/BUILD"), &[""]);

        // These could be a distinct method
        config.create(
            &format!("{TOOLS_REPOSITORY_SCRATCH}{MOCK_LICENSE_SCRATCH}BUILD"),
            &["genrule(name='license', cmd='exit 0', outs=['dummy_license'])"],
        );
        config.create(
            &format!("{TOOLS_REPOSITORY_SCRATCH}{MOCK_LICENSE_SCRATCH}rules/BUILD"),
            &[],
        );
        config.create(
            &format!("{TOOLS_REPOSITORY_SCRATCH}{MOCK_LICENSE_SCRATCH}rules/license.bzl"),
            &["def license(name, **kwargs):", "    pass", ""],
        );
    }

    // Java: com/google/devtools/build/lib/packages/util/MockCcSupport.java:338 MockCcSupport.createParseHeadersAndLayeringCheckWhitelist
    fn create_parse_headers_and_layering_check_whitelist(config: &MockToolsConfig) {
        config.create(
            &format!(
                "{TOOLS_REPOSITORY_SCRATCH}tools/build_defs/cc/whitelists/parse_headers_and_layering_check/BUILD"
            ),
            &[
                "package_group(",
                "    name = 'disabling_parse_headers_and_layering_check_allowed',",
                "    packages = ['//...']",
                ")",
            ],
        );
    }

    // Java: com/google/devtools/build/lib/packages/util/MockCcSupport.java:349 MockCcSupport.createStarlarkLooseHeadersWhitelist
    pub fn create_starlark_loose_headers_whitelist(config: &MockToolsConfig, packages: &[&str]) {
        let joined_packages = packages
            .iter()
            .map(|s| format!("'{s}'"))
            .collect::<Vec<_>>()
            .join(",");
        config.overwrite(
            &format!(
                "{TOOLS_REPOSITORY_SCRATCH}tools/build_defs/cc/whitelists/starlark_hdrs_check/BUILD"
            ),
            &[
                "package_group(",
                "    name = 'loose_header_check_allowed_in_toolchain',",
                &format!("    packages = [{joined_packages}]"),
                ")",
            ],
        );
    }

    // Java: com/google/devtools/build/lib/packages/util/MockCcSupport.java:395 MockCcSupport.writeMacroFile
    pub fn write_macro_file(&self, config: &MockToolsConfig) {
        let mut rule_names = vec![
            "cc_library",
            "cc_binary",
            "cc_test",
            "cc_import",
            "objc_import",
            "objc_library",
            "cc_toolchain",
            "fdo_profile",
            "fdo_prefetch_hints",
        ];
        // TestConstants.PRODUCT_NAME.equals("bazel")
        rule_names.push("cc_proto_library");
        config.create(
            &format!("{TOOLS_REPOSITORY_SCRATCH}third_party/cc_rules/macros/BUILD"),
            &[""],
        );

        let mut macros = String::new();
        macros.push_str("load('@rules_cc//cc:defs.bzl'");
        for rule_name in &rule_names {
            macros.push_str(&format!(", _{rule_name}='{rule_name}'"));
        }
        macros.push_str(")\n");
        for rule_name in &rule_names {
            macros.push_str(
                &[
                    format!("def {rule_name}(**attrs):"),
                    "    if 'tags' in attrs and attrs['tags'] != None:".to_string(),
                    "        attrs['tags'] = attrs['tags'] + ['__CC_RULES_MIGRATION_DO_NOT_USE_WILL_BREAK__']"
                        .to_string(),
                    "    else:".to_string(),
                    "        attrs['tags'] = ['__CC_RULES_MIGRATION_DO_NOT_USE_WILL_BREAK__']"
                        .to_string(),
                    format!("    _{rule_name}(**attrs)"),
                ]
                .join("\n"),
            );
            macros.push('\n');
        }
        config.create(
            &format!("{TOOLS_REPOSITORY_SCRATCH}third_party/cc_rules/macros/defs.bzl"),
            &[&macros],
        );
    }

    // Java: com/google/devtools/build/lib/packages/util/MockCcSupport.java:438 MockCcSupport.getMacroLoadStatement
    pub fn get_macro_load_statement(&self, load_macro: bool, rule_names: &[&str]) -> String {
        if !load_macro {
            return String::new();
        }
        blaze_util::check_state!(!rule_names.is_empty());
        let quoted: Vec<String> = rule_names.iter().map(|r| format!("'{r}'")).collect();
        format!(
            "load('{TOOLS_REPOSITORY}//third_party/cc_rules/macros:defs.bzl', {})",
            quoted.join(",")
        )
    }

    /// Setup the support for building C/C++.
    // Java: com/google/devtools/build/lib/packages/util/BazelMockCcSupport.java:68 BazelMockCcSupport.setup
    pub fn setup(&self, config: &MockToolsConfig) {
        self.write_macro_file(config);
        self.setup_rules_cc(config);
        // getToolchainConfigs(): CcToolchainConfig.builder().build() (Linux x86-64).
        self.setup_cc_toolchain_config_list(config, vec![CcToolchainConfig::builder().build()]);
        Self::create_parse_headers_and_layering_check_whitelist(config);
        Self::create_starlark_loose_headers_whitelist(config, &["//..."]);
        config.append(
            &format!("{TOOLS_REPOSITORY_SCRATCH}tools/cpp/BUILD"),
            &[
                "load('@rules_cc//cc:cc_library.bzl', 'cc_library')",
                "alias(name='host_xcodes',actual='@local_config_xcode//:host_xcodes')",
            ],
        );
        if config.is_real_file_system() && self.should_use_real_file_system_crosstool() {
            blaze_util::unported!("BazelMockCcSupport.setup (real file system crosstool)");
        }

        // Copies rules_cc from real @rules_cc
        config.create(
            "third_party/bazel_rules/rules_cc/MODULE.bazel",
            &["module(name='rules_cc')"],
        );
        let rules_cc = test_deps().repository("rules_cc");
        config.copy_directory(
            &rules_cc.join("cc"),
            "third_party/bazel_rules/rules_cc/cc",
            usize::MAX,
            true,
        );

        // avoid cc_compatibility_proxy indirection
        for rule_name in [
            "cc_binary",
            "cc_import",
            "cc_library",
            "cc_shared_library",
            "cc_static_library",
            "cc_test",
            "objc_import",
            "objc_library",
        ] {
            config.overwrite(
                &format!("third_party/bazel_rules/rules_cc/cc/{rule_name}.bzl"),
                &[&format!(
                    "load(\"//cc/private/rules_impl:{rule_name}.bzl\", _{rule_name} = \"{rule_name}\")\n{rule_name} = _{rule_name}\n"
                )],
            );
        }
        for rule_name in ["cc_toolchain", "cc_toolchain_alias"] {
            config.overwrite(
                &format!("third_party/bazel_rules/rules_cc/cc/toolchains/{rule_name}.bzl"),
                &[&format!(
                    "load(\"//cc/private/rules_impl:{rule_name}.bzl\", _{rule_name} = \"{rule_name}\")\n{rule_name} = _{rule_name}\n"
                )],
            );
        }
        for rule_name in ["fdo_prefetch_hints", "fdo_profile", "propeller_optimize"] {
            config.overwrite(
                &format!("third_party/bazel_rules/rules_cc/cc/toolchains/{rule_name}.bzl"),
                &[&format!(
                    "load(\"//cc/private/rules_impl/fdo:{rule_name}.bzl\", _{rule_name} = \"{rule_name}\")\n{rule_name} = _{rule_name}\n"
                )],
            );
        }
        config.overwrite(
            "third_party/bazel_rules/rules_cc/cc/common/cc_info.bzl",
            &[r#"load("//cc/private:cc_info.bzl", _CcInfo = "CcInfo")
CcInfo = _CcInfo
"#],
        );
        config.overwrite("third_party/bazel_rules/rules_cc/cc/common/cc_shared_library_info.bzl", &[r#"load("//cc/private:cc_shared_library_info.bzl", _CcSharedLibraryInfo = "CcSharedLibraryInfo")
CcSharedLibraryInfo = _CcSharedLibraryInfo
"#]);
        config.overwrite("third_party/bazel_rules/rules_cc/cc/common/debug_package_info.bzl", &[r#"load("//cc/private:debug_package_info.bzl", _DebugPackageInfo = "DebugPackageInfo")
DebugPackageInfo = _DebugPackageInfo
"#]);
        config.overwrite(
            "third_party/bazel_rules/rules_cc/cc/common/cc_common.bzl",
            &[
                r#"load("//cc/private:cc_common.bzl", _cc_common = "cc_common")
cc_common = _cc_common
"#,
            ],
        );
        config.overwrite(
            "third_party/bazel_rules/rules_cc/cc/common/objc_info.bzl",
            &[
                r#"load("//cc/private:objc_info.bzl", _ObjcInfo = "ObjcInfo")
ObjcInfo = _ObjcInfo
"#,
            ],
        );
        config.overwrite("third_party/bazel_rules/rules_cc/cc/toolchains/cc_toolchain_config_info.bzl", &[r#"load("//cc/private/toolchain_config:cc_toolchain_config_info.bzl", _CcToolchainConfigInfo = "CcToolchainConfigInfo")
CcToolchainConfigInfo = _CcToolchainConfigInfo
"#]);
        config.overwrite("third_party/bazel_rules/rules_cc/cc/toolchains/BUILD", &[]);
        config.overwrite(
            "third_party/bazel_rules/rules_cc/cc/toolchains/impl/BUILD",
            &[],
        );
        config.overwrite("third_party/bazel_rules/rules_cc/cc/common/BUILD", &[]);
        config.overwrite("third_party/bazel_rules/rules_cc/cc/private/BUILD", &[]);
    }
}
