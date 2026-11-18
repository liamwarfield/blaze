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

// -------------------------------------------------------------------------------------------
// Starlark (user-defined) flags. Expected checksums: the oracle's transcripts
// `crates/blaze_sky_top/tests/data/flags_probe.oracle.txt` (scripts/w4/flags-probe: full
// checksums of the configuration of `//:t`, short ids of the exec configuration of `//:tool`)
// and `scripts/w4/scenarios/flags-transitions/ORACLE.txt` (short ids). The options are built
// the way the oracle does: the value objects of `CoreOptionConverters.BUILD_SETTING_CONVERTERS`
// for command-line flags, the raw Starlark values for flags set by a transition, and the scope
// type of each flag.

mod starlark_flags {
    use super::*;
    use blaze_build_config::build_options::{ScopeType, StarlarkOptionValue};

    fn s(v: &str) -> StarlarkOptionValue {
        StarlarkOptionValue::Str(v.to_string())
    }

    fn strings(v: &[&str]) -> Vec<StarlarkOptionValue> {
        v.iter().map(|e| s(e)).collect()
    }

    fn with_flags(
        base: &BuildOptions,
        flags: &[(&str, StarlarkOptionValue, ScopeType)],
    ) -> BuildOptions {
        let mut builder = base.to_builder();
        for (label, value, scope_type) in flags {
            let label = Label::parse_canonical_unchecked(label);
            builder.add_starlark_option(label.clone(), value.clone());
            builder.add_scope_type(label, *scope_type);
        }
        builder.build()
    }

    /// The top-level options of the flags-probe workspace: its `.bazelrc` adds
    /// `common --flag_alias=myint=//:int_flag` (parsed after the MODULE.bazel flag aliases).
    fn probe_top_level(flags: &[(&str, StarlarkOptionValue, ScopeType)]) -> BuildOptions {
        with_flags(&top_level(&["--flag_alias=myint=//:int_flag"]), flags)
    }

    /// The configuration of `//:t` in the flags-probe workspace.
    fn probe_target_checksum(flags: &[(&str, StarlarkOptionValue, ScopeType)]) -> String {
        test_trimming_trim(&probe_top_level(flags))
            .checksum()
            .to_string()
    }

    /// The exec configuration of `//:tool`: the exec transition of the configuration of `//:t`,
    /// then the scope types `BuildOptionsScopeFunction` resolves for the propagated flags.
    fn probe_exec_short_id(
        flags: &[(&str, StarlarkOptionValue, ScopeType)],
        propagated: &[(&str, ScopeType)],
    ) -> String {
        let top = probe_top_level(flags);
        let trimmed = test_trimming_trim(&top);
        let exec = exec_of(&top, &trimmed);
        let mut builder = exec.to_builder();
        for (label, scope_type) in propagated {
            let label = Label::parse_canonical_unchecked(label);
            assert!(
                exec.get_starlark_options().contains_key(&label),
                "{} was not propagated to the exec configuration",
                label
            );
            builder.add_scope_type(label, *scope_type);
        }
        assert_eq!(exec.get_starlark_options().len(), propagated.len());
        builder.build().short_id()
    }

    /// A configuration of the flags-transitions workspace (no `.bazelrc`): the test-trimmed
    /// top-level options plus the given flags.
    fn transitions_short_id(flags: &[(&str, StarlarkOptionValue, ScopeType)]) -> String {
        with_flags(&test_trimming_trim(&top_level(&[])), flags).short_id()
    }

    const D: ScopeType = ScopeType::DEFAULT;

    // flags-probe case 1.
    #[test]
    fn probe_default_configuration() {
        assert_eq!(
            probe_target_checksum(&[]),
            "f074357488e956df5806734c1d16db0828ab19db9dc6ca0404f73315df7f6a3d"
        );
        assert_eq!(probe_exec_short_id(&[], &[]), "5f0adec");
    }

    // flags-probe cases 3, 6, 58, 9, 10, 59: Boolean, StarlarkInt and String values.
    #[test]
    fn probe_scalar_flags() {
        assert_eq!(
            probe_target_checksum(&[("//:bool_flag", StarlarkOptionValue::Bool(false), D)]),
            "f64112417a1c3ee845a20830bd398710908152a0c7a1a6394a4d906534af28e5"
        );
        assert_eq!(
            probe_target_checksum(&[("//:int_flag", StarlarkOptionValue::Int(7), D)]),
            "76d0183f1a3c5ae583d3b517937bf303fbf19396c4a72d081c245babbe572823"
        );
        assert_eq!(
            probe_target_checksum(&[("//:int_flag", StarlarkOptionValue::Int(-16), D)]),
            "819a633895fe2081579390abd046d1d2f45ce42f710675709a4ba6f52f9a1f8e"
        );
        assert_eq!(
            probe_target_checksum(&[("//:string_flag", s("abc"), D)]),
            "d3b3295ab3326bed0f23252716b0745940bd2fb347947968931def15fef8b8c4"
        );
        assert_eq!(
            probe_target_checksum(&[("//:string_flag", s(""), D)]),
            "caf2bd0974a0508524617e7f49f92e7df8780b6846e1be6a95e7d71582c6dcb9"
        );
        assert_eq!(
            probe_target_checksum(&[("//:string_flag", s("a=b"), D)]),
            "d5773a2d2b10104b42c3becace9ed6873063f996af6bb915a9a43cf8ed7bf015"
        );
    }

