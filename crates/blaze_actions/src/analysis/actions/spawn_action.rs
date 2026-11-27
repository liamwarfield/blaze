//! Ports of `lib/analysis/actions/SpawnAction.java` (with its `Builder`) and
//! `lib/analysis/actions/ShellCommand.java`.
//!
//! `StarlarkAction` (`ctx.actions.run` / `run_shell`) is a `SpawnAction` subclass that only adds
//! the unused-inputs-list and shadowed-action machinery; it is the [`SpawnActionClass::Starlark`]
//! and [`SpawnActionClass::EnhancedStarlark`] variants here (see `starlark_action.rs` for its
//! builder entry points).

use std::any::Any;
use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

use blaze_collect::command_line_item::CommandLineExpansionException;
use blaze_collect::{NestedSet, NestedSetBuilder, Order};
use blaze_shell::ShellEscaper;
use blaze_util::{Fingerprint, check_argument};
use blaze_vfs::PathFragment;
use indexmap::{IndexMap, IndexSet};

use crate::actions::action::{
    AbstractAction, Action, ActionRef, CommandAction, CommandLineError, ComputeKeyError,
    InputMetadataProvider, merge_maps,
};
use crate::actions::action_environment::ActionEnvironment;
use crate::actions::action_key_context::ActionKeyContext;
use crate::actions::action_owner::ActionOwner;
use crate::actions::artifact::Artifact;
use crate::actions::command_line::{self, CommandLine, CommandLineRef};
use crate::actions::command_lines::{CommandLineAndParamFileInfo, CommandLines};
use crate::actions::execution_requirements;
use crate::actions::param_file_info::ParamFileInfo;
use crate::actions::path_mapper::{OutputPathsMode, PathMapper};
use crate::analysis::actions::custom_command_line::CustomCommandLine;
use crate::analysis::actions::path_mappers;

// Java: com/google/devtools/build/lib/analysis/actions/SpawnAction.java:101 SpawnAction.GUID
const GUID: &str = "ebd6fce3-093e-45ee-adb6-bf513b602f0d";

/// The resources an action declares (`ResourceSetOrBuilder`).
///
/// The precondition of `SpawnAction.Builder.setMnemonic`:
/// `!mnemonic.isEmpty() && CharMatcher.javaLetterOrDigit().matchesAllOf(mnemonic)`.
///
/// Java tests the chars of the string as Bazel holds it, the internal string: one Latin-1 char
/// per byte. So `"Qé"` (bytes c3 a9: `Ã`, a letter, then `©`, not one) is rejected, while `"Qê"`
/// (c3 aa: `Ã` then `ª`, a letter) is accepted; the oracle does both
/// (`tests/data/map_each_probe/expected.txt`).
// Java: com/google/devtools/build/lib/analysis/actions/SpawnAction.java:1258 SpawnAction.Builder.setMnemonic
pub fn is_valid_mnemonic(mnemonic: &str) -> bool {
    !mnemonic.is_empty() && mnemonic.chars().all(java_is_letter_or_digit_latin1)
}

/// Java `Character.isLetterOrDigit(char)` for a char of an internal string (`<= U+00FF`): the
/// letters of Latin-1 (general categories Lu, Ll, Lo) and the ASCII digits. The other Latin-1
/// characters with a numeric value (`²`, `³`, `¹`, `¼`, `½`, `¾`) are category No, which is not
/// a digit for Java.
// Java (JDK): java.base/java/lang/Character.java Character.isLetterOrDigit
fn java_is_letter_or_digit_latin1(c: char) -> bool {
    match c {
        '0'..='9' | 'A'..='Z' | 'a'..='z' => true,
        '\u{aa}' | '\u{b5}' | '\u{ba}' => true,
        '\u{c0}'..='\u{d6}' | '\u{d8}'..='\u{f6}' | '\u{f8}'..='\u{ff}' => true,
        '\u{0}'..='\u{ff}' => false,
        // Not a char of an internal string.
        _ => blaze_util::unported!(
            "Character.isLetterOrDigit",
            "char U+{:04X} is not a char of an internal string",
            c as u32
        ),
    }
}

/// PORT: `ResourceSet`/`ResourceSetOrBuilder` belong to the execution layer (wave W6); they are
/// not part of the action key. Kept as plain data.
#[derive(Clone, Debug, PartialEq)]
pub struct ResourceSet {
    pub memory_mb: f64,
    pub cpu_usage: f64,
    pub extra_resource_usage: IndexMap<String, f64>,
    pub local_test_count: i32,
}

impl ResourceSet {
    /// `AbstractAction.DEFAULT_RESOURCE_SET` = `ResourceSet.createWithRamCpu(250, 1)`.
    // Java: com/google/devtools/build/lib/actions/AbstractAction.java:78 AbstractAction.DEFAULT_RESOURCE_SET
    pub fn default_resource_set() -> ResourceSet {
        ResourceSet {
            memory_mb: 250.0,
            cpu_usage: 1.0,
            extra_resource_usage: IndexMap::new(),
            local_test_count: 0,
        }
    }
}

/// The failure of a [`ResourceSetBuilder`]: Java's `UserExecException` with a `FailureDetail`
/// whose message is `message` and whose code is `StarlarkAction.Code.STARLARK_ACTION_UNKNOWN`.
// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkActionFactory.java:954 StarlarkActionResourceSetBuilder.buildResourceSet
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResourceSetBuildError {
    /// `FailureDetail.message` (e.g. `Could not build resources for CppLink. ...`).
    pub message: String,
}

/// A `ResourceSetOrBuilder` that computes the resource set at execution time from the OS and
/// the number of inputs (Java `StarlarkActionFactory.StarlarkActionResourceSetBuilder`, the
/// `resource_set` callback of `ctx.actions.run` / `run_shell`).
///
/// A fixed resource set stays [`ResourceSet`] (`SpawnAction::get_resource_set_or_builder`);
/// an action built with [`SpawnActionBuilder::set_resources_builder`] carries this instead, and
/// the execution layer must call it where Java calls `BaseSpawn.getLocalResources`.
// Java: com/google/devtools/build/lib/actions/ResourceSetOrBuilder.java:22 ResourceSetOrBuilder
pub trait ResourceSetBuilder: Send + Sync + fmt::Debug {
    /// `buildResourceSet(OS os, int inputsSize)`.
    // Java: com/google/devtools/build/lib/actions/ResourceSetOrBuilder.java:29 ResourceSetOrBuilder.buildResourceSet
    fn build_resource_set(
        &self,
        os: blaze_shell::util::os::OS,
        inputs_size: i32,
    ) -> Result<ResourceSet, ResourceSetBuildError>;
}

/// The configuration values `SpawnAction.Builder.build(owner, configuration)` reads from
/// `BuildConfigurationValue`.
///
/// PORT: implemented by the analysis layer for its `BuildConfigurationValue` (blaze_build_config).
pub trait SpawnActionConfiguration: Send + Sync {
    /// `configuration.getActionEnvironment()` (the default shell environment).
    fn get_action_environment(&self) -> Arc<ActionEnvironment>;

    /// `configuration.modifiedExecutionInfo(executionInfo, mnemonic)` (`--modify_execution_info`).
    fn modified_execution_info(
        &self,
        execution_info: &IndexMap<String, String>,
        mnemonic: &str,
    ) -> IndexMap<String, String>;

