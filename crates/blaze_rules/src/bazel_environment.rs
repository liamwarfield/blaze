//! Blaze's analysis-capable Starlark environment: W3's loading environment
//! (`blaze_sky_loading::environment`) with the analysis natives substituted
//! (crates/blaze_analysis/DESIGN.md "Builtins / environment").
//!
//! Java has one `BazelStarlarkEnvironment` whose objects work in both phases; Blaze's loading
//! crates register loading-only stand-ins for analysis natives, which this module replaces with
//! the analysis implementations (same Starlark names and provider identities).

use std::sync::Arc;

use blaze_packages::builtins::env::{EnvEntry, env_from_registration};
use blaze_packages::builtins::{
    BazelStarlarkEnvironment, RuleClassProviderRegistrations, StarlarkGlobals, StarlarkGlobalsFns,
};
use blaze_packages::native_rules::RuleClassProvider;
use blaze_packages::native_rules::builtin_rule_function::add_rule_functions;
use starlark::environment::GlobalsBuilder;

/// W3's fixed .bzl toplevels followed by the analysis overrides (later registrations replace
/// earlier ones of the same name).
fn bzl_toplevels_with_analysis(builder: &mut GlobalsBuilder) {
    blaze_packages::starlark_defs::register_bzl_toplevels(builder);
    analysis_bzl_overrides(builder);
}

/// The analysis implementations of .bzl toplevels the loading phase also defines.
pub fn analysis_bzl_overrides(builder: &mut GlobalsBuilder) {
    blaze_analysis::providers::register_provider_toplevels(builder);
    blaze_analysis::platform::platform_common::register_platform_common(builder);
    blaze_analysis::test::starlark_testing_module::register_testing_toplevels(builder);
    blaze_analysis::test::instrumented_files_info::register_coverage_common(builder);
    crate::config::config_starlark_common::register_config_common(builder);
}

/// The registration functions of the fixed symbols of each dialect (Java
/// `StarlarkGlobalsImpl.INSTANCE`), with the analysis overrides.
pub fn bazel_starlark_globals_fns() -> StarlarkGlobalsFns {
    let mut fns = blaze_sky_loading::environment::bazel_starlark_globals_fns();
    fns.bzl_globals = vec![
        blaze_starlark::structs::register_struct,
        blaze_starlark::label::register_label,
        bzl_toplevels_with_analysis,
    ];
    fns
}

/// The rule class provider's registrations with the analysis overrides (Java
/// `addBzlToplevel` / `addStarlarkBuiltinsInternal` of the rule sets).
pub fn registrations() -> RuleClassProviderRegistrations {
    let mut registrations = RuleClassProviderRegistrations::bazel();
    // `_builtins.internal` natives with analysis behaviour.
    registrations.builtins_internals.insert(
        "py_builtins".to_owned(),
        EnvEntry::plain(crate::python::py_builtins_value()),
    );
    registrations.builtins_internals.insert(
        "cc_internal".to_owned(),
        EnvEntry::plain(crate::cpp::cc_internal_value()),
    );
    registrations.builtins_internals.insert(
        "cc_common".to_owned(),
        EnvEntry::plain(crate::cpp::cc_common_value()),
    );
    registrations.builtins_internals.insert(
        "apple_common".to_owned(),
        EnvEntry::plain(crate::apple::apple_common_value()),
    );
    let overrides = env_from_registration(&analysis_bzl_overrides);
    for (name, entry) in overrides {
        if registrations.registered_bzl_toplevels.contains_key(&name) {
            registrations.registered_bzl_toplevels.insert(name, entry);
        }
    }
    registrations
}

/// Bazel's `BazelStarlarkEnvironment` over `rule_class_provider`'s native rules, with analysis
/// natives.
// Java: com/google/devtools/build/lib/analysis/ConfiguredRuleClassProvider.java:612 ConfiguredRuleClassProvider.<init>
pub fn bazel_starlark_environment(
    rule_class_provider: &RuleClassProvider,
) -> Arc<BazelStarlarkEnvironment> {
    let rule_functions =
        env_from_registration(&|b| add_rule_functions(b, rule_class_provider.get_rule_functions()));
    Arc::new(BazelStarlarkEnvironment::new(
        StarlarkGlobals::new(&bazel_starlark_globals_fns()),
        blaze_sky_loading::environment::BAZEL_VERSION,
        rule_functions,
        &registrations(),
    ))
}
