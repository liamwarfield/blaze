//! Port of `rules/java/JavaOptions.java`.

use super::converters::{
    CoreOptionConvertersStrictDepsMode, EmptyToNullLabelConverter, LabelConverter,
    LabelListConverter, LabelMap, LabelMapConverter, OptionLabel, StrictDepsConverter,
};
use super::java_configuration::{
    JavaConfigurationJavaClasspathMode, JavaConfigurationOneVersionEnforcementLevel,
};
use blaze_options::Void;
use blaze_options::options;

use super::fragment_options::FragmentOptions;

// Java: com/google/devtools/build/lib/rules/java/JavaOptions.java:35 JavaOptions
options! {
    /// Command-line options for building Java targets
    pub struct JavaOptions {
        #[option(
            name = "javacopt",
            allow_multiple = true,
            default_value = "null",
            documentation_category = UNCATEGORIZED,
            effect_tags = [UNKNOWN],
            help = "Additional options to pass to javac.",
        )]
        pub javac_opts: Vec<String>,

        #[option(
            name = "host_javacopt",
            allow_multiple = true,
            default_value = "null",
            documentation_category = UNCATEGORIZED,
            effect_tags = [UNKNOWN],
            help = concat!(
                "Additional options to pass to javac when building tools that are executed during a",
                " build.",
            ),
        )]
        pub host_javac_opts: Vec<String>,

        #[option(
            name = "jvmopt",
            allow_multiple = true,
            default_value = "null",
            documentation_category = UNCATEGORIZED,
            effect_tags = [UNKNOWN],
            help = concat!(
                "Additional options to pass to the Java VM. These options will get added to the ",
                "VM startup options of each java_binary target.",
            ),
        )]
        pub jvm_opts: Vec<String>,

        #[option(
            name = "host_jvmopt",
            allow_multiple = true,
            default_value = "null",
            documentation_category = UNCATEGORIZED,
            effect_tags = [UNKNOWN],
            help = concat!(
                "Additional options to pass to the Java VM when building tools that are executed during ",
                " the build. These options will get added to the VM startup options of each ",
                " java_binary target.",
            ),
        )]
        pub host_jvm_opts: Vec<String>,

        #[option(
            name = "use_ijars",
            default_value = "true",
            documentation_category = UNCATEGORIZED,
            effect_tags = [UNKNOWN],
            help = concat!(
                "If enabled, this option causes Java compilation to use interface jars. ",
                "This will result in faster incremental compilation, ",
                "but error messages can be different.",
            ),
        )]
        pub use_ijars: bool,

        #[option(
            name = "java_header_compilation",
            default_value = "true",
            documentation_category = UNCATEGORIZED,
            effect_tags = [UNKNOWN],
            help = "Compile ijars directly from source.",
            old_name = "experimental_java_header_compilation",
        )]
        pub header_compilation: bool,

        #[option(
            name = "java_deps",
            default_value = "true",
            documentation_category = UNCATEGORIZED,
            effect_tags = [UNKNOWN],
            help = "Generate dependency information (for now, compile-time classpath) per Java target.",
        )]
        pub java_deps: bool,

        #[option(
            name = "experimental_java_classpath",
            allow_multiple = false,
            default_value = "bazel",
            converter = JavaClasspathModeConverter,
            documentation_category = UNCATEGORIZED,
            effect_tags = [UNKNOWN],
            help = "Enables reduced classpaths for Java compilations.",
            old_name = "java_classpath",
        )]
        pub java_classpath: JavaConfigurationJavaClasspathMode,

        #[option(
            name = "experimental_inmemory_jdeps_files",
            default_value = "true",
            documentation_category = BUILD_TIME_OPTIMIZATION,
            effect_tags = [LOADING_AND_ANALYSIS, EXECUTION, AFFECTS_OUTPUTS],
            metadata_tags = [EXPERIMENTAL],
            help = concat!(
                "If enabled, the dependency (.jdeps) files generated from Java compilations will be ",
                "passed through in memory directly from the remote build nodes instead of being ",
                "written to disk.",
            ),
        )]
        pub inmemory_jdeps_files: bool,

        #[option(
            name = "java_debug",
            default_value = "null",
            expansion = ["--test_arg=--wrapper_script_flag=--debug", "--test_output=streamed", "--test_strategy=exclusive", "--test_timeout=9999", "--nocache_test_results"],
            documentation_category = UNCATEGORIZED,
            effect_tags = [UNKNOWN],
            help = concat!(
                "Causes the Java virtual machine of a java test to wait for a connection from a ",
                "JDWP-compliant debugger (such as jdb) before starting the test. Implies ",
                "-test_output=streamed.",
            ),
        )]
        pub java_test_debug: Void,

        #[option(
            name = "experimental_strict_java_deps",
            allow_multiple = false,
            default_value = "default",
            converter = StrictDepsConverter,
            documentation_category = INPUT_STRICTNESS,
            effect_tags = [BUILD_FILE_SEMANTICS, EAGERNESS_TO_EXIT],
            help = concat!(
                "If true, checks that a Java target explicitly declares all directly used ",
                "targets as dependencies.",
            ),
            old_name = "strict_java_deps",
        )]
        pub strict_java_deps: CoreOptionConvertersStrictDepsMode,

        #[option(
            name = "experimental_fix_deps_tool",
            default_value = "add_dep",
            documentation_category = UNDOCUMENTED,
            effect_tags = [BUILD_FILE_SEMANTICS],
            help = "Specifies which tool should be used to resolve missing dependencies.",
        )]
        pub fix_deps_tool: String,

        #[option(
            name = "explicit_java_test_deps",
            default_value = "false",
            documentation_category = UNCATEGORIZED,
            effect_tags = [UNKNOWN],
            help = concat!(
                "Explicitly specify a dependency to JUnit or Hamcrest in a java_test instead of ",
                " accidentally obtaining from the TestRunner's deps. Only works for bazel right ",
                "now.",
            ),
        )]
        pub explicit_java_test_deps: bool,

        #[option(
            name = "host_java_launcher",
            default_value = "null",
            converter = EmptyToNullLabelConverter,
            documentation_category = UNCATEGORIZED,
            effect_tags = [UNKNOWN],
            help = "The Java launcher used by tools that are executed during a build.",
        )]
        pub host_java_launcher: Option<OptionLabel>,

        #[option(
            name = "java_launcher",
            default_value = "null",
            converter = EmptyToNullLabelConverter,
            documentation_category = UNCATEGORIZED,
            effect_tags = [UNKNOWN],
            help = concat!(
                "The Java launcher to use when building Java binaries. ",
                " If this flag is set to the empty string, the JDK launcher is used. ",
                "The \"launcher\" attribute overrides this flag. ",
            ),
        )]
        pub java_launcher: Option<OptionLabel>,

        #[option(
            name = "proguard_top",
            default_value = "null",
            converter = LabelConverter,
            documentation_category = UNCATEGORIZED,
            effect_tags = [UNKNOWN],
            help = concat!(
                "Specifies which version of ProGuard to use for code removal when building a Java ",
                "binary.",
            ),
        )]
        pub proguard: Option<OptionLabel>,

        /// Comma-separated list of Mnemonic=label pairs of optimizers to run in the given order, treating
        /// {@code Proguard} specially by substituting in the relevant Proguard binary automatically. All
        /// optimizers must understand the same flags as Proguard.
        #[option(
            name = "bytecode_optimizers",
            default_value = "Proguard",
            converter = LabelMapConverter,
            documentation_category = UNDOCUMENTED,
            effect_tags = [UNKNOWN],
            help = "Do not use.",
            old_name = "experimental_bytecode_optimizers",
        )]
        pub bytecode_optimizers: LabelMap,

        /// If true, the bytecode optimizer will be used to incrementally optimize each compiled Java
        /// artifact.
        #[option(
            name = "experimental_local_java_optimizations",
            default_value = "false",
            documentation_category = UNDOCUMENTED,
            effect_tags = [UNKNOWN],
            metadata_tags = [EXPERIMENTAL],
            help = "Do not use.",
        )]
        pub run_local_java_optimizations: bool,

        /// Configuration for the bytecode optimizer if --experimental_local_java_optimizations is enabled.
        #[option(
            name = "experimental_local_java_optimization_configuration",
            default_value = "null",
            converter = LabelConverter,
            documentation_category = UNDOCUMENTED,
            effect_tags = [UNKNOWN],
            help = "Do not use.",
        )]
        pub local_java_optimization_configuration: Option<OptionLabel>,

        /// If true, the OPTIMIZATION stage of the bytecode optimizer will be split across two actions.
        #[option(
            name = "split_bytecode_optimization_pass",
            default_value = "false",
            documentation_category = UNDOCUMENTED,
            effect_tags = [UNKNOWN],
            help = "Do not use.",
        )]
        pub split_bytecode_optimization_pass: bool,

        /// This specifies the number of actions to divide the OPTIMIZATION stage of the bytecode optimizer
        /// into. Note that if split_bytecode_optimization_pass is set, bytecode_optimization_pass_actions
        /// will only effectively change build behavior if it is > 2.
        #[option(
            name = "bytecode_optimization_pass_actions",
            default_value = "1",
            documentation_category = UNDOCUMENTED,
            effect_tags = [UNKNOWN],
            help = "Do not use.",
        )]
        pub bytecode_optimization_pass_actions: i32,

        #[option(
            name = "enforce_proguard_file_extension",
            default_value = "false",
            documentation_category = UNDOCUMENTED,
            effect_tags = [EAGERNESS_TO_EXIT],
            help = concat!(
                "If enabled, requires that ProGuard configuration files outside of third_party/ use the",
                " *.pgcfg file extension.",
            ),
        )]
        pub enforce_proguard_file_extension: bool,

        #[option(
            name = "experimental_one_version_enforcement",
            default_value = "OFF",
            converter = OneVersionEnforcementLevelConverter,
            documentation_category = INPUT_STRICTNESS,
            effect_tags = [LOADING_AND_ANALYSIS],
            help = concat!(
                "When enabled, enforce that a java_binary rule can't contain more than one version ",
                "of the same class file on the classpath. This enforcement can break the build, or ",
                "can just result in warnings.",
            ),
        )]
        pub enforce_one_version: JavaConfigurationOneVersionEnforcementLevel,

        #[option(
            name = "one_version_enforcement_on_java_tests",
            default_value = "true",
            documentation_category = INPUT_STRICTNESS,
            effect_tags = [LOADING_AND_ANALYSIS],
            help = concat!(
                "When enabled, and with experimental_one_version_enforcement set to a non-NONE value,",
                " enforce one version on java_test targets. This flag can be disabled to improve",
                " incremental test performance at the expense of missing potential one version",
                " violations.",
            ),
        )]
        pub enforce_one_version_on_java_tests: bool,

        #[option(
            name = "experimental_add_test_support_to_compile_time_deps",
            default_value = "true",
            documentation_category = UNDOCUMENTED,
            effect_tags = [UNKNOWN],
            metadata_tags = [EXPERIMENTAL],
            help = concat!(
                "Flag to help transition away from adding test support libraries to the compile-time",
                " deps of Java test rules.",
            ),
        )]
        pub add_test_support_to_compile_time_deps: bool,

        #[option(
            name = "experimental_run_android_lint_on_java_rules",
            default_value = "false",
            documentation_category = UNCATEGORIZED,
            effect_tags = [AFFECTS_OUTPUTS],
            metadata_tags = [EXPERIMENTAL],
            help = "Whether to validate java_* sources.",
        )]
        pub run_android_lint: bool,

        #[option(
            name = "plugin",
            converter = LabelListConverter,
            allow_multiple = true,
            default_value = "null",
            documentation_category = UNCATEGORIZED,
            effect_tags = [UNKNOWN],
            help = "Plugins to use in the build. Currently works with java_plugin.",
        )]
        pub plugin_list: Vec<OptionLabel>,

        #[option(
            name = "experimental_turbine_annotation_processing",
            default_value = "false",
            documentation_category = UNDOCUMENTED,
            effect_tags = [UNKNOWN],
            metadata_tags = [EXPERIMENTAL],
            help = "If enabled, turbine is used for all annotation processing",
        )]
        pub experimental_turbine_annotation_processing: bool,

        #[option(
            name = "java_runtime_version",
            default_value = "local_jdk",
            documentation_category = UNCATEGORIZED,
            effect_tags = [UNKNOWN],
            help = "The Java runtime version",
        )]
        pub java_runtime_version: String,

        #[option(
            name = "tool_java_runtime_version",
            default_value = "remotejdk_11",
            documentation_category = UNCATEGORIZED,
            effect_tags = [UNKNOWN],
            help = "The Java runtime version used to execute tools during the build",
        )]
        pub host_java_runtime_version: String,

        #[option(
            name = "java_language_version",
            default_value = "",
            documentation_category = UNCATEGORIZED,
            effect_tags = [UNKNOWN],
            help = "The Java language version",
        )]
        pub java_language_version: String,

        #[option(
            name = "tool_java_language_version",
            default_value = "",
            documentation_category = UNCATEGORIZED,
            effect_tags = [UNKNOWN],
            help = "The Java language version used to execute the tools that are needed during a build",
        )]
        pub host_java_language_version: String,

        #[option(
            name = "incompatible_multi_release_deploy_jars",
            default_value = "true",
            documentation_category = UNDOCUMENTED,
            effect_tags = [UNKNOWN],
            metadata_tags = [INCOMPATIBLE_CHANGE],
            help = "When enabled, java_binary creates Multi-Release deploy jars.",
        )]
        pub multi_release_deploy_jars: bool,

        #[option(
            name = "incompatible_disallow_java_import_exports",
            default_value = "false",
            documentation_category = UNDOCUMENTED,
            effect_tags = [UNKNOWN],
            metadata_tags = [INCOMPATIBLE_CHANGE],
            help = "When enabled, java_import.exports is not supported.",
        )]
        pub disallow_java_import_exports: bool,

        #[option(
            name = "experimental_enable_jspecify",
            default_value = "true",
            documentation_category = UNDOCUMENTED,
            effect_tags = [UNKNOWN],
            metadata_tags = [EXPERIMENTAL],
            help = "Enable experimental jspecify integration.",
        )]
        pub experimental_enable_jspecify: bool,

        #[option(
            name = "experimental_java_test_auto_create_deploy_jar",
            default_value = "false",
            documentation_category = UNDOCUMENTED,
            effect_tags = [UNKNOWN],
            metadata_tags = [EXPERIMENTAL],
            help = "DO NOT USE",
        )]
        pub auto_create_deploy_jar_for_java_tests: bool,
    }
}