    /// `PathMappers.getOutputPathsMode(configuration)`: `CoreOptions.outputPathsMode`.
    fn get_output_paths_mode(&self) -> OutputPathsMode;
}

/// Which Java class a [`SpawnAction`] is.
#[derive(Clone)]
pub enum SpawnActionClass {
    // Java: com/google/devtools/build/lib/analysis/actions/SpawnAction.java:99 SpawnAction
    Spawn,
    // Java: com/google/devtools/build/lib/analysis/actions/StarlarkAction.java:65 StarlarkAction
    Starlark,
    // Java: com/google/devtools/build/lib/analysis/actions/StarlarkAction.java:224 StarlarkAction.EnhancedStarlarkAction
    EnhancedStarlark(EnhancedStarlarkData),
}

/// The fields of `EnhancedStarlarkAction`.
#[derive(Clone)]
pub struct EnhancedStarlarkData {
    /// All the inputs of the Starlark action including those listed in the unused inputs and
    /// excluding the shadowed action inputs.
    pub all_starlark_action_inputs: NestedSet<Artifact>,
    pub unused_inputs_list: Option<Artifact>,
    pub shadowed_action: Option<ActionRef>,
    inputs_discovered: Arc<parking_lot::Mutex<bool>>,
}

/// An Action representing an arbitrary subprocess to be forked and exec'd.
// Java: com/google/devtools/build/lib/analysis/actions/SpawnAction.java:99 SpawnAction
pub struct SpawnAction {
    base: AbstractAction,
    tools: NestedSet<Artifact>,
    command_lines: CommandLines,
    env: Arc<ActionEnvironment>,
    progress_message: Option<String>,
    mnemonic: String,
    resource_set: ResourceSet,
    /// Set instead of `resource_set` when the builder was given a [`ResourceSetBuilder`] (Java
    /// keeps one `ResourceSetOrBuilder` field).
    resource_set_builder: Option<Arc<dyn ResourceSetBuilder>>,
    /// `ImmutableSortedMap` of the execution info.
    sorted_execution_info: BTreeMap<String, String>,
    output_paths_mode: OutputPathsMode,
    class: SpawnActionClass,
}

impl fmt::Debug for SpawnAction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_string_java())
    }
}

impl SpawnAction {
    /// Constructs a SpawnAction using direct initialization arguments.
    ///
    /// `tools`: the set of files comprising the tool that does the work (e.g. compiler). This is a
    /// subset of "inputs" and is only used by the WorkerSpawnStrategy. `inputs`: the set of all
    /// files potentially read by this action. `outputs`: the set of all files written by this
    /// action. `command_lines`: the command lines to execute, including the main argv vector and
    /// any param file-backed command lines.
    // Java: com/google/devtools/build/lib/analysis/actions/SpawnAction.java:137 SpawnAction.<init>
    pub fn new(
        owner: ActionOwner,
        tools: NestedSet<Artifact>,
        inputs: NestedSet<Artifact>,
        outputs: impl IntoIterator<Item = Artifact>,
        resource_set: ResourceSet,
        command_lines: CommandLines,
        env: Arc<ActionEnvironment>,
        execution_info: &IndexMap<String, String>,
        progress_message: Option<String>,
        mnemonic: impl Into<String>,
        output_paths_mode: OutputPathsMode,
    ) -> SpawnAction {
        SpawnAction::new_with_class(
            owner,
            tools,
            inputs,
            outputs,
            resource_set,
            command_lines,
            env,
            execution_info,
            progress_message,
            mnemonic.into(),
            output_paths_mode,
            SpawnActionClass::Spawn,
        )
    }

    pub(crate) fn new_with_class(
        owner: ActionOwner,
        tools: NestedSet<Artifact>,
        inputs: NestedSet<Artifact>,
        outputs: impl IntoIterator<Item = Artifact>,
        resource_set: ResourceSet,
        command_lines: CommandLines,
        env: Arc<ActionEnvironment>,
        execution_info: &IndexMap<String, String>,
        progress_message: Option<String>,
        mnemonic: String,
        output_paths_mode: OutputPathsMode,
        class: SpawnActionClass,
    ) -> SpawnAction {
        SpawnAction {
            base: AbstractAction::new(owner, inputs, outputs),
            tools,
            command_lines,
            env,
            progress_message,
            mnemonic,
            resource_set,
            resource_set_builder: None,
            sorted_execution_info: execution_info
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
            output_paths_mode,
            class,
        }
    }

    // Java: com/google/devtools/build/lib/analysis/actions/SpawnAction.java:202 SpawnAction.getCommandLines
    pub fn get_command_lines(&self) -> &CommandLines {
        &self.command_lines
    }

    pub fn get_class(&self) -> &SpawnActionClass {
        &self.class
    }

    /// `this instanceof StarlarkAction`.
    pub fn is_starlark_action(&self) -> bool {
        !matches!(self.class, SpawnActionClass::Spawn)
    }

    /// The path mapper of `PathMappers.create(this, outputPathsMode, this instanceof
    /// StarlarkAction, null)`.
    // Java: com/google/devtools/build/lib/analysis/actions/SpawnAction.java:207 SpawnAction.getArguments
    fn path_mapper_for_arguments(&self) -> std::sync::Arc<dyn PathMapper> {
        path_mappers::create(
            self,
            self.output_paths_mode,
            self.is_starlark_action(),
            /* input_metadata_provider= */ None,
        )
    }

    /// Returns command argument, argv[0].
    // Java: com/google/devtools/build/lib/analysis/actions/SpawnAction.java:245 SpawnAction.getCommandFilename
    pub fn get_command_filename(&self) -> Result<Option<String>, CommandLineError> {
        Ok(self.get_arguments()?.into_iter().next())
    }

    /// Returns the (immutable) list of arguments, excluding the command name, argv[0].
    // Java: com/google/devtools/build/lib/analysis/actions/SpawnAction.java:251 SpawnAction.getRemainingArguments
    pub fn get_remaining_arguments(&self) -> Result<Vec<String>, CommandLineError> {
        Ok(self.get_arguments()?.into_iter().skip(1).collect())
    }

    // Java: com/google/devtools/build/lib/analysis/actions/SpawnAction.java:311 SpawnAction.getResourceSetOrBuilder
    pub fn get_resource_set_or_builder(&self) -> &ResourceSet {
        &self.resource_set
    }

    /// The `ResourceSetOrBuilder` when it is a builder (a Starlark `resource_set` callback);
    /// `None` when it is the fixed [`get_resource_set_or_builder`](Self::get_resource_set_or_builder).
    // Java: com/google/devtools/build/lib/analysis/actions/SpawnAction.java:311 SpawnAction.getResourceSetOrBuilder
    pub fn get_resource_set_builder(&self) -> Option<&Arc<dyn ResourceSetBuilder>> {
        self.resource_set_builder.as_ref()
    }

    // Java: com/google/devtools/build/lib/analysis/actions/SpawnAction.java:511 SpawnAction.getIncompleteEnvironmentForTesting
    pub fn get_incomplete_environment_for_testing(&self) -> IndexMap<String, String> {
        self.get_environment().get_fixed_env()
    }

