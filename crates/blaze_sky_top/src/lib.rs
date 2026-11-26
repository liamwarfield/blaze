//! The `SkyframeExecutor` of one command and the analysis phase on top of it: Java's
//! `SkyframeExecutor` assembly (loading, bzlmod, configuration, toolchain and configured-target
//! functions in one evaluator), `TargetPatternPhaseFunction` (top-level targets from patterns,
//! test_suite expansion), `BuildView.update` / `SkyframeBuildView.configureTargets`,
//! `AnalysisPhaseRunner` and the analysis-only path of `BuildTool`.
//!
//! Design and ownership: `crates/blaze_analysis/DESIGN.md`.

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

pub mod action_artifact_cycle_reporter;
pub mod action_conflict_exception;
pub mod action_lookup_conflict_finding_function;
pub mod action_template_expansion_function;
pub mod analysis_failure_event;
pub mod analysis_phase_runner;
pub mod artifact_conflict_finder;
pub mod build_tool;
pub mod build_view;
pub mod bzlmod_cycle_reporters;
pub mod cycles_reporter;
pub mod execution_phase;
pub mod execution_seam;
pub mod incremental;
pub mod instrumentation_filter_support;
pub mod pattern_expanding_error;
pub mod project;
pub mod skyframe_build_view;
pub mod skyframe_error_processor;
pub mod skyframe_executor;
pub mod skyframe_progress_receiver;
pub mod target_pattern_phase;
#[cfg(any(test, feature = "testing"))]
pub mod testing;
#[cfg(test)]
mod tests;
pub mod toolchain_cycle_reporters;
