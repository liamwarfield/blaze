//! Ports of `lib/actions/Action.java`, `ActionAnalysisMetadata.java`,
//! `ActionExecutionMetadata.java`, `ActionKeyComputer.java`, `AbstractAction.java`,
//! `CommandAction.java` and the parts of `InputMetadataProvider.java` key computation reads.
//!
//! PORT: Java's interface/abstract-class stack (ActionAnalysisMetadata → ActionExecutionMetadata
//! → Action, ActionKeyComputer → AbstractAction) becomes one object-safe trait [`Action`] whose
//! provided methods are AbstractAction's implementations, plus the [`AbstractAction`] struct
//! holding AbstractAction's fields. An action type embeds an `AbstractAction`, returns it from
//! [`Action::abstract_action`], and overrides what its Java class overrides. Java `instanceof`
//! tests on interfaces become the `as_*` hooks.

use std::any::Any;
use std::fmt;
use std::sync::Arc;

use blaze_cmdline::RepositoryMapping;
use blaze_collect::command_line_item::{
    CommandLineExpansionException, InterruptedException, MapFnError,
};
use blaze_collect::{NestedSet, NestedSetBuilder, Order};
use blaze_util::{Fingerprint, check_argument, check_state};
use indexmap::{IndexMap, IndexSet};
use parking_lot::RwLock;

use super::action_environment::ActionEnvironment;
use super::action_key_context::ActionKeyContext;
use super::action_owner::ActionOwner;
use super::artifact::Artifact;
use super::platform_info::PlatformInfoRef;

/// The members of `InputMetadataProvider` used while computing keys and expanding command lines.
// Java: com/google/devtools/build/lib/actions/InputMetadataProvider.java:33 InputMetadataProvider
// PORT: only `getTreeMetadata(...).getChildren()` is read during key computation; the full
// interface belongs to the execution layer.
pub trait InputMetadataProvider: Send + Sync {
    /// The children of a tree artifact (`getTreeMetadata(tree).getChildren()`), or `None` where
    /// Java returns null.
    fn get_tree_children(&self, tree: &Artifact) -> Option<Vec<Artifact>>;

    /// `getInputMetadata(input)` then `getDigest()`: `Ok(None)` where the metadata or its digest
    /// is null. The default answers `Ok(None)`: a provider that holds no file metadata (the
    /// analysis-time key fakes of the tests).
    // Java: com/google/devtools/build/lib/actions/InputMetadataProvider.java:75 InputMetadataProvider.getInputMetadata
    fn get_input_digest(&self, _input: &Artifact) -> std::io::Result<Option<Vec<u8>>> {
        Ok(None)
    }
}

/// Errors `computeKey` may throw: `CommandLineExpansionException`, `EvalException`,
/// `InterruptedException`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ComputeKeyError {
    CommandLineExpansion(CommandLineExpansionException),
    /// `net.starlark.java.eval.EvalException` (its message).
    Eval(String),
    Interrupted(InterruptedException),
}

impl fmt::Display for ComputeKeyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ComputeKeyError::CommandLineExpansion(e) => fmt::Display::fmt(e, f),
            ComputeKeyError::Eval(e) => f.write_str(e),
            ComputeKeyError::Interrupted(e) => fmt::Display::fmt(e, f),
        }
    }
}

impl std::error::Error for ComputeKeyError {}

impl From<CommandLineExpansionException> for ComputeKeyError {
    fn from(e: CommandLineExpansionException) -> Self {
        ComputeKeyError::CommandLineExpansion(e)
    }
}

impl From<InterruptedException> for ComputeKeyError {
    fn from(e: InterruptedException) -> Self {
        ComputeKeyError::Interrupted(e)
    }
}

impl From<MapFnError> for ComputeKeyError {
    fn from(e: MapFnError) -> Self {
        match e {
            MapFnError::CommandLineExpansion(e) => ComputeKeyError::CommandLineExpansion(e),
            MapFnError::Interrupted(e) => ComputeKeyError::Interrupted(e),
        }
    }
}

/// Errors a command line expansion may throw: `CommandLineExpansionException`,
/// `InterruptedException`.
pub type CommandLineError = MapFnError;

/// Returned by `getKey` if the key cannot be computed.
// Java: com/google/devtools/build/lib/actions/ActionAnalysisMetadata.java:37 ActionAnalysisMetadata.KEY_ERROR
pub const KEY_ERROR: &str = "1ea50e01-0349-4552-80cf-76cf520e8592";