    // Java: com/google/devtools/build/lib/analysis/actions/StarlarkAction.java:121 StarlarkAction.getUnusedInputsList
    pub fn get_unused_inputs_list(&self) -> Option<Artifact> {
        match &self.class {
            SpawnActionClass::EnhancedStarlark(d) => d.unused_inputs_list.clone(),
            _ => None,
        }
    }

    pub fn get_output_paths_mode(&self) -> OutputPathsMode {
        self.output_paths_mode
    }

    /// `SpawnAction.createActionEnvironment(configuration, useDefaultShellEnvironment, env)`.
    // Java: com/google/devtools/build/lib/analysis/actions/SpawnAction.java:584 SpawnAction.createActionEnvironment
    pub fn create_action_environment(
        configuration: &dyn SpawnActionConfiguration,
        use_default_shell_environment: bool,
        environment: &IndexMap<String, String>,
    ) -> Arc<ActionEnvironment> {
        let env: Arc<ActionEnvironment>;
        if use_default_shell_environment && !environment.is_empty() {
            // Inherited variables override fixed variables in ActionEnvironment. Since we want the
            // fixed part of the action-provided environment to override the inherited part of the
            // user-provided environment, we have to explicitly filter the inherited part.
            let configuration_env = configuration.get_action_environment();
            let inherited = configuration_env.get_inherited_env();
            let user_filtered_inherited_env: IndexSet<String> = inherited
                .iter()
                .filter(|k| !environment.contains_key(*k))
                .cloned()
                .collect();
            // Do not create a new ActionEnvironment in the common case where no vars have been
            // filtered out.
            let base = if user_filtered_inherited_env.len() == inherited.len() {
                configuration_env
            } else {
                Arc::new(ActionEnvironment::create(
                    configuration_env.get_fixed_env(),
                    user_filtered_inherited_env,
                ))
            };
            env = base.with_additional_fixed_variables(environment.clone());
        } else if use_default_shell_environment {
            // This produces the same result as the previous case, but without the overhead.
            env = configuration.get_action_environment();
        } else {
            env = Arc::new(ActionEnvironment::create_fixed(environment.clone()));
        }
        env
    }

    /// Returns information about this spawn action for use by the extra action mechanism.
    ///
    /// PORT: the `Spawn` of `getSpawnForExtraActionSpawnInfo` is not built (the spawn types are
    /// the execution layer's); its parts are read where `ActionSpawn` takes them: the arguments
    /// (`commandLines.allArguments()`), the effective environment for an empty client
    /// environment, the given inputs (no additional inputs) and the outputs (`reportOutputs`).
    // Java: com/google/devtools/build/lib/analysis/actions/SpawnAction.java:487 SpawnAction.getExtraActionSpawnInfo
    fn get_extra_action_spawn_info(
        &self,
    ) -> Result<blaze_protos::blaze::SpawnInfo, CommandLineError> {
        let inputs = self.get_inputs_for_extra_action_spawn_info();
        let mut info = blaze_protos::blaze::SpawnInfo {
            argument: self.command_lines.all_arguments()?,
            ..Default::default()
        };
        // ActionSpawn: parent.getEffectiveEnvironment(clientEnv = ImmutableMap.of(), NOOP).
        for (name, value) in self
            .get_effective_environment(&IndexMap::new(), &crate::actions::path_mapper::NOOP)
            .map_err(CommandLineError::CommandLineExpansion)?
        {
            info.variable
                .push(blaze_protos::blaze::EnvironmentVariable { name, value });
        }
        for input in inputs.to_list().iter() {
            // Explicitly ignore runfiles tree artifacts here.
            if !input.is_runfiles_tree() {
                info.input_file
                    .push(input.get_exec_path_string().to_owned());
            }
        }
        for output in self.get_outputs() {
            info.output_file
                .push(output.get_exec_path_string().to_owned());
        }
        Ok(info)
    }

    /// The inputs `getSpawnForExtraActionSpawnInfo()` gives its spawn: `getInputs()`, or for an
    /// `EnhancedStarlarkAction` all its Starlark inputs (after its shadowed action's inputs).
    // Java: com/google/devtools/build/lib/analysis/actions/SpawnAction.java:329 SpawnAction.getSpawnForExtraActionSpawnInfo
    // Java: com/google/devtools/build/lib/analysis/actions/StarlarkAction.java:443 EnhancedStarlarkAction.getSpawnForExtraActionSpawnInfo
    fn get_inputs_for_extra_action_spawn_info(&self) -> NestedSet<Artifact> {
        match &self.class {
            SpawnActionClass::EnhancedStarlark(d) => match &d.shadowed_action {
                Some(shadowed) => {
                    create_inputs(&[shadowed.get_inputs(), d.all_starlark_action_inputs.clone()])
                }
                None => d.all_starlark_action_inputs.clone(),
            },
            _ => self.get_inputs(),
        }
    }

    fn enhanced(&self) -> Option<&EnhancedStarlarkData> {
        match &self.class {
            SpawnActionClass::EnhancedStarlark(d) => Some(d),
            _ => None,
        }
    }
}

