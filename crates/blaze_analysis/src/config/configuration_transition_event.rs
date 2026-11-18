//! Port of `analysis/config/ConfigurationTransitionEvent.java`.

use blaze_events::Postable;

/// Posted when there is a configuration transition.
// Java: com/google/devtools/build/lib/analysis/config/ConfigurationTransitionEvent.java:21 ConfigurationTransitionEvent
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ConfigurationTransitionEvent {
    // Field order is Java's `compareTo` order.
    parent_checksum: String,
    child_checksum: String,
}

impl ConfigurationTransitionEvent {
    // Java: com/google/devtools/build/lib/analysis/config/ConfigurationTransitionEvent.java:28 ConfigurationTransitionEvent.create
    pub fn create(parent_checksum: &str, child_checksum: &str) -> ConfigurationTransitionEvent {
        ConfigurationTransitionEvent {
            parent_checksum: parent_checksum.to_owned(),
            child_checksum: child_checksum.to_owned(),
        }
    }

    pub fn parent_checksum(&self) -> &str {
        &self.parent_checksum
    }

    pub fn child_checksum(&self) -> &str {
        &self.child_checksum
    }
}

impl Postable for ConfigurationTransitionEvent {}
