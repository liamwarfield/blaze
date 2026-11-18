//! Conformance harness and integration spike for running Bazel's Starlark on starlark-rust.
//!
//! - `script_test`: runs Bazel's `src/test/java/net/starlark/java/eval/testdata/*.star`.
//! - `dialect`: Bazel-like `Dialect` and core globals.
//! - `harness`: the ScriptTest predeclared builtins.
//! - `build_check`: Bazel's BUILD-file syntax restrictions as an AST pass.
//! - `traceback`: Bazel-format error rendering from a starlark-rust error.
//! - `spike`: Bazel-like custom values (Label, depset, provider, ctx) used by the spike tests.
//!
//! Findings are written up in docs/research/05-starlark-rust.md.

pub mod build_check;
pub mod dialect;
pub mod harness;
pub mod script_test;
pub mod spike;
pub mod traceback;
