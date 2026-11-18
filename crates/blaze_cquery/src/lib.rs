//! Post-analysis queries: `query2/PostAnalysisQueryEnvironment`, `query2/cquery` (configured
//! target query), `query2/aquery` (action graph query), the processors of `buildtool` that run
//! them after the analysis phase, and the bodies of the `cquery` / `aquery` commands.
//!
//! Design and ownership: `crates/blaze_analysis/DESIGN.md`.

#![allow(clippy::too_many_arguments, clippy::new_without_default)]
#![allow(clippy::collapsible_if, clippy::type_complexity)]

pub mod aquery;
pub mod buildtool;
pub mod commands;
pub mod cquery;
pub mod post_analysis_query_environment;
