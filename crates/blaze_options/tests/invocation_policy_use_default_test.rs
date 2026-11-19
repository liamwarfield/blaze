//! Port of src/test/java/com/google/devtools/common/options/InvocationPolicyUseDefaultTest.java.

mod common;

use common::invocation_policy::*;
use common::test_options::*;

fn assert_expanded(t: &TestOptions, a: bool, b: bool, c: i32, d: &str) {
    assert_eq!(t.expanded_a, a);
    assert_eq!(t.expanded_b, b);
    assert_eq!(t.expanded_c, c);
    assert_eq!(t.expanded_d, d);
}

// Java: InvocationPolicyUseDefaultTest.testUseDefault
#[test]
fn test_use_default() {
    let mut parser = set_parser();
    let enforcer = create_options_policy_enforcer(&policy(
        r#"flag_policies { flag_name: "test_string" use_default {} }"#,
    ));
    parser
        .parse(&[&format!("--test_string={TEST_STRING_USER_VALUE}")])
        .unwrap();

    // Options should be the user specified value before enforcing policy.
    assert_eq!(
        get_test_options(&parser).test_string,
        TEST_STRING_USER_VALUE
    );

    enforce(&enforcer, &mut parser).unwrap();

    // Get the options again after policy enforcement: The flag should now be back to its default
    // value
    assert_eq!(get_test_options(&parser).test_string, TEST_STRING_DEFAULT);
}

/// Tests UseDefault when the user never actually specified the flag.
// Java: InvocationPolicyUseDefaultTest.testUseDefaultWhenFlagWasntSet
#[test]
fn test_use_default_when_flag_wasnt_set() {
    let mut parser = set_parser();
    let enforcer = create_options_policy_enforcer(&policy(
        r#"flag_policies { flag_name: "test_string" use_default {} }"#,
    ));

    // Options should be the default since the user never specified it.
    assert_eq!(get_test_options(&parser).test_string, TEST_STRING_DEFAULT);

    enforce(&enforcer, &mut parser).unwrap();

    // Still the default.
    assert_eq!(get_test_options(&parser).test_string, TEST_STRING_DEFAULT);
}

// Java: InvocationPolicyUseDefaultTest.testUseDefaultWithExpansionFlags
#[test]
fn test_use_default_with_expansion_flags() {
    let mut parser = set_parser();
    let enforcer = create_options_policy_enforcer(&policy(
        r#"flag_policies { flag_name: "test_expansion" use_default {} }"#,
    ));
    parser.parse(&["--test_expansion"]).unwrap();

    assert_expanded(
        &get_test_options(&parser),
        EXPANDED_A_TEST_EXPANSION,
        EXPANDED_B_TEST_EXPANSION,
        EXPANDED_C_TEST_EXPANSION,
        EXPANDED_D_TEST_EXPANSION,
    );

    enforce(&enforcer, &mut parser).unwrap();

    // After policy enforcement, all the flags that --test_expansion expanded into should be back
    // to their default values.
    assert_expanded(
        &get_test_options(&parser),
        EXPANDED_A_DEFAULT,
        EXPANDED_B_DEFAULT,
        EXPANDED_C_DEFAULT,
        EXPANDED_D_DEFAULT,
    );
}

// Java: InvocationPolicyUseDefaultTest.testUseDefaultWithExpansionFlagAndLaterOverride
#[test]
fn test_use_default_with_expansion_flag_and_later_override() {
    let mut parser = set_parser();
    let enforcer = create_options_policy_enforcer(&policy(
        r#"flag_policies { flag_name: "test_expansion" use_default {} }
           flag_policies { flag_name: "expanded_b" allow_values { allowed_values: "false" } }"#,
    ));
    parser.parse(&["--test_expansion"]).unwrap();

    assert_expanded(
        &get_test_options(&parser),
        EXPANDED_A_TEST_EXPANSION,
        EXPANDED_B_TEST_EXPANSION,
        EXPANDED_C_TEST_EXPANSION,
        EXPANDED_D_TEST_EXPANSION,
    );

    // If the UseDefault is run, then the value of --expanded_b is back to it's default true,
    // which isn't allowed. However, the allowValues in the later policy should wipe the
    // expansion's policy on --expanded_b, so that the enforcement does not fail.
    enforce(&enforcer, &mut parser).unwrap();

    assert_expanded(
        &get_test_options(&parser),
        EXPANDED_A_DEFAULT,
        EXPANDED_B_TEST_EXPANSION,
        EXPANDED_C_DEFAULT,
        EXPANDED_D_DEFAULT,
    );
}

// Java: InvocationPolicyUseDefaultTest.testUseDefaultWithRecursiveExpansionFlags
#[test]
fn test_use_default_with_recursive_expansion_flags() {
    let mut parser = set_parser();
    let enforcer = create_options_policy_enforcer(&policy(
        r#"flag_policies { flag_name: "test_expansion" use_default {} }"#,
    ));
    parser
        .parse(&["--test_recursive_expansion_top_level"])
        .unwrap();

    assert_expanded(
        &get_test_options(&parser),
        EXPANDED_A_TEST_RECURSIVE_EXPANSION,
        EXPANDED_B_TEST_RECURSIVE_EXPANSION,
        EXPANDED_C_TEST_RECURSIVE_EXPANSION,
        EXPANDED_D_TEST_RECURSIVE_EXPANSION,
    );

    enforce(&enforcer, &mut parser).unwrap();

    // After policy enforcement, all the flags that --test_recursive_expansion_top_level and its
    // recursive expansions set should be back to their default values.
    assert_expanded(
        &get_test_options(&parser),
        EXPANDED_A_DEFAULT,
        EXPANDED_B_DEFAULT,
        EXPANDED_C_DEFAULT,
        EXPANDED_D_DEFAULT,
    );
}

