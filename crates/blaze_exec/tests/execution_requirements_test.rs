//! Port of src/test/java/com/google/devtools/build/lib/actions/ExecutionRequirementsTest.java.

use blaze_exec::actions::execution_requirements::parse_resources;
use indexmap::IndexMap;

fn map(entries: &[(&str, &str)]) -> IndexMap<String, String> {
    entries
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

/// `assertThat(map).containsExactly(k1, v1, ...)` (any order).
fn assert_contains_exactly(actual: &IndexMap<String, f64>, expected: &[(&str, f64)]) {
    assert_eq!(actual.len(), expected.len(), "{actual:?}");
    for (k, v) in expected {
        assert_eq!(actual.get(*k), Some(v), "{actual:?}");
    }
}

// Java: src/test/java/com/google/devtools/build/lib/actions/ExecutionRequirementsTest.java:29 ExecutionRequirementsTest.parseResources_empty
#[test]
fn parse_resources_empty() {
    assert!(parse_resources(&IndexMap::new()).unwrap().is_empty());
}

// Java: src/test/java/com/google/devtools/build/lib/actions/ExecutionRequirementsTest.java:34 ExecutionRequirementsTest.parseResources_ignoresUnrelatedKeys
#[test]
fn parse_resources_ignores_unrelated_keys() {
    assert!(
        parse_resources(&map(&[
            ("pool", "default"),
            ("no-sandbox", ""),
            ("local", "")
        ]))
        .unwrap()
        .is_empty()
    );
}

// exec_properties format: key = "resources:name", value = "amount"

// Java: src/test/java/com/google/devtools/build/lib/actions/ExecutionRequirementsTest.java:44 ExecutionRequirementsTest.parseResources_execProp_cpu
#[test]
fn parse_resources_exec_prop_cpu() {
    assert_contains_exactly(
        &parse_resources(&map(&[("resources:cpu", "4")])).unwrap(),
        &[("cpu", 4.0)],
    );
}

// Java: src/test/java/com/google/devtools/build/lib/actions/ExecutionRequirementsTest.java:50 ExecutionRequirementsTest.parseResources_execProp_memory
#[test]
fn parse_resources_exec_prop_memory() {
    assert_contains_exactly(
        &parse_resources(&map(&[("resources:memory", "2000")])).unwrap(),
        &[("memory", 2000.0)],
    );
}

// Java: src/test/java/com/google/devtools/build/lib/actions/ExecutionRequirementsTest.java:56 ExecutionRequirementsTest.parseResources_execProp_multiple
#[test]
fn parse_resources_exec_prop_multiple() {
    assert_contains_exactly(
        &parse_resources(&map(&[
            ("resources:cpu", "8"),
            ("resources:memory", "4000"),
        ]))
        .unwrap(),
        &[("cpu", 8.0), ("memory", 4000.0)],
    );
}

// Java: src/test/java/com/google/devtools/build/lib/actions/ExecutionRequirementsTest.java:64 ExecutionRequirementsTest.parseResources_execProp_customResource
#[test]
fn parse_resources_exec_prop_custom_resource() {
    assert_contains_exactly(
        &parse_resources(&map(&[("resources:gpu", "2")])).unwrap(),
        &[("gpu", 2.0)],
    );
}

// Java: src/test/java/com/google/devtools/build/lib/actions/ExecutionRequirementsTest.java:70 ExecutionRequirementsTest.parseResources_execProp_floatingPoint
#[test]
fn parse_resources_exec_prop_floating_point() {
    assert_contains_exactly(
        &parse_resources(&map(&[("resources:cpu", "2.5")])).unwrap(),
        &[("cpu", 2.5)],
    );
}

// Java: src/test/java/com/google/devtools/build/lib/actions/ExecutionRequirementsTest.java:76 ExecutionRequirementsTest.parseResources_execProp_mixedKeys
#[test]
fn parse_resources_exec_prop_mixed_keys() {
    assert_contains_exactly(
        &parse_resources(&map(&[
            ("pool", "default"),
            ("resources:cpu", "4"),
            ("container-image", "docker://foo"),
            ("resources:gpu", "1"),
        ]))
        .unwrap(),
        &[("cpu", 4.0), ("gpu", 1.0)],
    );
}

// Tag format: key = "resources:name:amount" or "cpu:amount", value = ""

// Java: src/test/java/com/google/devtools/build/lib/actions/ExecutionRequirementsTest.java:90 ExecutionRequirementsTest.parseResources_tag_resources
#[test]
fn parse_resources_tag_resources() {
    assert_contains_exactly(
        &parse_resources(&map(&[("resources:cpu:4", "")])).unwrap(),
        &[("cpu", 4.0)],
    );
}

// Java: src/test/java/com/google/devtools/build/lib/actions/ExecutionRequirementsTest.java:96 ExecutionRequirementsTest.parseResources_tag_cpu
#[test]
fn parse_resources_tag_cpu() {
    assert_contains_exactly(
        &parse_resources(&map(&[("cpu:2", "")])).unwrap(),
        &[("cpu", 2.0)],
    );
}

// Java: src/test/java/com/google/devtools/build/lib/actions/ExecutionRequirementsTest.java:102 ExecutionRequirementsTest.parseResources_tag_customResource
#[test]
fn parse_resources_tag_custom_resource() {
    assert_contains_exactly(
        &parse_resources(&map(&[("resources:gpu:3", "")])).unwrap(),
        &[("gpu", 3.0)],
    );
}

// Java: src/test/java/com/google/devtools/build/lib/actions/ExecutionRequirementsTest.java:108 ExecutionRequirementsTest.parseResources_tag_multiple
#[test]
fn parse_resources_tag_multiple() {
    assert_contains_exactly(
        &parse_resources(&map(&[("resources:gpu:2", ""), ("cpu:4", "")])).unwrap(),
        &[("gpu", 2.0), ("cpu", 4.0)],
    );
}

// Validation

// Java: src/test/java/com/google/devtools/build/lib/actions/ExecutionRequirementsTest.java:118 ExecutionRequirementsTest.parseResources_throwsOnTagMissingAmount
#[test]
fn parse_resources_throws_on_tag_missing_amount() {
    assert!(parse_resources(&map(&[("resources:cpu:", "")])).is_err());
}

// Java: src/test/java/com/google/devtools/build/lib/actions/ExecutionRequirementsTest.java:125 ExecutionRequirementsTest.parseResources_throwsOnExecPropMissingAmount
#[test]
fn parse_resources_throws_on_exec_prop_missing_amount() {
    assert!(parse_resources(&map(&[("resources:cpu", "")])).is_err());
}

// Java: src/test/java/com/google/devtools/build/lib/actions/ExecutionRequirementsTest.java:132 ExecutionRequirementsTest.parseResources_throwsOnInvalidTagValue
#[test]
fn parse_resources_throws_on_invalid_tag_value() {
    assert!(parse_resources(&map(&[("resources:cpu:notanumber", "")])).is_err());
}

// Java: src/test/java/com/google/devtools/build/lib/actions/ExecutionRequirementsTest.java:140 ExecutionRequirementsTest.parseResources_throwsOnInvalidExecPropValue
#[test]
fn parse_resources_throws_on_invalid_exec_prop_value() {
    assert!(parse_resources(&map(&[("resources:cpu", "notanumber")])).is_err());
}

// Java: src/test/java/com/google/devtools/build/lib/actions/ExecutionRequirementsTest.java:147 ExecutionRequirementsTest.parseResources_throwsOnNegativeTagValue
#[test]
fn parse_resources_throws_on_negative_tag_value() {
    assert!(parse_resources(&map(&[("resources:cpu:-1", "")])).is_err());
}

// Java: src/test/java/com/google/devtools/build/lib/actions/ExecutionRequirementsTest.java:154 ExecutionRequirementsTest.parseResources_throwsOnDuplicateResource
#[test]
fn parse_resources_throws_on_duplicate_resource() {
    assert!(parse_resources(&map(&[("resources:cpu:4", ""), ("cpu:2", "")])).is_err());
}
