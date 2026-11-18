//! Conformance harness for Bazel's Starlark on starlark-rust (vendored, with Blaze's patches).
//!
//! - `script_test`: runs Bazel's `src/test/java/net/starlark/java/eval/testdata/*.star`.
//! - `dialect`: the Bazel dialect and universe, as seen from stock starlark-rust.
//! - `harness`: the ScriptTest predeclared builtins.
//! - `traceback`: CLI evaluation with Bazel-format errors (rendering lives in blaze_starlark).
//!
//! The integration spikes moved to crates/blaze_starlark/tests/integration_test.rs; the BUILD
//! checker to blaze_starlark::syntax. Findings: docs/research/05-starlark-rust.md.

pub mod dialect;
pub mod harness;
pub mod script_test;
pub mod traceback;
