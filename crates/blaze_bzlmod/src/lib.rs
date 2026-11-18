//! Port of Bazel's `lib/bazel/bzlmod` SkyFunctions and Starlark parts, plus `modcommand`.
//!
//! - [`module_file`]: MODULE.bazel evaluation (`ModuleFileGlobals`, `ModuleThreadContext`,
//!   `CompiledModuleFile`).
//! - [`skyframe`]: keys, values and functions (MODULE_FILE, BAZEL_MODULE_RESOLUTION,
//!   BAZEL_DEP_GRAPH, BAZEL_MODULE_INSPECTION, BAZEL_LOCK_FILE, REGISTRY, REPO_SPEC,
//!   YANKED_VERSIONS) and the lockfile writer (`BazelLockFileModule.afterCommand`).
//! - [`inspector`]: `BazelModuleInspectorValue` and `computeAugmentedGraph`.
//! - [`modcommand`]: `ModExecutor`, its output formatters, and `ModCommand`'s logic.
//! - [`registry_fetcher`]: registry file downloads through the repository cache.
//!
//! The Starlark-free model (versions, selection, discovery, registries, lockfile bytes) is
//! `blaze_bzlmod_model`.

#![allow(clippy::too_many_arguments, clippy::new_without_default)]

pub mod inspector;
pub mod modcommand;
pub mod module_file;
pub mod registry_fetcher;
pub mod skyframe;
