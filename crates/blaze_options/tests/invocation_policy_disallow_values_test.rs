//! Port of src/test/java/com/google/devtools/common/options/InvocationPolicyDisallowValuesTest.java.

mod common;

use common::invocation_policy::*;
use common::test_options::*;

// Useful constants
const DISALLOWED_VALUE_1: &str = "foo";
const DISALLOWED_VALUE_2: &str = "bar";
const UNFILTERED_VALUE: &str = "baz";

fn s(v: &str) -> String {
    v.to_string()
}

const FOO_BAR_POLICY: &str = r#"flag_policies { flag_name: "test_string"
     disallow_values { disallowed_values: "foo" disallowed_values: "bar" } }"#;

// Java: InvocationPolicyDisallowValuesTest.testDisallowValuesAllowsValue
#[test]
fn test_disallow_values_allows_value() {
    let mut parser = set_parser();
    let enforcer = create_options_policy_enforcer(&policy(FOO_BAR_POLICY));
    parser
        .parse(&[&format!("--test_string={UNFILTERED_VALUE}")])
        .unwrap();

    // Option should be "baz" as specified by the user.
    assert_eq!(get_test_options(&parser).test_string, UNFILTERED_VALUE);

    enforce(&enforcer, &mut parser).unwrap();

    // Still "baz" since "baz" is allowed by the policy.
    assert_eq!(get_test_options(&parser).test_string, UNFILTERED_VALUE);
}

// Java: InvocationPolicyDisallowValuesTest.testDisallowValuesDisallowsValue
#[test]
fn test_disallow_values_disallows_value() {
    let mut parser = set_parser();
    let enforcer = create_options_policy_enforcer(&policy(FOO_BAR_POLICY));
    parser
        .parse(&[&format!("--test_string={DISALLOWED_VALUE_1}")])
        .unwrap();

    // Option should be "foo" as specified by the user.
    assert_eq!(get_test_options(&parser).test_string, DISALLOWED_VALUE_1);

    // expected, since foo is disallowed.
    assert!(enforce(&enforcer, &mut parser).is_err());
}

// Java: InvocationPolicyDisallowValuesTest.testDisallowValuesDisallowsMultipleValues
#[test]
fn test_disallow_values_disallows_multiple_values() {
    let mut parser = set_parser();
    let enforcer = create_options_policy_enforcer(&policy(
        r#"flag_policies { flag_name: "test_multiple_string"
             disallow_values { disallowed_values: "foo" disallowed_values: "bar" } }"#,
    ));
    parser
        .parse(&[
            &format!("--test_multiple_string={UNFILTERED_VALUE}"),
            &format!("--test_multiple_string={DISALLOWED_VALUE_2}"),
        ])
        .unwrap();

    // Option should be "baz" and "bar" as specified by the user.
    assert_eq!(
        get_test_options(&parser).test_multiple_string,
        vec![s(UNFILTERED_VALUE), s(DISALLOWED_VALUE_2)]
    );

    // expected, since bar is disallowed.
    assert!(enforce(&enforcer, &mut parser).is_err());
}

// Java: InvocationPolicyDisallowValuesTest.testDisallowValuesSetsNewValue
#[test]
fn test_disallow_values_sets_new_value() {
    let mut parser = set_parser();
    let enforcer = create_options_policy_enforcer(&policy(
        r#"flag_policies { flag_name: "test_string"
             disallow_values { disallowed_values: "foo" new_value: "baz" } }"#,
    ));
    parser
        .parse(&[&format!("--test_string={DISALLOWED_VALUE_1}")])
        .unwrap();

    assert_eq!(get_test_options(&parser).test_string, DISALLOWED_VALUE_1);

    enforce(&enforcer, &mut parser).unwrap();

    // Should now be "baz" because the policy forces disallowed values to "baz"
    assert_eq!(get_test_options(&parser).test_string, UNFILTERED_VALUE);
}

// Java: InvocationPolicyDisallowValuesTest.testDisallowValuesSetsDefaultValue
#[test]
fn test_disallow_values_sets_default_value() {
    let mut parser = set_parser();
    let enforcer = create_options_policy_enforcer(&policy(
        r#"flag_policies { flag_name: "test_string"
             disallow_values { disallowed_values: "foo" use_default {} } }"#,
    ));
    parser
        .parse(&[&format!("--test_string={DISALLOWED_VALUE_1}")])
        .unwrap();

    assert_eq!(get_test_options(&parser).test_string, DISALLOWED_VALUE_1);

    enforce(&enforcer, &mut parser).unwrap();

    assert_eq!(get_test_options(&parser).test_string, TEST_STRING_DEFAULT);
}

