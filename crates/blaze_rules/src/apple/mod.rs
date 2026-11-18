//! Apple natives (agent PY; crates/blaze_analysis/DESIGN.md): `_builtins.internal.apple_common`
//! (`rules/objc/AppleStarlarkCommon.java`), the Starlark `DottedVersion`
//! (`rules/apple/DottedVersion.java`) and `apple_platform` (`rules/apple/ApplePlatform.java`)
//! values.
//!
//! The `apple`, `objc` and `j2objc` configuration fragments' Starlark values are in
//! `crate::fragments`.
//!
//! `_builtins.internal.java_common_internal_do_not_use` is not replaced: no Java rule is analyzed
//! in the corpus projects (only `@_builtins//:common/java/java_common.bzl` reads it, at load
//! time), so W3's loading object (`blaze_packages::builtins::java_common`) stays.

pub mod apple_platform;
pub mod apple_starlark_common;
pub mod dotted_version;

pub use apple_platform::ApplePlatform;
pub use apple_starlark_common::{AppleStarlarkCommon, apple_common_value};
pub use dotted_version::StarlarkDottedVersion;
