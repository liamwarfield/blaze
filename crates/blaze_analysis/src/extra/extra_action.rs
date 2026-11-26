//! Port of `analysis/extra/ExtraAction.java` (its analysis half; the execution half is
//! [`super::execution`]).

use std::any::Any;
use std::fmt;
use std::sync::Arc;

use blaze_actions::actions::action::{
    AbstractAction, Action, CommandAction, CommandLineError, ComputeKeyError, InputMetadataProvider,
};
use blaze_actions::actions::command_line::CommandLineRef;
use blaze_actions::actions::command_lines::CommandLines;
use blaze_actions::actions::path_mapper::OutputPathsMode;
use blaze_actions::analysis::actions::spawn_action::{ResourceSet, SpawnAction};
use blaze_actions::{ActionEnvironment, ActionKeyContext, ActionOwner, ActionRef, Artifact};
use blaze_collect::command_line_item::CommandLineExpansionException;
use blaze_collect::{NestedSet, NestedSetBuilder, Order};
use blaze_util::{Fingerprint, check_argument};
use indexmap::IndexMap;
use parking_lot::Mutex;

/// Action used by extra_action rules to create an action that shadows an existing action. Runs a
/// command-line using `SpawnStrategy` for executions.
///
/// PORT: Java's `ExtraAction extends SpawnAction`. Here the `SpawnAction` part is an inner
/// [`SpawnAction`] built with the same arguments; every method `SpawnAction` overrides is
/// delegated to it, and the methods `ExtraAction` overrides are overridden here.
// Java: com/google/devtools/build/lib/analysis/extra/ExtraAction.java:57 ExtraAction
pub struct ExtraAction {
    spawn_action: SpawnAction,
    shadowed_action: ActionRef,
    create_dummy_output: bool,
    extra_action_inputs: NestedSet<Artifact>,
    inputs_discovered: Mutex<bool>,
}

impl fmt::Debug for ExtraAction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_string_java())
    }
}

impl ExtraAction {
    #[allow(clippy::too_many_arguments)]
    // Java: com/google/devtools/build/lib/analysis/extra/ExtraAction.java:63 ExtraAction.<init>
    pub fn new(
        owner: ActionOwner,
        extra_action_inputs: NestedSet<Artifact>,
        outputs: Vec<Artifact>,
        shadowed_action: ActionRef,
        create_dummy_output: bool,
        argv: CommandLineRef,
        env: Arc<ActionEnvironment>,
        execution_info: IndexMap<String, String>,
        progress_message: String,
        mnemonic: String,
    ) -> ExtraAction {
        let output_count = outputs.len();
        let spawn_action = SpawnAction::new(
            owner,
            /* tools= */ NestedSetBuilder::empty_set(Order::STABLE_ORDER),
            create_inputs(
                &shadowed_action.get_inputs(),
                /* input_files_for_extra_action= */
                &NestedSetBuilder::empty_set(Order::STABLE_ORDER),
                &extra_action_inputs,
            ),
            outputs,
            ResourceSet::default_resource_set(),
            CommandLines::of(argv),
            env,
            &execution_info,
            Some(progress_message),
            mnemonic,
            OutputPathsMode::OFF,
        );
        if create_dummy_output {
            // Expecting just a single dummy file in the outputs.
            check_argument!(output_count == 1, "{:?}", spawn_action.get_outputs());
        }
        ExtraAction {
            spawn_action,
            shadowed_action,
            create_dummy_output,
            extra_action_inputs,
            inputs_discovered: Mutex::new(false),
        }
    }

    /// The `SpawnAction` this action is.
    pub fn as_spawn_action(&self) -> &SpawnAction {
        &self.spawn_action
    }

    /// Returns the action this extra action is 'shadowing'.
    // Java: com/google/devtools/build/lib/analysis/extra/ExtraAction.java:235 ExtraAction.getShadowedAction
    pub fn get_shadowed_action(&self) -> &ActionRef {
        &self.shadowed_action
    }

    /// `createDummyOutput`.
    pub fn creates_dummy_output(&self) -> bool {
        self.create_dummy_output
    }

    /// `extraActionInputs`.
    pub fn get_extra_action_inputs(&self) -> &NestedSet<Artifact> {
        &self.extra_action_inputs
    }
}

