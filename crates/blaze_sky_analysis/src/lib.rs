//! Port of the analysis SkyFunctions of `lib/skyframe` (`ConfiguredTargetFunction`,
//! `DependencyResolver`, `analysis/producers`, toolchain resolution, `PlatformFunction`,
//! `AspectFunction`). Design and ownership: `crates/blaze_analysis/DESIGN.md`.

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

pub mod analysis_progress_receiver;
pub mod aspect_function;
pub mod build_options_scope_function;
pub mod configured_target_function;
pub mod cquery_transition_resolver;
pub mod dependency_resolver;
pub mod flag_set_function;
pub mod incompatible_target_checker;
pub mod late_bound_defaults;
pub mod load_aspects_function;
pub mod parsed_flags_function;
pub mod platform;
pub mod producers;
pub mod registration;
pub mod starlark_options_parser;
pub mod toolchains;
pub mod top_level_status_events;
pub mod toplevel_starlark_aspect_function;
