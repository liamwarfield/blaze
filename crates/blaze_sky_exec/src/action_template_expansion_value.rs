//! Port of `lib/skyframe/ActionTemplateExpansionValue.java` (with its `ActionTemplateExpansionKey`).
//!
//! The function computing the value (`ActionTemplateExpansionFunction`) needs the analysis-side
//! conflict checks, so it lives in `blaze_sky_top`; the key and value are here so that
//! `ArtifactFunction` and `ActionExecutionFunction` can request them.

use std::any::Any;
use std::fmt;
use std::hash::{Hash, Hasher};

use blaze_actions::actions::action::ActionRef;
use blaze_actions::actions::action_lookup::{ACTION_TEMPLATE_EXPANSION, ActionLookupKey};
use blaze_actions::{ActionLookupData, ActionLookupKeyRef, Artifact};
use blaze_cmdline::Label;
use blaze_skyframe::{FunctionHermeticity, Key, SkyFunctionName};

use crate::analysis_seam::ActionLookupValue;

/// Value that stores expanded actions from ActionTemplate.
// Java: com/google/devtools/build/lib/skyframe/ActionTemplateExpansionValue.java:29 ActionTemplateExpansionValue
pub struct ActionTemplateExpansionValue {
    // Java: BasicActionLookupValue.actions
    actions: Vec<ActionRef>,
}

impl ActionTemplateExpansionValue {
    // Java: com/google/devtools/build/lib/skyframe/ActionTemplateExpansionValue.java:31 ActionTemplateExpansionValue.<init>
    pub fn new(generating_actions: Vec<ActionRef>) -> ActionTemplateExpansionValue {
        ActionTemplateExpansionValue {
            actions: generating_actions,
        }
    }

    /// Returns the keys of actions that populate this tree, including empty subdirectories.
    /// Expanded actions that only populate another output tree of the same template are excluded.
    // Java: com/google/devtools/build/lib/skyframe/ActionTemplateExpansionValue.java:39 ActionTemplateExpansionValue.getGeneratingActionKeys
    pub fn get_generating_action_keys(&self, tree: &Artifact) -> Vec<ActionLookupData> {
        let mut keys = Vec::with_capacity(self.actions.len());
        for action in &self.actions {
            for output in action.get_outputs() {
                if output.get_parent() == Some(tree) {
                    keys.push(output.get_generating_action_key().clone());
                    break;
                }
            }
        }
        keys
    }

    // Java: com/google/devtools/build/lib/skyframe/ActionTemplateExpansionValue.java:53 ActionTemplateExpansionValue.key
    pub fn key(
        action_lookup_key: ActionLookupKeyRef,
        action_index: i32,
    ) -> ActionTemplateExpansionKey {
        ActionTemplateExpansionKey::of(action_lookup_key, action_index)
    }
}

impl ActionLookupValue for ActionTemplateExpansionValue {
    // Java: com/google/devtools/build/lib/skyframe/BasicActionLookupValue.java BasicActionLookupValue.getActions
    fn get_actions(&self) -> &[ActionRef] {
        &self.actions
    }
}

impl fmt::Debug for ActionTemplateExpansionValue {
    // Java: BasicActionLookupValue.toString: toStringHelper(this).add("actions", actions)
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let actions: Vec<String> = self.actions.iter().map(|a| a.to_string_java()).collect();
        write!(
            f,
            "ActionTemplateExpansionValue{{actions=[{}]}}",
            actions.join(", ")
        )
    }
}

/// Key for `ActionTemplateExpansionValue` nodes.
// Java: com/google/devtools/build/lib/skyframe/ActionTemplateExpansionValue.java:58 ActionTemplateExpansionValue.ActionTemplateExpansionKey
#[derive(Clone)]
pub struct ActionTemplateExpansionKey {
    action_lookup_key: ActionLookupKeyRef,
    action_index: i32,
}