impl FragmentOptions for JavaOptions {}

// Java: com/google/devtools/build/lib/rules/java/JavaOptions.java:37 JavaOptions.JavaClasspathModeConverter
blaze_options::enum_converter!(
    pub JavaClasspathModeConverter,
    JavaConfigurationJavaClasspathMode,
    "Java classpath reduction strategy"
);

// Java: com/google/devtools/build/lib/rules/java/JavaOptions.java:44 JavaOptions.OneVersionEnforcementLevelConverter
blaze_options::enum_converter!(
    pub OneVersionEnforcementLevelConverter,
    JavaConfigurationOneVersionEnforcementLevel,
    "Enforcement level for Java One Version violations"
);

#[cfg(test)]
mod tests {
    use super::super::test_util::{assert_defaults_round_trip, assert_option_names, parse};
    use super::*;

    #[test]
    fn option_names_match_java() {
        assert_option_names::<JavaOptions>(&[
            "javacopt",
            "host_javacopt",
            "jvmopt",
            "host_jvmopt",
            "use_ijars",
            "java_header_compilation",
            "java_deps",
            "experimental_java_classpath",
            "experimental_inmemory_jdeps_files",
            "java_debug",
            "experimental_strict_java_deps",
            "experimental_fix_deps_tool",
            "explicit_java_test_deps",
            "host_java_launcher",
            "java_launcher",
            "proguard_top",
            "bytecode_optimizers",
            "experimental_local_java_optimizations",
            "experimental_local_java_optimization_configuration",
            "split_bytecode_optimization_pass",
            "bytecode_optimization_pass_actions",
            "enforce_proguard_file_extension",
            "experimental_one_version_enforcement",
            "one_version_enforcement_on_java_tests",
            "experimental_add_test_support_to_compile_time_deps",
            "experimental_run_android_lint_on_java_rules",
            "plugin",
            "experimental_turbine_annotation_processing",
            "java_runtime_version",
            "tool_java_runtime_version",
            "java_language_version",
            "tool_java_language_version",
            "incompatible_multi_release_deploy_jars",
            "incompatible_disallow_java_import_exports",
            "experimental_enable_jspecify",
            "experimental_java_test_auto_create_deploy_jar",
        ]);
    }

