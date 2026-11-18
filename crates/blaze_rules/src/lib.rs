//! Port of Bazel's native rule implementations and rule-set natives (`lib/rules`,
//! `lib/bazel/rules`): platform/config/generic native rules' configured target factories, and
//! the Starlark natives behind rules_cc / rules_python / apple_support (`cc_internal`,
//! `cc_common`, `py_builtins`, `apple_common`, configuration fragments).
//!
//! Design and ownership: `crates/blaze_analysis/DESIGN.md`.

#![allow(clippy::too_many_arguments, clippy::new_without_default)]
#![allow(
    clippy::collapsible_if,
    clippy::type_complexity,
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
pub mod platform;
pub mod python;
pub mod register;
