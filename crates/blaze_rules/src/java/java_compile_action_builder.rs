//! `rules/java/JavaCompileActionBuilder.java`: Java compilation action builder.

use std::sync::Arc;

use blaze_actions::Artifact;
use blaze_actions::actions::action::Action;
use blaze_actions::actions::command_line::CommandLineRef;
use blaze_actions::analysis::actions::custom_command_line::CustomCommandLine;
use blaze_analysis::rule::RuleContext;
use blaze_cmdline::Label;
use blaze_collect::{NestedSet, NestedSetBuilder, Order};
use blaze_optdefs::fragment::converters::CoreOptionConvertersStrictDepsMode as StrictDepsMode;
use blaze_optdefs::fragment::java_configuration::JavaConfigurationJavaClasspathMode as JavaClasspathMode;
use blaze_options::converters::JavaEnum;
use blaze_util::check_state;
use blaze_vfs::PathFragment;
use indexmap::{IndexMap, IndexSet};

use super::boot_class_path_info::BootClassPathInfo;
use super::java_compile_action::{CompilationType, JavaCompileAction, ProgressMessage};
use super::java_output::JavaCompileOutputs;
use super::java_plugin_info::JavaPluginData;
use super::java_toolchain_provider::JavaToolchainProvider;
use super::java_toolchain_tool::JavaToolchainTool;

// Java: com/google/devtools/build/lib/rules/java/JavaCompileActionBuilder.java:54 JavaCompileActionBuilder.JACOCO_INSTRUMENTATION_PROCESSOR
const JACOCO_INSTRUMENTATION_PROCESSOR: &str = "jacoco";
// Java: com/google/devtools/build/lib/rules/java/JavaCompileActionBuilder.java:55 JavaCompileActionBuilder.PROGRESS_MESSAGE_PREFIX
const PROGRESS_MESSAGE_PREFIX: &str = "Building";

// Java: com/google/devtools/build/lib/rules/java/JavaCompileActionBuilder.java:57 JavaCompileActionBuilder.MNEMONIC
pub const MNEMONIC: &str = "Javac";

/// Returns true if this is a Java compile action.
// Java: com/google/devtools/build/lib/rules/java/JavaCompileActionBuilder.java:60 JavaCompileActionBuilder.isJavaCompileAction
pub fn is_java_compile_action(action: Option<&dyn Action>) -> bool {
    action.is_some_and(|a| a.get_mnemonic() == MNEMONIC)
}

/// The `JavaCompileInfo` of a Java compilation for extra actions.
// Java: com/google/devtools/build/lib/rules/java/JavaCompileActionBuilder.java:66 JavaCompileActionBuilder.JavaCompileExtraActionInfoSupplier
#[derive(Debug)]
pub struct JavaCompileExtraActionInfoSupplier {
    output_jar: Artifact,
    /// The list of classpath entries to specify to javac.
    classpath_entries: NestedSet<Artifact>,
    /// The list of bootclasspath entries to specify to javac.
    bootclasspath_entries: NestedSet<Artifact>,
    /// An argument to the javac >= 9 `--system` flag.
    system: Option<PathFragment>,
    /// The list of classpath entries to search for annotation processors.
    processor_path: NestedSet<Artifact>,
    /// The list of annotation processor classes to run.
    processor_names: NestedSet<String>,
    /// Set of additional Java source files to compile.
    source_jars: Vec<Artifact>,
    /// The set of explicit Java source files to compile.
    source_files: IndexSet<Artifact>,
    /// The compiler options to pass to javac.
    javac_opts: Vec<String>,
}

impl JavaCompileExtraActionInfoSupplier {
    // Java: com/google/devtools/build/lib/rules/java/JavaCompileActionBuilder.java:117 JavaCompileActionBuilder.JavaCompileExtraActionInfoSupplier.extend
    pub fn extend(
        &self,
        builder: &mut blaze_protos::blaze::ExtraActionInfo,
        arguments: Vec<String>,
    ) {
        // Artifact.toExecPaths: runfiles trees are left out.
        let exec_paths = |artifacts: &mut dyn Iterator<Item = &Artifact>| -> Vec<String> {
            Artifact::to_exec_paths(artifacts).collect()
        };
        let mut info = blaze_protos::blaze::JavaCompileInfo {
            source_file: exec_paths(&mut self.source_files.iter()),
            classpath: exec_paths(&mut self.classpath_entries.to_list().iter()),
            bootclasspath: exec_paths(&mut self.bootclasspath_entries.to_list().iter()),
            sourcepath: exec_paths(&mut self.source_jars.iter()),
            javac_opt: self.javac_opts.clone(),
            processor: self.processor_names.to_list().iter().cloned().collect(),
            processorpath: exec_paths(&mut self.processor_path.to_list().iter()),
            outputjar: Some(self.output_jar.get_exec_path_string().to_owned()),
            ..Default::default()
        };
        if let Some(system) = &self.system {
            info.system = Some(system.to_string());
        }
        info.argument.extend(arguments);
        builder.java_compile_info = Some(info);
    }
}

