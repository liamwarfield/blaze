//! Port of src/test/java/com/google/devtools/common/options/InvocationPolicySetValueTest.java.
//! `@TestParameter` cases run as loops inside one test.

mod common;

use common::invocation_policy::*;
use common::test_options::*;

fn s(v: &str) -> String {
    v.to_string()
}

/// Tests that policy overwrites a value when that value is from the user.
// Java: InvocationPolicySetValueTest.finalValueIgnoreOverrides_overwritesUserSetting
#[test]
fn final_value_ignore_overrides_overwrites_user_setting() {
    let mut parser = set_parser();
    let enforcer = create_options_policy_enforcer(&policy(
        r#"flag_policies { flag_name: "test_string"
             set_value { behavior: FINAL_VALUE_IGNORE_OVERRIDES flag_value: "policy value" } }"#,
    ));
    parser
        .parse(&[&format!("--test_string={TEST_STRING_USER_VALUE}")])
        .unwrap();

    assert_eq!(get_test_options(&parser).test_string, TEST_STRING_USER_VALUE);

    enforce(&enforcer, &mut parser).unwrap();

    // Get the options again after policy enforcement.
    assert_eq!(get_test_options(&parser).test_string, TEST_STRING_POLICY_VALUE);
    assert_eq!(
        command_line_forms(&parser),
        vec![
            Some(format!("--test_string={TEST_STRING_USER_VALUE}")),
            Some(format!("--test_string={TEST_STRING_POLICY_VALUE}")),
        ]
    );
}

/// Tests that policy overwrites a value when the user doesn't specify the value (i.e., the value
/// is from the flag's default from its definition).
// Java: InvocationPolicySetValueTest.finalValueIgnoreOverrides_overwritesDefault
#[test]
fn final_value_ignore_overrides_overwrites_default() {
    let mut parser = set_parser();
    let enforcer = create_options_policy_enforcer(&policy(
        r#"flag_policies { flag_name: "test_string"
             set_value { behavior: FINAL_VALUE_IGNORE_OVERRIDES flag_value: "policy value" } }"#,
    ));

    // All the flags should be their default value.
    assert_eq!(get_test_options(&parser).test_string, TEST_STRING_DEFAULT);

    enforce(&enforcer, &mut parser).unwrap();

    // Get the options again after policy enforcement.
    assert_eq!(get_test_options(&parser).test_string, TEST_STRING_POLICY_VALUE);
}

/// Tests that SetValue overwrites the user's value when the flag allows multiple values.
// Java: InvocationPolicySetValueTest.finalValueIgnoreOverrides_flagWithMultipleValues_overwritesUserSetting
#[test]
fn final_value_ignore_overrides_flag_with_multiple_values_overwrites_user_setting() {
    let mut parser = set_parser();
    let enforcer = create_options_policy_enforcer(&policy(
        r#"flag_policies { flag_name: "test_multiple_string"
             set_value { behavior: FINAL_VALUE_IGNORE_OVERRIDES
                         flag_value: "policy value" flag_value: "policy value 2" } }"#,
    ));
    parser
        .parse(&[
            &format!("--test_multiple_string={TEST_STRING_USER_VALUE}"),
            &format!("--test_multiple_string={TEST_STRING_USER_VALUE_2}"),
        ])
        .unwrap();

    // Options should not be modified by running the parser through OptionsPolicyEnforcer.create().
    assert_eq!(
        get_test_options(&parser).test_multiple_string,
        vec![s(TEST_STRING_USER_VALUE), s(TEST_STRING_USER_VALUE_2)]
    );

    enforce(&enforcer, &mut parser).unwrap();

    // Get the options again after policy enforcement.
    assert_eq!(
        get_test_options(&parser).test_multiple_string,
        vec![s(TEST_STRING_POLICY_VALUE), s(TEST_STRING_POLICY_VALUE_2)]
    );
}

