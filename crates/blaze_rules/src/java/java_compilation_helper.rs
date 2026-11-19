//! `rules/java/JavaCompilationHelper.java`: a helper class for compiling Java targets. It
//! contains methods to create the various intermediate Artifacts for using ijars and source
//! ijars. Also supports the creation of resource and source only Jars.

use std::sync::Arc;

use blaze_actions::ActionRef;
use blaze_actions::Artifact;
use blaze_actions::actions::command_line::CommandLineRef;
use blaze_actions::actions::param_file_info::ParamFileInfo;
use blaze_actions::actions::parameter_file::ParameterFileType;
use blaze_actions::analysis::actions::custom_command_line::CustomCommandLine;
use blaze_actions::analysis::actions::lazy_write_paths_file_action::LazyWritePathsFileAction;
use blaze_actions::analysis::actions::spawn_action::SpawnActionBuilder;
use blaze_analysis::providers::file_provider::FilesToRunProvider;
use blaze_analysis::rule::RuleContext;
use blaze_analysis::rule::config_util::{self, ConfigurationSpawnActionConfiguration};
use blaze_analysis::test::instrumented_files_collector::should_include_local_sources;
use blaze_build_config::fragments::java_configuration::JavaConfiguration;
use blaze_collect::{NestedSet, NestedSetBuilder, Order};
use blaze_packages::rule_class::declared_exec_group::DEFAULT_EXEC_GROUP_NAME;
use blaze_packages::target::target_utils;
use indexmap::{IndexMap, IndexSet};
use starlark::eval::Evaluator;

use super::boot_class_path_info::BootClassPathInfo;
use super::java_compile_action::modified_execution_info;
use super::java_compile_action_builder::{self, JavaCompileActionBuilder};
use super::java_header_compile_action::{JavaHeaderCompileAction, JavaHeaderCompileActionBuilder};
use super::java_output::JavaCompileOutputs;
use super::java_plugin_info::JavaPluginData;
use super::java_runtime_info::JavaRuntimeInfo;
use super::java_semantics::{JavaSemantics, append_without_extension, replace_extension};
use super::java_target_attributes::{JavaTargetAttributes, JavaTargetAttributesBuilder};
use super::java_toolchain_provider::JavaToolchainProvider;
use super::resource_jar_action_builder::ResourceJarActionBuilder;
use crate::cpp::util::errorf;

// Java: com/google/devtools/build/lib/actions/ExecutionRequirements.java:403 ExecutionRequirements.SUPPORTS_PATH_MAPPING
const SUPPORTS_PATH_MAPPING: &str = "supports-path-mapping";
// Java: com/google/devtools/build/lib/actions/ExecutionRequirements.java:142 ExecutionRequirements.SUPPORTS_WORKERS
const SUPPORTS_WORKERS: &str = "supports-workers";
// Java: com/google/devtools/build/lib/actions/ExecutionRequirements.java:145 ExecutionRequirements.SUPPORTS_MULTIPLEX_WORKERS
const SUPPORTS_MULTIPLEX_WORKERS: &str = "supports-multiplex-workers";
// Java: com/google/devtools/build/lib/actions/ExecutionRequirements.java:151 ExecutionRequirements.SUPPORTS_WORKER_CANCELLATION
const SUPPORTS_WORKER_CANCELLATION: &str = "supports-worker-cancellation";
// Java: com/google/devtools/build/lib/actions/ExecutionRequirements.java:148 ExecutionRequirements.SUPPORTS_MULTIPLEX_SANDBOXING
const SUPPORTS_MULTIPLEX_SANDBOXING: &str = "supports-multiplex-sandboxing";

/// `ruleContext.registerAction(action)` from a native called by a rule implementation.
pub fn register_action(eval: &Evaluator, action: ActionRef) -> starlark::Result<()> {
    blaze_analysis::starlark::thread_payload::register_action(eval, action)
}

