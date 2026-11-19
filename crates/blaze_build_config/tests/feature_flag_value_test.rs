//! Port of `src/test/java/com/google/devtools/build/lib/rules/config/FeatureFlagValueTest.java`.

use blaze_build_config::build_options::{BuildOptions, StarlarkOptionValue};
use blaze_build_config::feature_flag_value::FeatureFlagValue;
use blaze_build_config::fragment_options_class::CONFIG_FEATURE_FLAG_OPTIONS;
use blaze_cmdline::Label;
use blaze_optdefs::fragment::config_feature_flag_options::ConfigFeatureFlagOptions;
use indexmap::{IndexMap, IndexSet};

fn label(s: &str) -> Label {
    Label::parse_canonical_unchecked(s)
}

fn empty_build_options() -> BuildOptions {
    BuildOptions::of_args(&[&CONFIG_FEATURE_FLAG_OPTIONS], &[]).expect("empty options")
}

fn get_known_default_flags(options: &BuildOptions) -> IndexSet<Label> {
    options
        .get_starlark_options()
        .iter()
        .filter(|(_, v)| **v == StarlarkOptionValue::FeatureFlag(FeatureFlagValue::DefaultValue))
        .map(|(k, _)| k.clone())
        .collect()
}

fn string_map(entries: &[(&str, &str)]) -> IndexMap<Label, String> {
    entries
        .iter()
        .map(|(k, v)| (label(k), (*v).to_owned()))
        .collect()
}

fn labels(entries: &[&str]) -> Vec<Label> {
    entries.iter().map(|s| label(s)).collect()
}

fn set_value(v: &str) -> StarlarkOptionValue {
    StarlarkOptionValue::FeatureFlag(FeatureFlagValue::set_value_of(v))
}

fn default_value() -> StarlarkOptionValue {
    StarlarkOptionValue::FeatureFlag(FeatureFlagValue::DefaultValue)
}

fn unknown_value() -> StarlarkOptionValue {
    StarlarkOptionValue::FeatureFlag(FeatureFlagValue::UnknownValue)
}

/// Truth `containsExactly` on a map (order-independent).
fn assert_contains_exactly(
    actual: &IndexMap<Label, StarlarkOptionValue>,
    expected: &[(&str, StarlarkOptionValue)],
) {
    let expected: IndexMap<Label, StarlarkOptionValue> = expected
        .iter()
        .map(|(k, v)| (label(k), v.clone()))
        .collect();
    assert_eq!(actual.len(), expected.len(), "{actual:?} vs {expected:?}");
    for (k, v) in &expected {
        assert_eq!(actual.get(k), Some(v), "{actual:?} vs {expected:?}");
    }
}

fn all_present(options: &BuildOptions) -> bool {
    options
        .get::<ConfigFeatureFlagOptions>()
        .unwrap()
        .all_feature_flag_values_are_present
}

fn with_unrelated_option() -> BuildOptions {
    empty_build_options()
        .to_builder()
        .add_starlark_option(
            label("//unrelated/starlark:option"),
            StarlarkOptionValue::Bool(true),
        )
        .build()
}

#[test]
fn replace_flag_values_reflected_in_get_flag_values() {
    let options = FeatureFlagValue::replace_flag_values(
        &empty_build_options(),
        &string_map(&[("//label:a", "value"), ("//label:b", "otherValue")]),
    );
    assert_contains_exactly(
        options.get_starlark_options(),
        &[
            ("//label:a", set_value("value")),
            ("//label:b", set_value("otherValue")),
        ],
    );
}

#[test]
fn replace_flag_values_totally_replaces_flag_values_map() {
    let mut options = empty_build_options();
    options = FeatureFlagValue::replace_flag_values(
        &options,
        &string_map(&[("//label:a", "value"), ("//label:b", "otherValue")]),
    );
    options = FeatureFlagValue::replace_flag_values(
        &options,
        &string_map(&[
            ("//label:a", "differentValue"),
            ("//label:c", "differentFlag"),
        ]),
    );
    assert_contains_exactly(
        options.get_starlark_options(),
        &[
            ("//label:a", set_value("differentValue")),
            ("//label:c", set_value("differentFlag")),
        ],
    );
}

#[test]
fn replace_flag_values_empties_known_default_flags_and_unknown_flags() {
    let original_map = string_map(&[("//label:a", "value"), ("//label:b", "otherValue")]);
    let mut options = empty_build_options();
    options = FeatureFlagValue::replace_flag_values(&options, &original_map);
    options = FeatureFlagValue::trim_flag_values(
        &options,
        &labels(&["//label:a", "//label:c", "//label:d"]),
    );
    options = FeatureFlagValue::replace_flag_values(&options, &original_map);
    assert!(all_present(&options));
}

#[test]
fn replace_flag_values_leaves_non_feature_flag_values_alone() {
    let original_map = string_map(&[("//label:a", "value"), ("//label:b", "otherValue")]);
    let new_map = string_map(&[
        ("//label:a", "differentValue"),
        ("//label:c", "differentFlag"),
    ]);
    let mut options = with_unrelated_option();
    options = FeatureFlagValue::replace_flag_values(&options, &original_map);
    options = FeatureFlagValue::replace_flag_values(&options, &new_map);
    assert_eq!(
        options
            .get_starlark_options()
            .get(&label("//unrelated/starlark:option")),
        Some(&StarlarkOptionValue::Bool(true))
    );
}

#[test]
fn trim_flag_values_defaults_to_empty_set_produces_empty_options() {
    let mut options = empty_build_options();

    options = FeatureFlagValue::trim_flag_values(&options, &labels(&[]));

    assert!(options.get_starlark_options().is_empty());
    assert!(!all_present(&options));
    assert!(get_known_default_flags(&options).is_empty());
}

