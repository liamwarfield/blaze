//! Ports of the attribute mapper tests: `analysis/select/AbstractAttributeMapperTest.java`,
//! `AggregatingAttributeMapperTest.java`, `RawAttributeMapperTest.java`,
//! `NonconfigurableAttributeMapperTest.java`. Their rules come from BUILD files
//! (`scratchRule`), so most belong on the PACKAGE harness of blaze_sky_loading
//! (`PackageLoadingTestCase`), which this crate cannot depend on; their stubs stay here, ignored
//! with the reason, until they are ported there. The combination logic of
//! `AggregatingAttributeMapper.getAllValues` is tested directly on selectors.

use blaze_cmdline::Label;
use indexmap::IndexSet;

use crate::attribute::aggregating_attribute_mapper::get_all_values;
use crate::types::AttrValue;
use crate::types::BuildTypeSelector;
use crate::types::Type;

fn label(s: &str) -> Label {
    Label::parse_canonical_unchecked(s)
}

/// A selector over label lists: `(condition, [labels])` entries, no default.
fn label_list_selector(entries: &[(&str, &[&str])]) -> BuildTypeSelector {
    BuildTypeSelector::new(
        entries.iter().map(|(c, _)| label(c)).collect(),
        entries
            .iter()
            .map(|(_, v)| Some(AttrValue::LabelList(v.iter().map(|l| label(l)).collect())))
            .collect(),
        Type::LabelList,
        String::new(),
        IndexSet::new(),
        -1,
    )
}

fn sorted(mut values: Vec<AttrValue>) -> Vec<String> {
    let mut out: Vec<String> = values.drain(..).map(|v| format!("{v:?}")).collect();
    out.sort();
    out
}

fn label_lists(lists: &[&[&str]]) -> Vec<AttrValue> {
    lists
        .iter()
        .map(|l| AttrValue::LabelList(l.iter().map(|s| label(s)).collect()))
        .collect()
}

// ---- AggregatingAttributeMapperTest

#[test]
#[ignore = "not ported yet: the rule comes from a BUILD file (Java scratchRule with //test_defs:foo_binary.bzl or genrule), so the test belongs on blaze_sky_loading's PackageLoadingTestCase, which this crate cannot depend on; nothing else blocks it"]
fn test_get_possible_values_direct_attribute() {}

#[test]
#[ignore = "not ported yet: the rule comes from a BUILD file (Java scratchRule with //test_defs:foo_binary.bzl or genrule), so the test belongs on blaze_sky_loading's PackageLoadingTestCase, which this crate cannot depend on; nothing else blocks it"]
fn test_get_possible_values_configurable_attribute() {}

/// Java `testGetPossibleValuesWithConcatenatedSelects`, on the selectors directly.
#[test]
fn test_get_possible_values_with_concatenated_selects() {
    let selectors = vec![
        label_list_selector(&[
            ("//conditions:a1", &["//a:a1.sh"]),
            ("//conditions:b1", &["//a:b1.sh"]),
        ]),
        label_list_selector(&[
            ("//conditions:a2", &["//a:a2.sh"]),
            ("//conditions:b2", &["//a:b2.sh"]),
        ]),
    ];
    assert_eq!(
        sorted(get_all_values(&selectors, Type::LabelList, false)),
        sorted(label_lists(&[
            &["//a:a1.sh", "//a:a2.sh"],
            &["//a:a1.sh", "//a:b2.sh"],
            &["//a:b1.sh", "//a:a2.sh"],
            &["//a:b1.sh", "//a:b2.sh"],
        ]))
    );
}

/// Java `testGetPossibleValuesWithManySelects`, on the selectors directly: equivalent key sets
/// are followed along one key, so 26 selects give 2 values, not 2^26.
#[test]
fn test_get_possible_values_with_many_selects() {
    let selectors: Vec<BuildTypeSelector> = "abcdefghijklmnopqrstuvwxyz"
        .chars()
        .map(|c| {
            BuildTypeSelector::new(
                vec![label("//conditions:a1"), label("//conditions:a2")],
                vec![
                    Some(AttrValue::String(c.to_string())),
                    Some(AttrValue::String(c.to_ascii_uppercase().to_string())),
                ],
                Type::String,
                String::new(),
                IndexSet::new(),
                -1,
            )
        })
        .collect();
    assert_eq!(
        sorted(get_all_values(&selectors, Type::String, false)),
        sorted(vec![
            AttrValue::String("abcdefghijklmnopqrstuvwxyz".to_owned()),
            AttrValue::String("ABCDEFGHIJKLMNOPQRSTUVWXYZ".to_owned()),
        ])
    );
}

