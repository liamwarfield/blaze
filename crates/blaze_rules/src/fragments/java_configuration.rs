//! The Starlark value of the `java` fragment (`rules/java/JavaConfiguration.java`, Starlark API
//! `starlarkbuildapi/java/JavaConfigurationApi.java`).

use blaze_analysis::rule::builtin_restriction::fail_if_called_outside_default_allowlist;
use blaze_analysis::rule::depsets::{depset_value, strings_to_depset};
use blaze_build_config::fragments::java_configuration::{JAVA_CONFIGURATION, JavaConfiguration};
use blaze_collect::{NestedSetBuilder, Order};
use blaze_options::converters::JavaEnum;
use blaze_starlark::label::LabelValue;
use starlark::environment::MethodsBuilder;
use starlark::eval::Evaluator;
use starlark::starlark_module;
use starlark::values::{Heap, Value};

use super::fragment_value;

fragment_value!(
    /// `ctx.fragments.java`: a java compiler configuration.
    // Java: com/google/devtools/build/lib/rules/java/JavaConfiguration.java:46 JavaConfiguration
    // Java: com/google/devtools/build/lib/starlarkbuildapi/java/JavaConfigurationApi.java:27 JavaConfigurationApi
    JavaConfigurationValue,
    JavaConfiguration,
    "java",
    "com.google.devtools.build.lib.rules.java.JavaConfiguration",
    java_configuration_methods
);

pub(super) fn register() {
    super::register(&JAVA_CONFIGURATION, JavaConfigurationValue::alloc);
}

/// Java `ImmutableList<String>` returned to Starlark (`Starlark.fromJava`: an immutable list).
fn immutable_string_list<'v>(heap: Heap<'v>, items: &[String]) -> Value<'v> {
    let elems: Vec<Value<'v>> = items.iter().map(|s| heap.alloc_str(s).to_value()).collect();
    starlark::values::list::alloc_immutable_list(heap, &elems)
}

/// Java `JavaStarlarkCommon.checkPrivateAccess`.
// Java: com/google/devtools/build/lib/rules/java/JavaStarlarkCommon.java:291 JavaStarlarkCommon.checkPrivateAccess
fn check_private_access(eval: &Evaluator) -> starlark::Result<()> {
    fail_if_called_outside_default_allowlist(eval)
}

