//! Port of `rules/test/ExclusiveTestStrategy.java`.

use std::io;
use std::sync::Arc;

use blaze_actions::ActionOwner;
use blaze_protos::blaze::TestResultData;
use blaze_vfs::Path;

use crate::actions::ExecException;
use crate::analysis::test::test_action_context::{
    AttemptGroup, TestActionContext, TestOutputs, TestRunnerSpawn, attempt_group_noop,
};
use crate::analysis::test::test_result::TestResult;
use crate::analysis::test::test_runner_action::TestRunnerAction;
use crate::exec::action_execution_context::ActionExecutionContext;

/// Test strategy wrapper called 'exclusive'. It should delegate to a test strategy for local
/// execution.
///
/// This strategy should be registered with a command line identifier of 'exclusive' which will
/// trigger behavior in SkyframeExecutor to schedule test execution sequentially after non-test
/// actions. This ensures streamed test output is not polluted by other action output.
///
/// Note: It's expected that this strategy is largely identical to the one it wraps. Most of the
/// behavior specific to the 'exclusive' strategy is enabled based on the value of the
/// `--test_strategy` flag, not instance methods of this class.
// Java: com/google/devtools/build/lib/rules/test/ExclusiveTestStrategy.java:40 ExclusiveTestStrategy
pub struct ExclusiveTestStrategy {
    parent: Arc<dyn TestActionContext>,
}

impl ExclusiveTestStrategy {
    // Java: com/google/devtools/build/lib/rules/test/ExclusiveTestStrategy.java:43 ExclusiveTestStrategy.<init>
    pub fn new(parent: Arc<dyn TestActionContext>) -> ExclusiveTestStrategy {
        ExclusiveTestStrategy { parent }
    }
}

impl TestActionContext for ExclusiveTestStrategy {
    // Java: com/google/devtools/build/lib/rules/test/ExclusiveTestStrategy.java:47 ExclusiveTestStrategy.createTestRunnerSpawn
    fn create_test_runner_spawn<'a>(
        &'a self,
        test_runner_action: &'a TestRunnerAction,
        action_execution_context: &'a dyn ActionExecutionContext,
    ) -> Result<Box<dyn TestRunnerSpawn<'a> + 'a>, ExecException> {
        self.parent
            .create_test_runner_spawn(test_runner_action, action_execution_context)
    }

    // Java: com/google/devtools/build/lib/rules/test/ExclusiveTestStrategy.java:54 ExclusiveTestStrategy.isTestKeepGoing
    fn is_test_keep_going(&self) -> bool {
        self.parent.is_test_keep_going()
    }

    // Java: com/google/devtools/build/lib/rules/test/ExclusiveTestStrategy.java:59 ExclusiveTestStrategy.newCachedTestResult
    fn new_cached_test_result(
        &self,
        exec_root: &Path,
        action: &TestRunnerAction,
        cached: TestResultData,
        test_outputs: TestOutputs,
    ) -> io::Result<TestResult> {
        self.parent
            .new_cached_test_result(exec_root, action, cached, test_outputs)
    }

    // Java: com/google/devtools/build/lib/rules/test/ExclusiveTestStrategy.java:68 ExclusiveTestStrategy.getAttemptGroup
    fn get_attempt_group(&self, _owner: &ActionOwner, _shard_num: i32) -> Arc<dyn AttemptGroup> {
        // TODO(ulfjack): Exclusive tests run sequentially, and this feature exists to allow faster
        //  aborts of concurrent actions. It's not clear what, if anything, we should do here.
        attempt_group_noop()
    }
}
