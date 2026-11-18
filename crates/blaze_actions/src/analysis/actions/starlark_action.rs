//! Port of `lib/analysis/actions/StarlarkAction.java`'s builder.
//!
//! The action itself is a [`SpawnAction`] whose class is `Starlark` or `EnhancedStarlark` (see
//! `spawn_action.rs`): `StarlarkAction` overrides no key-relevant behaviour of `SpawnAction`.

use crate::actions::action::ActionRef;
use crate::actions::artifact::Artifact;
use crate::analysis::actions::spawn_action::{SpawnActionBuilder, StarlarkBuilderFields};

/// Builder class to construct `StarlarkAction` instances.
// Java: com/google/devtools/build/lib/analysis/actions/StarlarkAction.java:149 StarlarkAction.Builder
pub fn starlark_action_builder() -> SpawnActionBuilder {
    let mut b = SpawnActionBuilder::new();
    b.starlark = Some(StarlarkBuilderFields::default());
    b
}

/// `StarlarkAction.Builder.setUnusedInputsList`.
// Java: com/google/devtools/build/lib/analysis/actions/StarlarkAction.java:155 StarlarkAction.Builder.setUnusedInputsList
pub fn set_unused_inputs_list(
    builder: &mut SpawnActionBuilder,
    unused_inputs_list: Option<Artifact>,
) {
    builder
        .starlark
        .get_or_insert_with(StarlarkBuilderFields::default)
        .unused_inputs_list = unused_inputs_list;
}

/// `StarlarkAction.Builder.setShadowedAction`.
// Java: com/google/devtools/build/lib/analysis/actions/StarlarkAction.java:161 StarlarkAction.Builder.setShadowedAction
pub fn set_shadowed_action(builder: &mut SpawnActionBuilder, shadowed_action: Option<ActionRef>) {
    builder
        .starlark
        .get_or_insert_with(StarlarkBuilderFields::default)
        .shadowed_action = shadowed_action;
}