/// Tests that policy overwrites the default value when the flag allows multiple values and the
/// user doesn't provide a value.
// Java: InvocationPolicySetValueTest.finalValueIgnoreOverrides_flagWithMultipleValues_overwritesDefault
#[test]
fn final_value_ignore_overrides_flag_with_multiple_values_overwrites_default() {
    let mut parser = set_parser();
    let enforcer = create_options_policy_enforcer(&policy(
        r#"flag_policies { flag_name: "test_multiple_string"
             set_value { behavior: FINAL_VALUE_IGNORE_OVERRIDES
                         flag_value: "policy value" flag_value: "policy value 2" } }"#,
    ));

    // Repeatable flags always default to the empty list.
    assert!(get_test_options(&parser).test_multiple_string.is_empty());

    enforce(&enforcer, &mut parser).unwrap();

    // Options should now be the values from the policy.
    assert_eq!(
        get_test_options(&parser).test_multiple_string,
        vec![s(TEST_STRING_POLICY_VALUE), s(TEST_STRING_POLICY_VALUE_2)]
    );
}

// Java: InvocationPolicySetValueTest.setMultipleValuesForSingleValuedFlag_fails
#[test]
fn set_multiple_values_for_single_valued_flag_fails() {
    for behavior in DEFINED_BEHAVIORS {
        let mut parser = set_parser();
        let enforcer = create_options_policy_enforcer(&policy(&format!(
            r#"flag_policies {{ flag_name: "test_string"
                 set_value {{ behavior: {behavior}
                             flag_value: "policy value" flag_value: "policy value 2" }} }}"#
        )));
        assert!(enforce(&enforcer, &mut parser).is_err(), "{behavior}");
    }
}

// Java: InvocationPolicySetValueTest.append_appendsToMultipleValuedFlag
#[test]
fn append_appends_to_multiple_valued_flag() {
    let mut parser = set_parser();
    let enforcer = create_options_policy_enforcer(&policy(
        r#"flag_policies { flag_name: "test_multiple_string"
             set_value { behavior: APPEND
                         flag_value: "policy value" flag_value: "policy value 2" } }"#,
    ));
    parser
        .parse(&[
            &format!("--test_multiple_string={TEST_STRING_USER_VALUE}"),
            &format!("--test_multiple_string={TEST_STRING_USER_VALUE_2}"),
        ])
        .unwrap();

    assert_eq!(
        get_test_options(&parser).test_multiple_string,
        vec![s(TEST_STRING_USER_VALUE), s(TEST_STRING_USER_VALUE_2)]
    );

    enforce(&enforcer, &mut parser).unwrap();

    assert_eq!(
        get_test_options(&parser).test_multiple_string,
        vec![
            s(TEST_STRING_USER_VALUE),
            s(TEST_STRING_USER_VALUE_2),
            s(TEST_STRING_POLICY_VALUE),
            s(TEST_STRING_POLICY_VALUE_2),
        ]
    );
}

fn assert_expanded_defaults(t: &TestOptions) {
    assert_eq!(t.expanded_a, EXPANDED_A_DEFAULT);
    assert_eq!(t.expanded_b, EXPANDED_B_DEFAULT);
    assert_eq!(t.expanded_c, EXPANDED_C_DEFAULT);
    assert_eq!(t.expanded_d, EXPANDED_D_DEFAULT);
}

fn assert_expanded_test_expansion(t: &TestOptions) {
    assert_eq!(t.expanded_a, EXPANDED_A_TEST_EXPANSION);
    assert_eq!(t.expanded_b, EXPANDED_B_TEST_EXPANSION);
    assert_eq!(t.expanded_c, EXPANDED_C_TEST_EXPANSION);
    assert_eq!(t.expanded_d, EXPANDED_D_TEST_EXPANSION);
}

