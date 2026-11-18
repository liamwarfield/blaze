//! Port of src/test/java/com/google/devtools/common/options/InvocationPolicyBreakingConditionsTest.java.
//! Every test is ignored: needs InvocationPolicyEnforcer (not ported).

// Java: InvocationPolicyBreakingConditionsTest.testFlagPolicyDoesNotApply
#[test]
#[ignore = "needs InvocationPolicyEnforcer (not ported)"]
fn test_flag_policy_does_not_apply() {
    blaze_options::unported!("InvocationPolicyBreakingConditionsTest.testFlagPolicyDoesNotApply");
}

// Java: InvocationPolicyBreakingConditionsTest.testNonExistantFlagFromPolicy
#[test]
#[ignore = "needs InvocationPolicyEnforcer (not ported)"]
fn test_non_existant_flag_from_policy() {
    blaze_options::unported!(
        "InvocationPolicyBreakingConditionsTest.testNonExistantFlagFromPolicy"
    );
}

// Java: InvocationPolicyBreakingConditionsTest.testOperationNotSet
#[test]
#[ignore = "needs InvocationPolicyEnforcer (not ported)"]
fn test_operation_not_set() {
    blaze_options::unported!("InvocationPolicyBreakingConditionsTest.testOperationNotSet");
}
