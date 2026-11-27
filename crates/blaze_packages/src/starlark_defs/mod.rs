//! The .bzl rule-definition builtins (Java `analysis/starlark/StarlarkRuleClassFunctions`,
//! `StarlarkAttrModule`, `packages/StarlarkProvider`, `BazelBuildApiGlobals`, the config /
//! platform / testing modules, and the builtin providers usable at load time).
//!
//! PORT: move to blaze_analysis (these are Java `analysis/` classes). Owner: agent B
//! (crates/blaze_packages/DESIGN.md).
//!
//! Starlark objects held by model types (implementation functions, computed defaults,
//! transitions, aspects, subrules, initializers) are handed to the model as
//! `StarlarkCallable` handles that are filled when the defining module freezes; see
//! [`value_cell`].

use std::sync::Arc;

use starlark::environment::GlobalsBuilder;
use starlark::values::Value;
use starlark::values::ValueLike;

use crate::rule_class::RuleClass;

pub mod aspect;
pub mod attr_module;
pub mod builtin_providers;
pub mod bzl_globals;
pub mod config;
pub mod config_common;
pub mod documentable;
pub mod macro_function;
pub mod platform_common;
pub mod provider;
pub mod rule_class_functions;
pub mod subrule;
pub mod testing;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_ignored;
pub mod util;
pub mod value_cell;

/// Registers every fixed .bzl toplevel of this module (Java
/// `StarlarkGlobalsImpl.getFixedBzlToplevels` minus `native` and the util toplevels `depset`,
/// `json`, `proto`, `select`). The rule-class-provider bootstraps are in
/// [`register_rule_class_provider_bzl_toplevels`].
///
/// Not registered here (environment composition, agent D): the context-guarded symbols
/// (`cc_common`, `apple_common`, `java_common`, `py_internal`, `android_common`,
/// `CcToolchainConfigInfo`, the CcRules empty symbols `CcInfo`, `DebugPackageInfo`, ...),
/// `native`, and `_builtins` injection.
// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkGlobalsImpl.java:91 StarlarkGlobalsImpl.getFixedBzlToplevels
pub fn register_bzl_toplevels(builder: &mut GlobalsBuilder) {
    // BazelBuildApiGlobals: visibility, configuration_field.
    bzl_globals::register_bazel_build_api_globals(builder);
    // StarlarkRuleClassFunctions: provider, macro, rule, aspect, Label, exec_group, subrule,
    // materializer_rule.
    provider::register_provider(builder);
    macro_function::register_macro(builder);
    rule_class_functions::register_rule_class_functions(builder);
    aspect::register_aspect(builder);
    blaze_starlark::label::register_label(builder);
    attr_module::register_attr(builder);
    blaze_starlark::structs::register_struct(builder);
    // OutputGroupInfo, Actions, DefaultInfo, RunEnvironmentInfo, MaterializedDepsInfo.
    builtin_providers::register_fixed_bzl_providers(builder);
}

/// Registers the .bzl toplevels Bazel's rule class provider adds with `addStarlarkBootstrap` /
/// `addBzlToplevel` (Java `ConfiguredRuleClassProvider` `starlarkAccessibleTopLevels`): unlike
/// the fixed ones of [`register_bzl_toplevels`], `@_builtins` may override them.
// Java: com/google/devtools/build/lib/analysis/ConfiguredRuleClassProvider.java:331 ConfiguredRuleClassProvider.Builder.addBzlToplevel
pub fn register_rule_class_provider_bzl_toplevels(builder: &mut GlobalsBuilder) {
    // ConfigRules: ConfigBootstrap (config_common, config, transition, exec_transition,
    // analysis_test_transition).
    config_common::register_config_common(builder);
    config::register_config(builder);
    config::register_config_global_library(builder);
    // PlatformRules: PlatformBootstrap.
    platform_common::register_platform_common(builder);
    // TestingSupportRules: TestingBootstrap (testing, coverage_common, InstrumentedFilesInfo,
    // AnalysisFailureInfo, AnalysisTestResultInfo).
    testing::register_testing(builder);
    builtin_providers::register_testing_providers(builder);
    // BazelRuleClassProvider.PROTO_RULES: proto_common_do_not_use.
    builtin_providers::register_proto_rules(builder);
    // BazelRuleClassProvider.PACKAGING_RULES: PackageSpecificationInfo.
    builtin_providers::register_packaging_rules(builder);
}

/// The rule class of a native rule function value (Java `RuleFunction` that is not a
/// `StarlarkRuleFunction`).
pub fn native_rule_class_of(v: Value) -> Option<Arc<RuleClass>> {
    v.downcast_ref::<crate::native_rules::BuiltinRuleFunctionValue>()
        .map(|f| f.get_rule_class())
}
