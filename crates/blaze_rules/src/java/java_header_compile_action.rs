//! `rules/java/JavaHeaderCompileAction.java`: action for Java header compilation, to be used if
//! `--java_header_compilation` is enabled.
//!
//! The header compiler consumes the inputs of a java compilation, and produces an interface jar
//! that can be used as a compile-time jar by upstream targets. The header interface jar is
//! equivalent to the output of ijar, but unlike ijar the header compiler operates directly on
//! Java source files instead post-processing the class outputs of the compilation. Compiling the
//! interface jar from source moves javac off the build's critical path.
//!
//! PORT: Java subclasses `SpawnAction`; the Rust action wraps a `SpawnAction` and delegates every
//! `Action` member to it except the ones Java overrides (`getExecutionInfo`,
//! `getAdditionalArtifactsForPathMapping`). `computeKey` is copied here because the inherited
//! `SpawnAction` body reads the overridden `getExecutionInfo()` virtually; it passes the literal
//! empty set to `PathMappers.addToFingerprint`, not `getAdditionalArtifactsForPathMapping()`
//! (SpawnAction.java:410-416).

use std::any::Any;
use std::collections::HashSet;
use std::sync::Arc;

use blaze_actions::Artifact;
use blaze_actions::actions::action::{
    AbstractAction, Action, ActionApiError, CommandAction, CommandLineError, ComputeKeyError,
    InputMetadataProvider,
};
use blaze_actions::actions::action_environment::ActionEnvironment;
use blaze_actions::actions::action_key_context::ActionKeyContext;
use blaze_actions::actions::action_owner::ActionOwner;
use blaze_actions::actions::command_line_limits::CommandLineLimits;
use blaze_actions::actions::command_lines::CommandLines;
use blaze_actions::actions::param_file_info::ParamFileInfo;
use blaze_actions::actions::parameter_file::ParameterFileType;
use blaze_actions::actions::path_mapper::OutputPathsMode;
use blaze_actions::analysis::actions::custom_command_line::CustomCommandLine;
use blaze_actions::analysis::actions::path_mappers;
use blaze_actions::analysis::actions::spawn_action::{
    ResourceSet as AnalysisResourceSet, SpawnAction,
};
use blaze_analysis::rule::RuleContext;
use blaze_cmdline::Label;
use blaze_collect::command_line_item::CommandLineExpansionException;
use blaze_collect::{NestedSet, NestedSetBuilder, Order};
use blaze_exec::actions::action_adapters::{ActionMetadataSnapshot, artifact_input};
use blaze_exec::actions::execution_requirements::REMOTE_EXECUTION_INLINE_OUTPUTS;
use blaze_exec::actions::{
    ActionExecutionException, ActionInputRef, ActionResult, ExecutableAction,
    PathMapper as SpawnPathMapper, ResourceSet, SimpleSpawn,
};
use blaze_exec::analysis::actions::to_action_input_set;
use blaze_exec::exec::action_execution_context::{ActionExecutionContext, get_context};
use blaze_exec::exec::spawn_strategy_resolver::SpawnStrategyResolver;
use blaze_optdefs::fragment::converters::CoreOptionConvertersStrictDepsMode as StrictDepsMode;
use blaze_optdefs::fragment::java_configuration::JavaConfigurationJavaClasspathMode as JavaClasspathMode;
use blaze_packages::target::target_utils;
use blaze_util::Fingerprint;
use indexmap::{IndexMap, IndexSet};

use super::java_compilation_helper::register_action;
use super::java_compile_action::{
    CompilationType, JavaActionMetadata, JavaCompileAction, ProgressMessage,
    create_full_output_deps, get_output_paths_mode, merge_maps, modified_execution_info,
    to_action_execution_exception,
};
use super::java_compile_action_context::JavaCompileActionContext;
use super::java_plugin_info::JavaPluginData;
use super::java_toolchain_provider::JavaToolchainProvider;

// Java: com/google/devtools/build/lib/rules/java/JavaHeaderCompileAction.java:78 JavaHeaderCompileAction.DIRECT_CLASSPATH_MNEMONIC
const DIRECT_CLASSPATH_MNEMONIC: &str = "Turbine";
// Java: com/google/devtools/build/lib/rules/java/JavaHeaderCompileAction.java:79 JavaHeaderCompileAction.PROGRESS_MESSAGE_PREFIX
const PROGRESS_MESSAGE_PREFIX: &str = "Compiling Java headers";

/// `SpawnAction.GUID` (the `computeKey` this class inherits).
// Java: com/google/devtools/build/lib/analysis/actions/SpawnAction.java:101 SpawnAction.GUID
const SPAWN_ACTION_GUID: &str = "ebd6fce3-093e-45ee-adb6-bf513b602f0d";

/// `JavaCompileActionBuilder.MNEMONIC`.
// Java: com/google/devtools/build/lib/rules/java/JavaCompileActionBuilder.java:57 JavaCompileActionBuilder.MNEMONIC
const JAVAC_MNEMONIC: &str = "Javac";

// Java: com/google/devtools/build/lib/actions/ExecutionRequirements.java:403 ExecutionRequirements.SUPPORTS_PATH_MAPPING
const SUPPORTS_PATH_MAPPING: &str = "supports-path-mapping";

/// Action for Java header compilation.
// Java: com/google/devtools/build/lib/rules/java/JavaHeaderCompileAction.java:76 JavaHeaderCompileAction
pub struct JavaHeaderCompileAction {
    spawn: SpawnAction,
    insert_dependencies: bool,
    in_memory_jdeps: bool,
    additional_artifacts_for_path_mapping: NestedSet<Artifact>,
}

