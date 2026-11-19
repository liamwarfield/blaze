//! Port of `actions/ActionExecutionStatusReporterTest.java`.

use std::any::Any;
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};

use blaze_actions::actions::action::{AbstractAction, Action, ActionRef, ComputeKeyError};
use blaze_actions::actions::platform_info::empty_platform_info;
use blaze_actions::{ActionKeyContext, ActionOwner, Artifact, ArtifactOwner, ArtifactRoot};
use blaze_buildtool::buildtool::action_execution_status_reporter::ActionExecutionStatusReporter;
use blaze_cmdline::RepositoryMapping;
use blaze_collect::{NestedSetBuilder, Order};
use blaze_events::location::Location;
use blaze_events::{Clock, Event, EventBus, EventHandler};
use blaze_sky_exec::events::{
    ActionStartedEvent, RunningActionEvent, ScanningActionEvent, SchedulingActionEvent,
};
use blaze_util::Fingerprint;
use blaze_vfs::{DigestHashFunction, Path, PathFragment, Root, UnixFileSystem};
use indexmap::IndexMap;
use parking_lot::Mutex;

/// `ManualClock`.
#[derive(Default)]
struct ManualClock(AtomicI64);

impl ManualClock {
    fn advance_millis(&self, millis: i64) {
        self.0.fetch_add(millis, Ordering::SeqCst);
    }
}

impl Clock for ManualClock {
    fn current_time_millis(&self) -> i64 {
        self.0.load(Ordering::SeqCst)
    }
    fn nano_time(&self) -> i64 {
        self.0.load(Ordering::SeqCst) * 1_000_000
    }
}

/// `EventCollectionApparatus(EventKind.ALL_EVENTS)`: the collected events.
#[derive(Default)]
struct Collector(Mutex<Vec<Event>>);

impl EventHandler for Collector {
    fn handle(&self, event: &Event) {
        self.0.lock().push(event.clone());
    }
}

#[derive(Debug)]
struct MockAction {
    base: AbstractAction,
    progress_message: Option<String>,
}

impl Action for MockAction {
    fn abstract_action(&self) -> &AbstractAction {
        &self.base
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn get_mnemonic(&self) -> String {
        "Mock".into()
    }
    fn compute_key(
        &self,
        _: &ActionKeyContext,
        _: Option<&dyn blaze_actions::actions::action::InputMetadataProvider>,
        _: &mut Fingerprint,
    ) -> Result<(), ComputeKeyError> {
        unreachable!()
    }
    fn get_progress_message(
        &self,
        _main_repository_mapping: Option<&RepositoryMapping>,
    ) -> Option<String> {
        self.progress_message.clone()
    }
    fn pretty_print(&self) -> String {
        "default message".into()
    }
}

/// `mockAction(progressMessage)`: owner `NULL_ACTION_OWNER`.
fn mock_action(progress_message: Option<&str>) -> ActionRef {
    static FS: std::sync::LazyLock<blaze_vfs::FileSystemRef> =
        std::sync::LazyLock::new(|| UnixFileSystem::create(DigestHashFunction::Sha256));
    let root = ArtifactRoot::as_source_root(Root::from_path(Path::create(
        PathFragment::create("/outputbase"),
        &FS,
    )));
    let output =
        Artifact::new_source_artifact(root, PathFragment::create("out"), ArtifactOwner::NullOwner);
    let owner = ActionOwner::create_dummy(
        None,
        Location::new("dummy-file", 0, 0),
        "dummy-target-kind",
        "",
        "",
        None,
        false,
        Some(empty_platform_info()),
        Vec::new(),
        IndexMap::new(),
    );
    Arc::new(MockAction {
        base: AbstractAction::new(
            owner,
            NestedSetBuilder::empty_set(Order::STABLE_ORDER),
            vec![output],
        ),
        progress_message: progress_message.map(str::to_owned),
    })
}

struct Fixture {
    events: Arc<Collector>,
    status_reporter: Arc<ActionExecutionStatusReporter>,
    clock: Arc<ManualClock>,
}

// Java: com/google/devtools/build/lib/actions/ActionExecutionStatusReporterTest.java:56 ActionExecutionStatusReporterTest.initializeEventBus
fn initialize_event_bus() -> Fixture {
    let events = Arc::new(Collector::default());
    let clock = Arc::new(ManualClock::default());
    let status_reporter =
        ActionExecutionStatusReporter::create(events.clone(), Some(clock.clone()));
    Fixture {
        events,
        status_reporter,
        clock,
    }
}

impl Fixture {
    fn verify_no_output(&self) {
        self.events.0.lock().clear();
        self.status_reporter.show_currently_executing_actions("");
        assert!(self.events.0.lock().is_empty());
    }

