//! `rules/java/JavaCompileAction.java`: action that represents a Java compilation (and the
//! javac-based header compilation, mnemonic `JavacTurbine`), with its execution half: the
//! reduced classpath of `--experimental_java_classpath=bazel` read from the `.jdeps` files of
//! the direct dependencies, and the fallback re-execution with the full classpath.

use std::any::Any;
use std::collections::HashSet;
use std::io::Read;
use std::sync::Arc;

use blaze_actions::Artifact;
use blaze_actions::actions::action::{
    AbstractAction, Action, ActionApiError, CommandAction, CommandLineError, ComputeKeyError,
    InputMetadataProvider,
};
use blaze_actions::actions::action_environment::ActionEnvironment;
use blaze_actions::actions::action_key_context::ActionKeyContext;
use blaze_actions::actions::action_owner::ActionOwner;
use blaze_actions::actions::command_line::CommandLineRef;
use blaze_actions::actions::command_line_limits::CommandLineLimits;
use blaze_actions::actions::command_lines::CommandLines;
use blaze_actions::actions::param_file_info::ParamFileInfo;
use blaze_actions::actions::parameter_file::ParameterFileType;
use blaze_actions::actions::path_mapper::OutputPathsMode;
use blaze_actions::analysis::actions::custom_command_line::CustomCommandLine;
use blaze_actions::analysis::actions::path_mappers;
use blaze_build_config::build_configuration_value::BuildConfigurationValue;
use blaze_collect::{NestedSet, NestedSetBuilder, Order};
use blaze_exec::actions::action_adapters::{ActionMetadataSnapshot, artifact_input};
use blaze_exec::actions::action_execution_metadata::ActionExecutionMetadata;
use blaze_exec::actions::execution_requirements::REMOTE_EXECUTION_INLINE_OUTPUTS;
use blaze_exec::actions::{
    ActionExecutionException, ActionInputRef, ActionResult, EnvironmentalExecException,
    ExecException, ExecutableAction, PathMapper as SpawnPathMapper, ResourceSet, SimpleSpawn,
    SpawnResult,
};
use blaze_exec::analysis::actions::to_action_input_set;
use blaze_exec::exec::action_execution_context::{ActionExecutionContext, get_context};
use blaze_exec::exec::spawn_strategy_resolver::SpawnStrategyResolver;
use blaze_optdefs::fragment::java_configuration::JavaConfigurationJavaClasspathMode as JavaClasspathMode;
use blaze_options::converters::JavaEnum;
use blaze_protos::failure_details::{self, FailureDetail, failure_detail::Category};
use blaze_util::{Fingerprint, check_state};
use indexmap::IndexMap;

use super::java_compile_action_context::{Dependencies, JavaCompileActionContext};
use super::java_plugin_info::JavaPluginData;

/// `UUID.fromString("e423747c-2827-49e6-b961-f6c08c10bb51")`: (least, most) significant bits.
// Java: com/google/devtools/build/lib/rules/java/JavaCompileAction.java:99 JavaCompileAction.GUID
const GUID_LEAST: u64 = 0xb961_f6c0_8c10_bb51;
const GUID_MOST: u64 = 0xe423_747c_2827_49e6;

/// `ParamFileInfo.builder(UNQUOTED).setUseAlways(true).build()`.
// Java: com/google/devtools/build/lib/rules/java/JavaCompileAction.java:101 JavaCompileAction.PARAM_FILE_INFO
fn param_file_info() -> ParamFileInfo {
    ParamFileInfo::builder(ParameterFileType::UNQUOTED)
        .set_use_always(true)
        .build()
}

/// `ResourceSet.createWithRamCpu(750, 1)`.
// Java: com/google/devtools/build/lib/rules/java/JavaCompileAction.java:97 JavaCompileAction.LOCAL_RESOURCES
fn local_resources() -> ResourceSet {
    ResourceSet::create_with_ram_cpu(/* memory_mb= */ 750.0, /* cpu= */ 1.0)
}

/// `Enum.ordinal()` of the classpath mode.
fn classpath_mode_ordinal(mode: JavaClasspathMode) -> i32 {
    JavaClasspathMode::values()
        .iter()
        .position(|m| *m == mode)
        .expect("enum constant") as i32
}

// Java: com/google/devtools/build/lib/rules/java/JavaCompileAction.java:104 JavaCompileAction.CompilationType
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CompilationType {
    JAVAC,
    /// 'javac turbine' has been replaced by just 'turbine', but the mnemonic is unchanged for
    /// continuity in the blaze performance logs, and to distinguish direct classpath actions
    /// which use the 'Turbine' mnemonic.
    TURBINE,
}

impl CompilationType {
    // Java: com/google/devtools/build/lib/rules/java/JavaCompileAction.java:111 JavaCompileAction.CompilationType.mnemonic
    pub fn mnemonic(self) -> &'static str {
        match self {
            CompilationType::JAVAC => "Javac",
            CompilationType::TURBINE => "JavacTurbine",
        }
    }
}

/// `JavaCompileAction.ReducedClasspath`: the reduced jars and the lengths of the reduced and
/// full classpaths.
// Java: com/google/devtools/build/lib/rules/java/JavaCompileAction.java:294 JavaCompileAction.ReducedClasspath
#[allow(dead_code)]
pub struct ReducedClasspath {
    pub reduced_jars: NestedSet<Artifact>,
    pub reduced_length: usize,
    pub full_length: usize,
}