/// Integer embedded in every action key.
///
/// The purpose of this member and associated property is to allow to easily invalidate the action
/// cache in case we want to mitigate bugs resulting with false-sharing.
// Java: com/google/devtools/build/lib/actions/ActionKeyComputer.java:33 ActionKeyComputer.ACTION_KEY_UNIQUIFIER
// PORT: Java reads the system property `ACTION_KEY_UNIQUIFIER` (default "0"); Blaze has no JVM
// system properties, so the default applies.
pub const ACTION_KEY_UNIQUIFIER: i32 = 0;

/// An arbitrary default resource set (250 MB RAM, 1 CPU).
// Java: com/google/devtools/build/lib/actions/AbstractAction.java:78 AbstractAction.DEFAULT_RESOURCE_SET
pub const DEFAULT_RESOURCE_SET_MEMORY_MB: f64 = 250.0;
pub const DEFAULT_RESOURCE_SET_CPU: f64 = 1.0;

/// `ActionAnalysisMetadata.mergeMaps`: `first` then `second`, later values win.
// Java: com/google/devtools/build/lib/actions/ActionAnalysisMetadata.java:252 ActionAnalysisMetadata.mergeMaps
pub fn merge_maps(
    first: &IndexMap<String, String>,
    second: &IndexMap<String, String>,
) -> IndexMap<String, String> {
    if first.is_empty() {
        return second.clone();
    }
    if second.is_empty() {
        return first.clone();
    }
    // PORT: Guava buildKeepingLast keeps the first position of a duplicated key with the last
    // value, as IndexMap::insert does.
    let mut m = first.clone();
    for (k, v) in second {
        m.insert(k.clone(), v.clone());
    }
    m
}

/// The fields of `AbstractAction`: owner, inputs (mutable for input-discovering actions) and
/// outputs.
// Java: com/google/devtools/build/lib/actions/AbstractAction.java:68 AbstractAction
pub struct AbstractAction {
    owner: ActionOwner,
    /// The variable inputs is non-final only so that actions that discover their inputs can
    /// modify it.
    inputs: RwLock<NestedSet<Artifact>>,
    /// Duplicate-free outputs, in insertion order.
    outputs: Vec<Artifact>,
}

impl AbstractAction {
    // Java: com/google/devtools/build/lib/actions/AbstractAction.java:94 AbstractAction.<init>
    pub fn new(
        owner: ActionOwner,
        inputs: NestedSet<Artifact>,
        outputs: impl IntoIterator<Item = Artifact>,
    ) -> AbstractAction {
        // Java: com/google/devtools/build/lib/actions/AbstractAction.java:96 AbstractAction.singletonOrArray
        let set: IndexSet<Artifact> = outputs.into_iter().collect();
        check_argument!(!set.is_empty(), "Action outputs may not be empty");
        AbstractAction {
            owner,
            inputs: RwLock::new(inputs),
            outputs: set.into_iter().collect(),
        }
    }

    pub fn owner(&self) -> &ActionOwner {
        &self.owner
    }

    pub fn inputs(&self) -> NestedSet<Artifact> {
        self.inputs.read().clone()
    }

    pub fn outputs(&self) -> &[Artifact] {
        &self.outputs
    }

    /// Replaces the inputs (Java's assignments to the `inputs` field by input-discovering
    /// actions).
    pub fn set_inputs(&self, inputs: NestedSet<Artifact>) {
        *self.inputs.write() = inputs;
    }
}

impl fmt::Debug for AbstractAction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AbstractAction")
            .field("owner", &self.owner.get_label())
            .field("outputs", &self.outputs)
            .finish()
    }
}

/// An Action represents a function from Artifacts to Artifacts executed as an atomic build step.
/// Examples include compilation of a single C++ source file, or linking a single library.
///
/// All subclasses of Action need to follow a strict set of invariants to ensure correctness on
/// incremental builds. In our experience, getting this wrong is a lot more expensive than any
/// benefits it might entail.
///
/// Notably, all Action subclasses must be effectively immutable: all fields must be immutable,
/// and all objects referenced by fields must also be immutable. Actions are compared by their
/// key ([`Action::get_key`]).
// Java: com/google/devtools/build/lib/actions/Action.java:80 Action
// Java: com/google/devtools/build/lib/actions/ActionAnalysisMetadata.java:27 ActionAnalysisMetadata
// Java: com/google/devtools/build/lib/actions/ActionExecutionMetadata.java:27 ActionExecutionMetadata
// Java: com/google/devtools/build/lib/actions/ActionKeyComputer.java:25 ActionKeyComputer
// Java: com/google/devtools/build/lib/actions/AbstractAction.java:68 AbstractAction
pub trait Action: Send + Sync + fmt::Debug + Any {
    /// The `AbstractAction` fields of this action.
    fn abstract_action(&self) -> &AbstractAction;

