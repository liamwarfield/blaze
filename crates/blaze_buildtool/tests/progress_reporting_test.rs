//! Port of `buildtool/ProgressReportingTest.java` (the tests of `ExecutionProgressReceiver` and
//! `ActionExecutionStatusReporter` through a whole build).
//!
//! The Java test is a `BuildIntegrationTestCase`, which has no Rust port; the second and third
//! tests also need `ActionExecutionInactivityWatchdog` (the "Preparing:" / "Scheduling:" status
//! messages of `--progress_report_interval`), which is not ported.

// Java: com/google/devtools/build/lib/buildtool/ProgressReportingTest.java:74 ProgressReportingTest.testAdditionalInfo
#[test]
#[ignore = "needs BuildIntegrationTestCase"]
fn test_additional_info() {}

// Java: com/google/devtools/build/lib/buildtool/ProgressReportingTest.java:94 ProgressReportingTest.testPreparingMessage
#[test]
#[ignore = "needs BuildIntegrationTestCase and ActionExecutionInactivityWatchdog"]
fn test_preparing_message() {}

// Java: com/google/devtools/build/lib/buildtool/ProgressReportingTest.java:128 ProgressReportingTest.testWaitForResources
#[test]
#[ignore = "needs BuildIntegrationTestCase and ActionExecutionInactivityWatchdog"]
fn test_wait_for_resources() {}
