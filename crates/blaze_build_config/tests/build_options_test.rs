//! Port of `src/test/java/com/google/devtools/build/lib/analysis/config/BuildOptionsTest.java`.
//!
//! Most tests of the Java class use its test-only `DummyTestOptions`/`SecondDummyTestOptions`
//! fragment options classes, or Bazel's serialization framework. Those are kept as ignored tests.

use blaze_build_config::build_options::{BuildOptions, ScopeType, StarlarkOptionValue};
use blaze_cmdline::Label;
use blaze_options::{OptionsParser, Value};

/// A parser whose Starlark options are `entries` (Java `parser.setStarlarkOptions(Map.of(...))`).
///
/// PORT: the Java tests build the parser with `BUILD_CONFIG_OPTIONS` (the test-only
/// `DummyTestOptions`); the tests ported with this helper parse no native option, so the options
/// class does not matter.
fn parser_with_starlark_options(entries: &[(&str, &str)]) -> OptionsParser {
    let mut parser = OptionsParser::builder().build().unwrap();
    parser.set_starlark_options(
        entries
            .iter()
            .map(|(k, v)| (k.to_string(), Value::new(v.to_string())))
            .collect(),
    );
    parser
}

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
fn parsing_result_match_starlark() {
    let original = BuildOptions::builder()
        .add_starlark_option(
            Label::parse_canonical_unchecked("//custom:flag"),
            StarlarkOptionValue::Str("hello".to_string()),
        )
        .build();

    let matching_parser = parser_with_starlark_options(&[("//custom:flag", "hello")]);
    let not_matching_parser = parser_with_starlark_options(&[("//custom:flag", "foo")]);

    assert!(original.matches(&matching_parser).unwrap());
    assert!(!original.matches(&not_matching_parser).unwrap());
}

#[test]
#[ignore = "needs test-only FragmentOptions class (BuildOptionsTest.DummyTestOptions, SecondDummyTestOptions)"]
fn parsing_result_match_missing_fragment() {}

#[test]
#[ignore = "needs test-only FragmentOptions class (BuildOptionsTest.DummyTestOptions, SecondDummyTestOptions)"]
fn parsing_result_match_empty_native_match() {}

#[test]
#[ignore = "needs test-only FragmentOptions class (BuildOptionsTest.DummyTestOptions, SecondDummyTestOptions): the parser parses --second_str_option"]
fn parsing_result_match_empty_native_match_with_starlark() {}

#[test]
fn parsing_result_match_starlark_option_missing() {
    let original = BuildOptions::builder()
        .add_starlark_option(
            Label::parse_canonical_unchecked("//custom:flag1"),
            StarlarkOptionValue::Str("hello".to_string()),
        )
        .build();

    let parser = parser_with_starlark_options(&[("//custom:flag2", "foo")]);

    assert!(!original.matches(&parser).unwrap());
}

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

fn strs(v: &[&str]) -> Vec<StarlarkOptionValue> {
    v.iter()
        .map(|e| StarlarkOptionValue::Str(e.to_string()))
        .collect()
}

// Blaze-only: a Starlark option value keeps the `toString()` of its Java class (analysis review
// S2-2). Expected strings: `AbstractCollection.toString` for ImmutableList/ArrayList and sets,
// `Starlark.repr` for StarlarkList (net/starlark/java/eval/StarlarkList.java:272) and Tuple
// (net/starlark/java/eval/Tuple.java:139), `Label.repr` (cmdline/Label.java:718); the oracle's
// `config` output prints `//:multi: [m0]` for the list of a command line and
// `//:multi: ["m0"]` for the list a transition returned.
#[test]
fn starlark_option_values_print_like_their_java_class() {
    let elems = strs(&["a", "b c", "d\"e"]);
    assert_eq!(
        StarlarkOptionValue::list(elems.clone()).java_to_string(),
        "[a, b c, d\"e]"
    );
    assert_eq!(
        StarlarkOptionValue::set(elems.clone()).java_to_string(),
        "[a, b c, d\"e]"
    );
    assert_eq!(
        StarlarkOptionValue::starlark_list(elems.clone()).java_to_string(),
        "[\"a\", \"b c\", \"d\\\"e\"]"
    );
    assert_eq!(
        StarlarkOptionValue::tuple(elems).java_to_string(),
        "(\"a\", \"b c\", \"d\\\"e\")"
    );
    assert_eq!(
        StarlarkOptionValue::tuple(strs(&["a"])).java_to_string(),
        "(\"a\",)"
    );
    assert_eq!(
        StarlarkOptionValue::starlark_list(vec![
            StarlarkOptionValue::Bool(true),
            StarlarkOptionValue::Int(-3),
            StarlarkOptionValue::Label(Label::parse_canonical_unchecked("//pkg:t")),
        ])
        .java_to_string(),
        "[True, -3, Label(\"//pkg:t\")]"
    );
    assert_eq!(StarlarkOptionValue::Bool(true).java_to_string(), "true");
}

