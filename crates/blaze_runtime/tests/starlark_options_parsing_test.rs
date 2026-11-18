//! Port of `src/test/java/com/google/devtools/build/lib/starlark/StarlarkOptionsParsingTest.java`
//! (the tests of `runtime/StarlarkOptionsParser.java`).
//!
//! The Java tests extend `StarlarkOptionsTestCase` (a `BuildViewTestCase`): they write BUILD and
//! .bzl files into a scratch workspace and load the build settings through a real
//! `SkyframeExecutor`. The tests that only need a `BuildSettingLoader` that fails run here against
//! a fake loader; the others need loaded `Target`s and are ignored. The same behaviours are
//! compared end to end with the oracle (Bazel 9.3.2) by `scripts/w4/flags-probe` (transcript:
//! `crates/blaze_sky_top/tests/data/flags_probe.oracle.txt`); each ignore reason names the case.

use blaze_cmdline::{TargetParsingException, TargetPatternsCode};
use blaze_options::OptionsParser;
use blaze_packages::target::Target;
use blaze_runtime::starlark_options_parser::{
    BuildSettingLoadError, BuildSettingLoader, StarlarkOptionsParser, StarlarkOptionsParsingError,
};

/// A loader for a workspace without build settings: every flag name fails to load, like the
/// Java tests' `scratch.file("test/BUILD")` workspace does for `//fake_flag`.
struct NoSuchTargetLoader;

impl BuildSettingLoader for NoSuchTargetLoader {
    fn load_build_setting(&self, name: &str) -> Result<Option<Target>, BuildSettingLoadError> {
        Err(BuildSettingLoadError::TargetParsing(
            TargetParsingException::new(
                format!("no such target '{}'", name),
                TargetPatternsCode::TargetPatternParseFailure,
            ),
        ))
    }
}

/// `StarlarkOptionsTestCase.parseStarlarkOptions(options, onlyStarlarkParser = true)`.
fn parse_starlark_options(options: &str) -> Result<bool, StarlarkOptionsParsingError> {
    let as_list: Vec<String> = options.split(' ').map(str::to_string).collect();
    let mut options_parser = OptionsParser::builder().build().unwrap();
    let loader = NoSuchTargetLoader;
    let mut starlark_options_parser = StarlarkOptionsParser::builder()
        .build_setting_loader(&loader)
        .native_options_parser(&mut options_parser)
        .build();
    starlark_options_parser.parse_given_args(&as_list)
}

fn options_parsing_exception(
    result: Result<bool, StarlarkOptionsParsingError>,
) -> blaze_options::OptionsParsingException {
    match result {
        Err(StarlarkOptionsParsingError::OptionsParsing(e)) => e,
        other => panic!("expected an OptionsParsingException, got {:?}", other),
    }
}

// test --fake_flag=value
#[test]
fn test_bad_flag_equals_form() {
    let e = options_parsing_exception(parse_starlark_options("--//fake_flag=blahblahblah"));

    assert!(
        e.get_message().contains("Error loading option //fake_flag"),
        "{}",
        e.get_message()
    );
    assert_eq!(e.get_invalid_argument(), Some("//fake_flag"));
}

// test --fake_flag
#[test]
fn test_bad_flag_bool_form() {
    let e = options_parsing_exception(parse_starlark_options("--//fake_flag"));

    assert!(
        e.get_message().contains("Error loading option //fake_flag"),
        "{}",
        e.get_message()
    );
    assert_eq!(e.get_invalid_argument(), Some("//fake_flag"));
}

#[test]
fn test_single_dash_not_allowed() {
    let e = options_parsing_exception(parse_starlark_options("-//test:my_int_setting=666"));
    assert_eq!(
        e.get_message(),
        "Invalid options syntax: -//test:my_int_setting=666"
    );
}

// The message of a failed load, as the oracle prints it (flags-probe case 35):
// "ERROR: //:nonexistent :: Error loading option //:nonexistent: <loader's message>".
#[test]
fn error_loading_option_carries_the_loaders_message() {
    let e = options_parsing_exception(parse_starlark_options("--//:nonexistent=1"));
    assert_eq!(
        e.get_message(),
        "Error loading option //:nonexistent: no such target '//:nonexistent'"
    );
    assert_eq!(e.get_invalid_argument(), Some("//:nonexistent"));
}

/// The tests that need `StarlarkOptionsTestCase` (a `BuildViewTestCase`: scratch packages with
/// build settings loaded through a `SkyframeExecutor`). Each reason names the oracle-compared
/// end-to-end case that covers the behaviour.
macro_rules! needs_build_view_test_case {
    ($($name:ident: $reason:literal;)*) => {
        $(
            #[test]
            #[ignore = $reason]
            fn $name() {}
        )*
    };
}

