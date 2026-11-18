//! Post-analysis queries: `query2/PostAnalysisQueryEnvironment`, `query2/cquery` (configured
//! target query), `query2/aquery` (action graph query), the processors of `buildtool` that run
//! them after the analysis phase, and the bodies of the `cquery` / `aquery` commands.
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

pub mod aquery;
pub mod buildtool;
pub mod commands;
pub mod cquery;
pub mod post_analysis_query_environment;
