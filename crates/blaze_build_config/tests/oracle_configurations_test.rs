//! Blaze-only (not a Java port): configuration checksums of the oracle (Bazel 9.3.2) for the
//! corpus projects. The expected values are the oracle's real outputs.
//!
//! The top-level options are built the way the oracle builds them for brotli and re2: every
//! fragment options class sorted by Java class name, the four rules_python `--flag_alias` options
//! of the oracle's command line, then `PlatformOptions.platforms` set to
//! `@@bazel_tools//tools:host_platform` (what the empty platform mapping does).

use std::sync::Arc;

use blaze_build_config::build_options::BuildOptions;
use blaze_build_config::fragment_options_class::{
    FragmentOptionsClass, all_known_fragment_options_classes,
};
use blaze_build_config::function_transition_util::{
    exec_transition_patch, exec_transition_required_fragments,
};
use blaze_build_config::starlark_transition::load_starlark_exec_transition;
use blaze_build_config::transitions::{EMPTY_OPTIONS, restrict, test_trimming_trim};
use blaze_cmdline::Label;
use blaze_events::{Event, EventHandler};
use blaze_optdefs::fragment::platform_options::PlatformOptions;
use blaze_options::Foreign;
use blaze_starlark::StarlarkSemantics;

const FLAG_ALIASES: [&str; 4] = [
    "--flag_alias=build_python_zip=@@rules_python+//python/config_settings:build_python_zip",
    "--flag_alias=incompatible_default_to_explicit_init_py=@@rules_python+//python/config_settings:incompatible_default_to_explicit_init_py",
    "--flag_alias=python_path=@@rules_python+//python/config_settings:python_path",
    "--flag_alias=experimental_python_import_all_repositories=@@rules_python+//python/config_settings:experimental_python_import_all_repositories",
];

/// re2's `.bazelrc` build options.
const RE2_BAZELRC: [&str; 3] = [
    "--features=layering_check",
    "--features=parse_headers",
    "--cxxopt=-std=c++17",
];

const BROTLI_TOP_LEVEL: &str = "e722a217ced723dfeb49874b2007895ae0dc9a9d49d004d95320b88c609c409a";
const BROTLI_TRIMMED: &str = "a7a71fd16b14112464272f754633480c74c664f2488224c592ebf2dfc7a06b18";
const RE2_TOP_LEVEL: &str = "d2d2a0afb30f952b7fcf9b350ea1ac28b281e96df9d843b449b97ffd15360a9b";
const RE2_TRIMMED: &str = "30daceab53532d26d8b2ce18aec2262d49e47ed3f89221149abc0d46b920144e";
/// The exec configuration of `@@platforms//host:host`, the same for both projects.
const EXEC: &str = "04bf134dfa57115b01daa56bd3a30421a6e74b1819e002496a26c94ed0087067";
const EMPTY: &str = "3030ed4c810cf3435d066294fa495e650fe75173e4b4ef410ea56cb683458569";

struct NoopEventHandler;

impl EventHandler for NoopEventHandler {
    fn handle(&self, _event: &Event) {}
}

fn sorted_classes() -> Vec<FragmentOptionsClass> {
    let mut classes = all_known_fragment_options_classes();
    classes.sort_by(|a, b| a.get_name().cmp(b.get_name()));
    classes
}

/// The top-level options for `extra_args` after the flag aliases.
fn top_level(extra_args: &[&str]) -> BuildOptions {
    let mut args: Vec<&str> = FLAG_ALIASES.to_vec();
    args.extend_from_slice(extra_args);
    let mut options = BuildOptions::of_args(&sorted_classes(), &args)
        .unwrap_or_else(|e| panic!("{}", e.get_message()));
    options.get_mut::<PlatformOptions>().unwrap().platforms = vec![Foreign(
        Label::parse_canonical_unchecked("@@bazel_tools//tools:host_platform"),
    )];
    // Fresh checksum after the mutation.
    BuildOptions::clone(&options)
}

/// `test_trimming_trim(exec_transition_patch(@@platforms//host:host, trimmed))` with the Starlark
/// exec transition loaded from the top-level options.
fn exec_of(top_level: &BuildOptions, trimmed: &BuildOptions) -> BuildOptions {
    let semantics = Arc::new(StarlarkSemantics::default());
    let transition = load_starlark_exec_transition(Some(top_level), &semantics)
        .unwrap_or_else(|e| panic!("{}", e))
        .expect("Starlark exec transition");
    let host = Label::parse_canonical_unchecked("@@platforms//host:host");
    let view = restrict(exec_transition_required_fragments(), trimmed);
    let exec = exec_transition_patch(
        Some(&host),
        &transition,
        &view,
        &semantics,
        &NoopEventHandler,
    );
    test_trimming_trim(&exec)
}

#[test]
fn brotli_top_level_checksum() {
    assert_eq!(top_level(&[]).checksum(), BROTLI_TOP_LEVEL);
}

#[test]
fn brotli_trimmed_checksum() {
    assert_eq!(
        test_trimming_trim(&top_level(&[])).checksum(),
        BROTLI_TRIMMED
    );
}

#[test]
fn brotli_exec_checksum() {
    let top = top_level(&[]);
    let trimmed = test_trimming_trim(&top);
    assert_eq!(exec_of(&top, &trimmed).checksum(), EXEC);
}

#[test]
fn re2_top_level_checksum() {
    assert_eq!(top_level(&RE2_BAZELRC).checksum(), RE2_TOP_LEVEL);
}

#[test]
fn re2_trimmed_checksum() {
    assert_eq!(
        test_trimming_trim(&top_level(&RE2_BAZELRC)).checksum(),
        RE2_TRIMMED
    );
}

#[test]
fn re2_exec_checksum() {
    let top = top_level(&RE2_BAZELRC);
    let trimmed = test_trimming_trim(&top);
    assert_eq!(exec_of(&top, &trimmed).checksum(), EXEC);
}

#[test]
fn empty_options_checksum() {
    assert_eq!(EMPTY_OPTIONS.checksum(), EMPTY);
}