    fn only_message_lines(&self) -> Vec<String> {
        let events = self.events.0.lock();
        assert_eq!(events.len(), 1, "expected exactly one event");
        let message = events[0].get_message().into_owned();
        let mut collapsed = String::new();
        let mut last_space = false;
        for c in message.chars() {
            if c == ' ' {
                if !last_space {
                    collapsed.push(c);
                }
                last_space = true;
            } else {
                collapsed.push(c);
                last_space = false;
            }
        }
        collapsed
            .split('\n')
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .map(str::to_owned)
            .collect()
    }

    fn verify_output(&self, lines: &[&str]) {
        self.events.0.lock().clear();
        self.status_reporter.show_currently_executing_actions("");
        assert_eq!(self.only_message_lines(), lines);
    }

    fn verify_warning_output(&self, lines: &[&str]) {
        self.events.0.lock().clear();
        self.status_reporter
            .warn_about_currently_executing_actions();
        assert_eq!(self.only_message_lines(), lines);
    }

    fn set_scheduling(&self, action: &ActionRef, strategy: &str) {
        self.status_reporter.post(Arc::new(SchedulingActionEvent {
            action: action.clone(),
            strategy: strategy.to_owned(),
        }));
    }

    fn set_preparing(&self, action: &ActionRef) {
        self.status_reporter.post(Arc::new(ActionStartedEvent {
            action: action.clone(),
            nano_time_start: std::time::Instant::now(),
        }));
    }

