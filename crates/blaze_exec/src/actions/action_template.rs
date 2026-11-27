//! Port of `lib/actions/ActionTemplate.java`.
//!
//! An action template is registered by analysis like any other action (it is an
//! `ActionAnalysisMetadata`, so it sits in the `ActionLookupValue`'s action list as an
//! [`ActionRef`]) but is never executed: at execution time `ActionTemplateExpansionFunction`
//! expands it into concrete actions over the children of its input tree artifacts.
//!
//! PORT: Java tests `instanceof ActionTemplate`; here each template class registers itself once
//! with [`register_action_template`], and [`as_action_template`] finds the implementation for a
//! `&dyn Action` by its concrete type (as [`super::as_executable`] does for `Action.execute`).

use std::any::{Any, TypeId};
use std::collections::HashMap;

use blaze_actions::actions::action::{Action, ActionRef};
use blaze_actions::{ActionLookupKeyRef, Artifact};
use blaze_events::handler::EventHandler;
use parking_lot::RwLock;

use super::action_execution::ActionExecutionException;

/// What `generateActionsForInputArtifacts` throws: `ActionExecutionException` or
/// `InterruptedException`.
///
/// PORT: Java's signature also declares `ActionConflictException` (a `blaze_analysis` type); no
/// ported template throws it (`CppCompileActionTemplate` does not).
#[derive(Clone, Debug)]
pub enum ActionTemplateError {
    ActionExecution(ActionExecutionException),
    Interrupted,
}

impl From<ActionExecutionException> for ActionTemplateError {
    fn from(e: ActionExecutionException) -> Self {
        ActionTemplateError::ActionExecution(e)
    }
}

/// A placeholder action that, at execution time, expands into a list of `Action`s to be executed.
///
/// ActionTemplate is for users who want to dynamically register Actions operating on individual
/// `TreeFileArtifact` inside input and output TreeArtifacts at execution time.
///
/// It takes in one or more input TreeArtifacts and generates one or more output TreeArtifacts.
/// The following happens at execution time for ActionTemplate:
///
/// 1. Input TreeArtifact(s) are resolved.
/// 2. Given the set of `TreeFileArtifact`s inside each input TreeArtifact, generate actions with
///    outputs inside output TreeArtifact(s).
/// 3. All expanded `Action`s are executed and their output `TreeFileArtifact`s collected.
/// 4. Output TreeArtifact(s) are resolved.
///
/// Implementations of ActionTemplate must follow the contract of this interface and also make
/// sure:
///
/// 1. ActionTemplate instances should be immutable and side-effect free.
/// 2. ActionTemplate inputs and outputs are supersets of the inputs and outputs of expanded
///    actions, excluding inputs discovered at execution time. This ensures the ActionTemplate can
///    properly represent the expanded actions at analysis time, and the action graph at analysis
///    time is correct. This is important because the action graph is walked in a lot of places
///    for correctness checks and build analysis.
/// 3. The outputs of expanded actions must be under one of the output TreeArtifact(s) and must
///    not have artifact or artifact path prefix conflicts.
///
/// The `ActionAnalysisMetadata` defaults of the Java interface (`getPrimaryInput`,
/// `getPrimaryOutput`, `getExecProperties`, `getExecutionPlatform`) are overridden in each
/// implementation's [`Action`] impl.
// Java: com/google/devtools/build/lib/actions/ActionTemplate.java:59 ActionTemplate
pub trait ActionTemplate: Action {
    /// Given a list of input TreeFileArtifacts resolved at execution time, returns a list of
    /// expanded actions to be executed.
    ///
    /// Each of the expanded actions' outputs must be a `TreeFileArtifact` owned by
    /// `artifact_owner` with a parent in `getOutputs`. This is generally satisfied by calling
    /// `TreeFileArtifact.createTemplateExpansionOutput`.
    // Java: com/google/devtools/build/lib/actions/ActionTemplate.java:76 ActionTemplate.generateActionsForInputArtifacts
    fn generate_actions_for_input_artifacts(
        &self,
        input_tree_file_artifacts: Vec<Artifact>,
        artifact_owner: ActionLookupKeyRef,
        event_handler: &dyn EventHandler,
    ) -> Result<Vec<ActionRef>, ActionTemplateError>;

