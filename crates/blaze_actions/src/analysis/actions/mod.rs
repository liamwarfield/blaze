//! Port of `com.google.devtools.build.lib.analysis.actions`.

pub mod abstract_file_write_action;
pub mod binary_file_write_action;
pub mod build_info_file_write_action;
pub mod custom_command_line;
pub mod file_write_action;
pub mod lazy_write_nested_set_of_tuple_action;
pub mod lazy_write_paths_file_action;
pub mod parameter_file_write_action;
pub mod path_mappers;
pub mod spawn_action;
pub mod starlark_action;
pub mod substitution;
pub mod symlink_action;
pub mod symlink_tree_action;
pub mod template;
pub mod template_expansion_action;
