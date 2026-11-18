//! The `SkyframeExecutor` of one command and the analysis phase on top of it: Java's
//! `SkyframeExecutor` assembly (loading, bzlmod, configuration, toolchain and configured-target
//! functions in one evaluator), `TargetPatternPhaseFunction` (top-level targets from patterns,
//! test_suite expansion), `BuildView.update` / `SkyframeBuildView.configureTargets`,
//! `AnalysisPhaseRunner` and the analysis-only path of `BuildTool`.
//!
//! Design and ownership: `crates/blaze_analysis/DESIGN.md`.

#![allow(clippy::too_many_arguments, clippy::new_without_default)]
#![allow(clippy::collapsible_if, clippy::type_complexity)]

pub mod analysis_phase_runner;
pub mod build_tool;
pub mod build_view;
pub mod execution_phase;
pub mod execution_seam;
pub mod skyframe_build_view;
pub mod skyframe_executor;
pub mod target_pattern_phase;