    // flags-probe cases 11, 13, 70 (ImmutableList of a string_list flag; the empty list is
    // "EMPTY" in the cache key), 14 and 69 (ArrayList of an allow_multiple flag), 16 and 68
    // (ArrayList of a repeatable flag).
    #[test]
    fn probe_list_flags() {
        let list = |v: &[&str]| StarlarkOptionValue::list(strings(v));
        assert_eq!(
            probe_target_checksum(&[("//:string_list_flag", list(&["a", "b"]), D)]),
            "41a8348d91a42daad1996671a70030dd88e7e719daac5df4e99df70e759b47d8"
        );
        assert_eq!(
            probe_target_checksum(&[("//:string_list_flag", list(&[]), D)]),
            "c21f4b02fc968e0b218433b9c1c7515b2c49aaeba5350b1d1b8001b8736dd871"
        );
        assert_eq!(
            probe_target_checksum(&[("//:string_list_flag", list(&["a", "", "b"]), D)]),
            "95544d54c67f472baa530ec93e3f95d19c9a8a3cb925a1831471ca89f4a826b6"
        );
        assert_eq!(
            probe_target_checksum(&[("//:multi", list(&["a", "b"]), D)]),
            "dde918d13da388fc365d87642b49bde40872ec944f2129e03d85b2321a2d1a40"
        );
        assert_eq!(
            probe_target_checksum(&[("//:multi", list(&["a,b"]), D)]),
            "47ebdc35afdef0aed141be473a25d242d926b039acd979637eaab83fc9d6281b"
        );
        assert_eq!(
            probe_target_checksum(&[("//:rep", list(&["a", "b,c"]), D)]),
            "f867312317df5a99213da7d3fd47d16da8e0ae2d9a7b5e3a3b10290e698731e7"
        );
        assert_eq!(
            probe_target_checksum(&[("//:rep", list(&["a,b"]), D)]),
            "367c5b78abbec0e5d41fa685d2007ab0ddbe8daf4bf414244a6d28faee77f1d7"
        );
    }

    // flags-probe cases 52, 54 (ImmutableSet of a string_set flag; an empty set is not "EMPTY")
    // and 55 (LinkedHashSet of a repeatable string_set flag).
    #[test]
    fn probe_set_flags() {
        let set = |v: &[&str]| StarlarkOptionValue::set(strings(v));
        assert_eq!(
            probe_target_checksum(&[("//:sset", set(&["b", "a", "b"]), D)]),
            "dec73819ff5a41f0c3ee90163b25394e040a5ca85c7ca48ee68892a388ec7119"
        );
        assert_eq!(
            probe_target_checksum(&[("//:sset", set(&[]), D)]),
            "aeb0e4adca84601bd62cfc8a0a99aa5094ac145b67b889b3cf8957b014301300"
        );
        assert_eq!(
            probe_target_checksum(&[("//:rsset", set(&["a", "b", "a"]), D)]),
            "ba7fafb3acd16fd8cced6fb2ae70c8e9bcfbaf2df2984a59c351a3b6489e1c86"
        );
    }

    // flags-probe case 18: a Label value.
    #[test]
    fn probe_label_flag() {
        assert_eq!(
            probe_target_checksum(&[(
                "//:lf",
                StarlarkOptionValue::Label(Label::parse_canonical_unchecked("//:lf_other")),
                D
            )]),
            "a5d95eb87fe8b29ec88ef9c92d8e248ea099a6bd2b14a3a4f9f11d46531f741e"
        );
    }

    // flags-probe cases 28, 29 and 32: the scope types are part of the checksum.
    #[test]
    fn probe_scoped_flags() {
        assert_eq!(
            probe_target_checksum(&[("//:universal_flag", s("uu"), ScopeType::UNIVERSAL)]),
            "64c5c8088fbd5ac8c457eef10e0d1f85b7b1f4de9c858dc45050532654f5fcd7"
        );
        assert_eq!(
            probe_target_checksum(&[("//:target_flag", s("tt"), ScopeType::TARGET)]),
            "563974032bb1a41089dbafdbe341945757794807c47efcdd5bf2fe407ffb5006"
        );
        assert_eq!(
            probe_target_checksum(&[
                ("//:int_flag", StarlarkOptionValue::Int(7), D),
                ("//:string_flag", s("abc"), D),
                (
                    "//:string_list_flag",
                    StarlarkOptionValue::list(strings(&["b"])),
                    D
                ),
                ("//:bool_flag", StarlarkOptionValue::Bool(false), D),
            ]),
            "97ddbf839b785fe94240c99420d61a794231ee637291cb8afc9e4728723421dc"
        );
    }