impl std::fmt::Debug for JavaHeaderCompileAction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.to_string_java())
    }
}

impl JavaHeaderCompileAction {
    // Java: com/google/devtools/build/lib/rules/java/JavaHeaderCompileAction.java:85 JavaHeaderCompileAction.<init>
    #[allow(clippy::too_many_arguments)]
    fn new(
        owner: ActionOwner,
        tools: NestedSet<Artifact>,
        inputs: NestedSet<Artifact>,
        outputs: Vec<Artifact>,
        resource_set: AnalysisResourceSet,
        command_lines: CommandLines,
        env: Arc<ActionEnvironment>,
        execution_info: &IndexMap<String, String>,
        progress_message: String,
        mnemonic: &str,
        output_paths_mode: OutputPathsMode,
        insert_dependencies: bool,
        in_memory_jdeps: bool,
        additional_artifacts_for_path_mapping: NestedSet<Artifact>,
    ) -> JavaHeaderCompileAction {
        JavaHeaderCompileAction {
            spawn: SpawnAction::new(
                owner,
                tools,
                inputs,
                outputs,
                resource_set,
                command_lines,
                env,
                execution_info,
                Some(progress_message),
                mnemonic,
                output_paths_mode,
            ),
            insert_dependencies,
            in_memory_jdeps,
            additional_artifacts_for_path_mapping,
        }
    }

    /// The wrapped `SpawnAction` part of this action.
    pub fn spawn_action(&self) -> &SpawnAction {
        &self.spawn
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaHeaderCompileAction.java:168 JavaHeaderCompileAction.newBuilder
    pub fn new_builder(rule_context: &RuleContext) -> JavaHeaderCompileActionBuilder<'_> {
        JavaHeaderCompileActionBuilder::new(rule_context)
    }
}