needs_build_view_test_case! {
    test_flag_equals_value_form: "needs StarlarkOptionsTestCase (BuildViewTestCase); end to end: scripts/w4/flags-probe case 6";
    test_flag_name_with_external_repo: "needs StarlarkOptionsTestCase (BuildViewTestCase); end to end: scripts/w4/flags-probe case 31 (@//, @flags_probe//)";
    test_bad_flag_keep_going: "needs StarlarkOptionsTestCase (BuildViewTestCase); end to end: needs --keep_going on the native parser; flags-probe case 35";
    test_non_flag_parsing: "needs StarlarkOptionsTestCase (BuildViewTestCase); end to end: scripts/w4/flags-probe case 41";
    test_boolean_flag: "needs StarlarkOptionsTestCase (BuildViewTestCase); end to end: scripts/w4/flags-probe cases 2, 4 and 5";
    test_no_prefixed_boolean_flag: "needs StarlarkOptionsTestCase (BuildViewTestCase); end to end: scripts/w4/flags-probe case 3";
    test_no_prefixed_boolean_flag_with_workspace: "needs StarlarkOptionsTestCase (BuildViewTestCase); end to end: scripts/w4/flags-probe case 3 (no WORKSPACE repositories in Bazel 9 corpus)";
    test_no_prefixed_non_boolean_flag: "needs StarlarkOptionsTestCase (BuildViewTestCase); end to end: scripts/w4/flags-probe case 39";
    test_flag_without_value: "needs StarlarkOptionsTestCase (BuildViewTestCase); end to end: scripts/w4/flags-probe cases 38 and 46";
    test_repeat_flag_last_one_wins: "needs StarlarkOptionsTestCase (BuildViewTestCase); end to end: scripts/w4/flags-probe cases 23, 60, 61 and 63";
    test_multiple_flags: "needs StarlarkOptionsTestCase (BuildViewTestCase); end to end: scripts/w4/flags-probe case 32";
    test_non_build_setting: "needs StarlarkOptionsTestCase (BuildViewTestCase); end to end: scripts/w4/flags-probe case 42";
    test_non_rule_configured_target: "needs StarlarkOptionsTestCase (BuildViewTestCase); end to end: scripts/w4/flags-probe case 45";
    test_wrong_value_type_int: "needs StarlarkOptionsTestCase (BuildViewTestCase); end to end: scripts/w4/flags-probe case 37";
    test_wrong_value_type_bool: "needs StarlarkOptionsTestCase (BuildViewTestCase); end to end: scripts/w4/flags-probe case 40";
    test_dont_store_default_value: "needs StarlarkOptionsTestCase (BuildViewTestCase); end to end: scripts/w4/flags-probe cases 7, 12, 15, 17 and 19";
    test_options_are_parsed_with_build_tests_only: "needs StarlarkOptionsTestCase (BuildViewTestCase); end to end: not covered (needs --build_tests_only)";
    test_expected_build_event_output_as_flag: "needs StarlarkOptionsTestCase (BuildViewTestCase); end to end: not covered (needs the build event protocol stream)";
    test_expected_build_event_output_as_target: "needs StarlarkOptionsTestCase (BuildViewTestCase); end to end: not covered (needs the build event protocol stream)";
    test_allow_multiple_string_flag: "needs StarlarkOptionsTestCase (BuildViewTestCase); end to end: scripts/w4/flags-probe cases 14, 62 and 69";
    test_repeated_string_list_flag: "needs StarlarkOptionsTestCase (BuildViewTestCase); end to end: scripts/w4/flags-probe cases 16 and 68";
    flag_references_exactly_one_target: "needs StarlarkOptionsTestCase (BuildViewTestCase); end to end: scripts/w4/flags-probe case 49";
    flag_is_alias: "needs StarlarkOptionsTestCase (BuildViewTestCase); end to end: scripts/w4/flags-probe cases 21 and 22";
    flag_is_alias_cycle: "needs StarlarkOptionsTestCase (BuildViewTestCase); end to end: scripts/w4/flags-probe case 50";
    flag_is_alias_uses_select: "needs StarlarkOptionsTestCase (BuildViewTestCase); end to end: scripts/w4/flags-probe cases 51 and 44 (an unconditional select is simplified at loading)";
    flag_is_alias_resolves_to_non_build_setting_target: "needs StarlarkOptionsTestCase (BuildViewTestCase); end to end: scripts/w4/flags-probe case 43";
}
