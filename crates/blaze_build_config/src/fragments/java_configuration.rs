//! Port of `rules/java/JavaConfiguration.java` (the fragment; its option value enums are
//! `blaze_optdefs::fragment::java_configuration`).

use std::any::Any;
use std::sync::Arc;

use blaze_cmdline::Label;
use blaze_optdefs::fragment::converters::CoreOptionConvertersStrictDepsMode;
use blaze_optdefs::fragment::java_configuration::{
    JavaConfigurationJavaClasspathMode, JavaConfigurationOneVersionEnforcementLevel,
};
use blaze_optdefs::fragment::java_options::JavaOptions;
use blaze_optdefs::fragment::shell_utils;
use blaze_options::converters::JavaEnum;

use crate::build_options::BuildOptions;
use crate::fragment::{
    Fragment, FragmentClass, FragmentClassInfo, InvalidConfigurationException, required,
};
use crate::fragment_options_class::{JAVA_OPTIONS, PLATFORM_OPTIONS};

// Java: com/google/devtools/build/lib/rules/java/JavaConfiguration.java:282 JavaConfiguration.NamedLabel
/// Stores a String name and an optional associated label.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct JavaConfigurationNamedLabel {
    name: String,
    label: Option<Label>,
}

impl JavaConfigurationNamedLabel {
    // Java: com/google/devtools/build/lib/rules/java/JavaConfiguration.java:288 JavaConfiguration.NamedLabel.create
    pub fn create(name: String, label: Option<Label>) -> JavaConfigurationNamedLabel {
        JavaConfigurationNamedLabel { name, label }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn label(&self) -> Option<&Label> {
        self.label.as_ref()
    }
}

// Java: com/google/devtools/build/lib/rules/java/JavaConfiguration.java:46 JavaConfiguration
/// A Java compiler configuration containing the flags required for compilation.
#[derive(Debug)]
pub struct JavaConfiguration {
    /// PORT: Java's `NestedSet<String>` (NAIVE_LINK_ORDER) built by
    /// `JavaHelper.detokenizeJavaOptions`: empty, or a single element. Kept as its element list.
    command_line_javac_flags: Vec<String>,
    java_launcher_label: Option<Label>,
    use_ijars: bool,
    use_header_compilation: bool,
    generate_java_deps: bool,
    enforce_one_version: JavaConfigurationOneVersionEnforcementLevel,
    enforce_one_version_on_java_tests: bool,
    java_classpath: JavaConfigurationJavaClasspathMode,
    inmemory_jdeps_files: bool,
    default_jvm_flags: Vec<String>,
    strict_java_deps: CoreOptionConvertersStrictDepsMode,
    fix_deps_tool: String,
    proguard_binary: Option<Label>,
    bytecode_optimizer: JavaConfigurationNamedLabel,
    run_local_java_optimizations: bool,
    local_java_optimization_configuration: Option<Label>,
    split_bytecode_optimization_pass: bool,
    bytecode_optimization_pass_actions: i32,
    enforce_proguard_file_extension: bool,
    run_android_lint: bool,
    explicit_java_test_deps: bool,
    add_test_support_to_compile_time_deps: bool,
    plugin_list: Vec<Label>,
    experimental_turbine_annotation_processing: bool,
    experimental_enable_jspecify: bool,
    multi_release_deploy_jars: bool,
    disallow_java_import_exports: bool,
    auto_create_deploy_jar_for_java_tests: bool,
}

/// `JavaConfiguration.class`.
pub static JAVA_CONFIGURATION: FragmentClassInfo = FragmentClassInfo {
    java_name: "com.google.devtools.build.lib.rules.java.JavaConfiguration",
    starlark_name: Some("java"),
    required_options: || vec![&JAVA_OPTIONS, &PLATFORM_OPTIONS],
    requires_starlark_options: false,
    constructor: create,
};

fn create(options: &BuildOptions) -> Result<Arc<dyn Fragment>, InvalidConfigurationException> {
    let fragment: Arc<dyn Fragment> = Arc::new(JavaConfiguration::new(options)?);
    Ok(fragment)
}

// Java: com/google/devtools/build/lib/rules/java/JavaHelper.java:48 JavaHelper.tokenizeJavaOptions
/// Javac options require special processing - People use them and expect the options to be
/// tokenized.
// PORT: JavaHelper (rules/java) is not ported elsewhere; ported locally.
fn tokenize_java_options<'a>(in_opts: impl IntoIterator<Item = &'a String>) -> Vec<String> {
    // Ideally, this would be in the options parser. Unfortunately,
    // the options parser can't handle a converter that expands
    // from a value X into a List<X> and allow-multiple at the
    // same time.
    let mut result: Vec<String> = Vec::new();
    for current in in_opts {
        // Like Java, tokens found before a failure stay in `result`.
        if shell_utils::tokenize(&mut result, current).is_err() {
            // Tokenization failed; this likely means that the user
            // did not want tokenization to happen on their argument.
            // (Any tokenization where we should produce an error
            // has already been done by the shell that invoked
            // blaze). Therefore, pass the argument through to
            // the tool, so that we can see the original error.
            result.push(current.clone());
        }
    }
    result
}

