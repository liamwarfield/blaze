//! Build configurations (wave W4, configuration half): `BuildOptions` and its checksum,
//! `BuildConfigurationValue`, `OutputDirectories`, `OutputPathMnemonicComputer`, the configuration
//! fragments, Starlark transitions (the exec transition) and the configuration SkyFunctions.
//!
//! PORT: the architecture map (docs/research/06 §5) puts `BuildConfigurationValue` and friends in
//! `blaze_actions` and the low slice of `analysis/config` in `blaze_config`; this crate holds the
//! W4 configuration work until root reconciles the placement.

#![allow(clippy::too_many_arguments, clippy::new_without_default)]
// Ported control flow keeps Java's branch structure (identical branches, nested ifs).

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

pub mod action_environment;
pub mod bazel_rule_class_provider;
pub mod build_configuration_key;
pub mod build_configuration_value;
pub mod build_options;
pub mod build_options_scope_value;
pub mod config_output;
pub mod core_option_converters;
pub mod flag_aliases;
pub mod fragment;
pub mod fragment_options_class;
pub mod fragments;
pub mod function_transition_util;
pub mod options_diff;
pub mod output_directories;
pub mod output_path_mnemonic_computer;
pub mod platform;
pub mod skyframe;
pub mod starlark_transition;
pub mod top_level;
pub mod transitions;