// Java: InvocationPolicySetValueTest.setFlagWithExpansion_finalValueIgnoreOverrides_setsExpandedValuesAsFinal
#[test]
fn set_flag_with_expansion_final_value_ignore_overrides_sets_expanded_values_as_final() {
    for value in [None, Some(""), Some("some value")] {
        let mut parser = set_parser();
        let flag_value = match value {
            None => String::new(),
            Some(v) => format!("flag_value: \"{v}\""),
        };
        // SetValue must have no values for a Void flag.
        let enforcer = create_options_policy_enforcer(&policy(&format!(
            r#"flag_policies {{ flag_name: "test_expansion"
                 set_value {{ behavior: FINAL_VALUE_IGNORE_OVERRIDES {flag_value} }} }}"#
        )));
        // Unrelated flag, but --test_expansion is not set
        parser.parse(&["--test_string=throwaway value"]).unwrap();

        // The flags that --test_expansion expands into should still be their default values
        assert_expanded_defaults(&get_test_options(&parser));

        enforce(&enforcer, &mut parser).unwrap();

        // After policy enforcement, the flags should be the values from --test_expansion
        assert_expanded_test_expansion(&get_test_options(&parser));
    }
}

// Java: InvocationPolicySetValueTest.finalValueIgnoreOverrides_flagExpandingToExpansion_setsRecursiveValues
#[test]
fn final_value_ignore_overrides_flag_expanding_to_expansion_sets_recursive_values() {
    let mut parser = set_parser();
    let enforcer = create_options_policy_enforcer(&policy(
        r#"flag_policies { flag_name: "test_recursive_expansion_top_level"
             set_value { behavior: FINAL_VALUE_IGNORE_OVERRIDES } }"#,
    ));
    // Unrelated flag, but --test_expansion is not set
    parser.parse(&["--test_string=throwaway value"]).unwrap();

    // The flags that --test_expansion expands into should still be their default values
    assert_expanded_defaults(&get_test_options(&parser));

    enforce(&enforcer, &mut parser).unwrap();

    // After policy enforcement, the flags should be the values from the expansion flag
    let t = get_test_options(&parser);
    assert_eq!(t.expanded_a, EXPANDED_A_TEST_RECURSIVE_EXPANSION);
    assert_eq!(t.expanded_b, EXPANDED_B_TEST_RECURSIVE_EXPANSION);
    assert_eq!(t.expanded_c, EXPANDED_C_TEST_RECURSIVE_EXPANSION);
    assert_eq!(t.expanded_d, EXPANDED_D_TEST_RECURSIVE_EXPANSION);
}

// Java: InvocationPolicySetValueTest.allowOverrides_setFlagWithExpansion_keepsUserSpecifiedFlag
#[test]
fn allow_overrides_set_flag_with_expansion_keeps_user_specified_flag() {
    let mut parser = set_parser();
    // this value is arbitrary, the value for a Void flag is ignored
    let enforcer = create_options_policy_enforcer(&policy(
        r#"flag_policies { flag_name: "test_expansion"
             set_value { behavior: ALLOW_OVERRIDES flag_value: "" } }"#,
    ));
    // Unrelated flag, but --test_expansion is not set
    parser.parse(&["--expanded_c=23"]).unwrap();

    // The flags that --test_expansion expands into should still be their default values
    // except for the explicitly marked flag.
    let t = get_test_options(&parser);
    assert_eq!(t.expanded_a, EXPANDED_B_DEFAULT);
    assert_eq!(t.expanded_b, EXPANDED_B_DEFAULT);
    assert_eq!(t.expanded_c, 23);
    assert_eq!(t.expanded_d, EXPANDED_D_DEFAULT);

    enforce(&enforcer, &mut parser).unwrap();

    // After policy enforcement, the flags should be the values from --test_expansion, except for
    // the user-set value, since the expansion flag was set to overridable.
    let t = get_test_options(&parser);
    assert_eq!(t.expanded_a, EXPANDED_A_TEST_EXPANSION);
    assert_eq!(t.expanded_b, EXPANDED_B_TEST_EXPANSION);
    assert_eq!(t.expanded_c, 23);
    assert_eq!(t.expanded_d, EXPANDED_D_TEST_EXPANSION);
}

