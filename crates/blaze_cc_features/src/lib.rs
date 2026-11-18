//! Port of the Starlark- and analysis-independent core of
//! `com.google.devtools.build.lib.rules.cpp` (Bazel 9.3.2): the C++ toolchain feature engine
//! (`CcToolchainFeatures`, `FeatureSelection`, `CcToolchainFeaturesLib`), build variables
//! (`CcToolchainVariables`), the compile and link command lines, `.d` file parsing
//! (`util/DependencySet`), header discovery and include validation.
//!
//! rules_cc 0.2.17 drives this through `cc_internal` / `cc_common` / `FeatureConfiguration`. The
//! Starlark wrappers live in another crate; every Starlark-facing type here is plain Rust with the
//! Java method names so a thin `#[starlark_module]` can sit on top.

#![allow(clippy::too_many_arguments, clippy::new_without_default)]

pub mod actions;
pub mod artifact_category;
pub mod cc_common;
pub mod cc_module;
pub mod cc_starlark_internal;
pub mod cc_toolchain_config_info;
pub mod cc_toolchain_features;
pub mod cc_toolchain_features_lib;
pub mod cc_toolchain_variables;
pub mod compile_build_variables;
pub mod compile_command_line;
pub mod cpp_action_names;
pub mod cpp_compile_action;
pub mod cpp_rule_classes;
pub mod dependency_set;
pub mod errors;
pub mod feature_configuration_for_starlark;
mod feature_selection;
pub mod header_discovery;
pub mod include_problems;
pub mod link;
pub mod link_command_line;
pub mod starlark_info;

pub use artifact_category::ArtifactCategory;
pub use cc_toolchain_config_info::CcToolchainConfigInfo;
pub use cc_toolchain_features::{CcToolchainFeatures, FeatureConfiguration};
pub use cc_toolchain_variables::{CcToolchainVariables, CcToolchainVariablesRef};
pub use cpp_action_names::CppActionNames;
pub use errors::{CollidingProvidesException, EvalException, ExpansionException};
pub use feature_configuration_for_starlark::FeatureConfigurationForStarlark;

// Everything Skyframe or the Starlark layer may share across threads is Send + Sync.
const _: fn() = || {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<CcToolchainFeatures>();
    assert_send_sync::<FeatureConfiguration>();
    assert_send_sync::<FeatureConfigurationForStarlark>();
    assert_send_sync::<CcToolchainVariables>();
    assert_send_sync::<CcToolchainConfigInfo>();
    assert_send_sync::<compile_command_line::CompileCommandLine>();
    assert_send_sync::<link_command_line::LinkCommandLine>();
};
