//! `FragmentOptions` subclasses (every class that `extends FragmentOptions`, directly or
//! transitively, in Bazel 9.3.2), their converters and the small value classes their option
//! fields hold. One module per Java class.
//!
//! In 9.3.2 no `FragmentOptions` subclass overrides a `getExec()` hook any more: the exec
//! transition is Starlark (`builtins_bzl/common/builtin_exec_platforms.bzl`, selected by
//! `--experimental_exec_config`). The Java hooks that remain and affect configuration identity are
//! `getNormalized()` (CoreOptions, PlatformOptions, TestOptions) and the normalization helpers of
//! `FragmentOptions`, ported in [`fragment_options`].

pub mod fragment_options;

// Converters and value classes.
pub mod apple_configuration;
pub mod apple_platform;
pub mod auto_cpu_converter;
pub mod compilation_mode;
pub mod converters;
pub mod cpp_configuration;
pub mod dotted_version;
pub mod execution_info_modifier;
pub mod java_configuration;
pub use crate::common::per_label_options;
pub mod platform_mapping_key;
pub mod proto_constants;
pub mod python_version;
pub use crate::common::regex_filter;
pub use crate::common::resource_converter;
pub mod run_under;
pub mod shell_utils;
pub mod test_resources_converter;
pub mod test_sharding_strategy;
pub use crate::common::test_size;
pub use crate::common::test_timeout;

// FragmentOptions subclasses.
pub mod android_configuration;
pub mod apple_command_line_options;
pub mod bazel_android_configuration;
pub mod bazel_python_configuration;
pub mod bazel_rule_class_provider;
pub mod config_feature_flag_options;
pub mod configuration_for_output;
pub mod core_options;
pub mod coverage_configuration;
pub mod cpp_options;
pub mod j2objc_command_line_options;
pub mod java_options;
pub mod objc_command_line_options;
pub mod platform_options;
pub mod proto_configuration;
pub mod python_options;
pub mod shell_configuration;
pub mod test_configuration;

// Not a FragmentOptions subclass, but SPLIT.tsv assigned it to this group.
pub mod config_command;

#[cfg(test)]
mod test_util;

pub use fragment_options::FragmentOptions;