impl ReducedClasspath {
    // Java: com/google/devtools/build/lib/rules/java/JavaCompileAction.java:299 JavaCompileAction.ReducedClasspath.<init>
    pub fn new(reduced_jars: Vec<Artifact>, full_length: usize) -> ReducedClasspath {
        let reduced_length = reduced_jars.len();
        ReducedClasspath {
            reduced_jars: NestedSetBuilder::wrap(Order::STABLE_ORDER, reduced_jars),
            reduced_length,
            full_length,
        }
    }
}

/// The progress message of a Java compilation (`JavaCompileAction.ProgressMessage` with the
/// prefix of its subclass).
// Java: com/google/devtools/build/lib/rules/java/JavaCompileAction.java:518 JavaCompileAction.ProgressMessage
#[derive(Clone, Debug)]
pub struct ProgressMessage {
    prefix: &'static str,
    output: Artifact,
    source_files: usize,
    source_jars: usize,
    processor_classes: NestedSet<String>,
}

impl ProgressMessage {
    // Java: com/google/devtools/build/lib/rules/java/JavaCompileAction.java:536 JavaCompileAction.ProgressMessage.prefix
    pub fn prefix(&self) -> &'static str {
        self.prefix
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaCompileAction.java:525 JavaCompileAction.ProgressMessage.<init>
    pub fn new(
        prefix: &'static str,
        output: &Artifact,
        source_files: usize,
        source_jars: usize,
        plugins: &JavaPluginData,
    ) -> ProgressMessage {
        ProgressMessage {
            prefix,
            output: output.clone(),
            source_files,
            source_jars,
            processor_classes: plugins.processor_classes().clone(),
        }
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaCompileAction.java:539 JavaCompileAction.ProgressMessage.toString
    pub fn to_string_java(&self) -> String {
        let mut sb = String::from(self.prefix);
        sb.push(' ');
        sb.push_str(&self.output.pretty_print());
        sb.push_str(" (");
        let mut first = true;
        first = Self::append_count(&mut sb, first, self.source_files, "source file");
        Self::append_count(&mut sb, first, self.source_jars, "source jar");
        sb.push(')');
        Self::append_processor_names(&mut sb, &self.processor_classes);
        sb
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaCompileAction.java:548 JavaCompileAction.ProgressMessage.appendProcessorNames
    fn append_processor_names(sb: &mut String, processor_classes: &NestedSet<String>) {
        if processor_classes.is_empty() {
            return;
        }
        let mut short_names = Vec::new();
        for name in processor_classes.to_list().iter() {
            // Annotation processor names are qualified class names. Omit the package part for
            // the progress message, e.g. `com.google.Foo` -> `Foo`.
            let short_name = match name.rfind('.') {
                Some(idx) => &name[idx + 1..],
                None => name.as_str(),
            };
            short_names.push(short_name.to_owned());
        }
        sb.push_str(" and running annotation processors (");
        sb.push_str(&short_names.join(", "));
        sb.push(')');
    }

    /// Append an input count to the progress message, e.g. "2 source jars". If an input count
    /// has already been appended, prefix with ", ".
    // Java: com/google/devtools/build/lib/rules/java/JavaCompileAction.java:545 JavaCompileAction.ProgressMessage.appendCount
    fn append_count(sb: &mut String, mut first: bool, count: usize, name: &str) -> bool {
        if count > 0 {
            if !first {
                sb.push_str(", ");
            } else {
                first = false;
            }
            sb.push_str(&count.to_string());
            sb.push(' ');
            sb.push_str(name);
            if count > 1 {
                sb.push('s');
            }
        }
        first
    }
}

/// Action that represents a Java compilation.
// Java: com/google/devtools/build/lib/rules/java/JavaCompileAction.java:96 JavaCompileAction
pub struct JavaCompileAction {
    base: AbstractAction,
    tools: NestedSet<Artifact>,
    compilation_type: CompilationType,
    env: Arc<ActionEnvironment>,
    execution_info: IndexMap<String, String>,
    executable_line: CommandLineRef,
    flag_line: CommandLineRef,
    extra_command_line_args: Vec<CommandLineRef>,
    configuration: Arc<BuildConfigurationValue>,
    progress_message: ProgressMessage,

    direct_jars: NestedSet<Artifact>,
    mandatory_inputs: NestedSet<Artifact>,
    transitive_inputs: NestedSet<Artifact>,
    dependency_artifacts: NestedSet<Artifact>,
    output_deps_proto: Option<Artifact>,
    classpath_mode: JavaClasspathMode,
    // PORT: `extraActionInfoSupplier` feeds `getExtraActionInfo` (extra actions), which Blaze's
    // `Action` does not have; the supplier is not kept.
}

impl std::fmt::Debug for JavaCompileAction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.to_string_java())
    }
}

/// `configuration.getFragment(JavaConfiguration.class).inmemoryJdepsFiles()`.
fn inmemory_jdeps_files(configuration: &BuildConfigurationValue) -> bool {
    configuration
        .get_fragment::<blaze_build_config::fragments::java_configuration::JavaConfiguration>()
        .unwrap_or_else(|| panic!("NullPointerException: JavaConfiguration fragment"))
        .inmemory_jdeps_files()
}

/// `PathMappers.getOutputPathsMode(configuration)`.
// Java: com/google/devtools/build/lib/analysis/actions/PathMappers.java:145 PathMappers.getOutputPathsMode
pub(crate) fn get_output_paths_mode(
    configuration: &Arc<BuildConfigurationValue>,
) -> OutputPathsMode {
    use blaze_actions::analysis::actions::spawn_action::SpawnActionConfiguration;
    blaze_analysis::rule::config_util::ConfigurationSpawnActionConfiguration(configuration.clone())
        .get_output_paths_mode()
}

