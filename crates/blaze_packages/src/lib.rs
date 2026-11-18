//! Port of `com.google.devtools.build.lib.packages` (plus the `.bzl` rule-definition builtins of
//! `analysis/starlark`, marked `// PORT: move to blaze_analysis`). Design and file ownership:
//! `crates/blaze_packages/DESIGN.md`.

#![allow(
    clippy::too_many_arguments,
    clippy::new_without_default,
    clippy::type_complexity,
    clippy::large_enum_variant
)]

pub mod attribute;
pub mod build;
pub mod builtins;
pub mod bzl_context;
pub mod exceptions;
pub mod macros;
pub mod native_rules;
pub mod package;
pub mod proto;
pub mod repo_file;
pub mod rule_class;
pub mod starlark_defs;
pub mod target;
pub mod types;