#[test]
fn trim_flag_values_defaults_to_populated_set_populates_known_default_flags() {
    let mut options = empty_build_options();

    options = FeatureFlagValue::trim_flag_values(
        &options,
        &labels(&["//label:a", "//label:b", "//label:c"]),
    );

    assert_contains_exactly(
        options.get_starlark_options(),
        &[
            ("//label:a", default_value()),
            ("//label:b", default_value()),
            ("//label:c", default_value()),
        ],
    );
    assert!(!all_present(&options));
    assert_eq!(
        get_known_default_flags(&options),
        labels(&["//label:a", "//label:b", "//label:c"])
            .into_iter()
            .collect::<IndexSet<_>>()
    );
}

#[test]
fn trim_flag_values_with_flags_set_to_empty_set_produces_empty_options() {
    let mut options = empty_build_options();
    options = FeatureFlagValue::replace_flag_values(
        &options,
        &string_map(&[("//label:a", "value"), ("//label:d", "otherValue")]),
    );

    options = FeatureFlagValue::trim_flag_values(&options, &labels(&[]));

    assert!(options.get_starlark_options().is_empty());
    assert!(!all_present(&options));
    assert!(get_known_default_flags(&options).is_empty());
}

#[test]
fn trim_flag_values_with_flags_set_to_populated_set_populates_flag_values_and_known_default_flags()
{
    let mut options = empty_build_options();
    options = FeatureFlagValue::replace_flag_values(
        &options,
        &string_map(&[("//label:a", "value"), ("//label:d", "otherValue")]),
    );

    options = FeatureFlagValue::trim_flag_values(
        &options,
        &labels(&["//label:a", "//label:b", "//label:c"]),
    );

    assert_contains_exactly(
        options.get_starlark_options(),
        &[
            ("//label:a", set_value("value")),
            ("//label:b", default_value()),
            ("//label:c", default_value()),
        ],
    );
    assert!(!all_present(&options));
    assert_eq!(
        get_known_default_flags(&options),
        labels(&["//label:b", "//label:c"])
            .into_iter()
            .collect::<IndexSet<_>>()
    );
}

#[test]
fn trim_flag_values_with_trimmed_flags_set_to_empty_set_produces_empty_options() {
    let mut options = empty_build_options();
    options = FeatureFlagValue::replace_flag_values(
        &options,
        &string_map(&[("//label:a", "value"), ("//label:d", "otherValue")]),
    );
    options = FeatureFlagValue::trim_flag_values(&options, &labels(&["//label:a", "//label:b"]));

    options = FeatureFlagValue::trim_flag_values(&options, &labels(&[]));

    assert!(options.get_starlark_options().is_empty());
    assert!(!all_present(&options));
    assert!(get_known_default_flags(&options).is_empty());
}

#[test]
fn trim_flag_values_with_trimmed_flags_set_to_populated_set_populates_flag_state() {
    let mut options = empty_build_options();
    options = FeatureFlagValue::replace_flag_values(
        &options,
        &string_map(&[("//label:a", "value"), ("//label:d", "otherValue")]),
    );
    options = FeatureFlagValue::trim_flag_values(&options, &labels(&["//label:a", "//label:b"]));

    options = FeatureFlagValue::trim_flag_values(
        &options,
        &labels(&["//label:a", "//label:b", "//label:c"]),
    );

    assert_contains_exactly(
        options.get_starlark_options(),
        &[
            ("//label:a", set_value("value")),
            ("//label:b", default_value()),
            ("//label:c", unknown_value()),
        ],
    );
}

#[test]
fn trim_flag_values_leaves_non_feature_flag_values_alone() {
    let mut options = with_unrelated_option();
    options = FeatureFlagValue::replace_flag_values(
        &options,
        &string_map(&[("//label:a", "value"), ("//label:d", "otherValue")]),
    );
    options = FeatureFlagValue::trim_flag_values(&options, &labels(&["//label:a", "//label:b"]));

    options = FeatureFlagValue::trim_flag_values(&options, &labels(&[]));

    assert_eq!(
        options
            .get_starlark_options()
            .get(&label("//unrelated/starlark:option")),
        Some(&StarlarkOptionValue::Bool(true))
    );
}

#[test]
fn trim_flag_values_overwrites_requested_non_feature_flag_value_with_default_if_untrimmed() {
    let mut options = with_unrelated_option();
    options = FeatureFlagValue::replace_flag_values(
        &options,
        &string_map(&[("//label:a", "value"), ("//label:d", "otherValue")]),
    );
    options = FeatureFlagValue::trim_flag_values(
        &options,
        &labels(&["//label:a", "//label:b", "//unrelated/starlark:option"]),
    );

    assert_eq!(
        options
            .get_starlark_options()
            .get(&label("//unrelated/starlark:option")),
        Some(&default_value())
    );
}

#[test]
fn trim_flag_values_overwrites_requested_non_feature_flag_value_with_unknown_if_trimmed() {
    let mut options = with_unrelated_option();
    options = FeatureFlagValue::replace_flag_values(
        &options,
        &string_map(&[("//label:a", "value"), ("//label:d", "otherValue")]),
    );
    options = FeatureFlagValue::trim_flag_values(&options, &labels(&["//label:a", "//label:b"]));
    options = FeatureFlagValue::trim_flag_values(
        &options,
        &labels(&["//label:a", "//label:b", "//unrelated/starlark:option"]),
    );

    assert_eq!(
        options
            .get_starlark_options()
            .get(&label("//unrelated/starlark:option")),
        Some(&unknown_value())
    );
}