    /// For downcasting to the concrete action class (Java `instanceof` on classes).
    fn as_any(&self) -> &dyn Any;

    /// Returns the mnemonic for this action.
    // Java: com/google/devtools/build/lib/actions/AbstractAction.java:324 AbstractAction.getMnemonic
    // Java: com/google/devtools/build/lib/actions/ActionAnalysisMetadata.java:58 ActionAnalysisMetadata.getMnemonic
    fn get_mnemonic(&self) -> String;

    /// See the javadoc for `Action` and [`Action::get_key`] for the contract of this method.
    // Java: com/google/devtools/build/lib/actions/ActionKeyComputer.java:43 ActionKeyComputer.computeKey
    fn compute_key(
        &self,
        action_key_context: &ActionKeyContext,
        input_metadata_provider: Option<&dyn InputMetadataProvider>,
        fp: &mut Fingerprint,
    ) -> Result<(), ComputeKeyError>;

    /// Returns a string encoding all of the significant behaviour of this Action that might
    /// affect the output. The general contract of `getKey` is this: if the work to be performed
    /// by the execution of this action changes, the key must change.
    ///
    /// Do not override: Java's `ActionKeyComputer.getKey` is final.
    // Java: com/google/devtools/build/lib/actions/ActionKeyComputer.java:37 ActionKeyComputer.getKey
    fn get_key(
        &self,
        action_key_context: &ActionKeyContext,
        input_metadata_provider: Option<&dyn InputMetadataProvider>,
    ) -> Result<String, InterruptedException> {
        let mut fp = Fingerprint::new();
        match self.compute_key(action_key_context, input_metadata_provider, &mut fp) {
            Ok(()) => {}
            Err(ComputeKeyError::Interrupted(e)) => return Err(e),
            Err(ComputeKeyError::CommandLineExpansion(_)) | Err(ComputeKeyError::Eval(_)) => {
                return Ok(KEY_ERROR.to_string());
            }
        }
        match self.get_execution_platform() {
            None => {
                fp.add_boolean(false);
            }
            Some(execution_platform) => {
                fp.add_boolean(true);
                execution_platform.add_to(&mut fp);
            }
        }
        let exec_properties = self.get_exec_properties();
        // Invariant: exec properties are internal strings (see `fingerprint_util`).
        fp.add_string_map(
            exec_properties
                .iter()
                .map(|(k, v)| (k.as_str(), v.as_str())),
        );
        fp.add_int(ACTION_KEY_UNIQUIFIER);
        Ok(fp.hex_digest_and_reset())
    }

    // ---------------------------------------------------------------------------------------
    // AbstractAction
    // ---------------------------------------------------------------------------------------

    /// Returns the owner of this executable if this executable can supply verbose information.
    // Java: com/google/devtools/build/lib/actions/AbstractAction.java:118 AbstractAction.getOwner
    // Java: com/google/devtools/build/lib/actions/ActionAnalysisMetadata.java:43 ActionAnalysisMetadata.getOwner
    fn get_owner(&self) -> &ActionOwner {
        self.abstract_action().owner()
    }

    /// `this instanceof ActionApi` (the Starlark `Action` type, implemented by `AbstractAction`):
    /// false for action templates, which override it.
    // Java: com/google/devtools/build/lib/starlarkbuildapi/ActionApi.java ActionApi
    fn is_action_api(&self) -> bool {
        true
    }

    /// `this instanceof ActionExecutionMetadata`: true for every action; false for action
    /// templates (`ActionTemplate` is only an `ActionAnalysisMetadata`), which override it.
    // Java: com/google/devtools/build/lib/actions/ActionExecutionMetadata.java:27 ActionExecutionMetadata
    fn is_action_execution_metadata(&self) -> bool {
        true
    }

    /// Returns true if inputs are known for this action.
    // Java: com/google/devtools/build/lib/actions/AbstractAction.java:123 AbstractAction.inputsKnown
    fn inputs_known(&self) -> bool {
        if !self.discovers_inputs() {
            return true;
        }
        self.inputs_discovered()
    }

    // Java: com/google/devtools/build/lib/actions/AbstractAction.java:139 AbstractAction.discoversInputs
    fn discovers_inputs(&self) -> bool {
        false
    }

    /// Returns true if inputs have been discovered.
    // Java: com/google/devtools/build/lib/actions/AbstractAction.java:173 AbstractAction.inputsDiscovered
    fn inputs_discovered(&self) -> bool {
        panic!(
            "IllegalStateException: Must be overridden by input-discovering action: {}",
            self.to_string_java()
        )
    }