// Java: InvocationPolicySetValueTest.allowOverrides_flagExpandingToRepeatingFlag_appendsRepeatedValues
#[test]
fn allow_overrides_flag_expanding_to_repeating_flag_appends_repeated_values() {
    let mut parser = set_parser();
    // SetValue must have no values for a Void flag.
    let enforcer = create_options_policy_enforcer(&policy(
        r#"flag_policies { flag_name: "test_expansion_to_repeatable"
             set_value { behavior: ALLOW_OVERRIDES } }"#,
    ));
    // Unrelated flag, but --test_expansion is not set
    parser.parse(&["--test_multiple_string=foo"]).unwrap();

    assert_eq!(get_test_options(&parser).test_multiple_string, vec![s("foo")]);

    enforce(&enforcer, &mut parser).unwrap();

    // After policy enforcement, the flags should be the values from --test_expansion, except for
    // the user-set value, since the expansion flag was set to overridable.
    let mut got = get_test_options(&parser).test_multiple_string;
    got.sort();
    let mut want = vec![s("foo"), s(EXPANDED_MULTIPLE_1), s(EXPANDED_MULTIPLE_2)];
    want.sort();
    // `containsExactly` without `inOrder`.
    assert_eq!(got, want);
}

// Java: InvocationPolicySetValueTest.finalValueIgnoreOverrides_setFlagWithExpansion_overwritesUserSettingForExpandedFlag
#[test]
fn final_value_ignore_overrides_set_flag_with_expansion_overwrites_user_setting_for_expanded_flag() {
    let mut parser = set_parser();
    // SetValue must have no values for a Void flag.
    let enforcer = create_options_policy_enforcer(&policy(
        r#"flag_policies { flag_name: "test_expansion"
             set_value { behavior: FINAL_VALUE_IGNORE_OVERRIDES } }"#,
    ));
    // Unrelated flag, but --test_expansion is not set
    parser.parse(&["--expanded_c=23"]).unwrap();

    let t = get_test_options(&parser);
    assert_eq!(t.expanded_a, EXPANDED_B_DEFAULT);
    assert_eq!(t.expanded_b, EXPANDED_B_DEFAULT);
    assert_eq!(t.expanded_c, 23);
    assert_eq!(t.expanded_d, EXPANDED_D_DEFAULT);

    enforce(&enforcer, &mut parser).unwrap();

    // After policy enforcement, the flags should be the values from --test_expansion, including
    // the value that the user tried to set, since the expansion flag was set non-overridably.
    assert_expanded_test_expansion(&get_test_options(&parser));
}

// Java: InvocationPolicySetValueTest.finalValueIgnoreOverrides_flagWithExpansionToRepeatingFlag_overwritesUserSetting
#[test]
fn final_value_ignore_overrides_flag_with_expansion_to_repeating_flag_overwrites_user_setting() {
    let mut parser = set_parser();
    // SetValue must have no values for a Void flag.
    let enforcer = create_options_policy_enforcer(&policy(
        r#"flag_policies { flag_name: "test_expansion_to_repeatable"
             set_value { behavior: FINAL_VALUE_IGNORE_OVERRIDES } }"#,
    ));
    // Unrelated flag, but --test_expansion is not set
    parser.parse(&["--test_multiple_string=foo"]).unwrap();

    assert!(
        get_test_options(&parser)
            .test_multiple_string
            .contains(&s("foo"))
    );

    enforce(&enforcer, &mut parser).unwrap();

    // After policy enforcement, the flag should no longer have the user's value.
    let mut got = get_test_options(&parser).test_multiple_string;
    got.sort();
    let mut want = vec![s(EXPANDED_MULTIPLE_1), s(EXPANDED_MULTIPLE_2)];
    want.sort();
    assert_eq!(got, want);
}

