//! Port of `src/test/java/com/google/devtools/build/lib/analysis/config/FeatureSetTest.java`.

use std::collections::BTreeSet;

use blaze_build_config::build_configuration_value::FeatureSet;

fn parse(features: &[&str]) -> FeatureSet {
    let features: Vec<String> = features.iter().map(|s| s.to_string()).collect();
    FeatureSet::parse(&features)
}

fn merge(coarse: &FeatureSet, fine: &FeatureSet) -> FeatureSet {
    FeatureSet::merge(coarse, fine)
}

/// `containsExactly` (order-insensitive) against a set of strings.
fn set(items: &[&str]) -> BTreeSet<String> {
    items.iter().map(|s| s.to_string()).collect()
}

#[test]
fn parse_basic() {
    let feature_set = parse(&["foo", "bar", "-baz"]);
    assert_eq!(feature_set.on, set(&["foo", "bar"]));
    assert_eq!(feature_set.off, set(&["baz"]));
}

#[test]
fn parse_off_trumps_on() {
    let feature_set = parse(&["foo", "bar", "-bar"]);
    assert_eq!(feature_set.on, set(&["foo"]));
    assert_eq!(feature_set.off, set(&["bar"]));
}

#[test]
fn parse_no_layering_check() {
    let feature_set = parse(&["foo", "bar", "no_layering_check"]);
    assert_eq!(feature_set.on, set(&["foo", "bar"]));
    assert_eq!(feature_set.off, set(&["layering_check"]));
}

#[test]
fn merge_basic() {
    assert_eq!(
        merge(&parse(&["foo", "-bar"]), &parse(&["kek", "-lol"])),
        parse(&["foo", "kek", "-bar", "-lol"])
    );
}

#[test]
fn merge_fine_trumps_coarse() {
    assert_eq!(
        merge(&parse(&["foo", "-bar"]), &parse(&["bar", "-foo"])),
        parse(&["-foo", "bar"])
    );
}

#[test]
fn merge_associativity() {
    let a = parse(&["foo", "bar", "-baz"]);
    let b = parse(&["-foo", "-bar"]);
    let c = parse(&["foo", "baz"]);
    assert_eq!(merge(&a, &merge(&b, &c)), merge(&merge(&a, &b), &c));
    assert_eq!(merge(&b, &merge(&c, &a)), merge(&merge(&b, &c), &a));
    assert_eq!(merge(&c, &merge(&b, &a)), merge(&merge(&c, &b), &a));
}

#[test]
fn merge_with_global_features_basic() {
    assert_eq!(
        FeatureSet::merge_with_global_features(&parse(&["foo", "-bar"]), &parse(&["kek", "-lol"])),
        parse(&["foo", "kek", "-bar", "-lol"])
    );
}

#[test]
fn merge_with_global_features_global_off_trumps_everything() {
    assert_eq!(
        FeatureSet::merge_with_global_features(&parse(&["foo", "-bar"]), &parse(&["bar", "-foo"])),
        parse(&["-bar", "-foo"])
    );
}
