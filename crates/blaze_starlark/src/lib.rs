//! Blaze's Starlark layer on starlark-rust (vendored 0.14.2 + third_party/starlark-rust/PATCHES.md).
//!
//! Design: docs/research/05-starlark-rust.md section 6. Crates above code against:
//! - [`semantics::StarlarkSemantics`]: typed BuildLanguageOptions.
//! - [`file_kind::FileKind`]: parse options and dialect per kind of file.
//! - [`syntax`]: Bazel's static checks (BUILD restrictions, load placement, rebinding, .scl).
//! - [`env`]: universe, `print`/`fail`, and the [`env::NativeRegistry`] crates add natives to.
//! - [`thread::ThreadContext`]: per-evaluation state in `Evaluator::extra`, with an opaque
//!   payload for phase-specific owners (package builder, rule context, …).
//! - [`eval`]: compile, execute, call frozen functions, `frozen_into`, evaluation threads.
//! - [`loader::LoadMap`]: resolved `load()`s.
//! - [`error::EvalError`]: Bazel tracebacks; [`error::SyntaxError`].
//! - Values: [`structs`] (`struct`, provider-instance base), [`depset`], [`label`], [`json`].

pub mod depset;
pub mod env;
pub mod error;
pub mod eval;
pub mod file_kind;
pub mod json;
pub mod label;
pub mod loader;
pub mod printer;
pub mod semantics;
pub mod structs;
pub mod syntax;
pub mod thread;

pub use error::EvalError;
pub use error::Location;
pub use error::SyntaxError;
pub use file_kind::FileKind;
pub use semantics::StarlarkSemantics;
pub use thread::Phase;
pub use thread::ThreadContext;