    #[test]
    fn defaults_round_trip() {
        assert_defaults_round_trip::<JavaOptions>(&[]);
    }

    #[test]
    fn spot_check_parses() {
        let o = parse::<JavaOptions>(&[
            "--javacopt=-g",
            "--experimental_java_classpath=bazel_no_fallback",
            "--bytecode_optimizers=Optimizer=//opt:tool,Other",
            "--experimental_one_version_enforcement=warning",
            "--plugin=//p:a",
        ])
        .unwrap();
        assert_eq!(o.javac_opts, vec!["-g"]);
        assert_eq!(
            o.java_classpath,
            JavaConfigurationJavaClasspathMode::BAZEL_NO_FALLBACK
        );
        assert_eq!(
            o.bytecode_optimizers.get("Optimizer").unwrap().to_string(),
            "//opt:tool"
        );
        assert!(o.bytecode_optimizers.contains_key("Other"));
        assert_eq!(
            o.enforce_one_version,
            JavaConfigurationOneVersionEnforcementLevel::WARNING
        );
        assert_eq!(o.plugin_list.len(), 1);
        assert_eq!(
            parse::<JavaOptions>(&["--experimental_java_classpath=foo"])
                .unwrap_err()
                .get_message(),
            "While parsing option --experimental_java_classpath=foo: Not a valid Java classpath \
             reduction strategy: 'foo' (should be off, javabuilder, bazel or bazel_no_fallback)"
        );
    }
}
