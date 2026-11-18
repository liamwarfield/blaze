//! Port of `lib/bazel/bzlmod/modcommand` (ModExecutor, output formatters, ModuleArg/ExtensionArg
//! resolution, InvalidArgumentException) and the post-evaluation part of
//! `bazel/commands/ModCommand`. OWNER: agent B.
//!
//! Not ported: `RepoOutputFormatter` and `ModExecutor.showRepo` (need `RepoDefinitionValue`),
//! `mod dump_repo_mapping` (needs `RepositoryMappingValue`) and `mod tidy` (needs
//! `BazelModTidyValue`).

pub mod extension_arg;
pub mod graphviz_output_formatter;
pub mod invalid_argument_exception;
pub mod java_hash;
pub mod json_output_formatter;
pub mod maybe_complete_set;
pub mod mod_command;
pub mod mod_executor;
pub mod module_arg;
pub mod output_formatters;
pub mod text_output_formatter;

pub use extension_arg::ExtensionArg;
pub use invalid_argument_exception::{InvalidArgumentException, ModCommandCode};
pub use maybe_complete_set::MaybeCompleteSet;
pub use mod_executor::{
    IsCycle, IsExpanded, IsIndirect, ModExecutor, NodeMetadata, ResultNode, ResultNodeBuilder,
};
pub use module_arg::ModuleArg;
pub use output_formatters::{Explanation, OutputFormatter, Printer, get_formatter};
