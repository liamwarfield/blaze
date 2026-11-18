//! `AggregatingTestListener` has no JUnit test at the pin; this is a smoke test of the paths that
//! need no `TestResult` (failed-to-build targets and the final exit code), following
//! `AggregatingTestListener.java` and `TestResultAggregator.java`.

mod common;

use std::any::Any;
use std::fmt;
use std::sync::Arc;

use blaze_buildtool::buildtool::build_result::{
    BuildResult, DetailedExitCodeFactory, DetailedExitCodeRef, DetailedExitCodeView,
};
use blaze_buildtool::runtime::aggregating_test_listener::AggregatingTestListener;
use blaze_buildtool::runtime::terminal_test_result_notifier::TestResultNotifier;
use blaze_buildtool::runtime::test_summary::TestSummary;
use blaze_events::{EventBus, Postable};
use blaze_optdefs::command::execution_options::ExecutionOptions;
use blaze_optdefs::command::test_summary_options::TestSummaryOptions;
use blaze_protos::blaze::BlazeTestStatus;
use blaze_protos::failure_details::FailureDetail;
use blaze_sky_exec::analysis_seam::ConfiguredTargetView;
use common::{configuration, target};
use parking_lot::Mutex;

#[derive(Default)]
struct RecordingBus {
    posted: Mutex<Vec<Arc<dyn Postable>>>,
}

impl EventBus for RecordingBus {
    fn post(&self, event: Arc<dyn Postable>) {
        self.posted.lock().push(event);
    }
}

#[derive(Debug)]
struct Code(Option<FailureDetail>);

impl fmt::Display for Code {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self.0)
    }
}

impl DetailedExitCodeView for Code {
    fn is_success(&self) -> bool {
        self.0.is_none()
    }
    fn is_infrastructure_failure(&self) -> bool {
        false
    }
    fn get_failure_detail(&self) -> Option<&FailureDetail> {
        self.0.as_ref()
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

struct Codes;

impl DetailedExitCodeFactory for Codes {
    fn of(&self, failure_detail: FailureDetail) -> DetailedExitCodeRef {
        Arc::new(Code(Some(failure_detail)))
    }
    fn success(&self) -> DetailedExitCodeRef {
        Arc::new(Code(None))
    }
}

#[derive(Default)]
struct RecordingNotifier {
    calls: Mutex<Vec<(Vec<Arc<TestSummary>>, i32)>>,
}

impl TestResultNotifier for RecordingNotifier {
    fn notify(&self, summaries: &[Arc<TestSummary>], number_of_executed_targets: i32) {
        self.calls
            .lock()
            .push((summaries.to_vec(), number_of_executed_targets));
    }
}

#[test]
fn failed_to_build_target_fails_the_test_command() {
    let bus = Arc::new(RecordingBus::default());
    let listener = AggregatingTestListener::new(
        Arc::new(TestSummaryOptions::default()),
        Arc::new(ExecutionOptions::default()),
        bus.clone(),
        Arc::new(Codes),
    );
    let a: Arc<dyn ConfiguredTargetView> = target("pkg", "a_test");
    let b: Arc<dyn ConfiguredTargetView> = target("pkg", "b_test");
    listener.populate_test(&a, configuration(), false);
    listener.populate_test(&b, configuration(), false);

    let mut result = BuildResult::new(1);
    result.set_actual_targets(Some(vec![a.clone(), b.clone()]));
    result.set_successful_targets(Some(vec![b.clone()]));
    result.set_skipped_targets(Some(vec![]));
    listener.build_complete_event(&result);

    let notifier = RecordingNotifier::default();
    let code =
        listener.differential_analyze_and_report(&[a.clone(), b.clone()], &[], None, &notifier);

    assert!(!code.is_success());
    assert_eq!(code.get_failure_detail().unwrap().message, "tests failed");
    let calls = notifier.calls.lock();
    assert_eq!(calls.len(), 1);
    let (summaries, total_run) = &calls[0];
    assert_eq!(*total_run, 0);
    let statuses: Vec<(String, BlazeTestStatus)> = summaries
        .iter()
        .map(|s| (s.get_label().to_string(), s.get_status()))
        .collect();
    // Sorted by status first (NO_STATUS = 0 before FAILED_TO_BUILD = 7).
    assert_eq!(
        statuses,
        vec![
            ("//pkg:b_test".to_string(), BlazeTestStatus::NoStatus),
            ("//pkg:a_test".to_string(), BlazeTestStatus::FailedToBuild),
        ]
    );
    // Both summaries were posted to the event bus.
    assert_eq!(bus.posted.lock().len(), 2);
}
