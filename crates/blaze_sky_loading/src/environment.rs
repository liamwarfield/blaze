//! Bazel's Starlark environments: the composition of every agent's natives into
//! `BazelStarlarkEnvironment` (Java: `ConfiguredRuleClassProvider.getBazelStarlarkEnvironment()`
//! for the Bazel rule class provider).
//!
//! Environment composition contract: crates/blaze_packages/DESIGN.md.

use std::sync::Arc;

use blaze_packages::builtins::env::env_from_registration;
use blaze_packages::builtins::{
    BazelStarlarkEnvironment, RuleClassProviderRegistrations, StarlarkGlobals, StarlarkGlobalsFns,
};
use blaze_packages::native_rules::RuleClassProvider;
use blaze_packages::native_rules::builtin_rule_function::add_rule_functions;

/// Java `BlazeVersionInfo.instance().getVersion()` of the pinned Bazel (`native.bazel_version`
/// in MODULE-loaded .bzl files).
pub const BAZEL_VERSION: &str = "9.3.2";

/// The registration functions of the fixed symbols of each dialect (Java
/// `StarlarkGlobalsImpl.INSTANCE`).
pub fn bazel_starlark_globals_fns() -> StarlarkGlobalsFns {
    StarlarkGlobalsFns {
        // Java addCommonUtilToplevels (depset, json, proto) + SelectLibrary.
        // PORT: the `proto` module (Java Proto.INSTANCE) is not ported.
        util_toplevels: vec![
            blaze_starlark::depset::register_depset,
            blaze_starlark::json::register_json,
            blaze_packages::build::register_select,
        ],
        build_file_toplevels_shared_with_native: vec![
            blaze_packages::build::register_native_module_functions,
        ],
        build_globals: vec![blaze_packages::build::register_build_globals],
        bzl_globals: vec![
            blaze_starlark::structs::register_struct,
            blaze_starlark::label::register_label,
            blaze_packages::starlark_defs::register_bzl_toplevels,
        ],
        scl_toplevels: vec![blaze_starlark::structs::register_struct],
    }
}

/// Bazel's `BazelStarlarkEnvironment` over `rule_class_provider`'s native rules.
// Java: com/google/devtools/build/lib/analysis/ConfiguredRuleClassProvider.java:612 ConfiguredRuleClassProvider.<init>
pub fn bazel_starlark_environment(
    rule_class_provider: &RuleClassProvider,
) -> Arc<BazelStarlarkEnvironment> {
    let rule_functions =
        env_from_registration(&|b| add_rule_functions(b, rule_class_provider.get_rule_functions()));
    Arc::new(BazelStarlarkEnvironment::new(
        StarlarkGlobals::new(&bazel_starlark_globals_fns()),
        BAZEL_VERSION,
        rule_functions,
        &RuleClassProviderRegistrations::bazel(),
    ))
}
