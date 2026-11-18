//! Low slice of Bazel's analysis/config: data types rule() and attr need (Starlark-defined
//! transitions, the exec/target/none transition factories). Owner: agent B
//! (crates/blaze_packages/DESIGN.md).

#![allow(clippy::too_many_arguments, clippy::new_without_default)]

pub mod transition_factories;
pub mod transitions;
