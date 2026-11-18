//! Port of the analysis SkyFunctions of `lib/skyframe` (`ConfiguredTargetFunction`,
//! `DependencyResolver`, `analysis/producers`, toolchain resolution, `PlatformFunction`,
//! `AspectFunction`). Design and ownership: `crates/blaze_analysis/DESIGN.md`.

#![allow(clippy::too_many_arguments, clippy::new_without_default)]
#![allow(
    clippy::collapsible_if,
    clippy::type_complexity,
    clippy::if_same_then_else
)]

pub mod analysis_progress_receiver;
pub mod aspect_function;
pub mod build_options_scope_function;
pub mod configured_target_function;
pub mod dependency_resolver;
pub mod incompatible_target_checker;
pub mod late_bound_defaults;
pub mod platform;
pub mod producers;
pub mod registration;
pub mod toolchains;