/// `configuration.modifiedExecutionInfo(executionInfo, mnemonic)`.
// Java: com/google/devtools/build/lib/analysis/config/BuildConfigurationValue.java:926 BuildConfigurationValue.modifiedExecutionInfo
pub(crate) fn modified_execution_info(
    configuration: &Arc<BuildConfigurationValue>,
    execution_info: &IndexMap<String, String>,
    mnemonic: &str,
) -> IndexMap<String, String> {
    use blaze_actions::analysis::actions::spawn_action::SpawnActionConfiguration;
    blaze_analysis::rule::config_util::ConfigurationSpawnActionConfiguration(configuration.clone())
        .modified_execution_info(execution_info, mnemonic)
}

/// `ActionAnalysisMetadata.mergeMaps(first, second)`.
// Java: com/google/devtools/build/lib/actions/ActionAnalysisMetadata.java:252 ActionAnalysisMetadata.mergeMaps
pub(crate) fn merge_maps(
    first: IndexMap<String, String>,
    second: IndexMap<String, String>,
) -> IndexMap<String, String> {
    if first.is_empty() {
        return second;
    }
    if second.is_empty() {
        return first;
    }
    // ImmutableMap.Builder.putAll(first).putAll(second).buildKeepingLast(): first position,
    // last value.
    let mut result = first;
    for (k, v) in second {
        result.insert(k, v);
    }
    result
}

