//! Port of src/test/java/com/google/devtools/common/options/InvocationPolicyBreakingConditionsTest.java.
//! Tests InvocationPolicies on cases where we expect it to fail gracefully.

mod common;

use common::invocation_policy::*;

// Java: InvocationPolicyBreakingConditionsTest.testFlagPolicyDoesNotApply
#[test]
fn test_flag_policy_does_not_apply() {
    let mut parser = set_parser();
    let enforcer = create_options_policy_enforcer(&policy(
        r#"flag_policies { flag_name: "test_string" commands: "build"
             set_value { flag_value: "policy value" } }"#,
    ));
    parser
        .parse(&[&format!("--test_string={TEST_STRING_USER_VALUE}")])
        .unwrap();

    assert_eq!(
        get_test_options(&parser).test_string,
        TEST_STRING_USER_VALUE
    );

    enforcer
        .enforce(&mut parser, "test", &mut Vec::new())
        .unwrap();

    // Still user value.
    assert_eq!(
        get_test_options(&parser).test_string,
        TEST_STRING_USER_VALUE
    );
}

// Java: InvocationPolicyBreakingConditionsTest.testNonExistantFlagFromPolicy
#[test]
fn test_non_existant_flag_from_policy() {
    let mut parser = set_parser();
    let enforcer = create_options_policy_enforcer(&policy(
        r#"flag_policies { flag_name: "i_do_not_exist"
             set_value { behavior: FINAL_VALUE_IGNORE_OVERRIDES flag_value: "policy value" } }
           flag_policies { flag_name: "test_string"
             set_value { behavior: FINAL_VALUE_IGNORE_OVERRIDES flag_value: "policy value 2" } }"#,
    ));
    parser
        .parse(&[&format!("--test_string={TEST_STRING_USER_VALUE}")])
        .unwrap();

    assert_eq!(
        get_test_options(&parser).test_string,
        TEST_STRING_USER_VALUE
    );

    enforcer
        .enforce(&mut parser, "test", &mut Vec::new())
        .unwrap();

    // Still user value.
    assert_eq!(
        get_test_options(&parser).test_string,
        TEST_STRING_POLICY_VALUE_2
    );
}

// Java: InvocationPolicyBreakingConditionsTest.testOperationNotSet
#[test]
fn test_operation_not_set() {
    let mut parser = set_parser();
    // No operations added to the flag policy
    let enforcer = create_options_policy_enforcer(&policy("flag_policies {}"));
    parser
        .parse(&[&format!("--test_string={TEST_STRING_USER_VALUE}")])
        .unwrap();

    assert_eq!(
        get_test_options(&parser).test_string,
        TEST_STRING_USER_VALUE
    );

    // Shouldn't throw.
    enforcer
        .enforce(&mut parser, "test", &mut Vec::new())
        .unwrap();

    // Still user value.
    assert_eq!(
        get_test_options(&parser).test_string,
        TEST_STRING_USER_VALUE
    );
}
