//! Repository fetching (Java `com.google.devtools.build.lib.bazel.repository`,
//! `rules.repository`).

pub mod attribute_utils;
pub mod digest_writer;
pub mod directory_tree_digest;
pub mod exceptions;
pub mod repo_definition;
pub mod repo_definition_function;
pub mod repo_environment_function;
pub mod repo_recorded_input;
pub mod remote_helpers;
pub mod repo_rule;
pub mod repository_fetch_function;
pub mod repository_resolved_event;
pub mod repository_utils;
