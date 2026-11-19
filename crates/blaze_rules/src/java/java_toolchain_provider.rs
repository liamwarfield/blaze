//! `rules/java/JavaToolchainProvider.java`: information about the JDK used by the `java_*`
//! rules (a wrapped instance of rules_java's `JavaToolchainInfo`).

use std::sync::Arc;

use blaze_actions::Artifact;
use blaze_analysis::providers::file_provider::FilesToRunProvider;
use blaze_analysis::providers::package_specification_provider::PackageSpecificationProvider;
use blaze_analysis::rule::RuleContext;
use blaze_analysis::rule::rule_error_consumer::RuleErrorConsumer;
use blaze_analysis::starlark::starlark_action_factory::files_to_run_provider_from_value;
use blaze_cmdline::Label;
use blaze_collect::NestedSet;
use blaze_packages::rule_class::ProviderKey;
use blaze_starlark::label::LabelValue;
use indexmap::IndexSet;
use starlark::values::{Heap, Value, ValueLike};

use super::boot_class_path_info::BootClassPathInfo;
use super::java_package_configuration_provider::JavaPackageConfigurationProvider;
use super::java_plugin_info::JavaPluginData;
use super::java_runtime_info::JavaRuntimeInfo;
use super::java_toolchain_tool::JavaToolchainTool;
use super::provider_util::{
    info_key, is_starlark_info_with_schema, is_struct_impl, main_repo_provider_key,
    rules_java_provider_key, starlark_type,
};
use super::starlark_info_wrapper::{StarlarkInfoWrapper, struct_get_value};
use crate::cpp::util::errorf;

const BZL: &str = "java/common/rules:java_toolchain.bzl";

/// `JavaToolchainProvider.PROVIDER.getKey()`.
// Java: com/google/devtools/build/lib/rules/java/JavaToolchainProvider.java:323 JavaToolchainProvider.Provider.<init>
pub fn provider_key() -> ProviderKey {
    rules_java_provider_key(BZL, "JavaToolchainInfo")
}

/// `JavaToolchainProvider.RULES_JAVA_PROVIDER.getKey()`.
// Java: com/google/devtools/build/lib/rules/java/JavaToolchainProvider.java:316 JavaToolchainProvider.RulesJavaProvider.<init>
pub fn rules_java_provider_key_() -> ProviderKey {
    main_repo_provider_key(BZL, "JavaToolchainInfo")
}

/// Information about the JDK used by the `java_*` rules.
// Java: com/google/devtools/build/lib/rules/java/JavaToolchainProvider.java:52 JavaToolchainProvider
#[derive(Clone, Copy)]
pub struct JavaToolchainProvider<'v> {
    wrapper: StarlarkInfoWrapper<'v>,
}