impl Action for SpawnAction {
    fn abstract_action(&self) -> &AbstractAction {
        &self.base
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    // Java: com/google/devtools/build/lib/analysis/actions/SpawnAction.java:462 SpawnAction.getMnemonic
    fn get_mnemonic(&self) -> String {
        self.mnemonic.clone()
    }

    // Java: com/google/devtools/build/lib/analysis/actions/SpawnAction.java:396 SpawnAction.computeKey
    fn compute_key(
        &self,
        action_key_context: &ActionKeyContext,
        input_metadata_provider: Option<&dyn InputMetadataProvider>,
        fp: &mut Fingerprint,
    ) -> Result<(), ComputeKeyError> {
        fp.add_string(GUID);
        let execution_info = self.get_execution_info();
        self.command_lines.add_to_fingerprint(
            action_key_context,
            input_metadata_provider,
            path_mappers::get_effective_output_paths_mode(
                self.output_paths_mode,
                &self.get_mnemonic(),
                &execution_info,
            ),
            fp,
        )?;
        fp.add_string(&self.mnemonic);
        self.env.add_to(fp);
        // Invariant: execution info keys and values are internal strings (see
        // `actions::fingerprint_util`).
        fp.add_string_map(execution_info.iter().map(|(k, v)| (k.as_str(), v.as_str())));
        path_mappers::add_to_fingerprint(
            &self.get_mnemonic(),
            &execution_info,
            &NestedSetBuilder::empty_set(Order::STABLE_ORDER),
            action_key_context,
            self.output_paths_mode,
            fp,
        )?;
        Ok(())
    }

    // Java: com/google/devtools/build/lib/analysis/actions/SpawnAction.java:420 SpawnAction.describeKey
    fn describe_key(&self) -> Option<String> {
        let mut message = String::new();
        message.push_str(
            &self
                .get_progress_message(None)
                .unwrap_or_else(|| "null".into()),
        );
        message.push('\n');
        for (k, v) in self.env.get_fixed_env() {
            message.push_str("  Environment variable: ");
            message.push_str(&ShellEscaper::escape_string(&k));
            message.push('=');
            message.push_str(&ShellEscaper::escape_string(&v));
            message.push('\n');
        }
        for var in self.get_client_environment_variables() {
            message.push_str("  Environment variables taken from the client environment: ");
            message.push_str(&ShellEscaper::escape_string(&var));
            message.push('\n');
        }
        for (k, v) in self.get_execution_info() {
            message.push_str("  Execution info: ");
            message.push_str(&k);
            message.push('=');
            message.push_str(&v);
            message.push('\n');
        }
        match self.get_arguments() {
            Ok(args) => {
                for argument in args {
                    message.push_str("  Argument: ");
                    message.push_str(&ShellEscaper::escape_string(&argument));
                    message.push('\n');
                }
            }
            Err(CommandLineError::Interrupted(_)) => {
                message.push_str("Interrupted while expanding command line\n");
            }
            Err(CommandLineError::CommandLineExpansion(e)) => {
                message.push_str("Could not expand command line: ");
                // Java: e.toString() = class name + ": " + message.
                message.push_str(&format!(
                    "com.google.devtools.build.lib.actions.CommandLineExpansionException: {}",
                    e.message
                ));
                message.push('\n');
            }
        }
        message.push_str("  Output paths mode: ");
        message.push_str(&format!("{:?}", self.output_paths_mode));
        message.push('\n');
        Some(message)
    }

    // Java: com/google/devtools/build/lib/analysis/actions/SpawnAction.java:192 SpawnAction.getTools
    fn get_tools(&self) -> NestedSet<Artifact> {
        self.tools.clone()
    }

    // Java: com/google/devtools/build/lib/analysis/actions/SpawnAction.java:197 SpawnAction.getEnvironment
    fn get_environment(&self) -> Arc<ActionEnvironment> {
        Arc::clone(&self.env)
    }

    // Java: com/google/devtools/build/lib/analysis/actions/SpawnAction.java:257 SpawnAction.isVolatile
    fn is_volatile(&self) -> bool {
        self.execute_unconditionally()
    }

    // Java: com/google/devtools/build/lib/analysis/actions/SpawnAction.java:467 SpawnAction.getRawProgressMessage
    fn get_raw_progress_message(&self) -> Option<String> {
        if let Some(m) = &self.progress_message {
            return Some(m.clone());
        }
        Some(self.default_progress_message())
    }

    /// Returns the out-of-band execution data for this action.
    // Java: com/google/devtools/build/lib/analysis/actions/SpawnAction.java:405 SpawnAction.getExecutionInfo
    fn get_execution_info(&self) -> IndexMap<String, String> {
        let sorted: IndexMap<String, String> = self
            .sorted_execution_info
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        merge_maps(&self.get_exec_properties(), &sorted)
    }

    // Java: com/google/devtools/build/lib/analysis/actions/StarlarkAction.java:303 EnhancedStarlarkAction.getSchedulingDependencies
    fn get_scheduling_dependencies(&self) -> NestedSet<Artifact> {
        match self.enhanced().and_then(|d| d.shadowed_action.as_ref()) {
            Some(shadowed) => shadowed.get_scheduling_dependencies(),
            None => NestedSetBuilder::empty_set(Order::STABLE_ORDER),
        }
    }

    // Java: com/google/devtools/build/lib/analysis/actions/StarlarkAction.java:315 EnhancedStarlarkAction.isShareable
    fn is_shareable(&self) -> bool {
        match self.enhanced() {
            Some(d) => d.unused_inputs_list.is_none(),
            None => true,
        }
    }

    // Java: com/google/devtools/build/lib/analysis/actions/StarlarkAction.java:320 EnhancedStarlarkAction.discoversInputs
    fn discovers_inputs(&self) -> bool {
        match self.enhanced() {
            Some(d) => {
                d.unused_inputs_list.is_some()
                    || d.shadowed_action
                        .as_ref()
                        .is_some_and(|s| s.discovers_inputs())
            }
            None => false,
        }
    }

    // Java: com/google/devtools/build/lib/analysis/actions/StarlarkAction.java:326 EnhancedStarlarkAction.getOriginalInputs
    fn get_original_inputs(&self) -> NestedSet<Artifact> {
        match self.enhanced() {
            Some(d) => d.all_starlark_action_inputs.clone(),
            None => self.get_inputs(),
        }
    }

    // Java: com/google/devtools/build/lib/analysis/actions/StarlarkAction.java:331 EnhancedStarlarkAction.inputsDiscovered
    fn inputs_discovered(&self) -> bool {
        match self.enhanced() {
            Some(d) => *d.inputs_discovered.lock(),
            None => panic!(
                "IllegalStateException: Must be overridden by input-discovering action: {}",
                self.to_string_java()
            ),
        }
    }

    // Java: com/google/devtools/build/lib/analysis/actions/StarlarkAction.java:336 EnhancedStarlarkAction.setInputsDiscovered
    fn set_inputs_discovered(&self, inputs_discovered: bool) {
        match self.enhanced() {
            Some(d) => *d.inputs_discovered.lock() = inputs_discovered,
            None => panic!(
                "IllegalStateException: Must be overridden by input-discovering action: {}",
                self.to_string_java()
            ),
        }
    }

    // Java: com/google/devtools/build/lib/analysis/actions/StarlarkAction.java:341 EnhancedStarlarkAction.getAllowedDerivedInputs
    fn get_allowed_derived_inputs(&self) -> NestedSet<Artifact> {
        match self.enhanced() {
            Some(d) => {
                if let Some(shadowed) = &d.shadowed_action {
                    return create_inputs(&[
                        shadowed.get_allowed_derived_inputs(),
                        self.get_inputs(),
                    ]);
                }
                self.get_inputs()
            }
            None => panic!(
                "IllegalStateException: Must be overridden for action that may have unknown inputs: {}",
                self.to_string_java()
            ),
        }
    }

    // Java: com/google/devtools/build/lib/analysis/actions/StarlarkAction.java:483 EnhancedStarlarkAction.getEffectiveEnvironment
    fn get_effective_environment(
        &self,
        client_env: &IndexMap<String, String>,
        path_mapper: &dyn crate::actions::path_mapper::PathMapper,
    ) -> Result<IndexMap<String, String>, CommandLineExpansionException> {
        let env = self.get_environment();
        let mut environment = IndexMap::with_capacity(env.estimated_size());
        if let Some(shadowed) = self.enhanced().and_then(|d| d.shadowed_action.as_ref()) {
            // Put all the variables of the shadowed action's environment
            for (k, v) in shadowed.get_effective_environment(client_env, path_mapper)? {
                environment.insert(k, v);
            }
        }
        // This order guarantees that the Starlark action can overwrite any variable in its
        // shadowed action environment with a new value.
        env.resolve(&mut environment, client_env);
        Ok(environment)
    }

    fn as_command_action(&self) -> Option<&dyn CommandAction> {
        Some(self)
    }

    /// `SpawnAction` keeps `AbstractAction`'s empty set; a `StarlarkAction` gives its inputs, an
    /// `EnhancedStarlarkAction` all its Starlark inputs (after its shadowed action's set, `None`
    /// when that is `None`).
    // Java: com/google/devtools/build/lib/actions/AbstractAction.java:606 AbstractAction.getInputFilesForExtraAction
    // Java: com/google/devtools/build/lib/analysis/actions/StarlarkAction.java:126 StarlarkAction.getInputFilesForExtraAction
    // Java: com/google/devtools/build/lib/analysis/actions/StarlarkAction.java:454 EnhancedStarlarkAction.getInputFilesForExtraAction
    fn get_input_files_for_extra_action(&self) -> Option<NestedSet<Artifact>> {
        match &self.class {
            SpawnActionClass::Spawn => Some(NestedSetBuilder::empty_set(Order::STABLE_ORDER)),
            SpawnActionClass::Starlark => Some(self.get_inputs()),
            SpawnActionClass::EnhancedStarlark(d) => match &d.shadowed_action {
                None => Some(d.all_starlark_action_inputs.clone()),
                Some(shadowed) => {
                    let input_files_for_extra_action =
                        shadowed.get_input_files_for_extra_action()?;
                    Some(create_inputs(&[
                        input_files_for_extra_action,
                        d.all_starlark_action_inputs.clone(),
                    ]))
                }
            },
        }
    }

    // Java: com/google/devtools/build/lib/analysis/actions/SpawnAction.java:475 SpawnAction.getExtraActionInfo
    fn get_extra_action_info(
        &self,
        action_key_context: &crate::actions::action_key_context::ActionKeyContext,
    ) -> Result<blaze_protos::blaze::ExtraActionInfo, CommandLineError> {
        let mut info =
            crate::actions::action::abstract_get_extra_action_info(self, action_key_context)?;
        info.spawn_info = Some(self.get_extra_action_spawn_info()?);
        Ok(info)
    }

    // Java: com/google/devtools/build/lib/analysis/actions/SpawnAction.java:217 SpawnAction.getStarlarkArgs
    fn get_starlark_args(&self) -> Option<Vec<crate::analysis::starlark::args::RegisteredArgs>> {
        let mut result = Vec::new();
        // PORT: Java ImmutableSet (first occurrence order, no duplicates).
        let mut directory_inputs: IndexSet<Artifact> = IndexSet::new();
        for a in self.get_inputs().to_list().iter() {
            if a.is_directory() {
                directory_inputs.insert(a.clone());
            }
        }
        for command_line in self.command_lines.unpack() {
            result.push(
                crate::analysis::starlark::args::RegisteredArgs::for_registered_action(
                    command_line.command_line,
                    command_line.param_file_info,
                    directory_inputs.iter().cloned().collect(),
                ),
            );
        }
        Some(result)
    }

    // Java: com/google/devtools/build/lib/analysis/actions/SpawnAction.java:229 SpawnAction.getStarlarkArgv
    fn get_starlark_argv(
        &self,
    ) -> Result<Option<Vec<String>>, crate::actions::action::ActionApiError> {
        // Java: a CommandLineExpansionException becomes `new EvalException(ex)`.
        Ok(Some(self.get_arguments()?))
    }
}

impl CommandAction for SpawnAction {
    // Java: com/google/devtools/build/lib/analysis/actions/SpawnAction.java:207 SpawnAction.getArguments
    fn get_arguments(&self) -> Result<Vec<String>, CommandLineError> {
        self.command_lines
            .all_arguments_with(self.path_mapper_for_arguments().as_ref())
    }
}

/// `ShellCommand`: `<sh> -c <command> [""]`.
// Java: com/google/devtools/build/lib/analysis/actions/ShellCommand.java:31 ShellCommand
#[derive(Debug)]
pub struct ShellCommand {
    sh_executable: PathFragment,
    command: String,
    pad: bool,
}

impl ShellCommand {
    // Java: com/google/devtools/build/lib/analysis/actions/ShellCommand.java:37 ShellCommand.<init>
    pub fn new(sh_executable: PathFragment, command: String, pad: bool) -> ShellCommand {
        ShellCommand {
            sh_executable,
            command,
            pad,
        }
    }
}

impl CommandLine for ShellCommand {
    // Java: com/google/devtools/build/lib/analysis/actions/ShellCommand.java:44 ShellCommand.arguments
    fn arguments(&self) -> Result<Vec<String>, CommandLineError> {
        let sh = self.sh_executable.get_path_string().to_string();
        Ok(if self.pad {
            vec![sh, "-c".to_string(), self.command.clone(), String::new()]
        } else {
            vec![sh, "-c".to_string(), self.command.clone()]
        })
    }
}

/// `SpawnAction.Builder.executableArg`: a string, path or artifact (`ensureCallable`), or a
/// shell command.
#[derive(Clone)]
enum ExecutableArg {
    /// `PathFragment`, `String`, or `ensureCallable(artifact)`: expanded as a string.
    Str(String),
    ShellCommand(CommandLineRef),
}

/// Builder class to construct [`SpawnAction`] instances.
// Java: com/google/devtools/build/lib/analysis/actions/SpawnAction.java:618 SpawnAction.Builder
pub struct SpawnActionBuilder {
    tools_builder: NestedSetBuilder<Artifact>,
    inputs_builder: NestedSetBuilder<Artifact>,
    outputs: Vec<Artifact>,
    resource_set: ResourceSet,
    resource_set_builder: Option<Arc<dyn ResourceSetBuilder>>,
    environment: IndexMap<String, String>,
    action_environment: Option<Arc<ActionEnvironment>>,
    output_paths_mode: Option<OutputPathsMode>,
    execution_info: IndexMap<String, String>,
    use_default_shell_environment: bool,
    // Java: protected field for subclasses; no ported subclass reads it yet.
    #[allow(dead_code)]
    pub(crate) execute_unconditionally: bool,
    executable_arg: Option<ExecutableArg>,
    executable_args: Option<Vec<String>>,
    command_lines: Vec<CommandLineAndParamFileInfo>,
    progress_message: Option<String>,
    mnemonic: String,
    exec_group: String,
    /// `StarlarkAction.Builder` fields.
    pub(crate) starlark: Option<StarlarkBuilderFields>,
}

/// The fields `StarlarkAction.Builder` adds.
#[derive(Clone, Default)]
pub(crate) struct StarlarkBuilderFields {
    pub unused_inputs_list: Option<Artifact>,
    pub shadowed_action: Option<ActionRef>,
}

/// `DeclaredExecGroup.DEFAULT_EXEC_GROUP_NAME`.
// Java: com/google/devtools/build/lib/packages/DeclaredExecGroup.java:57 DeclaredExecGroup.DEFAULT_EXEC_GROUP_NAME
pub const DEFAULT_EXEC_GROUP_NAME: &str = "default-exec-group";

impl Default for SpawnActionBuilder {
    fn default() -> Self {
        SpawnActionBuilder::new()
    }
}

impl SpawnActionBuilder {
    /// Creates a SpawnAction builder.
    // Java: com/google/devtools/build/lib/analysis/actions/SpawnAction.java:639 SpawnAction.Builder.<init>
    pub fn new() -> SpawnActionBuilder {
        SpawnActionBuilder {
            tools_builder: NestedSetBuilder::stable_order(),
            inputs_builder: NestedSetBuilder::stable_order(),
            outputs: Vec::new(),
            resource_set: ResourceSet::default_resource_set(),
            resource_set_builder: None,
            environment: IndexMap::new(),
            action_environment: None,
            output_paths_mode: None,
            execution_info: IndexMap::new(),
            use_default_shell_environment: false,
            execute_unconditionally: false,
            executable_arg: None,
            executable_args: None,
            command_lines: Vec::new(),
            progress_message: None,
            mnemonic: "Unknown".to_string(),
            exec_group: DEFAULT_EXEC_GROUP_NAME.to_string(),
            starlark: None,
        }
    }

