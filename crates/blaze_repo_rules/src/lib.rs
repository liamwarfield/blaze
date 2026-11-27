//! Port of Bazel's Starlark repository rules and module extensions (Bazel 9.3.2).
//!
//! - [`starlark`]: `repository_rule()` / `module_extension()` / `tag_class()`
//!   (`StarlarkRepositoryModule`), `repository_ctx` and `module_ctx` and their shared base
//!   (`StarlarkBaseExternalContext`), path, exec_result, repository_os, repo_metadata.
//! - [`repository`]: REPO_DEFINITION, REPOSITORY_DIRECTORY (`RepositoryFetchFunction`, marker
//!   files, repo contents cache), recorded inputs, REPOSITORY_ENVIRONMENT_VARIABLE,
//!   DIRECTORY_TREE_DIGEST.
//! - [`bzlmod`]: SINGLE_EXTENSION_EVAL, SINGLE_EXTENSION_USAGES,
//!   MODULE_EXTENSION_REPO_MAPPING_ENTRIES, `bazel_module`, tags, extension metadata.
//! - [`external_repos`]: the `blaze_sky_loading::repos::ExternalRepos` implementation
//!   (REPOSITORY_MAPPING / REPOSITORY_DIRECTORY).
//! - [`registration`]: the Starlark environment and SkyFunction wiring.
#![allow(
    clippy::too_many_arguments,
    clippy::new_without_default,
    clippy::type_complexity,
    clippy::large_enum_variant
)]

pub mod attr_values;
pub mod bzl_load;
pub mod bzlmod;
pub mod env_handle;
pub mod external_repos;
pub mod precomputed;
pub mod registration;
pub mod repository;
pub mod semantics_fingerprint;
pub mod services;
pub mod setup;
pub mod starlark;
pub mod workspace_rule_event;