// Java: com/google/devtools/build/lib/analysis/extra/ExtraAction.java:186 ExtraAction.createInputs
pub(crate) fn create_inputs(
    shadowed_action_inputs: &NestedSet<Artifact>,
    input_files_for_extra_action: &NestedSet<Artifact>,
    extra_action_inputs: &NestedSet<Artifact>,
) -> NestedSet<Artifact> {
    let mut builder = NestedSetBuilder::new_builder(Order::STABLE_ORDER);
    builder.add_transitive(shadowed_action_inputs);
    builder.add_transitive(input_files_for_extra_action);
    builder.add_transitive(extra_action_inputs);
    builder.build()
}

impl Action for ExtraAction {
    fn abstract_action(&self) -> &AbstractAction {
        self.spawn_action.abstract_action()
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn get_mnemonic(&self) -> String {
        self.spawn_action.get_mnemonic()
    }

    fn compute_key(
        &self,
        action_key_context: &ActionKeyContext,
        input_metadata_provider: Option<&dyn InputMetadataProvider>,
        fp: &mut Fingerprint,
    ) -> Result<(), ComputeKeyError> {
        self.spawn_action
            .compute_key(action_key_context, input_metadata_provider, fp)
    }

    fn describe_key(&self) -> Option<String> {
        self.spawn_action.describe_key()
    }

    fn get_tools(&self) -> NestedSet<Artifact> {
        self.spawn_action.get_tools()
    }

    fn get_environment(&self) -> Arc<ActionEnvironment> {
        self.spawn_action.get_environment()
    }

    fn is_volatile(&self) -> bool {
        self.spawn_action.is_volatile()
    }

    fn get_raw_progress_message(&self) -> Option<String> {
        self.spawn_action.get_raw_progress_message()
    }

    fn get_execution_info(&self) -> IndexMap<String, String> {
        self.spawn_action.get_execution_info()
    }

    fn is_shareable(&self) -> bool {
        self.spawn_action.is_shareable()
    }

    fn get_effective_environment(
        &self,
        client_env: &IndexMap<String, String>,
    ) -> Result<IndexMap<String, String>, CommandLineExpansionException> {
        self.spawn_action.get_effective_environment(client_env)
    }

    fn as_command_action(&self) -> Option<&dyn CommandAction> {
        Some(self)
    }

    fn get_extra_action_info(
        &self,
        action_key_context: &ActionKeyContext,
    ) -> Result<blaze_protos::blaze::ExtraActionInfo, CommandLineError> {
        self.spawn_action.get_extra_action_info(action_key_context)
    }

    fn get_starlark_args(
        &self,
    ) -> Option<Vec<blaze_actions::analysis::starlark::args::RegisteredArgs>> {
        self.spawn_action.get_starlark_args()
    }

    fn get_starlark_argv(
        &self,
    ) -> Result<Option<Vec<String>>, blaze_actions::actions::action::ActionApiError> {
        self.spawn_action.get_starlark_argv()
    }

    // Java: com/google/devtools/build/lib/analysis/extra/ExtraAction.java:138 ExtraAction.discoversInputs
    fn discovers_inputs(&self) -> bool {
        self.shadowed_action.discovers_inputs()
    }

    // Java: com/google/devtools/build/lib/analysis/extra/ExtraAction.java:143 ExtraAction.inputsDiscovered
    fn inputs_discovered(&self) -> bool {
        *self.inputs_discovered.lock()
    }

    // Java: com/google/devtools/build/lib/analysis/extra/ExtraAction.java:148 ExtraAction.setInputsDiscovered
    fn set_inputs_discovered(&self, inputs_discovered: bool) {
        *self.inputs_discovered.lock() = inputs_discovered;
    }

    // Java: com/google/devtools/build/lib/analysis/extra/ExtraAction.java:177 ExtraAction.getOriginalInputs
    fn get_original_inputs(&self) -> NestedSet<Artifact> {
        self.shadowed_action.get_original_inputs()
    }

    // Java: com/google/devtools/build/lib/analysis/extra/ExtraAction.java:182 ExtraAction.getSchedulingDependencies
    fn get_scheduling_dependencies(&self) -> NestedSet<Artifact> {
        self.shadowed_action.get_scheduling_dependencies()
    }

    // Java: com/google/devtools/build/lib/analysis/extra/ExtraAction.java:198 ExtraAction.getAllowedDerivedInputs
    fn get_allowed_derived_inputs(&self) -> NestedSet<Artifact> {
        self.shadowed_action.get_allowed_derived_inputs()
    }
}

impl CommandAction for ExtraAction {
    fn get_arguments(&self) -> Result<Vec<String>, CommandLineError> {
        self.spawn_action.get_arguments()
    }
}
