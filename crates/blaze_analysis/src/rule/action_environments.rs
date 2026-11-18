//! Conversion between the two ports of `actions/ActionEnvironment.java`: the configuration's
//! (`blaze_build_config::action_environment::ActionEnvironment`, fixed map + inherited set,
//! both sorted) and the action layer's (`blaze_actions::ActionEnvironment`).
//!
//! Java has one class; `BuildConfigurationValue.getActionEnvironment()` returns the object that
//! actions fingerprint. The configuration's environment is built by `ActionEnvironment.split`
//! (sorted keys), so the converted value iterates in the same order.

use blaze_actions::ActionEnvironment;
use blaze_build_config::action_environment::ActionEnvironment as ConfigActionEnvironment;
use indexmap::{IndexMap, IndexSet};

/// `configuration.getActionEnvironment()` / `getTestActionEnvironment()` as the action layer's
/// type.
// Java: com/google/devtools/build/lib/actions/ActionEnvironment.java:56 ActionEnvironment.create
pub fn to_actions_env(env: &ConfigActionEnvironment) -> ActionEnvironment {
    let fixed: IndexMap<String, String> = env
        .get_fixed_env()
        .iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    let inherited: IndexSet<String> = env.get_inherited_env().iter().cloned().collect();
    ActionEnvironment::create(fixed, inherited)
}