impl<'v> JavaToolchainProvider<'v> {
    // Java: com/google/devtools/build/lib/rules/java/JavaToolchainProvider.java:63 JavaToolchainProvider.wrap
    pub fn wrap(info: Value<'v>, heap: Heap<'v>) -> starlark::Result<JavaToolchainProvider<'v>> {
        let key = info_key(info);
        if key == Some(provider_key()) || key == Some(rules_java_provider_key_()) {
            // Java: com/google/devtools/build/lib/rules/java/JavaToolchainProvider.java:332 JavaToolchainProvider.Provider.wrap
            if is_starlark_info_with_schema(info) {
                return Ok(JavaToolchainProvider {
                    wrapper: StarlarkInfoWrapper::new(info, heap),
                });
            }
            return Err(errorf(format!(
                "got value of type '{}', want 'JavaToolchainInfo'",
                starlark_type(info)
            )));
        }
        Err(errorf(format!(
            "expected JavaToolchainInfo, got: {}",
            key.map(|k| k.to_string())
                .unwrap_or_else(|| "null".to_owned())
        )))
    }

    /// Returns the Java Toolchain associated with the rule being analyzed or `None`.
    // Java: com/google/devtools/build/lib/rules/java/JavaToolchainProvider.java:96 JavaToolchainProvider.from
    pub fn from_rule_context(
        rule_context: &RuleContext,
        heap: Heap<'v>,
        toolchain_info: Option<Value<'v>>,
    ) -> Option<JavaToolchainProvider<'v>> {
        Self::from(toolchain_info, Some(rule_context), heap)
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaToolchainProvider.java:110 JavaToolchainProvider.from
    fn from(
        toolchain_info: Option<Value<'v>>,
        error_consumer: Option<&dyn RuleErrorConsumer>,
        heap: Heap<'v>,
    ) -> Option<JavaToolchainProvider<'v>> {
        if let Some(toolchain_info) = toolchain_info {
            let java = struct_get_value(toolchain_info, "java", heap);
            let result = match java {
                Some(v) if !v.is_none() => Self::wrap(v, heap),
                // Java: wrap(null) throws a NullPointerException from info.getProvider().
                _ => panic!("NullPointerException: ToolchainInfo.java"),
            };
            match result {
                Ok(provider) => return Some(provider),
                Err(e) => {
                    if let Some(c) = error_consumer {
                        c.rule_error(&format!(
                            "There was an error reading the Java toolchain: {}",
                            e
                        ));
                    }
                }
            }
        }
        if let Some(c) = error_consumer {
            c.rule_error("The selected Java toolchain is not a JavaToolchainProvider");
        }
        None
    }

    /// Returns the label for this `java_toolchain`.
    // Java: com/google/devtools/build/lib/rules/java/JavaToolchainProvider.java:134 JavaToolchainProvider.getToolchainLabel
    pub fn get_toolchain_label(&self) -> starlark::Result<Label> {
        match self.wrapper.get_underlying_value("label") {
            None => panic!("NullPointerException"),
            Some(v) => match v.downcast_ref::<LabelValue>() {
                Some(l) => Ok(l.label().clone()),
                None => Err(StarlarkInfoWrapper::cast_error("label", v, "Label")),
            },
        }
    }

    /// Returns the target Java bootclasspath.
    // Java: com/google/devtools/build/lib/rules/java/JavaToolchainProvider.java:139 JavaToolchainProvider.getBootclasspath
    pub fn get_bootclasspath(&self) -> starlark::Result<BootClassPathInfo<'v>> {
        match self.wrapper.get_underlying_value("_bootclasspath_info") {
            // Java: BootClassPathInfo.wrap((StructImpl) null) wraps null; reading it fails.
            None => panic!("NullPointerException: _bootclasspath_info"),
            Some(v) => Ok(BootClassPathInfo::wrap(v, self.wrapper.heap)),
        }
    }

    /// Returns the Artifacts of compilation tools.
    // Java: com/google/devtools/build/lib/rules/java/JavaToolchainProvider.java:144 JavaToolchainProvider.getTools
    pub fn get_tools(&self) -> starlark::Result<NestedSet<Artifact>> {
        self.wrapper.get_underlying_nested_set_artifacts("tools")
    }

    fn get_struct(&self, key: &str) -> starlark::Result<Option<Value<'v>>> {
        match self.wrapper.get_underlying_value(key) {
            None => Ok(None),
            Some(v) if is_struct_impl(v) => Ok(Some(v)),
            Some(v) => Err(StarlarkInfoWrapper::cast_error(key, v, "struct")),
        }
    }

    /// Returns the JavaToolchainTool for JavaBuilder.
    // Java: com/google/devtools/build/lib/rules/java/JavaToolchainProvider.java:149 JavaToolchainProvider.getJavaBuilder
    pub fn get_java_builder(&self) -> starlark::Result<Option<JavaToolchainTool>> {
        JavaToolchainTool::from_starlark(self.get_struct("_javabuilder")?, self.wrapper.heap)
    }

    /// Returns the JavaToolchainTool for the header compiler.
    // Java: com/google/devtools/build/lib/rules/java/JavaToolchainProvider.java:155 JavaToolchainProvider.getHeaderCompiler
    pub fn get_header_compiler(&self) -> starlark::Result<Option<JavaToolchainTool>> {
        JavaToolchainTool::from_starlark(self.get_struct("_header_compiler")?, self.wrapper.heap)
    }

    /// Returns the header compiler for direct-classpath, non-annotation processing actions.
    // Java: com/google/devtools/build/lib/rules/java/JavaToolchainProvider.java:164 JavaToolchainProvider.getHeaderCompilerDirect
    pub fn get_header_compiler_direct(&self) -> starlark::Result<Option<JavaToolchainTool>> {
        JavaToolchainTool::from_starlark(
            self.get_struct("_header_compiler_direct")?,
            self.wrapper.heap,
        )
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaToolchainProvider.java:171 JavaToolchainProvider.getAndroidLint
    pub fn get_android_lint(&self) -> starlark::Result<Option<Value<'v>>> {
        self.get_struct("_android_linter")
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaToolchainProvider.java:176 JavaToolchainProvider.jspecifyInfo
    pub fn jspecify_info(&self) -> starlark::Result<Option<JspecifyInfo>> {
        JspecifyInfo::from_starlark(
            self.wrapper.get_underlying_value("_jspecify_info"),
            self.wrapper.heap,
        )
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaToolchainProvider.java:181 JavaToolchainProvider.getBytecodeOptimizer
    pub fn get_bytecode_optimizer(&self) -> starlark::Result<Option<JavaToolchainTool>> {
        JavaToolchainTool::from_starlark(self.get_struct("_bytecode_optimizer")?, self.wrapper.heap)
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaToolchainProvider.java:186 JavaToolchainProvider.getLocalJavaOptimizationConfiguration
    pub fn get_local_java_optimization_configuration(&self) -> starlark::Result<Vec<Artifact>> {
        self.wrapper
            .get_underlying_sequence_artifacts("_local_java_optimization_config")
    }

    /// Returns class names of annotation processors that are built in to the header compiler.
    // Java: com/google/devtools/build/lib/rules/java/JavaToolchainProvider.java:192 JavaToolchainProvider.getHeaderCompilerBuiltinProcessors
    pub fn get_header_compiler_builtin_processors(&self) -> starlark::Result<IndexSet<String>> {
        Ok(self
            .wrapper
            .get_underlying_nested_set_strings("_header_compiler_builtin_processors")?
            .to_set())
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaToolchainProvider.java:196 JavaToolchainProvider.getReducedClasspathIncompatibleProcessors
    pub fn get_reduced_classpath_incompatible_processors(
        &self,
    ) -> starlark::Result<IndexSet<String>> {
        Ok(self
            .wrapper
            .get_underlying_nested_set_strings("_reduced_classpath_incompatible_processors")?
            .to_set())
    }

    fn get_files_to_run(&self, key: &str) -> starlark::Result<Option<Arc<FilesToRunProvider>>> {
        match self.wrapper.get_underlying_value(key) {
            None => Ok(None),
            Some(v) => match files_to_run_provider_from_value(v) {
                Some(f) => Ok(Some(f)),
                None => Err(StarlarkInfoWrapper::cast_error(
                    key,
                    v,
                    "FilesToRunProvider",
                )),
            },
        }
    }

    /// Returns the FilesToRunProvider of the SingleJar tool.
    // Java: com/google/devtools/build/lib/rules/java/JavaToolchainProvider.java:203 JavaToolchainProvider.getSingleJar
    pub fn get_single_jar(&self) -> starlark::Result<Option<Arc<FilesToRunProvider>>> {
        self.get_files_to_run("single_jar")
    }

    /// Returns the Artifact of the GenClass deploy jar.
    // Java: com/google/devtools/build/lib/rules/java/JavaToolchainProvider.java:208 JavaToolchainProvider.getGenClass
    pub fn get_gen_class(&self) -> starlark::Result<Option<Artifact>> {
        self.wrapper.get_underlying_artifact("_gen_class")
    }

    /// Returns the Artifact of the latest timezone data resource jar that can be loaded by
    /// Java 8 binaries.
    // Java: com/google/devtools/build/lib/rules/java/JavaToolchainProvider.java:218 JavaToolchainProvider.getTimezoneData
    pub fn get_timezone_data(&self) -> starlark::Result<Option<Artifact>> {
        self.wrapper.get_underlying_artifact("_timezone_data")
    }

    /// Returns the ijar executable.
    // Java: com/google/devtools/build/lib/rules/java/JavaToolchainProvider.java:223 JavaToolchainProvider.getIjar
    pub fn get_ijar(&self) -> starlark::Result<Option<Arc<FilesToRunProvider>>> {
        self.get_files_to_run("ijar")
    }

    /// Returns the NestedSet of default options for the JVM running the java compiler and
    /// associated tools.
    // Java: com/google/devtools/build/lib/rules/java/JavaToolchainProvider.java:231 JavaToolchainProvider.getJvmOptions
    pub fn get_jvm_options(&self) -> starlark::Result<NestedSet<String>> {
        self.wrapper.get_underlying_nested_set_strings("jvm_opt")
    }

    fn get_bool(&self, key: &str) -> starlark::Result<bool> {
        // Java: unboxing a null Boolean is a NullPointerException.
        Ok(self
            .wrapper
            .get_underlying_bool(key)?
            .unwrap_or_else(|| panic!("NullPointerException: {key}")))
    }

    /// Returns whether JavaBuilders supports running as a persistent worker or not.
    // Java: com/google/devtools/build/lib/rules/java/JavaToolchainProvider.java:236 JavaToolchainProvider.getJavacSupportsWorkers
    pub fn get_javac_supports_workers(&self) -> starlark::Result<bool> {
        self.get_bool("_javac_supports_workers")
    }

    /// Returns whether JavaBuilders supports running persistent workers in multiplex mode.
    // Java: com/google/devtools/build/lib/rules/java/JavaToolchainProvider.java:241 JavaToolchainProvider.getJavacSupportsMultiplexWorkers
    pub fn get_javac_supports_multiplex_workers(&self) -> starlark::Result<bool> {
        self.get_bool("_javac_supports_multiplex_workers")
    }

    /// Returns whether JavaBuilders supports running persistent workers with cancellation.
    // Java: com/google/devtools/build/lib/rules/java/JavaToolchainProvider.java:246 JavaToolchainProvider.getJavacSupportsWorkerCancellation
    pub fn get_javac_supports_worker_cancellation(&self) -> starlark::Result<bool> {
        self.get_bool("_javac_supports_worker_cancellation")
    }

    /// Returns whether JavaBuilders supports running multiplex persistent workers in sandbox
    /// mode.
    // Java: com/google/devtools/build/lib/rules/java/JavaToolchainProvider.java:251 JavaToolchainProvider.getJavacSupportsWorkerMultiplexSandboxing
    pub fn get_javac_supports_worker_multiplex_sandboxing(&self) -> starlark::Result<bool> {
        self.get_bool("_javac_supports_worker_multiplex_sandboxing")
    }

    /// Returns the global `java_package_configuration` data.
    // Java: com/google/devtools/build/lib/rules/java/JavaToolchainProvider.java:256 JavaToolchainProvider.packageConfiguration
    pub fn package_configuration(
        &self,
    ) -> starlark::Result<Vec<JavaPackageConfigurationProvider<'v>>> {
        let seq = self
            .wrapper
            .get_underlying_sequence("_package_configuration")?;
        JavaPackageConfigurationProvider::wrap_sequence(&seq, self.wrapper.heap)
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaToolchainProvider.java:262 JavaToolchainProvider.getJacocoRunner
    pub fn get_jacoco_runner(&self) -> starlark::Result<Option<Arc<FilesToRunProvider>>> {
        self.get_files_to_run("jacocorunner")
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaToolchainProvider.java:266 JavaToolchainProvider.getJavaRuntime
    pub fn get_java_runtime(&self) -> starlark::Result<JavaRuntimeInfo<'v>> {
        match self.wrapper.get_underlying_value("java_runtime") {
            None => panic!("NullPointerException: java_runtime"),
            Some(v) => JavaRuntimeInfo::wrap(v, self.wrapper.heap),
        }
    }
}

/// JSpecify configuration of the toolchain.
// Java: com/google/devtools/build/lib/rules/java/JavaToolchainProvider.java:270 JavaToolchainProvider.JspecifyInfo
#[derive(Clone)]
pub struct JspecifyInfo {
    pub jspecify_processor: JavaPluginData,
    pub jspecify_implicit_deps: NestedSet<Artifact>,
    pub jspecify_javacopts: Vec<String>,
    pub jspecify_packages: Vec<PackageSpecificationProvider>,
}

impl JspecifyInfo {
    // Java: com/google/devtools/build/lib/rules/java/JavaToolchainProvider.java:283 JavaToolchainProvider.JspecifyInfo.matches
    pub fn matches(&self, label: &Label) -> bool {
        for provider in &self.jspecify_packages {
            for specifications in provider.get_package_specifications().to_list().iter() {
                if specifications
                    .0
                    .contains_package(label.get_package_identifier())
                {
                    return true;
                }
            }
        }
        false
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaToolchainProvider.java:294 JavaToolchainProvider.JspecifyInfo.fromStarlark
    fn from_starlark<'v>(
        value: Option<Value<'v>>,
        heap: Heap<'v>,
    ) -> starlark::Result<Option<JspecifyInfo>> {
        let Some(value) = value else {
            return Ok(None);
        };
        if value.is_none() {
            return Ok(None);
        }
        if !is_struct_impl(value) {
            return Err(errorf(format!(
                "expected JspecifyInfo, got: {}",
                starlark_type(value)
            )));
        }
        let get = |k: &str| struct_get_value(value, k, heap).unwrap_or_else(Value::new_none);
        let jspecify_processor =
            JavaPluginData::wrap(struct_get_value(value, "processor", heap), heap)?;
        let implicit_deps = get("implicit_deps");
        let jspecify_implicit_deps = if implicit_deps.is_none() {
            blaze_collect::NestedSetBuilder::empty_set(blaze_collect::Order::STABLE_ORDER)
        } else {
            blaze_analysis::rule::depsets::depset_to_artifacts(implicit_deps, "implicit_deps")?
        };
        let javacopts = get("javacopts");
        let jspecify_javacopts = if javacopts.is_none() {
            Vec::new()
        } else {
            crate::cpp::util::sequence_cast_strings(javacopts, "javacopts", heap)?
        };
        let packages = get("packages");
        let mut jspecify_packages = Vec::new();
        if !packages.is_none() {
            if !crate::cpp::util::is_sequence(packages) {
                return Err(errorf(format!(
                    "for packages, got {}, want sequence",
                    packages.get_type()
                )));
            }
            for (i, e) in crate::cpp::util::sequence_elements(packages, heap)?
                .into_iter()
                .enumerate()
            {
                match e.downcast_ref::<PackageSpecificationProvider>() {
                    Some(p) => jspecify_packages.push(p.clone()),
                    None => {
                        return Err(errorf(format!(
                            "at index {i} of packages, got element of type {}, want PackageSpecificationInfo",
                            e.get_type()
                        )));
                    }
                }
            }
        }
        Ok(Some(JspecifyInfo {
            jspecify_processor,
            jspecify_implicit_deps,
            jspecify_javacopts,
            jspecify_packages,
        }))
    }
}
