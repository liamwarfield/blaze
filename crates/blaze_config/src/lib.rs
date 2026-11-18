//! Low slice of Bazel's analysis/config: data types rule() and attr need (Starlark-defined
//! transitions, the exec/target/none transition factories). Owner: agent B
//! (crates/blaze_packages/DESIGN.md).

#![allow(clippy::too_many_arguments, clippy::new_without_default)]
// Ported signatures and Java's control flow win (PORTING.md "Style"): maps keyed by `Artifact`
// (Java `Map<Artifact, ...>`), exceptions carried by value, Java method names (`from_*`), Java's
// branch structure.
#![allow(
    clippy::mutable_key_type,
    clippy::result_large_err,
    clippy::large_enum_variant,
    clippy::type_complexity,
    clippy::wrong_self_convention,
    clippy::collapsible_if,
    clippy::collapsible_match,
    clippy::if_same_then_else
)]

pub mod transition_factories;
pub mod transitions;