// Java: InvocationPolicyDisallowValuesTest.testDisallowValuesSetsDefaultValueForRepeatableFlag
#[test]
fn test_disallow_values_sets_default_value_for_repeatable_flag() {
    let mut parser = set_parser();
    let enforcer = create_options_policy_enforcer(&policy(
        r#"flag_policies { flag_name: "test_multiple_string"
             disallow_values { disallowed_values: "foo" use_default {} } }"#,
    ));
    parser
        .parse(&[&format!("--test_multiple_string={DISALLOWED_VALUE_1}")])
        .unwrap();

    assert_eq!(
        get_test_options(&parser).test_multiple_string,
        vec![s(DISALLOWED_VALUE_1)]
    );

    enforce(&enforcer, &mut parser).unwrap();

    // Default for repeatable flags is always empty.
    assert!(get_test_options(&parser).test_multiple_string.is_empty());
}

// Java: InvocationPolicyDisallowValuesTest.testDisallowValuesRaisesErrorIfDefaultIsDisallowedAndSetsUseDefault
#[test]
fn test_disallow_values_raises_error_if_default_is_disallowed_and_sets_use_default() {
    let mut parser = set_parser();
    let enforcer = create_options_policy_enforcer(&policy(&format!(
        r#"flag_policies {{ flag_name: "test_string"
             disallow_values {{ disallowed_values: "{TEST_STRING_DEFAULT}" use_default {{}} }} }}"#
    )));

    let e = enforce(&enforcer, &mut parser).unwrap_err();
    assert!(
        e.get_message()
            .contains("but also specifies to use the default value"),
        "{}",
        e.get_message()
    );
}

// Java: InvocationPolicyDisallowValuesTest.testDisallowValuesSetsNewValueWhenDefaultIsDisallowed
#[test]
fn test_disallow_values_sets_new_value_when_default_is_disallowed() {
    let mut parser = set_parser();
    let enforcer = create_options_policy_enforcer(&policy(&format!(
        r#"flag_policies {{ flag_name: "test_string"
             disallow_values {{ disallowed_values: "{TEST_STRING_DEFAULT}"
                               new_value: "{UNFILTERED_VALUE}" }} }}"#
    )));

    // Option should be the default since the use didn't specify a value.
    assert_eq!(get_test_options(&parser).test_string, TEST_STRING_DEFAULT);

    enforce(&enforcer, &mut parser).unwrap();

    // Should now be "baz" because the policy set the new default to "baz"
    assert_eq!(get_test_options(&parser).test_string, UNFILTERED_VALUE);
}

// Java: InvocationPolicyDisallowValuesTest.testDisallowValuesDisallowsFlagDefaultButNoPolicyDefault
#[test]
fn test_disallow_values_disallows_flag_default_but_no_policy_default() {
    let mut parser = set_parser();
    // No new default is set
    let enforcer = create_options_policy_enforcer(&policy(&format!(
        r#"flag_policies {{ flag_name: "test_string"
             disallow_values {{ disallowed_values: "{TEST_STRING_DEFAULT}" }} }}"#
    )));

    // Option should be the default since the use didn't specify a value.
    assert_eq!(get_test_options(&parser).test_string, TEST_STRING_DEFAULT);

    assert!(enforce(&enforcer, &mut parser).is_err());
}

// Java: InvocationPolicyDisallowValuesTest.testDisallowValuesDisallowsListConverterFlag
#[test]
fn test_disallow_values_disallows_list_converter_flag() {
    let mut parser = set_parser();
    let enforcer = create_options_policy_enforcer(&policy(
        r#"flag_policies { flag_name: "test_list_converters"
             disallow_values { disallowed_values: "a" } }"#,
    ));
    parser.parse(&["--test_list_converters=a,b,c"]).unwrap();

    assert_eq!(
        get_test_options(&parser).test_list_converters,
        vec![s("a"), s("b"), s("c")]
    );

    let e = enforce(&enforcer, &mut parser).unwrap_err();
    assert!(
        e.get_message().contains(
            "Flag value 'a' for option '--test_list_converters' is not allowed by invocation \
             policy"
        ),
        "{}",
        e.get_message()
    );
}

// Java: InvocationPolicyDisallowValuesTest.testAllowValuesWithNullDefault_DoesNotConfuseNullForDefault
#[test]
fn test_allow_values_with_null_default_does_not_confuse_null_for_default() {
    let mut parser = set_parser();
    let enforcer = create_options_policy_enforcer(&policy(
        r#"flag_policies { flag_name: "test_string_null_by_default"
             disallow_values { disallowed_values: "null" use_default {} } }"#,
    ));

    // Check the value before invocation policy enforcement.
    parser
        .parse(&["--test_string_null_by_default=null"])
        .unwrap();
    assert_eq!(
        get_test_options(&parser)
            .test_string_null_by_default
            .as_deref(),
        Some("null")
    );

    // Check the value afterwards.
    enforce(&enforcer, &mut parser).unwrap();
    assert_eq!(get_test_options(&parser).test_string_null_by_default, None);
}
