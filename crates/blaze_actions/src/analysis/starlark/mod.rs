//! Port of the action-related parts of `com.google.devtools.build.lib.analysis.starlark`
//! (`Args`, `StarlarkCustomCommandLine`, `TemplateDict`, `UnresolvedSymlinkAction`) and the
//! Starlark `File` value.

pub mod args;
pub mod starlark_custom_command_line;
pub mod starlark_file;
pub mod template_dict;
pub mod unresolved_symlink_action;