// Blaze-only: `equals` across the Java collection classes. `ImmutableList.equals` accepts any
// `java.util.List` with equal elements; `StarlarkList.equals` (StarlarkList.java:244) and
// `Tuple.equals` (Tuple.java:129) require their own class; a `Set` never equals a `List`.
#[test]
fn starlark_option_values_compare_like_their_java_class() {
    let list = StarlarkOptionValue::list(strs(&["a", "b"]));
    let starlark_list = StarlarkOptionValue::starlark_list(strs(&["a", "b"]));
    let tuple = StarlarkOptionValue::tuple(strs(&["a", "b"]));
    let set = StarlarkOptionValue::set(strs(&["a", "b"]));
    let reversed_set = StarlarkOptionValue::set(strs(&["b", "a"]));

    assert!(list.java_equals(&starlark_list));
    assert!(list.java_equals(&tuple));
    assert!(!starlark_list.java_equals(&list));
    assert!(starlark_list.java_equals(&starlark_list.clone()));
    assert!(!starlark_list.java_equals(&tuple));
    assert!(!tuple.java_equals(&list));
    assert!(!list.java_equals(&set));
    assert!(!set.java_equals(&list));
    assert!(set.java_equals(&reversed_set));
    assert!(!list.java_equals(&StarlarkOptionValue::list(strs(&["b", "a"]))));
    assert!(!StarlarkOptionValue::Str("a".to_string()).java_equals(&list));
    // `==` is structural: the class is part of the value.
    assert_ne!(list, starlark_list);
}

// BuildOptions.of(optionsList, provider): labelizeStarlarkOptions and convertScopesAttributes
// (analysis/config/BuildOptions.java:76 and :87). The scope of a flag that is not in the
// Starlark options map (a flag set to its default) is dropped; scope names are matched ignoring
// case (Scope.ScopeType.valueOfIgnoreCase).
#[test]
fn of_takes_starlark_options_and_scopes_from_the_provider() {
    let mut parser = OptionsParser::builder().build().unwrap();
    parser.set_starlark_options(
        [
            (
                "//pkg:b".to_string(),
                Value::new(StarlarkOptionValue::Int(7)),
            ),
            (
                "@@repo+//pkg:a".to_string(),
                Value::new(StarlarkOptionValue::list(strs(&["x"]))),
            ),
        ]
        .into_iter()
        .collect(),
    );
    parser.set_scopes_attributes(
        [
            ("//pkg:b".to_string(), "universal".to_string()),
            ("@@repo+//pkg:a".to_string(), "DEFAULT".to_string()),
            ("//pkg:at_default".to_string(), "target".to_string()),
        ]
        .into_iter()
        .collect(),
    );

    let options = BuildOptions::of(&[], &parser);

    let starlark: Vec<(String, String)> = options
        .get_starlark_options()
        .iter()
        .map(|(k, v)| (k.to_string(), v.java_to_string()))
        .collect();
    assert_eq!(
        starlark,
        vec![
            ("//pkg:b".to_string(), "7".to_string()),
            ("@@repo+//pkg:a".to_string(), "[x]".to_string()),
        ]
    );
    let scopes: Vec<(String, ScopeType)> = options
        .get_scope_type_map()
        .iter()
        .map(|(k, v)| (k.to_string(), *v))
        .collect();
    assert_eq!(
        scopes,
        vec![
            ("//pkg:b".to_string(), ScopeType::UNIVERSAL),
            ("@@repo+//pkg:a".to_string(), ScopeType::DEFAULT),
        ]
    );
}

// Scope.ScopeType.allowedAttributeValues (analysis/config/Scope.java:54); the oracle prints them
// in the error of flags-probe case 48: `Allowed values: ["universal", "target", "project"].`
#[test]
fn scope_type_allowed_attribute_values() {
    assert_eq!(
        ScopeType::allowed_attribute_values(),
        vec!["universal", "target", "project"]
    );
}