    /// Informs input-discovering actions about their discovery state so that they can correctly
    /// implement [`Action::inputs_discovered`].
    // Java: com/google/devtools/build/lib/actions/AbstractAction.java:183 AbstractAction.setInputsDiscovered
    fn set_inputs_discovered(&self, _inputs_discovered: bool) {
        panic!(
            "IllegalStateException: Must be overridden by input-discovering action: {}",
            self.to_string_java()
        )
    }

    // Java: com/google/devtools/build/lib/actions/AbstractAction.java:151 AbstractAction.resetDiscoveredInputs
    fn reset_discovered_inputs(&self) {
        check_state!(
            self.discovers_inputs(),
            "Not an input-discovering action: {}",
            self.to_string_java()
        );
        if !self.inputs_known() {
            return;
        }
        self.abstract_action()
            .set_inputs(self.get_original_inputs());
        self.set_inputs_discovered(false);
    }

    /// Should be called when the inputs of the action become known, that is, either during
    /// `discoverInputs` or during `execute`. Also called by the action cache when an action's
    /// inputs are loaded from it.
    // Java: com/google/devtools/build/lib/actions/AbstractAction.java:215 AbstractAction.updateInputs
    fn update_inputs(&self, inputs: NestedSet<Artifact>) {
        check_state!(
            self.discovers_inputs(),
            "Not an input-discovering action: {}",
            self.to_string_java()
        );
        self.abstract_action().set_inputs(inputs);
        self.set_inputs_discovered(true);
    }

    // Java: com/google/devtools/build/lib/actions/AbstractAction.java:194 AbstractAction.getAllowedDerivedInputs
    fn get_allowed_derived_inputs(&self) -> NestedSet<Artifact> {
        panic!(
            "IllegalStateException: Must be overridden for action that may have unknown inputs: {}",
            self.to_string_java()
        )
    }

    // Java: com/google/devtools/build/lib/actions/AbstractAction.java:157 AbstractAction.getOriginalInputs
    // Java: com/google/devtools/build/lib/actions/ActionAnalysisMetadata.java:143 ActionAnalysisMetadata.getOriginalInputs
    fn get_original_inputs(&self) -> NestedSet<Artifact> {
        check_state!(
            !self.discovers_inputs(),
            "Must be overridden by input-discovering action"
        );
        self.get_inputs()
    }

    // Java: com/google/devtools/build/lib/actions/AbstractAction.java:200 AbstractAction.getSchedulingDependencies
    // Java: com/google/devtools/build/lib/actions/ActionAnalysisMetadata.java:152 ActionAnalysisMetadata.getSchedulingDependencies
    fn get_scheduling_dependencies(&self) -> NestedSet<Artifact> {
        NestedSetBuilder::empty_set(Order::STABLE_ORDER)
    }

    // Java: com/google/devtools/build/lib/actions/AbstractAction.java:222 AbstractAction.getTools
    // Java: com/google/devtools/build/lib/actions/ActionAnalysisMetadata.java:127 ActionAnalysisMetadata.getTools
    fn get_tools(&self) -> NestedSet<Artifact> {
        NestedSetBuilder::empty_set(Order::STABLE_ORDER)
    }

    // Java: com/google/devtools/build/lib/actions/AbstractAction.java:227 AbstractAction.getInputs
    // Java: com/google/devtools/build/lib/actions/ActionAnalysisMetadata.java:136 ActionAnalysisMetadata.getInputs
    fn get_inputs(&self) -> NestedSet<Artifact> {
        self.abstract_action().inputs()
    }

    // Java: com/google/devtools/build/lib/actions/AbstractAction.java:238 AbstractAction.getEnvironment
    fn get_environment(&self) -> Arc<ActionEnvironment> {
        Arc::new(ActionEnvironment::Empty)
    }

    /// Returns the environment variables to set for this action, resolved against `client_env`.
    // Java: com/google/devtools/build/lib/actions/AbstractAction.java:236 AbstractAction.getEffectiveEnvironment
    fn get_effective_environment(
        &self,
        client_env: &IndexMap<String, String>,
        _path_mapper: &dyn crate::actions::path_mapper::PathMapper,
    ) -> Result<IndexMap<String, String>, CommandLineExpansionException> {
        let env = self.get_environment();
        let mut effective_environment = IndexMap::with_capacity(env.estimated_size());
        env.resolve(&mut effective_environment, client_env);
        Ok(effective_environment)
    }

