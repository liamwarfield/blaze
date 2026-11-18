//! Port of `src/test/java/com/google/devtools/build/lib/analysis/config/OptionsDiffTest.java`.

// OptionDefinition's OnceLock caches do not take part in Hash/Eq (as in blaze_options).
#![allow(clippy::mutable_key_type)]

use std::collections::HashSet;

use blaze_build_config::build_options::{BuildOptions, StarlarkOptionValue};
use blaze_build_config::fragment_options_class::{CORE_OPTIONS, CPP_OPTIONS, FragmentOptionsClass};
use blaze_build_config::options_diff::OptionsDiff;
use blaze_cmdline::Label;
use indexmap::IndexMap;

fn build_config_options() -> Vec<FragmentOptionsClass> {
    vec![&CORE_OPTIONS]
}

fn of(classes: &[FragmentOptionsClass], args: &[&str]) -> BuildOptions {
    BuildOptions::of_args(classes, args).unwrap_or_else(|e| panic!("{}", e.get_message()))
}

fn of_starlark(entries: &[(&Label, &str)]) -> BuildOptions {
    let map: IndexMap<Label, StarlarkOptionValue> = entries
        .iter()
        .map(|(k, v)| ((*k).clone(), StarlarkOptionValue::Str(v.to_string())))
        .collect();
    BuildOptions::of_starlark(map)
}

#[test]
fn diff() {
    let one = of(
        &build_config_options(),
        &["--compilation_mode=opt", "cpu=k8"],
    );
    let two = of(
        &build_config_options(),
        &["--compilation_mode=dbg", "cpu=k8"],
    );
    let three = of(
        &build_config_options(),
        &["--compilation_mode=dbg", "cpu=k8"],
    );

    let diff_one_two = OptionsDiff::diff(&one, &two);
    let diff_two_three = OptionsDiff::diff(&two, &three);

    assert!(!diff_one_two.are_same());
    let first_keys: HashSet<_> = diff_one_two.get_first().keys().collect();
    let second_keys: HashSet<_> = diff_one_two.get_second().keys().collect();
    assert!(first_keys == second_keys);
    assert!(diff_one_two.pretty_print().contains("opt"));
    assert!(diff_one_two.pretty_print().contains("dbg"));

    assert!(diff_two_three.are_same());
}

#[test]
fn diff_different_fragments() {
    let one = of(&[&CPP_OPTIONS], &[]);
    let two = of(&build_config_options(), &[]);

    let diff = OptionsDiff::diff(&one, &two);

    assert!(!diff.are_same());
    let expected_first: Vec<FragmentOptionsClass> = vec![&CPP_OPTIONS];
    assert_eq!(
        diff.get_extra_first_fragment_classes().to_vec(),
        expected_first
    );
    // PORT: Java's getExtraSecondFragmentsForTesting returns the FragmentOptions instances and the
    // test maps them to their classes; the Rust diff stores the classes.
    assert_eq!(
        diff.get_extra_second_fragments().to_vec(),
        build_config_options()
    );
}

#[test]
#[ignore = "not applicable: needs nullable BuildOptions (Rust references cannot be null, so OptionsDiff.diff cannot throw NullPointerException)"]
fn biff_null_options_throw() {}

#[test]
fn diff_same_starlark_options() {
    let flag_name = Label::parse_canonical_unchecked("//foo/flag");
    let flag_value = "value";
    let one = of_starlark(&[(&flag_name, flag_value)]);
    let two = of_starlark(&[(&flag_name, flag_value)]);

    assert!(OptionsDiff::diff(&one, &two).are_same());
}

#[test]
fn diff_different_starlark_options() {
    let flag_name = Label::parse_canonical_unchecked("//bar/flag");
    let flag_value_one = "valueOne";
    let flag_value_two = "valueTwo";
    let one = of_starlark(&[(&flag_name, flag_value_one)]);
    let two = of_starlark(&[(&flag_name, flag_value_two)]);

    let diff = OptionsDiff::diff(&one, &two);

    assert!(!diff.are_same());
    let first_keys: HashSet<&Label> = diff.get_starlark_first_for_testing().keys().collect();
    let second_keys: HashSet<&Label> = diff.get_starlark_second_for_testing().keys().collect();
    assert_eq!(first_keys, second_keys);
    assert_eq!(
        diff.get_starlark_first_for_testing()
            .keys()
            .cloned()
            .collect::<Vec<_>>(),
        vec![flag_name.clone()]
    );
    assert_eq!(
        diff.get_starlark_first_for_testing()
            .values()
            .cloned()
            .collect::<Vec<_>>(),
        vec![StarlarkOptionValue::Str(flag_value_one.to_string())]
    );
    assert_eq!(
        diff.get_starlark_second_for_testing()
            .values()
            .cloned()
            .collect::<Vec<_>>(),
        vec![StarlarkOptionValue::Str(flag_value_two.to_string())]
    );
}

#[test]
fn diff_extra_starlark_options() {
    let flag_name_one = Label::parse_canonical_unchecked("//extra/flag/one");
    let flag_name_two = Label::parse_canonical_unchecked("//extra/flag/two");
    let flag_value = "foo";
    let one = of_starlark(&[(&flag_name_one, flag_value)]);
    let two = of_starlark(&[(&flag_name_two, flag_value)]);

    let diff = OptionsDiff::diff(&one, &two);

    assert!(!diff.are_same());
    assert_eq!(
        diff.get_extra_starlark_options_first_for_testing().to_vec(),
        vec![flag_name_one.clone()]
    );
    assert_eq!(
        diff.get_extra_starlark_options_second_for_testing()
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect::<Vec<_>>(),
        vec![(
            flag_name_two.clone(),
            StarlarkOptionValue::Str(flag_value.to_string())
        )]
    );
}
