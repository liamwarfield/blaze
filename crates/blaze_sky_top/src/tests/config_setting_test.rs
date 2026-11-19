//! Port of `src/test/java/com/google/devtools/build/lib/rules/config/ConfigSettingTest.java` (a
//! `BuildViewTestCase`): `config_setting`.
//!
//! Java's test class builds its own rule class provider (`createRuleClassProvider`):
//! `TestRuleClassProvider.addStandardRules` plus the test rule `feature_flag_setter`
//! (`FeatureFlagSetterRule`) and the configuration fragment `DummyTestOptionsFragment`, which
//! requires the options class `DummyTestOptions` (`--internal_option`, `--allow_multiple_option`,
//! `--new_option_name` with old name `--old_option_name`, `--non_configurable_option`). The
//! harness analyzes with Bazel's rule class provider and cannot register either, so the tests that
//! use them are ignored.
//!
//! Java runs the parameterized tests (`@TestParameters` / `@TestParameter`) once per value; each
//! value is one Rust test here, named after the Java method with the value as a suffix.

use std::collections::BTreeSet;

use blaze_analysis::config::config_matching_provider::{ConfigMatchingProvider, MatchResult};
use blaze_cmdline::Label;
use blaze_cmdline::repository_name::RepositoryName;
use blaze_packages::attribute::AttributeMap;
use blaze_packages::attribute::raw_attribute_mapper::RawAttributeMapper;
use blaze_packages::target::Target;
use blaze_packages::types::LicenseType;
use blaze_packages::types::type_::Type;
use blaze_rules::config::config_setting;

use crate::testing::{BuildViewTestCase, assert_contains_exactly};

// Java: ConfigSettingTest.writeSimpleExample
fn write_simple_example(t: &BuildViewTestCase) {
    t.scratch.file(
        "pkg/BUILD",
        &[r#"config_setting(
    name = "foo",
    values = {
        "compilation_mode": "dbg",
        "stamp": "1",
    },
)
"#],
    );
}

// Java: ConfigSettingTest.getConfigMatchingProvider
fn get_config_matching_provider(t: &mut BuildViewTestCase, label: &str) -> ConfigMatchingProvider {
    let ct = t
        .get_configured_target(label)
        .unwrap_or_else(|| panic!("NullPointerException: no configured target {label}"));
    ct.get_provider::<ConfigMatchingProvider>()
        .unwrap_or_else(|| panic!("NullPointerException: {label} has no ConfigMatchingProvider"))
        .clone()
}

// Java: ConfigSettingTest.forceConvertMatchResult
fn force_convert_match_result(result: &MatchResult) -> bool {
    match result {
        MatchResult::Match => true,
        MatchResult::NoMatch(_) => false,
        other => panic!("IllegalStateException: Unexpected MatchResult: {other:?}"),
    }
}

// Java: ConfigSettingTest.getConfigMatchingProviderResultAsBoolean
fn get_config_matching_provider_result_as_boolean(t: &mut BuildViewTestCase, label: &str) -> bool {
    force_convert_match_result(get_config_matching_provider(t, label).result())
}

/// Checks the behavior of `ConfigSetting.isUnderToolsPackage`.
// Java: ConfigSettingTest.isUnderToolsPackage
#[test]
fn is_under_tools_package() {
    let tools_repo = RepositoryName::create("tools").unwrap();
    // Subpackage of the tools package.
    assert!(config_setting::is_under_tools_package(
        &Label::parse_canonical_unchecked("@tools//tools/subpkg:foo"),
        &tools_repo
    ));
    // The tools package itself.
    assert!(config_setting::is_under_tools_package(
        &Label::parse_canonical_unchecked("@tools//tools:foo"),
        &tools_repo
    ));
    // The tools repo, but wrong package.
    assert!(!config_setting::is_under_tools_package(
        &Label::parse_canonical_unchecked("@tools//nottools:foo"),
        &tools_repo
    ));
    // Not even the tools repo.
    assert!(!config_setting::is_under_tools_package(
        &Label::parse_canonical_unchecked("@nottools//nottools:foo"),
        &tools_repo
    ));
    // A tools package but in the wrong repo.
    assert!(!config_setting::is_under_tools_package(
        &Label::parse_canonical_unchecked("@nottools//tools:foo"),
        &tools_repo
    ));
}

/// Tests that a config_setting only matches build configurations where *all* of its flag
/// specifications match.
// Java: ConfigSettingTest.matchingCriteria
#[test]
fn matching_criteria() {
    let mut t = BuildViewTestCase::new();
    write_simple_example(&t);

    // First flag mismatches:
    t.use_configuration(&["-c", "opt", "--stamp"]);
    assert!(!get_config_matching_provider_result_as_boolean(
        &mut t,
        "//pkg:foo"
    ));

    // Second flag mismatches:
    t.use_configuration(&["-c", "dbg", "--nostamp"]);
    assert!(!get_config_matching_provider_result_as_boolean(
        &mut t,
        "//pkg:foo"
    ));

    // Both flags mismatch:
    t.use_configuration(&["-c", "opt", "--nostamp"]);
    assert!(!get_config_matching_provider_result_as_boolean(
        &mut t,
        "//pkg:foo"
    ));

    // Both flags match:
    t.use_configuration(&["-c", "dbg", "--stamp"]);
    assert!(get_config_matching_provider_result_as_boolean(
        &mut t,
        "//pkg:foo"
    ));
}

/// Tests that `ConfigMatchingProvider.label` is correct.
// Java: ConfigSettingTest.labelGetter
#[test]
fn label_getter() {
    let mut t = BuildViewTestCase::new();
    write_simple_example(&t);
    assert_eq!(
        get_config_matching_provider(&mut t, "//pkg:foo").label(),
        &Label::parse_canonical("//pkg:foo").unwrap()
    );
}

/// Tests that rule analysis fails on unknown options.
// Java: ConfigSettingTest.unknownOption
#[test]
fn unknown_option() {
    let mut t = BuildViewTestCase::new();
    t.check_error(
        "foo",
        "badoption",
        "unknown option: 'not_an_option'",
        &[
            "config_setting(",
            "    name = 'badoption',",
            "    values = {'not_an_option': 'bar'})",
        ],
    );
}

/// Tests that rule analysis fails on internal options.
// Java: ConfigSettingTest.internalOption
#[test]
#[ignore = "needs DummyTestOptionsFragment (the test's custom rule class provider registering DummyTestOptions' --internal_option)"]
fn internal_option() {
    let mut t = BuildViewTestCase::new();
    t.check_error(
        "foo",
        "badoption",
        "unknown option: 'internal_option'",
        &[
            "config_setting(",
            "    name = 'badoption',",
            "    values = {'internal_option': 'bar'})",
        ],
    );
}

// Java: ConfigSettingTest.oldNameReference
#[test]
#[ignore = "needs DummyTestOptionsFragment (the test's custom rule class provider registering DummyTestOptions' --new_option_name/--old_option_name)"]
fn old_name_reference() {
    let mut t = BuildViewTestCase::new();
    t.scratch.file(
        "test/BUILD",
        &[r#"config_setting(
    name = "match",
    values = {"old_option_name": "foo"},
)
"#],
    );
    assert!(t.get_configured_target("//test:match").is_some());
    t.assert_no_events();
}

/// Tests that rule analysis fails on invalid option values.
// Java: ConfigSettingTest.invalidOptionValue
#[test]
fn invalid_option_value() {
    let mut t = BuildViewTestCase::new();
    t.check_error(
        "foo",
        "badvalue",
        "Not a valid compilation mode: 'baz'",
        &[
            "config_setting(",
            "    name = 'badvalue',",
            "    values = {'compilation_mode': 'baz'})",
        ],
    );
}

/// Tests that when the first option is valid but the config_setting doesn't match, remaining
/// options are still validity-checked.
// Java: ConfigSettingTest.invalidOptionFartherDown
#[test]
fn invalid_option_farther_down() {
    let mut t = BuildViewTestCase::new();
    t.check_error(
        "foo",
        "badoption",
        "unknown option: 'not_an_option'",
        &[
            "config_setting(",
            "    name = 'badoption',",
            "    values = {",
            "        'compilation_mode': 'opt',",
            "        'not_an_option': 'bar',",
            "    })",
        ],
    );
}

/// Tests that None is not specifiable for a key's value.
// Java: ConfigSettingTest.noneValueInSetting
#[test]
fn none_value_in_setting() {
    let mut t = BuildViewTestCase::new();
    let expected = t.ws("ERROR /workspace/foo/BUILD:1:15: //foo:none: \
         expected value of type 'string' for dict value element, but got None (NoneType)");
    t.check_error(
        "foo",
        "none",
        &expected,
        &[
            "config_setting(",
            "    name = 'none',",
            "    values = {\"none_value\": None})",
        ],
    );
}

/// Tests that *some* settings (values or flag_values) must be specified.
// Java: ConfigSettingTest.emptySettings
#[test]
fn empty_settings() {
    let mut t = BuildViewTestCase::new();
    t.check_error(
        "foo",
        "empty",
        "in config_setting rule //foo:empty: \
         Either values, flag_values or constraint_values must be specified and non-empty",
        &["config_setting(", "    name = 'empty',", "    values = {})"],
    );
}

/// Tests matching on multi-value attributes with key=value entries (e.g. --define).
// Java: ConfigSettingTest.multiValueDict
#[test]
fn multi_value_dict() {
    let mut t = BuildViewTestCase::new();
    t.scratch.file(
        "test/BUILD",
        &[r#"config_setting(
    name = "match",
    values = {
        "define": "foo=bar",
    },
)
"#],
    );

    t.use_configuration(&[""]);
    assert!(!get_config_matching_provider_result_as_boolean(
        &mut t,
        "//test:match"
    ));
    t.use_configuration(&["--define", "foo=bar"]);
    assert!(get_config_matching_provider_result_as_boolean(
        &mut t,
        "//test:match"
    ));
    t.use_configuration(&["--define", "foo=baz"]);
    assert!(!get_config_matching_provider_result_as_boolean(
        &mut t,
        "//test:match"
    ));
    t.use_configuration(&["--define", "foo=bar", "--define", "bar=baz"]);
    assert!(get_config_matching_provider_result_as_boolean(
        &mut t,
        "//test:match"
    ));
    t.use_configuration(&[
        "--define", "foo=bar", "--define", "bar=baz", "--define", "foo=nope",
    ]);
    assert!(!get_config_matching_provider_result_as_boolean(
        &mut t,
        "//test:match"
    ));
    t.use_configuration(&[
        "--define", "foo=nope", "--define", "bar=baz", "--define", "foo=bar",
    ]);
    assert!(get_config_matching_provider_result_as_boolean(
        &mut t,
        "//test:match"
    ));
}

// Java: ConfigSettingTest.invalidDefineProducesError
#[test]
fn invalid_define_produces_error() {
    let mut t = BuildViewTestCase::new();
    t.scratch.file(
        "test/BUILD",
        &[r#"config_setting(
    name = "match",
    values = {
        "define": "foo",  # Value should be "foo=<something>".
    },
)
"#],
    );

    t.check_error_for_label(
        "//test:match",
        "Variable definitions must be in the form of a 'name=value' assignment",
    );
}

// Java: ConfigSettingTest.multipleDefines
#[test]
fn multiple_defines() {
    let mut t = BuildViewTestCase::new();
    t.scratch.file(
        "test/BUILD",
        &[r#"config_setting(
    name = "match",
    define_values = {
        "foo1": "bar",
        "foo2": "baz",
    },
)
"#],
    );

    t.use_configuration(&[""]);
    assert!(!get_config_matching_provider_result_as_boolean(
        &mut t,
        "//test:match"
    ));
    t.use_configuration(&["--define", "foo1=bar"]);
    assert!(!get_config_matching_provider_result_as_boolean(
        &mut t,
        "//test:match"
    ));
    t.use_configuration(&["--define", "foo2=baz"]);
    assert!(!get_config_matching_provider_result_as_boolean(
        &mut t,
        "//test:match"
    ));
    t.use_configuration(&["--define", "foo1=bar", "--define", "foo2=baz"]);
    assert!(get_config_matching_provider_result_as_boolean(
        &mut t,
        "//test:match"
    ));
}

/// Tests that for a multi-value dictionary, `values = { 'key': 'value' }` always refers to a
/// single map entry. Fancy syntax like `values = { 'key': 'value=1,key2=value2' }` doesn't get
/// around that.
// Java: ConfigSettingTest.multiValueDictSettingAlwaysSingleEntry
#[test]
fn multi_value_dict_setting_always_single_entry() {
    let mut t = BuildViewTestCase::new();
    t.scratch.file(
        "test/BUILD",
        &[r#"config_setting(
    name = "match",
    values = {
        "define": "foo=bar,baz=bat",
    },
)
"#],
    );

    t.use_configuration(&[""]);
    assert!(!get_config_matching_provider_result_as_boolean(
        &mut t,
        "//test:match"
    ));
    t.use_configuration(&["--define", "foo=bar"]);
    assert!(!get_config_matching_provider_result_as_boolean(
        &mut t,
        "//test:match"
    ));
    t.use_configuration(&["--define", "foo=bar", "--define", "baz=bat"]);
    assert!(!get_config_matching_provider_result_as_boolean(
        &mut t,
        "//test:match"
    ));
    t.use_configuration(&["--define", "foo=bar,baz=bat"]);
    assert!(get_config_matching_provider_result_as_boolean(
        &mut t,
        "//test:match"
    ));
    t.use_configuration(&[
        "--define",
        "makethis=a_superset",
        "--define",
        "foo=bar,baz=bat",
    ]);
    assert!(get_config_matching_provider_result_as_boolean(
        &mut t,
        "//test:match"
    ));
}