// Java: InvocationPolicySetValueTest.finalValueIgnoreOverrides_overwritesUserSettingFromExpandedFlag
#[test]
fn final_value_ignore_overrides_overwrites_user_setting_from_expanded_flag() {
    let mut parser = set_parser();
    let enforcer = create_options_policy_enforcer(&policy(
        r#"flag_policies { flag_name: "expanded_c"
             set_value { behavior: FINAL_VALUE_IGNORE_OVERRIDES flag_value: "64" } }"#,
    ));
    parser.parse(&["--test_expansion"]).unwrap();

    // --test_expansion should turn set the values from its expansion.
    assert_expanded_test_expansion(&get_test_options(&parser));

    enforce(&enforcer, &mut parser).unwrap();

    // After policy enforcement, expanded_c should be set to 64 from the policy, but the
    // flags should remain the same from the expansion of --test_expansion.
    let t = get_test_options(&parser);
    assert_eq!(t.expanded_a, EXPANDED_A_TEST_EXPANSION);
    assert_eq!(t.expanded_b, EXPANDED_B_TEST_EXPANSION);
    assert_eq!(t.expanded_c, 64);
    assert_eq!(t.expanded_d, EXPANDED_D_TEST_EXPANSION);
}

// Java: InvocationPolicySetValueTest.finalValueIgnoreOverrides_overwritesImplicitRequirementFromUserSetting
#[test]
fn final_value_ignore_overrides_overwrites_implicit_requirement_from_user_setting() {
    let mut parser = set_parser();
    let enforcer = create_options_policy_enforcer(&policy(
        r#"flag_policies { flag_name: "implicit_requirement_a"
             set_value { behavior: FINAL_VALUE_IGNORE_OVERRIDES flag_value: "policy value" } }"#,
    ));
    parser
        .parse(&[&format!(
            "--test_implicit_requirement={TEST_STRING_USER_VALUE}"
        )])
        .unwrap();

    // test_implicit_requirement sets implicit_requirement_a to "foo"
    let t = get_test_options(&parser);
    assert_eq!(t.test_implicit_requirement, TEST_STRING_USER_VALUE);
    assert_eq!(t.implicit_requirement_a, IMPLICIT_REQUIREMENT_A_REQUIRED);

    enforce(&enforcer, &mut parser).unwrap();

    let t = get_test_options(&parser);
    assert_eq!(t.test_implicit_requirement, TEST_STRING_USER_VALUE);
    assert_eq!(t.implicit_requirement_a, TEST_STRING_POLICY_VALUE);
}

// Java: InvocationPolicySetValueTest.testSetValueWithImplicitlyRequiredFlags
#[test]
fn test_set_value_with_implicitly_required_flags() {
    let mut parser = set_parser();
    let enforcer = create_options_policy_enforcer(&policy(
        r#"flag_policies { flag_name: "test_implicit_requirement"
             set_value { behavior: FINAL_VALUE_IGNORE_OVERRIDES flag_value: "policy value" } }"#,
    ));
    parser
        .parse(&[&format!("--implicit_requirement_a={TEST_STRING_USER_VALUE}")])
        .unwrap();

    assert_eq!(
        get_test_options(&parser).implicit_requirement_a,
        TEST_STRING_USER_VALUE
    );

    enforce(&enforcer, &mut parser).unwrap();

    let t = get_test_options(&parser);
    assert_eq!(t.test_implicit_requirement, TEST_STRING_POLICY_VALUE);
    assert_eq!(t.implicit_requirement_a, IMPLICIT_REQUIREMENT_A_REQUIRED);
}

// Java: InvocationPolicySetValueTest.allowOverrides_leavesUserSetting
#[test]
fn allow_overrides_leaves_user_setting() {
    let mut parser = set_parser();
    let enforcer = create_options_policy_enforcer(&policy(
        r#"flag_policies { flag_name: "test_string"
             set_value { behavior: ALLOW_OVERRIDES flag_value: "policy value" } }"#,
    ));
    parser
        .parse(&[&format!("--test_string={TEST_STRING_USER_VALUE}")])
        .unwrap();

    assert_eq!(get_test_options(&parser).test_string, TEST_STRING_USER_VALUE);

    enforce(&enforcer, &mut parser).unwrap();

    // Even though the policy sets the value for test_string, the policy is overridable and the
    // user set the value, so it should be the user's value.
    assert_eq!(get_test_options(&parser).test_string, TEST_STRING_USER_VALUE);
}