    /// Returns the set of client environment variables this action depends on.
    // Java: com/google/devtools/build/lib/actions/AbstractAction.java:246 AbstractAction.getClientEnvironmentVariables
    // Java: com/google/devtools/build/lib/actions/ActionAnalysisMetadata.java:163 ActionAnalysisMetadata.getClientEnvironmentVariables
    fn get_client_environment_variables(&self) -> Vec<String> {
        self.get_environment()
            .get_inherited_env()
            .into_iter()
            .collect()
    }

    // Java: com/google/devtools/build/lib/actions/AbstractAction.java:251 AbstractAction.getOutputs
    // Java: com/google/devtools/build/lib/actions/ActionAnalysisMetadata.java:170 ActionAnalysisMetadata.getOutputs
    fn get_outputs(&self) -> Vec<Artifact> {
        self.abstract_action().outputs().to_vec()
    }

    /// The default behavior is to return the first input artifact.
    // Java: com/google/devtools/build/lib/actions/AbstractAction.java:285 AbstractAction.getPrimaryInput
    fn get_primary_input(&self) -> Option<Artifact> {
        get_first_or_null(&self.get_inputs())
    }

    // Java: com/google/devtools/build/lib/actions/AbstractAction.java:311 AbstractAction.getPrimaryOutput
    // Java: com/google/devtools/build/lib/actions/ActionAnalysisMetadata.java:214 ActionAnalysisMetadata.getPrimaryOutput
    fn get_primary_output(&self) -> Artifact {
        self.abstract_action().outputs()[0].clone()
    }

    // Java: com/google/devtools/build/lib/actions/AbstractAction.java:316 AbstractAction.getMandatoryInputs
    // Java: com/google/devtools/build/lib/actions/ActionAnalysisMetadata.java:226 ActionAnalysisMetadata.getMandatoryInputs
    fn get_mandatory_inputs(&self) -> NestedSet<Artifact> {
        self.get_inputs()
    }

    // Java: com/google/devtools/build/lib/actions/AbstractAction.java:337 AbstractAction.describeKey
    fn describe_key(&self) -> Option<String> {
        None
    }

    // Java: com/google/devtools/build/lib/actions/AbstractAction.java:342 AbstractAction.executeUnconditionally
    fn execute_unconditionally(&self) -> bool {
        false
    }

    // Java: com/google/devtools/build/lib/actions/AbstractAction.java:347 AbstractAction.isVolatile
    fn is_volatile(&self) -> bool {
        false
    }

    // Java: com/google/devtools/build/lib/actions/AbstractAction.java:352 AbstractAction.isShareable
    // Java: com/google/devtools/build/lib/actions/ActionAnalysisMetadata.java:51 ActionAnalysisMetadata.isShareable
    fn is_shareable(&self) -> bool {
        true
    }

    // Java: com/google/devtools/build/lib/actions/AbstractAction.java:357 AbstractAction.showsOutputUnconditionally
    fn shows_output_unconditionally(&self) -> bool {
        false
    }

    /// Returns a progress message string that is specific for this action. This is then
    /// annotated with additional information, currently the string '[for tool]' for actions in
    /// the tool configurations.
    ///
    /// `None` indicates no message should be reported.
    // Java: com/google/devtools/build/lib/actions/AbstractAction.java:376 AbstractAction.getRawProgressMessage
    fn get_raw_progress_message(&self) -> Option<String> {
        // A cheesy default implementation. Subclasses are invited to do something more
        // meaningful.
        Some(self.default_progress_message())
    }

    // Java: com/google/devtools/build/lib/actions/AbstractAction.java:416 AbstractAction.defaultProgressMessage
    fn default_progress_message(&self) -> String {
        format!(
            "{} {}",
            self.get_mnemonic(),
            self.get_primary_output().pretty_print()
        )
    }

    /// `getProgressMessage()` / `getProgressMessage(mainRepositoryMapping)`.
    // Java: com/google/devtools/build/lib/actions/AbstractAction.java:363 AbstractAction.getProgressMessage
    // Java: com/google/devtools/build/lib/actions/AbstractAction.java:363 AbstractAction.getProgressMessage
    fn get_progress_message(
        &self,
        main_repository_mapping: Option<&RepositoryMapping>,
    ) -> Option<String> {
        // Java: com/google/devtools/build/lib/actions/AbstractAction.java:375 AbstractAction.getProgressMessageChecked
        let message = self.get_raw_progress_message()?;
        let message = self.replace_progress_message_placeholders(message, main_repository_mapping);
        Some(if self.get_owner().is_build_configuration_for_tool() {
            message + " [for tool]"
        } else {
            message
        })
    }

