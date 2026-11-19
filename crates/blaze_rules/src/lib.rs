//! Port of Bazel's native rule implementations and rule-set natives (`lib/rules`,
//! `lib/bazel/rules`): platform/config/generic native rules' configured target factories, and
//! the Starlark natives behind rules_cc / rules_python / apple_support (`cc_internal`,
//! `cc_common`, `py_builtins`, `apple_common`, configuration fragments).
//!
//! Design and ownership: `crates/blaze_analysis/DESIGN.md`.

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

pub mod apple;
pub mod bazel_environment;
pub mod build_info;
pub mod config;
pub mod cpp;
pub mod executable_actions;
pub mod fragments;
pub mod generic;
pub mod java;
pub mod platform;
pub mod python;
pub mod register;