// Java: com/google/devtools/build/lib/rules/java/JavaHelper.java:88 JavaHelper.detokenizeJavaOptions
/// De-tokenizes a collection of `javac` options into a `NestedSet`.
///
/// Each option is shell-escaped to get back the original option as-is when we tokenize the
/// depset string.
fn detokenize_java_options(javac_opts: &[String]) -> Vec<String> {
    if javac_opts.is_empty() {
        return Vec::new();
    }
    vec![
        javac_opts
            .iter()
            .map(|s| shell_escape(s))
            .collect::<Vec<_>>()
            .join(" "),
    ]
}

const SAFE_PUNCTUATION: &[u8] = b"@%-_+:,./";

/// `Character.isLetterOrDigit(c)` for a Latin-1 char (Java holds Bazel's internal strings as
/// Latin-1 chars, one per UTF-8 byte).
fn is_latin1_letter_or_digit(b: u8) -> bool {
    b.is_ascii_alphanumeric()
        || b == 0xAA
        || b == 0xB5
        || b == 0xBA
        || (0xC0..=0xD6).contains(&b)
        || (0xD8..=0xF6).contains(&b)
        || b >= 0xF8
}

// Java: com/google/devtools/build/lib/shell/ShellUtils.java:36 ShellUtils.shellEscape
/// Quotes a word so that it can be used, without further quoting, as an argument (or part of an
/// argument) in a shell command.
// PORT: copy of blaze_shell::shell_utils::ShellUtils::shell_escape (blaze_shell is not a
// dependency of this crate).
fn shell_escape(word: &str) -> String {
    if word.is_empty() {
        // Empty string is a special case: needs to be quoted to ensure that it gets treated as a
        // separate argument.
        return "''".to_string();
    }
    // PORT: StringEncoding: Java iterates the internal (Latin-1) form, one char per UTF-8 byte,
    // so the check is per byte.
    for &c in word.as_bytes() {
        // We do this positively so as to be sure we don't inadvertently forget any unsafe
        // characters.
        if !is_latin1_letter_or_digit(c) && !SAFE_PUNCTUATION.contains(&c) {
            // replace() actually means "replace all".
            return format!("'{}'", word.replace('\'', "'\\''"));
        }
    }
    word.to_string()
}