    /// The exec group whose action owner `build(context)` uses.
    pub fn get_exec_group(&self) -> &str {
        &self.exec_group
    }

    /// Builds the SpawnAction using the passed-in action configuration.
    ///
    /// PORT: Java's `build(ActionConstructionContext)` is `build(context.getActionOwner(execGroup),
    /// context.getConfiguration())`; the analysis layer resolves the owner for
    /// [`Self::get_exec_group`] and calls this.
    // Java: com/google/devtools/build/lib/analysis/actions/SpawnAction.java:674 SpawnAction.Builder.build
    pub fn build(
        &self,
        owner: ActionOwner,
        configuration: &dyn SpawnActionConfiguration,
    ) -> SpawnAction {
        let mut result = CommandLines::builder();
        if let Some(executable_arg) = &self.executable_arg {
            match executable_arg {
                ExecutableArg::Str(s) => {
                    result.add_single_argument(s.clone());
                }
                ExecutableArg::ShellCommand(c) => {
                    result.add_command_line(Arc::clone(c));
                }
            }
        } else if let Some(executable_args) = &self.executable_args {
            result.add_command_line(self.executable_args_command_line(executable_args));
        }
        for pair in &self.command_lines {
            result.add_command_line_and_param_file_info(pair.clone());
        }
        let command_lines = result.build();
        let env = SpawnAction::create_action_environment(
            configuration,
            self.use_default_shell_environment,
            &self.environment,
        );
        self.build_spawn_action(owner, command_lines, Some(configuration), env)
    }

