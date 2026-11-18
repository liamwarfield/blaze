//! The test summary classes of `lib/runtime` (`TestSummary`, `TestSummaryPrinter`,
//! `TerminalTestResultNotifier`, `TestResultAggregator`, `AggregatingTestListener`).
//!
//! PORT: Java keeps these in `lib/runtime`; they live here, below `blaze_runtime`, so that
//! `BuildTool`/`TestCommand` logic and their tests do not need the dispatcher crate.

pub mod aggregating_test_listener;
pub mod standalone_module;
pub mod terminal_test_result_notifier;
pub mod test_command_support;
pub mod test_result_aggregator;
pub mod test_summary;
pub mod test_summary_printer;
