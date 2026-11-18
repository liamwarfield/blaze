//! Skyframe loading functions: FILE_STATE, FILE, DIRECTORY_LISTING, PACKAGE_LOOKUP, BZL_COMPILE,
//! BZL_LOAD, STARLARK_BUILTINS, GLOB, PACKAGE, REPO_FILE, target patterns, TRANSITIVE_TRAVERSAL;
//! the external repos provider seam.
//!
//! Design and ownership: `crates/blaze_packages/DESIGN.md`. Keys are newtypes implementing
//! [`blaze_skyframe::Key`]; their `Debug` prints what Java's `SkyKey.toString` prints.
//!
//! Module map (Java sources under `com/google/devtools/build/lib`):
//! - [`sky_functions`]: `skyframe/SkyFunctions` names.
//! - [`precomputed`]: `skyframe/PrecomputedValue` (`STARLARK_SEMANTICS`, `PATH_PACKAGE_LOCATOR`).
//! - [`pkgcache`]: `pkgcache/PathPackageLocator`.
//! - [`directories`]: the `BlazeDirectories` paths the loading functions need.
//! - [`io`]: `java.io.IOException` and `lib/io` exceptions thrown through Skyframe;
//!   [`file_chain_uniqueness`]: `lib/io/*UniquenessFunction`.
//! - [`actions`]: `actions/FileStateValue`, `actions/FileValue`, `FileContentsProxy`.
//! - [`file_state`] (FILE_STATE, `ExternalFilesHelper`), [`file`] (FILE),
//!   [`directory_listing`] (DIRECTORY_LISTING_STATE, DIRECTORY_LISTING).
//! - [`repos`]: REPOSITORY_MAPPING, REPOSITORY_DIRECTORY, the [`repos::ExternalRepos`] seam and
//!   [`repos::OracleRepos`].
//! - [`build_file_name`], [`exceptions`], [`package_lookup`] (PACKAGE_LOOKUP),
//!   [`local_repository_lookup`], [`containing_package_lookup`],
//!   [`ignored_subdirectories`] (IGNORED_SUBDIRECTORIES, `cmdline/IgnoredSubdirectories`),
//!   [`unix_glob`] (the `vfs/UnixGlob` matchers it needs).
//! - [`repo_file`]: REPO_FILE, REPO_PACKAGE_ARGS.
//! - [`starlark_util`], [`bzl_compile`]: BZL_COMPILE.
//! - [`bundled_fs`]: the bundled `@_builtins` .bzl files; [`starlark_builtins`]:
//!   STARLARK_BUILTINS; [`bzl_load`]: BZL_LOAD; [`environment`]: Bazel's Starlark environments.
//! - [`globs`]: GLOBS; [`package_function`]: PACKAGE (`PackageFunctionWithSingleGlobsDep`,
//!   `PackageValue`, the BUILD-file locator of `SkyframePackageManager`); [`package_manager`]:
//!   loading packages and targets from outside Skyframe (`TargetProvider`,
//!   `SkyframePackageManager`, `getMainRepoMapping`, `SkyframeTargetPatternEvaluator`).
//! - [`target_pattern_eval`]: `TargetPattern.eval` / `TargetPatternResolver`;
//!   [`target_pattern_function`]: TARGET_PATTERN; [`recursive_pkg`]: RECURSIVE_PKG,
//!   PACKAGE_ERROR(_MESSAGE); [`transitive_target`]: TRANSITIVE_TARGET.
//! - [`skyframe_executor`]: registers every function on an `EvaluatorBuilder`.

#![allow(clippy::too_many_arguments, clippy::new_without_default)]

pub mod actions;
pub mod build_file_name;
pub mod bundled_fs;
pub mod bzl_compile;
pub mod bzl_load;
pub mod containing_package_lookup;
pub mod directories;
pub mod directory_listing;
pub mod environment;
pub mod exceptions;
pub mod file;
pub mod file_chain_uniqueness;
pub mod file_state;
pub mod globs;
pub mod ignored_subdirectories;
pub mod io;
pub mod local_repository_lookup;
pub mod package_function;
pub mod package_lookup;
pub mod package_manager;
pub mod package_progress_receiver;
pub mod pkgcache;
pub mod precomputed;
pub mod recursive_pkg;
pub mod repo_file;
pub mod repos;
pub mod sky_functions;
pub mod skyframe_executor;
pub mod starlark_builtins;
pub mod starlark_util;
pub mod target_pattern_eval;
pub mod target_pattern_function;
pub mod transitive_target;
pub mod unix_glob;

#[cfg(test)]
mod test_util;
#[cfg(test)]
mod tests;
