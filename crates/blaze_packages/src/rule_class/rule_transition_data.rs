//! Port of `packages/RuleTransitionData.java`.

use crate::target::Rule;

/// Data used by a transition factory to create a rule transition (Java
/// `RuleTransitionData`).
// PORT: `configConditions` (analysis ConfigMatchingProvider map) is omitted until W4.
// Java: com/google/devtools/build/lib/packages/RuleTransitionData.java:28 RuleTransitionData
pub struct RuleTransitionData<'a> {
    pub rule: &'a Rule,
    pub config_hash: String,
}

impl<'a> RuleTransitionData<'a> {
    // Java: com/google/devtools/build/lib/packages/RuleTransitionData.java:38 RuleTransitionData.create
    pub fn create(rule: &'a Rule, config_hash: String) -> Self {
        RuleTransitionData { rule, config_hash }
    }
}