    // Java: com/google/devtools/build/lib/actions/AbstractAction.java:380 AbstractAction.replaceProgressMessagePlaceholders
    fn replace_progress_message_placeholders(
        &self,
        progress_message: String,
        main_repository_mapping: Option<&RepositoryMapping>,
    ) -> String {
        let mut progress_message = progress_message;
        if progress_message.contains("%{label}") {
            if let Some(label) = self.get_owner().get_label() {
                progress_message = progress_message
                    .replace("%{label}", &label.get_display_form(main_repository_mapping));
            }
        }
        if progress_message.contains("%{output}") {
            progress_message = progress_message.replace(
                "%{output}",
                &self.get_primary_output().get_root_relative_path_string(),
            );
        }
        if progress_message.contains("%{input}") {
            // Java: com/google/devtools/build/lib/actions/AbstractAction.java:292 AbstractAction.getOriginalPrimaryInput
            if let Some(input) = get_first_or_null(&self.get_original_inputs()) {
                progress_message =
                    progress_message.replace("%{input}", &input.get_root_relative_path_string());
            }
        }
        progress_message
    }

    // Java: com/google/devtools/build/lib/actions/AbstractAction.java:421 AbstractAction.prettyPrint
    // Java: com/google/devtools/build/lib/actions/ActionAnalysisMetadata.java:104 ActionAnalysisMetadata.prettyPrint
    fn pretty_print(&self) -> String {
        format!("action '{}'", self.describe())
    }

    // Java: com/google/devtools/build/lib/actions/AbstractAction.java:422 AbstractAction.describe
    // Java: com/google/devtools/build/lib/actions/ActionAnalysisMetadata.java:107 ActionAnalysisMetadata.describe
    fn describe(&self) -> String {
        match self.get_progress_message(None) {
            Some(m) => m,
            None => self.default_progress_message(),
        }
    }

    // Java: com/google/devtools/build/lib/actions/AbstractAction.java:321 AbstractAction.toString
    fn to_string_java(&self) -> String {
        let inputs = self.get_inputs().to_list();
        let outputs = self.get_outputs();
        format!(
            "{} ({}[{}{}{}])",
            self.pretty_print(),
            self.get_mnemonic(),
            java_list_to_string(inputs.iter()),
            if self.inputs_known() {
                " -> "
            } else {
                ", unknown inputs -> "
            },
            java_list_to_string(outputs.iter())
        )
    }

    // Java: com/google/devtools/build/lib/actions/AbstractAction.java:587 AbstractAction.getMandatoryOutputs
    // Java: com/google/devtools/build/lib/actions/ActionAnalysisMetadata.java:195 ActionAnalysisMetadata.getMandatoryOutputs
    fn get_mandatory_outputs(&self) -> Vec<Artifact> {
        Vec::new()
    }

    /// Returns the execution info (by default the owner's exec properties).
    // Java: com/google/devtools/build/lib/actions/ActionAnalysisMetadata.java:248 ActionAnalysisMetadata.getExecutionInfo
    fn get_execution_info(&self) -> IndexMap<String, String> {
        self.get_exec_properties()
    }

    // Java: com/google/devtools/build/lib/actions/AbstractAction.java:659 AbstractAction.getExecProperties
    fn get_exec_properties(&self) -> IndexMap<String, String> {
        self.get_owner().get_exec_properties().clone()
    }

    // Java: com/google/devtools/build/lib/actions/AbstractAction.java:665 AbstractAction.getExecutionPlatform
    fn get_execution_platform(&self) -> Option<PlatformInfoRef> {
        self.get_owner().get_execution_platform().cloned()
    }

    /// Returns artifacts that should be subject to path mapping, but aren't inputs of the action.
    // Java: com/google/devtools/build/lib/actions/AbstractAction.java:673 AbstractAction.getAdditionalArtifactsForPathMapping
    fn get_additional_artifacts_for_path_mapping(&self) -> NestedSet<Artifact> {
        NestedSetBuilder::empty_set(Order::STABLE_ORDER)
    }

    /// Returns the information extra actions receive about this action (written to the `.xa`
    /// file of an extra action, printed by `print_action`). Overrides call
    /// [`abstract_get_extra_action_info`] where Java calls `super.getExtraActionInfo`.
    // Java: com/google/devtools/build/lib/actions/Action.java:221 Action.getExtraActionInfo
    fn get_extra_action_info(
        &self,
        action_key_context: &ActionKeyContext,
    ) -> Result<blaze_protos::blaze::ExtraActionInfo, CommandLineError> {
        abstract_get_extra_action_info(self, action_key_context)
    }

