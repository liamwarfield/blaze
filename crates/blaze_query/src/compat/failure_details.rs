//! Conversions between the crates' reductions of Java's `DetailedExitCode` / `FailureDetail`.
//!
//! PORT: Java has one `DetailedExitCode` (lib/util). blaze_packages keeps a message plus a
//! `PackageLoading` code, blaze_cmdline's `TargetParsingException` a `TargetPatterns` code, and
//! blaze_query_engine the full `FailureDetail`. Query works with the engine's; these functions
//! rebuild the `FailureDetail` the Java object would hold.

use blaze_cmdline::TargetParsingException;
use blaze_packages::exceptions::DetailedExitCode as PackagesDetailedExitCode;
use blaze_protos::failure_details::{FailureDetail, TargetPatterns, failure_detail::Category};
use blaze_query_engine::DetailedExitCode;

/// The `FailureDetail` of a blaze_packages `DetailedExitCode`.
pub fn packages_failure_detail(detailed_exit_code: &PackagesDetailedExitCode) -> FailureDetail {
    detailed_exit_code
        .to_util()
        .get_failure_detail()
        .expect("a DetailedExitCode of a FailureDetail")
        .clone()
}

/// Java `DetailedExitCode.of(failureDetail)` for a blaze_packages `DetailedExitCode`.
pub fn from_packages(detailed_exit_code: &PackagesDetailedExitCode) -> DetailedExitCode {
    DetailedExitCode::of(packages_failure_detail(detailed_exit_code))
}

/// Java `TargetParsingException.getDetailedExitCode().getFailureDetail()`.
// Java: com/google/devtools/build/lib/cmdline/TargetParsingException.java:76 TargetParsingException.getDetailedExitCode
pub fn target_parsing_failure_detail(e: &TargetParsingException) -> FailureDetail {
    FailureDetail {
        message: e.to_string(),
        category: Some(Category::TargetPatterns(TargetPatterns {
            code: e.get_code() as i32,
        })),
    }
}
