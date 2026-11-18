//! Port of `packages/RuleFunction.java`.

use std::sync::Arc;

use crate::rule_class::RuleClass;

/// A Starlark callable that instantiates a rule (Java `RuleFunction`): native rule functions
/// and the value `rule()` returns implement it.
// Java: com/google/devtools/build/lib/packages/RuleFunction.java:22 RuleFunction
pub trait RuleFunction {
    // Java: com/google/devtools/build/lib/packages/RuleFunction.java:23 RuleFunction.getRuleClass
    fn get_rule_class(&self) -> Arc<RuleClass>;
}