/// `new JavaCompileProgressMessage(output, sourceFiles, sourceJars, plugins)`: a
/// `ProgressMessage` with the prefix "Building".
// Java: com/google/devtools/build/lib/rules/java/JavaCompileActionBuilder.java:505 JavaCompileActionBuilder.JavaCompileProgressMessage.<init>
// Java: com/google/devtools/build/lib/rules/java/JavaCompileActionBuilder.java:514 JavaCompileActionBuilder.JavaCompileProgressMessage.prefix
fn java_compile_progress_message(
    output: &Artifact,
    source_files: &IndexSet<Artifact>,
    source_jars: &[Artifact],
    plugins: &JavaPluginData,
) -> ProgressMessage {
    ProgressMessage::new(
        PROGRESS_MESSAGE_PREFIX,
        output,
        source_files.len(),
        source_jars.len(),
        plugins,
    )
}

/// Java compilation action builder.
// Java: com/google/devtools/build/lib/rules/java/JavaCompileActionBuilder.java:52 JavaCompileActionBuilder
pub struct JavaCompileActionBuilder<'a, 'v> {
    rule_context: &'a RuleContext,
    toolchain: JavaToolchainProvider<'v>,
    exec_group: String,
    additional_outputs: IndexSet<Artifact>,
    coverage_artifact: Option<Artifact>,
    source_files: IndexSet<Artifact>,
    source_jars: Vec<Artifact>,
    strict_java_deps: StrictDepsMode,
    fix_deps_tool: String,
    direct_jars: NestedSet<Artifact>,
    compile_time_dependency_artifacts: NestedSet<Artifact>,
    javac_opts: Vec<String>,
    utf8_environment: Option<IndexMap<String, String>>,
    execution_info: IndexMap<String, String>,
    compress_jar: bool,
    classpath_entries: NestedSet<Artifact>,
    boot_class_path: BootClassPathInfo<'v>,
    source_path_entries: Vec<Artifact>,
    java_builder: Option<JavaToolchainTool>,
    tools_jars: NestedSet<Artifact>,
    plugins: JavaPluginData,
    extra_data: NestedSet<Artifact>,
    target_label: Option<Label>,
    injecting_rule_kind: Option<String>,
    extra_command_line_args: Vec<CommandLineRef>,
    additional_inputs: Vec<Artifact>,
    gen_source_output: Option<Artifact>,
    outputs: Option<JavaCompileOutputs>,
    classpath_mode: JavaClasspathMode,
    manifest_output: Option<Artifact>,
}