// Java: InvocationPolicyUseDefaultTest.testUseDefaultWithExpandedFlags
#[test]
fn test_use_default_with_expanded_flags() {
    let mut parser = set_parser();
    let enforcer = create_options_policy_enforcer(&policy(
        r#"flag_policies { flag_name: "expanded_b" use_default {} }"#,
    ));
    parser.parse(&["--test_expansion"]).unwrap();

    // --test_expansion should turn set the values from its expansion
    assert_expanded(
        &get_test_options(&parser),
        EXPANDED_A_TEST_EXPANSION,
        EXPANDED_B_TEST_EXPANSION,
        EXPANDED_C_TEST_EXPANSION,
        EXPANDED_D_TEST_EXPANSION,
    );

    enforce(&enforcer, &mut parser).unwrap();

    // After policy enforcement, expanded_b should be back to its default (true), but the rest
    // should remain the same.
    assert_expanded(
        &get_test_options(&parser),
        EXPANDED_A_TEST_EXPANSION,
        EXPANDED_B_DEFAULT,
        EXPANDED_C_TEST_EXPANSION,
        EXPANDED_D_TEST_EXPANSION,
    );
}

// Java: InvocationPolicyUseDefaultTest.testUseDefaultWithFlagWithImplicitRequirements
#[test]
fn test_use_default_with_flag_with_implicit_requirements() {
    let mut parser = set_parser();
    let enforcer = create_options_policy_enforcer(&policy(
        r#"flag_policies { flag_name: "test_implicit_requirement" use_default {} }"#,
    ));
    parser
        .parse(&[&format!(
            "--test_implicit_requirement={TEST_STRING_USER_VALUE}"
        )])
        .unwrap();

    // test_implicit_requirement sets implicit_requirement_a to "foo", which ignores the user's
    // value because the parser processes implicit values last.
    let t = get_test_options(&parser);
    assert_eq!(t.test_implicit_requirement, TEST_STRING_USER_VALUE);
    assert_eq!(t.implicit_requirement_a, IMPLICIT_REQUIREMENT_A_REQUIRED);

    // Then policy puts test_implicit_requirement and its implicit requirements back to its
    // default.
    enforce(&enforcer, &mut parser).unwrap();

    let t = get_test_options(&parser);
    assert_eq!(
        t.test_implicit_requirement,
        TEST_IMPLICIT_REQUIREMENT_DEFAULT
    );
    assert_eq!(t.implicit_requirement_a, IMPLICIT_REQUIREMENT_A_DEFAULT);
}

// Java: InvocationPolicyUseDefaultTest.testUseDefaultWithImplicitlyRequiredFlag
#[test]
fn test_use_default_with_implicitly_required_flag() {
    let mut parser = set_parser();
    let enforcer = create_options_policy_enforcer(&policy(
        r#"flag_policies { flag_name: "implicit_requirement_a" use_default {} }"#,
    ));
    parser
        .parse(&[
            &format!("--test_implicit_requirement={TEST_STRING_USER_VALUE}"),
            &format!("--implicit_requirement_a={TEST_STRING_USER_VALUE}"),
        ])
        .unwrap();

    // test_implicit_requirement sets implicit_requirement_a to "foo", but it gets overwritten by
    // the user value.
    let t = get_test_options(&parser);
    assert_eq!(t.test_implicit_requirement, TEST_STRING_USER_VALUE);
    assert_eq!(t.implicit_requirement_a, TEST_STRING_USER_VALUE);

    // Then policy puts implicit_requirement_a back to its default. This is "broken" since it
    // wipes the user value, but this is the behavior that was agreed on and is documented for
    // expansion flags as well.
    enforce(&enforcer, &mut parser).unwrap();

    let t = get_test_options(&parser);
    assert_eq!(t.test_implicit_requirement, TEST_STRING_USER_VALUE);
    assert_eq!(t.implicit_requirement_a, IMPLICIT_REQUIREMENT_A_DEFAULT);
}

// Java: InvocationPolicyUseDefaultTest.testUseDefaultWithFlagWithRecursiveImplicitRequirements
#[test]
fn test_use_default_with_flag_with_recursive_implicit_requirements() {
    let mut parser = set_parser();
    let enforcer = create_options_policy_enforcer(&policy(
        r#"flag_policies { flag_name: "test_recursive_implicit_requirement" use_default {} }"#,
    ));
    parser
        .parse(&[&format!(
            "--test_recursive_implicit_requirement={TEST_STRING_USER_VALUE}"
        )])
        .unwrap();

    // test_recursive_implicit_requirement gets its value from the command line,
    // test_implicit_requirement gets its value from test_recursive_implicit_requirement, and
    // implicit_requirement_a gets its value from test_implicit_requirement.
    let t = get_test_options(&parser);
    assert_eq!(
        t.test_recursive_implicit_requirement,
        TEST_STRING_USER_VALUE
    );
    assert_eq!(
        t.test_implicit_requirement,
        TEST_IMPLICIT_REQUIREMENT_REQUIRED
    );
    assert_eq!(t.implicit_requirement_a, IMPLICIT_REQUIREMENT_A_REQUIRED);

    enforce(&enforcer, &mut parser).unwrap();

    // Policy enforcement should set everything back to its default value.
    let t = get_test_options(&parser);
    assert_eq!(
        t.test_recursive_implicit_requirement,
        TEST_RECURSIVE_IMPLICIT_REQUIREMENT_DEFAULT
    );
    assert_eq!(
        t.test_implicit_requirement,
        TEST_IMPLICIT_REQUIREMENT_DEFAULT
    );
    assert_eq!(t.implicit_requirement_a, IMPLICIT_REQUIREMENT_A_DEFAULT);
}