    /// `executableArgs.build()` (a `CustomCommandLine` of plain strings).
    fn executable_args_command_line(&self, executable_args: &[String]) -> CommandLineRef {
        let mut b = CustomCommandLine::builder();
        for a in executable_args {
            b.add(a.clone());
        }
        Arc::new(b.build())
    }

    // Java: com/google/devtools/build/lib/analysis/actions/SpawnAction.java:697 SpawnAction.Builder.buildForActionTemplate
    pub fn build_for_action_template(&self, owner: ActionOwner) -> SpawnAction {
        self.build_spawn_action(
            owner,
            self.build_command_lines(),
            None,
            Arc::new(ActionEnvironment::create_fixed(self.environment.clone())),
        )
    }

    // Java: com/google/devtools/build/lib/analysis/actions/SpawnAction.java:703 SpawnAction.Builder.buildForStarlarkActionTemplate
    pub fn build_for_starlark_action_template(&mut self, owner: ActionOwner) -> SpawnAction {
        let tools = self.tools_builder.build();
        let inputs = self.inputs_builder.add_transitive(&tools).build();
        let mut action = SpawnAction::new(
            owner,
            tools,
            inputs,
            self.outputs.clone(),
            self.resource_set.clone(),
            self.build_command_lines(),
            self.action_environment
                .clone()
                .expect("NullPointerException: actionEnvironment"),
            &self.execution_info,
            self.progress_message.clone(),
            self.mnemonic.clone(),
            self.output_paths_mode
                .expect("NullPointerException: outputPathsMode"),
        );
        action.resource_set_builder = self.resource_set_builder.clone();
        action
    }

    // Java: com/google/devtools/build/lib/analysis/actions/SpawnAction.java:699 SpawnAction.Builder.buildCommandLines
    fn build_command_lines(&self) -> CommandLines {
        let mut result = CommandLines::builder();
        if let Some(executable_arg) = &self.executable_arg {
            match executable_arg {
                ExecutableArg::Str(s) => {
                    result.add_single_argument(s.clone());
                }
                ExecutableArg::ShellCommand(c) => {
                    result.add_command_line(Arc::clone(c));
                }
            }
        } else {
            let args = self
                .executable_args
                .as_ref()
                .expect("NullPointerException: executableArgs");
            result.add_command_line(self.executable_args_command_line(args));
        }
        for pair in &self.command_lines {
            result.add_command_line(Arc::clone(&pair.command_line));
        }
        result.build()
    }

    /// Builds the SpawnAction using the passed-in action configuration.
    // Java: com/google/devtools/build/lib/analysis/actions/SpawnAction.java:739 SpawnAction.Builder.buildSpawnAction
    fn build_spawn_action(
        &self,
        owner: ActionOwner,
        command_lines: CommandLines,
        configuration: Option<&dyn SpawnActionConfiguration>,
        env: Arc<ActionEnvironment>,
    ) -> SpawnAction {
        let tools = self.tools_builder.build();

        // Don't call getInputsAndTools - it wouldn't reuse the built set of tools.
        let inputs_and_tools = NestedSetBuilder::stable_order()
            .add_transitive(&self.inputs_builder.build())
            .add_transitive(&tools)
            .build();

        let mut outputs: IndexSet<Artifact> = IndexSet::new();
        for o in &self.outputs {
            outputs.insert(o.clone());
        }
        let execution_info = match configuration {
            None => self.execution_info.clone(),
            Some(c) => c.modified_execution_info(&self.execution_info, &self.mnemonic),
        };
        let mut action = self.create_spawn_action(
            owner,
            tools,
            inputs_and_tools,
            outputs.into_iter().collect(),
            self.resource_set.clone(),
            command_lines,
            env,
            configuration,
            execution_info,
            self.progress_message.clone(),
            self.mnemonic.clone(),
        );
        action.resource_set_builder = self.resource_set_builder.clone();
        action
    }