/// A helper class for compiling Java targets.
///
/// PORT: Java's interners of javacopts and execution info only share memory.
// Java: com/google/devtools/build/lib/rules/java/JavaCompilationHelper.java:66 JavaCompilationHelper
pub struct JavaCompilationHelper<'a, 'v> {
    rule_context: &'a RuleContext,
    java_toolchain: JavaToolchainProvider<'v>,
    attributes: JavaTargetAttributesBuilder<'v>,
    built_attributes: Option<JavaTargetAttributes<'v>>,
    custom_javac_opts: Vec<String>,
    java_builder_jvm_flags: NestedSet<String>,
    semantics: &'static dyn JavaSemantics,
    additional_inputs_for_databinding: Vec<Artifact>,
    enable_jspecify: bool,
    enable_direct_classpath: bool,
    extra_command_line_args: Vec<CommandLineRef>,
    exec_group: String,
}

impl<'a, 'v> JavaCompilationHelper<'a, 'v> {
    // Java: com/google/devtools/build/lib/rules/java/JavaCompilationHelper.java:87 JavaCompilationHelper.<init>
    pub fn new(
        rule_context: &'a RuleContext,
        semantics: &'static dyn JavaSemantics,
        javac_opts: Vec<String>,
        attributes: JavaTargetAttributesBuilder<'v>,
        java_toolchain_provider: JavaToolchainProvider<'v>,
        additional_inputs_for_databinding: Vec<Artifact>,
    ) -> Self {
        let exec_group = if rule_context.use_auto_exec_groups() {
            semantics.get_java_toolchain_type()
        } else {
            DEFAULT_EXEC_GROUP_NAME.to_owned()
        };
        JavaCompilationHelper {
            rule_context,
            java_toolchain: java_toolchain_provider,
            attributes,
            built_attributes: None,
            custom_javac_opts: javac_opts,
            java_builder_jvm_flags: NestedSetBuilder::empty_set(Order::STABLE_ORDER),
            semantics,
            additional_inputs_for_databinding,
            enable_jspecify: true,
            enable_direct_classpath: true,
            extra_command_line_args: Vec::new(),
            exec_group,
        }
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaCompilationHelper.java:107 JavaCompilationHelper.javaBuilderJvmFlags
    pub fn java_builder_jvm_flags(&mut self, java_builder_jvm_flags: NestedSet<String>) {
        self.java_builder_jvm_flags = java_builder_jvm_flags;
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaCompilationHelper.java:111 JavaCompilationHelper.enableJspecify
    pub fn enable_jspecify(&mut self, enable_jspecify: bool) {
        self.enable_jspecify = enable_jspecify;
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaCompilationHelper.java:115 JavaCompilationHelper.setExtraCommandLineArgs
    pub fn set_extra_command_line_args(&mut self, extra_command_line_args: Vec<CommandLineRef>) {
        self.extra_command_line_args = extra_command_line_args;
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaCompilationHelper.java:119 JavaCompilationHelper.getAttributes
    fn get_attributes(&mut self) -> JavaTargetAttributes<'v> {
        if self.built_attributes.is_none() {
            self.built_attributes = Some(self.attributes.build());
        }
        self.built_attributes.clone().unwrap()
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaCompilationHelper.java:126 JavaCompilationHelper.enableDirectClasspath
    pub fn enable_direct_classpath(&mut self, enable_direct_classpath: bool) {
        self.enable_direct_classpath = enable_direct_classpath;
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaCompilationHelper.java:130 JavaCompilationHelper.getRuleContext
    pub fn get_rule_context(&self) -> &'a RuleContext {
        self.rule_context
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaCompilationHelper.java:142 JavaCompilationHelper.getJavaConfiguration
    fn get_java_configuration(&self) -> &'a JavaConfiguration {
        self.rule_context
            .get_fragment::<JavaConfiguration>("JavaConfiguration", Some("java"))
            .unwrap_or_else(|| panic!("NullPointerException: JavaConfiguration fragment"))
    }

    fn is_sibling_repository_layout(&self) -> bool {
        config_util::is_sibling_repository_layout(self.rule_context.get_configuration())
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaCompilationHelper.java:146 JavaCompilationHelper.createCompileAction
    pub fn create_compile_action(
        &mut self,
        eval: &Evaluator,
        mut outputs: JavaCompileOutputs,
    ) -> starlark::Result<()> {
        if let Some(gen_class) = outputs.gen_class().cloned() {
            self.create_gen_jar_action(
                eval,
                outputs.output(),
                outputs
                    .manifest_proto()
                    .unwrap_or_else(|| panic!("NullPointerException: manifestProto")),
                &gen_class,
                &self.java_toolchain.get_java_runtime()?,
            )?;
        }

        let mut attributes = self.get_attributes();

        let jspecify_info = self.java_toolchain.jspecify_info()?;
        let jspecify = self.enable_jspecify
            && self.get_java_configuration().experimental_enable_jspecify()
            && jspecify_info
                .as_ref()
                .is_some_and(|j| j.matches(self.rule_context.get_label()));
        if jspecify {
            // JSpecify requires these on the compile-time classpath; see b/187113128
            // Add them as non-direct deps (for the purposes of Strict Java Deps) to still
            // require an explicit dep if they're directly used by the compiled source.
            attributes = attributes.append_additional_transitive_class_path_entries(
                &jspecify_info.as_ref().unwrap().jspecify_implicit_deps,
            );
        }

        let mut source_jars: Vec<Artifact> = attributes.get_source_jars().to_vec();
        let mut plugins: JavaPluginData = attributes.plugins().plugins().clone();
        let mut resource_jars: Vec<Artifact> = Vec::new();

        let turbine_annotation_processing = self.uses_annotation_processing()
            && self
                .get_java_configuration()
                .experimental_turbine_annotation_processing();
        if turbine_annotation_processing {
            let turbine_resources = self.turbine_output(outputs.output(), "-turbine-resources.jar");
            resource_jars.push(turbine_resources.clone());
            let output_jar = self.turbine_output(outputs.output(), "-turbine-apt.jar");
            let turbine_jdeps = self.turbine_output(outputs.output(), "-turbine-apt.jdeps");
            let turbine_gensrc = match outputs.gen_source() {
                Some(g) => g.clone(),
                None => self.turbine_output(outputs.output(), "-turbine-apt-gensrc.jar"),
            };

            let mut builder = self.get_java_header_compile_action_builder()?;
            builder.set_output_jar(output_jar);
            builder.set_output_deps_proto(turbine_jdeps);
            builder.set_plugins(plugins.clone());
            builder.set_resource_output_jar(Some(turbine_resources));
            builder.set_gensrc_output_jar(Some(turbine_gensrc.clone()));
            builder.set_manifest_output(outputs.manifest_proto().cloned());
            builder.set_additional_outputs(attributes.get_additional_outputs().clone());
            // TODO(cushon): GraalVM/native-image doesn't support service-loading for Dagger SPI
            // plugins
            builder.enable_header_compiler_direct(false);
            builder.build(eval, &self.java_toolchain)?;

            // The sources generated by the turbine annotation processing action are added to the
            // list of source jars passed to JavaBuilder.
            source_jars.push(turbine_gensrc);
        }

        if self.separate_resource_jar(&resource_jars, &attributes) {
            let original_output = outputs.output().clone();
            outputs = outputs.with_output(
                self.rule_context.get_derived_artifact(
                    &append_without_extension(
                        &outputs
                            .output()
                            .get_output_dir_relative_path(self.is_sibling_repository_layout()),
                        "-class",
                    ),
                    outputs.output().get_root(),
                ),
            );
            resource_jars.push(outputs.output().clone());
            self.create_resource_jar_action(eval, &original_output, resource_jars)?;
        }

        let mut optimized_jar: Option<Artifact> = None;
        if self.get_java_configuration().run_local_java_optimizations() {
            optimized_jar = Some(outputs.output().clone());
            outputs = outputs.with_output(
                self.rule_context.get_derived_artifact(
                    &replace_extension(
                        &outputs
                            .output()
                            .get_output_dir_relative_path(self.is_sibling_repository_layout()),
                        "-pre-optimization.jar",
                    ),
                    outputs.output().get_root(),
                ),
            );
        }

        let mut javacopts = self.custom_javac_opts.clone();
        if jspecify {
            let jspecify_info = jspecify_info.as_ref().unwrap();
            plugins = JavaPluginData::merge([&plugins, &jspecify_info.jspecify_processor]);
            // Add JSpecify options last to discourage overriding them, at least for now.
            javacopts.extend(jspecify_info.jspecify_javacopts.iter().cloned());
        }

        let mut builder = JavaCompileActionBuilder::new(
            self.rule_context,
            self.java_toolchain,
            self.exec_group.clone(),
        );

        let classpath_mode = self.get_java_configuration().get_reduce_java_classpath();
        builder.set_classpath_mode(classpath_mode);
        builder.set_additional_inputs(self.additional_inputs_for_databinding.clone());
        let label = self.rule_context.get_label().clone();
        builder.set_target_label(Some(label.clone()));
        let coverage_artifact = self.maybe_create_coverage_artifact(outputs.output());
        builder.set_coverage_artifact(coverage_artifact.clone());
        let boot_class_path_info = self.get_bootclasspath_or_default()?;
        builder.set_boot_class_path(boot_class_path_info);
        let mut classpath_builder = NestedSetBuilder::naive_link_order();
        classpath_builder
            .add_transitive(&boot_class_path_info.auxiliary()?)
            .add_transitive(attributes.get_compile_time_class_path());
        let classpath = classpath_builder.build();
        if !boot_class_path_info.auxiliary()?.is_empty() {
            builder.set_classpath_entries(classpath.clone());
            let mut direct = NestedSetBuilder::naive_link_order();
            direct
                .add_transitive(&boot_class_path_info.auxiliary()?)
                .add_transitive(attributes.get_direct_jars());
            builder.set_direct_jars(direct.build());
        } else {
            builder.set_classpath_entries(attributes.get_compile_time_class_path().clone());
            builder.set_direct_jars(attributes.get_direct_jars().clone());
        }
        builder.set_source_path_entries(attributes.get_source_path().to_vec());
        builder.set_tools_jars(self.java_toolchain.get_tools()?);
        builder.set_java_builder(
            self.java_toolchain
                .get_java_builder()?
                .unwrap_or_else(|| panic!("NullPointerException: _javabuilder"))
                .with_additional_jvm_flags(&self.java_builder_jvm_flags),
        );
        if !turbine_annotation_processing {
            builder.set_gen_source_output(outputs.gen_source().cloned());
            builder.set_additional_outputs(attributes.get_additional_outputs().clone());
            builder.set_plugins(plugins.clone());
            builder.set_manifest_output(outputs.manifest_proto().cloned());
        } else {
            // Don't do annotation processing, but pass the processorpath through to allow
            // service-loading Error Prone plugins.
            builder.set_plugins(JavaPluginData::create(
                /* processor_classes= */ NestedSetBuilder::empty_set(Order::STABLE_ORDER),
                plugins.processor_classpath().clone(),
                plugins.data().clone(),
            ));
        }
        builder.set_outputs(outputs.clone());

        let source_files: IndexSet<Artifact> = attributes.get_source_files().clone();
        builder.set_source_files(source_files.clone());
        builder.set_source_jars(source_jars);
        builder.set_javac_opts(javacopts);
        builder.set_utf8_environment(
            self.semantics
                .utf8_environment(self.rule_context.get_execution_platform().as_deref()),
        );
        builder.set_javac_execution_info(self.get_execution_info()?);
        builder.set_compress_jar(true);
        builder.set_extra_data(Self::compute_per_package_data(
            self.rule_context,
            &self.java_toolchain,
        )?);
        builder.set_strict_java_deps(attributes.get_strict_java_deps());
        builder.set_fix_deps_tool(self.get_java_configuration().get_fix_deps_tool().to_owned());
        builder.set_compile_time_dependency_artifacts(
            attributes.get_compile_time_dependency_artifacts().clone(),
        );
        builder.set_extra_command_line_args(self.extra_command_line_args.clone());
        builder.set_target_label(Some(
            attributes
                .get_target_label()
                .cloned()
                .unwrap_or_else(|| label.clone()),
        ));
        builder.set_injecting_rule_kind(attributes.get_injecting_rule_kind().map(str::to_owned));

        if let Some(coverage_artifact) = &coverage_artifact {
            let mut files = NestedSetBuilder::stable_order();
            files.add_all(source_files.iter().cloned());
            register_action(
                eval,
                Arc::new(LazyWritePathsFileAction::new(
                    self.action_owner()?,
                    coverage_artifact.clone(),
                    files.build(),
                    /* files_to_ignore= */ IndexSet::new(),
                    false,
                )),
            )?;
        }

        let java_compile_action = builder.build()?;
        register_action(eval, Arc::new(java_compile_action))?;

        if let Some(optimized_jar) = optimized_jar {
            let optimizer_label = self.get_java_configuration().get_bytecode_optimizer();
            let mut cp = NestedSetBuilder::naive_link_order();
            cp.add_transitive(&boot_class_path_info.bootclasspath()?)
                .add_transitive(&classpath);
            self.create_local_optimization_action(
                eval,
                outputs.output(),
                &optimized_jar,
                &cp.build(),
                &self
                    .java_toolchain
                    .get_local_java_optimization_configuration()?,
                self.java_toolchain
                    .get_bytecode_optimizer()?
                    .unwrap_or_else(|| panic!("NullPointerException: _bytecode_optimizer"))
                    .tool(),
                optimizer_label.name(),
            )?;
        }
        Ok(())
    }

    fn action_owner(&self) -> starlark::Result<blaze_actions::actions::action_owner::ActionOwner> {
        self.rule_context
            .get_action_owner_for(&self.exec_group)
            .ok_or_else(|| {
                errorf(format!(
                    "no action owner for exec group {}",
                    self.exec_group
                ))
            })
    }

    /// Returns the per-package configured runfiles.
    // Java: com/google/devtools/build/lib/rules/java/JavaCompilationHelper.java:327 JavaCompilationHelper.computePerPackageData
    fn compute_per_package_data(
        rule_context: &RuleContext,
        toolchain: &JavaToolchainProvider<'_>,
    ) -> starlark::Result<NestedSet<Artifact>> {
        // Do not use streams here as they create excessive garbage.
        let mut data = NestedSetBuilder::naive_link_order();
        for provider in toolchain.package_configuration()? {
            if provider.matches(rule_context.get_label())? {
                data.add_transitive(&provider.data()?);
            }
        }
        Ok(data.build())
    }

    /// If there are sources and no resource, the only output is from the javac action.
    /// Otherwise create a separate jar for the compilation and add resources with singlejar.
    // Java: com/google/devtools/build/lib/rules/java/JavaCompilationHelper.java:344 JavaCompilationHelper.separateResourceJar
    fn separate_resource_jar(
        &self,
        resource_jars: &[Artifact],
        attributes: &JavaTargetAttributes<'_>,
    ) -> bool {
        !resource_jars.is_empty()
            || !attributes.get_resources().is_empty()
            || !attributes.get_resource_jars().is_empty()
            || !attributes.get_class_path_resources().is_empty()
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaCompilationHelper.java:352 JavaCompilationHelper.getExecutionInfo
    fn get_execution_info(&self) -> starlark::Result<IndexMap<String, String>> {
        let mut modifiable_execution_info = IndexMap::new();
        modifiable_execution_info.insert(SUPPORTS_PATH_MAPPING.to_owned(), "1".to_owned());
        if self.java_toolchain.get_javac_supports_workers()? {
            modifiable_execution_info.insert(SUPPORTS_WORKERS.to_owned(), "1".to_owned());
        }
        if self.java_toolchain.get_javac_supports_multiplex_workers()? {
            modifiable_execution_info.insert(SUPPORTS_MULTIPLEX_WORKERS.to_owned(), "1".to_owned());
        }
        if self
            .java_toolchain
            .get_javac_supports_worker_cancellation()?
        {
            modifiable_execution_info
                .insert(SUPPORTS_WORKER_CANCELLATION.to_owned(), "1".to_owned());
        }
        if self
            .java_toolchain
            .get_javac_supports_worker_multiplex_sandboxing()?
        {
            modifiable_execution_info
                .insert(SUPPORTS_MULTIPLEX_SANDBOXING.to_owned(), "1".to_owned());
        }
        let mut execution_info = IndexMap::new();
        for (k, v) in modified_execution_info(
            self.rule_context.get_configuration(),
            &modifiable_execution_info,
            java_compile_action_builder::MNEMONIC,
        ) {
            execution_info.insert(k, v);
        }
        for (k, v) in target_utils::get_execution_info_with_propagation(
            self.rule_context.get_rule(),
            self.rule_context.is_allow_tags_propagation(),
        ) {
            execution_info.insert(k, v);
        }
        // Java: executionInfo.buildKeepingLast()
        Ok(execution_info)
    }

    /// Returns the bootclasspath explicit set in attributes if present, or else the default.
    // Java: com/google/devtools/build/lib/rules/java/JavaCompilationHelper.java:378 JavaCompilationHelper.getBootclasspathOrDefault
    pub fn get_bootclasspath_or_default(&mut self) -> starlark::Result<BootClassPathInfo<'v>> {
        let attributes = self.get_attributes();
        if !attributes.get_boot_class_path().is_empty()? {
            Ok(attributes.get_boot_class_path())
        } else {
            self.java_toolchain.get_bootclasspath()
        }
    }

    /// Creates an Artifact needed by `JacocoCoverageRunner`.
    ///
    /// The Artifact is created in the same directory as the given `compileJar` and has the
    /// suffix `-paths-for-coverage.txt`. Returns `None` if `compileJar` should not be
    /// instrumented.
    // Java: com/google/devtools/build/lib/rules/java/JavaCompilationHelper.java:396 JavaCompilationHelper.maybeCreateCoverageArtifact
    fn maybe_create_coverage_artifact(&self, compile_jar: &Artifact) -> Option<Artifact> {
        if !self.should_instrument_jar() {
            return None;
        }
        let package_relative_path = compile_jar
            .get_root_relative_path()
            .relative_to(&self.rule_context.get_package_directory());
        let path = replace_extension(&package_relative_path, "-paths-for-coverage.txt");
        Some(
            self.rule_context
                .get_package_relative_artifact(&path, compile_jar.get_root()),
        )
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaCompilationHelper.java:407 JavaCompilationHelper.shouldInstrumentJar
    fn should_instrument_jar(&self) -> bool {
        let rule_context = self.get_rule_context();
        config_util::is_code_coverage_enabled(rule_context.get_configuration())
            && self.attributes.has_source_files()
            && should_include_local_sources(
                rule_context.get_configuration(),
                rule_context.get_label(),
                rule_context.is_test_target(),
            )
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaCompilationHelper.java:415 JavaCompilationHelper.turbineOutput
    fn turbine_output(&self, class_jar: &Artifact, new_extension: &str) -> Artifact {
        self.rule_context
            .get_analysis_environment()
            .get_derived_artifact(
                &replace_extension(
                    &class_jar.get_output_dir_relative_path(self.is_sibling_repository_layout()),
                    new_extension,
                ),
                class_jar.get_root(),
            )
    }

    /// Creates the Action that compiles ijars from source.
    // Java: com/google/devtools/build/lib/rules/java/JavaCompilationHelper.java:430 JavaCompilationHelper.createHeaderCompilationAction
    pub fn create_header_compilation_action(
        &mut self,
        eval: &Evaluator,
        output_jar: &Artifact,
        header_compilation_output_jar: &Artifact,
        header_deps: &Artifact,
    ) -> starlark::Result<()> {
        let attributes = self.get_attributes();

        // only run API-generating annotation processors during header compilation
        let plugins = attributes.plugins().api_generating_plugins().clone();

        let mut builder = self.get_java_header_compile_action_builder()?;
        builder.set_output_jar(output_jar.clone());
        builder.set_header_compilation_output_jar(header_compilation_output_jar.clone());
        builder.set_output_deps_proto(header_deps.clone());
        builder.set_plugins(plugins.clone());
        if plugins
            .processor_classes()
            .to_list()
            .iter()
            .any(|c| c == "dagger.internal.codegen.ComponentProcessor")
        {
            // See b/31371210 and b/142059842.
            builder.add_turbine_hjar_javac_opt();
        }
        builder.enable_direct_classpath(self.enable_direct_classpath);
        builder.build(eval, &self.java_toolchain)
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaCompilationHelper.java:454 JavaCompilationHelper.getJavaHeaderCompileActionBuilder
    fn get_java_header_compile_action_builder(
        &mut self,
    ) -> starlark::Result<JavaHeaderCompileActionBuilder<'a>> {
        let attributes = self.get_attributes();
        let mut builder = JavaHeaderCompileAction::new_builder(self.rule_context);
        builder.set_source_files(attributes.get_source_files().clone());
        builder.set_source_jars(attributes.get_source_jars().to_vec());
        builder.set_classpath_entries(attributes.get_compile_time_class_path().clone());
        builder.set_bootclasspath_entries(self.get_bootclasspath_or_default()?.bootclasspath()?);
        // Exclude any per-package configured data (see computePerPackageData).
        // It is used to allow Error Prone checks to load additional data,
        // and Error Prone doesn't run during header compilation.
        builder.set_javac_opts(self.custom_javac_opts.clone());
        builder.set_strict_java_deps(attributes.get_strict_java_deps());
        builder.set_compile_time_dependency_artifacts(
            attributes.get_compile_time_dependency_artifacts().clone(),
        );
        builder.set_header_compilation_direct_jars(
            attributes.get_header_compilation_direct_jars().clone(),
        );
        builder.set_direct_jars(attributes.get_direct_jars().clone());
        builder.set_target_label(attributes.get_target_label().cloned());
        builder.set_injecting_rule_kind(attributes.get_injecting_rule_kind().map(str::to_owned));
        builder.set_additional_inputs(self.additional_inputs_for_databinding.clone());
        builder.set_tools_jars(self.java_toolchain.get_tools()?);
        builder.set_exec_group(self.exec_group.clone());
        builder.set_utf8_environment(
            self.semantics
                .utf8_environment(self.rule_context.get_execution_platform().as_deref()),
        );
        Ok(builder)
    }

    /// Returns whether this target uses annotation processing.
    // Java: com/google/devtools/build/lib/rules/java/JavaCompilationHelper.java:482 JavaCompilationHelper.usesAnnotationProcessing
    pub fn uses_annotation_processing(&mut self) -> bool {
        let attributes = self.get_attributes();
        self.get_javac_opts().iter().any(|o| o == "-processor")
            || attributes.plugins().has_processors()
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaCompilationHelper.java:487 JavaCompilationHelper.createGenJarAction
    fn create_gen_jar_action(
        &self,
        eval: &Evaluator,
        class_jar: &Artifact,
        manifest_proto: &Artifact,
        gen_class_jar: &Artifact,
        host_javabase: &JavaRuntimeInfo<'_>,
    ) -> starlark::Result<()> {
        let mut builder = SpawnActionBuilder::new();
        builder
            .add_input(manifest_proto.clone())
            .add_input(class_jar.clone())
            .add_output(gen_class_jar.clone())
            .add_transitive_inputs(&host_javabase.java_base_inputs()?);
        // Java: setJarExecutable(javaBinaryExecPathFragment, getGenClassJar, jvmOptions).
        // PORT: Blaze's SpawnAction.Builder has no setJarExecutable; its executable command line
        // is added as the first command line and the deploy jar as a tool, which builds the
        // same command lines and tools.
        let deploy_jar = self.get_gen_class_jar()?;
        let mut jar_executable = CustomCommandLine::builder();
        jar_executable
            .add_path(host_javabase.java_binary_exec_path_fragment()?)
            .add_all_nested_set(self.java_toolchain.get_jvm_options()?)
            .add("-jar")
            .add_exec_path(deploy_jar.clone());
        builder.add_tool(deploy_jar);
        builder.add_command_line(Arc::new(jar_executable.build()));
        let mut command = CustomCommandLine::builder();
        command
            .add_exec_path_with_arg("--manifest_proto", Some(manifest_proto.clone()))
            .add_exec_path_with_arg("--class_jar", Some(class_jar.clone()))
            .add_exec_path_with_arg("--output_jar", Some(gen_class_jar.clone()));
        builder
            .add_command_line(Arc::new(command.build()))
            .set_progress_message("Building genclass jar %{output}")
            .set_mnemonic("JavaSourceJar")
            .set_exec_group(self.exec_group.clone());
        let action = builder.build(
            self.rule_context
                .get_action_owner_for(builder.get_exec_group())
                .unwrap_or_else(|| panic!("NullPointerException: action owner")),
            &ConfigurationSpawnActionConfiguration(self.rule_context.get_configuration().clone()),
        );
        register_action(eval, Arc::new(action))
    }

    /// Returns the GenClass deploy jar Artifact.
    // Java: com/google/devtools/build/lib/rules/java/JavaCompilationHelper.java:513 JavaCompilationHelper.getGenClassJar
    fn get_gen_class_jar(&self) -> starlark::Result<Artifact> {
        if let Some(gen_class) = self.java_toolchain.get_gen_class()? {
            return Ok(gen_class);
        }
        Ok(self
            .rule_context
            .get_prerequisite_artifact("$genclass")
            .unwrap_or_else(|| panic!("NullPointerException: $genclass")))
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaCompilationHelper.java:521 JavaCompilationHelper.createResourceJarAction
    fn create_resource_jar_action(
        &mut self,
        eval: &Evaluator,
        resource_jar: &Artifact,
        extra_jars: Vec<Artifact>,
    ) -> starlark::Result<()> {
        let attributes = self.get_attributes();
        let mut resource_jars = NestedSetBuilder::from_nested_set(attributes.get_resource_jars());
        resource_jars.add_all(extra_jars);
        ResourceJarActionBuilder::new()
            .set_additional_inputs(NestedSetBuilder::wrap(
                Order::STABLE_ORDER,
                self.additional_inputs_for_databinding.iter().cloned(),
            ))
            .set_java_toolchain(self.java_toolchain)
            .set_output_jar(resource_jar.clone())
            .set_resources(attributes.get_resources().clone())
            .set_classpath_resources(attributes.get_class_path_resources().to_vec())
            .set_resource_jars(resource_jars.build())
            .build(eval, self.semantics, self.rule_context, &self.exec_group)
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaCompilationHelper.java:536 JavaCompilationHelper.createLocalOptimizationAction
    fn create_local_optimization_action(
        &self,
        eval: &Evaluator,
        unoptimized_output_jar: &Artifact,
        optimized_output_jar: &Artifact,
        classpath: &NestedSet<Artifact>,
        configs: &[Artifact],
        optimizer: &Arc<FilesToRunProvider>,
        mnemonic: &str,
    ) -> starlark::Result<()> {
        let mut command = CustomCommandLine::builder();
        command
            .add("-runtype")
            .add("LOCAL_ONLY")
            .add_exec_path_with_arg("-injars", Some(unoptimized_output_jar.clone()))
            .add_exec_path_with_arg("-outjars", Some(optimized_output_jar.clone()))
            .add_exec_paths_vector_arg(
                blaze_actions::analysis::actions::custom_command_line::vector_arg::add_before(
                    "-libraryjars",
                )
                .each_nested_set(classpath.clone()),
            );
        for config in configs {
            command.add_prefixed_exec_path("@", config.clone());
        }

        let mut builder = SpawnActionBuilder::new();
        builder
            .add_input(unoptimized_output_jar.clone())
            .add_transitive_inputs(classpath)
            .add_inputs(configs.iter().cloned())
            .add_output(optimized_output_jar.clone())
            .set_executable_files_to_run(
                optimizer.get_executable().cloned(),
                optimizer.get_files_to_run(),
            )
            .add_command_line_with_param_file(
                Arc::new(command.build()),
                Some(ParamFileInfo::builder(ParameterFileType::UNQUOTED).build()),
            )
            .set_progress_message("Optimizing jar %{label}")
            .set_mnemonic(mnemonic.to_owned())
            .set_exec_group(self.exec_group.clone());
        let action = builder.build(
            self.rule_context
                .get_action_owner_for(builder.get_exec_group())
                .unwrap_or_else(|| panic!("NullPointerException: action owner")),
            &ConfigurationSpawnActionConfiguration(self.rule_context.get_configuration().clone()),
        );
        register_action(eval, Arc::new(action))
    }

    /// Gets the value of the "javacopts" attribute combining them with the default options. If
    /// the current rule has no javacopts attribute, this method only returns the default
    /// options.
    // Java: com/google/devtools/build/lib/rules/java/JavaCompilationHelper.java:578 JavaCompilationHelper.getJavacOpts
    fn get_javac_opts(&self) -> &[String] {
        &self.custom_javac_opts
    }
}
