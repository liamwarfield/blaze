//! Port of Bazel's analysis phase core (`com.google.devtools.build.lib.analysis` and the analysis
//! keys and values of `lib/skyframe`): configured targets and their providers, `RuleContext`,
//! the Starlark rule context (`ctx`) and action factory, platform and toolchain providers,
//! configured attribute mapping and transitions, and test actions.
//!
//! Design, shared representations and file ownership: `crates/blaze_analysis/DESIGN.md`.

#![allow(clippy::too_many_arguments, clippy::new_without_default)]
// Ported signatures and Java's control flow win (PORTING.md "Style"): maps keyed by `Artifact`
// (Java `Map<Artifact, ...>`), exceptions carried by value, Java method names (`from_*`), Java's
// branch structure.
#![allow(
    clippy::mutable_key_type,
    clippy::result_large_err,
    clippy::large_enum_variant,
    clippy::type_complexity,
    clippy::wrong_self_convention,
    clippy::collapsible_if,
    clippy::collapsible_match,
    clippy::if_same_then_else
)]

// Lead-owned contract files.
pub mod analysis_services;
pub mod configured_rule_class_provider;
pub mod configured_target;
pub mod configured_target_and_data;
pub mod configured_target_key;
pub mod configured_target_value;
pub mod dependency_kind;
pub mod info;
pub mod provider_map;

// Agent-owned areas (DESIGN.md "Ownership").
pub mod config;
pub mod configuredtargets;
pub mod constraints;
pub mod platform;
pub mod providers;
pub mod rule;
pub mod starlark;
pub mod test;
pub mod toolchains;
