//! Builtins injection: the predeclared environment of every Starlark dialect, the `@_builtins`
//! internal module and its stand-in objects, and autoloads.
//!
//! Ownership: D (crates/blaze_packages/DESIGN.md). The SkyFunctions that evaluate `@_builtins`
//! (STARLARK_BUILTINS) and .bzl files (BZL_LOAD) are in `blaze_sky_loading`.
//!
//! - [`env`]: predeclared environments as frozen values, with Java's `GuardedValue`s.
//! - [`starlark_globals`]: Java `StarlarkGlobals` (fixed symbols per dialect).
//! - [`registered`]: what Bazel's rule sets register (bzl toplevels, `@_builtins` internals).
//! - [`stubs`]: stand-ins for analysis-time Java builtins.
//! - [`builtins_internal_module`]: Java `BuiltinsInternalModule` (`_builtins`).
//! - [`bazel_starlark_environment`]: Java `BazelStarlarkEnvironment`.
//! - [`autoload_symbols`]: Java `AutoloadSymbols`.

pub mod autoload_symbols;
pub mod bazel_starlark_environment;
pub mod builtins_internal_module;
pub mod cc_internal;
pub mod env;
pub mod java_common;
pub mod registered;
pub mod starlark_globals;
pub mod stubs;

pub use autoload_symbols::AutoloadSymbols;
pub use bazel_starlark_environment::{BazelStarlarkEnvironment, InjectionException};
pub use env::{EnvEntry, Guard, StarlarkEnv};
pub use registered::RuleClassProviderRegistrations;
pub use starlark_globals::{StarlarkGlobals, StarlarkGlobalsFns};