    /// Returns input files that need to be present to allow extra_action rules to shadow this
    /// action correctly when run remotely. This is at least the normal inputs of the action, but
    /// may include other files as well. For example C(++) compilation may perform include file
    /// header scanning. This needs to be mirrored by the extra_action rule. Called by
    /// `ExtraAction.discoverInputs` at execution time.
    ///
    /// PORT: Java passes the `ActionExecutionContext` (for include scanning); the actions that
    /// need it fail loudly here. `None` where Java returns null (a Skyframe restart).
    // Java: com/google/devtools/build/lib/actions/AbstractAction.java:606 AbstractAction.getInputFilesForExtraAction
    // Java: com/google/devtools/build/lib/actions/ActionAnalysisMetadata.java:184 ActionAnalysisMetadata.getInputFilesForExtraAction
    fn get_input_files_for_extra_action(&self) -> Option<NestedSet<Artifact>> {
        Some(NestedSetBuilder::empty_set(Order::STABLE_ORDER))
    }

    // ---------------------------------------------------------------------------------------
    // Interface hooks (Java `instanceof` on interfaces)
    // ---------------------------------------------------------------------------------------

    /// `action instanceof CommandAction`.
    fn as_command_action(&self) -> Option<&dyn CommandAction> {
        None
    }

    /// `action instanceof AbstractFileWriteAction.FileContentsProvider`.
    fn as_file_contents_provider(&self) -> Option<&dyn FileContentsProvider> {
        None
    }

    /// `action instanceof TemplateExpansionAction` as aquery reads it: the template content
    /// (`AqueryUtils.getTemplateContent`) and the substitutions as (key, value) pairs; `Err` is the
    /// `EvalException`/`IOException` message.
    fn aquery_template_expansion(&self) -> Option<Result<(String, Vec<(String, String)>), String>> {
        None
    }

    /// `action instanceof UnresolvedSymlinkAction`: `getTarget()`.
    fn aquery_unresolved_symlink_target(&self) -> Option<String> {
        None
    }

    /// `action instanceof ParameterFileWriteAction`: `getArguments()`.
    fn aquery_parameter_file_arguments(&self) -> Option<Result<Vec<String>, CommandLineError>> {
        None
    }

    // ---------------------------------------------------------------------------------------
    // ActionApi (the Starlark `Action` value): the getters action classes override. The other
    // ActionApi getters (`mnemonic`, `inputs`, `outputs`, `env`, `execution_info`) are
    // AbstractAction's final implementations over the methods above.
    // PORT: every override at the pin is listed with its class; a class that is ported but whose
    // override is not must override these methods (CppCompileAction.getStarlarkArgv/Args,
    // JavaCompileAction.getStarlarkArgv/Args).
    // ---------------------------------------------------------------------------------------

    /// The `argv` of the action, `None` where Java returns null.
    // Java: com/google/devtools/build/lib/actions/AbstractAction.java:624 AbstractAction.getStarlarkArgv
    fn get_starlark_argv(&self) -> Result<Option<Vec<String>>, ActionApiError> {
        Ok(None)
    }

    /// The frozen `Args` objects of the action, `None` where Java returns null ("Not all action
    /// types support returning Args").
    // Java: com/google/devtools/build/lib/actions/AbstractAction.java:630 AbstractAction.getStarlarkArgs
    fn get_starlark_args(&self) -> Option<Vec<crate::analysis::starlark::args::RegisteredArgs>> {
        // Not all action types support returning Args.
        None
    }

    /// The contents of the file the action writes, `None` where Java returns null.
    // Java: com/google/devtools/build/lib/actions/AbstractAction.java:637 AbstractAction.getStarlarkContent
    fn get_starlark_content(&self) -> Result<Option<String>, ActionApiError> {
        Ok(None)
    }

    /// The substitution mapping of a template expansion, `None` where Java returns null.
    // Java: com/google/devtools/build/lib/actions/AbstractAction.java:643 AbstractAction.getStarlarkSubstitutions
    fn get_starlark_substitutions(
        &self,
    ) -> Result<Option<IndexMap<String, String>>, ActionApiError> {
        Ok(None)
    }
}

/// The exceptions the `ActionApi` getters throw to Starlark: `EvalException` (Java wraps every
/// other checked exception, such as `IOException` or `CommandLineExpansionException`, in an
/// `EvalException` carrying its message) and `InterruptedException`.
// Java: net/starlark/java/eval/MethodDescriptor.java:282 MethodDescriptor.call
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ActionApiError {
    /// The `EvalException` message.
    Eval(String),
    Interrupted(InterruptedException),
}

impl fmt::Display for ActionApiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ActionApiError::Eval(e) => f.write_str(e),
            ActionApiError::Interrupted(e) => fmt::Display::fmt(e, f),
        }
    }
}