// Java: ConfigSettingTest.definesCrossAttributes
#[test]
fn defines_cross_attributes() {
    let mut t = BuildViewTestCase::new();
    t.scratch.file(
        "test/BUILD",
        &[r#"config_setting(
    name = "match",
    define_values = {
        "b": "d",
    },
    values = {
        "define": "a=c",
    },
)
"#],
    );

    t.use_configuration(&[""]);
    assert!(!get_config_matching_provider_result_as_boolean(
        &mut t,
        "//test:match"
    ));
    t.use_configuration(&["--define", "a=c"]);
    assert!(!get_config_matching_provider_result_as_boolean(
        &mut t,
        "//test:match"
    ));
    t.use_configuration(&["--define", "b=d"]);
    assert!(!get_config_matching_provider_result_as_boolean(
        &mut t,
        "//test:match"
    ));
    t.use_configuration(&["--define", "a=c", "--define", "b=d"]);
    assert!(get_config_matching_provider_result_as_boolean(
        &mut t,
        "//test:match"
    ));
}

/// Tests matching on multi-value attributes against single expected values: the actual list must
/// contain the expected value.
// Java: ConfigSettingTest.multiValueListSingleExpectedValue
#[test]
fn multi_value_list_single_expected_value() {
    let mut t = BuildViewTestCase::new();
    t.scratch.file(
        "test/BUILD",
        &[r#"config_setting(
    name = "match",
    values = {
        "copt": "-Dfoo",
    },
)
"#],
    );

    t.use_configuration(&[""]);
    assert!(!get_config_matching_provider_result_as_boolean(
        &mut t,
        "//test:match"
    ));
    t.use_configuration(&["--copt", "-Dfoo"]);
    assert!(get_config_matching_provider_result_as_boolean(
        &mut t,
        "//test:match"
    ));
    t.use_configuration(&["--copt", "-Dbar"]);
    assert!(!get_config_matching_provider_result_as_boolean(
        &mut t,
        "//test:match"
    ));
    t.use_configuration(&["--copt", "-Dfoo", "--copt", "-Dbar"]);
    assert!(get_config_matching_provider_result_as_boolean(
        &mut t,
        "//test:match"
    ));
    t.use_configuration(&["--copt", "-Dbar", "--copt", "-Dfoo"]);
    assert!(get_config_matching_provider_result_as_boolean(
        &mut t,
        "//test:match"
    ));
}

/// Tests matching on multi-value flags against multiple expected values: the actual list must
/// contain all expected values (and possibly more).
///
/// This only works for flags that can parse multiple values in the same entry. Not all flags do
/// this: this varies according to each flag's definition. For example "--copt=a,b" produces a
/// single entry ["a,b"], while "--extra_platforms=a,b" produces ["a", "b"].
// Java: ConfigSettingTest.multiValueListMultipleExpectedValues
fn multi_value_list_multiple_expected_values(flags: &[&str], match_expected: bool) {
    let mut t = BuildViewTestCase::new();
    t.scratch.file(
        "test/BUILD",
        &[r#"config_setting(
    name = "match",
    values = {
        "allow_multiple_option": "one,two",  # This produces ["one", "two"]
    },
)
"#],
    );

    t.use_configuration(flags);
    assert_eq!(
        get_config_matching_provider_result_as_boolean(&mut t, "//test:match"),
        match_expected
    );
}

// Java: ConfigSettingTest.multiValueListMultipleExpectedValues {flags: [''], matchExpected: false}
#[test]
#[ignore = "needs DummyTestOptionsFragment (the test's custom rule class provider registering DummyTestOptions' --allow_multiple_option)"]
fn multi_value_list_multiple_expected_values_no_flag() {
    multi_value_list_multiple_expected_values(&[""], false);
}

// Java: ConfigSettingTest.multiValueListMultipleExpectedValues {flags: ['--allow_multiple_option', 'one'], matchExpected: false}
#[test]
#[ignore = "needs DummyTestOptionsFragment (the test's custom rule class provider registering DummyTestOptions' --allow_multiple_option)"]
fn multi_value_list_multiple_expected_values_one() {
    multi_value_list_multiple_expected_values(&["--allow_multiple_option", "one"], false);
}

// Java: ConfigSettingTest.multiValueListMultipleExpectedValues {flags: ['--allow_multiple_option', 'two'], matchExpected: false}
#[test]
#[ignore = "needs DummyTestOptionsFragment (the test's custom rule class provider registering DummyTestOptions' --allow_multiple_option)"]
fn multi_value_list_multiple_expected_values_two() {
    multi_value_list_multiple_expected_values(&["--allow_multiple_option", "two"], false);
}

// Java: ConfigSettingTest.multiValueListMultipleExpectedValues {flags: ['--allow_multiple_option', 'one', '--allow_multiple_option', 'two'], matchExpected: true}
#[test]
#[ignore = "needs DummyTestOptionsFragment (the test's custom rule class provider registering DummyTestOptions' --allow_multiple_option)"]
fn multi_value_list_multiple_expected_values_one_then_two() {
    multi_value_list_multiple_expected_values(
        &[
            "--allow_multiple_option",
            "one",
            "--allow_multiple_option",
            "two",
        ],
        true,
    );
}

// Java: ConfigSettingTest.multiValueListMultipleExpectedValues {flags: ['--allow_multiple_option', 'two', '--allow_multiple_option', 'one'], matchExpected: true}
#[test]
#[ignore = "needs DummyTestOptionsFragment (the test's custom rule class provider registering DummyTestOptions' --allow_multiple_option)"]
fn multi_value_list_multiple_expected_values_two_then_one() {
    multi_value_list_multiple_expected_values(
        &[
            "--allow_multiple_option",
            "two",
            "--allow_multiple_option",
            "one",
        ],
        true,
    );
}

// Java: ConfigSettingTest.multiValueListMultipleExpectedValues {flags: ['--allow_multiple_option', 'one,two'], matchExpected: true}
#[test]
#[ignore = "needs DummyTestOptionsFragment (the test's custom rule class provider registering DummyTestOptions' --allow_multiple_option)"]
fn multi_value_list_multiple_expected_values_one_comma_two() {
    multi_value_list_multiple_expected_values(&["--allow_multiple_option", "one,two"], true);
}

// Java: ConfigSettingTest.multiValueListMultipleExpectedValues {flags: ['--allow_multiple_option', 'two,one'], matchExpected: true}
#[test]
#[ignore = "needs DummyTestOptionsFragment (the test's custom rule class provider registering DummyTestOptions' --allow_multiple_option)"]
fn multi_value_list_multiple_expected_values_two_comma_one() {
    multi_value_list_multiple_expected_values(&["--allow_multiple_option", "two,one"], true);
}

// Java: ConfigSettingTest.multiValueListMultipleExpectedValues {flags: ['--allow_multiple_option', 'ten', '--allow_multiple_option', 'two', '--allow_multiple_option', 'three', '--allow_multiple_option', 'one'], matchExpected: true}
#[test]
#[ignore = "needs DummyTestOptionsFragment (the test's custom rule class provider registering DummyTestOptions' --allow_multiple_option)"]
fn multi_value_list_multiple_expected_values_superset() {
    multi_value_list_multiple_expected_values(
        &[
            "--allow_multiple_option",
            "ten",
            "--allow_multiple_option",
            "two",
            "--allow_multiple_option",
            "three",
            "--allow_multiple_option",
            "one",
        ],
        true,
    );
}

// Java: ConfigSettingTest.flagWithOldName_NoMatch
#[test]
#[ignore = "needs DummyTestOptionsFragment (the test's custom rule class provider registering DummyTestOptions' --new_option_name/--old_option_name)"]
fn flag_with_old_name_no_match() {
    let mut t = BuildViewTestCase::new();
    t.scratch.file(
        "test/BUILD",
        &[r#"config_setting(
    name = "match",
    values = {
        "old_option_name": "different_setting",
    },
)
"#],
    );
    t.use_configuration(&["--new_option_name=is_set"]);
    assert!(!get_config_matching_provider_result_as_boolean(
        &mut t,
        "//test:match"
    ));
}

// Java: ConfigSettingTest.flagWithOldName_Match
#[test]
#[ignore = "needs DummyTestOptionsFragment (the test's custom rule class provider registering DummyTestOptions' --new_option_name/--old_option_name)"]
fn flag_with_old_name_match() {
    let mut t = BuildViewTestCase::new();
    t.scratch.file(
        "test/BUILD",
        &[r#"config_setting(
    name = "match",
    values = {
        "old_option_name": "is_set",
    },
)
"#],
    );
    t.use_configuration(&["--new_option_name=is_set"]);
    assert!(get_config_matching_provider_result_as_boolean(
        &mut t,
        "//test:match"
    ));
}

/// Tests multi-value flags that don't support multiple values *in the same instance*. See
/// comments on `multi_value_list_multiple_expected_values` for details.
// Java: ConfigSettingTest.multiValueListSingleValueThatLooksLikeMultiple
#[test]
fn multi_value_list_single_value_that_looks_like_multiple() {
    let mut t = BuildViewTestCase::new();
    t.scratch.file(
        "test/BUILD",
        &[r#"config_setting(
    name = "match",
    values = {
        "copt": "one,two",  # This produces ["one,two"]
    },
)
"#],
    );

    t.use_configuration(&[""]);
    assert!(!get_config_matching_provider_result_as_boolean(
        &mut t,
        "//test:match"
    ));
    t.use_configuration(&["--copt", "one"]);
    assert!(!get_config_matching_provider_result_as_boolean(
        &mut t,
        "//test:match"
    ));
    t.use_configuration(&["--copt", "two"]);
    assert!(!get_config_matching_provider_result_as_boolean(
        &mut t,
        "//test:match"
    ));
    t.use_configuration(&["--copt", "one", "--copt", "two"]);
    assert!(!get_config_matching_provider_result_as_boolean(
        &mut t,
        "//test:match"
    ));
    t.use_configuration(&["--copt", "one,two", "--copt", "one"]);
    assert!(get_config_matching_provider_result_as_boolean(
        &mut t,
        "//test:match"
    ));
    t.use_configuration(&["--copt", "two,one", "--copt", "one"]);
    assert!(!get_config_matching_provider_result_as_boolean(
        &mut t,
        "//test:match"
    ));
}

// Java: ConfigSettingTest.selectForDefaultGrteTop
#[test]
fn select_for_default_grte_top() {
    let mut t = BuildViewTestCase::new();
    t.scratch_configured_target(
        "a",
        "a",
        &[
            "config_setting(name='cs', values={'grte_top': 'default'})",
            "filegroup(name='a', srcs=select({':cs': []}))",
        ],
    );
}

// Java: ConfigSettingTest.requiredConfigFragmentMatcher
#[test]
fn required_config_fragment_matcher() {
    let mut t = BuildViewTestCase::new();
    t.scratch.file(
        "test/BUILD",
        &[r#"config_setting(
    name = "match",
    values = {
        "copt": "-Dfoo",
        "javacopt": "-Dbar",
    },
)
"#],
    );

    let Target::Rule(target) = t.get_target("//test:match").unwrap() else {
        panic!("ClassCastException: //test:match is not a Rule");
    };
    let option_reference_function = target
        .get_rule_class_object()
        .get_option_reference_function()
        .expect("NullPointerException: no option reference function");
    let references: Vec<String> = option_reference_function(&target).into_iter().collect();
    assert_contains_exactly(&references, &["copt", "javacopt"]);
}

// Java: ConfigSettingTest.matchesIfFlagValuesAndValuesBothMatch
#[test]
fn matches_if_flag_values_and_values_both_match() {
    let mut t = BuildViewTestCase::new();
    t.use_configuration(&[
        "--copt=-Dright",
        "--enforce_transitive_configs_for_config_feature_flag",
    ]);
    t.scratch.file(
        "test/BUILD",
        &[r#"config_setting(
    name = "match",
    flag_values = {
        ":flag": "right",
    },
    transitive_configs = [":flag"],
    values = {
        "copt": "-Dright",
    },
)

config_feature_flag(
    name = "flag",
    allowed_values = ["right"],
    default_value = "right",
)
"#],
    );
    assert!(get_config_matching_provider_result_as_boolean(
        &mut t,
        "//test:match"
    ));
}