// Java: InvocationPolicySetValueTest.setFlagValueWithNoValue_fails
#[test]
fn set_flag_value_with_no_value_fails() {
    for behavior in DEFINED_BEHAVIORS {
        let mut parser = set_parser();
        // No value.
        let enforcer = create_options_policy_enforcer(&policy(&format!(
            r#"flag_policies {{ flag_name: "test_string" set_value {{ behavior: {behavior} }} }}"#
        )));
        parser
            .parse(&[&format!("--test_string={TEST_STRING_USER_VALUE}")])
            .unwrap();

        assert_eq!(get_test_options(&parser).test_string, TEST_STRING_USER_VALUE);

        assert!(enforce(&enforcer, &mut parser).is_err(), "{behavior}");
    }
}

// Java: InvocationPolicySetValueTest.enforce_setValueWithUndefinedBehavior_fails
#[test]
fn enforce_set_value_with_undefined_behavior_fails() {
    for has_behavior in [false, true] {
        for flag_name in [
            "test_string",
            "test_expansion",
            "test_implicit_requirement",
        ] {
            let mut parser = set_parser();
            let behavior = if has_behavior {
                "behavior: UNDEFINED"
            } else {
                ""
            };
            let enforcer = create_options_policy_enforcer(&policy(&format!(
                r#"flag_policies {{ flag_name: "{flag_name}"
                     set_value {{ flag_value: "any value" {behavior} }} }}"#
            )));

            let e = enforce(&enforcer, &mut parser).unwrap_err();

            let want = format!(
                "SetValue operation from invocation policy for has an undefined behavior: \
                 flag_name: \"{flag_name}\"\nset_value {{\n"
            );
            assert!(
                e.get_message().starts_with(&want),
                "{:?} does not start with {want:?}",
                e.get_message()
            );
        }
    }
}

// Java: InvocationPolicySetValueTest.enforce_policySettingConfig_fails
#[test]
fn enforce_policy_setting_config_fails() {
    for behavior in DEFINED_BEHAVIORS {
        let mut parser = set_parser();
        let enforcer = create_options_policy_enforcer(&policy(&format!(
            r#"flag_policies {{ flag_name: "config"
                 set_value {{ behavior: {behavior} flag_value: "foo" }} }}"#
        )));
        parser.parse(&[]).unwrap();

        let expected = enforce(&enforcer, &mut parser).unwrap_err();

        let want = format!(
            "Invocation policy is applied after --config expansion, changing config values now \
             would have no effect and is disallowed to prevent confusion. Please remove the \
             following policy : flag_name: \"config\"\nset_value {{\n  flag_value: \"foo\"\n  \
             behavior: {behavior}\n"
        );
        assert!(
            expected.get_message().starts_with(&want),
            "{:?} does not start with {want:?}",
            expected.get_message()
        );
    }
}

// Java: InvocationPolicySetValueTest.enforce_setValueForNonexistentFlag_doesNothing
#[test]
fn enforce_set_value_for_nonexistent_flag_does_nothing() {
    for behavior in DEFINED_BEHAVIORS {
        let mut parser = set_parser();
        let enforcer = create_options_policy_enforcer(&policy(&format!(
            r#"flag_policies {{ flag_name: "nonexistent"
                 set_value {{ behavior: {behavior} flag_value: "hello" }} }}"#
        )));
        parser.parse(&[]).unwrap();

        enforce(&enforcer, &mut parser).unwrap();

        assert_eq!(get_test_options(&parser), TestOptions::default());
    }
}

