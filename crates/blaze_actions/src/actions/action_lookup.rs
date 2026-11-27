//! Ports of `lib/actions/ArtifactOwner.java`, `ActionLookupKey.java` and `ActionLookupData.java`.

use std::any::Any;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use blaze_cmdline::Label;

/// Java `SkyFunctions.ACTION_TEMPLATE_EXPANSION` function name.
pub const ACTION_TEMPLATE_EXPANSION: &str = "ACTION_TEMPLATE_EXPANSION";
/// Java `SkyFunctions.ACTION_EXECUTION` function name.
pub const ACTION_EXECUTION: &str = "ACTION_EXECUTION";

/// Uniquely determines the SkyKey of the value that owns a set of actions (configured targets,
/// aspects, action templates, the build-info and workspace-status keys...).
///
/// PORT: Java's `ActionLookupKey` is an interface implemented by Skyframe keys in other packages
/// (`ConfiguredTargetKey`, `AspectKey`, `ActionTemplateExpansionKey`, `BuildInfoKey`,
/// `WorkspaceStatusValue.BUILD_INFO_KEY`). It is a trait here; the crates owning those keys
/// implement it. Equality and hashing go through [`ActionLookupKey::key_equals`] and
/// [`ActionLookupKey::key_hash`] so that trait objects can be compared.
// Java: com/google/devtools/build/lib/actions/ActionLookupKey.java:33 ActionLookupKey
pub trait ActionLookupKey: Send + Sync + fmt::Debug + fmt::Display + Any {
    /// `ArtifactOwner.getLabel`.
    fn get_label(&self) -> Option<Label>;

    /// `getConfigurationKey()`, reduced to the configuration checksum (`None` where Java returns
    /// null).
    // PORT: BuildConfigurationKey lives in blaze_build_config; actions only need its identity.
    fn get_configuration_checksum(&self) -> Option<String>;

    /// `SkyKey.functionName()` (for example `CONFIGURED_TARGET`, `ACTION_TEMPLATE_EXPANSION`).
    fn function_name(&self) -> &'static str;

    /// Java `equals` against another key (compare the concrete type first).
    fn key_equals(&self, other: &dyn ActionLookupKey) -> bool;

    /// Java `hashCode` (any hash consistent with [`ActionLookupKey::key_equals`]).
    fn key_hash(&self) -> u64;

    fn as_any(&self) -> &dyn Any;

    /// Returns `true` if this key may own shareable actions, as determined by
    /// `Action.isShareable`.
    ///
    /// Returns `false` for some non-standard keys such as the build info key.
    // Java: com/google/devtools/build/lib/actions/ActionLookupKey.java:52 ActionLookupKey.mayOwnShareableActions
    fn may_own_shareable_actions(&self) -> bool {
        self.get_label().is_some()
    }
}

/// Shared handle to an [`ActionLookupKey`] with Java's `equals`/`hashCode`.
#[derive(Clone)]
pub struct ActionLookupKeyRef(pub Arc<dyn ActionLookupKey>);

impl ActionLookupKeyRef {
    pub fn new<K: ActionLookupKey>(key: K) -> ActionLookupKeyRef {
        ActionLookupKeyRef(Arc::new(key))
    }
}

impl std::ops::Deref for ActionLookupKeyRef {
    type Target = dyn ActionLookupKey;
    fn deref(&self) -> &(dyn ActionLookupKey + 'static) {
        &*self.0
    }
}

impl PartialEq for ActionLookupKeyRef {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0) || self.0.key_equals(&*other.0)
    }
}

impl Eq for ActionLookupKeyRef {}

impl Hash for ActionLookupKeyRef {
    fn hash<H: Hasher>(&self, state: &mut H) {
        state.write_u64(self.0.key_hash());
    }
}

impl fmt::Debug for ActionLookupKeyRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&*self.0, f)
    }
}

impl fmt::Display for ActionLookupKeyRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&*self.0, f)
    }
}

/// An interface for objects that can own an Artifact.
///
/// PORT: Java's interface is implemented by `Label` (source artifacts), every `ActionLookupKey`
/// (derived artifacts) and `NULL_OWNER`; a closed enum here.
// Java: com/google/devtools/build/lib/actions/ArtifactOwner.java:25 ArtifactOwner
#[derive(Clone, PartialEq, Eq, Hash)]
pub enum ArtifactOwner {
    Label(Label),
    ActionLookupKey(ActionLookupKeyRef),
    /// Java `ArtifactOwner.NULL_OWNER`.
    // Java: com/google/devtools/build/lib/actions/ArtifactOwner.java:35 ArtifactOwner.NULL_OWNER
    NullOwner,
}

impl ArtifactOwner {
    // Java: com/google/devtools/build/lib/actions/ArtifactOwner.java:28 ArtifactOwner.getLabel
    pub fn get_label(&self) -> Option<Label> {
        match self {
            ArtifactOwner::Label(l) => Some(l.clone()),
            ArtifactOwner::ActionLookupKey(k) => k.get_label(),
            ArtifactOwner::NullOwner => None,
        }
    }
}

