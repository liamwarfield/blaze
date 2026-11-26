//! The Starlark side: `repository_rule()` / `module_extension()` / `tag_class()`, the
//! `repository_ctx` / `module_ctx` objects and their helper values.

pub mod base_external_context;
pub mod context;
pub mod execution_result;
pub mod os;
pub mod path;
pub mod repo_metadata;
pub mod repository_context;
pub mod repository_module;
pub mod thread;
pub mod wasm;