// Java: InvocationPolicySetValueTest.finalValueThrowOnOverride_throwsOnUserOverride
#[test]
fn final_value_throw_on_override_throws_on_user_override() {
    let mut parser = set_parser();
    let enforcer = create_options_policy_enforcer(&policy(
        r#"flag_policies { flag_name: "test_string"
             custom_error_message: "See {link to test_string policy} for more details."
             set_value { behavior: FINAL_VALUE_THROW_ON_OVERRIDE flag_value: "policy value" } }"#,
    ));
    parser
        .parse(&[&format!("--test_string={TEST_STRING_USER_VALUE}")])
        .unwrap();

    assert_eq!(get_test_options(&parser).test_string, TEST_STRING_USER_VALUE);

    let e = enforce(&enforcer, &mut parser).unwrap_err();
    assert_eq!(
        e.get_message(),
        "User set a value for option '--test_string' which is not permitted by the invocation \
         policy. This flag value will always be overridden to [policy value]. See {link to \
         test_string policy} for more details."
    );
}

// Java: InvocationPolicySetValueTest.finalValueThrowOnOverride_successOnNoUserOverride
#[test]
fn final_value_throw_on_override_success_on_no_user_override() {
    let mut parser = set_parser();
    let enforcer = create_options_policy_enforcer(&policy(
        r#"flag_policies { flag_name: "test_string"
             custom_error_message: "See {link to test_string policy} for more details."
             set_value { behavior: FINAL_VALUE_THROW_ON_OVERRIDE flag_value: "policy value" } }"#,
    ));

    assert_eq!(get_test_options(&parser).test_string, "test string default");

    enforce(&enforcer, &mut parser).unwrap();

    assert_eq!(get_test_options(&parser).test_string, TEST_STRING_POLICY_VALUE);
    assert_eq!(
        command_line_forms(&parser),
        vec![Some(format!("--test_string={TEST_STRING_POLICY_VALUE}"))]
    );
}

// Java: InvocationPolicySetValueTest.finalValueThrowOnOverride_flagWithMultipleValues_throwsOnUserOverride
#[test]
fn final_value_throw_on_override_flag_with_multiple_values_throws_on_user_override() {
    let mut parser = set_parser();
    let enforcer = create_options_policy_enforcer(&policy(
        r#"flag_policies { flag_name: "test_multiple_string"
             custom_error_message: "See {link to test_multiple_string policy} for more details."
             set_value { behavior: FINAL_VALUE_THROW_ON_OVERRIDE
                         flag_value: "policy value" flag_value: "policy value 2" } }"#,
    ));
    parser
        .parse(&[
            &format!("--test_multiple_string={TEST_STRING_USER_VALUE}"),
            &format!("--test_multiple_string={TEST_STRING_USER_VALUE_2}"),
        ])
        .unwrap();

    assert_eq!(
        get_test_options(&parser).test_multiple_string,
        vec![s(TEST_STRING_USER_VALUE), s(TEST_STRING_USER_VALUE_2)]
    );

    let e = enforce(&enforcer, &mut parser).unwrap_err();
    assert_eq!(
        e.get_message(),
        "User set a value for option '--test_multiple_string' which is not permitted by the \
         invocation policy. This flag value will always be overridden to [policy value, policy \
         value 2]. See {link to test_multiple_string policy} for more details."
    );
}

// Java: InvocationPolicySetValueTest.finalValueThrowOnOverride_flagWithMultipleValues_successOnNoUserOverride
#[test]
fn final_value_throw_on_override_flag_with_multiple_values_success_on_no_user_override() {
    let mut parser = set_parser();
    let enforcer = create_options_policy_enforcer(&policy(
        r#"flag_policies { flag_name: "test_multiple_string"
             set_value { behavior: FINAL_VALUE_THROW_ON_OVERRIDE
                         flag_value: "policy value" flag_value: "policy value 2" } }"#,
    ));

    assert!(get_test_options(&parser).test_multiple_string.is_empty());

    enforce(&enforcer, &mut parser).unwrap();

    assert_eq!(
        get_test_options(&parser).test_multiple_string,
        vec![s(TEST_STRING_POLICY_VALUE), s(TEST_STRING_POLICY_VALUE_2)]
    );
}