impl JavaCompileAction {
    // Java: com/google/devtools/build/lib/rules/java/JavaCompileAction.java:715 JavaCompileAction.getIncompleteEnvironmentForTesting
    pub fn get_incomplete_environment_for_testing(&self) -> IndexMap<String, String> {
        self.get_environment().get_fixed_env()
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaCompileAction.java:721 JavaCompileAction.getPossibleInputsForTesting
    pub fn get_possible_inputs_for_testing(&self) -> Option<NestedSet<Artifact>> {
        None
    }

    /// `getExtraActionInfo(actionKeyContext)`: the `ExtraActionInfo` of extra actions, extended
    /// with the `JavaCompileInfo` of the action's `ExtraActionInfoSupplier`.
    // Java: com/google/devtools/build/lib/rules/java/JavaCompileAction.java:590 JavaCompileAction.getExtraActionInfo
    // Java: com/google/devtools/build/lib/rules/java/JavaCompileAction.java:291 JavaCompileAction.ExtraActionInfoSupplier.extend
    pub fn get_extra_action_info(&self) -> ! {
        // PORT: extra actions (`extra_action`, `action_listener`, the `ExtraActionInfo` /
        // `JavaCompileInfo` protos of extra_actions_base.proto) are not ported; the supplier's
        // data is not kept (see JavaCompileActionBuilder.JavaCompileExtraActionInfoSupplier).
        blaze_util::unported!(
            "JavaCompileAction.getExtraActionInfo",
            "extra actions (ExtraActionInfo) are not ported"
        )
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaCompileAction.java:138 JavaCompileAction.<init>
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        compilation_type: CompilationType,
        owner: ActionOwner,
        tools: NestedSet<Artifact>,
        progress_message: ProgressMessage,
        mandatory_inputs: NestedSet<Artifact>,
        transitive_inputs: NestedSet<Artifact>,
        direct_jars: NestedSet<Artifact>,
        outputs: Vec<Artifact>,
        env: Arc<ActionEnvironment>,
        execution_info: IndexMap<String, String>,
        executable_line: CommandLineRef,
        flag_line: CommandLineRef,
        extra_command_line_args: Vec<CommandLineRef>,
        configuration: Arc<BuildConfigurationValue>,
        dependency_artifacts: NestedSet<Artifact>,
        output_deps_proto: Option<Artifact>,
        classpath_mode: JavaClasspathMode,
    ) -> JavaCompileAction {
        if outputs.iter().any(|o| o.is_tree_artifact()) {
            panic!(
                "IllegalArgumentException: Unexpected tree artifact output(s): [{}] in \
                 JavaCompileAction for {}",
                outputs
                    .iter()
                    .filter(|o| o.is_tree_artifact())
                    .map(|o| o.get_exec_path_string().to_string())
                    .collect::<Vec<_>>()
                    .join(","),
                owner
                    .get_label()
                    .map(|l| l.to_string())
                    .unwrap_or_else(|| "null".to_owned())
            );
        }
        let base = AbstractAction::new(
            owner,
            Self::all_inputs(&mandatory_inputs, &transitive_inputs, &dependency_artifacts),
            outputs,
        );
        let execution_info =
            modified_execution_info(&configuration, &execution_info, compilation_type.mnemonic());
        let action = JavaCompileAction {
            base,
            tools,
            compilation_type,
            env,
            execution_info,
            executable_line,
            flag_line,
            extra_command_line_args,
            configuration,
            progress_message,
            direct_jars,
            mandatory_inputs,
            transitive_inputs,
            dependency_artifacts,
            output_deps_proto,
            classpath_mode,
        };
        check_state!(
            action.output_deps_proto.is_some()
                || (action.classpath_mode != JavaClasspathMode::BAZEL
                    && action.classpath_mode != JavaClasspathMode::BAZEL_NO_FALLBACK),
            "Cannot have null outputDepsProto with reduced class path mode BAZEL {}",
            action.describe()
        );
        action
    }

    /// Computes all of a JavaCompileAction's inputs.
    // Java: com/google/devtools/build/lib/rules/java/JavaCompileAction.java:157 JavaCompileAction.allInputs
    pub fn all_inputs(
        mandatory_inputs: &NestedSet<Artifact>,
        transitive_inputs: &NestedSet<Artifact>,
        dependency_artifacts: &NestedSet<Artifact>,
    ) -> NestedSet<Artifact> {
        let mut b = NestedSetBuilder::stable_order();
        b.add_transitive(mandatory_inputs)
            .add_transitive(transitive_inputs)
            .add_transitive(dependency_artifacts);
        b.build()
    }

    pub fn get_compilation_type(&self) -> CompilationType {
        self.compilation_type
    }

    pub fn get_classpath_mode(&self) -> JavaClasspathMode {
        self.classpath_mode
    }

    pub fn get_output_deps_proto(&self) -> Option<&Artifact> {
        self.output_deps_proto.as_ref()
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaCompileAction.java:648 JavaCompileAction.getCommandLines
    pub fn get_command_lines(&self) -> CommandLines {
        let mut builder = CommandLines::builder();
        builder
            .add_command_line(self.executable_line.clone())
            .add_command_line_with_param_file(self.flag_line.clone(), Some(param_file_info()))
            .add_command_line_with_param_file(
                self.get_full_classpath_line(),
                Some(param_file_info()),
            );
        for extra_command_line in &self.extra_command_line_args {
            builder.add_command_line_with_param_file(
                extra_command_line.clone(),
                Some(param_file_info()),
            );
        }
        builder.build()
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaCompileAction.java:660 JavaCompileAction.getFullClasspathLine
    fn get_full_classpath_line(&self) -> CommandLineRef {
        let mut classpath_line = CustomCommandLine::builder();
        classpath_line
            .add_exec_paths_nested_set_with_arg("--classpath", self.transitive_inputs.clone());
        if self.classpath_mode == JavaClasspathMode::JAVABUILDER {
            classpath_line
                .add("--reduce_classpath_mode")
                .add("JAVABUILDER_REDUCED");
            if !self.dependency_artifacts.is_empty() {
                classpath_line.add_exec_paths_nested_set_with_arg(
                    "--deps_artifacts",
                    self.dependency_artifacts.clone(),
                );
            }
        }
        Arc::new(classpath_line.build())
    }

    fn command_line_limits(&self) -> CommandLineLimits {
        CommandLineLimits::new(self.configuration.get_command_line_limits_max_length() as i64)
    }

    /// Compute a reduced classpath that is comprised of the header jars of all the direct
    /// dependencies and the jars needed to build those (read from the produced .jdeps file).
    /// This duplicates the logic from
    /// `com.google.devtools.build.buildjar.javac.plugins.dependency.DependencyModule#computeStrictClasspath`.
    // Java: com/google/devtools/build/lib/rules/java/JavaCompileAction.java:263 JavaCompileAction.getReducedClasspath
    pub fn get_reduced_classpath(
        &self,
        action_execution_context: &dyn ActionExecutionContext,
        context: &JavaCompileActionContext,
    ) -> std::io::Result<NestedSet<Artifact>> {
        let mut direct: HashSet<String> = HashSet::new();
        for direct_jar in self.direct_jars.to_list().iter() {
            direct.insert(direct_jar.get_exec_path_string().to_string());
        }
        for dep_artifact in self.dependency_artifacts.to_list().iter() {
            for dep in &context
                .get_dependencies(dep_artifact, action_execution_context)?
                .dependency
            {
                direct.insert(dep.get_path());
            }
        }
        let transitive_collection = self.transitive_inputs.to_list();
        let reduced_jars: Vec<Artifact> = transitive_collection
            .iter()
            .filter(|input| direct.contains(input.get_exec_path_string()))
            .cloned()
            .collect();
        Ok(NestedSetBuilder::wrap(Order::STABLE_ORDER, reduced_jars))
    }

    /// The path mapper of `PathMappers.create(this, getOutputPathsMode(configuration), false,
    /// metadata)`, as the spawn's mapper.
    fn spawn_path_mapper(
        &self,
    ) -> (
        Arc<dyn blaze_actions::actions::path_mapper::PathMapper>,
        SpawnPathMapper,
    ) {
        let mapper = path_mappers::create(
            self,
            get_output_paths_mode(&self.configuration),
            false,
            None,
        );
        // PORT: `path_mappers::create` returns a non-noop mapper only for STRIP, which is
        // unported there; the spawn's mapper is therefore PathMapper.NOOP.
        (mapper, SpawnPathMapper::NOOP)
    }

    /// Builds the `JavaSpawn` of `expanded` command lines over `inputs`.
    ///
    /// PORT: `JavaSpawn` is a `SimpleSpawn` whose fields hold what Java's overrides return:
    /// `getInputFiles` (inputs plus the param files), `isMandatoryOutput` (the mandatory output
    /// set: only `onlyMandatoryOutput` when it is set) and `getPathMapper`.
    // Java: com/google/devtools/build/lib/rules/java/JavaCompileAction.java:610 JavaCompileAction.JavaSpawn.<init>
    // Java: com/google/devtools/build/lib/rules/java/JavaCompileAction.java:632 JavaCompileAction.JavaSpawn.getInputFiles
    // Java: com/google/devtools/build/lib/rules/java/JavaCompileAction.java:637 JavaCompileAction.JavaSpawn.isMandatoryOutput
    // Java: com/google/devtools/build/lib/rules/java/JavaCompileAction.java:642 JavaCompileAction.JavaSpawn.getPathMapper
    fn java_spawn(
        &self,
        command_lines: &CommandLines,
        action_execution_context: &dyn ActionExecutionContext,
        inputs: NestedSet<Artifact>,
        only_mandatory_output: Option<&Artifact>,
    ) -> Result<SimpleSpawn, ActionExecutionException> {
        let (mapper, spawn_mapper) = self.spawn_path_mapper();
        // PORT: the input metadata provider only expands tree artifacts, which Java compile
        // actions do not take as inputs.
        let expanded = command_lines
            .expand(
                None,
                self.get_primary_output().get_exec_path(),
                mapper.as_ref(),
                &self.command_line_limits(),
            )
            .map_err(|e| {
                self.create_action_execution_exception(
                    &e.to_string(),
                    failure_details::java_compile::Code::CommandLineExpansionFailure,
                )
            })?;
        let mut spawn_inputs = NestedSetBuilder::<ActionInputRef>::stable_order();
        spawn_inputs.add_transitive(&to_action_input_set(&inputs));
        spawn_inputs.add_all(
            expanded
                .get_param_files()
                .iter()
                .map(|p| ActionInputRef::new(p.clone())),
        );
        let environment = self
            .get_effective_environment(action_execution_context.get_client_env())
            .map_err(|e| {
                self.create_action_execution_exception(
                    &e.message,
                    failure_details::java_compile::Code::CommandLineExpansionFailure,
                )
            })?;
        // Java: isMandatoryOutput(output) = onlyMandatoryOutput == null ||
        // onlyMandatoryOutput.equals(output).
        let mandatory_outputs: Option<HashSet<ActionInputRef>> =
            only_mandatory_output.map(|o| std::iter::once(artifact_input(o)).collect());
        Ok(SimpleSpawn::new(
            Arc::new(JavaActionMetadata(ActionMetadataSnapshot::of(self))),
            expanded.arguments().to_vec(),
            environment,
            self.get_execution_info(),
            spawn_inputs.build(),
            // BaseSpawn.getToolFiles: action.getTools().
            to_action_input_set(&self.tools),
            self.get_outputs().iter().map(artifact_input).collect(),
            mandatory_outputs,
            Arc::new(local_resources()),
            spawn_mapper,
        ))
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaCompileAction.java:306 JavaCompileAction.getReducedSpawn
    fn get_reduced_spawn(
        &self,
        action_execution_context: &dyn ActionExecutionContext,
        reduced_classpath: &NestedSet<Artifact>,
        fallback: bool,
    ) -> Result<SimpleSpawn, ActionExecutionException> {
        let mut classpath_line = CustomCommandLine::builder();
        if fallback {
            classpath_line
                .add_exec_paths_nested_set_with_arg("--classpath", self.transitive_inputs.clone());
        } else {
            classpath_line
                .add_exec_paths_nested_set_with_arg("--classpath", reduced_classpath.clone());
        }

        if self.classpath_mode == JavaClasspathMode::BAZEL_NO_FALLBACK {
            // No need of fallback logic, invoke SimpleJavaLibraryBuilder with a reduced
            // --classpath
            classpath_line.add("--reduce_classpath_mode").add("NONE");
        } else {
            // These flags instruct JavaBuilder that this is a compilation with a reduced
            // classpath and that it should report a special value back if a compilation error
            // occurs that suggests retrying with the full classpath.
            classpath_line
                .add("--reduce_classpath_mode")
                .add(if fallback {
                    "BAZEL_FALLBACK"
                } else {
                    "BAZEL_REDUCED"
                });
        }

        let mut command_lines_builder = CommandLines::builder();
        command_lines_builder
            .add_command_line(self.executable_line.clone())
            .add_command_line_with_param_file(self.flag_line.clone(), Some(param_file_info()))
            .add_command_line_with_param_file(
                Arc::new(classpath_line.build()),
                Some(param_file_info()),
            );
        for extra_command_line in &self.extra_command_line_args {
            command_lines_builder.add_command_line_with_param_file(
                extra_command_line.clone(),
                Some(param_file_info()),
            );
        }
        let reduced_command_line = command_lines_builder.build();
        let mut inputs = NestedSetBuilder::stable_order();
        inputs.add_transitive(&self.mandatory_inputs);
        inputs.add_transitive(if fallback {
            &self.transitive_inputs
        } else {
            reduced_classpath
        });
        self.java_spawn(
            &reduced_command_line,
            action_execution_context,
            inputs.build(),
            /* only_mandatory_output= */
            if fallback {
                None
            } else {
                self.output_deps_proto.as_ref()
            },
        )
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaCompileAction.java:364 JavaCompileAction.getFullSpawn
    fn get_full_spawn(
        &self,
        action_execution_context: &dyn ActionExecutionContext,
    ) -> Result<SimpleSpawn, ActionExecutionException> {
        let mut inputs = NestedSetBuilder::stable_order();
        inputs
            .add_transitive(&self.mandatory_inputs)
            .add_transitive(&self.transitive_inputs)
            // Full spawn mode means classPathMode != JavaClasspathMode.BAZEL, which means
            // JavaBuilder may read .jdeps files to perform classpath reduction on the executor.
            // So make sure these files are staged as inputs to the executor action.
            //
            // Contrast this with getReducedSpawn, which reduces the classpath in the Blaze
            // process *before* sending actions to the executor. In those cases we want to avoid
            // staging .jdeps files, which have config prefixes in output paths, which compromise
            // caching possible by stripping prefixes on the executor.
            .add_transitive(&self.dependency_artifacts);
        self.java_spawn(
            &self.get_command_lines(),
            action_execution_context,
            inputs.build(),
            None,
        )
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaCompileAction.java:400 JavaCompileAction.wrapIOException
    fn wrap_io_exception(&self, e: &std::io::Error, message: &str) -> ActionExecutionException {
        ActionExecutionException::from_exec_exception(
            &ExecException::Environmental(EnvironmentalExecException::with_cause(
                e,
                create_failure_detail(
                    message,
                    failure_details::java_compile::Code::ReducedClasspathFallbackCleanupFailure,
                ),
            )),
            None,
            self,
        )
    }

    /// Reads the full `.jdeps` output from the given spawn results.
    // Java: com/google/devtools/build/lib/rules/java/JavaCompileAction.java:870 JavaCompileAction.readFullOutputDeps
    fn read_full_output_deps(
        &self,
        results: &[SpawnResult],
        action_execution_context: &dyn ActionExecutionContext,
        path_mapper: &SpawnPathMapper,
    ) -> Result<Arc<Dependencies>, ActionExecutionException> {
        // Java: Iterables.getOnlyElement(results)
        if results.len() != 1 {
            panic!(
                "IllegalArgumentException: expected one element but was: {} spawn results",
                results.len()
            );
        }
        let result = &results[0];
        create_full_output_deps(
            result,
            self.output_deps_proto
                .as_ref()
                .expect("outputDepsProto with a reduced classpath"),
            &self.get_inputs(),
            &self.get_additional_artifacts_for_path_mapping(),
            action_execution_context,
            path_mapper,
        )
        .map_err(|e| {
            ActionExecutionException::from_exec_exception(
                &ExecException::Environmental(EnvironmentalExecException::with_cause(
                    &e,
                    create_failure_detail(
                        ".jdeps read IOException",
                        failure_details::java_compile::Code::JdepsReadIoException,
                    ),
                )),
                None,
                self,
            )
        })
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaCompileAction.java:892 JavaCompileAction.createActionExecutionException
    fn create_action_execution_exception(
        &self,
        message: &str,
        detailed_code: failure_details::java_compile::Code,
    ) -> ActionExecutionException {
        // Java: DetailedExitCode.of(createFailureDetail(Strings.nullToEmpty(e.getMessage()), ..))
        ActionExecutionException::with_cause(
            message,
            &message,
            Some(self),
            /* catastrophe= */ false,
            Some(create_failure_detail(message, detailed_code)),
        )
    }

    fn get_context(
        action_execution_context: &dyn ActionExecutionContext,
    ) -> Arc<JavaCompileActionContext> {
        get_context::<JavaCompileActionContext>(action_execution_context).unwrap_or_else(|| {
            panic!("NullPointerException: getContext(JavaCompileActionContext.class) returned null")
        })
    }
}

/// `ActionExecutionMetadata` of a Java compile spawn: the action's metadata, with
/// `mayModifySpawnOutputsAfterExecution()` true.
// Java: com/google/devtools/build/lib/rules/java/JavaCompileAction.java:726 JavaCompileAction.mayModifySpawnOutputsAfterExecution
// Java: com/google/devtools/build/lib/rules/java/JavaHeaderCompileAction.java:161 JavaHeaderCompileAction.mayModifySpawnOutputsAfterExecution
#[derive(Debug)]
pub(crate) struct JavaActionMetadata(pub Arc<ActionMetadataSnapshot>);

impl ActionExecutionMetadata for JavaActionMetadata {
    fn get_mnemonic(&self) -> String {
        self.0.get_mnemonic()
    }
    fn get_progress_message(&self) -> Option<String> {
        self.0.get_progress_message()
    }
    fn get_owner(&self) -> &blaze_exec::actions::ActionOwner {
        self.0.get_owner()
    }
    fn get_primary_output(&self) -> ActionInputRef {
        self.0.get_primary_output()
    }
    fn get_execution_platform(&self) -> Option<&blaze_exec::actions::PlatformInfo> {
        self.0.get_execution_platform()
    }
    fn pretty_print(&self) -> String {
        self.0.pretty_print()
    }
    fn describe(&self) -> String {
        self.0.describe()
    }
    fn may_modify_spawn_outputs_after_execution(&self) -> bool {
        // Causes of spawn output modification after execution:
        // - Fallback to the full classpath with --experimental_java_classpath=bazel.
        // - In-place rewriting of .jdeps files with --experimental_output_paths=strip.
        true
    }
}

// Java: com/google/devtools/build/lib/rules/java/JavaCompileAction.java:887 JavaCompileAction.createFailureDetail
pub(crate) fn create_failure_detail(
    message: &str,
    detailed_code: failure_details::java_compile::Code,
) -> FailureDetail {
    FailureDetail {
        message: message.to_owned(),
        category: Some(Category::JavaCompile(failure_details::JavaCompile {
            code: detailed_code as i32,
        })),
        ..Default::default()
    }
}

/// Locally rewrites a .jdeps file to replace missing config prefixes.
///
/// For example: `bazel-out/bin/foo/foo.jar -> bazel-out/x86-fastbuild/bin/foo/foo.jar`.
///
/// If the executor doesn't strip config prefixes (i.e. config stripping isn't turned on as a
/// feature), this is a trivial copy. Returns the full deps proto (also written to disk to
/// satisfy the action's declared output).
// Java: com/google/devtools/build/lib/rules/java/JavaCompileAction.java:768 JavaCompileAction.createFullOutputDeps
pub(crate) fn create_full_output_deps(
    spawn_result: &SpawnResult,
    output_deps_proto: &Artifact,
    _action_inputs: &NestedSet<Artifact>,
    _additional_artifacts_for_path_mapping: &NestedSet<Artifact>,
    action_execution_context: &dyn ActionExecutionContext,
    path_mapper: &SpawnPathMapper,
) -> std::io::Result<Arc<Dependencies>> {
    let executor_jdeps =
        read_executor_jdeps(spawn_result, output_deps_proto, action_execution_context)?;

    if path_mapper.is_noop() {
        return Ok(Arc::new(executor_jdeps));
    }

    // No paths to rewrite.
    if executor_jdeps.dependency.is_empty() {
        return Ok(Arc::new(executor_jdeps));
    }

    // PORT: a non-noop path mapper is only created for --experimental_output_paths=strip
    // (StrippingPathMapper), which is not ported.
    blaze_util::unported!(
        "JavaCompileAction.createFullOutputDeps",
        "rewriting a .jdeps file of a path-mapped spawn"
    )
}

// Java: com/google/devtools/build/lib/rules/java/JavaCompileAction.java:778 JavaCompileAction.readExecutorJdeps
fn read_executor_jdeps(
    spawn_result: &SpawnResult,
    output_deps_proto: &Artifact,
    action_execution_context: &dyn ActionExecutionContext,
) -> std::io::Result<Dependencies> {
    if let Some(in_memory_output) =
        spawn_result.get_in_memory_output(&artifact_input(output_deps_proto))
    {
        return Dependencies::parse_from(in_memory_output);
    }
    let path =
        blaze_exec::analysis::actions::get_input_path(action_execution_context, output_deps_proto);
    let mut bytes = Vec::new();
    path.get_input_stream()?.read_to_end(&mut bytes)?;
    Dependencies::parse_from(&bytes)
}

impl Action for JavaCompileAction {
    fn abstract_action(&self) -> &AbstractAction {
        &self.base
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaCompileAction.java:216 JavaCompileAction.getMnemonic
    fn get_mnemonic(&self) -> String {
        self.compilation_type.mnemonic().to_owned()
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaCompileAction.java:221 JavaCompileAction.computeKey
    fn compute_key(
        &self,
        action_key_context: &ActionKeyContext,
        input_metadata_provider: Option<&dyn InputMetadataProvider>,
        fp: &mut Fingerprint,
    ) -> Result<(), ComputeKeyError> {
        fp.add_uuid(GUID_LEAST as i64, GUID_MOST as i64);
        fp.add_int(classpath_mode_ordinal(self.classpath_mode));
        let output_paths_mode = get_output_paths_mode(&self.configuration);
        let execution_info = self.get_execution_info();
        let effective_output_paths_mode = path_mappers::get_effective_output_paths_mode(
            output_paths_mode,
            &self.get_mnemonic(),
            &execution_info,
        );
        self.executable_line.add_to_fingerprint(
            action_key_context,
            input_metadata_provider,
            effective_output_paths_mode,
            fp,
        )?;
        self.flag_line.add_to_fingerprint(
            action_key_context,
            input_metadata_provider,
            effective_output_paths_mode,
            fp,
        )?;
        for extra_command_line in &self.extra_command_line_args {
            extra_command_line.add_to_fingerprint(
                action_key_context,
                input_metadata_provider,
                effective_output_paths_mode,
                fp,
            )?;
        }
        // As the classpath is no longer part of commandLines implicitly, we need to explicitly
        // add the transitive inputs to the key here.
        action_key_context.add_nested_set_to_fingerprint(fp, &self.transitive_inputs)?;
        self.get_environment().add_to(fp);
        // Invariant: execution info keys and values are internal strings.
        fp.add_string_map(
            self.execution_info
                .iter()
                .map(|(k, v)| (k.as_str(), v.as_str())),
        );
        fp.add_boolean(
            self.output_deps_proto.is_some() && inmemory_jdeps_files(&self.configuration),
        );
        path_mappers::add_to_fingerprint(
            &self.get_mnemonic(),
            &execution_info,
            &self.get_additional_artifacts_for_path_mapping(),
            action_key_context,
            output_paths_mode,
            fp,
        )?;
        Ok(())
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaCompileAction.java:206 JavaCompileAction.getTools
    fn get_tools(&self) -> NestedSet<Artifact> {
        self.tools.clone()
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaCompileAction.java:211 JavaCompileAction.getEnvironment
    fn get_environment(&self) -> Arc<ActionEnvironment> {
        self.env.clone()
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaCompileAction.java:514 JavaCompileAction.getRawProgressMessage
    fn get_raw_progress_message(&self) -> Option<String> {
        Some(self.progress_message.to_string_java())
    }

    /// Returns the out-of-band execution data for this action.
    // Java: com/google/devtools/build/lib/rules/java/JavaCompileAction.java:683 JavaCompileAction.getExecutionInfo
    fn get_execution_info(&self) -> IndexMap<String, String> {
        let result = merge_maps(self.get_exec_properties(), self.execution_info.clone());
        if self.output_deps_proto.is_none() || !inmemory_jdeps_files(&self.configuration) {
            return result;
        }
        let mut inline = IndexMap::new();
        inline.insert(
            REMOTE_EXECUTION_INLINE_OUTPUTS.to_owned(),
            self.output_deps_proto
                .as_ref()
                .unwrap()
                .get_exec_path_string()
                .to_string(),
        );
        merge_maps(result, inline)
    }

    fn as_command_action(&self) -> Option<&dyn CommandAction> {
        Some(self)
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaCompileAction.java:703 JavaCompileAction.getStarlarkArgs
    fn get_starlark_args(
        &self,
    ) -> Option<Vec<blaze_actions::analysis::starlark::args::RegisteredArgs>> {
        // PORT: Java ImmutableSet (first occurrence order, no duplicates).
        let mut directory_inputs: indexmap::IndexSet<Artifact> = indexmap::IndexSet::new();
        for a in self.get_inputs().to_list().iter() {
            if a.is_directory() {
                directory_inputs.insert(a.clone());
            }
        }
        let mut result = Vec::new();
        for command_line in self.get_command_lines().unpack() {
            result.push(
                blaze_actions::analysis::starlark::args::RegisteredArgs::for_registered_action(
                    command_line.command_line,
                    command_line.param_file_info,
                    directory_inputs.iter().cloned().collect(),
                ),
            );
        }
        Some(result)
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaCompileAction.java:673 JavaCompileAction.getStarlarkArgv
    fn get_starlark_argv(&self) -> Result<Option<Vec<String>>, ActionApiError> {
        // Java: a CommandLineExpansionException becomes `new EvalException(ex)`.
        Ok(Some(self.get_arguments()?))
    }
}

impl CommandAction for JavaCompileAction {
    // Java: com/google/devtools/build/lib/rules/java/JavaCompileAction.java:697 JavaCompileAction.getArguments
    fn get_arguments(&self) -> Result<Vec<String>, CommandLineError> {
        self.get_command_lines().all_arguments()
    }
}

impl ExecutableAction for JavaCompileAction {
    // Java: com/google/devtools/build/lib/rules/java/JavaCompileAction.java:408 JavaCompileAction.execute
    fn execute(
        &self,
        action_execution_context: &dyn ActionExecutionContext,
    ) -> Result<ActionResult, ActionExecutionException> {
        let reduced_classpath: Option<NestedSet<Artifact>>;
        let mut spawn: SimpleSpawn;
        if self.classpath_mode == JavaClasspathMode::BAZEL
            || self.classpath_mode == JavaClasspathMode::BAZEL_NO_FALLBACK
        {
            let context = Self::get_context(action_execution_context);
            let classpath = match self.get_reduced_classpath(action_execution_context, &context) {
                Ok(c) => c,
                Err(e) => {
                    return Err(self.create_action_execution_exception(
                        &e.to_string(),
                        failure_details::java_compile::Code::ReducedClasspathFailure,
                    ));
                }
            };
            spawn = self.get_reduced_spawn(action_execution_context, &classpath, false)?;
            reduced_classpath = Some(classpath);
        } else {
            reduced_classpath = None;
            spawn = self.get_full_spawn(action_execution_context)?;
        }

        let resolver = get_context::<SpawnStrategyResolver>(action_execution_context)
            .unwrap_or_else(|| {
                panic!(
                    "NullPointerException: getContext(SpawnStrategyResolver.class) returned null"
                )
            });
        let primary_results = resolver
            .exec(&spawn, action_execution_context)
            .map_err(|e| to_action_execution_exception(&e, self))?;

        let Some(reduced_classpath) = reduced_classpath else {
            return Ok(ActionResult::create(primary_results));
        };

        let dependencies = self.read_full_output_deps(
            &primary_results,
            action_execution_context,
            blaze_exec::actions::Spawn::get_path_mapper(&spawn),
        )?;

        if self.compilation_type == CompilationType::TURBINE {
            Self::get_context(action_execution_context).insert_dependencies(
                self.output_deps_proto.as_ref().unwrap(),
                dependencies.clone(),
            );
        }
        if !dependencies.get_requires_reduced_classpath_fallback() {
            return Ok(ActionResult::create(primary_results));
        }

        // Fall back to running with the full classpath. This requires first deleting potential
        // artifacts generated by the reduced action and clearing the metadata caches.
        if let Err(e) = blaze_exec::actions::action_execution::delete_outputs(
            self,
            action_execution_context.get_exec_root(),
        ) {
            return Err(self.wrap_io_exception(&e, "Failed to delete reduced action outputs"));
        }

        // Java: actionExecutionContext.getOutputMetadataStore().resetOutputs(getOutputs())
        reset_outputs(self, action_execution_context);

        let out_err = action_execution_context.get_file_out_err();
        if let Err(e) = out_err.clear_out().and_then(|()| out_err.clear_err()) {
            return Err(self.wrap_io_exception(&e, "Failed to clean reduced action stdout/stderr"));
        }

        spawn = self.get_reduced_spawn(action_execution_context, &reduced_classpath, true)?;

        let fallback_results = resolver
            .exec(&spawn, action_execution_context)
            .map_err(|e| to_action_execution_exception(&e, self))?;

        if self.compilation_type == CompilationType::TURBINE {
            let deps = self.read_full_output_deps(
                &fallback_results,
                action_execution_context,
                blaze_exec::actions::Spawn::get_path_mapper(&spawn),
            )?;
            Self::get_context(action_execution_context)
                .insert_dependencies(self.output_deps_proto.as_ref().unwrap(), deps);
        } else if !blaze_exec::actions::Spawn::get_path_mapper(&spawn).is_noop() {
            // As a side effect, readFullOutputDeps rewrites the on-disk .jdeps file from
            // mapped to unmapped paths. To make path mapping fully transparent to consumers
            // of this action's output, we ensure that the file always contains unmapped
            // paths.
            let _unused = self.read_full_output_deps(
                &fallback_results,
                action_execution_context,
                blaze_exec::actions::Spawn::get_path_mapper(&spawn),
            )?;
        }
        let mut all = primary_results;
        all.extend(fallback_results);
        Ok(ActionResult::create(all))
    }
}

/// `actionExecutionContext.getOutputMetadataStore().resetOutputs(action.getOutputs())`.
fn reset_outputs(action: &JavaCompileAction, ctx: &dyn ActionExecutionContext) {
    let outputs: Vec<ActionInputRef> = action.get_outputs().iter().map(artifact_input).collect();
    ctx.reset_outputs(&outputs);
}

/// `ActionExecutionException.fromExecException(e, this)`, with a spawn failure as Java's
/// `SpawnExecException.toActionExecutionException` (what `fromExecException` delegates to).
pub(crate) fn to_action_execution_exception(
    e: &ExecException,
    action: &dyn Action,
) -> ActionExecutionException {
    match e {
        ExecException::Spawn(spawn_exec_exception) => {
            blaze_exec::analysis::actions::spawn_action::to_action_execution_exception(
                spawn_exec_exception,
                action,
            )
        }
        _ => ActionExecutionException::from_exec_exception(e, None, action),
    }
}