// Java: ConfigSettingTest.matchesIfFlagValuesMatchAndValuesAreEmpty
#[test]
fn matches_if_flag_values_match_and_values_are_empty() {
    let mut t = BuildViewTestCase::new();
    t.use_configuration(&[
        "--copt=-Dright",
        "--enforce_transitive_configs_for_config_feature_flag",
    ]);
    t.scratch.file(
        "test/BUILD",
        &[r#"config_setting(
    name = "match",
    flag_values = {
        ":flag": "right",
    },
    transitive_configs = [":flag"],
    values = {},
)

config_feature_flag(
    name = "flag",
    allowed_values = ["right"],
    default_value = "right",
)
"#],
    );
    assert!(get_config_matching_provider_result_as_boolean(
        &mut t,
        "//test:match"
    ));
}

// Java: ConfigSettingTest.matchesIfValuesMatchAndFlagValuesAreEmpty
#[test]
fn matches_if_values_match_and_flag_values_are_empty() {
    let mut t = BuildViewTestCase::new();
    t.use_configuration(&["--copt=-Dright"]);
    t.scratch.file(
        "test/BUILD",
        &[r#"config_setting(
    name = "match",
    flag_values = {},
    values = {
        "copt": "-Dright",
    },
)
"#],
    );
    assert!(get_config_matching_provider_result_as_boolean(
        &mut t,
        "//test:match"
    ));
}

// Java: ConfigSettingTest.doesNotMatchIfNeitherFlagValuesNorValuesMatches
#[test]
fn does_not_match_if_neither_flag_values_nor_values_matches() {
    let mut t = BuildViewTestCase::new();
    t.use_configuration(&[
        "--copt=-Dright",
        "--enforce_transitive_configs_for_config_feature_flag",
    ]);
    t.scratch.file(
        "test/BUILD",
        &[r#"config_setting(
    name = "match",
    flag_values = {
        ":flag": "wrong",
    },
    transitive_configs = [":flag"],
    values = {
        "copt": "-Dwrong",
    },
)

config_feature_flag(
    name = "flag",
    allowed_values = [
        "right",
        "wrong",
    ],
    default_value = "right",
)
"#],
    );
    assert!(!get_config_matching_provider_result_as_boolean(
        &mut t,
        "//test:match"
    ));
}

// Java: ConfigSettingTest.doesNotMatchIfFlagValuesDoNotMatchAndValuesAreEmpty
#[test]
fn does_not_match_if_flag_values_do_not_match_and_values_are_empty() {
    let mut t = BuildViewTestCase::new();
    t.use_configuration(&["--enforce_transitive_configs_for_config_feature_flag"]);
    t.scratch.file(
        "test/BUILD",
        &[r#"config_setting(
    name = "match",
    flag_values = {
        ":flag": "wrong",
    },
    transitive_configs = [":flag"],
    values = {},
)

config_feature_flag(
    name = "flag",
    allowed_values = [
        "right",
        "wrong",
    ],
    default_value = "right",
)
"#],
    );
    assert!(!get_config_matching_provider_result_as_boolean(
        &mut t,
        "//test:match"
    ));
}

// Java: ConfigSettingTest.doesNotMatchIfFlagValuesDoNotMatchButValuesDo
#[test]
fn does_not_match_if_flag_values_do_not_match_but_values_do() {
    let mut t = BuildViewTestCase::new();
    t.use_configuration(&[
        "--copt=-Dright",
        "--enforce_transitive_configs_for_config_feature_flag",
    ]);
    t.scratch.file(
        "test/BUILD",
        &[r#"config_setting(
    name = "match",
    flag_values = {
        ":flag": "wrong",
    },
    transitive_configs = [":flag"],
    values = {
        "copt": "-Dright",
    },
)

config_feature_flag(
    name = "flag",
    allowed_values = [
        "right",
        "wrong",
    ],
    default_value = "right",
)
"#],
    );
    assert!(!get_config_matching_provider_result_as_boolean(
        &mut t,
        "//test:match"
    ));
}

// Java: ConfigSettingTest.doesNotMatchIfValuesDoNotMatchAndFlagValuesAreEmpty
#[test]
fn does_not_match_if_values_do_not_match_and_flag_values_are_empty() {
    let mut t = BuildViewTestCase::new();
    t.use_configuration(&["--copt=-Dright"]);
    t.scratch.file(
        "test/BUILD",
        &[r#"config_setting(
    name = "match",
    flag_values = {},
    values = {
        "copt": "-Dwrong",
    },
)
"#],
    );
    assert!(!get_config_matching_provider_result_as_boolean(
        &mut t,
        "//test:match"
    ));
}

// Java: ConfigSettingTest.doesNotMatchIfValuesDoNotMatchButFlagValuesDo
#[test]
fn does_not_match_if_values_do_not_match_but_flag_values_do() {
    let mut t = BuildViewTestCase::new();
    t.use_configuration(&[
        "--copt=-Dright",
        "--enforce_transitive_configs_for_config_feature_flag",
    ]);
    t.scratch.file(
        "test/BUILD",
        &[r#"config_setting(
    name = "match",
    flag_values = {
        ":flag": "right",
    },
    transitive_configs = [":flag"],
    values = {
        "copt": "-Dwrong",
    },
)

config_feature_flag(
    name = "flag",
    allowed_values = [
        "right",
        "wrong",
    ],
    default_value = "right",
)
"#],
    );
    assert!(!get_config_matching_provider_result_as_boolean(
        &mut t,
        "//test:match"
    ));
}

// Java: ConfigSettingTest.doesNotMatchIfEvenOneFlagValueDoesNotMatch
#[test]
fn does_not_match_if_even_one_flag_value_does_not_match() {
    let mut t = BuildViewTestCase::new();
    t.use_configuration(&["--enforce_transitive_configs_for_config_feature_flag"]);
    t.scratch.file(
        "test/BUILD",
        &[r#"config_setting(
    name = "match",
    flag_values = {
        ":flag": "right",
        ":flag2": "bad",
    },
    transitive_configs = [
        ":flag",
        ":flag2",
    ],
    values = {},
)

config_feature_flag(
    name = "flag",
    allowed_values = [
        "right",
        "wrong",
    ],
    default_value = "right",
)

config_feature_flag(
    name = "flag2",
    allowed_values = [
        "good",
        "bad",
    ],
    default_value = "good",
)
"#],
    );
    assert!(!get_config_matching_provider_result_as_boolean(
        &mut t,
        "//test:match"
    ));
}

// Java: ConfigSettingTest.matchesIfNonDefaultIsSpecifiedAndFlagValueIsThatValue
#[test]
#[ignore = "needs the test rule feature_flag_setter (FeatureFlagSetterRule, registered by the test's custom rule class provider)"]
fn matches_if_non_default_is_specified_and_flag_value_is_that_value() {
    let mut t = BuildViewTestCase::new();
    t.use_configuration(&["--enforce_transitive_configs_for_config_feature_flag"]);
    t.scratch.file(
        "test/BUILD",
        &[r#"feature_flag_setter(
    name = "setter",
    exports_setting = ":match",
    flag_values = {":flag": "actual"},
    transitive_configs = [":flag"],
)

config_setting(
    name = "match",
    flag_values = {
        ":flag": "actual",
    },
    transitive_configs = [":flag"],
    values = {},
)

config_feature_flag(
    name = "flag",
    allowed_values = [
        "default",
        "actual",
    ],
    default_value = "default",
)
"#],
    );
    assert!(get_config_matching_provider_result_as_boolean(
        &mut t,
        "//test:setter"
    ));
}

// Java: ConfigSettingTest.doesNotMatchIfDefaultIsSpecifiedAndFlagValueIsNotDefault
#[test]
#[ignore = "needs the test rule feature_flag_setter (FeatureFlagSetterRule, registered by the test's custom rule class provider)"]
fn does_not_match_if_default_is_specified_and_flag_value_is_not_default() {
    let mut t = BuildViewTestCase::new();
    t.use_configuration(&["--enforce_transitive_configs_for_config_feature_flag"]);
    t.scratch.file(
        "test/BUILD",
        &[r#"feature_flag_setter(
    name = "setter",
    exports_setting = ":match",
    flag_values = {":flag": "actual"},
    transitive_configs = [":flag"],
)

config_setting(
    name = "match",
    flag_values = {
        ":flag": "default",
    },
    transitive_configs = [":flag"],
    values = {},
)

config_feature_flag(
    name = "flag",
    allowed_values = [
        "default",
        "actual",
    ],
    default_value = "default",
)
"#],
    );
    assert!(!get_config_matching_provider_result_as_boolean(
        &mut t,
        "//test:setter"
    ));
}

/// `useConfiguration("--copt=-Dright", "--javacopt=-Dgood",
/// "--enforce_transitive_configs_for_config_feature_flag")` and `test/BUILD` with `build_file`,
/// then whether `//test:<refined>` refines `//test:<other>` (the body of the `*Refine*` tests).
fn refines_with_flags(build_file: &str, refined: &str, other: &str) -> bool {
    let mut t = BuildViewTestCase::new();
    t.use_configuration(&[
        "--copt=-Dright",
        "--javacopt=-Dgood",
        "--enforce_transitive_configs_for_config_feature_flag",
    ]);
    t.scratch.file("test/BUILD", &[build_file]);
    let refined = get_config_matching_provider(&mut t, refined);
    let other = get_config_matching_provider(&mut t, other);
    refined.refines(&other)
}

// Java: ConfigSettingTest.doesNotRefineSettingWithSameValuesAndSameFlagValuesAndSameConstraintValues
#[test]
fn does_not_refine_setting_with_same_values_and_same_flag_values_and_same_constraint_values() {
    assert!(!refines_with_flags(
        r#"constraint_setting(name = "setting_a")

constraint_value(
    name = "value_a",
    constraint_setting = "setting_a",
)

config_setting(
    name = "refined",
    constraint_values = [
        ":value_a",
    ],
    flag_values = {
        ":flag": "right",
        ":flag2": "good",
    },
    transitive_configs = [
        ":flag",
        ":flag2",
    ],
    values = {
        "copt": "-Dright",
        "javacopt": "-Dgood",
    },
)

config_setting(
    name = "other",
    constraint_values = [
        ":value_a",
    ],
    flag_values = {
        ":flag": "right",
        ":flag2": "good",
    },
    transitive_configs = [
        ":flag",
        ":flag2",
    ],
    values = {
        "copt": "-Dright",
        "javacopt": "-Dgood",
    },
)

config_feature_flag(
    name = "flag",
    allowed_values = [
        "right",
        "wrong",
    ],
    default_value = "right",
)

config_feature_flag(
    name = "flag2",
    allowed_values = [
        "good",
        "bad",
    ],
    default_value = "good",
)
"#,
        "//test:refined",
        "//test:other"
    ));
}

// Java: ConfigSettingTest.doesNotRefineSettingWithDifferentValuesAndSameFlagValuesAndSameConstraintValues
#[test]
fn does_not_refine_setting_with_different_values_and_same_flag_values_and_same_constraint_values() {
    assert!(!refines_with_flags(
        r#"constraint_setting(name = "setting_a")

constraint_value(
    name = "value_a",
    constraint_setting = "setting_a",
)

config_setting(
    name = "refined",
    constraint_values = [
        ":value_a",
    ],
    flag_values = {
        ":flag": "right",
        ":flag2": "good",
    },
    transitive_configs = [
        ":flag",
        ":flag2",
    ],
    values = {
        "javacopt": "-Dgood",
    },
)

config_setting(
    name = "other",
    constraint_values = [
        ":value_a",
    ],
    flag_values = {
        ":flag": "right",
        ":flag2": "good",
    },
    transitive_configs = [
        ":flag",
        ":flag2",
    ],
    values = {
        "copt": "-Dright",
    },
)

config_feature_flag(
    name = "flag",
    allowed_values = [
        "right",
        "wrong",
    ],
    default_value = "right",
)

config_feature_flag(
    name = "flag2",
    allowed_values = [
        "good",
        "bad",
    ],
    default_value = "good",
)
"#,
        "//test:refined",
        "//test:other"
    ));
}

// Java: ConfigSettingTest.doesNotRefineSettingWithSameValuesAndSameConstraintValuesAndDifferentFlagValues
#[test]
fn does_not_refine_setting_with_same_values_and_same_constraint_values_and_different_flag_values() {
    assert!(!refines_with_flags(
        r#"constraint_setting(name = "setting_a")

constraint_value(
    name = "value_a",
    constraint_setting = "setting_a",
)

config_setting(
    name = "refined",
    constraint_values = [
        ":value_a",
    ],
    flag_values = {
        ":flag": "right",
    },
    transitive_configs = [":flag"],
    values = {
        "copt": "-Dright",
        "javacopt": "-Dgood",
    },
)

config_setting(
    name = "other",
    constraint_values = [
        ":value_a",
    ],
    flag_values = {
        ":flag2": "good",
    },
    transitive_configs = [":flag2"],
    values = {
        "copt": "-Dright",
        "javacopt": "-Dgood",
    },
)

config_feature_flag(
    name = "flag",
    allowed_values = [
        "right",
        "wrong",
    ],
    default_value = "right",
)

config_feature_flag(
    name = "flag2",
    allowed_values = [
        "good",
        "bad",
    ],
    default_value = "good",
)
"#,
        "//test:refined",
        "//test:other"
    ));
}

