//! The analysis test harness: the Rust equivalent of Java's `BuildViewTestCase` /
//! `AnalysisTestCase` (src/test/java/com/google/devtools/build/lib/analysis/util/) with the
//! Bazel analysis mock (`BazelAnalysisMock`, `MockToolsConfig`, `MockPlatformSupport`, ...).
//!
//! Compiled for this crate's tests and, with the `testing` feature, for other crates' tests.
//! See `build_view_test_case.rs` for the differences from the Java test environment.

pub mod analysis_mock;
pub mod build_view_test_case;
pub mod configuration_test_case;
pub mod mock_tools_config;
pub mod scratch;

pub use configuration_test_case::ConfigurationTestCase;

pub use build_view_test_case::{
    AnalysisTestCase, BuildViewTestCase, Flag, FlagBuilder, assert_contains_exactly,
    assert_contains_exactly_in_order, base_artifact_names, exec_paths, get_data_runfiles,
    get_default_runfiles, get_executable, get_files_to_build, get_files_to_run, get_output_group,
    has_error, pretty_artifact_names,
};