    /// Returns the input TreeArtifacts.
    // Java: com/google/devtools/build/lib/actions/ActionTemplate.java:83 ActionTemplate.getInputTreeArtifacts
    fn get_input_tree_artifacts(&self) -> Vec<Artifact>;
}

/// Helper method to partition/denormalize the flattened list of input `TreeFileArtifact`s into a
/// list multimap of input `SpecialArtifact` -> children `TreeFileArtifact`s.
// Java: com/google/devtools/build/lib/actions/ActionTemplate.java:89 ActionTemplate.getInputTreeArtifactsToChildren
pub fn get_input_tree_artifacts_to_children(
    input_tree_artifacts: &[Artifact],
) -> indexmap::IndexMap<Artifact, Vec<Artifact>> {
    let mut result: indexmap::IndexMap<Artifact, Vec<Artifact>> = indexmap::IndexMap::new();
    for child in input_tree_artifacts {
        let parent = child
            .get_parent()
            .unwrap_or_else(|| panic!("NullPointerException: parent of {child}"))
            .clone();
        result.entry(parent).or_default().push(child.clone());
    }
    result
}

type Caster = fn(&dyn Any) -> Option<&dyn ActionTemplate>;

fn registry() -> &'static RwLock<HashMap<TypeId, Caster>> {
    static REGISTRY: std::sync::OnceLock<RwLock<HashMap<TypeId, Caster>>> =
        std::sync::OnceLock::new();
    REGISTRY.get_or_init(|| RwLock::new(HashMap::new()))
}

fn cast<T: ActionTemplate>(a: &dyn Any) -> Option<&dyn ActionTemplate> {
    a.downcast_ref::<T>().map(|t| t as &dyn ActionTemplate)
}

/// Registers `T` as an [`ActionTemplate`] class (idempotent). Call once at startup.
pub fn register_action_template<T: ActionTemplate>() {
    registry().write().insert(TypeId::of::<T>(), cast::<T>);
}

/// `action instanceof ActionTemplate`: the template view of `action`, `None` for actions.
pub fn as_action_template(action: &dyn Action) -> Option<&dyn ActionTemplate> {
    let any = action.as_any();
    let caster = *registry().read().get(&any.type_id())?;
    caster(any)
}

/// `ActionTemplateExpansionValue.key(actionLookupKey, actionIndex)`: the key type lives in
/// blaze_sky_exec, which installs the factory with [`set_action_template_expansion_key_factory`]
/// (templates that compute their action key from a dummy expansion, as `SpawnActionTemplate`,
/// are defined in crates that cannot depend on it).
pub type ActionTemplateExpansionKeyFactory = fn(ActionLookupKeyRef, i32) -> ActionLookupKeyRef;

static EXPANSION_KEY_FACTORY: std::sync::OnceLock<ActionTemplateExpansionKeyFactory> =
    std::sync::OnceLock::new();

/// Installs the [`ActionTemplateExpansionKeyFactory`] (first call wins).
pub fn set_action_template_expansion_key_factory(factory: ActionTemplateExpansionKeyFactory) {
    let _ = EXPANSION_KEY_FACTORY.set(factory);
}

/// `ActionTemplateExpansionValue.key(action_lookup_key, action_index)`.
// Java: com/google/devtools/build/lib/skyframe/ActionTemplateExpansionValue.java:53 ActionTemplateExpansionValue.key
pub fn action_template_expansion_key(
    action_lookup_key: ActionLookupKeyRef,
    action_index: i32,
) -> ActionLookupKeyRef {
    match EXPANSION_KEY_FACTORY.get() {
        Some(factory) => factory(action_lookup_key, action_index),
        None => blaze_util::unported!(
            "ActionTemplateExpansionValue.key (no factory installed by blaze_sky_exec)"
        ),
    }
}