/// Java `testGetPossibleValuesWithMultipleSelectsWithOverlappingConditions`, on the selectors
/// directly.
#[test]
fn test_get_possible_values_with_multiple_selects_with_overlapping_conditions() {
    let selectors = vec![
        label_list_selector(&[("//conditions:x", &["//a:x1.sh"])]),
        label_list_selector(&[("//conditions:y", &["//a:y1.sh"])]),
        label_list_selector(&[
            ("//conditions:x", &["//a:x2.sh"]),
            ("//conditions:y", &["//a:y2.sh"]),
            ("//conditions:z", &["//a:z2.sh"]),
        ]),
    ];
    assert_eq!(
        sorted(get_all_values(&selectors, Type::LabelList, false)),
        sorted(label_lists(&[
            &["//a:x1.sh", "//a:y1.sh", "//a:x2.sh"],
            &["//a:x1.sh", "//a:y1.sh", "//a:y2.sh"],
            &["//a:x1.sh", "//a:y1.sh", "//a:z2.sh"],
        ]))
    );
}

/// Not in the Java test: `mayTreatMultipleAsNone` gives up on a second value.
#[test]
fn get_all_values_may_treat_multiple_as_none() {
    let selectors = vec![
        label_list_selector(&[
            ("//conditions:a1", &["//a:a1.sh"]),
            ("//conditions:b1", &["//a:b1.sh"]),
        ]),
        label_list_selector(&[("//conditions:a2", &["//a:a2.sh"])]),
    ];
    assert!(get_all_values(&selectors, Type::LabelList, true).is_empty());
}

#[test]
#[ignore = "not ported yet: the rule comes from a BUILD file (Java scratchRule with //test_defs:foo_binary.bzl or genrule), so the test belongs on blaze_sky_loading's PackageLoadingTestCase, which this crate cannot depend on; nothing else blocks it"]
fn test_visitation_configurable_attribute() {}

#[test]
#[ignore = "the rule comes from a BUILD file that loads @rules_cc//cc:cc_binary.bzl (Java scratchRule): needs blaze_sky_loading's PackageLoadingTestCase (this crate cannot depend on it) with an @rules_cc repository in the scratch workspace, which the harness does not have"]
fn test_get_reachable_labels() {}

#[test]
#[ignore = "the rule comes from a BUILD file (Java scratchRule) and is of a test-only native rule class added to the test rule class provider (rule_with_default / rule_with_computed_defaults): needs blaze_sky_loading's PackageLoadingTestCase (this crate cannot depend on it) and a way to add a native rule class to its rule class provider"]
fn test_visitation_with_default_values() {}

#[test]
#[ignore = "the rule comes from a BUILD file (Java scratchRule) and is of a test-only native rule class added to the test rule class provider (rule_with_default / rule_with_computed_defaults): needs blaze_sky_loading's PackageLoadingTestCase (this crate cannot depend on it) and a way to add a native rule class to its rule class provider"]
fn test_get_reachable_labels_with_default_values() {}

#[test]
#[ignore = "the rule comes from a BUILD file (Java scratchRule) and is of a test-only native rule class added to the test rule class provider (rule_with_default / rule_with_computed_defaults): needs blaze_sky_loading's PackageLoadingTestCase (this crate cannot depend on it) and a way to add a native rule class to its rule class provider"]
fn test_computed_default_with_configurable_deps() {}

#[test]
#[ignore = "the rule comes from a BUILD file (Java scratchRule) and is of a test-only native rule class added to the test rule class provider (rule_with_default / rule_with_computed_defaults): needs blaze_sky_loading's PackageLoadingTestCase (this crate cannot depend on it) and a way to add a native rule class to its rule class provider"]
fn test_computed_default_without_configurable_deps() {}

// ---- AbstractAttributeMapperTest

#[test]
#[ignore = "the rule comes from a BUILD file that loads @rules_cc//cc:cc_binary.bzl (Java scratchRule): needs blaze_sky_loading's PackageLoadingTestCase (this crate cannot depend on it) with an @rules_cc repository in the scratch workspace, which the harness does not have"]
fn test_rule_properties() {}

#[test]
#[ignore = "the rule comes from a BUILD file that loads @rules_cc//cc:cc_binary.bzl (Java scratchRule): needs blaze_sky_loading's PackageLoadingTestCase (this crate cannot depend on it) with an @rules_cc repository in the scratch workspace, which the harness does not have"]
fn test_package_default_properties() {}

#[test]
#[ignore = "the rule comes from a BUILD file that loads @rules_cc//cc:cc_binary.bzl (Java scratchRule): needs blaze_sky_loading's PackageLoadingTestCase (this crate cannot depend on it) with an @rules_cc repository in the scratch workspace, which the harness does not have"]
fn test_attribute_type_checking() {}

#[test]
#[ignore = "the rule comes from a BUILD file that loads @rules_cc//cc:cc_binary.bzl (Java scratchRule): needs blaze_sky_loading's PackageLoadingTestCase (this crate cannot depend on it) with an @rules_cc repository in the scratch workspace, which the harness does not have"]
fn test_get_attribute_type() {}