// Java: ConfigSettingTest.doesNotRefineSettingWithDifferentValuesAndDifferentFlagValuesAndDifferentConstraintValues
#[test]
fn does_not_refine_setting_with_different_values_and_different_flag_values_and_different_constraint_values()
 {
    assert!(!refines_with_flags(
        r#"constraint_setting(name = "setting_a")

constraint_value(
    name = "value_a",
    constraint_setting = "setting_a",
)

constraint_setting(name = "setting_b")

constraint_value(
    name = "value_b",
    constraint_setting = "setting_b",
)

config_setting(
    name = "refined",
    constraint_values = [
        ":value_a",
    ],
    flag_values = {
        ":flag": "right",
    },
    transitive_configs = [":flag"],
    values = {
        "copt": "-Dright",
    },
)

config_setting(
    name = "other",
    constraint_values = [
        ":value_b",
    ],
    flag_values = {
        ":flag2": "good",
    },
    transitive_configs = [":flag2"],
    values = {
        "javacopt": "-Dgood",
    },
)

config_feature_flag(
    name = "flag",
    allowed_values = [
        "right",
        "wrong",
    ],
    default_value = "right",
)

config_feature_flag(
    name = "flag2",
    allowed_values = [
        "good",
        "bad",
    ],
    default_value = "good",
)
"#,
        "//test:refined",
        "//test:other"
    ));
}

// Java: ConfigSettingTest.doesNotRefineSettingWithDifferentValuesAndSubsetFlagValuesAndSubsetConstraintValues
#[test]
fn does_not_refine_setting_with_different_values_and_subset_flag_values_and_subset_constraint_values()
 {
    assert!(!refines_with_flags(
        r#"constraint_setting(name = "setting_a")

constraint_value(
    name = "value_a",
    constraint_setting = "setting_a",
)

constraint_setting(name = "setting_b")

constraint_value(
    name = "value_b",
    constraint_setting = "setting_b",
)

config_setting(
    name = "refined",
    constraint_values = [
        ":value_a",
        ":value_b",
    ],
    flag_values = {
        ":flag": "right",
        ":flag2": "good",
    },
    transitive_configs = [
        ":flag",
        ":flag2",
    ],
    values = {
        "copt": "-Dright",
    },
)

config_setting(
    name = "other",
    constraint_values = [
        ":value_a",
    ],
    flag_values = {
        ":flag": "right",
    },
    transitive_configs = [":flag"],
    values = {
        "javacopt": "-Dgood",
    },
)

config_feature_flag(
    name = "flag",
    allowed_values = [
        "right",
        "wrong",
    ],
    default_value = "right",
)

config_feature_flag(
    name = "flag2",
    allowed_values = [
        "good",
        "bad",
    ],
    default_value = "good",
)
"#,
        "//test:refined",
        "//test:other"
    ));
}

// Java: ConfigSettingTest.doesNotRefineSettingWithSubsetValuesAndSubsetFlagValuesAndDifferentConstraintValues
#[test]
fn does_not_refine_setting_with_subset_values_and_subset_flag_values_and_different_constraint_values()
 {
    assert!(!refines_with_flags(
        r#"constraint_setting(name = "setting_a")

constraint_value(
    name = "value_a",
    constraint_setting = "setting_a",
)

constraint_setting(name = "setting_b")

constraint_value(
    name = "value_b",
    constraint_setting = "setting_b",
)

config_setting(
    name = "refined",
    constraint_values = [
        ":value_a",
    ],
    flag_values = {
        ":flag": "right",
        ":flag2": "good",
    },
    transitive_configs = [
        ":flag",
        ":flag2",
    ],
    values = {
        "copt": "-Dright",
        "javacopt": "-Dgood",
    },
)

config_setting(
    name = "other",
    constraint_values = [
        ":value_b",
    ],
    flag_values = {
        ":flag": "right",
        ":flag2": "good",
    },
    transitive_configs = [
        ":flag",
        ":flag2",
    ],
    values = {
        "copt": "-Dright",
    },
)

config_feature_flag(
    name = "flag",
    allowed_values = [
        "right",
        "wrong",
    ],
    default_value = "right",
)

config_feature_flag(
    name = "flag2",
    allowed_values = [
        "good",
        "bad",
    ],
    default_value = "good",
)
"#,
        "//test:refined",
        "//test:other"
    ));
}

// Java: ConfigSettingTest.doesNotRefineSettingWithSubsetValuesAndSubsetConstraintValuesAndDifferentFlagValues
#[test]
fn does_not_refine_setting_with_subset_values_and_subset_constraint_values_and_different_flag_values()
 {
    assert!(!refines_with_flags(
        r#"constraint_setting(name = "setting_a")

constraint_value(
    name = "value_a",
    constraint_setting = "setting_a",
)

constraint_setting(name = "setting_b")

constraint_value(
    name = "value_b",
    constraint_setting = "setting_b",
)

config_setting(
    name = "refined",
    constraint_values = [
        ":value_a",
        ":value_b",
    ],
    flag_values = {
        ":flag": "right",
    },
    transitive_configs = [":flag"],
    values = {
        "copt": "-Dright",
        "javacopt": "-Dgood",
    },
)

config_setting(
    name = "other",
    constraint_values = [
        ":value_a",
    ],
    flag_values = {
        ":flag2": "good",
    },
    transitive_configs = [":flag2"],
    values = {
        "copt": "-Dright",
    },
)

config_feature_flag(
    name = "flag",
    allowed_values = [
        "right",
        "wrong",
    ],
    default_value = "right",
)

config_feature_flag(
    name = "flag2",
    allowed_values = [
        "good",
        "bad",
    ],
    default_value = "good",
)
"#,
        "//test:refined",
        "//test:other"
    ));
}

