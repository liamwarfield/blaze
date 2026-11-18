//! Port of Bazel's spawn execution: `lib/exec` (+`local`), `lib/sandbox` (Linux),
//! `lib/standalone`, the `Spawn*` types of `lib/actions` and the action cache.
#![allow(clippy::too_many_arguments, clippy::new_without_default)]

pub mod actions;
pub mod analysis;
pub mod exec;
pub mod runtime;
pub mod sandbox;
pub mod standalone;
pub mod util;
