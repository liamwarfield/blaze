//! `lib/analysis/test`: `TestRunnerAction`, `TestResult`, `TestAttempt`, `TestStrategy`,
//! `TestActionContext` and the analysis-time data `TestRunnerAction` carries
//! (`TestTargetProperties`, `TestTargetExecutionSettings`).

pub mod test_action_context;
pub mod test_attempt;
pub mod test_file_name_constants;
pub mod test_result;
pub mod test_runner_action;
pub mod test_strategy;
pub mod test_target_execution_settings;
pub mod test_target_properties;

use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

use blaze_actions::actions::action::Action;
use parking_lot::RwLock;

use test_runner_action::TestRunnerAction;

/// Registers the [`crate::actions::ExecutableAction`] implementations of this package
/// (`TestRunnerAction`). Idempotent.
pub fn register() {
    crate::actions::register_executable_action::<test_runner_action::TestRunnerAction>();
}

type TestRunnerActionView = Box<dyn Fn(&dyn Any) -> Option<Arc<TestRunnerAction>> + Send + Sync>;

fn test_runner_action_views() -> &'static RwLock<HashMap<TypeId, TestRunnerActionView>> {
    static VIEWS: OnceLock<RwLock<HashMap<TypeId, TestRunnerActionView>>> = OnceLock::new();
    VIEWS.get_or_init(|| RwLock::new(HashMap::new()))
}

/// Registers how an action class defined above this crate (the analysis `TestRunnerAction`)
/// exposes its execution half, so that execution code can do Java's
/// `(TestRunnerAction) action` cast (e.g. `TestCompletionFunction`).
///
/// PORT: Java has one `TestRunnerAction` class; the analysis crate's action delegates its
/// execution to this crate's [`TestRunnerAction`] (see
/// `blaze_analysis::test::test_runner_action_execution`).
pub fn register_test_runner_action_view<T: Action + 'static>(
    view: fn(&T) -> Arc<TestRunnerAction>,
) {
    test_runner_action_views().write().insert(
        TypeId::of::<T>(),
        Box::new(move |any: &dyn Any| any.downcast_ref::<T>().map(view)),
    );
}

/// Java `(TestRunnerAction) action`: the [`TestRunnerAction`] behind `action`, or `None` if it is
/// not a test runner action.
pub fn as_test_runner_action(action: &dyn Action) -> Option<Arc<TestRunnerAction>> {
    let any = action.as_any();
    if let Some(a) = any.downcast_ref::<TestRunnerAction>() {
        return Some(a.arc());
    }
    let views = test_runner_action_views().read();
    let view = views.get(&any.type_id())?;
    view(any)
}