#[starlark_module]
fn java_configuration_methods(builder: &mut MethodsBuilder) {
    /// The default flags for the Java compiler.
    // Java: com/google/devtools/build/lib/rules/java/JavaConfiguration.java:163 JavaConfiguration.getDefaultJavacFlagsForStarlarkAsList
    #[starlark(attribute)]
    fn default_javac_flags<'v>(
        this: &JavaConfigurationValue,
        heap: Heap<'v>,
    ) -> starlark::Result<Value<'v>> {
        Ok(immutable_string_list(
            heap,
            &this
                .fragment()
                .get_default_javac_flags_for_starlark_as_list(),
        ))
    }

    /// The default flags for the Java compiler.
    // Java: com/google/devtools/build/lib/rules/java/JavaConfiguration.java:170 JavaConfiguration.getDefaultJavacFlagsStarlark
    #[starlark(attribute)]
    fn default_javac_flags_depset<'v>(
        this: &JavaConfigurationValue,
    ) -> starlark::Result<Value<'v>> {
        // Java: Depset.of(String.class, commandLineJavacFlags), a NAIVE_LINK_ORDER NestedSet.
        let flags = NestedSetBuilder::wrap(
            Order::NAIVE_LINK_ORDER,
            this.fragment()
                .get_default_javac_flags_starlark()
                .iter()
                .cloned(),
        );
        Ok(depset_value(&strings_to_depset(&flags)))
    }

    /// The value of the strict_java_deps flag.
    // Java: com/google/devtools/build/lib/rules/java/JavaConfiguration.java:175 JavaConfiguration.getStrictJavaDepsName
    #[starlark(attribute)]
    fn strict_java_deps(this: &JavaConfigurationValue) -> starlark::Result<String> {
        Ok(this.fragment().get_strict_java_deps_name())
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaConfiguration.java:195 JavaConfiguration.useHeaderCompilationStarlark
    fn use_header_compilation(
        this: &JavaConfigurationValue,
        eval: &mut Evaluator,
    ) -> starlark::Result<bool> {
        check_private_access(eval)?;
        Ok(this.fragment().use_header_compilation())
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaConfiguration.java:206 JavaConfiguration.getGenerateJavaDepsStarlark
    fn generate_java_deps(
        this: &JavaConfigurationValue,
        eval: &mut Evaluator,
    ) -> starlark::Result<bool> {
        check_private_access(eval)?;
        Ok(this.fragment().get_generate_java_deps())
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaConfiguration.java:216 JavaConfiguration.getReduceJavaClasspathStarlark
    fn reduce_java_classpath(
        this: &JavaConfigurationValue,
        eval: &mut Evaluator,
    ) -> starlark::Result<&'static str> {
        check_private_access(eval)?;
        Ok(this.fragment().get_reduce_java_classpath().name())
    }

    /// Additional options to pass to the Java VM for each java_binary target
    // Java: com/google/devtools/build/lib/rules/java/JavaConfiguration.java:226 JavaConfiguration.getDefaultJvmFlags
    #[starlark(attribute)]
    fn default_jvm_opts<'v>(
        this: &JavaConfigurationValue,
        heap: Heap<'v>,
    ) -> starlark::Result<Value<'v>> {
        Ok(immutable_string_list(
            heap,
            this.fragment().get_default_jvm_flags(),
        ))
    }

    /// The value of the --experimental_one_version_enforcement flag.
    // Java: com/google/devtools/build/lib/rules/java/JavaConfiguration.java:373 JavaConfiguration.starlarkOneVersionEnforcementLevel
    #[starlark(attribute)]
    fn one_version_enforcement_level(
        this: &JavaConfigurationValue,
    ) -> starlark::Result<&'static str> {
        Ok(this.fragment().starlark_one_version_enforcement_level())
    }

    /// The value of the --one_version_enforcement_on_java_tests flag.
    // Java: com/google/devtools/build/lib/rules/java/JavaConfiguration.java:378 JavaConfiguration.enforceOneVersionOnJavaTests
    #[starlark(attribute)]
    fn one_version_enforcement_on_java_tests(
        this: &JavaConfigurationValue,
    ) -> starlark::Result<bool> {
        Ok(this.fragment().enforce_one_version_on_java_tests())
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaConfiguration.java:383 JavaConfiguration.addTestSupportToCompileTimeDeps
    #[starlark(attribute)]
    fn add_test_support_to_compile_deps(this: &JavaConfigurationValue) -> starlark::Result<bool> {
        Ok(this.fragment().add_test_support_to_compile_time_deps())
    }

    /// The value of the --experimental_run_android_lint_on_java_rules flag.
    // Java: com/google/devtools/build/lib/rules/java/JavaConfiguration.java:388 JavaConfiguration.runAndroidLint
    #[starlark(attribute)]
    fn run_android_lint(this: &JavaConfigurationValue) -> starlark::Result<bool> {
        Ok(this.fragment().run_android_lint())
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaConfiguration.java:347 JavaConfiguration.explicitJavaTestDepsStarlark
    fn enforce_explicit_java_test_deps(
        this: &JavaConfigurationValue,
        eval: &mut Evaluator,
    ) -> starlark::Result<bool> {
        check_private_access(eval)?;
        Ok(this.fragment().explicit_java_test_deps())
    }

    /// The value of the --incompatible_multi_release_deploy_jars flag.
    // Java: com/google/devtools/build/lib/rules/java/JavaConfiguration.java:353 JavaConfiguration.multiReleaseDeployJars
    #[starlark(attribute)]
    fn multi_release_deploy_jars(this: &JavaConfigurationValue) -> starlark::Result<bool> {
        Ok(this.fragment().multi_release_deploy_jars())
    }

    /// A list containing the labels provided with --plugins, if any.
    // Java: com/google/devtools/build/lib/rules/java/JavaConfiguration.java:393 JavaConfiguration.getPlugins
    #[starlark(attribute)]
    fn plugins<'v>(this: &JavaConfigurationValue, heap: Heap<'v>) -> starlark::Result<Value<'v>> {
        let elems: Vec<Value<'v>> = this
            .fragment()
            .get_plugins()
            .iter()
            .map(|l| heap.alloc(LabelValue(l.clone())))
            .collect();
        Ok(starlark::values::list::alloc_immutable_list(heap, &elems))
    }

    /// Returns true iff Java compilation should use ijars.
    // Java: com/google/devtools/build/lib/rules/java/JavaConfiguration.java:189 JavaConfiguration.getUseIjarsInStarlark
    fn use_ijars(this: &JavaConfigurationValue, eval: &mut Evaluator) -> starlark::Result<bool> {
        check_private_access(eval)?;
        Ok(this.fragment().get_use_ijars())
    }

    /// Returns true if Java header compilation should use separate outputs for direct deps.
    // Java: com/google/devtools/build/lib/rules/java/JavaConfiguration.java:413 JavaConfiguration.getUseHeaderCompilationDirectDepsInStarlark
    fn use_header_compilation_direct_deps(
        this: &JavaConfigurationValue,
        eval: &mut Evaluator,
    ) -> starlark::Result<bool> {
        check_private_access(eval)?;
        Ok(this.fragment().get_use_header_compilation_direct_deps())
    }

    /// Returns true if java_import exports are not allowed.
    // Java: com/google/devtools/build/lib/rules/java/JavaConfiguration.java:359 JavaConfiguration.getDisallowJavaImportExportsInStarlark
    fn disallow_java_import_exports(
        this: &JavaConfigurationValue,
        eval: &mut Evaluator,
    ) -> starlark::Result<bool> {
        check_private_access(eval)?;
        Ok(this.fragment().get_disallow_java_import_exports())
    }

    /// The mnemonic for the bytecode optimizer.
    // Java: com/google/devtools/build/lib/rules/java/JavaConfiguration.java:300 JavaConfiguration.getBytecodeOptimizerMnemonic
    #[starlark(attribute)]
    fn bytecode_optimizer_mnemonic(this: &JavaConfigurationValue) -> starlark::Result<String> {
        Ok(this.fragment().get_bytecode_optimizer_mnemonic().to_owned())
    }

    /// Returns whether the OPTIMIZATION stage of the bytecode optimizer will be split across
    /// two actions.
    // Java: com/google/devtools/build/lib/rules/java/JavaConfiguration.java:260 JavaConfiguration.splitBytecodeOptimizationPass
    #[starlark(attribute)]
    fn split_bytecode_optimization_pass(this: &JavaConfigurationValue) -> starlark::Result<bool> {
        Ok(this.fragment().split_bytecode_optimization_pass())
    }

    /// This specifies the number of actions to divide the OPTIMIZATION stage of the bytecode
    /// optimizer into.
    // Java: com/google/devtools/build/lib/rules/java/JavaConfiguration.java:270 JavaConfiguration.bytecodeOptimizationPassActions
    #[starlark(attribute)]
    fn bytecode_optimization_pass_actions(this: &JavaConfigurationValue) -> starlark::Result<i32> {
        Ok(this.fragment().bytecode_optimization_pass_actions())
    }

    /// Returns whether ProGuard configuration files are required to use a *.pgcfg extension.
    // Java: com/google/devtools/build/lib/rules/java/JavaConfiguration.java:276 JavaConfiguration.enforceProguardFileExtension
    #[starlark(attribute)]
    fn enforce_proguard_file_extension(this: &JavaConfigurationValue) -> starlark::Result<bool> {
        Ok(this.fragment().enforce_proguard_file_extension())
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaConfiguration.java:406 JavaConfiguration.autoCreateJavaTestDeployJars
    fn auto_create_java_test_deploy_jars(
        this: &JavaConfigurationValue,
        eval: &mut Evaluator,
    ) -> starlark::Result<bool> {
        fail_if_called_outside_default_allowlist(eval)?;
        Ok(this.fragment().auto_create_java_test_deploy_jars())
    }
}