    // flags-probe cases 9, 28, 29 and 30: which flags the exec transition propagates.
    #[test]
    fn probe_exec_configurations() {
        assert_eq!(
            probe_exec_short_id(&[("//:string_flag", s("abc"), D)], &[("//:string_flag", D)]),
            "4fbd23c"
        );
        assert_eq!(
            probe_exec_short_id(
                &[("//:universal_flag", s("uu"), ScopeType::UNIVERSAL)],
                &[("//:universal_flag", ScopeType::UNIVERSAL)]
            ),
            "755119a"
        );
        // scope = "target": not propagated.
        assert_eq!(
            probe_exec_short_id(&[("//:target_flag", s("tt"), ScopeType::TARGET)], &[]),
            "5f0adec"
        );
        assert_eq!(
            probe_exec_short_id(
                &[
                    ("//:universal_flag", s("uu"), ScopeType::UNIVERSAL),
                    ("//:target_flag", s("tt"), ScopeType::TARGET),
                    ("//:string_flag", s("abc"), D),
                ],
                &[
                    ("//:universal_flag", ScopeType::UNIVERSAL),
                    ("//:string_flag", D)
                ]
            ),
            "8ecff6a"
        );
    }

    // flags-transitions steps 1, 2, 3 and 5: the same elements hash differently as a
    // StarlarkList (set by a transition: its toString is the Starlark repr), an ImmutableList
    // (command line) and a Tuple; an empty StarlarkList is a List, so it is "EMPTY".
    #[test]
    fn transition_list_values_keep_their_java_class() {
        let elems = strings(&["-DX", "-DY"]);
        assert_eq!(
            transitions_short_id(&[(
                "//:copts",
                StarlarkOptionValue::starlark_list(elems.clone()),
                D
            )]),
            "7223fb8"
        );
        assert_eq!(
            transitions_short_id(&[("//:copts", StarlarkOptionValue::list(elems.clone()), D)]),
            "c3adc31"
        );
        assert_eq!(
            transitions_short_id(&[("//:copts", StarlarkOptionValue::tuple(elems), D)]),
            "56746da"
        );
        assert_eq!(
            transitions_short_id(&[(
                "//:copts",
                StarlarkOptionValue::starlark_list(Vec::new()),
                D
            )]),
            "782d514"
        );
    }

    // flags-transitions steps 7 to 10: an allow_multiple flag. A transition that returns the
    // default leaves the `ImmutableList.of(default)` that was added for the transition to read
    // (step 7); over a command-line value it stores its StarlarkList (steps 8 and 9) unless the
    // lists are equal (step 10: the command line's ArrayList stays).
    #[test]
    fn transition_allow_multiple_values() {
        assert_eq!(
            transitions_short_id(&[("//:multi", StarlarkOptionValue::list(strings(&["m0"])), D)]),
            "9e6eb09"
        );
        assert_eq!(
            transitions_short_id(&[(
                "//:multi",
                StarlarkOptionValue::starlark_list(strings(&["m0"])),
                D
            )]),
            "1108066"
        );
        assert_eq!(
            transitions_short_id(&[(
                "//:multi",
                StarlarkOptionValue::starlark_list(strings(&["a", "b"])),
                D
            )]),
            "49c071d"
        );
        assert_eq!(
            transitions_short_id(&[(
                "//:multi",
                StarlarkOptionValue::list(strings(&["a", "b"])),
                D
            )]),
            "c483b1d"
        );
    }

    // flags-transitions steps 12, 20, 22, 27 and 31: scalars (a bool flag set to the int 0 by a
    // transition keeps the StarlarkInt).
    #[test]
    fn transition_scalar_values() {
        assert_eq!(
            transitions_short_id(&[("//:level", StarlarkOptionValue::Int(2), D)]),
            "a5761cf"
        );
        assert_eq!(
            transitions_short_id(&[("//:enabled", StarlarkOptionValue::Int(0), D)]),
            "b3dfdbb"
        );
        assert_eq!(
            transitions_short_id(&[("//:mode", s("fast"), D)]),
            "e8ec9a7"
        );
        assert_eq!(
            transitions_short_id(&[("//:level", StarlarkOptionValue::Int(16), D)]),
            "d0e84ef"
        );
        assert_eq!(
            transitions_short_id(&[("//:enabled", StarlarkOptionValue::Bool(false), D)]),
            "702c5eb"
        );
    }
}