    fn set_running(&self, action: &ActionRef, strategy: &str) {
        self.status_reporter.post(Arc::new(RunningActionEvent {
            action: action.clone(),
            strategy: strategy.to_owned(),
        }));
    }
}

// Java: com/google/devtools/build/lib/actions/ActionExecutionStatusReporterTest.java:101 ActionExecutionStatusReporterTest.testCategories
#[test]
fn test_categories() {
    let f = initialize_event_bus();
    f.verify_no_output();
    f.verify_warning_output(&["There are no active jobs - stopping the build"]);
    f.set_preparing(&mock_action(Some("action1")));
    f.clock.advance_millis(1000);
    f.verify_warning_output(&["Still waiting for unfinished jobs"]);
    f.set_scheduling(&mock_action(Some("action2")), "remote");
    f.clock.advance_millis(1000);
    f.set_running(&mock_action(Some("action3")), "remote");
    f.clock.advance_millis(1000);
    f.set_running(&mock_action(Some("action4")), "something else");
    f.verify_output(&[
        "Still waiting for 4 jobs to complete:",
        "Preparing:",
        "action1, 3 s",
        "Running (remote):",
        "action3, 1 s",
        "Running (something else):",
        "action4, 0 s",
        "Scheduling:",
        "action2, 2 s",
    ]);
    f.verify_warning_output(&[
        "Still waiting for 3 jobs to complete:",
        "Running (remote):",
        "action3, 1 s",
        "Running (something else):",
        "action4, 0 s",
        "Scheduling:",
        "action2, 2 s",
        "Build will be stopped after these tasks terminate",
    ]);
}

// Java: com/google/devtools/build/lib/actions/ActionExecutionStatusReporterTest.java:125 ActionExecutionStatusReporterTest.testSingleAction
#[test]
fn test_single_action() {
    let f = initialize_event_bus();
    let action = mock_action(Some("action1"));
    f.verify_no_output();
    f.set_preparing(&action);
    f.clock.advance_millis(1200);
    f.verify_output(&[
        "Still waiting for 1 job to complete:",
        "Preparing:",
        "action1, 1 s",
    ]);
    f.clock.advance_millis(5000);

    f.set_scheduling(&action, "remote");
    f.clock.advance_millis(1200);
    // Only started *scheduling* 1200 ms ago, not 6200 ms ago.
    f.verify_output(&[
        "Still waiting for 1 job to complete:",
        "Scheduling:",
        "action1, 1 s",
    ]);
    f.set_running(&action, "remote");
    f.clock.advance_millis(3000);
    // Only started *running* 3000 ms ago, not 4200 ms ago.
    f.verify_output(&[
        "Still waiting for 1 job to complete:",
        "Running (remote):",
        "action1, 3 s",
    ]);
    f.status_reporter.remove(&action);
    f.verify_no_output();
}

// Java: com/google/devtools/build/lib/actions/ActionExecutionStatusReporterTest.java:146 ActionExecutionStatusReporterTest.testDynamicUpdate
#[test]
fn test_dynamic_update() {
    let f = initialize_event_bus();
    let action = mock_action(Some("action1"));
    f.verify_no_output();
    f.set_preparing(&action);
    f.clock.advance_millis(1000);
    f.verify_output(&[
        "Still waiting for 1 job to complete:",
        "Preparing:",
        "action1, 1 s",
    ]);
    f.set_scheduling(&action, "remote");
    f.clock.advance_millis(1000);
    f.verify_output(&[
        "Still waiting for 1 job to complete:",
        "Scheduling:",
        "action1, 1 s",
    ]);
    f.set_running(&action, "remote");
    f.clock.advance_millis(1000);
    f.verify_output(&[
        "Still waiting for 1 job to complete:",
        "Running (remote):",
        "action1, 1 s",
    ]);
    f.clock.advance_millis(1000);

    f.status_reporter.post(Arc::new(ScanningActionEvent {
        action: action.clone(),
    }));
    // Locality strategy was changed, so timer was reset to 0 s.
    f.verify_output(&[
        "Still waiting for 1 job to complete:",
        "Scanning:",
        "action1, 0 s",
    ]);
    f.status_reporter.remove(&action);
    f.verify_no_output();
}

// Java: com/google/devtools/build/lib/actions/ActionExecutionStatusReporterTest.java:168 ActionExecutionStatusReporterTest.testGroups
#[test]
fn test_groups() {
    let f = initialize_event_bus();
    f.verify_no_output();
    let names = [
        "remote1", "remote2", "remote3", "local1", "local2", "local3",
    ];
    let actions: Vec<ActionRef> = names.iter().map(|n| mock_action(Some(n))).collect();

    for a in &actions {
        f.set_scheduling(a, "remote");
        f.clock.advance_millis(1000);
    }

    f.verify_output(&[
        "Still waiting for 6 jobs to complete:",
        "Scheduling:",
        "remote1, 6 s",
        "remote2, 5 s",
        "remote3, 4 s",
        "local1, 3 s",
        "local2, 2 s",
        "local3, 1 s",
    ]);

    for (a, name) in actions.iter().zip(names) {
        f.set_running(
            a,
            if name.starts_with("remote") {
                "remote"
            } else {
                "something else"
            },
        );
        f.clock.advance_millis(2000);
    }

    // Timers got reset because now they are no longer scheduling but running.
    f.verify_output(&[
        "Still waiting for 6 jobs to complete:",
        "Running (remote):",
        "remote1, 12 s",
        "remote2, 10 s",
        "remote3, 8 s",
        "Running (something else):",
        "local1, 6 s",
        "local2, 4 s",
        "local3, 2 s",
    ]);

    f.status_reporter.remove(&actions[0]);
    f.verify_output(&[
        "Still waiting for 5 jobs to complete:",
        "Running (remote):",
        "remote2, 10 s",
        "remote3, 8 s",
        "Running (something else):",
        "local1, 6 s",
        "local2, 4 s",
        "local3, 2 s",
    ]);
}

// Java: com/google/devtools/build/lib/actions/ActionExecutionStatusReporterTest.java:201 ActionExecutionStatusReporterTest.testTruncation
#[test]
fn test_truncation() {
    let f = initialize_event_bus();
    f.verify_no_output();
    let mut actions = Vec::new();
    for i in 1..=100 {
        let a = mock_action(Some(&format!("a{i}")));
        f.set_scheduling(&a, "remote");
        actions.push(a);
        f.clock.advance_millis(1000);
    }
    f.verify_output(&[
        "Still waiting for 100 jobs to complete:",
        "Scheduling:",
        "a1, 100 s",
        "a2, 99 s",
        "a3, 98 s",
        "a4, 97 s",
        "a5, 96 s",
        "a6, 95 s",
        "a7, 94 s",
        "a8, 93 s",
        "a9, 92 s",
        "... 91 more jobs",
    ]);

    for a in actions.iter().take(5) {
        f.set_running(a, "something else");
        f.clock.advance_millis(1000);
    }
    f.verify_output(&[
        "Still waiting for 100 jobs to complete:",
        "Running (something else):",
        "a1, 5 s",
        "a2, 4 s",
        "a3, 3 s",
        "a4, 2 s",
        "a5, 1 s",
        "Scheduling:",
        "a6, 100 s",
        "a7, 99 s",
        "a8, 98 s",
        "a9, 97 s",
        "a10, 96 s",
        "a11, 95 s",
        "a12, 94 s",
        "a13, 93 s",
        "a14, 92 s",
        "... 86 more jobs",
    ]);
}

// Java: com/google/devtools/build/lib/actions/ActionExecutionStatusReporterTest.java:225 ActionExecutionStatusReporterTest.testOrdering
#[test]
fn test_ordering() {
    let f = initialize_event_bus();
    f.verify_no_output();
    f.set_scheduling(&mock_action(Some("a1")), "remote");
    f.clock.advance_millis(1000);
    f.set_preparing(&mock_action(Some("b1")));
    f.clock.advance_millis(1000);
    f.set_preparing(&mock_action(Some("b2")));
    f.clock.advance_millis(1000);
    f.set_scheduling(&mock_action(Some("a2")), "remote");
    f.clock.advance_millis(1000);
    f.verify_output(&[
        "Still waiting for 4 jobs to complete:",
        "Preparing:",
        "b1, 3 s",
        "b2, 2 s",
        "Scheduling:",
        "a1, 4 s",
        "a2, 1 s",
    ]);
}

// Java: com/google/devtools/build/lib/actions/ActionExecutionStatusReporterTest.java:241 ActionExecutionStatusReporterTest.testNoProgressMessage
#[test]
fn test_no_progress_message() {
    let f = initialize_event_bus();
    f.verify_no_output();
    f.set_scheduling(&mock_action(None), "remote");
    f.verify_output(&[
        "Still waiting for 1 job to complete:",
        "Scheduling:",
        "default message, 0 s",
    ]);
}

// Java: com/google/devtools/build/lib/actions/ActionExecutionStatusReporterTest.java:248 ActionExecutionStatusReporterTest.testWaitTimeCalculation
#[test]
fn test_wait_time_calculation() {
    // --progress_report_interval=0
    assert_eq!(ActionExecutionStatusReporter::get_wait_time(0, 0), 10);
    assert_eq!(ActionExecutionStatusReporter::get_wait_time(0, 10), 30);
    assert_eq!(ActionExecutionStatusReporter::get_wait_time(0, 30), 60);
    assert_eq!(ActionExecutionStatusReporter::get_wait_time(0, 60), 60);

    // --progress_report_interval=42
    assert_eq!(ActionExecutionStatusReporter::get_wait_time(42, 0), 42);
    assert_eq!(ActionExecutionStatusReporter::get_wait_time(42, 42), 42);

    // --progress_report_interval=30 (looks like one of the default timeout stages)
    assert_eq!(ActionExecutionStatusReporter::get_wait_time(30, 0), 30);
    assert_eq!(ActionExecutionStatusReporter::get_wait_time(30, 30), 30);
}
