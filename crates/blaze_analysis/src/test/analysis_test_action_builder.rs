//! Port of `analysis/test/AnalysisTestActionBuilder.java`: the test script of an analysis test
//! rule.

use std::sync::Arc;

use blaze_actions::ActionRef;
use blaze_actions::analysis::actions::file_write_action::FileWriteAction;

use crate::rule::rule_context::RuleContext;
use crate::test::execution_info::AnalysisTestResultInfo;

// Java: com/google/devtools/build/lib/analysis/test/AnalysisTestActionBuilder.java:34 AnalysisTestActionBuilder
/// Helper for writing test actions for analysis test rules. Analysis test rules are restricted
/// to disallow the rule implementation functions from registering actions themselves; such rules
/// register test success/failure via `AnalysisTestResultInfo`. This helper registers the
/// appropriate test script simulating success or failure of the test.
pub struct AnalysisTestActionBuilder;

impl AnalysisTestActionBuilder {
    // Java: com/google/devtools/build/lib/analysis/test/AnalysisTestActionBuilder.java:43 AnalysisTestActionBuilder.writeAnalysisTestAction
    /// Register an action to write a test script to the default executable location. The script
    /// should return exit status 0 if the test passed. It should print the failure message and
    /// return exit status 1 if the test failed.
    pub fn write_analysis_test_action(
        rule_context: &RuleContext,
        test_result_info: &AnalysisTestResultInfo,
    ) {
        let on_windows = rule_context.is_default_exec_group_executing_on_windows();
        let escaped_message = if on_windows {
            test_result_info.get_message().replace('%', "%%")
        } else {
            // Prefix each character with \ (double-escaped; once in the string, once in the
            // replacement sequence, which allows backslash-escaping literal "$").
            // Java's replaceAll("(.)", "\\\\$1"): `.` does not match line terminators
            // (\n, \r, \u0085,  ,  ).
            let mut out = String::new();
            for c in test_result_info.get_message().chars() {
                if !matches!(c, '\n' | '\r' | '\u{85}' | '\u{2028}' | '\u{2029}') {
                    out.push('\\');
                }
                out.push(c);
            }
            out
        };
        let mut sb = String::new();
        if on_windows {
            sb.push_str("@echo off\n");
        } else {
            sb.push_str("#!/bin/sh\n");
        }
        // Splitter.on("\n").split(escapedMessage)
        for line in escaped_message.split('\n') {
            sb.push_str("echo ");
            sb.push_str(line);
            sb.push('\n');
        }
        sb.push_str("exit ");
        if on_windows {
            sb.push_str("/b ");
        }
        sb.push_str(if test_result_info.get_success() {
            "0"
        } else {
            "1"
        });
        let action: ActionRef = Arc::new(FileWriteAction::create_from_context(
            rule_context.get_action_owner(),
            rule_context.create_output_artifact_script_for_analysis_test(),
            sb,
            /* make_executable= */ true,
        ));
        rule_context.register_action(action);
    }
}