    // Java: com/google/devtools/build/lib/analysis/actions/SpawnAction.java:770 SpawnAction.Builder.createSpawnAction
    // Java: com/google/devtools/build/lib/analysis/actions/StarlarkAction.java:168 StarlarkAction.Builder.createSpawnAction
    fn create_spawn_action(
        &self,
        owner: ActionOwner,
        tools: NestedSet<Artifact>,
        inputs_and_tools: NestedSet<Artifact>,
        outputs: Vec<Artifact>,
        resource_set: ResourceSet,
        command_lines: CommandLines,
        env: Arc<ActionEnvironment>,
        configuration: Option<&dyn SpawnActionConfiguration>,
        execution_info: IndexMap<String, String>,
        progress_message: Option<String>,
        mnemonic: String,
    ) -> SpawnAction {
        // PathMappers.getOutputPathsMode(configuration)
        let output_paths_mode = match configuration {
            None => OutputPathsMode::OFF,
            Some(c) => c.get_output_paths_mode(),
        };
        let Some(starlark) = &self.starlark else {
            return SpawnAction::new(
                owner,
                tools,
                inputs_and_tools,
                outputs,
                resource_set,
                command_lines,
                env,
                &execution_info,
                progress_message,
                mnemonic,
                output_paths_mode,
            );
        };
        let mut execution_info = execution_info;
        if let Some(unused) = &starlark.unused_inputs_list {
            // Always download unused_inputs_list file from remote cache.
            // Java: ImmutableMap.builder().putAll(executionInfo).put(...).buildOrThrow()
            check_argument!(
                !execution_info
                    .contains_key(execution_requirements::REMOTE_EXECUTION_INLINE_OUTPUTS),
                "Multiple entries with same key: {}",
                execution_requirements::REMOTE_EXECUTION_INLINE_OUTPUTS
            );
            execution_info.insert(
                execution_requirements::REMOTE_EXECUTION_INLINE_OUTPUTS.to_string(),
                unused.get_exec_path_string().to_string(),
            );
        }
        if starlark.unused_inputs_list.is_some() || starlark.shadowed_action.is_some() {
            // Java: com/google/devtools/build/lib/analysis/actions/StarlarkAction.java:234 EnhancedStarlarkAction.<init>
            let inputs = match &starlark.shadowed_action {
                Some(shadowed) => create_inputs(&[shadowed.get_inputs(), inputs_and_tools.clone()]),
                None => inputs_and_tools.clone(),
            };
            SpawnAction::new_with_class(
                owner,
                tools,
                inputs,
                outputs,
                resource_set,
                command_lines,
                env,
                &execution_info,
                progress_message,
                mnemonic,
                output_paths_mode,
                SpawnActionClass::EnhancedStarlark(EnhancedStarlarkData {
                    all_starlark_action_inputs: inputs_and_tools,
                    unused_inputs_list: starlark.unused_inputs_list.clone(),
                    shadowed_action: starlark.shadowed_action.clone(),
                    inputs_discovered: Arc::new(parking_lot::Mutex::new(false)),
                }),
            )
        } else {
            SpawnAction::new_with_class(
                owner,
                tools,
                inputs_and_tools,
                outputs,
                resource_set,
                command_lines,
                env,
                &execution_info,
                progress_message,
                mnemonic,
                output_paths_mode,
                SpawnActionClass::Starlark,
            )
        }
    }

    // Java: com/google/devtools/build/lib/analysis/actions/SpawnAction.java:803 SpawnAction.Builder.addTool
    pub fn add_tool(&mut self, tool: Artifact) -> &mut Self {
        self.tools_builder.add(tool);
        self
    }

    // Java: com/google/devtools/build/lib/analysis/actions/SpawnAction.java:821 SpawnAction.Builder.addInput
    pub fn add_input(&mut self, artifact: Artifact) -> &mut Self {
        self.inputs_builder.add(artifact);
        self
    }

    // Java: com/google/devtools/build/lib/analysis/actions/SpawnAction.java:828 SpawnAction.Builder.addTools
    pub fn add_tools(&mut self, artifacts: impl IntoIterator<Item = Artifact>) -> &mut Self {
        self.tools_builder.add_all(artifacts);
        self
    }

    // Java: com/google/devtools/build/lib/analysis/actions/SpawnAction.java:815 SpawnAction.Builder.addTransitiveTools
    pub fn add_transitive_tools(&mut self, artifacts: &NestedSet<Artifact>) -> &mut Self {
        self.tools_builder.add_transitive(artifacts);
        self
    }

    // Java: com/google/devtools/build/lib/analysis/actions/SpawnAction.java:842 SpawnAction.Builder.addInputs
    pub fn add_inputs(&mut self, artifacts: impl IntoIterator<Item = Artifact>) -> &mut Self {
        self.inputs_builder.add_all(artifacts);
        self
    }

    // Java: com/google/devtools/build/lib/analysis/actions/SpawnAction.java:855 SpawnAction.Builder.getInputsAndTools
    pub fn get_inputs_and_tools(&self) -> NestedSet<Artifact> {
        NestedSetBuilder::stable_order()
            .add_transitive(&self.inputs_builder.build())
            .add_transitive(&self.tools_builder.build())
            .build()
    }

    // Java: com/google/devtools/build/lib/analysis/actions/SpawnAction.java:864 SpawnAction.Builder.addTransitiveInputs
    pub fn add_transitive_inputs(&mut self, artifacts: &NestedSet<Artifact>) -> &mut Self {
        self.inputs_builder.add_transitive(artifacts);
        self
    }

    // Java: com/google/devtools/build/lib/analysis/actions/SpawnAction.java:870 SpawnAction.Builder.addOutput
    pub fn add_output(&mut self, artifact: Artifact) -> &mut Self {
        self.outputs.push(artifact);
        self
    }

    // Java: com/google/devtools/build/lib/analysis/actions/SpawnAction.java:876 SpawnAction.Builder.addOutputs
    pub fn add_outputs(&mut self, artifacts: impl IntoIterator<Item = Artifact>) -> &mut Self {
        self.outputs.extend(artifacts);
        self
    }

    // Java: com/google/devtools/build/lib/analysis/actions/SpawnAction.java:886 SpawnAction.Builder.setResources
    pub fn set_resources(&mut self, resource_set: ResourceSet) -> &mut Self {
        self.resource_set = resource_set;
        self.resource_set_builder = None;
        self
    }

    /// `setResources(ResourceSetOrBuilder)` with a builder (Java's single slot: the last call
    /// wins over [`set_resources`](Self::set_resources)).
    // Java: com/google/devtools/build/lib/analysis/actions/SpawnAction.java:886 SpawnAction.Builder.setResources
    pub fn set_resources_builder(&mut self, builder: Arc<dyn ResourceSetBuilder>) -> &mut Self {
        self.resource_set = ResourceSet::default_resource_set();
        self.resource_set_builder = Some(builder);
        self
    }

    /// Sets the map of environment variables. Do not use on the same builder as
    /// `use_default_shell_environment`.
    // Java: com/google/devtools/build/lib/analysis/actions/SpawnAction.java:899 SpawnAction.Builder.setEnvironment
    pub fn set_environment(&mut self, environment: IndexMap<String, String>) -> &mut Self {
        self.environment = environment;
        self.use_default_shell_environment = false;
        self
    }

    // Java: com/google/devtools/build/lib/analysis/actions/SpawnAction.java:912 SpawnAction.Builder.setActionEnvironment
    pub fn set_action_environment(
        &mut self,
        action_environment: Arc<ActionEnvironment>,
    ) -> &mut Self {
        self.action_environment = Some(action_environment);
        self
    }

    // Java: com/google/devtools/build/lib/analysis/actions/SpawnAction.java:924 SpawnAction.Builder.setOutputPathsMode
    pub fn set_output_paths_mode(&mut self, output_paths_mode: OutputPathsMode) -> &mut Self {
        self.output_paths_mode = Some(output_paths_mode);
        self
    }

    /// Sets the map of execution info.
    // Java: com/google/devtools/build/lib/analysis/actions/SpawnAction.java:931 SpawnAction.Builder.setExecutionInfo
    pub fn set_execution_info(&mut self, info: IndexMap<String, String>) -> &mut Self {
        self.execution_info = info;
        self
    }

    /// Sets the environment to the configuration's default shell environment, with the given
    /// fixed variables overriding it.
    // Java: com/google/devtools/build/lib/analysis/actions/SpawnAction.java:969 SpawnAction.Builder.useDefaultShellEnvironment
    pub fn use_default_shell_environment(
        &mut self,
        environment: IndexMap<String, String>,
    ) -> &mut Self {
        self.environment = environment;
        self.use_default_shell_environment = true;
        self
    }