impl<'a, 'v> JavaCompileActionBuilder<'a, 'v> {
    // Java: com/google/devtools/build/lib/rules/java/JavaCompileActionBuilder.java:168 JavaCompileActionBuilder.<init>
    pub fn new(
        rule_context: &'a RuleContext,
        toolchain: JavaToolchainProvider<'v>,
        exec_group: String,
    ) -> Self {
        JavaCompileActionBuilder {
            rule_context,
            toolchain,
            exec_group,
            additional_outputs: IndexSet::new(),
            coverage_artifact: None,
            source_files: IndexSet::new(),
            source_jars: Vec::new(),
            strict_java_deps: StrictDepsMode::ERROR,
            fix_deps_tool: "add_dep".to_owned(),
            direct_jars: NestedSetBuilder::empty_set(Order::NAIVE_LINK_ORDER),
            compile_time_dependency_artifacts: NestedSetBuilder::empty_set(Order::STABLE_ORDER),
            javac_opts: Vec::new(),
            utf8_environment: None,
            execution_info: IndexMap::new(),
            compress_jar: false,
            classpath_entries: NestedSetBuilder::empty_set(Order::NAIVE_LINK_ORDER),
            boot_class_path: BootClassPathInfo::empty(),
            source_path_entries: Vec::new(),
            java_builder: None,
            tools_jars: NestedSetBuilder::empty_set(Order::NAIVE_LINK_ORDER),
            plugins: JavaPluginData::empty(),
            extra_data: NestedSetBuilder::empty_set(Order::NAIVE_LINK_ORDER),
            target_label: None,
            injecting_rule_kind: None,
            extra_command_line_args: Vec::new(),
            additional_inputs: Vec::new(),
            gen_source_output: None,
            outputs: None,
            // PORT: Java leaves the field null until setClasspathMode; every caller sets it.
            classpath_mode: JavaClasspathMode::OFF,
            manifest_output: None,
        }
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaCompileActionBuilder.java:175 JavaCompileActionBuilder.build
    pub fn build(&mut self) -> starlark::Result<JavaCompileAction> {
        let utf8_environment = self
            .utf8_environment
            .clone()
            .unwrap_or_else(|| panic!("NullPointerException: utf8Environment must not be null"));
        let outputs = self
            .outputs
            .clone()
            .unwrap_or_else(|| panic!("NullPointerException: outputs"));

        // TODO(bazel-team): all the params should be calculated before getting here, and the
        // various aggregation code below should go away.

        // Invariant: if strictJavaDeps is OFF, then directJars and
        // dependencyArtifacts are ignored
        if self.strict_java_deps == StrictDepsMode::OFF {
            self.direct_jars = NestedSetBuilder::empty_set(Order::NAIVE_LINK_ORDER);
            self.compile_time_dependency_artifacts =
                NestedSetBuilder::empty_set(Order::STABLE_ORDER);
        }

        // Invariant: if java_classpath is set to 'off', dependencyArtifacts are ignored
        let incompatible = self
            .toolchain
            .get_reduced_classpath_incompatible_processors()?;
        if self
            .plugins
            .processor_classes()
            .to_set()
            .iter()
            .any(|p| incompatible.contains(p))
        {
            self.classpath_mode = JavaClasspathMode::OFF;
        }
        if self.classpath_mode == JavaClasspathMode::OFF {
            self.compile_time_dependency_artifacts =
                NestedSetBuilder::empty_set(Order::STABLE_ORDER);
        }

        let java_builder = self
            .java_builder
            .clone()
            .unwrap_or_else(|| panic!("NullPointerException: javaBuilder"));
        let mut tools_builder = NestedSetBuilder::compile_order();
        java_builder.add_inputs(&self.toolchain, &mut tools_builder)?;
        tools_builder.add_transitive(&self.tools_jars);

        let mut mandatory_inputs_builder = NestedSetBuilder::stable_order();
        mandatory_inputs_builder
            .add_transitive(self.plugins.processor_classpath())
            .add_transitive(self.plugins.data())
            .add_transitive(&self.extra_data)
            .add_all(self.source_jars.iter().cloned())
            .add_all(self.source_files.iter().cloned())
            .add_transitive(&self.toolchain.get_java_runtime()?.java_base_inputs()?)
            .add_transitive(&self.boot_class_path.bootclasspath()?)
            .add_all(self.source_path_entries.iter().cloned())
            .add_all(self.additional_inputs.iter().cloned())
            .add_transitive(&self.boot_class_path.system_inputs()?);
        if let Some(coverage_artifact) = &self.coverage_artifact {
            mandatory_inputs_builder.add(coverage_artifact.clone());
        }

        let extra_action_info_supplier = JavaCompileExtraActionInfoSupplier {
            output_jar: outputs.output().clone(),
            classpath_entries: self.classpath_entries.clone(),
            bootclasspath_entries: self.boot_class_path.bootclasspath()?,
            system: self.boot_class_path.system_path()?,
            processor_path: self.plugins.processor_classpath().clone(),
            processor_names: self.plugins.processor_classes().clone(),
            source_jars: self.source_jars.clone(),
            source_files: self.source_files.clone(),
            javac_opts: self.javac_opts.clone(),
        };

        // TODO(b/123076347): outputDepsProto should never be null if SJD is enabled
        if self.strict_java_deps == StrictDepsMode::OFF || outputs.deps_proto().is_none() {
            self.classpath_mode = JavaClasspathMode::OFF;
        }

        let tools = tools_builder.build();
        mandatory_inputs_builder.add_transitive(&tools);
        let mandatory_inputs = mandatory_inputs_builder.build();

        let executable_line = java_builder.get_command_line(&self.toolchain)?;

        let action_environment =
            Arc::new(blaze_analysis::rule::action_environments::to_actions_env(
                self.rule_context
                    .get_configuration()
                    .get_action_environment(),
            ))
            .with_additional_fixed_variables(utf8_environment.clone());

        let owner = self
            .rule_context
            .get_action_owner_for(&self.exec_group)
            .unwrap_or_else(|| panic!("NullPointerException: action owner of {}", self.exec_group));

        let flag_line = self.build_param_file_contents(&outputs, &self.javac_opts.clone())?;

        Ok(JavaCompileAction::new(
            /* compilation_type= */ CompilationType::JAVAC,
            owner,
            tools,
            /* progress_message= */
            java_compile_progress_message(
                outputs.output(),
                &self.source_files,
                &self.source_jars,
                &self.plugins,
            ),
            mandatory_inputs,
            /* transitive_inputs= */ self.classpath_entries.clone(),
            self.direct_jars.clone(),
            self.all_outputs(&outputs),
            action_environment,
            self.execution_info.clone(),
            Arc::new(executable_line),
            Arc::new(flag_line),
            self.extra_command_line_args.clone(),
            self.rule_context.get_configuration().clone(),
            /* dependency_artifacts= */ self.compile_time_dependency_artifacts.clone(),
            outputs.deps_proto().cloned(),
            self.classpath_mode,
        )
        .with_extra_action_info_supplier(Arc::new(extra_action_info_supplier)))
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaCompileActionBuilder.java:272 JavaCompileActionBuilder.allOutputs
    fn all_outputs(&self, outputs: &JavaCompileOutputs) -> Vec<Artifact> {
        let mut result: IndexSet<Artifact> = IndexSet::new();
        result.insert(outputs.output().clone());
        result.extend(self.additional_outputs.iter().cloned());
        for a in [
            outputs.deps_proto(),
            outputs.native_header(),
            self.gen_source_output.as_ref(),
            self.manifest_output.as_ref(),
        ]
        .into_iter()
        .flatten()
        {
            result.insert(a.clone());
        }
        result.into_iter().collect()
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaCompileActionBuilder.java:281 JavaCompileActionBuilder.buildParamFileContents
    fn build_param_file_contents(
        &self,
        outputs: &JavaCompileOutputs,
        javac_opts: &[String],
    ) -> starlark::Result<CustomCommandLine> {
        let mut result = CustomCommandLine::builder();

        result.add_exec_path_with_arg("--output", Some(outputs.output().clone()));
        result.add_exec_path_with_arg("--native_header_output", outputs.native_header().cloned());
        result.add_exec_path_with_arg("--generated_sources_output", self.gen_source_output.clone());
        result.add_exec_path_with_arg("--output_manifest_proto", self.manifest_output.clone());
        if self.compress_jar {
            result.add("--compress_jar");
        }
        result.add_exec_path_with_arg("--output_deps_proto", outputs.deps_proto().cloned());
        result.add_exec_paths_nested_set_with_arg(
            "--bootclasspath",
            self.boot_class_path.bootclasspath()?,
        );
        if let Some(system) = self.boot_class_path.system_path()? {
            result.add_path_with_arg("--system", Some(system));
        }
        result.add_exec_paths_with_arg("--sourcepath", self.source_path_entries.iter().cloned());
        result.add_exec_paths_nested_set_with_arg(
            "--processorpath",
            self.plugins.processor_classpath().clone(),
        );
        result
            .add_all_nested_set_with_arg("--processors", self.plugins.processor_classes().clone());
        result.add_exec_paths_with_arg("--source_jars", self.source_jars.iter().cloned());
        result.add_exec_paths_with_arg("--sources", self.source_files.iter().cloned());
        if !javac_opts.is_empty() {
            result
                .add("--javacopts")
                .add_all(javac_opts.iter().cloned());
            // terminate --javacopts with `--` to support javac flags that start with `--`
            result.add("--");
        }
        if let Some(target_label) = &self.target_label {
            result.add("--target_label");
            if target_label.get_repository().is_main() {
                result.add_label(target_label.clone());
            } else {
                // @-prefixed strings will be assumed to be filenames and expanded by
                // JavaLibraryBuildRequest, so add an extra &at; to escape it.
                let main_repo_mapping = self
                    .rule_context
                    .get_analysis_environment()
                    .get_main_repo_mapping()
                    .map_err(|e| crate::cpp::util::errorf(e.to_string()))?
                    .clone();
                result.add_prefixed_label("@", target_label.clone(), Some(main_repo_mapping));
            }
        }
        result.add_with_arg("--injecting_rule_kind", self.injecting_rule_kind.clone());
        // strict_java_deps controls whether the mapping from jars to targets is
        // written out and whether we try to minimize the compile-time classpath.
        if self.strict_java_deps != StrictDepsMode::OFF {
            result
                .add("--strict_java_deps")
                .add(self.strict_java_deps.name());
            result.add_exec_paths_nested_set_with_arg(
                "--direct_dependencies",
                self.direct_jars.clone(),
            );
        }
        result
            .add("--experimental_fix_deps_tool")
            .add(self.fix_deps_tool.clone());

        // Chose what artifact to pass to JavaBuilder, as input to jacoco instrumentation
        // processor.
        if let Some(coverage_artifact) = &self.coverage_artifact {
            result.add("--post_processor");
            result.add_exec_path_with_arg(
                JACOCO_INSTRUMENTATION_PROCESSOR,
                Some(coverage_artifact.clone()),
            );
        }
        Ok(result.build())
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaCompileActionBuilder.java:337 JavaCompileActionBuilder.setAdditionalOutputs
    pub fn set_additional_outputs(&mut self, outputs: IndexSet<Artifact>) -> &mut Self {
        self.additional_outputs = outputs;
        self
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaCompileActionBuilder.java:343 JavaCompileActionBuilder.setSourceFiles
    pub fn set_source_files(&mut self, source_files: IndexSet<Artifact>) -> &mut Self {
        self.source_files = source_files;
        self
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaCompileActionBuilder.java:349 JavaCompileActionBuilder.setSourceJars
    pub fn set_source_jars(&mut self, source_jars: Vec<Artifact>) -> &mut Self {
        check_state!(self.source_jars.is_empty());
        self.source_jars = source_jars;
        self
    }

    /// Sets the strictness of Java dependency checking, see `StrictDepsMode`.
    // Java: com/google/devtools/build/lib/rules/java/JavaCompileActionBuilder.java:357 JavaCompileActionBuilder.setStrictJavaDeps
    pub fn set_strict_java_deps(&mut self, strict_deps: StrictDepsMode) -> &mut Self {
        self.strict_java_deps = strict_deps;
        self
    }

    /// Sets the tool with which to fix dependency errors.
    // Java: com/google/devtools/build/lib/rules/java/JavaCompileActionBuilder.java:364 JavaCompileActionBuilder.setFixDepsTool
    pub fn set_fix_deps_tool(&mut self, deps_tool: String) -> &mut Self {
        self.fix_deps_tool = deps_tool;
        self
    }

    /// Accumulates the given jar artifacts as being provided by direct dependencies.
    // Java: com/google/devtools/build/lib/rules/java/JavaCompileActionBuilder.java:371 JavaCompileActionBuilder.setDirectJars
    pub fn set_direct_jars(&mut self, direct_jars: NestedSet<Artifact>) -> &mut Self {
        self.direct_jars = direct_jars;
        self
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaCompileActionBuilder.java:377 JavaCompileActionBuilder.setCompileTimeDependencyArtifacts
    pub fn set_compile_time_dependency_artifacts(
        &mut self,
        dependency_artifacts: NestedSet<Artifact>,
    ) -> &mut Self {
        self.compile_time_dependency_artifacts = dependency_artifacts;
        self
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaCompileActionBuilder.java:385 JavaCompileActionBuilder.setExtraCommandLineArgs
    pub fn set_extra_command_line_args(
        &mut self,
        extra_command_line_args: Vec<CommandLineRef>,
    ) -> &mut Self {
        self.extra_command_line_args = extra_command_line_args;
        self
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaCompileActionBuilder.java:392 JavaCompileActionBuilder.setJavacOpts
    pub fn set_javac_opts(&mut self, copts: Vec<String>) -> &mut Self {
        self.javac_opts = copts;
        self
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaCompileActionBuilder.java:398 JavaCompileActionBuilder.setUtf8Environment
    pub fn set_utf8_environment(
        &mut self,
        utf8_environment: IndexMap<String, String>,
    ) -> &mut Self {
        self.utf8_environment = Some(utf8_environment);
        self
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaCompileActionBuilder.java:404 JavaCompileActionBuilder.setJavacExecutionInfo
    pub fn set_javac_execution_info(
        &mut self,
        execution_info: IndexMap<String, String>,
    ) -> &mut Self {
        self.execution_info = execution_info;
        self
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaCompileActionBuilder.java:411 JavaCompileActionBuilder.setCompressJar
    pub fn set_compress_jar(&mut self, compress_jar: bool) -> &mut Self {
        self.compress_jar = compress_jar;
        self
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaCompileActionBuilder.java:417 JavaCompileActionBuilder.setClasspathEntries
    pub fn set_classpath_entries(&mut self, classpath_entries: NestedSet<Artifact>) -> &mut Self {
        self.classpath_entries = classpath_entries;
        self
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaCompileActionBuilder.java:423 JavaCompileActionBuilder.setBootClassPath
    pub fn set_boot_class_path(&mut self, boot_class_path: BootClassPathInfo<'v>) -> &mut Self {
        self.boot_class_path = boot_class_path;
        self
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaCompileActionBuilder.java:429 JavaCompileActionBuilder.setSourcePathEntries
    pub fn set_source_path_entries(&mut self, source_path_entries: Vec<Artifact>) -> &mut Self {
        self.source_path_entries = source_path_entries;
        self
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaCompileActionBuilder.java:435 JavaCompileActionBuilder.setPlugins
    pub fn set_plugins(&mut self, plugins: JavaPluginData) -> &mut Self {
        check_state!(self.plugins.is_empty());
        self.plugins = plugins;
        self
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaCompileActionBuilder.java:442 JavaCompileActionBuilder.setExtraData
    pub fn set_extra_data(&mut self, extra_data: NestedSet<Artifact>) {
        check_state!(self.extra_data.is_empty());
        self.extra_data = extra_data;
    }

    /// Sets the tools jars.
    // Java: com/google/devtools/build/lib/rules/java/JavaCompileActionBuilder.java:450 JavaCompileActionBuilder.setToolsJars
    pub fn set_tools_jars(&mut self, tools_jars: NestedSet<Artifact>) -> &mut Self {
        self.tools_jars = tools_jars;
        self
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaCompileActionBuilder.java:457 JavaCompileActionBuilder.setJavaBuilder
    pub fn set_java_builder(&mut self, java_builder: JavaToolchainTool) -> &mut Self {
        self.java_builder = Some(java_builder);
        self
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaCompileActionBuilder.java:463 JavaCompileActionBuilder.setCoverageArtifact
    pub fn set_coverage_artifact(&mut self, coverage_artifact: Option<Artifact>) -> &mut Self {
        self.coverage_artifact = coverage_artifact;
        self
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaCompileActionBuilder.java:469 JavaCompileActionBuilder.setTargetLabel
    pub fn set_target_label(&mut self, target_label: Option<Label>) -> &mut Self {
        self.target_label = target_label;
        self
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaCompileActionBuilder.java:475 JavaCompileActionBuilder.setInjectingRuleKind
    pub fn set_injecting_rule_kind(&mut self, injecting_rule_kind: Option<String>) -> &mut Self {
        self.injecting_rule_kind = injecting_rule_kind;
        self
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaCompileActionBuilder.java:481 JavaCompileActionBuilder.setAdditionalInputs
    pub fn set_additional_inputs(&mut self, additional_inputs: Vec<Artifact>) -> &mut Self {
        self.additional_inputs = additional_inputs;
        self
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaCompileActionBuilder.java:487 JavaCompileActionBuilder.setGenSourceOutput
    pub fn set_gen_source_output(&mut self, gen_source_output: Option<Artifact>) {
        self.gen_source_output = gen_source_output;
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaCompileActionBuilder.java:491 JavaCompileActionBuilder.setOutputs
    pub fn set_outputs(&mut self, outputs: JavaCompileOutputs) {
        self.outputs = Some(outputs);
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaCompileActionBuilder.java:495 JavaCompileActionBuilder.setClasspathMode
    pub fn set_classpath_mode(&mut self, classpath_mode: JavaClasspathMode) {
        self.classpath_mode = classpath_mode;
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaCompileActionBuilder.java:499 JavaCompileActionBuilder.setManifestOutput
    pub fn set_manifest_output(&mut self, manifest_output: Option<Artifact>) {
        self.manifest_output = manifest_output;
    }
}
