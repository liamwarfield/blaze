//! Ports of `analysis/test/*` (agent R): test actions (`TestActionBuilder`, `TestRunnerAction`),
//! `TestProvider`/`TestParams`, `ExecutionInfo` (`testing.ExecutionInfo`),
//! `testing.TestEnvironment`, `InstrumentedFilesInfo` with `coverage_common`, and
//! `AnalysisTestResultInfo`.

pub mod execution_info;
pub mod instrumented_files_collector;
pub mod instrumented_files_info;
pub mod starlark_testing_module;
pub mod test_action_builder;
pub mod test_provider;
pub mod test_runner_action;
pub mod test_target_properties;

pub use instrumented_files_info::register_coverage_common;
pub use starlark_testing_module::register_testing_toplevels;
