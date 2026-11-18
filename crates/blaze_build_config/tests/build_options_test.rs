//! Port of `src/test/java/com/google/devtools/build/lib/analysis/config/BuildOptionsTest.java`.
//!
//! Most tests of the Java class use its test-only `DummyTestOptions`/`SecondDummyTestOptions`
//! fragment options classes, or Bazel's serialization framework. Those are kept as ignored tests.

use blaze_build_config::build_options::{BuildOptions, StarlarkOptionValue};
use blaze_cmdline::Label;

#[test]
#[ignore = "needs test-only FragmentOptions class (BuildOptionsTest.DummyTestOptions)"]
fn option_set_caching() {}

#[test]
#[ignore = "needs test-only FragmentOptions class (BuildOptionsTest.DummyTestOptions, SecondDummyTestOptions)"]
fn options_equality() {}

#[test]
#[ignore = "needs serialization (SerializationTester, OptionsChecksumCache) and test-only FragmentOptions class"]
fn serialization() {}

#[test]
#[ignore = "needs serialization (ObjectCodecs, OptionsChecksumCache) and test-only FragmentOptions class"]
fn serialize_prime_fails_throws() {}

#[test]
#[ignore = "needs serialization (ObjectCodecs, MapBackedChecksumCache) and test-only FragmentOptions class"]
fn deserialize_unprimed_cache_throws() {}

#[test]
#[ignore = "needs serialization (ObjectCodecs, MapBackedChecksumCache) and test-only FragmentOptions class"]
fn deserialize_primed_cache_returns_primed_instance() {}

#[test]
#[ignore = "needs test-only FragmentOptions class (BuildOptionsTest.DummyTestOptions)"]
fn test_multi_value_option_immutability() {}

#[test]
#[ignore = "needs test-only FragmentOptions class (BuildOptionsTest.DummyTestOptions)"]
fn parsing_result_match() {}

#[test]
#[ignore = "needs test-only FragmentOptions class and OptionsParser.setStarlarkOptions (BuildOptions.matches with Starlark options is unported)"]
fn parsing_result_match_starlark() {}

#[test]
#[ignore = "needs test-only FragmentOptions class (BuildOptionsTest.DummyTestOptions, SecondDummyTestOptions)"]
fn parsing_result_match_missing_fragment() {}

#[test]
#[ignore = "needs test-only FragmentOptions class (BuildOptionsTest.DummyTestOptions, SecondDummyTestOptions)"]
fn parsing_result_match_empty_native_match() {}

#[test]
#[ignore = "needs test-only FragmentOptions class and OptionsParser.setStarlarkOptions (BuildOptions.matches with Starlark options is unported)"]
fn parsing_result_match_empty_native_match_with_starlark() {}

#[test]
#[ignore = "needs test-only FragmentOptions class and OptionsParser.setStarlarkOptions (BuildOptions.matches with Starlark options is unported)"]
fn parsing_result_match_starlark_option_missing() {}

#[test]
#[ignore = "needs test-only FragmentOptions class and OptionsParser.setStarlarkOptions"]
fn parsing_result_match_null_option() {}

#[test]
#[ignore = "needs test-only FragmentOptions class (BuildOptionsTest.DummyTestOptions, SecondDummyTestOptions)"]
fn native_options_ordered_lexicographically() {}

#[test]
fn starlark_options_ordered_by_label() {
    let label1 = Label::parse_canonical_unchecked("//pkg:option1");
    let label2 = Label::parse_canonical_unchecked("//pkg:option2");

    let forward = BuildOptions::builder()
        .add_starlark_option(label1.clone(), StarlarkOptionValue::Bool(true))
        .add_starlark_option(label2.clone(), StarlarkOptionValue::Bool(false))
        .build();
    let backward = BuildOptions::builder()
        .add_starlark_option(label2.clone(), StarlarkOptionValue::Bool(false))
        .add_starlark_option(label1.clone(), StarlarkOptionValue::Bool(true))
        .build();

    let expected: Vec<(Label, StarlarkOptionValue)> = vec![
        (label1.clone(), StarlarkOptionValue::Bool(true)),
        (label2.clone(), StarlarkOptionValue::Bool(false)),
    ];
    let forward_entries: Vec<(Label, StarlarkOptionValue)> = forward
        .get_starlark_options()
        .iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    let backward_entries: Vec<(Label, StarlarkOptionValue)> = backward
        .get_starlark_options()
        .iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    assert_eq!(forward_entries, expected);
    assert_eq!(backward_entries, expected);
}
