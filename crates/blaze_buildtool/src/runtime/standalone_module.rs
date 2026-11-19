//! The test half of `standalone/StandaloneModule.registerActionContexts` (the
//! `TestActionContext` implementations) and `ExecutionTool`'s
//! `restrictTo(TestActionContext.class, options.testStrategy)`.
//!
//! PORT: Java keeps `StandaloneModule` in `lib/standalone` and registers into a
//! `ModuleActionContextRegistry.Builder`; it lives here, next to the test summary classes of
//! `lib/runtime`, because this crate is the lowest one that sees both `StandaloneTestStrategy`
//! (blaze_exec) and the executor's `ActionContexts` (blaze_sky_exec). The other contexts
//! `StandaloneModule` registers (`FileWriteActionContext`, `TemplateExpansionContext`, the C++
//! include-scanning dummies) belong to the crates that define them.

use std::sync::Arc;

use blaze_exec::analysis::test::test_action_context::TestActionContext;
use blaze_exec::analysis::test::test_strategy::TestStrategy;
use blaze_exec::exec::ExecCommandEnvironment;
use blaze_exec::exec::standalone_test_strategy::StandaloneTestStrategy;
use blaze_exec::rules::test::ExclusiveTestStrategy;
use blaze_optdefs::command::test_summary_options::TestSummaryOptions;
use blaze_options::Options;
use blaze_protos::failure_details::{self, FailureDetail, failure_detail};
use blaze_sky_exec::skyframe_action_executor::ActionContexts;
use blaze_util::DetailedExitCode;
use blaze_vfs::Path;

/// The command-line identifiers `StandaloneModule` registers `TestActionContext`s under, in
/// registration order.
const TEST_ACTION_CONTEXT_IDENTIFIERS: [&str; 2] = ["exclusive", "standalone"];

/// Registers the `TestActionContext` for this build: `StandaloneModule.registerActionContexts`
/// registers `ExclusiveTestStrategy` ("exclusive") and then `StandaloneTestStrategy`
/// ("standalone", last so that it is the default), and `ExecutionTool` restricts
/// `TestActionContext` to `--test_strategy` (`ExecutionOptions.testStrategy`, read from
/// `env.execution_options`).
///
/// `workspace` is `env.getWorkspace()` (the `--test_tmpdir` base). `test_summary_options` is the
/// command's `TestSummaryOptions`, `None` when the command has none (Java then uses
/// `TestSummaryOptions.DEFAULTS`).
///
/// Errors are the `AbruptExitException` `ModuleActionContextRegistry.Builder.build` throws for a
/// `--test_strategy` that matches no registered identifier.
// Java: com/google/devtools/build/lib/standalone/StandaloneModule.java:50 StandaloneModule.registerActionContexts
// Java: com/google/devtools/build/lib/buildtool/ExecutionTool.java:205 ExecutionTool.<init>
pub fn register_test_action_contexts(
    contexts: &mut ActionContexts,
    env: &ExecCommandEnvironment,
    workspace: &Path,
    test_summary_options: Option<Arc<TestSummaryOptions>>,
) -> Result<(), DetailedExitCode> {
    let execution_options = env.execution_options.clone();
    // It is possible, though unlikely, that the test summary options have not been set. This can
    // happen if a test runner is being run without the test command having been used.
    let test_summary_options = test_summary_options
        .unwrap_or_else(|| Arc::new(Options::<TestSummaryOptions>::get_defaults()));
    let test_tmp_root = TestStrategy::get_tmp_root(workspace, &env.exec_root, &execution_options);
    let test_strategy: Arc<dyn TestActionContext> = Arc::new(StandaloneTestStrategy::new(
        execution_options.clone(),
        test_summary_options,
        test_tmp_root,
    ));

    // Java: com/google/devtools/build/lib/exec/ModuleActionContextRegistry.java:142 ModuleActionContextRegistry.Builder.build
    // The last registered context matching the restriction wins; "" matches every one.
    let restriction = execution_options.test_strategy.as_str();
    if restriction.is_empty() || restriction == "standalone" {
        contexts.register::<Arc<dyn TestActionContext>>(Arc::new(test_strategy));
        return Ok(());
    }
    if restriction == "exclusive" {
        // registryBuilder.register(TestActionContext.class,
        //     new ExclusiveTestStrategy(testStrategy), "exclusive")
        let exclusive: Arc<dyn TestActionContext> =
            Arc::new(ExclusiveTestStrategy::new(test_strategy));
        contexts.register::<Arc<dyn TestActionContext>>(Arc::new(exclusive));
        return Ok(());
    }
    Err(DetailedExitCode::of(FailureDetail {
        message: get_missing_identifier_error_message(restriction),
        category: Some(failure_detail::Category::ExecutionOptions(
            failure_details::ExecutionOptions {
                code: failure_details::execution_options::Code::RestrictionUnmatchedToActionContext
                    as i32,
            },
        )),
    }))
}

/// The message for the one restricted type (`TestActionContext`) no registered context matched.
// Java: com/google/devtools/build/lib/exec/ModuleActionContextRegistry.java:176 ModuleActionContextRegistry.Builder.getMissingIdentifierErrorMessage
fn get_missing_identifier_error_message(restriction: &str) -> String {
    let mut message = String::new();
    message.push_str(&format!(
        "No context of type {} registered for requested value '{}', available identifiers are: [{}]\n",
        "TestActionContext",
        restriction,
        TEST_ACTION_CONTEXT_IDENTIFIERS.join(", ")
    ));
    // Java: Sets.SetView.toString() of the unused Class objects (Class.toString()).
    message.push_str(
        "unused [interface com.google.devtools.build.lib.analysis.test.TestActionContext]",
    );
    message
}

#[cfg(test)]
mod tests {
    use super::*;

    // ModuleActionContextRegistryTest.testUnfulfilledRestriction: the message names the type,
    // the requested value and the available identifiers ("IT1.*bar.*[foo, baz, boz]").
    #[test]
    fn test_unfulfilled_restriction() {
        assert_eq!(
            get_missing_identifier_error_message("foo"),
            "No context of type TestActionContext registered for requested value 'foo', \
             available identifiers are: [exclusive, standalone]\n\
             unused [interface com.google.devtools.build.lib.analysis.test.TestActionContext]"
        );
    }
}
