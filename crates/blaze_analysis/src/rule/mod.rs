//! `RuleContext` and the analysis core every rule's analysis goes through (agent R;
//! crates/blaze_analysis/DESIGN.md): `ConfiguredTargetFactory`, `RuleContext` (+ builder and
//! prerequisite validation), `RuleConfiguredTargetBuilder`, `PrerequisitesCollection`,
//! `PrerequisiteArtifacts`, `CachingAnalysisEnvironment`, make-variable and location expansion,
//! conversions between `blaze_build_config` and `blaze_actions` types, and NestedSet <-> depset
//! conversion.

pub mod action_environments;
pub mod actions_util;
pub mod analysis_environment;
pub mod analysis_utils;
pub mod artifact_roots;
pub mod builtin_restriction;
pub mod command_helper;
pub mod common_prerequisite_validator;
pub mod config_util;
pub mod configuration_make_variable_context;
pub mod configured_target_and_data_ext;
pub mod configured_target_factory;
pub mod depsets;
pub mod expander;
pub mod location_expander;
pub mod prerequisite_artifacts;
pub mod prerequisites_collection;
pub mod rule_configured_target_builder;
pub mod rule_context;
pub mod rule_error_consumer;
pub mod runfiles_support;
pub mod runfiles_util;
pub mod sh_toolchain;
pub mod stringtemplate;
pub mod target_context;

pub use analysis_environment::{AnalysisEnvironment, CachingAnalysisEnvironment};
pub use configured_target_and_data_ext::ConfiguredTargetAndDataExt;
pub use configured_target_factory::ConfiguredTargetFactory;
pub use prerequisite_artifacts::PrerequisiteArtifacts;
pub use prerequisites_collection::PrerequisitesCollection;
pub use rule_configured_target_builder::RuleConfiguredTargetBuilder;
pub use rule_context::{PrerequisiteMap, RuleContext, RuleContextBuilder};
pub use rule_error_consumer::{RuleErrorConsumer, RuleErrorException};
pub use target_context::TargetContext;

/// Converts a loading-phase location (`blaze_starlark::Location`, what targets carry) to the
/// event location (`blaze_events::Location`, what `Event`s and `ActionOwner`s carry). Both are
/// ports of `net.starlark.java.syntax.Location`.
pub fn to_event_location(loc: &blaze_starlark::Location) -> blaze_events::Location {
    blaze_events::Location::new(loc.file.clone(), loc.line as i32, loc.column as i32)
}
