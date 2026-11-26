//! `rules/core`: the core rules (Java `CoreRules`); only the internal `ValidateTarget` aspect is
//! native here (the other core rule classes are registered by the package layer).
//!
//! PORT: the module is `rules_core`, not `core`, so that it does not shadow the `core` crate.

pub mod validate_target;

use std::sync::Arc;

use blaze_analysis::configured_rule_class_provider::ConfiguredRuleClassProviderBuilder;

/// `CoreRules.INSTANCE.init(builder)`: the native aspect it adds.
// Java: com/google/devtools/build/lib/rules/core/CoreRules.java:32 CoreRules.init
pub fn register(builder: &mut ConfiguredRuleClassProviderBuilder) {
    let validate_target = Arc::new(validate_target::ValidateTarget);
    builder.add_native_aspect_class(validate_target.clone(), validate_target); // internally used aspect
}
