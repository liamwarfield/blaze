//! Port of `src/test/java/com/google/devtools/build/lib/packages/ImplicitOutputsFunctionTest.java`.

use std::collections::HashMap;

use blaze_cmdline::EvalException;
use indexmap::IndexSet;

use crate::attribute::AttributeMap;
use crate::attribute::ImplicitOutputsFunction;
use crate::attribute::implicit_outputs_function::ParsedTemplate;

fn assert_placeholder_collection(
    template: &str,
    expected_template: &str,
    expected_placeholders: &[&str],
) {
    let mut actual_placeholders = Vec::new();
    assert_eq!(
        ImplicitOutputsFunction::create_placeholder_substitution_format_string(
            template,
            &mut actual_placeholders
        ),
        expected_template
    );
    assert_eq!(actual_placeholders, expected_placeholders);
}

#[test]
fn test_no_placeholder() {
    assert_placeholder_collection("foo", "foo", &[]);
}

#[test]
fn test_just_placeholder() {
    assert_placeholder_collection("%{foo}", "%s", &["foo"]);
}

#[test]
fn test_prefixed_placeholder() {
    assert_placeholder_collection("foo%{bar}", "foo%s", &["bar"]);
}

#[test]
fn test_suffixed_placeholder() {
    assert_placeholder_collection("%{foo}bar", "%sbar", &["foo"]);
}

#[test]
fn test_multiple_placeholders_prefixed() {
    assert_placeholder_collection("foo%{bar}baz%{qux}", "foo%sbaz%s", &["bar", "qux"]);
}

#[test]
fn test_multiple_placeholders_suffixed() {
    assert_placeholder_collection("%{foo}bar%{baz}qux", "%sbar%squx", &["foo", "baz"]);
}

#[test]
fn test_tightly_packed_placeholders() {
    assert_placeholder_collection("%{foo}%{bar}%{baz}", "%s%s%s", &["foo", "bar", "baz"]);
}

#[test]
fn test_incomplete_placeholder() {
    assert_placeholder_collection("%{foo", "%%{foo", &[]);
}

#[test]
fn test_complete_and_incomplete_placeholder() {
    assert_placeholder_collection("%{foo}%{bar", "%s%%{bar", &["foo"]);
}

#[test]
fn test_placeholder_looks_like_nested_incomplete_placeholder() {
    assert_placeholder_collection("%{%{foo", "%%{%%{foo", &[]);
}

#[test]
fn test_placeholder_looks_like_nested_placeholder() {
    assert_placeholder_collection("%{%{foo}", "%s", &["%{foo"]);
}

#[test]
fn test_escapes_just_percent_sign() {
    assert_placeholder_collection("%", "%%", &[]);
}

#[test]
fn test_escapes_printf_placeholder() {
    assert_placeholder_collection("%{x}%s%{y}", "%s%%s%s", &["x", "y"]);
}

#[test]
fn test_escapes_percent_sign() {
    assert_placeholder_collection("foo%{bar}%baz", "foo%s%%baz", &["bar"]);
}

/// Java `attrs(values)`: an AttributeValueGetter over a fixed map (values deduplicated in order,
/// as Java's LinkedHashSet).
fn attrs(
    values: &[(&str, &[&str])],
) -> impl Fn(Option<&dyn AttributeMap>, &str) -> Result<IndexSet<String>, EvalException> {
    let values: HashMap<String, Vec<String>> = values
        .iter()
        .map(|(k, v)| ((*k).to_owned(), v.iter().map(|s| (*s).to_owned()).collect()))
        .collect();
    move |_ignored: Option<&dyn AttributeMap>, attr: &str| {
        Ok(values
            .get(attr)
            .expect("NullPointerException")
            .iter()
            .cloned()
            .collect())
    }
}

fn assert_placeholder_subtitution(
    template: &str,
    attr_values: &dyn Fn(
        Option<&dyn AttributeMap>,
        &str,
    ) -> Result<IndexSet<String>, EvalException>,
    expected_substitutions: &[&str],
    expected_found_placeholders: &[&str],
) {
    // Directly call into ParsedTemplate in order to access the attribute names.
    let parsed_template = ParsedTemplate::parse(template);
    assert_eq!(parsed_template.attribute_names, expected_found_placeholders);

    // Test the actual substitution code.
    let mut substitutions = ImplicitOutputsFunction::substitute_placeholder_into_template_with(
        template,
        None,
        attr_values,
    )
    .unwrap();
    // Java: containsExactlyElementsIn (any order).
    let mut expected: Vec<String> = expected_substitutions
        .iter()
        .map(|s| (*s).to_owned())
        .collect();
    substitutions.sort();
    expected.sort();
    assert_eq!(substitutions, expected);
}

#[test]
fn test_single_scalar_element_substitution() {
    assert_placeholder_subtitution("%{x}", &attrs(&[("x", &["a"])]), &["a"], &["x"]);
}

#[test]
fn test_single_vector_element_substitution() {
    assert_placeholder_subtitution(
        "%{x}",
        &attrs(&[("x", &["a", "b", "c"])]),
        &["a", "b", "c"],
        &["x"],
    );
}

#[test]
fn test_multiple_elements_substitution() {
    assert_placeholder_subtitution(
        "%{x}-%{y}-%{z}",
        &attrs(&[
            ("x", &["foo", "bar", "baz"]),
            ("y", &["meow"]),
            ("z", &["1", "2"]),
        ]),
        &[
            "foo-meow-1",
            "foo-meow-2",
            "bar-meow-1",
            "bar-meow-2",
            "baz-meow-1",
            "baz-meow-2",
        ],
        &["x", "y", "z"],
    );
}

#[test]
fn test_empty_element_substitution() {
    assert_placeholder_subtitution("a-%{x}", &attrs(&[("x", &[])]), &[], &["x"]);
}

#[test]
fn test_same_placeholder_multiple_times() {
    assert_placeholder_subtitution(
        "%{x}-%{y}-%{x}",
        &attrs(&[("x", &["a", "b"]), ("y", &["1", "2"])]),
        &[
            "a-1-a", "a-1-b", "a-2-a", "a-2-b", "b-1-a", "b-1-b", "b-2-a", "b-2-b",
        ],
        &["x", "y", "x"],
    );
}

#[test]
fn test_repeating_placeholder_value() {
    assert_placeholder_subtitution("%{x}", &attrs(&[("x", &["a", "a"])]), &["a"], &["x"]);
}

#[test]
fn test_incomplete_placeholder_treated_as_text() {
    assert_placeholder_subtitution(
        "%{x}-%{y-%{z",
        &attrs(&[("x", &["a", "b"])]),
        &["a-%{y-%{z", "b-%{y-%{z"],
        &["x"],
    );
}

/// Not in the Java test: the cross product is in lexicographic order (Sets.cartesianProduct).
#[test]
fn cartesian_product_order() {
    let out = ImplicitOutputsFunction::substitute_placeholder_into_template_with(
        "%{x}-%{y}",
        None,
        &attrs(&[("x", &["a", "b"]), ("y", &["1", "2"])]),
    )
    .unwrap();
    assert_eq!(out, vec!["a-1", "a-2", "b-1", "b-2"]);
}