impl Action for JavaHeaderCompileAction {
    fn java_class_name(&self) -> Option<&'static str> {
        Some("com.google.devtools.build.lib.rules.java.JavaHeaderCompileAction")
    }

    fn abstract_action(&self) -> &AbstractAction {
        self.spawn.abstract_action()
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn get_mnemonic(&self) -> String {
        self.spawn.get_mnemonic()
    }

    /// `SpawnAction.computeKey` with this class's `getExecutionInfo` and
    /// `getAdditionalArtifactsForPathMapping`.
    // Java: com/google/devtools/build/lib/analysis/actions/SpawnAction.java:396 SpawnAction.computeKey
    fn compute_key(
        &self,
        action_key_context: &ActionKeyContext,
        input_metadata_provider: Option<&dyn InputMetadataProvider>,
        fp: &mut Fingerprint,
    ) -> Result<(), ComputeKeyError> {
        fp.add_string(SPAWN_ACTION_GUID);
        let execution_info = self.get_execution_info();
        self.spawn.get_command_lines().add_to_fingerprint(
            action_key_context,
            input_metadata_provider,
            path_mappers::get_effective_output_paths_mode(
                self.spawn.get_output_paths_mode(),
                &self.get_mnemonic(),
                &execution_info,
            ),
            fp,
        )?;
        fp.add_string(&self.get_mnemonic());
        self.get_environment().add_to(fp);
        fp.add_string_map(execution_info.iter().map(|(k, v)| (k.as_str(), v.as_str())));
        path_mappers::add_to_fingerprint(
            &self.get_mnemonic(),
            &execution_info,
            // Java passes the literal empty set here, not the overridden
            // `getAdditionalArtifactsForPathMapping()`.
            &NestedSetBuilder::empty_set(Order::STABLE_ORDER),
            action_key_context,
            self.spawn.get_output_paths_mode(),
            fp,
        )?;
        Ok(())
    }

    fn describe_key(&self) -> Option<String> {
        self.spawn.describe_key()
    }

    fn get_tools(&self) -> NestedSet<Artifact> {
        self.spawn.get_tools()
    }

    fn get_environment(&self) -> Arc<ActionEnvironment> {
        self.spawn.get_environment()
    }

    fn is_volatile(&self) -> bool {
        self.spawn.is_volatile()
    }

    fn get_raw_progress_message(&self) -> Option<String> {
        self.spawn.get_raw_progress_message()
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaHeaderCompileAction.java:118 JavaHeaderCompileAction.getExecutionInfo
    fn get_execution_info(&self) -> IndexMap<String, String> {
        let result = self.spawn.get_execution_info();
        if !self.in_memory_jdeps {
            return result;
        }
        // Java: Iterables.get(getOutputs(), 1)
        let output_deps_proto = self.get_outputs()[1].clone();
        let mut inline = IndexMap::new();
        inline.insert(
            REMOTE_EXECUTION_INLINE_OUTPUTS.to_owned(),
            output_deps_proto.get_exec_path_string().to_string(),
        );
        merge_maps(result, inline)
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaHeaderCompileAction.java:132 JavaHeaderCompileAction.getAdditionalArtifactsForPathMapping
    fn get_additional_artifacts_for_path_mapping(&self) -> NestedSet<Artifact> {
        self.additional_artifacts_for_path_mapping.clone()
    }

    fn get_scheduling_dependencies(&self) -> NestedSet<Artifact> {
        self.spawn.get_scheduling_dependencies()
    }

    fn is_shareable(&self) -> bool {
        self.spawn.is_shareable()
    }

    fn get_effective_environment(
        &self,
        client_env: &IndexMap<String, String>,
        path_mapper: &dyn blaze_actions::actions::path_mapper::PathMapper,
    ) -> Result<IndexMap<String, String>, CommandLineExpansionException> {
        self.spawn
            .get_effective_environment(client_env, path_mapper)
    }

    fn as_command_action(&self) -> Option<&dyn CommandAction> {
        Some(self)
    }

    fn get_starlark_args(
        &self,
    ) -> Option<Vec<blaze_actions::analysis::starlark::args::RegisteredArgs>> {
        self.spawn.get_starlark_args()
    }

    // Java: com/google/devtools/build/lib/analysis/actions/SpawnAction.java:229 SpawnAction.getStarlarkArgv
    fn get_starlark_argv(&self) -> Result<Option<Vec<String>>, ActionApiError> {
        // Java: a CommandLineExpansionException becomes `new EvalException(ex)`.
        Ok(Some(self.get_arguments()?))
    }
}

impl CommandAction for JavaHeaderCompileAction {
    /// The inherited `SpawnAction.getArguments`: `PathMappers.create(this, ...)` reads this
    /// class's `getExecutionInfo` and `getAdditionalArtifactsForPathMapping`.
    // Java: com/google/devtools/build/lib/analysis/actions/SpawnAction.java:207 SpawnAction.getArguments
    fn get_arguments(&self) -> Result<Vec<String>, CommandLineError> {
        let path_mapper = path_mappers::create(
            self,
            self.spawn.get_output_paths_mode(),
            /* is_starlark_action= */ false,
            /* input_metadata_provider= */ None,
        );
        self.spawn
            .get_command_lines()
            .all_arguments_with(path_mapper.as_ref())
    }
}

impl ExecutableAction for JavaHeaderCompileAction {
    /// `SpawnAction.execute` with this class's `afterExecute`.
    // Java: com/google/devtools/build/lib/analysis/actions/SpawnAction.java:278 SpawnAction.execute
    fn execute(
        &self,
        ctx: &dyn ActionExecutionContext,
    ) -> Result<ActionResult, ActionExecutionException> {
        let spawn = self.get_spawn(ctx)?;
        let resolver = get_context::<SpawnStrategyResolver>(ctx).unwrap_or_else(|| {
            panic!("NullPointerException: getContext(SpawnStrategyResolver.class) returned null")
        });
        let result = resolver
            .exec(&spawn, ctx)
            .map_err(|e| to_action_execution_exception(&e, self))?;
        self.after_execute(
            ctx,
            &result,
            blaze_exec::actions::Spawn::get_path_mapper(&spawn),
        );
        Ok(ActionResult::create(result))
    }
}

impl JavaHeaderCompileAction {
    /// `SpawnAction.getSpawn(ctx)`: an `ActionSpawn` over the expanded command lines, the
    /// action's inputs and the param files, with this class's execution info.
    // Java: com/google/devtools/build/lib/analysis/actions/SpawnAction.java:362 SpawnAction.getSpawn
    fn get_spawn(
        &self,
        ctx: &dyn ActionExecutionContext,
    ) -> Result<SimpleSpawn, ActionExecutionException> {
        // Java: PathMappers.create(this, outputPathsMode, this instanceof StarlarkAction,
        // actionExecutionContext.getInputMetadataProvider())
        let metadata =
            blaze_exec::analysis::actions::KeyMetadataAdapter(ctx.get_input_metadata_provider());
        let path_mapper = path_mappers::create(
            self,
            self.spawn.get_output_paths_mode(),
            false,
            Some(&metadata),
        );
        let limits = CommandLineLimits::new(
            self.get_owner()
                .get_build_configuration_info()
                .command_line_limits_max_length as i64,
        );
        let expanded = self
            .spawn
            .get_command_lines()
            .expand(
                Some(&metadata),
                self.get_primary_output().get_exec_path(),
                path_mapper.as_ref(),
                &limits,
            )
            .map_err(|e| self.command_line_exception(&e.to_string()))?;
        let mut inputs = NestedSetBuilder::<ActionInputRef>::new_builder(Order::STABLE_ORDER);
        inputs.add_transitive(&to_action_input_set(&self.get_inputs()));
        inputs.add_all(
            expanded
                .get_param_files()
                .iter()
                .map(|p| ActionInputRef::new(p.clone())),
        );
        let environment = self
            .get_effective_environment(ctx.get_client_env(), path_mapper.as_ref())
            .map_err(|e| self.command_line_exception(&e.message))?;
        let r = self.spawn.get_resource_set_or_builder();
        let mut resources = IndexMap::new();
        resources.insert(ResourceSet::MEMORY.to_string(), r.memory_mb);
        resources.insert(ResourceSet::CPU.to_string(), r.cpu_usage);
        for (k, v) in &r.extra_resource_usage {
            resources.insert(k.clone(), *v);
        }
        Ok(SimpleSpawn::new(
            Arc::new(JavaActionMetadata(ActionMetadataSnapshot::of(self))),
            expanded.arguments().to_vec(),
            environment,
            self.get_execution_info(),
            inputs.build(),
            to_action_input_set(&self.get_tools()),
            self.get_outputs().iter().map(artifact_input).collect(),
            None::<HashSet<ActionInputRef>>,
            Arc::new(ResourceSet::create_from_map(resources, r.local_test_count)),
            SpawnPathMapper::of(path_mapper),
        ))
    }

    // Java: com/google/devtools/build/lib/analysis/actions/SpawnAction.java:299 SpawnAction.createCommandLineException
    fn command_line_exception(&self, message: &str) -> ActionExecutionException {
        use blaze_protos::failure_details::{self, FailureDetail, failure_detail::Category};
        let detail = FailureDetail {
            message: message.to_owned(),
            category: Some(Category::Spawn(failure_details::Spawn {
                code: failure_details::spawn::Code::CommandLineExpansionFailure as i32,
                ..Default::default()
            })),
            ..Default::default()
        };
        ActionExecutionException::with_cause(message, &message, Some(self), false, Some(detail))
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaHeaderCompileAction.java:137 JavaHeaderCompileAction.afterExecute
    fn after_execute(
        &self,
        context: &dyn ActionExecutionContext,
        spawn_results: &[blaze_exec::actions::SpawnResult],
        path_mapper: &SpawnPathMapper,
    ) {
        // Java: Iterables.getOnlyElement(spawnResults)
        if spawn_results.len() != 1 {
            panic!(
                "IllegalArgumentException: expected one element but was: {} spawn results",
                spawn_results.len()
            );
        }
        let spawn_result = &spawn_results[0];
        let output_deps_proto = self.get_outputs()[1].clone();
        if let Ok(full_output_deps) = create_full_output_deps(
            spawn_result,
            &output_deps_proto,
            &self.get_inputs(),
            &self.get_additional_artifacts_for_path_mapping(),
            context,
            path_mapper,
        ) {
            let java_context = get_context::<JavaCompileActionContext>(context);
            if self.insert_dependencies
                && let Some(java_context) = java_context
            {
                java_context.insert_dependencies(&output_deps_proto, full_output_deps);
            }
        }
        // Left empty on IOException. If we cannot read the .jdeps file now, we will read it
        // later or throw an appropriate error then.
    }
}

/// `new JavaHeaderCompileProgressMessage(output, sourceFiles, sourceJars, plugins)`: a
/// `ProgressMessage` with the prefix "Compiling Java headers".
// Java: com/google/devtools/build/lib/rules/java/JavaHeaderCompileAction.java:653 JavaHeaderCompileAction.Builder.JavaHeaderCompileProgressMessage.<init>
// Java: com/google/devtools/build/lib/rules/java/JavaHeaderCompileAction.java:662 JavaHeaderCompileAction.Builder.JavaHeaderCompileProgressMessage.prefix
fn java_header_compile_progress_message(
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

/// Builder for `JavaHeaderCompileAction`.
// Java: com/google/devtools/build/lib/rules/java/JavaHeaderCompileAction.java:173 JavaHeaderCompileAction.Builder
pub struct JavaHeaderCompileActionBuilder<'a> {
    rule_context: &'a RuleContext,
    output_jar: Option<Artifact>,
    header_compilation_output_jar: Option<Artifact>,
    output_deps_proto: Option<Artifact>,
    manifest_output: Option<Artifact>,
    gensrc_output_jar: Option<Artifact>,
    resource_output_jar: Option<Artifact>,
    additional_outputs: IndexSet<Artifact>,
    source_files: IndexSet<Artifact>,
    source_jars: Vec<Artifact>,
    classpath_entries: NestedSet<Artifact>,
    bootclasspath_entries: NestedSet<Artifact>,
    target_label: Option<Label>,
    injecting_rule_kind: Option<String>,
    strict_java_deps: StrictDepsMode,
    direct_jars: NestedSet<Artifact>,
    header_compilation_direct_jars: NestedSet<Artifact>,
    compile_time_dependency_artifacts: NestedSet<Artifact>,
    javac_opts: Vec<String>,
    add_turbine_hjar_javac_opt: bool,
    plugins: JavaPluginData,
    additional_inputs: Vec<Artifact>,
    tools_jars: NestedSet<Artifact>,
    enable_header_compiler_direct: bool,
    enable_direct_classpath: bool,
    exec_group: String,
    utf8_environment: Option<IndexMap<String, String>>,
}

/// `ParamFileInfo.builder(UNQUOTED).build()`.
// Java: com/google/devtools/build/lib/rules/java/JavaHeaderCompileAction.java:175 JavaHeaderCompileAction.Builder.PARAM_FILE_INFO
fn param_file_info() -> ParamFileInfo {
    ParamFileInfo::builder(ParameterFileType::UNQUOTED).build()
}

impl<'a> JavaHeaderCompileActionBuilder<'a> {
    // Java: com/google/devtools/build/lib/rules/java/JavaHeaderCompileAction.java:216 JavaHeaderCompileAction.Builder.<init>
    fn new(rule_context: &'a RuleContext) -> Self {
        JavaHeaderCompileActionBuilder {
            rule_context,
            output_jar: None,
            header_compilation_output_jar: None,
            output_deps_proto: None,
            manifest_output: None,
            gensrc_output_jar: None,
            resource_output_jar: None,
            additional_outputs: IndexSet::new(),
            source_files: IndexSet::new(),
            source_jars: Vec::new(),
            classpath_entries: NestedSetBuilder::empty_set(Order::NAIVE_LINK_ORDER),
            bootclasspath_entries: NestedSetBuilder::empty_set(Order::NAIVE_LINK_ORDER),
            target_label: None,
            injecting_rule_kind: None,
            strict_java_deps: StrictDepsMode::OFF,
            direct_jars: NestedSetBuilder::empty_set(Order::NAIVE_LINK_ORDER),
            header_compilation_direct_jars: NestedSetBuilder::empty_set(Order::NAIVE_LINK_ORDER),
            compile_time_dependency_artifacts: NestedSetBuilder::empty_set(Order::STABLE_ORDER),
            javac_opts: Vec::new(),
            add_turbine_hjar_javac_opt: false,
            plugins: JavaPluginData::empty(),
            additional_inputs: Vec::new(),
            tools_jars: NestedSetBuilder::empty_set(Order::NAIVE_LINK_ORDER),
            enable_header_compiler_direct: true,
            enable_direct_classpath: true,
            exec_group: blaze_packages::rule_class::declared_exec_group::DEFAULT_EXEC_GROUP_NAME
                .to_owned(),
            utf8_environment: None,
        }
    }

    /// Sets the output jdeps file.
    // Java: com/google/devtools/build/lib/rules/java/JavaHeaderCompileAction.java:222 JavaHeaderCompileAction.Builder.setOutputDepsProto
    pub fn set_output_deps_proto(&mut self, output_deps_proto: Artifact) -> &mut Self {
        self.output_deps_proto = Some(output_deps_proto);
        self
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaHeaderCompileAction.java:228 JavaHeaderCompileAction.Builder.setManifestOutput
    pub fn set_manifest_output(&mut self, manifest_output: Option<Artifact>) -> &mut Self {
        self.manifest_output = manifest_output;
        self
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaHeaderCompileAction.java:234 JavaHeaderCompileAction.Builder.setGensrcOutputJar
    pub fn set_gensrc_output_jar(&mut self, gensrc_output_jar: Option<Artifact>) -> &mut Self {
        self.gensrc_output_jar = gensrc_output_jar;
        self
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaHeaderCompileAction.java:240 JavaHeaderCompileAction.Builder.setResourceOutputJar
    pub fn set_resource_output_jar(&mut self, resource_output_jar: Option<Artifact>) -> &mut Self {
        self.resource_output_jar = resource_output_jar;
        self
    }

    /// Sets the direct dependency artifacts.
    // Java: com/google/devtools/build/lib/rules/java/JavaHeaderCompileAction.java:247 JavaHeaderCompileAction.Builder.setDirectJars
    pub fn set_direct_jars(&mut self, direct_jars: NestedSet<Artifact>) -> &mut Self {
        self.direct_jars = direct_jars;
        self
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaHeaderCompileAction.java:254 JavaHeaderCompileAction.Builder.setHeaderCompilationDirectJars
    pub fn set_header_compilation_direct_jars(
        &mut self,
        header_compilation_direct_jars: NestedSet<Artifact>,
    ) -> &mut Self {
        self.header_compilation_direct_jars = header_compilation_direct_jars;
        self
    }

    /// Sets the .jdeps artifacts for direct dependencies.
    // Java: com/google/devtools/build/lib/rules/java/JavaHeaderCompileAction.java:262 JavaHeaderCompileAction.Builder.setCompileTimeDependencyArtifacts
    pub fn set_compile_time_dependency_artifacts(
        &mut self,
        dependency_artifacts: NestedSet<Artifact>,
    ) -> &mut Self {
        self.compile_time_dependency_artifacts = dependency_artifacts;
        self
    }

    /// Sets Java compiler flags.
    // Java: com/google/devtools/build/lib/rules/java/JavaHeaderCompileAction.java:270 JavaHeaderCompileAction.Builder.setJavacOpts
    pub fn set_javac_opts(&mut self, javac_opts: Vec<String>) -> &mut Self {
        self.javac_opts = javac_opts;
        self
    }

    /// Adds `-Aexperimental_turbine_hjar` to Java compiler flags without creating an entirely
    /// new list.
    // Java: com/google/devtools/build/lib/rules/java/JavaHeaderCompileAction.java:280 JavaHeaderCompileAction.Builder.addTurbineHjarJavacOpt
    pub fn add_turbine_hjar_javac_opt(&mut self) -> &mut Self {
        self.add_turbine_hjar_javac_opt = true;
        self
    }

    /// Sets the output jar.
    // Java: com/google/devtools/build/lib/rules/java/JavaHeaderCompileAction.java:287 JavaHeaderCompileAction.Builder.setOutputJar
    pub fn set_output_jar(&mut self, output_jar: Artifact) -> &mut Self {
        self.output_jar = Some(output_jar);
        self
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaHeaderCompileAction.java:294 JavaHeaderCompileAction.Builder.setHeaderCompilationOutputJar
    pub fn set_header_compilation_output_jar(
        &mut self,
        header_compilation_output_jar: Artifact,
    ) -> &mut Self {
        self.header_compilation_output_jar = Some(header_compilation_output_jar);
        self
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaHeaderCompileAction.java:301 JavaHeaderCompileAction.Builder.setAdditionalOutputs
    pub fn set_additional_outputs(&mut self, outputs: IndexSet<Artifact>) -> &mut Self {
        self.additional_outputs = outputs;
        self
    }

    /// Adds Java source files to compile.
    // Java: com/google/devtools/build/lib/rules/java/JavaHeaderCompileAction.java:309 JavaHeaderCompileAction.Builder.setSourceFiles
    pub fn set_source_files(&mut self, source_files: IndexSet<Artifact>) -> &mut Self {
        self.source_files = source_files;
        self
    }

    /// Adds a jar archive of Java sources to compile.
    // Java: com/google/devtools/build/lib/rules/java/JavaHeaderCompileAction.java:317 JavaHeaderCompileAction.Builder.setSourceJars
    pub fn set_source_jars(&mut self, source_jars: Vec<Artifact>) -> &mut Self {
        self.source_jars = source_jars;
        self
    }

    /// Sets the compilation classpath entries.
    // Java: com/google/devtools/build/lib/rules/java/JavaHeaderCompileAction.java:325 JavaHeaderCompileAction.Builder.setClasspathEntries
    pub fn set_classpath_entries(&mut self, classpath_entries: NestedSet<Artifact>) -> &mut Self {
        self.classpath_entries = classpath_entries;
        self
    }

    /// Sets the compilation bootclasspath entries.
    // Java: com/google/devtools/build/lib/rules/java/JavaHeaderCompileAction.java:333 JavaHeaderCompileAction.Builder.setBootclasspathEntries
    pub fn set_bootclasspath_entries(
        &mut self,
        bootclasspath_entries: NestedSet<Artifact>,
    ) -> &mut Self {
        self.bootclasspath_entries = bootclasspath_entries;
        self
    }

    /// Sets the annotation processors classpath entries.
    // Java: com/google/devtools/build/lib/rules/java/JavaHeaderCompileAction.java:341 JavaHeaderCompileAction.Builder.setPlugins
    pub fn set_plugins(&mut self, plugins: JavaPluginData) -> &mut Self {
        blaze_util::check_state!(self.plugins.is_empty());
        self.plugins = plugins;
        self
    }

    /// Sets the label of the target being compiled.
    // Java: com/google/devtools/build/lib/rules/java/JavaHeaderCompileAction.java:350 JavaHeaderCompileAction.Builder.setTargetLabel
    pub fn set_target_label(&mut self, target_label: Option<Label>) -> &mut Self {
        self.target_label = target_label;
        self
    }

    /// Sets the injecting rule kind of the target being compiled.
    // Java: com/google/devtools/build/lib/rules/java/JavaHeaderCompileAction.java:357 JavaHeaderCompileAction.Builder.setInjectingRuleKind
    pub fn set_injecting_rule_kind(&mut self, injecting_rule_kind: Option<String>) -> &mut Self {
        self.injecting_rule_kind = injecting_rule_kind;
        self
    }

    /// Sets the Strict Java Deps mode.
    // Java: com/google/devtools/build/lib/rules/java/JavaHeaderCompileAction.java:364 JavaHeaderCompileAction.Builder.setStrictJavaDeps
    pub fn set_strict_java_deps(&mut self, strict_java_deps: StrictDepsMode) -> &mut Self {
        self.strict_java_deps = strict_java_deps;
        self
    }

    /// Sets additional inputs, e.g. for databinding support.
    // Java: com/google/devtools/build/lib/rules/java/JavaHeaderCompileAction.java:372 JavaHeaderCompileAction.Builder.setAdditionalInputs
    pub fn set_additional_inputs(&mut self, additional_inputs: Vec<Artifact>) -> &mut Self {
        self.additional_inputs = additional_inputs;
        self
    }

    /// Sets the tools jars.
    // Java: com/google/devtools/build/lib/rules/java/JavaHeaderCompileAction.java:380 JavaHeaderCompileAction.Builder.setToolsJars
    pub fn set_tools_jars(&mut self, tools_jars: NestedSet<Artifact>) -> &mut Self {
        self.tools_jars = tools_jars;
        self
    }

    /// Sets the exec group used for selecting execution platform of `JavaHeaderCompileAction`.
    // Java: com/google/devtools/build/lib/rules/java/JavaHeaderCompileAction.java:388 JavaHeaderCompileAction.Builder.setExecGroup
    pub fn set_exec_group(&mut self, exec_group: String) -> &mut Self {
        self.exec_group = exec_group;
        self
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaHeaderCompileAction.java:395 JavaHeaderCompileAction.Builder.enableHeaderCompilerDirect
    pub fn enable_header_compiler_direct(
        &mut self,
        enable_header_compiler_direct: bool,
    ) -> &mut Self {
        self.enable_header_compiler_direct = enable_header_compiler_direct;
        self
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaHeaderCompileAction.java:401 JavaHeaderCompileAction.Builder.enableDirectClasspath
    pub fn enable_direct_classpath(&mut self, enable_direct_classpath: bool) -> &mut Self {
        self.enable_direct_classpath = enable_direct_classpath;
        self
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaHeaderCompileAction.java:407 JavaHeaderCompileAction.Builder.setUtf8Environment
    pub fn set_utf8_environment(
        &mut self,
        utf8_environment: IndexMap<String, String>,
    ) -> &mut Self {
        self.utf8_environment = Some(utf8_environment);
        self
    }

    /// Builds and registers the action for a header compilation.
    // Java: com/google/devtools/build/lib/rules/java/JavaHeaderCompileAction.java:414 JavaHeaderCompileAction.Builder.build
    pub fn build(
        &mut self,
        eval: &starlark::eval::Evaluator,
        java_toolchain: &JavaToolchainProvider<'_>,
    ) -> starlark::Result<()> {
        let output_deps_proto = self
            .output_deps_proto
            .clone()
            .unwrap_or_else(|| panic!("NullPointerException: outputDepsProto must not be null"));
        let utf8_environment = self
            .utf8_environment
            .clone()
            .unwrap_or_else(|| panic!("NullPointerException: utf8Environment must not be null"));
        let output_jar = self
            .output_jar
            .clone()
            .unwrap_or_else(|| panic!("NullPointerException: outputJar must not be null"));

        // Invariant: if strictJavaDeps is OFF, then directJars and
        // dependencyArtifacts are ignored
        if self.strict_java_deps == StrictDepsMode::OFF {
            self.direct_jars = NestedSetBuilder::empty_set(Order::NAIVE_LINK_ORDER);
            self.compile_time_dependency_artifacts =
                NestedSetBuilder::empty_set(Order::STABLE_ORDER);
        }

        // Enable the direct classpath optimization if there are no annotation processors.
        // N.B. we only check if the processor classes are empty, we don't care if there is
        // plugin data or dependencies if there are no annotation processors to run. This differs
        // from javac where java_plugin may be used with processor_class unset to declare Error
        // Prone plugins.
        let use_direct_classpath =
            self.enable_direct_classpath && self.plugins.processor_classes().is_empty();

        // Use the optimized 'direct' implementation if it is available, and either there are no
        // annotation processors or they are built in to the tool and listed in
        // java_toolchain.header_compiler_direct_processors.
        let processor_classes: IndexSet<String> = self.plugins.processor_classes().to_set();
        let use_header_compiler_direct = self.enable_header_compiler_direct
            && java_toolchain.get_header_compiler_direct()?.is_some()
            && {
                let builtin = java_toolchain.get_header_compiler_builtin_processors()?;
                processor_classes.iter().all(|p| builtin.contains(p))
            };
        let configuration = self.rule_context.get_configuration().clone();
        let java_configuration = configuration
            .get_fragment::<blaze_build_config::fragments::java_configuration::JavaConfiguration>()
            .unwrap_or_else(|| panic!("NullPointerException: JavaConfiguration fragment"));
        let mut classpath_mode = java_configuration.get_reduce_java_classpath();
        let incompatible = java_toolchain.get_reduced_classpath_incompatible_processors()?;
        if processor_classes.iter().any(|p| incompatible.contains(p)) {
            classpath_mode = JavaClasspathMode::OFF;
        }
        let action_environment =
            Arc::new(blaze_analysis::rule::action_environments::to_actions_env(
                configuration.get_action_environment(),
            ))
            .with_additional_fixed_variables(utf8_environment.clone());
        let progress_message = java_header_compile_progress_message(
            &output_jar,
            &self.source_files,
            &self.source_jars,
            &self.plugins,
        );
        let mut outputs: IndexSet<Artifact> = IndexSet::new();
        outputs.insert(output_jar.clone());
        outputs.insert(output_deps_proto.clone());
        outputs.extend(self.additional_outputs.iter().cloned());
        for a in [
            &self.gensrc_output_jar,
            &self.resource_output_jar,
            &self.manifest_output,
            &self.header_compilation_output_jar,
        ]
        .into_iter()
        .flatten()
        {
            outputs.insert(a.clone());
        }
        let outputs: Vec<Artifact> = outputs.into_iter().collect();

        let mut mandatory_inputs_builder = NestedSetBuilder::<Artifact>::stable_order();
        mandatory_inputs_builder
            .add_all(self.additional_inputs.iter().cloned())
            .add_transitive(&self.bootclasspath_entries)
            .add_all(self.source_jars.iter().cloned())
            .add_all(self.source_files.iter().cloned())
            .add_transitive(&self.tools_jars);

        let header_compiler = if use_header_compiler_direct {
            java_toolchain.get_header_compiler_direct()?
        } else {
            java_toolchain.get_header_compiler()?
        }
        .unwrap_or_else(|| panic!("NullPointerException: header compiler"));

        // The header compiler is either a jar file that needs to be executed using
        // `java -jar <path>`, or an executable that can be run directly.
        header_compiler.add_inputs(java_toolchain, &mut mandatory_inputs_builder)?;
        let mut command_line = CustomCommandLine::builder();
        command_line
            .add_exec_path_with_arg("--output", Some(output_jar.clone()))
            .add_exec_path_with_arg(
                "--header_compilation_output",
                self.header_compilation_output_jar.clone(),
            )
            .add_exec_path_with_arg("--gensrc_output", self.gensrc_output_jar.clone())
            .add_exec_path_with_arg("--resource_output", self.resource_output_jar.clone())
            .add_exec_path_with_arg("--output_manifest_proto", self.manifest_output.clone())
            .add_exec_path_with_arg("--output_deps", Some(output_deps_proto.clone()))
            .add_exec_paths_nested_set_with_arg(
                "--bootclasspath",
                self.bootclasspath_entries.clone(),
            )
            .add_exec_paths_with_arg("--sources", self.source_files.iter().cloned())
            .add_exec_paths_with_arg("--source_jars", self.source_jars.iter().cloned())
            .add_with_arg("--injecting_rule_kind", self.injecting_rule_kind.clone());

        if !self.javac_opts.is_empty() || self.add_turbine_hjar_javac_opt {
            command_line.add("--javacopts");
            if !self.javac_opts.is_empty() {
                command_line.add_all(self.javac_opts.iter().cloned());
            }
            if self.add_turbine_hjar_javac_opt {
                command_line.add("-Aexperimental_turbine_hjar");
            }
            // terminate --javacopts with `--` to support javac flags that start with `--`
            command_line.add("--");
        }

        if let Some(target_label) = &self.target_label {
            command_line.add("--target_label");
            if target_label.get_repository().is_main() {
                command_line.add_label(target_label.clone());
            } else {
                // @-prefixed strings will be assumed to be params filenames and expanded,
                // so add an extra @ to escape it.
                let main_repo_mapping = self
                    .rule_context
                    .get_analysis_environment()
                    .get_main_repo_mapping()
                    .map_err(|e| crate::cpp::util::errorf(e.to_string()))?
                    .clone();
                command_line.add_prefixed_label("@", target_label.clone(), Some(main_repo_mapping));
            }
        }

        let mut execution_info: IndexMap<String, String> = IndexMap::new();
        let mut path_mapping = IndexMap::new();
        path_mapping.insert(SUPPORTS_PATH_MAPPING.to_owned(), "1".to_owned());
        for (k, v) in modified_execution_info(&configuration, &path_mapping, JAVAC_MNEMONIC) {
            execution_info.insert(k, v);
        }
        for (k, v) in target_utils::get_execution_info_with_propagation(
            self.rule_context.get_rule(),
            self.rule_context.is_allow_tags_propagation(),
        ) {
            execution_info.insert(k, v);
        }
        // Java: executionInfo.buildKeepingLast() (IndexMap::insert keeps the first position).

        let action_owner = if self.rule_context.use_auto_exec_groups() {
            self.rule_context
                .get_action_owner_for(&self.exec_group)
                .unwrap_or_else(|| {
                    panic!("NullPointerException: action owner of {}", self.exec_group)
                })
        } else {
            self.rule_context.get_action_owner()
        };

        if use_direct_classpath {
            let classpath: NestedSet<Artifact>;
            let additional_artifacts_for_path_mapping: NestedSet<Artifact>;
            if !self.header_compilation_direct_jars.is_empty() || self.classpath_entries.is_empty()
            {
                classpath = self.header_compilation_direct_jars.clone();
                // When using the direct classpath optimization, Turbine generates .jdeps entries
                // based on the transitive dependency information packages into
                // META-INF/TRANSITIVE. When path mapping is used, these entries may have been
                // subject to it when they were generated. Since the contents of that directory
                // are not unmapped, we need to instead unmap the paths emitted in the .jdeps
                // file, which requires knowing the full list of artifact paths even if they
                // aren't inputs to the current action.
                // https://github.com/google/turbine/commit/f9f2decee04a3c651671f7488a7c9d7952df88c8
                additional_artifacts_for_path_mapping = self.classpath_entries.clone();
            } else {
                classpath = self.classpath_entries.clone();
                additional_artifacts_for_path_mapping =
                    NestedSetBuilder::empty_set(Order::STABLE_ORDER);
            }
            mandatory_inputs_builder.add_transitive(&classpath);

            command_line.add_exec_paths_nested_set_with_arg("--classpath", classpath);
            command_line.add("--reduce_classpath_mode").add("NONE");

            let all_inputs = mandatory_inputs_builder.build();
            let executable_line = header_compiler.get_command_line(java_toolchain)?;
            let mut command_lines = CommandLines::builder();
            command_lines
                .add_command_line(Arc::new(executable_line))
                .add_command_line_with_param_file(
                    Arc::new(command_line.build()),
                    Some(param_file_info()),
                );
            let action = JavaHeaderCompileAction::new(
                action_owner,
                /* tools= */ NestedSetBuilder::empty_set(Order::STABLE_ORDER),
                all_inputs,
                outputs,
                /* resource_set= */ AnalysisResourceSet::default_resource_set(),
                command_lines.build(),
                action_environment,
                &modified_execution_info(
                    &configuration,
                    &execution_info,
                    DIRECT_CLASSPATH_MNEMONIC,
                ),
                progress_message.to_string_java(),
                DIRECT_CLASSPATH_MNEMONIC,
                get_output_paths_mode(&configuration),
                // If classPathMode == BAZEL, also make sure to inject the dependencies to be
                // available to downstream actions. Else just do enough work to locally create
                // the full .jdeps from the .stripped .jdeps produced on the executor.
                /* insert_dependencies= */
                classpath_mode == JavaClasspathMode::BAZEL
                    || classpath_mode == JavaClasspathMode::BAZEL_NO_FALLBACK,
                java_configuration.inmemory_jdeps_files(),
                additional_artifacts_for_path_mapping,
            );
            register_action(eval, Arc::new(action))?;
            return Ok(());
        }

        // If we get here the action requires annotation processing, so add additional inputs
        // and flags needed for the javac-based header compiler implementations that supports
        // annotation processing.
        if !use_header_compiler_direct {
            mandatory_inputs_builder.add_transitive(self.plugins.processor_classpath());
            mandatory_inputs_builder.add_transitive(self.plugins.data());
        }

        // Java: Sets.intersection(processorClasses, builtinProcessors): the first set's order.
        let builtin = java_toolchain.get_header_compiler_builtin_processors()?;
        command_line.add_all_with_arg(
            "--builtin_processors",
            processor_classes
                .iter()
                .filter(|p| builtin.contains(*p))
                .cloned()
                .collect::<Vec<_>>(),
        );
        command_line
            .add_all_nested_set_with_arg("--processors", self.plugins.processor_classes().clone());
        if !use_header_compiler_direct {
            command_line.add_exec_paths_nested_set_with_arg(
                "--processorpath",
                self.plugins.processor_classpath().clone(),
            );
        }
        if self.strict_java_deps != StrictDepsMode::OFF {
            command_line.add_exec_paths_nested_set_with_arg(
                "--direct_dependencies",
                self.direct_jars.clone(),
            );
        }

        let mandatory_inputs = mandatory_inputs_builder.build();
        let executable_line = header_compiler.get_command_line(java_toolchain)?;

        let action = JavaCompileAction::new(
            CompilationType::TURBINE,
            action_owner,
            /* tools= */ self.tools_jars.clone(),
            progress_message,
            mandatory_inputs,
            /* transitive_inputs= */ self.classpath_entries.clone(),
            self.direct_jars.clone(),
            outputs,
            action_environment,
            execution_info,
            Arc::new(executable_line),
            /* flag_line= */ Arc::new(command_line.build()),
            /* extra_command_line_args= */ Vec::new(),
            configuration,
            /* dependency_artifacts= */ self.compile_time_dependency_artifacts.clone(),
            Some(output_deps_proto),
            classpath_mode,
        );
        register_action(eval, Arc::new(action))?;
        Ok(())
    }
}

/// `JavaCompileActionBuilder.isJavaCompileAction`-style test for the header action.
pub fn is_java_header_compile_action(action: &dyn Action) -> bool {
    action.as_any().is::<JavaHeaderCompileAction>()
}
