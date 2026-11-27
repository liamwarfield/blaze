//! Action templates of `analysis/actions` (Java `SpawnActionTemplate`,
//! `StarlarkMapActionTemplate`): actions expanded at execution time over the children of their
//! input tree artifacts (`blaze_exec::actions::ActionTemplate`).

pub mod spawn_action_template;
pub mod starlark_map_action_template;

/// Registers the action template classes of this crate (`instanceof ActionTemplate`).
/// Idempotent.
pub fn register_action_templates() {
    blaze_exec::actions::register_action_template::<spawn_action_template::SpawnActionTemplate>();
}