// Java: ConfigSettingTest.doesNotRefineSettingIfThereIsNoOverlap
#[test]
fn does_not_refine_setting_if_there_is_no_overlap() {
    let mut t = BuildViewTestCase::new();
    t.use_configuration(&[
        "--copt=-Dright",
        "--javacopt=-Dgood",
        "--enforce_transitive_configs_for_config_feature_flag",
    ]);
    t.scratch.file(
        "test/BUILD",
        &[r#"constraint_setting(name = "setting_a")

constraint_value(
    name = "value_a",
    constraint_setting = "setting_a",
)

config_setting(
    name = "configA",
    flag_values = {
        ":flag": "right",
    },
    transitive_configs = [":flag"],
)

config_setting(
    name = "configB",
    constraint_values = [
        ":value_a",
    ],
)

config_feature_flag(
    name = "flag",
    allowed_values = [
        "right",
        "wrong",
    ],
    default_value = "right",
)
"#],
    );
    let config_a = get_config_matching_provider(&mut t, "//test:configA");
    let config_b = get_config_matching_provider(&mut t, "//test:configB");
    assert!(!config_a.refines(&config_b));
    let config_b = get_config_matching_provider(&mut t, "//test:configB");
    let config_a = get_config_matching_provider(&mut t, "//test:configA");
    assert!(!config_b.refines(&config_a));
}

// Java: ConfigSettingTest.refinesSettingWithSubsetValuesAndSubsetConstraintValuesAndSameFlagValues
#[test]
fn refines_setting_with_subset_values_and_subset_constraint_values_and_same_flag_values() {
    assert!(refines_with_flags(
        r#"constraint_setting(name = "setting_a")

constraint_value(
    name = "value_a",
    constraint_setting = "setting_a",
)

constraint_setting(name = "setting_b")

constraint_value(
    name = "value_b",
    constraint_setting = "setting_b",
)

config_setting(
    name = "refined",
    constraint_values = [
        ":value_a",
        ":value_b",
    ],
    flag_values = {
        ":flag": "right",
        ":flag2": "good",
    },
    transitive_configs = [
        ":flag",
        ":flag2",
    ],
    values = {
        "copt": "-Dright",
        "javacopt": "-Dgood",
    },
)

config_setting(
    name = "other",
    constraint_values = [
        ":value_b",
    ],
    flag_values = {
        ":flag": "right",
        ":flag2": "good",
    },
    transitive_configs = [
        ":flag",
        ":flag2",
    ],
    values = {
        "copt": "-Dright",
    },
)

config_feature_flag(
    name = "flag",
    allowed_values = [
        "right",
        "wrong",
    ],
    default_value = "right",
)

config_feature_flag(
    name = "flag2",
    allowed_values = [
        "good",
        "bad",
    ],
    default_value = "good",
)
"#,
        "//test:refined",
        "//test:other"
    ));
}

// Java: ConfigSettingTest.refinesSettingWithSameValuesAndSubsetFlagValuesAndSubsetConstraintValues
#[test]
fn refines_setting_with_same_values_and_subset_flag_values_and_subset_constraint_values() {
    assert!(refines_with_flags(
        r#"constraint_setting(name = "setting_a")

constraint_value(
    name = "value_a",
    constraint_setting = "setting_a",
)

constraint_setting(name = "setting_b")

constraint_value(
    name = "value_b",
    constraint_setting = "setting_b",
)

config_setting(
    name = "refined",
    constraint_values = [
        ":value_a",
        ":value_b",
    ],
    flag_values = {
        ":flag": "right",
        ":flag2": "good",
    },
    transitive_configs = [
        ":flag",
        ":flag2",
    ],
    values = {
        "copt": "-Dright",
        "javacopt": "-Dgood",
    },
)

config_setting(
    name = "other",
    constraint_values = [
        ":value_a",
    ],
    flag_values = {
        ":flag": "right",
    },
    transitive_configs = [":flag"],
    values = {
        "copt": "-Dright",
        "javacopt": "-Dgood",
    },
)

config_feature_flag(
    name = "flag",
    allowed_values = [
        "right",
        "wrong",
    ],
    default_value = "right",
)

config_feature_flag(
    name = "flag2",
    allowed_values = [
        "good",
        "bad",
    ],
    default_value = "good",
)
"#,
        "//test:refined",
        "//test:other"
    ));
}

// Java: ConfigSettingTest.refinesSettingWithSubsetValuesAndSubsetFlagValuesAndConstraintValues
#[test]
fn refines_setting_with_subset_values_and_subset_flag_values_and_constraint_values() {
    assert!(refines_with_flags(
        r#"constraint_setting(name = "setting_a")

constraint_value(
    name = "value_a",
    constraint_setting = "setting_a",
)

constraint_setting(name = "setting_b")

constraint_value(
    name = "value_b",
    constraint_setting = "setting_b",
)

config_setting(
    name = "refined",
    constraint_values = [
        ":value_a",
        ":value_b",
    ],
    flag_values = {
        ":flag": "right",
        ":flag2": "good",
    },
    transitive_configs = [
        ":flag",
        ":flag2",
    ],
    values = {
        "copt": "-Dright",
        "javacopt": "-Dgood",
    },
)

config_setting(
    name = "other",
    constraint_values = [
        ":value_a",
    ],
    flag_values = {
        ":flag": "right",
    },
    transitive_configs = [":flag"],
    values = {
        "copt": "-Dright",
    },
)

config_feature_flag(
    name = "flag",
    allowed_values = [
        "right",
        "wrong",
    ],
    default_value = "right",
)

config_feature_flag(
    name = "flag2",
    allowed_values = [
        "good",
        "bad",
    ],
    default_value = "good",
)
"#,
        "//test:refined",
        "//test:other"
    ));
}

// Java: ConfigSettingTest.refinesSettingWithSubsetConstraintValues
#[test]
fn refines_setting_with_subset_constraint_values() {
    let mut t = BuildViewTestCase::new();
    t.scratch.file(
        "test/BUILD",
        &[r#"constraint_setting(name = "setting_a")

constraint_value(
    name = "value_a",
    constraint_setting = "setting_a",
)

constraint_setting(name = "setting_b")

constraint_value(
    name = "value_b",
    constraint_setting = "setting_b",
)

constraint_setting(name = "setting_c")

constraint_value(
    name = "value_c",
    constraint_setting = "setting_c",
)

platform(
    name = "refined_platform",
    constraint_values = [
        ":value_a",
        ":value_b",
        ":value_c",
    ],
)

platform(
    name = "other_platform",
    constraint_values = [
        ":value_a",
        ":value_b",
    ],
)

config_setting(
    name = "refined",
    constraint_values = [
        ":value_a",
        ":value_b",
        ":value_c",
    ],
)

config_setting(
    name = "other",
    constraint_values = [
        ":value_a",
        ":value_b",
    ],
)
"#],
    );
    t.use_configuration(&["--platforms=//test:refined_platform"]);
    let refined = get_config_matching_provider(&mut t, "//test:refined");
    let other = get_config_matching_provider(&mut t, "//test:other");
    assert!(refined.refines(&other));
}

// Java: ConfigSettingTest.refinesSettingWithRefinedConstraintValue
#[test]
fn refines_setting_with_refined_constraint_value() {
    let mut t = BuildViewTestCase::new();
    t.scratch.file(
        "libc/BUILD",
        &[r#"constraint_setting(name = "libc")

constraint_value(
    name = "glibc",
    constraint_setting = ":libc",
)
"#],
    );
    t.scratch.file(
        "libc/glibc/BUILD",
        &[r#"constraint_setting(
    name = "version",
    refines_constraint_value = "//libc:glibc",
)

constraint_value(
    name = "2.42",
    constraint_setting = ":version",
)
"#],
    );
    t.scratch.file(
        "test/BUILD",
        &[r#"platform(
    name = "p",
    constraint_values = [
        "//libc:glibc",
        "//libc/glibc:2.42",
    ],
)

config_setting(
    name = "refining",
    constraint_values = ["//libc/glibc:2.42"],
)

config_setting(
    name = "refined",
    constraint_values = ["//libc:glibc"],
)
"#],
    );
    t.use_configuration(&["--platforms=//test:p"]);
    let refining = get_config_matching_provider(&mut t, "//test:refining");
    let refined = get_config_matching_provider(&mut t, "//test:refined");
    assert!(refining.refines(&refined));
    let refined = get_config_matching_provider(&mut t, "//test:refined");
    let refining = get_config_matching_provider(&mut t, "//test:refining");
    assert!(!refined.refines(&refining));
}

// Java: ConfigSettingTest.matchesAliasedFlagsInFlagValues
#[test]
fn matches_aliased_flags_in_flag_values() {
    let mut t = BuildViewTestCase::new();
    t.use_configuration(&["--enforce_transitive_configs_for_config_feature_flag"]);
    t.scratch.file(
        "test/BUILD",
        &[r#"config_setting(
    name = "alias_matcher",
    flag_values = {
        ":alias": "right",
    },
    transitive_configs = [":flag"],
)

alias(
    name = "alias",
    actual = "flag",
    transitive_configs = [":flag"],
)

config_feature_flag(
    name = "flag",
    allowed_values = [
        "right",
        "wrong",
    ],
    default_value = "right",
)
"#],
    );
    assert!(get_config_matching_provider_result_as_boolean(
        &mut t,
        "//test:alias_matcher"
    ));
}

// Java: ConfigSettingTest.aliasedFlagsAreCountedInRefining
#[test]
fn aliased_flags_are_counted_in_refining() {
    let mut t = BuildViewTestCase::new();
    t.use_configuration(&["--enforce_transitive_configs_for_config_feature_flag"]);
    t.scratch.file(
        "test/BUILD",
        &[r#"config_setting(
    name = "refined",
    flag_values = {
        ":alias": "right",
        ":flag2": "good",
    },
    transitive_configs = [
        ":flag",
        ":flag2",
    ],
)

config_setting(
    name = "other",
    flag_values = {
        ":flag": "right",
    },
    transitive_configs = [":flag"],
)

alias(
    name = "alias",
    actual = "flag",
    transitive_configs = [":flag"],
)

config_feature_flag(
    name = "flag",
    allowed_values = [
        "right",
        "wrong",
    ],
    default_value = "right",
)

config_feature_flag(
    name = "flag2",
    allowed_values = [
        "good",
        "bad",
    ],
    default_value = "good",
)
"#],
    );
    let refined = get_config_matching_provider(&mut t, "//test:refined");
    let other = get_config_matching_provider(&mut t, "//test:other");
    assert!(refined.refines(&other));
}

// Java: ConfigSettingTest.referencingSameFlagViaMultipleAliasesFails
#[test]
fn referencing_same_flag_via_multiple_aliases_fails() {
    let mut t = BuildViewTestCase::new();
    t.use_configuration(&["--enforce_transitive_configs_for_config_feature_flag"]);
    t.check_error(
        "test",
        "multialias",
        "in flag_values attribute of config_setting rule //test:multialias: \
         flag '//test:direct' referenced multiple times as ['//test:alias', '//test:direct']",
        &[
            "config_setting(",
            "    name = 'multialias',",
            "    flag_values = {",
            "        ':alias': 'right',",
            "        ':direct': 'right',",
            "    },",
            "    transitive_configs = [':direct'],",
            ")",
            "alias(",
            "    name = 'alias',",
            "    actual = 'direct',",
            "    transitive_configs = [':direct'],",
            ")",
            "config_feature_flag(",
            "    name = 'direct',",
            "    allowed_values = ['right', 'wrong'],",
            "    default_value = 'right',",
            ")",
        ],
    );
}

// Java: ConfigSettingTest.requiresValidValueForFlagValues
#[test]
fn requires_valid_value_for_flag_values() {
    let mut t = BuildViewTestCase::new();
    t.use_configuration(&["--enforce_transitive_configs_for_config_feature_flag"]);
    t.check_error(
        "test",
        "invalid_flag",
        "in flag_values attribute of config_setting rule //test:invalid_flag: \
         error while parsing user-defined configuration values: \
         'invalid' is not a valid value for '//test:flag'",
        &[
            "config_setting(",
            "    name = 'invalid_flag',",
            "    flag_values = {",
            "        ':flag': 'invalid',",
            "    },",
            "    transitive_configs = [':flag'])",
            "config_feature_flag(",
            "    name = 'flag',",
            "    allowed_values = ['right', 'valid'],",
            "    default_value = 'valid',",
            ")",
        ],
    );
}

// Java: ConfigSettingTest.usesAliasLabelWhenReportingErrorInFlagValues
#[test]
fn uses_alias_label_when_reporting_error_in_flag_values() {
    let mut t = BuildViewTestCase::new();
    t.use_configuration(&["--enforce_transitive_configs_for_config_feature_flag"]);
    t.check_error(
        "test",
        "invalid_flag",
        "in flag_values attribute of config_setting rule //test:invalid_flag: \
         error while parsing user-defined configuration values: \
         'invalid' is not a valid value for '//test:alias'",
        &[
            "config_setting(",
            "    name = 'invalid_flag',",
            "    flag_values = {",
            "        ':alias': 'invalid',",
            "    },",
            "    transitive_configs = [':flag'])",
            "alias(",
            "    name = 'alias',",
            "    actual = ':flag',",
            "    transitive_configs = [':flag'],",
            ")",
            "config_feature_flag(",
            "    name = 'flag',",
            "    allowed_values = ['right', 'valid'],",
            "    default_value = 'valid',",
            ")",
        ],
    );
}

// Java: ConfigSettingTest.buildsettings_matchesFromDefault
#[test]
fn buildsettings_matches_from_default() {
    let mut t = BuildViewTestCase::new();
    t.scratch.file(
        "test/build_settings.bzl",
        &[r#"def _impl(ctx):
    return []

string_flag = rule(implementation = _impl, build_setting = config.string(flag = True))
"#],
    );
    t.scratch.file(
        "test/BUILD",
        &[r#"load("//test:build_settings.bzl", "string_flag")

config_setting(
    name = "match",
    flag_values = {
        ":cheese": "parmesan",
    },
)

string_flag(
    name = "cheese",
    build_setting_default = "parmesan",
)
"#],
    );
    assert!(get_config_matching_provider_result_as_boolean(
        &mut t,
        "//test:match"
    ));
}

// Java: ConfigSettingTest.buildsettings_matchesFromCommandLine
#[test]
fn buildsettings_matches_from_command_line() {
    let mut t = BuildViewTestCase::new();
    t.scratch.file(
        "test/build_settings.bzl",
        &[r#"def _impl(ctx):
    return []

string_flag = rule(implementation = _impl, build_setting = config.string(flag = True))
"#],
    );
    t.scratch.file(
        "test/BUILD",
        &[r#"load("//test:build_settings.bzl", "string_flag")

config_setting(
    name = "match",
    flag_values = {
        ":cheese": "gouda",
    },
)

string_flag(
    name = "cheese",
    build_setting_default = "parmesan",
)
"#],
    );

    t.use_configuration(&["--//test:cheese=gouda"]);
    assert!(get_config_matching_provider_result_as_boolean(
        &mut t,
        "//test:match"
    ));
}

/// Regression test to ensure that non-String typed build setting values are being properly
/// converted from Strings to their real type.
// Java: ConfigSettingTest.buildsettings_convertedType
#[test]
fn buildsettings_converted_type() {
    let mut t = BuildViewTestCase::new();
    t.scratch.file(
        "test/build_settings.bzl",
        &[r#"def _impl(ctx):
    return []

bool_flag = rule(implementation = _impl, build_setting = config.bool(flag = True))
"#],
    );
    t.scratch.file(
        "test/BUILD",
        &[r#"load("//test:build_settings.bzl", "bool_flag")

config_setting(
    name = "match",
    flag_values = {
        ":cheese": "True",
    },
)

bool_flag(
    name = "cheese",
    build_setting_default = True,
)
"#],
    );
    assert!(get_config_matching_provider_result_as_boolean(
        &mut t,
        "//test:match"
    ));
}

// Java: ConfigSettingTest.buildsettings_doesntMatch
#[test]
fn buildsettings_doesnt_match() {
    let mut t = BuildViewTestCase::new();
    t.scratch.file(
        "test/build_settings.bzl",
        &[r#"def _impl(ctx):
    return []

string_flag = rule(implementation = _impl, build_setting = config.string(flag = True))
"#],
    );
    t.scratch.file(
        "test/BUILD",
        &[r#"load("//test:build_settings.bzl", "string_flag")

config_setting(
    name = "match",
    flag_values = {
        ":cheese": "parmesan",
    },
)

string_flag(
    name = "cheese",
    build_setting_default = "parmesan",
)
"#],
    );

    t.use_configuration(&["--//test:cheese=gouda"]);
    assert!(!get_config_matching_provider_result_as_boolean(
        &mut t,
        "//test:match"
    ));
}

// Java: ConfigSettingTest.buildsettings_badType
#[test]
fn buildsettings_bad_type() {
    let mut t = BuildViewTestCase::new();
    t.scratch.file(
        "test/build_settings.bzl",
        &[r#"def _impl(ctx):
    return []

int_flag = rule(implementation = _impl, build_setting = config.int(flag = True))
"#],
    );
    t.scratch.file(
        "test/BUILD",
        &[r#"load("//test:build_settings.bzl", "int_flag")

config_setting(
    name = "match",
    flag_values = {
        ":wishes": "gouda",
    },
)

int_flag(
    name = "wishes",
    build_setting_default = 3,
)
"#],
    );

    t.remove_fail_fast_handler();
    t.get_configured_target("//test:match");
    t.assert_contains_event("'gouda' cannot be converted to //test:wishes type int");
}

// Java: ConfigSettingTest.buildsettings_allowMultipleWorks
#[test]
fn buildsettings_allow_multiple_works() {
    let mut t = BuildViewTestCase::new();
    t.scratch.file(
        "test/build_settings.bzl",
        &[r#"def _impl(ctx):
    return []

string_flag = rule(
    implementation = _impl,
    build_setting = config.string(flag = True, allow_multiple = True),
)
"#],
    );
    t.scratch.file(
        "test/BUILD",
        &[r#"load("//test:build_settings.bzl", "string_flag")

config_setting(
    name = "match",
    flag_values = {
        ":cheese": "pepperjack",
    },
)

string_flag(
    name = "cheese",
    build_setting_default = "gouda",
)
"#],
    );
    t.use_configuration(&["--//test:cheese=pepperjack", "--//test:cheese=brie"]);
    assert!(get_config_matching_provider_result_as_boolean(
        &mut t,
        "//test:match"
    ));
}

// Java: ConfigSettingTest.buildsettings_repeatableWorks
#[test]
fn buildsettings_repeatable_works() {
    let mut t = BuildViewTestCase::new();
    t.scratch.file(
        "test/build_settings.bzl",
        &[r#"def _impl(ctx):
    return []

string_list_flag = rule(
    implementation = _impl,
    build_setting = config.string_list(flag = True, repeatable = True),
)
"#],
    );
    t.scratch.file(
        "test/BUILD",
        &[r#"load("//test:build_settings.bzl", "string_list_flag")

config_setting(
    name = "match",
    flag_values = {
        ":cheese": "pepperjack",
    },
)

string_list_flag(
    name = "cheese",
    build_setting_default = ["gouda"],
)
"#],
    );

    t.use_configuration(&[
        "--//test:cheese=pepperjack",
        "--//test:cheese=pepperjack=brie",
    ]);
    assert!(get_config_matching_provider_result_as_boolean(
        &mut t,
        "//test:match"
    ));
}

// Java: ConfigSettingTest.buildsettings_repeatableWithoutFlagErrors
#[test]
fn buildsettings_repeatable_without_flag_errors() {
    let mut t = BuildViewTestCase::new();
    t.scratch.file(
        "test/build_settings.bzl",
        &[r#"def _impl(ctx):
    return []

string_list_setting = rule(
    implementation = _impl,
    build_setting = config.string_list(repeatable = True),
)
"#],
    );
    t.scratch.file(
        "test/BUILD",
        &[r#"load("//test:build_settings.bzl", "string_list_setting")

string_list_setting(
    name = "cheese",
    build_setting_default = ["gouda"],
)
"#],
    );

    t.remove_fail_fast_handler();
    t.get_configured_target("//test:cheese");
    t.assert_contains_event("'repeatable' can only be set for a setting with 'flag = True'");
}

// Java: ConfigSettingTest.notBuildSettingOrFeatureFlag
#[test]
fn not_build_setting_or_feature_flag() {
    let mut t = BuildViewTestCase::new();
    t.scratch.file(
        "test/rules.bzl",
        &[r#"def _impl(ctx):
    return DefaultInfo()

default_info_rule = rule(implementation = _impl)
"#],
    );
    t.scratch.file(
        "test/BUILD",
        &[r#"load("//test:rules.bzl", "default_info_rule")

config_setting(
    name = "match",
    flag_values = {
        ":cheese": "gouda",
    },
)

default_info_rule(name = "cheese")
"#],
    );

    t.remove_fail_fast_handler();
    t.get_configured_target("//test:match");
    t.assert_contains_event(
        "flag_values keys must be build settings or feature flags and //test:cheese is not",
    );
}

// Java: ConfigSettingTest.buildsettingsMatch_featureFlagsMatch
#[test]
fn buildsettings_match_feature_flags_match() {
    let mut t = BuildViewTestCase::new();
    t.use_configuration(&["--enforce_transitive_configs_for_config_feature_flag"]);

    t.scratch.file(
        "test/build_settings.bzl",
        &[r#"def _impl(ctx):
    return []

string_flag = rule(implementation = _impl, build_setting = config.string(flag = True))
"#],
    );
    t.scratch.file(
        "test/BUILD",
        &[r#"load("//test:build_settings.bzl", "string_flag")

config_setting(
    name = "match",
    flag_values = {
        ":cheese": "parmesan",
        ":flag": "right",
    },
    transitive_configs = [":flag"],
)

string_flag(
    name = "cheese",
    build_setting_default = "parmesan",
)

config_feature_flag(
    name = "flag",
    allowed_values = ["right"],
    default_value = "right",
)
"#],
    );
    assert!(get_config_matching_provider_result_as_boolean(
        &mut t,
        "//test:match"
    ));
}

// Java: ConfigSettingTest.buildsettingsMatch_featureFlagsDontMatch
#[test]
fn buildsettings_match_feature_flags_dont_match() {
    let mut t = BuildViewTestCase::new();
    t.use_configuration(&["--enforce_transitive_configs_for_config_feature_flag"]);

    t.scratch.file(
        "test/build_settings.bzl",
        &[r#"def _impl(ctx):
    return []

string_flag = rule(implementation = _impl, build_setting = config.string(flag = True))
"#],
    );
    t.scratch.file(
        "test/BUILD",
        &[r#"load("//test:build_settings.bzl", "string_flag")

config_setting(
    name = "match",
    flag_values = {
        ":cheese": "parmesan",
        ":flag": "wrong",
    },
    transitive_configs = [":flag"],
)

string_flag(
    name = "cheese",
    build_setting_default = "parmesan",
)

config_feature_flag(
    name = "flag",
    allowed_values = [
        "right",
        "wrong",
    ],
    default_value = "right",
)
"#],
    );
    assert!(!get_config_matching_provider_result_as_boolean(
        &mut t,
        "//test:match"
    ));
}

// Java: ConfigSettingTest.buildsettingsDontMatch_featureFlagsMatch
#[test]
fn buildsettings_dont_match_feature_flags_match() {
    let mut t = BuildViewTestCase::new();
    t.use_configuration(&["--enforce_transitive_configs_for_config_feature_flag"]);

    t.scratch.file(
        "test/build_settings.bzl",
        &[r#"def _impl(ctx):
    return []

string_flag = rule(implementation = _impl, build_setting = config.string(flag = True))
"#],
    );
    t.scratch.file(
        "test/BUILD",
        &[r#"load("//test:build_settings.bzl", "string_flag")

config_setting(
    name = "match",
    flag_values = {
        ":cheese": "gouda",
        ":flag": "right",
    },
    transitive_configs = [":flag"],
)

string_flag(
    name = "cheese",
    build_setting_default = "parmesan",
)

config_feature_flag(
    name = "flag",
    allowed_values = ["right"],
    default_value = "right",
)
"#],
    );
    assert!(!get_config_matching_provider_result_as_boolean(
        &mut t,
        "//test:match"
    ));
}

// Java: ConfigSettingTest.constraintValue
#[test]
fn constraint_value() {
    let mut t = BuildViewTestCase::new();
    t.scratch.file(
        "test/BUILD",
        &[r#"constraint_setting(name = "notable_building")

constraint_value(
    name = "empire_state",
    constraint_setting = "notable_building",
)

constraint_value(
    name = "space_needle",
    constraint_setting = "notable_building",
)

platform(
    name = "new_york_platform",
    constraint_values = [":empire_state"],
)

platform(
    name = "seattle_platform",
    constraint_values = [":space_needle"],
)

config_setting(
    name = "match",
    constraint_values = [":empire_state"],
)
"#],
    );

    t.use_configuration(&["--experimental_platforms=//test:new_york_platform"]);
    assert!(get_config_matching_provider_result_as_boolean(
        &mut t,
        "//test:match"
    ));
    t.use_configuration(&["--experimental_platforms=//test:seattle_platform"]);
    assert!(!get_config_matching_provider_result_as_boolean(
        &mut t,
        "//test:match"
    ));
    t.use_configuration(&[""]);
    assert!(!get_config_matching_provider_result_as_boolean(
        &mut t,
        "//test:match"
    ));
}

// Java: ConfigSettingTest.multipleConstraintValues
#[test]
fn multiple_constraint_values() {
    let mut t = BuildViewTestCase::new();
    t.scratch.file(
        "test/BUILD",
        &[r#"constraint_setting(name = "notable_building")

constraint_value(
    name = "empire_state",
    constraint_setting = "notable_building",
)

constraint_setting(name = "museum")

constraint_value(
    name = "cloisters",
    constraint_setting = "museum",
)

constraint_setting(name = "theme_park")

constraint_value(
    name = "coney_island",
    constraint_setting = "theme_park",
)

platform(
    name = "manhattan_platform",
    constraint_values = [
        ":empire_state",
        ":cloisters",
    ],
)

platform(
    name = "museum_platform",
    constraint_values = [":cloisters"],
)

platform(
    name = "new_york_platform",
    constraint_values = [
        ":empire_state",
        ":cloisters",
        ":coney_island",
    ],
)

config_setting(
    name = "match",
    constraint_values = [
        ":empire_state",
        ":cloisters",
    ],
)
"#],
    );
    t.use_configuration(&["--experimental_platforms=//test:manhattan_platform"]);
    assert!(get_config_matching_provider_result_as_boolean(
        &mut t,
        "//test:match"
    ));
    t.use_configuration(&["--experimental_platforms=//test:museum_platform"]);
    assert!(!get_config_matching_provider_result_as_boolean(
        &mut t,
        "//test:match"
    ));
    t.use_configuration(&["--experimental_platforms=//test:new_york_platform"]);
    assert!(get_config_matching_provider_result_as_boolean(
        &mut t,
        "//test:match"
    ));
}

// Java: ConfigSettingTest.definesAndConstraints
#[test]
fn defines_and_constraints() {
    let mut t = BuildViewTestCase::new();
    t.scratch.file(
        "test/BUILD",
        &[r#"constraint_setting(name = "notable_building")

constraint_value(
    name = "empire_state",
    constraint_setting = "notable_building",
)

constraint_value(
    name = "space_needle",
    constraint_setting = "notable_building",
)

platform(
    name = "new_york_platform",
    constraint_values = [":empire_state"],
)

platform(
    name = "seattle_platform",
    constraint_values = [":space_needle"],
)

config_setting(
    name = "match",
    constraint_values = [":empire_state"],
    define_values = {
        "b": "d",
    },
    values = {
        "define": "a=c",
    },
)
"#],
    );

    t.use_configuration(&[
        "--experimental_platforms=//test:new_york_platform",
        "--define",
        "a=c",
        "--define",
        "b=d",
    ]);
    assert!(get_config_matching_provider_result_as_boolean(
        &mut t,
        "//test:match"
    ));
    t.use_configuration(&["--experimental_platforms=//test:new_york_platform"]);
    assert!(!get_config_matching_provider_result_as_boolean(
        &mut t,
        "//test:match"
    ));
    t.use_configuration(&["--define", "a=c"]);
    assert!(!get_config_matching_provider_result_as_boolean(
        &mut t,
        "//test:match"
    ));
    t.use_configuration(&[
        "--define",
        "a=c",
        "--experimental_platforms=//test:new_york_platform",
    ]);
    assert!(!get_config_matching_provider_result_as_boolean(
        &mut t,
        "//test:match"
    ));
}

/// Tests that a config_setting doesn't allow a constraint_values list with more than one
/// constraint value per constraint setting.
// Java: ConfigSettingTest.multipleValuesPerSetting
#[test]
fn multiple_values_per_setting() {
    let mut t = BuildViewTestCase::new();
    t.check_error(
        "foo",
        "bad",
        "in config_setting rule //foo:bad: \
         Duplicate constraint values detected: \
         constraint_setting //foo:notable_building has \
         [//foo:empire_state, //foo:space_needle], \
         constraint_setting //foo:museum has \
         [//foo:moma, //foo:sam]",
        &[
            "constraint_setting(name = 'notable_building')",
            "constraint_value(name = 'empire_state', constraint_setting = 'notable_building')",
            "constraint_value(name = 'space_needle', constraint_setting = 'notable_building')",
            "constraint_value(name = 'peace_arch', constraint_setting = 'notable_building')",
            "constraint_setting(name = 'museum')",
            "constraint_value(name = 'moma', constraint_setting = 'museum')",
            "constraint_value(name = 'sam', constraint_setting = 'museum')",
            "config_setting(",
            "    name = 'bad',",
            "    constraint_values = [",
            "        ':empire_state',",
            "        ':space_needle',",
            "        ':moma',",
            "        ':sam',",
            "    ],",
            ");",
        ],
    );
}

// Java: ConfigSettingTest.notAConstraintValue
#[test]
fn not_a_constraint_value() {
    let mut t = BuildViewTestCase::new();
    t.check_error(
        "test",
        "match",
        "//test:what_am_i is not a constraint_value",
        &[
            "genrule(",
            "    name = 'what_am_i',",
            "    srcs = [],",
            "    outs = ['the_answer'],",
            "    cmd = 'echo an eternal enigma > $@')",
            "config_setting(",
            "    name = 'match',",
            "    constraint_values = [':what_am_i'],",
            ")",
        ],
    );
}

// Java: ConfigSettingTest.getLicenses
fn get_licenses(t: &mut BuildViewTestCase, label: &str) -> BTreeSet<LicenseType> {
    let Target::Rule(rule) = t.get_target(label).unwrap() else {
        panic!("ClassCastException: {label} is not a Rule");
    };
    // There are two interfaces for retrieving a rule's license: from the Rule object and by
    // directly reading the "licenses" attribute. For config_setting both of these should always
    // be NONE. This method checks consistency between them.
    let from_rule = rule.get_license().get_license_types().clone();
    let from_attribute = RawAttributeMapper::of(&*rule)
        .get("licenses", Type::License)
        .expect("NullPointerException: no licenses attribute value")
        .as_license()
        .expect("ClassCastException: licenses is not a License")
        .get_license_types()
        .clone();
    assert_eq!(from_rule, from_attribute);
    from_rule
}

/// Tests that default license behavior is unaffected.
// Java: ConfigSettingTest.licensesDefault
#[test]
fn licenses_default() {
    let mut t = BuildViewTestCase::new();
    t.scratch.file(
        "test/BUILD",
        &[r#"config_setting(
    name = "match",
    values = {
        "copt": "-Dfoo",
    },
)
"#],
    );

    t.use_configuration(&["--copt", "-Dfoo"]);
    assert_eq!(
        get_licenses(&mut t, "//test:match"),
        BTreeSet::from([LicenseType::None])
    );
}

/// Tests that third-party doesn't require a license from config_setting.
// Java: ConfigSettingTest.thirdPartyLicenseRequirement
#[test]
fn third_party_license_requirement() {
    let mut t = BuildViewTestCase::new();
    t.scratch.file(
        "third_party/test/BUILD",
        &[r#"config_setting(
    name = "match",
    values = {
        "copt": "-Dfoo",
    },
)
"#],
    );

    t.use_configuration(&["--copt", "-Dfoo"]);
    assert_eq!(
        get_licenses(&mut t, "//third_party/test:match"),
        BTreeSet::from([LicenseType::None])
    );
}

/// Tests that package-wide licenses are ignored by config_setting.
// Java: ConfigSettingTest.packageLicensesIgnored
#[test]
fn package_licenses_ignored() {
    let mut t = BuildViewTestCase::new();
    t.scratch.file(
        "test/BUILD",
        &[r#"licenses(["restricted"])

config_setting(
    name = "match",
    values = {
        "copt": "-Dfoo",
    },
)
"#],
    );

    t.use_configuration(&["--copt", "-Dfoo"]);
    assert_eq!(
        get_licenses(&mut t, "//test:match"),
        BTreeSet::from([LicenseType::None])
    );
}

/// Tests that rule-specific licenses are ignored by config_setting.
// Java: ConfigSettingTest.ruleLicensesUsed
#[test]
fn rule_licenses_used() {
    let mut t = BuildViewTestCase::new();
    t.scratch.file(
        "test/BUILD",
        &[r#"config_setting(
    name = "match",
    licenses = ["restricted"],
    values = {
        "copt": "-Dfoo",
    },
)
"#],
    );

    t.use_configuration(&["--copt", "-Dfoo"]);
    assert_eq!(
        get_licenses(&mut t, "//test:match"),
        BTreeSet::from([LicenseType::None])
    );
}

// Java: ConfigSettingTest.aliasedStarlarkFlag
#[test]
fn aliased_starlark_flag() {
    let mut t = BuildViewTestCase::new();
    t.scratch.file(
        "test/flagdef.bzl",
        &[r#"def _impl(ctx):
    return []

my_flag = rule(
    implementation = _impl,
    build_setting = config.string(flag = True),
)
"#],
    );

    t.scratch.file(
        "test/BUILD",
        &[r#"load("//test:flagdef.bzl", "my_flag")

my_flag(
    name = "flag",
    build_setting_default = "default",
)

alias(
    name = "alias",
    actual = ":flag",
)

config_setting(
    name = "alias_setting",
    flag_values = {":alias": "specified"},
)
"#],
    );

    // Expect config_setting on an alias to pass completely through the alias to the underlying
    // flag it references. This means aliases model which flags trigger config_setting matches.
    // This keeps config_seting in sync with actual builds: if someone builds with
    // --//foo:alias=1, both the user and config_setting interpret it the same way even when the
    // underlying flag changes.
    t.use_configuration(&["--//test:flag=specified"]);
    assert!(get_config_matching_provider_result_as_boolean(
        &mut t,
        "//test:alias_setting"
    ));
}

// Java: ConfigSettingTest.labelStarlarkFlag
#[test]
fn label_starlark_flag() {
    let mut t = BuildViewTestCase::new();
    t.scratch.file(
        "test/BUILD",
        &[r#"
label_flag(
    name = "my_flag",
    build_setting_default = "other_target",
)

genrule(
    name = "other_target",
    srcs = [],
    outs = ["other_target"],
    cmd = "echo other_target",
)

config_setting(
    name = "my_setting",
    flag_values = {":my_flag": "//test:other_target"},
)
"#],
    );

    // While label_flag is technically an alias, we can't treat it the same way as a normal
    // alias: label_flag is by definition a flag, and therefore a valid config_setting input. But
    // the target it refers to isn't necessarily a flag (and for most practical uses won't be a
    // flag). So it doesn't make sense to treat it like an alias(), where config_setting setting
    // matches against the reference's value. So config_setting treats a label_flag's value like
    // any normal flag value and compares against it directly.
    assert!(get_config_matching_provider_result_as_boolean(
        &mut t,
        "//test:my_setting"
    ));
}

// Java: ConfigSettingTest.simpleStarlarkFlag
#[test]
fn simple_starlark_flag() {
    let mut t = BuildViewTestCase::new();
    t.scratch.file(
        "test/flagdef.bzl",
        &[r#"def _impl(ctx):
    return []

my_flag = rule(
    implementation = _impl,
    build_setting = config.string(flag = True),
)
"#],
    );
    t.scratch.file(
        "test/BUILD",
        &[r#"load("//test:flagdef.bzl", "my_flag")

my_flag(
    name = "flag",
    build_setting_default = "actual_flag_value",
)

config_setting(
    name = "matches",
    flag_values = {
        ":flag": "actual_flag_value",
    },
)

config_setting(
    name = "doesntmatch",
    flag_values = {
        ":flag": "other_flag_value",
    },
)
"#],
    );
    assert!(get_config_matching_provider_result_as_boolean(
        &mut t,
        "//test:matches"
    ));
    assert!(!get_config_matching_provider_result_as_boolean(
        &mut t,
        "//test:doesntmatch"
    ));
}

// Java: ConfigSettingTest.starlarkListFlagSingleValue
#[test]
fn starlark_list_flag_single_value() {
    let mut t = BuildViewTestCase::new();
    // When a list-typed Starlark flag has value ["foo"], the config_setting's expected value
    // "foo" must match exactly.
    t.scratch.file(
        "test/flagdef.bzl",
        &[r#"def _impl(ctx):
    return []

my_flag = rule(
    implementation = _impl,
    build_setting = config.string_list(flag = True),
)
"#],
    );
    t.scratch.file(
        "test/BUILD",
        &[r#"load("//test:flagdef.bzl", "my_flag")

my_flag(
    name = "one_value_flag",
    build_setting_default = ["one"],
)

config_setting(
    name = "matches",
    flag_values = {
        ":one_value_flag": "one",
    },
)

config_setting(
    name = "doesntmatch",
    flag_values = {
        ":one_value_flag": "other",
    },
)
"#],
    );
    assert!(get_config_matching_provider_result_as_boolean(
        &mut t,
        "//test:matches"
    ));
    assert!(!get_config_matching_provider_result_as_boolean(
        &mut t,
        "//test:doesntmatch"
    ));
}

// Java: ConfigSettingTest.starlarkListFlagMultiValue
#[test]
fn starlark_list_flag_multi_value() {
    let mut t = BuildViewTestCase::new();
    // When a list-typed Starlark flag has value ["foo", "bar"], the config_setting's expected
    // value "foo" must match *any* entry in the list.
    t.scratch.file(
        "test/flagdef.bzl",
        &[r#"def _impl(ctx):
    return []

my_flag = rule(
    implementation = _impl,
    build_setting = config.string_list(flag = True),
)
"#],
    );
    t.scratch.file(
        "test/BUILD",
        &[r#"load("//test:flagdef.bzl", "my_flag")

my_flag(
    name = "two_value_flag",
    build_setting_default = [
        "one",
        "two",
    ],
)

config_setting(
    name = "matches_one",
    flag_values = {
        ":two_value_flag": "one",
    },
)

config_setting(
    name = "matches_two",
    flag_values = {
        ":two_value_flag": "two",
    },
)

config_setting(
    name = "doesntmatch",
    flag_values = {
        ":two_value_flag": "other",
    },
)
"#],
    );
    assert!(get_config_matching_provider_result_as_boolean(
        &mut t,
        "//test:matches_one"
    ));
    assert!(get_config_matching_provider_result_as_boolean(
        &mut t,
        "//test:matches_two"
    ));
    assert!(!get_config_matching_provider_result_as_boolean(
        &mut t,
        "//test:doesntmatch"
    ));
}

// Java: ConfigSettingTest.canOnlyMatchSingleValueInMultiValueFlags
fn can_only_match_single_value_in_multi_value_flags(repeatable: bool) {
    let mut t = BuildViewTestCase::new();
    t.scratch.file(
        "test/build_settings.bzl",
        &[&format!(
            r#"def _impl(ctx):
    return []

string_list_flag = rule(
    implementation = _impl,
    build_setting = config.string_list(flag = True, repeatable = {}),
)
"#,
            if repeatable { "True" } else { "False" }
        )],
    );
    t.scratch.file(
        "test/BUILD",
        &[r#"load("//test:build_settings.bzl", "string_list_flag")

string_list_flag(
    name = "gouda",
    build_setting_default = ["smoked"],
)

config_setting(
    name = "match",
    flag_values = {
        ":gouda": "smoked,fresh",
    },
)

filegroup(
    name = "fg",
    srcs = select({
        ":match": [],
    }),
)
"#],
    );
    t.remove_fail_fast_handler(); // expect errors
    assert!(t.get_configured_target("//test:fg").is_none());
    t.assert_contains_event(
        "\"smoked,fresh\" not a valid value for flag //test:gouda. \
         Only single, exact values are allowed",
    );
}

// Java: ConfigSettingTest.canOnlyMatchSingleValueInMultiValueFlags (repeatable = true)
#[test]
fn can_only_match_single_value_in_multi_value_flags_repeatable_true() {
    can_only_match_single_value_in_multi_value_flags(true);
}

// Java: ConfigSettingTest.canOnlyMatchSingleValueInMultiValueFlags (repeatable = false)
#[test]
fn can_only_match_single_value_in_multi_value_flags_repeatable_false() {
    can_only_match_single_value_in_multi_value_flags(false);
}

// Java: ConfigSettingTest.singleValueThatLooksLikeMultiValueIsOkay
#[test]
fn single_value_that_looks_like_multi_value_is_okay() {
    let mut t = BuildViewTestCase::new();
    t.scratch.file(
        "test/build_settings.bzl",
        &[r#"def _impl(ctx):
    return []

string_flag = rule(
    implementation = _impl,
    build_setting = config.string(flag = True),
)
"#],
    );
    t.scratch.file(
        "test/BUILD",
        &[r#"load("//test:build_settings.bzl", "string_flag")

string_flag(
    name = "gouda",
    build_setting_default = "smoked,fresh",
)

config_setting(
    name = "match",
    flag_values = {
        ":gouda": "smoked,fresh",
    },
)

filegroup(
    name = "fg",
    srcs = select({
        ":match": [],
    }),
)
"#],
    );
    assert!(t.get_configured_target("//test:fg").is_some());
    t.assert_no_events();
}

// Java: ConfigSettingTest.labelInValuesError
#[test]
fn label_in_values_error() {
    let mut t = BuildViewTestCase::new();
    t.scratch.file(
        "test/BUILD",
        &[r#"config_setting(
    name = "match",
    values = {"//foo:bar": "value"},
)
"#],
    );
    t.remove_fail_fast_handler(); // expect errors
    assert!(t.get_configured_target("//test:match").is_none());
    t.assert_contains_event(
        "in values attribute of config_setting rule //test:match: '//foo:bar' is \
         not a valid setting name, but appears to be a label. Did you mean to place it in \
         flag_values instead?",
    );
}

// Java: ConfigSettingTest.selectOnDeprecatedFlagEmitsWarning
fn select_on_deprecated_flag_emits_warning(flag: &str) {
    let mut t = BuildViewTestCase::new();
    t.scratch.file(
        "test/BUILD",
        &[&format!(
            r#"config_setting(
    name = "match",
    values = {{
      "{flag}": "//foo",
    }},
)
"#
        )],
    );
    // empty --incompatible_disable_select_on to get the warning.
    t.use_configuration(&["--incompatible_disable_select_on="]);
    assert!(t.get_configured_target("//test:match").is_some());
    t.assert_contains_event(&format!(
        "select() on {flag} is deprecated. Use platform constraints instead"
    ));
}

// Java: ConfigSettingTest.selectOnDeprecatedFlagEmitsWarning {flag: cpu}
#[test]
fn select_on_deprecated_flag_emits_warning_cpu() {
    select_on_deprecated_flag_emits_warning("cpu");
}

// Java: ConfigSettingTest.selectOnDeprecatedFlagEmitsWarning {flag: host_cpu}
#[test]
fn select_on_deprecated_flag_emits_warning_host_cpu() {
    select_on_deprecated_flag_emits_warning("host_cpu");
}

// Java: ConfigSettingTest.selectOnDeprecatedFlagEmitsWarning {flag: crosstool_top}
#[test]
fn select_on_deprecated_flag_emits_warning_crosstool_top() {
    select_on_deprecated_flag_emits_warning("crosstool_top");
}

// Java: ConfigSettingTest.selectOnDisabledFlagFails
fn select_on_disabled_flag_fails(flag: &str) {
    let mut t = BuildViewTestCase::new();
    t.scratch.file(
        "test/BUILD",
        &[&format!(
            r#"config_setting(
    name = "match",
    values = {{
      "{flag}": "//foo",
    }},
)
"#
        )],
    );
    t.use_configuration(&[&format!("--incompatible_disable_select_on={flag}")]);
    t.remove_fail_fast_handler();
    assert!(t.get_configured_target("//test:match").is_none());
    t.assert_contains_event(&format!(
        "in values attribute of config_setting rule //test:match: error while parsing \
         configuration settings: select() on '{flag}' is not allowed. Use platform constraints \
         instead"
    ));
}

// Java: ConfigSettingTest.selectOnDisabledFlagFails {flag: cpu}
#[test]
fn select_on_disabled_flag_fails_cpu() {
    select_on_disabled_flag_fails("cpu");
}

// Java: ConfigSettingTest.selectOnDisabledFlagFails {flag: host_cpu}
#[test]
fn select_on_disabled_flag_fails_host_cpu() {
    select_on_disabled_flag_fails("host_cpu");
}

// Java: ConfigSettingTest.selectOnDisabledFlagFails {flag: crosstool_top}
#[test]
fn select_on_disabled_flag_fails_crosstool_top() {
    select_on_disabled_flag_fails("crosstool_top");
}

// Java: ConfigSettingTest.selectDisabledOnNonPlatformsFlag
#[test]
fn select_disabled_on_non_platforms_flag() {
    let mut t = BuildViewTestCase::new();
    t.scratch.file(
        "test/BUILD",
        &[r#"config_setting(
    name = "match",
    values = {
      "compilation_mode": "opt",
    },
)
"#],
    );
    t.use_configuration(&["--incompatible_disable_select_on=compilation_mode"]);
    t.remove_fail_fast_handler();
    assert!(t.get_configured_target("//test:match").is_none());
    t.assert_contains_event(
        "in values attribute of config_setting rule //test:match: error while parsing \
         configuration settings: select() on 'compilation_mode' is not allowed.",
    );
    t.assert_does_not_contain_event("Use platform constraints instead");
}

// If --foo has oldName --old_foo, let disabling flag selection have fine-grained control over
// which name is permitted. For example, if we want to force all config_settings to use the new
// name, we could set --incompatible_disable-select_on=old_foo.
// Java: ConfigSettingTest.selectOnDisabledFlagwithOldName
fn select_on_disabled_flagwith_old_name(
    config_setting_name: &str,
    disabled_name: &str,
    expect_success: bool,
) {
    let mut t = BuildViewTestCase::new();
    t.scratch.file(
        "test/BUILD",
        &[&format!(
            r#"config_setting(
    name = "match",
    values = {{
      "{config_setting_name}": "//foo",
    }},
)
"#
        )],
    );
    t.use_configuration(&[&format!("--incompatible_disable_select_on={disabled_name}")]);
    t.remove_fail_fast_handler();

    if expect_success {
        assert!(t.get_configured_target("//test:match").is_some());
        t.assert_no_events();
    } else {
        assert!(t.get_configured_target("//test:match").is_none());
        t.assert_contains_event(&format!(
            "in values attribute of config_setting rule //test:match: error while parsing \
             configuration settings: select() on '{disabled_name}' is not allowed."
        ));
    }
}

// Java: ConfigSettingTest.selectOnDisabledFlagwithOldName {configSettingName: new_option_name, disabledName: new_option_name, expectSuccess: false}
#[test]
#[ignore = "needs DummyTestOptionsFragment (the test's custom rule class provider registering DummyTestOptions' --new_option_name/--old_option_name)"]
fn select_on_disabled_flagwith_old_name_new_disabled_new() {
    select_on_disabled_flagwith_old_name("new_option_name", "new_option_name", false);
}

// Java: ConfigSettingTest.selectOnDisabledFlagwithOldName {configSettingName: old_option_name, disabledName: old_option_name, expectSuccess: false}
#[test]
#[ignore = "needs DummyTestOptionsFragment (the test's custom rule class provider registering DummyTestOptions' --new_option_name/--old_option_name)"]
fn select_on_disabled_flagwith_old_name_old_disabled_old() {
    select_on_disabled_flagwith_old_name("old_option_name", "old_option_name", false);
}

// Java: ConfigSettingTest.selectOnDisabledFlagwithOldName {configSettingName: new_option_name, disabledName: old_option_name, expectSuccess: true}
#[test]
#[ignore = "needs DummyTestOptionsFragment (the test's custom rule class provider registering DummyTestOptions' --new_option_name/--old_option_name)"]
fn select_on_disabled_flagwith_old_name_new_disabled_old() {
    select_on_disabled_flagwith_old_name("new_option_name", "old_option_name", true);
}

// Java: ConfigSettingTest.selectOnDisabledFlagwithOldName {configSettingName: old_option_name, disabledName: new_option_name, expectSuccess: true}
#[test]
#[ignore = "needs DummyTestOptionsFragment (the test's custom rule class provider registering DummyTestOptions' --new_option_name/--old_option_name)"]
fn select_on_disabled_flagwith_old_name_old_disabled_new() {
    select_on_disabled_flagwith_old_name("old_option_name", "new_option_name", true);
}

// Java: ConfigSettingTest.flagAlias
fn flag_alias(
    values_attr: &str,
    flag_values_attr: &str,
    config: &[&str],
    expect_match: bool,
    expected_error: &str,
) {
    let mut t = BuildViewTestCase::new();
    t.scratch.file(
        "test/build_settings.bzl",
        &[r#"def _impl(ctx):
    return []
string_flag = rule(implementation = _impl, build_setting = config.string(flag = True))
"#],
    );
    t.scratch.file(
        "test/BUILD",
        &[&format!(
            r#"load("//test:build_settings.bzl", "string_flag")
config_setting(
    name = "match",
    values = {{{values_attr}}},
    flag_values = {{{flag_values_attr}}}
)
string_flag(
    name = "myflag",
    build_setting_default = "parmesan",
)
"#
        )],
    );

    t.remove_fail_fast_handler();
    t.use_configuration(config);

    if expected_error.is_empty() {
        assert_eq!(
            get_config_matching_provider_result_as_boolean(&mut t, "//test:match"),
            expect_match
        );
    } else {
        assert!(t.get_configured_target("//test:match").is_none());
        t.assert_contains_event(expected_error);
    }
}

// Alias --nativeform to --//test:myflag, config_setting matches --nativeform on default value.
// Java: ConfigSettingTest.flagAlias (first parameter set)
#[test]
fn flag_alias_matches_default_value() {
    flag_alias(
        r#""nativeform": "parmesan""#,
        "",
        &["--flag_alias=nativeform=//test:myflag"],
        true,
        "",
    );
}

// Alias --nativeform to --//test:myflag, config_setting doesn't match --nativeform because it
// expects a different value.
// Java: ConfigSettingTest.flagAlias (second parameter set)
#[test]
fn flag_alias_expects_other_value() {
    flag_alias(
        r#""nativeform": "other_expected_value""#,
        "",
        &["--flag_alias=nativeform=//test:myflag"],
        false,
        "",
    );
}

// Alias --nativeform to --//test:doesnt_exist, config_setting on --nativeform fails because the
// target doesn't exist.
// Java: ConfigSettingTest.flagAlias (third parameter set)
#[test]
fn flag_alias_target_doesnt_exist() {
    flag_alias(
        r#""nativeform": "parmesan""#,
        "",
        &["--flag_alias=nativeform=//test:doesnt_exist"],
        false,
        "target 'doesnt_exist' not declared",
    );
}

// Alias --nativeform not set, config_setting on --nativeform errors.
// Java: ConfigSettingTest.flagAlias (fourth parameter set)
#[test]
fn flag_alias_not_set() {
    flag_alias(
        r#""nativeform": "parmesan""#,
        "",
        &[],
        false,
        "unknown option: 'nativeform'",
    );
}

// config_setting reads both alias and actual flags and matches because their value is the same.
// Java: ConfigSettingTest.flagAlias (fifth parameter set)
#[test]
fn flag_alias_and_actual_flag_match() {
    flag_alias(
        r#""nativeform": "parmesan""#,
        r#""//test:myflag": "parmesan""#,
        &["--flag_alias=nativeform=//test:myflag"],
        true,
        "",
    );
}

// config_setting reads both alias and actual flags and errors because of mismatching values.
// Java: ConfigSettingTest.flagAlias (sixth parameter set)
#[test]
fn flag_alias_and_actual_flag_conflict() {
    flag_alias(
        r#""nativeform": "parmesan""#,
        r#""//test:myflag": "other_value""#,
        &["--flag_alias=nativeform=//test:myflag"],
        false,
        "Conflicting flag value expectations",
    );
}

// Java: ConfigSettingTest.nonConfigurableOption
#[test]
#[ignore = "needs DummyTestOptionsFragment (the test's custom rule class provider registering DummyTestOptions' --non_configurable_option)"]
fn non_configurable_option() {
    let mut t = BuildViewTestCase::new();
    t.check_error(
        "foo",
        "non_configurable_option",
        "select() on 'non_configurable_option' is not allowed.",
        &[r#"config_setting(
    name = "non_configurable_option",
    values = {"non_configurable_option": "foo"},
)
"#],
    );
}

// Java: ConfigSettingTest.stringSet_singleValue_works
fn string_set_single_value_works(flag_default: bool) {
    let mut t = BuildViewTestCase::new();
    let match_value = if flag_default {
        "default_value"
    } else {
        "cmd_val"
    };
    t.scratch.file(
        "test/build_settings.bzl",
        &[r#"def _impl(ctx):
    return []

string_set_flag = rule(
    implementation = _impl,
    build_setting = config.string_set(flag = True),
)
"#],
    );
    t.scratch.file(
        "test/BUILD",
        &[&format!(
            r#"load("//test:build_settings.bzl", "string_set_flag")

config_setting(
    name = "match",
    flag_values = {{
        ":my_flag": "{match_value}",
    }},
)

config_setting(
    name = "no_match",
    flag_values = {{
        ":my_flag": "another_value",
    }},
)

string_set_flag(
    name = "my_flag",
    build_setting_default = set(["default_value"]),
)
"#
        )],
    );

    if !flag_default {
        t.use_configuration(&["--//test:my_flag=cmd_val"]);
    }
    assert!(get_config_matching_provider_result_as_boolean(
        &mut t,
        "//test:match"
    ));
    assert!(!get_config_matching_provider_result_as_boolean(
        &mut t,
        "//test:no_match"
    ));
}

// Java: ConfigSettingTest.stringSet_singleValue_works (flagDefault = true)
#[test]
fn string_set_single_value_works_flag_default_true() {
    string_set_single_value_works(true);
}

// Java: ConfigSettingTest.stringSet_singleValue_works (flagDefault = false)
#[test]
fn string_set_single_value_works_flag_default_false() {
    string_set_single_value_works(false);
}

// Java: ConfigSettingTest.stringSet_multipleValues_works
fn string_set_multiple_values_works(values_src: &str) {
    let mut t = BuildViewTestCase::new();
    let default_value = if values_src == "default" {
        "'v2', 'v1', 'v3'"
    } else {
        "'default_value'"
    };
    t.scratch.file(
        "test/build_settings.bzl",
        &[&format!(
            r#"def _impl(ctx):
    return []

string_set_flag = rule(
    implementation = _impl,
    build_setting = config.string_set(flag = True, repeatable = {}),
)
"#,
            if values_src == "repeatable" {
                "True"
            } else {
                "False"
            }
        )],
    );
    t.scratch.file(
        "test/BUILD",
        &[&format!(
            r#"load("//test:build_settings.bzl", "string_set_flag")

config_setting(
    name = "match_2",
    flag_values = {{
        ":my_flag": "v2",
    }},
)

config_setting(
    name = "match_3",
    flag_values = {{
        ":my_flag": "v3",
    }},
)

config_setting(
    name = "match_4",
    flag_values = {{
        ":my_flag": "v4",
    }},
)

string_set_flag(
    name = "my_flag",
    build_setting_default = set([{default_value}]),
)
"#
        )],
    );

    if values_src == "multiple" {
        t.use_configuration(&["--//test:my_flag=v2,v1,v3"]);
    } else if values_src == "repeatable" {
        t.use_configuration(&[
            "--//test:my_flag=v3",
            "--//test:my_flag=v1",
            "--//test:my_flag=v2",
        ]);
    }
    assert!(get_config_matching_provider_result_as_boolean(
        &mut t,
        "//test:match_2"
    ));
    assert!(get_config_matching_provider_result_as_boolean(
        &mut t,
        "//test:match_3"
    ));
    assert!(!get_config_matching_provider_result_as_boolean(
        &mut t,
        "//test:match_4"
    ));
}

// Java: ConfigSettingTest.stringSet_multipleValues_works (valuesSrc = "default")
#[test]
fn string_set_multiple_values_works_default() {
    string_set_multiple_values_works("default");
}

// Java: ConfigSettingTest.stringSet_multipleValues_works (valuesSrc = "multiple")
#[test]
fn string_set_multiple_values_works_multiple() {
    string_set_multiple_values_works("multiple");
}

// Java: ConfigSettingTest.stringSet_multipleValues_works (valuesSrc = "repeatable")
#[test]
fn string_set_multiple_values_works_repeatable() {
    string_set_multiple_values_works("repeatable");
}

// Java: ConfigSettingTest.canOnlyMatchSingleValueWithSetFlags
fn can_only_match_single_value_with_set_flags(repeatable: bool) {
    let mut t = BuildViewTestCase::new();
    t.scratch.file(
        "test/build_settings.bzl",
        &[&format!(
            r#"def _impl(ctx):
    return []

string_set_flag = rule(
    implementation = _impl,
    build_setting = config.string_set(flag = True, repeatable = {}),
)
"#,
            if repeatable { "True" } else { "False" }
        )],
    );
    t.scratch.file(
        "test/BUILD",
        &[r#"load("//test:build_settings.bzl", "string_set_flag")

string_set_flag(
    name = "my_flag",
    build_setting_default = set(["default_value"]),
)

config_setting(
    name = "match",
    flag_values = {
        ":my_flag": "v1,v2",
    },
)

filegroup(
    name = "fg",
    srcs = select({
        ":match": [],
    }),
)
"#],
    );
    t.remove_fail_fast_handler(); // expect errors
    assert!(t.get_configured_target("//test:fg").is_none());
    t.assert_contains_event(
        "\"v1,v2\" not a valid value for flag //test:my_flag. \
         Only single, exact values are allowed",
    );
}

// Java: ConfigSettingTest.canOnlyMatchSingleValueWithSetFlags (repeatable = true)
#[test]
fn can_only_match_single_value_with_set_flags_repeatable_true() {
    can_only_match_single_value_with_set_flags(true);
}

// Java: ConfigSettingTest.canOnlyMatchSingleValueWithSetFlags (repeatable = false)
#[test]
fn can_only_match_single_value_with_set_flags_repeatable_false() {
    can_only_match_single_value_with_set_flags(false);
}
