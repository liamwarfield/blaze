//! Port of src/test/java/com/google/devtools/common/options/InvocationPolicyAllowValuesTest.java.

mod common;

use common::invocation_policy::*;
use common::test_options::*;

// Useful constants
const ALLOWED_VALUE_1: &str = "foo";
const ALLOWED_VALUE_2: &str = "bar";
const UNFILTERED_VALUE: &str = "baz";

fn s(v: &str) -> String {
    v.to_string()
}

/// Tests that AllowValues works in the normal case where the value the user specified is allowed
/// by the policy.
// Java: InvocationPolicyAllowValuesTest.testAllowValuesAllowsValue
#[test]
fn test_allow_values_allows_value() {
    let mut parser = set_parser();
    let enforcer = create_options_policy_enforcer(&policy(&format!(
        r#"flag_policies {{ flag_name: "test_string"
             allow_values {{ allowed_values: "{TEST_STRING_DEFAULT}"
                            allowed_values: "{ALLOWED_VALUE_1}"
                            allowed_values: "{ALLOWED_VALUE_2}" }} }}"#
    )));
    parser
        .parse(&[&format!("--test_string={ALLOWED_VALUE_1}")])
        .unwrap();

    // Option should be "foo" as specified by the user.
    assert_eq!(get_test_options(&parser).test_string, ALLOWED_VALUE_1);

    enforce(&enforcer, &mut parser).unwrap();

    // Still "foo" since "foo" is allowed by the policy.
    assert_eq!(get_test_options(&parser).test_string, ALLOWED_VALUE_1);
}

// Java: InvocationPolicyAllowValuesTest.testAllowValuesDisallowsValue
#[test]
fn test_allow_values_disallows_value() {
    let mut parser = set_parser();
    // no foo!
    let enforcer = create_options_policy_enforcer(&policy(&format!(
        r#"flag_policies {{ flag_name: "test_string"
             allow_values {{ allowed_values: "{TEST_STRING_DEFAULT}"
                            allowed_values: "{ALLOWED_VALUE_2}" }} }}"#
    )));
    parser
        .parse(&[&format!("--test_string={ALLOWED_VALUE_1}")])
        .unwrap();

    // Option should be "foo" as specified by the user.
    assert_eq!(get_test_options(&parser).test_string, ALLOWED_VALUE_1);

    // Should throw because "foo" is not allowed.
    assert!(enforce(&enforcer, &mut parser).is_err());
}

// Java: InvocationPolicyAllowValuesTest.testAllowValuesDisallowsMultipleValues
#[test]
fn test_allow_values_disallows_multiple_values() {
    let mut parser = set_parser();
    let enforcer = create_options_policy_enforcer(&policy(&format!(
        r#"flag_policies {{ flag_name: "test_multiple_string"
             allow_values {{ allowed_values: "{ALLOWED_VALUE_1}"
                            allowed_values: "{ALLOWED_VALUE_2}" }} }}"#
    )));
    parser
        .parse(&[
            &format!("--test_multiple_string={UNFILTERED_VALUE}"),
            &format!("--test_multiple_string={ALLOWED_VALUE_2}"),
        ])
        .unwrap();

    // Option should be "baz" and "bar" as specified by the user.
    assert_eq!(
        get_test_options(&parser).test_multiple_string,
        vec![s(UNFILTERED_VALUE), s(ALLOWED_VALUE_2)]
    );

    // expected, since baz is not allowed.
    assert!(enforce(&enforcer, &mut parser).is_err());
}

// Java: InvocationPolicyAllowValuesTest.testAllowValuesSetsNewValue
#[test]
fn test_allow_values_sets_new_value() {
    let mut parser = set_parser();
    let enforcer = create_options_policy_enforcer(&policy(&format!(
        r#"flag_policies {{ flag_name: "test_string"
             allow_values {{ allowed_values: "{ALLOWED_VALUE_1}"
                            allowed_values: "{ALLOWED_VALUE_2}"
                            new_value: "{ALLOWED_VALUE_1}" }} }}"#
    )));
    parser
        .parse(&[&format!("--test_string={UNFILTERED_VALUE}")])
        .unwrap();

    assert_eq!(get_test_options(&parser).test_string, UNFILTERED_VALUE);

    enforce(&enforcer, &mut parser).unwrap();

    assert_eq!(get_test_options(&parser).test_string, ALLOWED_VALUE_1);
}

// Java: InvocationPolicyAllowValuesTest.testAllowValuesSetsDefaultValue
#[test]
fn test_allow_values_sets_default_value() {
    let mut parser = set_parser();
    let enforcer = create_options_policy_enforcer(&policy(&format!(
        r#"flag_policies {{ flag_name: "test_string"
             allow_values {{ allowed_values: "{ALLOWED_VALUE_1}"
                            allowed_values: "{TEST_STRING_DEFAULT}"
                            use_default {{}} }} }}"#
    )));
    parser
        .parse(&[&format!("--test_string={UNFILTERED_VALUE}")])
        .unwrap();

    assert_eq!(get_test_options(&parser).test_string, UNFILTERED_VALUE);

    enforce(&enforcer, &mut parser).unwrap();

    assert_eq!(get_test_options(&parser).test_string, TEST_STRING_DEFAULT);
}