    /// Sets the executable path; the path is interpreted relative to the execution root.
    // Java: com/google/devtools/build/lib/analysis/actions/SpawnAction.java:987 SpawnAction.Builder.setExecutable
    pub fn set_executable_path(&mut self, executable: &PathFragment) -> &mut Self {
        // Java: CommandLines.SingletonCommandLine expands a PathFragment with
        // CommandLineItem.expandToCommandLine (its path string).
        self.executable_arg = Some(ExecutableArg::Str(executable.get_path_string().to_string()));
        self.executable_args = None;
        self
    }

    /// Sets the executable as an artifact.
    // Java: com/google/devtools/build/lib/analysis/actions/SpawnAction.java:987 SpawnAction.Builder.setExecutable
    pub fn set_executable(&mut self, executable: Artifact) -> &mut Self {
        self.add_tool(executable.clone());
        self.executable_arg = Some(ExecutableArg::Str(ensure_callable(&executable)));
        self.executable_args = None;
        self
    }

    /// Sets the executable from a `FilesToRunProvider`: its executable and its files to run.
    // Java: com/google/devtools/build/lib/analysis/actions/SpawnAction.java:987 SpawnAction.Builder.setExecutable
    pub fn set_executable_files_to_run(
        &mut self,
        executable: Option<Artifact>,
        files_to_run: &NestedSet<Artifact>,
    ) -> &mut Self {
        let executable = executable.expect("The target does not have an executable");
        self.executable_arg = Some(ExecutableArg::Str(ensure_callable(&executable)));
        self.executable_args = None;
        // Java: addTool(FilesToRunProvider) = addTransitiveTools(tool.getFilesToRun()).
        self.add_transitive_tools(files_to_run)
    }

    /// Sets the executable as a String.
    // Java: com/google/devtools/build/lib/analysis/actions/SpawnAction.java:1049 SpawnAction.Builder.setExecutableAsString
    pub fn set_executable_as_string(&mut self, executable: impl Into<String>) -> &mut Self {
        self.executable_arg = Some(ExecutableArg::Str(executable.into()));
        self.executable_args = None;
        self
    }

    /// Sets the executable to be the shell and adds the given command as the command to be
    /// executed.
    // Java: com/google/devtools/build/lib/analysis/actions/SpawnAction.java:1088 SpawnAction.Builder.setShellCommand
    pub fn set_shell_command(
        &mut self,
        sh_executable: &PathFragment,
        command: impl Into<String>,
        pad: bool,
    ) -> &mut Self {
        self.executable_arg = Some(ExecutableArg::ShellCommand(Arc::new(ShellCommand::new(
            sh_executable.clone(),
            command.into(),
            pad,
        ))));
        self.executable_args = None;
        self
    }

    /// Sets the executable to a shell command (already split into an argv).
    // Java: com/google/devtools/build/lib/analysis/actions/SpawnAction.java:1088 SpawnAction.Builder.setShellCommand
    pub fn set_shell_command_argv(&mut self, command: Vec<String>, pad: bool) -> &mut Self {
        let mut args = command;
        if pad {
            args.push(String::new());
        }
        self.executable_args = Some(args);
        self.executable_arg = None;
        self
    }

    /// Appends the arguments to the list of executable arguments.
    // Java: com/google/devtools/build/lib/analysis/actions/SpawnAction.java:1110 SpawnAction.Builder.addExecutableArguments
    pub fn add_executable_arguments(&mut self, arguments: &[&str]) -> &mut Self {
        if let Some(arg) = self.executable_arg.take() {
            match arg {
                ExecutableArg::Str(s) => self.executable_args = Some(vec![s]),
                ExecutableArg::ShellCommand(_) => {
                    blaze_util::unported!(
                        "SpawnAction.Builder.addExecutableArguments(ShellCommand)"
                    )
                }
            }
        }
        let args = self
            .executable_args
            .as_mut()
            .expect("NullPointerException: executableArgs");
        args.extend(arguments.iter().map(|s| s.to_string()));
        self
    }

    /// Adds a delegate to compute the command line at a later time.
    // Java: com/google/devtools/build/lib/analysis/actions/SpawnAction.java:1131 SpawnAction.Builder.addCommandLine
    pub fn add_command_line(&mut self, command_line: CommandLineRef) -> &mut Self {
        self.command_lines.push(CommandLineAndParamFileInfo {
            command_line,
            param_file_info: None,
        });
        self
    }

    /// Adds a delegate to compute the command line at a later time, optionally spilled to a
    /// params file.
    // Java: com/google/devtools/build/lib/analysis/actions/SpawnAction.java:1131 SpawnAction.Builder.addCommandLine
    pub fn add_command_line_with_param_file(
        &mut self,
        command_line: CommandLineRef,
        param_file_info: Option<ParamFileInfo>,
    ) -> &mut Self {
        self.command_lines.push(CommandLineAndParamFileInfo {
            command_line,
            param_file_info,
        });
        self
    }

    /// Sets the progress message.
    // Java: com/google/devtools/build/lib/analysis/actions/SpawnAction.java:1163 SpawnAction.Builder.setProgressMessage
    pub fn set_progress_message(&mut self, progress_message: impl Into<String>) -> &mut Self {
        self.progress_message = Some(progress_message.into());
        self
    }

    // Java: com/google/devtools/build/lib/analysis/actions/SpawnAction.java:1252 SpawnAction.Builder.setProgressMessageFromStarlark
    pub fn set_progress_message_from_starlark(
        &mut self,
        progress_message: impl Into<String>,
    ) -> &mut Self {
        self.progress_message = Some(progress_message.into());
        self
    }

    // Java: com/google/devtools/build/lib/analysis/actions/SpawnAction.java:1258 SpawnAction.Builder.setMnemonic
    pub fn set_mnemonic(&mut self, mnemonic: impl Into<String>) -> &mut Self {
        let mnemonic = mnemonic.into();
        check_argument!(
            is_valid_mnemonic(&mnemonic),
            "mnemonic must only contain letters and/or digits, and have non-zero length, was: \
             \"{}\"",
            mnemonic
        );
        self.mnemonic = mnemonic;
        self
    }

    // Java: com/google/devtools/build/lib/analysis/actions/SpawnAction.java:1273 SpawnAction.Builder.setExecGroup
    pub fn set_exec_group(&mut self, exec_group: impl Into<String>) -> &mut Self {
        self.exec_group = exec_group.into();
        self
    }
}

// Java: com/google/devtools/build/lib/analysis/actions/SpawnAction.java:1002 SpawnAction.ensureCallable
fn ensure_callable(executable: &Artifact) -> String {
    let exec_path = executable.get_exec_path();
    // Either the artifact itself (expanded to its exec path) or its callable path string.
    exec_path.get_callable_path_string()
}

// Java: com/google/devtools/build/lib/analysis/actions/StarlarkAction.java:140 StarlarkAction.createInputs
pub(crate) fn create_inputs(inputs_lists: &[NestedSet<Artifact>]) -> NestedSet<Artifact> {
    let mut nested_set_builder = NestedSetBuilder::new_builder(Order::STABLE_ORDER);
    for inputs in inputs_lists {
        nested_set_builder.add_transitive(inputs);
    }
    nested_set_builder.build()
}

/// `CommandLine.of`, re-exported for builders.
pub fn flat_command_line(args: Vec<String>) -> CommandLineRef {
    command_line::of(args)
}
