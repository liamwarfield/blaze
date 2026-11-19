//! Port of src/test/java/com/google/devtools/common/options/InvocationPolicyMiscTest.java.

mod common;

use common::invocation_policy::*;

const TEST_DEPRECATED_USER_VALUE: &str = "user value";

const DEPRECATION_WARNING: &str =
    "Option 'test_deprecated' is deprecated: Flag for testing deprecation behavior.";

/// Test that deprecated flags set via setValue in the invocation policy don't elicit an extra
/// deprecation warning on top of the one elicted by the user setting the flag.
// Java: InvocationPolicyMiscTest.testDoPrintDeprecationWarning_setValue
#[test]
fn test_do_print_deprecation_warning_set_value() {
    let mut parser = set_parser();
    parser
        .parse(&[&format!("--test_deprecated={TEST_DEPRECATED_USER_VALUE}")])
        .unwrap();
    let enforcer = create_options_policy_enforcer(&policy(
        r#"flag_policies { flag_name: "test_deprecated" use_default {} }"#,
    ));
    enforce(&enforcer, &mut parser).unwrap();
    assert_eq!(parser.get_warnings(), vec![DEPRECATION_WARNING.to_string()]);
}

/// Test that deprecated flags set via UseDefault in the invocation policy don't elicit an extra
/// deprecation warning on top of the one elicted by the user setting the flag.
// Java: InvocationPolicyMiscTest.testDoPrintDeprecationWarning_useDefault
#[test]
fn test_do_print_deprecation_warning_use_default() {
    let mut parser = set_parser();
    parser
        .parse(&[&format!("--test_deprecated={TEST_DEPRECATED_USER_VALUE}")])
        .unwrap();
    let enforcer = create_options_policy_enforcer(&policy(
        r#"flag_policies { flag_name: "test_deprecated"
             set_value { behavior: FINAL_VALUE_IGNORE_OVERRIDES flag_value: "policy value" } }"#,
    ));
    enforce(&enforcer, &mut parser).unwrap();
    assert_eq!(parser.get_warnings(), vec![DEPRECATION_WARNING.to_string()]);
}

/// Test that deprecated flags touched via UseDefault in the invocation policy don't elicit a
/// deprecation warning.
// Java: InvocationPolicyMiscTest.testDontPrintDeprecationWarning_useDefault
#[test]
fn test_dont_print_deprecation_warning_use_default() {
    let mut parser = set_parser();
    let enforcer = create_options_policy_enforcer(&policy(
        r#"flag_policies { flag_name: "test_deprecated" use_default {} }"#,
    ));
    enforce(&enforcer, &mut parser).unwrap();
    assert!(parser.get_warnings().is_empty());
}

/// Test that deprecated flags set via SetValue in the invocation policy don't elicit a
/// deprecation warning.
// Java: InvocationPolicyMiscTest.testDontPrintDeprecatioNWarning_setValue
#[test]
fn test_dont_print_deprecatio_n_warning_set_value() {
    let mut parser = set_parser();
    let enforcer = create_options_policy_enforcer(&policy(
        r#"flag_policies { flag_name: "test_deprecated"
             set_value { behavior: FINAL_VALUE_IGNORE_OVERRIDES flag_value: "policy value" } }"#,
    ));
    enforce(&enforcer, &mut parser).unwrap();
    assert!(parser.get_warnings().is_empty());
}

// Java: InvocationPolicyMiscTest.testFlagPolicy_oldNameAndNewName_oldNameLast
#[test]
fn test_flag_policy_old_name_and_new_name_old_name_last() {
    let mut parser = set_parser();
    let enforcer = create_options_policy_enforcer(&policy(
        r#"flag_policies { flag_name: "test_new_and_old_name"
             set_value { behavior: FINAL_VALUE_IGNORE_OVERRIDES flag_value: "new_value" } }
           flag_policies { flag_name: "test_old_name"
             set_value { behavior: FINAL_VALUE_IGNORE_OVERRIDES flag_value: "old_value" } }"#,
    ));
    enforce(&enforcer, &mut parser).unwrap();
    assert_eq!(get_test_options(&parser).test_new_and_old_name, "old_value");
}

// Java: InvocationPolicyMiscTest.testFlagPolicy_oldNameAndNewName_newNameLast
#[test]
fn test_flag_policy_old_name_and_new_name_new_name_last() {
    let mut parser = set_parser();
    let enforcer = create_options_policy_enforcer(&policy(
        r#"flag_policies { flag_name: "test_old_name"
             set_value { behavior: FINAL_VALUE_IGNORE_OVERRIDES flag_value: "old_value" } }
           flag_policies { flag_name: "test_new_and_old_name"
             set_value { behavior: FINAL_VALUE_IGNORE_OVERRIDES flag_value: "new_value" } }"#,
    ));
    enforce(&enforcer, &mut parser).unwrap();
    assert_eq!(get_test_options(&parser).test_new_and_old_name, "new_value");
}
