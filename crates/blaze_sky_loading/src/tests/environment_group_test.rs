//! Port of EnvironmentGroupTest on the PACKAGE harness.
//!
//! Java: src/test/java/com/google/devtools/build/lib/packages/EnvironmentGroupTest.java ("Tests
//! for EnvironmentGroup. Note input validation is handled in PackageFactoryTest.").
//!
//! Java's `@Before createPackage` writes pkg/BUILD and keeps `(EnvironmentGroup)
//! getTarget("//pkg:group")`; [`create_package`] does the same for each test.

use std::sync::Arc;

use blaze_cmdline::Label;
use blaze_packages::target::EnvironmentGroup;

use crate::tests::package_loading_test_case::PackageLoadingTestCase;

// Java: EnvironmentGroupTest.createPackage
fn create_package() -> (PackageLoadingTestCase, Arc<EnvironmentGroup>) {
    let t = PackageLoadingTestCase::new();
    t.scratch_file(
        "pkg/BUILD",
        &[
            "environment(",
            "    name = \"foo\",",
            "    fulfills = [",
            "        \":bar\",",
            "        \":baz\",",
            "    ],",
            ")",
            "",
            "environment(",
            "    name = \"bar\",",
            "    fulfills = [\":baz\"],",
            ")",
            "",
            "environment(name = \"baz\")",
            "",
            "environment(name = \"not_in_group\")",
            "",
            "environment_group(",
            "    name = \"group\",",
            "    defaults = [\":foo\"],",
            "    environments = [",
            "        \":foo\",",
            "        \":bar\",",
            "        \":baz\",",
            "    ],",
            ")",
        ],
    );
    let loaded = t.get_package("pkg");
    // Java: failFastHandler is installed while the package loads.
    loaded.assert_no_error_events();
    let group = loaded
        .target("group")
        .as_environment_group()
        .expect("//pkg:group is not an environment_group")
        .clone();
    (t, group)
}

fn label(raw: &str) -> Label {
    Label::parse_canonical_unchecked(raw)
}

/// The labels as sorted strings (Java compares sets, and Truth's `containsExactly` ignores
/// order).
fn sorted_strings<'a>(labels: impl IntoIterator<Item = &'a Label>) -> Vec<String> {
    let mut strings: Vec<String> = labels.into_iter().map(|l| l.to_string()).collect();
    strings.sort();
    strings
}

// Java: EnvironmentGroupTest.testGroupMembership
#[test]
fn test_group_membership() {
    let (_t, group) = create_package();
    assert_eq!(
        sorted_strings(group.get_environments()),
        sorted_strings(&[label("//pkg:foo"), label("//pkg:bar"), label("//pkg:baz")])
    );
}

// Java: EnvironmentGroupTest.defaultsMembership
#[test]
fn defaults_membership() {
    let (_t, group) = create_package();
    assert_eq!(
        sorted_strings(group.get_defaults()),
        sorted_strings(&[label("//pkg:foo")])
    );
}

// Java: EnvironmentGroupTest.isDefault
#[test]
fn is_default() {
    let (_t, group) = create_package();
    let unpacked_group = group.get_environment_labels();
    assert!(unpacked_group.is_default(&label("//pkg:foo")));
    assert!(!unpacked_group.is_default(&label("//pkg:bar")));
    assert!(!unpacked_group.is_default(&label("//pkg:baz")));
    assert!(!unpacked_group.is_default(&label("//pkg:not_in_group")));
}

// Java: EnvironmentGroupTest.fulfillers
#[test]
fn fulfillers() {
    let (_t, group) = create_package();
    let unpacked_group = group.get_environment_labels();
    assert_eq!(
        sorted_strings(
            unpacked_group
                .get_fulfillers(&label("//pkg:baz"))
                .to_list()
                .iter()
        ),
        sorted_strings(&[label("//pkg:foo"), label("//pkg:bar")])
    );
    assert_eq!(
        sorted_strings(
            unpacked_group
                .get_fulfillers(&label("//pkg:bar"))
                .to_list()
                .iter()
        ),
        sorted_strings(&[label("//pkg:foo")])
    );
    assert!(
        unpacked_group
            .get_fulfillers(&label("//pkg:foo"))
            .to_list()
            .is_empty()
    );
}

// Java: EnvironmentGroupTest.emptyGroupsNotAllowed
#[test]
fn empty_groups_not_allowed() {
    let (t, _group) = create_package();
    t.scratch_file(
        "a/BUILD",
        &["environment_group(name = 'empty_group', environments = [], defaults = [])"],
    );
    let pkg = t.get_package("a");
    pkg.target("BUILD");
    assert!(pkg.package.contains_errors());
    pkg.assert_contains_event(
        "environment group empty_group must contain at least one environment",
    );
}

// Java: EnvironmentGroupTest.reduceForSerialization_hasConsistentValues
//   assertThat(group).hasSamePropertiesAs(group.reduceForSerialization());
#[test]
#[ignore = "needs EnvironmentGroup.reduceForSerialization (EnvironmentGroup.java:329, the EnvironmentGroupData TargetData; not ported) and the TargetData properties TargetDataSubject.hasSamePropertiesAs compares"]
fn reduce_for_serialization_has_consistent_values() {}
