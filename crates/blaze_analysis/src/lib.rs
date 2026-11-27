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
pub mod actions;
pub mod analysis_services;
pub mod base_target_prerequisites_supplier;
pub mod configured_rule_class_provider;
pub mod configured_target;
pub mod configured_target_and_data;
pub mod configured_target_key;
pub mod configured_target_value;
pub mod coverage_report_value;
pub mod dependency_kind;
pub mod dormant_dependency;
pub mod info;
pub mod provider_map;
pub mod required_fragments_util;
pub mod skyframe_env;

// Aspects wave, W3 (DESIGN.md "Ownership (aspects wave)").
pub mod aspect_aware_attribute_mapper;
pub mod aspect_collection;
pub mod aspect_key_creator;
pub mod aspect_resolution_helpers;
pub mod aspect_value;
pub mod configured_aspect;
pub mod configured_aspect_factory;
pub mod inconsistent_aspect_order_exception;

// Aspects wave, W4.
pub mod aspect_context;

// Agent-owned areas (DESIGN.md "Ownership").
pub mod bazel;
pub mod config;
pub mod configuredtargets;
pub mod constraints;
pub mod extra;
pub mod platform;
pub mod providers;
pub mod rule;
pub mod starlark;
pub mod test;
pub mod toolchains;