#[test]
#[ignore = "the rule comes from a BUILD file that loads @rules_cc//cc:cc_binary.bzl (Java scratchRule): needs blaze_sky_loading's PackageLoadingTestCase (this crate cannot depend on it) with an @rules_cc repository in the scratch workspace, which the harness does not have"]
fn test_get_attribute_definition() {}

#[test]
#[ignore = "the rule comes from a BUILD file that loads @rules_cc//cc:cc_binary.bzl (Java scratchRule): needs blaze_sky_loading's PackageLoadingTestCase (this crate cannot depend on it) with an @rules_cc repository in the scratch workspace, which the harness does not have"]
fn test_is_attribute_explicitly_specified() {}

#[test]
#[ignore = "the rule comes from a BUILD file that loads @rules_cc//cc:cc_binary.bzl (Java scratchRule): needs blaze_sky_loading's PackageLoadingTestCase (this crate cannot depend on it) with an @rules_cc repository in the scratch workspace, which the harness does not have"]
fn test_visitation() {}

// ---- RawAttributeMapperTest

#[test]
#[ignore = "not ported yet: the rule comes from a BUILD file (Java scratchRule with //test_defs:foo_binary.bzl or genrule), so the test belongs on blaze_sky_loading's PackageLoadingTestCase, which this crate cannot depend on; nothing else blocks it"]
fn test_get_attribute() {}

#[test]
#[ignore = "not ported yet: the rule comes from a BUILD file (Java scratchRule with //test_defs:foo_binary.bzl or genrule), so the test belongs on blaze_sky_loading's PackageLoadingTestCase, which this crate cannot depend on; nothing else blocks it"]
fn test_configurability_check() {}

#[test]
#[ignore = "not ported yet: the rule comes from a BUILD file (Java scratchRule with //test_defs:foo_binary.bzl or genrule), so the test belongs on blaze_sky_loading's PackageLoadingTestCase, which this crate cannot depend on; nothing else blocks it"]
fn test_visit_labels() {}

#[test]
#[ignore = "not ported yet: the rule comes from a BUILD file (Java scratchRule with //test_defs:foo_binary.bzl or genrule), so the test belongs on blaze_sky_loading's PackageLoadingTestCase, which this crate cannot depend on; nothing else blocks it"]
fn test_get_configurability_keys() {}

#[test]
#[ignore = "not ported yet: the rule comes from a BUILD file (Java scratchRule with //test_defs:foo_binary.bzl or genrule), so the test belongs on blaze_sky_loading's PackageLoadingTestCase, which this crate cannot depend on; nothing else blocks it"]
fn test_get_merged_values() {}

#[test]
#[ignore = "not ported yet: the rule comes from a BUILD file (Java scratchRule with //test_defs:foo_binary.bzl or genrule), so the test belongs on blaze_sky_loading's PackageLoadingTestCase, which this crate cannot depend on; nothing else blocks it"]
fn test_merged_values_with_concatenated_selects() {}

// ---- NonconfigurableAttributeMapperTest

#[test]
#[ignore = "the rule comes from a BUILD file that loads @rules_cc//cc:cc_binary.bzl (Java scratchRule): needs blaze_sky_loading's PackageLoadingTestCase (this crate cannot depend on it) with an @rules_cc repository in the scratch workspace, which the harness does not have"]
fn test_get_nonconfigurable_attribute() {}

#[test]
#[ignore = "the rule comes from a BUILD file that loads @rules_cc//cc:cc_binary.bzl (Java scratchRule): needs blaze_sky_loading's PackageLoadingTestCase (this crate cannot depend on it) with an @rules_cc repository in the scratch workspace, which the harness does not have"]
fn test_get_configurable_attribute() {}

#[test]
#[ignore = "the rule comes from a BUILD file that loads @rules_cc//cc:cc_binary.bzl (Java scratchRule): needs blaze_sky_loading's PackageLoadingTestCase (this crate cannot depend on it) with an @rules_cc repository in the scratch workspace, which the harness does not have"]
fn test_get_nonexistent_attribute() {}

#[test]
#[ignore = "the rule comes from a BUILD file that loads @rules_cc//cc:cc_binary.bzl (Java scratchRule): needs blaze_sky_loading's PackageLoadingTestCase (this crate cannot depend on it) with an @rules_cc repository in the scratch workspace, which the harness does not have"]
fn test_nonconfigurable_attribute_type_checking() {}

#[test]
#[ignore = "the rule comes from a BUILD file that loads @rules_cc//cc:cc_binary.bzl (Java scratchRule): needs blaze_sky_loading's PackageLoadingTestCase (this crate cannot depend on it) with an @rules_cc repository in the scratch workspace, which the harness does not have"]
fn test_nonconfigurable_visitation() {}