impl fmt::Display for ArtifactOwner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            // Java: Label.toString is the canonical form.
            ArtifactOwner::Label(l) => f.write_str(&l.get_canonical_form()),
            ArtifactOwner::ActionLookupKey(k) => fmt::Display::fmt(k, f),
            ArtifactOwner::NullOwner => f.write_str("NULL_OWNER"),
        }
    }
}

impl fmt::Debug for ArtifactOwner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

/// Data that uniquely identifies an action: its owner's key and its index among the owner's
/// actions.
// Java: com/google/devtools/build/lib/actions/ActionLookupData.java:33 ActionLookupData
// PORT: Java's ten index-specialized subclasses (a memory optimization) collapse into one struct;
// `UnshareableActionLookupData` is the `shareable == false` case.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct ActionLookupData {
    action_lookup_key: ActionLookupKeyRef,
    action_index: i32,
    shareable: bool,
}

impl ActionLookupData {
    /// Creates a key for the result of action execution. Does not intern its results.
    // Java: com/google/devtools/build/lib/actions/ActionLookupData.java:39 ActionLookupData.create
    pub fn create(action_lookup_key: ActionLookupKeyRef, action_index: i32) -> ActionLookupData {
        if !action_lookup_key.may_own_shareable_actions() {
            return ActionLookupData::create_unshareable(action_lookup_key, action_index);
        }
        ActionLookupData {
            action_lookup_key,
            action_index,
            shareable: true,
        }
    }

    /// Similar to [`ActionLookupData::create`], but the key will have `valueIsShareable()` return
    /// false.
    // Java: com/google/devtools/build/lib/actions/ActionLookupData.java:62 ActionLookupData.createUnshareable
    pub fn create_unshareable(
        action_lookup_key: ActionLookupKeyRef,
        action_index: i32,
    ) -> ActionLookupData {
        ActionLookupData {
            action_lookup_key,
            action_index,
            shareable: false,
        }
    }

    // Java: com/google/devtools/build/lib/actions/ActionLookupData.java:73 ActionLookupData.getActionLookupKey
    pub fn get_action_lookup_key(&self) -> &ActionLookupKeyRef {
        &self.action_lookup_key
    }

    /// Index of the action in question in the node keyed by [`Self::get_action_lookup_key`].
    /// Should be passed to `ActionLookupValue#getAction`.
    // Java: com/google/devtools/build/lib/actions/ActionLookupData.java:81 ActionLookupData.getActionIndex
    // Java: com/google/devtools/build/lib/actions/ActionLookupData.java:127 ActionLookupData.ActionLookupData0.getActionIndex
    // Java: com/google/devtools/build/lib/actions/ActionLookupData.java:138 ActionLookupData.ActionLookupData1.getActionIndex
    // Java: com/google/devtools/build/lib/actions/ActionLookupData.java:149 ActionLookupData.ActionLookupData2.getActionIndex
    // Java: com/google/devtools/build/lib/actions/ActionLookupData.java:160 ActionLookupData.ActionLookupData3.getActionIndex
    // Java: com/google/devtools/build/lib/actions/ActionLookupData.java:171 ActionLookupData.ActionLookupData4.getActionIndex
    // Java: com/google/devtools/build/lib/actions/ActionLookupData.java:182 ActionLookupData.ActionLookupData5.getActionIndex
    // Java: com/google/devtools/build/lib/actions/ActionLookupData.java:193 ActionLookupData.ActionLookupData6.getActionIndex
    // Java: com/google/devtools/build/lib/actions/ActionLookupData.java:204 ActionLookupData.ActionLookupData7.getActionIndex
    // Java: com/google/devtools/build/lib/actions/ActionLookupData.java:215 ActionLookupData.ActionLookupData8.getActionIndex
    // Java: com/google/devtools/build/lib/actions/ActionLookupData.java:226 ActionLookupData.ActionLookupData9.getActionIndex
    // Java: com/google/devtools/build/lib/actions/ActionLookupData.java:240 ActionLookupData.ActionLookupDataN.getActionIndex
    pub fn get_action_index(&self) -> i32 {
        self.action_index
    }

    // Java: com/google/devtools/build/lib/actions/ActionLookupData.java:83 ActionLookupData.getLabel
    pub fn get_label(&self) -> Option<Label> {
        self.action_lookup_key.get_label()
    }

    // Java: com/google/devtools/build/lib/actions/ActionLookupData.java:252 ActionLookupData.valueIsShareable
    pub fn value_is_shareable(&self) -> bool {
        self.shareable
    }

    // Java: com/google/devtools/build/lib/actions/ActionLookupData.java:118 ActionLookupData.functionName
    pub fn function_name(&self) -> &'static str {
        ACTION_EXECUTION
    }
}

// Java: com/google/devtools/build/lib/actions/ActionLookupData.java:110 ActionLookupData.toString
impl fmt::Display for ActionLookupData {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // PORT: MoreObjects.toStringHelper uses the simple class name of the concrete subclass.
        let class = if self.shareable {
            match self.action_index {
                0..=9 => format!("ActionLookupData{}", self.action_index),
                _ => "ActionLookupDataN".to_string(),
            }
        } else {
            "UnshareableActionLookupData".to_string()
        };
        write!(
            f,
            "{}{{actionLookupKey={}, actionIndex={}}}",
            class, self.action_lookup_key, self.action_index
        )
    }
}

impl fmt::Debug for ActionLookupData {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}