impl JavaConfiguration {
    // Java: com/google/devtools/build/lib/rules/java/JavaConfiguration.java:105 JavaConfiguration.<init>
    pub fn new(
        build_options: &BuildOptions,
    ) -> Result<JavaConfiguration, InvalidConfigurationException> {
        let java_options = required::<JavaOptions>(build_options);
        let command_line_javac_flags =
            detokenize_java_options(&tokenize_java_options(&java_options.javac_opts));
        let optimizers = &java_options.bytecode_optimizers;
        if optimizers.0.len() != 1 {
            return Err(InvalidConfigurationException::new(format!(
                "--experimental_bytecode_optimizers can only accept exactly one mapping, but {} \
                 mappings were provided.",
                optimizers.0.len()
            )));
        }
        let (mnemonic, optimizer_label) = &optimizers.0[0];
        if optimizer_label.is_none() && mnemonic != "Proguard" {
            return Err(InvalidConfigurationException::new(format!(
                "Must supply label for optimizer {}",
                mnemonic
            )));
        }
        let bytecode_optimizer =
            JavaConfigurationNamedLabel::create(mnemonic.clone(), optimizer_label.clone());
        let run_local_java_optimizations = java_options.run_local_java_optimizations;
        if run_local_java_optimizations && optimizer_label.is_none() {
            return Err(InvalidConfigurationException::new(
                "--experimental_local_java_optimizations cannot be provided without \
                 --experimental_bytecode_optimizers.",
            ));
        }

        Ok(JavaConfiguration {
            command_line_javac_flags,
            java_launcher_label: java_options.java_launcher.as_ref().map(|l| l.get().clone()),
            use_ijars: java_options.use_ijars,
            use_header_compilation: java_options.header_compilation,
            generate_java_deps: java_options.java_deps
                || java_options.java_classpath != JavaConfigurationJavaClasspathMode::OFF,
            java_classpath: java_options.java_classpath,
            inmemory_jdeps_files: java_options.inmemory_jdeps_files,
            default_jvm_flags: java_options.jvm_opts.clone(),
            strict_java_deps: java_options.strict_java_deps,
            fix_deps_tool: java_options.fix_deps_tool.clone(),
            proguard_binary: java_options.proguard.as_ref().map(|l| l.get().clone()),
            run_local_java_optimizations,
            local_java_optimization_configuration: java_options
                .local_java_optimization_configuration
                .as_ref()
                .map(|l| l.get().clone()),
            split_bytecode_optimization_pass: java_options.split_bytecode_optimization_pass,
            bytecode_optimization_pass_actions: java_options.bytecode_optimization_pass_actions,
            enforce_proguard_file_extension: java_options.enforce_proguard_file_extension,
            enforce_one_version: java_options.enforce_one_version,
            enforce_one_version_on_java_tests: java_options.enforce_one_version_on_java_tests,
            explicit_java_test_deps: java_options.explicit_java_test_deps,
            add_test_support_to_compile_time_deps: java_options
                .add_test_support_to_compile_time_deps,
            run_android_lint: java_options.run_android_lint,
            multi_release_deploy_jars: java_options.multi_release_deploy_jars,
            disallow_java_import_exports: java_options.disallow_java_import_exports,
            auto_create_deploy_jar_for_java_tests: java_options
                .auto_create_deploy_jar_for_java_tests,
            bytecode_optimizer,
            plugin_list: java_options
                .plugin_list
                .iter()
                .map(|l| l.get().clone())
                .collect(),
            experimental_turbine_annotation_processing: java_options
                .experimental_turbine_annotation_processing,
            experimental_enable_jspecify: java_options.experimental_enable_jspecify,
        })
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaConfiguration.java:163 JavaConfiguration.getDefaultJavacFlagsForStarlarkAsList
    // TODO(bazel-team): this is the command-line passed options, we should remove from Starlark
    // probably.
    pub fn get_default_javac_flags_for_starlark_as_list(&self) -> Vec<String> {
        // JavaHelper.tokenizeJavaOptions(NestedSet) tokenizes `inOpts.toList().reverse()`.
        tokenize_java_options(self.command_line_javac_flags.iter().rev())
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaConfiguration.java:170 JavaConfiguration.getDefaultJavacFlagsStarlark
    /// The elements of the `Depset` of command-line javac flags.
    // TODO(bazel-team): this is the command-line passed options, we should remove from Starlark
    // probably.
    pub fn get_default_javac_flags_starlark(&self) -> &[String] {
        &self.command_line_javac_flags
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaConfiguration.java:175 JavaConfiguration.getStrictJavaDepsName
    pub fn get_strict_java_deps_name(&self) -> String {
        self.strict_java_deps.name().to_ascii_lowercase()
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaConfiguration.java:180 JavaConfiguration.getUseIjars
    /// Returns true iff Java compilation should use ijars.
    pub fn get_use_ijars(&self) -> bool {
        self.use_ijars
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaConfiguration.java:195 JavaConfiguration.useHeaderCompilationStarlark
    // PORT: the Starlark method's private-access check belongs to the Starlark wrapper.
    pub fn use_header_compilation(&self) -> bool {
        self.use_header_compilation
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaConfiguration.java:201 JavaConfiguration.getGenerateJavaDeps
    /// Returns true iff dependency information is generated after compilation.
    pub fn get_generate_java_deps(&self) -> bool {
        self.generate_java_deps
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaConfiguration.java:211 JavaConfiguration.getReduceJavaClasspath
    pub fn get_reduce_java_classpath(&self) -> JavaConfigurationJavaClasspathMode {
        self.java_classpath
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaConfiguration.java:221 JavaConfiguration.inmemoryJdepsFiles
    pub fn inmemory_jdeps_files(&self) -> bool {
        self.inmemory_jdeps_files
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaConfiguration.java:226 JavaConfiguration.getDefaultJvmFlags
    pub fn get_default_jvm_flags(&self) -> &[String] {
        &self.default_jvm_flags
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaConfiguration.java:231 JavaConfiguration.getFixDepsTool
    /// Which tool to use for fixing dependency errors.
    pub fn get_fix_deps_tool(&self) -> &str {
        &self.fix_deps_tool
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaConfiguration.java:241 JavaConfiguration.getJavaLauncherLabel
    /// Returns proper label only if --java_launcher= is specified, otherwise `None`.
    pub fn get_java_launcher_label(&self) -> Option<&Label> {
        self.java_launcher_label.as_ref()
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaConfiguration.java:251 JavaConfiguration.getProguardBinary
    /// Returns the label provided with --proguard_top, if any.
    pub fn get_proguard_binary(&self) -> Option<&Label> {
        self.proguard_binary.as_ref()
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaConfiguration.java:260 JavaConfiguration.splitBytecodeOptimizationPass
    /// Returns whether the OPTIMIZATION stage of the bytecode optimizer will be split across two
    /// actions.
    pub fn split_bytecode_optimization_pass(&self) -> bool {
        self.split_bytecode_optimization_pass
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaConfiguration.java:270 JavaConfiguration.bytecodeOptimizationPassActions
    /// This specifies the number of actions to divide the OPTIMIZATION stage of the bytecode
    /// optimizer into. Note that if split_bytecode_optimization_pass is set, this will only
    /// change behavior if it is > 2.
    pub fn bytecode_optimization_pass_actions(&self) -> i32 {
        self.bytecode_optimization_pass_actions
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaConfiguration.java:276 JavaConfiguration.enforceProguardFileExtension
    /// Returns whether ProGuard configuration files are required to use a *.pgcfg extension.
    pub fn enforce_proguard_file_extension(&self) -> bool {
        self.enforce_proguard_file_extension
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaConfiguration.java:295 JavaConfiguration.getBytecodeOptimizer
    /// Returns bytecode optimizer to run.
    pub fn get_bytecode_optimizer(&self) -> &JavaConfigurationNamedLabel {
        &self.bytecode_optimizer
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaConfiguration.java:300 JavaConfiguration.getBytecodeOptimizerMnemonic
    pub fn get_bytecode_optimizer_mnemonic(&self) -> &str {
        self.bytecode_optimizer.name()
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaConfiguration.java:309 JavaConfiguration.getBytecodeOptimizerLabel
    pub fn get_bytecode_optimizer_label(&self) -> Option<&Label> {
        self.bytecode_optimizer.label()
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaConfiguration.java:314 JavaConfiguration.runLocalJavaOptimizations
    /// Returns true if the bytecode optimizer should incrementally optimize all Java artifacts.
    pub fn run_local_java_optimizations(&self) -> bool {
        self.run_local_java_optimizations
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaConfiguration.java:323 JavaConfiguration.getBytecodeOptimizerLabelForJavaToolchain
    pub fn get_bytecode_optimizer_label_for_java_toolchain(&self) -> Option<&Label> {
        if self.run_local_java_optimizations {
            self.bytecode_optimizer.label()
        } else {
            None
        }
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaConfiguration.java:334 JavaConfiguration.getLocalJavaOptimizationConfiguration
    /// Returns the optimization configuration for local Java optimizations if they are enabled.
    pub fn get_local_java_optimization_configuration(&self) -> Option<&Label> {
        if self.run_local_java_optimizations {
            self.local_java_optimization_configuration.as_ref()
        } else {
            None
        }
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaConfiguration.java:347 JavaConfiguration.explicitJavaTestDepsStarlark
    /// Make it mandatory for java_test targets to explicitly declare any JUnit or Hamcrest
    /// dependencies instead of accidentally obtaining them from the TestRunner's dependencies.
    // PORT: the Starlark method's private-access check belongs to the Starlark wrapper.
    pub fn explicit_java_test_deps(&self) -> bool {
        self.explicit_java_test_deps
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaConfiguration.java:353 JavaConfiguration.multiReleaseDeployJars
    pub fn multi_release_deploy_jars(&self) -> bool {
        self.multi_release_deploy_jars
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaConfiguration.java:359 JavaConfiguration.getDisallowJavaImportExportsInStarlark
    /// Returns true if java_import exports are not allowed.
    // PORT: the Starlark method's private-access check belongs to the Starlark wrapper.
    pub fn get_disallow_java_import_exports(&self) -> bool {
        self.disallow_java_import_exports
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaConfiguration.java:373 JavaConfiguration.starlarkOneVersionEnforcementLevel
    /// Returns the name of the enum representing whether or not Bazel should attempt to enforce
    /// one-version correctness on java_binary rules using the 'oneversion' tool in the
    /// java_toolchain.
    ///
    /// One-version correctness will inspect for multiple non-identical versions of java classes
    /// in the transitive dependencies for a java_binary.
    pub fn starlark_one_version_enforcement_level(&self) -> &'static str {
        self.enforce_one_version.name()
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaConfiguration.java:378 JavaConfiguration.enforceOneVersionOnJavaTests
    pub fn enforce_one_version_on_java_tests(&self) -> bool {
        self.enforce_one_version_on_java_tests
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaConfiguration.java:383 JavaConfiguration.addTestSupportToCompileTimeDeps
    pub fn add_test_support_to_compile_time_deps(&self) -> bool {
        self.add_test_support_to_compile_time_deps
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaConfiguration.java:388 JavaConfiguration.runAndroidLint
    pub fn run_android_lint(&self) -> bool {
        self.run_android_lint
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaConfiguration.java:393 JavaConfiguration.getPlugins
    pub fn get_plugins(&self) -> &[Label] {
        &self.plugin_list
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaConfiguration.java:397 JavaConfiguration.experimentalTurbineAnnotationProcessing
    pub fn experimental_turbine_annotation_processing(&self) -> bool {
        self.experimental_turbine_annotation_processing
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaConfiguration.java:401 JavaConfiguration.experimentalEnableJspecify
    pub fn experimental_enable_jspecify(&self) -> bool {
        self.experimental_enable_jspecify
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaConfiguration.java:406 JavaConfiguration.autoCreateJavaTestDeployJars
    // PORT: the Starlark method's BuiltinRestriction check belongs to the Starlark wrapper.
    pub fn auto_create_java_test_deploy_jars(&self) -> bool {
        self.auto_create_deploy_jar_for_java_tests
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaConfiguration.java:413 JavaConfiguration.getUseHeaderCompilationDirectDepsInStarlark
    // TODO: b/417791104 - Remove this method once usages are removed.
    // PORT: the Starlark method's private-access check belongs to the Starlark wrapper.
    pub fn get_use_header_compilation_direct_deps(&self) -> bool {
        true
    }
}

impl Fragment for JavaConfiguration {
    fn fragment_class(&self) -> FragmentClass {
        &JAVA_CONFIGURATION
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fragment_options_class::all_known_fragment_options_classes;

    // PORT: ConfigurationTestCase.create builds the whole configuration; these tests construct
    // the fragment directly from the parsed options.
    fn create(args: &[&str]) -> Result<JavaConfiguration, InvalidConfigurationException> {
        let options = BuildOptions::of_args(&all_known_fragment_options_classes(), args)
            .unwrap_or_else(|e| panic!("{}", e.get_message()));
        JavaConfiguration::new(&options)
    }

    #[test]
    fn test_experimental_bytecode_optimizers_flag() {
        let thrown = create(&["--experimental_bytecode_optimizers=somekey,somevalue"]).unwrap_err();
        assert!(
            thrown
                .get_message()
                .contains("can only accept exactly one mapping")
        );
    }

    #[test]
    fn default_bytecode_optimizer_is_proguard() {
        let config = create(&[]).unwrap();
        assert_eq!(config.get_bytecode_optimizer_mnemonic(), "Proguard");
        assert!(config.get_bytecode_optimizer_label().is_none());
    }

    #[test]
    fn javac_flags_round_trip() {
        let config = create(&["--javacopt=-source 8", "--javacopt=-Xlint:'a b'"]).unwrap();
        assert_eq!(
            config.get_default_javac_flags_starlark(),
            &["-source 8 '-Xlint:a b'"]
        );
        assert_eq!(
            config.get_default_javac_flags_for_starlark_as_list(),
            vec!["-source", "8", "-Xlint:a b"]
        );
    }

    #[test]
    fn shell_escape_quotes_unsafe() {
        assert_eq!(shell_escape(""), "''");
        assert_eq!(shell_escape("abc"), "abc");
        assert_eq!(shell_escape("a b"), "'a b'");
        assert_eq!(shell_escape("a'b"), "'a'\\''b'");
    }
}