impl std::error::Error for ActionApiError {}

impl From<CommandLineError> for ActionApiError {
    // Java: `new EvalException(commandLineExpansionException)` (message of the cause).
    fn from(e: CommandLineError) -> Self {
        match e {
            MapFnError::CommandLineExpansion(e) => ActionApiError::Eval(e.message),
            MapFnError::Interrupted(e) => ActionApiError::Interrupted(e),
        }
    }
}

/// `AbstractAction.getExtraActionInfo`: the owner's label, the action key and the mnemonic,
/// and the name and parameters of the owner's last aspect.
// Java: com/google/devtools/build/lib/actions/AbstractAction.java:563 AbstractAction.getExtraActionInfo
#[allow(deprecated)] // aspect_name, aspect_parameters: Java sets the deprecated fields.
pub fn abstract_get_extra_action_info<A: Action + ?Sized>(
    action: &A,
    action_key_context: &ActionKeyContext,
) -> Result<blaze_protos::blaze::ExtraActionInfo, CommandLineError> {
    use blaze_protos::blaze::extra_action_info;
    let owner = action.get_owner();
    let mut result = blaze_protos::blaze::ExtraActionInfo {
        // owner.getLabel().toString()
        owner: Some(match owner.get_label() {
            Some(label) => label.to_string(),
            None => panic!(
                "NullPointerException: owner label of {}",
                action.to_string_java()
            ),
        }),
        id: Some(
            action
                .get_key(action_key_context, /* input_metadata_provider= */ None)
                .map_err(MapFnError::Interrupted)?,
        ),
        mnemonic: Some(action.get_mnemonic()),
        ..Default::default()
    };
    let aspect_descriptors = owner.get_aspect_descriptors();
    if let Some(last_aspect) = aspect_descriptors.last() {
        result.aspect_name = Some(last_aspect.aspect_class_name.clone());

        // getParameters().getAttributes().asMap(): the values grouped by key, keys in first
        // appearance order.
        let mut grouped: IndexMap<&str, Vec<String>> = IndexMap::new();
        for (key, value) in &last_aspect.parameters {
            grouped.entry(key.as_str()).or_default().push(value.clone());
        }
        for (key, values) in grouped {
            // putAspectParameters: a later put of the same key replaces the entry in place.
            result
                .aspect_parameters
                .push(extra_action_info::AspectParametersEntry {
                    key: key.to_owned(),
                    value: Some(extra_action_info::StringList { value: values }),
                });
        }
    }
    Ok(result)
}

// Java: com/google/devtools/build/lib/actions/AbstractAction.java:300 AbstractAction.getFirstOrNull
fn get_first_or_null(inputs: &NestedSet<Artifact>) -> Option<Artifact> {
    if inputs.is_empty() {
        None
    } else if inputs.is_singleton() {
        Some(inputs.get_singleton().clone())
    } else {
        inputs.to_list().first().cloned()
    }
}

/// Java `AbstractCollection.toString`: `[a, b]`.
pub fn java_list_to_string<'a, T: fmt::Display + 'a>(items: impl Iterator<Item = &'a T>) -> String {
    let parts: Vec<String> = items.map(|i| i.to_string()).collect();
    format!("[{}]", parts.join(", "))
}

/// Shared handle to an action.
pub type ActionRef = Arc<dyn Action>;

/// Interface for Actions that can be run as commands.
// Java: com/google/devtools/build/lib/actions/CommandAction.java:26 CommandAction
pub trait CommandAction: Action {
    /// Returns a list of command line arguments that implements this action.
    ///
    /// In most cases, this expands any params files. One notable exception is C/C++ compilation
    /// with the "compiler_param_file" feature.
    // Java: com/google/devtools/build/lib/actions/CommandAction.java:34 CommandAction.getArguments
    fn get_arguments(&self) -> Result<Vec<String>, CommandLineError>;
}

/// Interface for actions that write files whose contents are known at analysis time.
// Java: com/google/devtools/build/lib/analysis/actions/AbstractFileWriteAction.java:118 AbstractFileWriteAction.FileContentsProvider
pub trait FileContentsProvider {
    // Java: com/google/devtools/build/lib/analysis/actions/AbstractFileWriteAction.java:119 FileContentsProvider.getFileContents
    fn get_file_contents(&self) -> Result<String, std::io::Error>;

    // Java: com/google/devtools/build/lib/analysis/actions/AbstractFileWriteAction.java:121 FileContentsProvider.makeExecutable
    fn make_executable(&self) -> bool;
}
