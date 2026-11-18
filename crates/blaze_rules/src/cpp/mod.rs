//! Port of the analysis-time C++ natives of `com.google.devtools.build.lib.rules.cpp` (agent CC;
//! crates/blaze_analysis/DESIGN.md): `cc_internal` (`CcStarlarkInternal`), `cc_common`
//! (`BazelCcModule`), the Starlark values over the `blaze_cc_features` engine
//! (`FeatureConfiguration`, `CcToolchainFeatures`, `Variables`, `HeaderInfo`), the `cpp`
//! configuration fragment, and `CppCompileAction` / `CppCompileActionBuilder`.
//!
//! The lead wires [`cc_internal_value`] and [`cc_common_value`] into
//! `_builtins.internal.{cc_internal, cc_common}` and calls [`register_fragments`] once.

pub mod cc_artifact;
pub mod cc_compilation_context;
pub mod cc_module;
pub mod cc_starlark_internal;
pub mod cc_toolchain_provider;
pub mod cpp_compile_action;
pub mod cpp_configuration;
pub mod cpp_file_types;
pub mod cpp_include_extraction_context;
pub mod header_info;
pub mod link_command_line;
pub mod starlark_values;
pub mod util;
pub mod variables;
pub mod wrapped_action_factory;

use starlark::values::OwnedFrozenValue;

/// `_builtins.internal.cc_internal` (Java `CcStarlarkInternal`).
// Java: com/google/devtools/build/lib/rules/cpp/CcStarlarkInternal.java:87 CcStarlarkInternal
pub fn cc_internal_value() -> OwnedFrozenValue {
    blaze_analysis::info::freeze_native_info(cc_starlark_internal::cc_internal())
}

/// `_builtins.internal.cc_common` (Java `BazelCcModule`).
// Java: com/google/devtools/build/lib/bazel/rules/cpp/BazelCcModule.java:22 BazelCcModule
pub fn cc_common_value() -> OwnedFrozenValue {
    blaze_analysis::info::freeze_native_info(cc_module::cc_common())
}

/// Registers the Starlark values of the C++ configuration fragments (`cpp`).
pub fn register_fragments() {
    cpp_configuration::register_cpp_fragment();
}
