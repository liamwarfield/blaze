//! Port of `analysis/extra/ExtraActionMapProvider.java`.

use std::collections::BTreeMap;

use super::extra_action_spec::ExtraActionSpec;
use crate::provider_map::TransitiveInfoProvider;

/// Provides an action type -> set of extra actions to run map.
///
/// PORT: Java's `ImmutableMultimap` is a copy of the `ImmutableSortedKeyListMultimap`
/// `ActionListener` builds: keys sorted, each key's values in insertion order.
// Java: com/google/devtools/build/lib/analysis/extra/ExtraActionMapProvider.java:25 ExtraActionMapProvider
#[derive(Clone, Debug)]
pub struct ExtraActionMapProvider {
    extra_action_map: BTreeMap<String, Vec<ExtraActionSpec>>,
}

impl ExtraActionMapProvider {
    // Java: com/google/devtools/build/lib/analysis/extra/ExtraActionMapProvider.java:28 ExtraActionMapProvider.<init>
    pub fn new(extra_action_map: BTreeMap<String, Vec<ExtraActionSpec>>) -> Self {
        ExtraActionMapProvider { extra_action_map }
    }

    /// Returns the extra action map.
    // Java: com/google/devtools/build/lib/analysis/extra/ExtraActionMapProvider.java:35 ExtraActionMapProvider.getExtraActionMap
    pub fn get_extra_action_map(&self) -> &BTreeMap<String, Vec<ExtraActionSpec>> {
        &self.extra_action_map
    }
}

impl TransitiveInfoProvider for ExtraActionMapProvider {
    fn java_class_name(&self) -> &'static str {
        "ExtraActionMapProvider"
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}