impl ActionTemplateExpansionKey {
    // Java: com/google/devtools/build/lib/skyframe/ActionTemplateExpansionValue.java:70 ActionTemplateExpansionKey.of
    pub fn of(action_lookup_key: ActionLookupKeyRef, action_index: i32) -> Self {
        ActionTemplateExpansionKey {
            action_lookup_key,
            action_index,
        }
    }

    // Java: com/google/devtools/build/lib/skyframe/ActionTemplateExpansionValue.java:96 ActionTemplateExpansionKey.getActionLookupKey
    pub fn get_action_lookup_key(&self) -> &ActionLookupKeyRef {
        &self.action_lookup_key
    }

    /// Index of the action in question in the node keyed by `getActionLookupKey`. Should be
    /// passed to `ActionLookupValue.getAction`.
    // Java: com/google/devtools/build/lib/skyframe/ActionTemplateExpansionValue.java:104 ActionTemplateExpansionKey.getActionIndex
    pub fn get_action_index(&self) -> i32 {
        self.action_index
    }
}

// Java: com/google/devtools/build/lib/skyframe/ActionTemplateExpansionValue.java:118 ActionTemplateExpansionKey.equals
impl PartialEq for ActionTemplateExpansionKey {
    fn eq(&self, other: &Self) -> bool {
        self.action_index == other.action_index && self.action_lookup_key == other.action_lookup_key
    }
}

impl Eq for ActionTemplateExpansionKey {}

// Java: com/google/devtools/build/lib/skyframe/ActionTemplateExpansionValue.java:113 ActionTemplateExpansionKey.hashCode
impl Hash for ActionTemplateExpansionKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.action_lookup_key.hash(state);
        self.action_index.hash(state);
    }
}

// Java: com/google/devtools/build/lib/skyframe/ActionTemplateExpansionValue.java:131 ActionTemplateExpansionKey.toString
impl fmt::Display for ActionTemplateExpansionKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "ActionTemplateExpansionKey{{actionLookupKey={}, actionIndex={}}}",
            self.action_lookup_key, self.action_index
        )
    }
}

impl fmt::Debug for ActionTemplateExpansionKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl Key for ActionTemplateExpansionKey {
    type Value = ActionTemplateExpansionValue;
    const FUNCTION: SkyFunctionName = SkyFunctionName(ACTION_TEMPLATE_EXPANSION);
    // Java: com/google/devtools/build/lib/skyframe/SkyFunctions.java:136 SkyFunctions.ACTION_TEMPLATE_EXPANSION (createHermetic)
    const HERMETICITY: FunctionHermeticity = FunctionHermeticity::Hermetic;
    // Default `values_equal` (never equal): Java's `ActionTemplateExpansionValue` (a
    // `BasicActionLookupValue`) has no `equals`.
    // Java: com/google/devtools/build/lib/skyframe/ActionTemplateExpansionValue.java:33 ActionTemplateExpansionValue
}

impl ActionLookupKey for ActionTemplateExpansionKey {
    // Java: com/google/devtools/build/lib/skyframe/ActionTemplateExpansionValue.java:86 ActionTemplateExpansionKey.getLabel
    fn get_label(&self) -> Option<Label> {
        self.action_lookup_key.get_label()
    }

    // Java: com/google/devtools/build/lib/skyframe/ActionTemplateExpansionValue.java:91 ActionTemplateExpansionKey.getConfigurationKey
    fn get_configuration_checksum(&self) -> Option<String> {
        self.action_lookup_key.get_configuration_checksum()
    }

    // Java: com/google/devtools/build/lib/skyframe/ActionTemplateExpansionValue.java:81 ActionTemplateExpansionKey.functionName
    fn function_name(&self) -> &'static str {
        ACTION_TEMPLATE_EXPANSION
    }

    fn key_equals(&self, other: &dyn ActionLookupKey) -> bool {
        other
            .as_any()
            .downcast_ref::<ActionTemplateExpansionKey>()
            .is_some_and(|o| o == self)
    }

    fn key_hash(&self) -> u64 {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        self.hash(&mut h);
        h.finish()
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}