// Java: InvocationPolicyAllowValuesTest.testAllowValuesSetsDefaultValueForRepeatableFlag
#[test]
fn test_allow_values_sets_default_value_for_repeatable_flag() {
    let mut parser = set_parser();
    let enforcer = create_options_policy_enforcer(&policy(&format!(
        r#"flag_policies {{ flag_name: "test_multiple_string"
             allow_values {{ allowed_values: "{ALLOWED_VALUE_1}"
                            allowed_values: "{ALLOWED_VALUE_2}"
                            use_default {{}} }} }}"#
    )));
    parser
        .parse(&[
            &format!("--test_multiple_string={ALLOWED_VALUE_1}"),
            &format!("--test_multiple_string={UNFILTERED_VALUE}"),
        ])
        .unwrap();

    assert_eq!(
        get_test_options(&parser).test_multiple_string,
        vec![s(ALLOWED_VALUE_1), s(UNFILTERED_VALUE)]
    );

    enforce(&enforcer, &mut parser).unwrap();

    // Default value for repeatable flags is always empty.
    assert!(get_test_options(&parser).test_multiple_string.is_empty());
}

/// Tests that AllowValues sets its default value when the user doesn't provide a value and the
/// flag's default value is disallowed.
// Java: InvocationPolicyAllowValuesTest.testAllowValuesSetsNewDefaultWhenFlagDefaultIsDisallowed
#[test]
fn test_allow_values_sets_new_default_when_flag_default_is_disallowed() {
    let mut parser = set_parser();
    // default value from flag's definition is not allowed
    let enforcer = create_options_policy_enforcer(&policy(&format!(
        r#"flag_policies {{ flag_name: "test_string"
             allow_values {{ allowed_values: "{ALLOWED_VALUE_1}"
                            allowed_values: "{ALLOWED_VALUE_2}"
                            new_value: "new default" }} }}"#
    )));

    // Option should be its default
    assert_eq!(get_test_options(&parser).test_string, TEST_STRING_DEFAULT);

    enforce(&enforcer, &mut parser).unwrap();

    // Flag's value should be the default value from the policy.
    assert_eq!(get_test_options(&parser).test_string, "new default");
}

// Java: InvocationPolicyAllowValuesTest.testAllowValuesDisallowsFlagDefaultButNoPolicyDefault
#[test]
fn test_allow_values_disallows_flag_default_but_no_policy_default() {
    let mut parser = set_parser();
    // default value from flag's definition is not allowed, and no alternate default is given.
    let enforcer = create_options_policy_enforcer(&policy(&format!(
        r#"flag_policies {{ flag_name: "test_string"
             allow_values {{ allowed_values: "{ALLOWED_VALUE_1}"
                            allowed_values: "{ALLOWED_VALUE_2}" }} }}"#
    )));

    // Option should be its default
    assert_eq!(get_test_options(&parser).test_string, TEST_STRING_DEFAULT);

    assert!(enforce(&enforcer, &mut parser).is_err());
}

// Java: InvocationPolicyAllowValuesTest.testAllowValuesDisallowsListConverterFlagValues
#[test]
fn test_allow_values_disallows_list_converter_flag_values() {
    let mut parser = set_parser();
    let enforcer = create_options_policy_enforcer(&policy(
        r#"flag_policies { flag_name: "test_list_converters" allow_values { allowed_values: "a" } }"#,
    ));
    parser.parse(&["--test_list_converters=a,b,c"]).unwrap();

    assert_eq!(
        get_test_options(&parser).test_list_converters,
        vec![s("a"), s("b"), s("c")]
    );

    let e = enforce(&enforcer, &mut parser).unwrap_err();
    assert!(
        e.get_message().contains(
            "Flag value 'b' for option '--test_list_converters' is not allowed by invocation \
             policy"
        ),
        "{}",
        e.get_message()
    );
}

const NULL_DEFAULT_POLICY: &str = r#"flag_policies { flag_name: "test_string_null_by_default"
     allow_values { allowed_values: "a" use_default {} } }"#;

// Java: InvocationPolicyAllowValuesTest.testAllowValuesWithNullDefault_AcceptedValue
#[test]
fn test_allow_values_with_null_default_accepted_value() {
    let mut parser = set_parser();
    let enforcer = create_options_policy_enforcer(&policy(NULL_DEFAULT_POLICY));

    // Check the value before invocation policy enforcement.
    parser.parse(&["--test_string_null_by_default=a"]).unwrap();
    assert_eq!(
        get_test_options(&parser).test_string_null_by_default.as_deref(),
        Some("a")
    );

    // Check the value afterwards.
    enforce(&enforcer, &mut parser).unwrap();
    assert_eq!(
        get_test_options(&parser).test_string_null_by_default.as_deref(),
        Some("a")
    );
}

// Java: InvocationPolicyAllowValuesTest.testAllowValuesWithNullDefault_UsesNullDefaultToOverrideUnacceptedValue
#[test]
fn test_allow_values_with_null_default_uses_null_default_to_override_unaccepted_value() {
    let mut parser = set_parser();
    let enforcer = create_options_policy_enforcer(&policy(NULL_DEFAULT_POLICY));

    // Check the value before invocation policy enforcement.
    parser.parse(&["--test_string_null_by_default=b"]).unwrap();
    assert_eq!(
        get_test_options(&parser).test_string_null_by_default.as_deref(),
        Some("b")
    );

    // Check the value afterwards.
    enforce(&enforcer, &mut parser).unwrap();
    assert_eq!(get_test_options(&parser).test_string_null_by_default, None);
}

// Java: InvocationPolicyAllowValuesTest.testAllowValuesWithNullDefault_AllowsUnsetValue
#[test]
fn test_allow_values_with_null_default_allows_unset_value() {
    let mut parser = set_parser();
    let enforcer = create_options_policy_enforcer(&policy(NULL_DEFAULT_POLICY));

    // Check the value before invocation policy enforcement.
    parser.parse(&[]).unwrap();
    assert_eq!(get_test_options(&parser).test_string_null_by_default, None);

    // Check the value afterwards.
    enforce(&enforcer, &mut parser).unwrap();
    assert_eq!(get_test_options(&parser).test_string_null_by_default, None);
}
