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

/// Registers the [`crate::actions::ExecutableAction`] implementations of this package
/// (`TestRunnerAction`). Idempotent.
pub fn register() {
    crate::actions::register_executable_action::<test_runner_action::TestRunnerAction>();
}
