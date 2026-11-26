//! Port of Bazel's spawn execution: `lib/exec` (+`local`), `lib/sandbox` (Linux),
//! `lib/standalone`, the `Spawn*` types of `lib/actions` and the action cache.
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

pub mod actions;
pub mod analysis;
pub mod dynamic;
pub mod exec;
pub mod rules;
pub mod runtime;
pub mod sandbox;
pub mod standalone;
pub mod util;
pub mod worker;
