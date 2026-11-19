//! Port of `runtime/UiStateTrackerTest.java`.
//!
//! The Java test is parameterised over `isSkymeld` (`UiStateTracker` and
//! `SkymeldUiStateTracker`); each Rust test runs both. The events of the Java test are the
//! arguments of the tracker's methods (see `ui_state_tracker.rs`).

use std::any::Any;
use std::collections::HashSet;
use std::fmt;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};

use blaze_actions::actions::action::{AbstractAction, Action, ActionRef, ComputeKeyError};
use blaze_actions::actions::action_lookup::ActionLookupKey;
use blaze_actions::actions::platform_info::empty_platform_info;
use blaze_actions::{
    ActionKeyContext, ActionLookupData, ActionLookupKeyRef, ActionOwner, Artifact, ArtifactOwner,
    ArtifactRoot,
};
use blaze_buildtool::runtime::ui_state_tracker::{
    AnalysisProgress, BuildStatus, ExecutionProgress, PackageProgress, StrategyIds, TransportId,
    UiStateTracker,
};
use blaze_cmdline::{Label, RepositoryMapping};
use blaze_collect::{NestedSetBuilder, Order};
use blaze_events::Clock;
use blaze_events::location::Location;
use blaze_events::terminal_writers::LoggingTerminalWriter;
use blaze_protos::blaze::BlazeTestStatus;
use blaze_util::Fingerprint;
use blaze_vfs::{DigestHashFunction, Path, PathFragment, Root, UnixFileSystem};
use indexmap::IndexMap;

// ---------------------------------------------------------------------------------------------
// Test utilities
// ---------------------------------------------------------------------------------------------

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

fn shared_fs() -> blaze_vfs::FileSystemRef {
    static FS: std::sync::LazyLock<blaze_vfs::FileSystemRef> =
        std::sync::LazyLock::new(|| UnixFileSystem::create(DigestHashFunction::Sha256));
    FS.clone()
}

/// `ActionsTestUtil.createArtifact(ArtifactRoot.asSourceRoot(Root.fromPath(outputBase)), path)`.
fn artifact(primary_output: &str) -> Artifact {
    let root = ArtifactRoot::as_source_root(Root::from_path(Path::create(
        PathFragment::create("/outputbase"),
        &shared_fs(),
    )));
    Artifact::new_source_artifact(
        root,
        PathFragment::create(primary_output),
        ArtifactOwner::NullOwner,
    )
}

/// An owner Java's mock would return as `null`: no label, no mnemonic.
fn null_owner() -> ActionOwner {
    owner(None, "", "")
}

fn owner(label: Option<&str>, mnemonic: &str, checksum: &str) -> ActionOwner {
    ActionOwner::create_dummy(
        label.map(|l| Label::parse_canonical(l).unwrap()),
        Location::new("dummy-file", 0, 0),
        "dummy-target-kind",
        mnemonic,
        checksum,
        None,
        false,
        Some(empty_platform_info()),
        Vec::new(),
        IndexMap::new(),
    )
}

/// `dummyActionOwner()`.
fn dummy_action_owner() -> ActionOwner {
    owner(Some("//foo:a"), "", "")
}

/// `mockAction(progressMessage, primaryOutput)`.
#[derive(Debug)]
struct MockAction {
    base: AbstractAction,
    progress_message: String,
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
        Some(self.progress_message.clone())
    }
}

fn mock_action(progress_message: &str, primary_output: &str) -> ActionRef {
    mock_action_with_owner(progress_message, primary_output, null_owner())
}

fn mock_action_with_owner(
    progress_message: &str,
    primary_output: &str,
    owner: ActionOwner,
) -> ActionRef {
    Arc::new(MockAction {
        base: AbstractAction::new(
            owner,
            NestedSetBuilder::empty_set(Order::STABLE_ORDER),
            vec![artifact(primary_output)],
        ),
        progress_message: progress_message.to_owned(),
    })
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct NullActionLookupKey;

impl fmt::Display for NullActionLookupKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("NULL")
    }
}

impl ActionLookupKey for NullActionLookupKey {
    fn get_label(&self) -> Option<Label> {
        None
    }
    fn get_configuration_checksum(&self) -> Option<String> {
        None
    }
    fn function_name(&self) -> &'static str {
        "NULL"
    }
    fn key_equals(&self, other: &dyn ActionLookupKey) -> bool {
        other.as_any().is::<NullActionLookupKey>()
    }
    fn key_hash(&self) -> u64 {
        let mut h = DefaultHasher::new();
        self.hash(&mut h);
        h.finish()
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

fn action_lookup_data() -> ActionLookupData {
    ActionLookupData::create(ActionLookupKeyRef::new(NullActionLookupKey), 1)
}

/// `new ExecutionProgressReceiver(0, null)`.
struct DummyExecutionProgressReceiver;

impl ExecutionProgress for DummyExecutionProgressReceiver {
    fn get_progress_string(&self) -> String {
        "[0 / 0]".into()
    }
    fn has_actions_in_flight(&self) -> bool {
        false
    }
    fn action_completed(&self, _action_lookup_data: &ActionLookupData) {}
}

struct FixedPackageProgress(String, String);

impl PackageProgress for FixedPackageProgress {
    fn progress_state(&self) -> (String, String) {
        (self.0.clone(), self.1.clone())
    }
}

struct FixedAnalysisProgress(String);

impl AnalysisProgress for FixedAnalysisProgress {
    fn get_progress_string(&self) -> String {
        self.0.clone()
    }
}

/// `getUiStateTracker(clock[, targetWidth])` for both values of `isSkymeld`.
fn trackers(clock: &Arc<ManualClock>, target_width: i32) -> Vec<(bool, UiStateTracker)> {
    vec![
        (false, UiStateTracker::new(clock.clone(), target_width)),
        (
            true,
            UiStateTracker::new_skymeld(clock.clone(), target_width),
        ),
    ]
}

/// `simulateExecutionPhase(uiStateTracker)`.
fn simulate_execution_phase(is_skymeld: bool, tracker: &UiStateTracker) {
    tracker.loading_complete(&[], Some(blaze_cmdline::repository_mapping::EMPTY.clone()));
    if is_skymeld {
        // SkymeldUiStateTracker needs to be in the configuration phase before the execution
        // phase.
        tracker.set_build_status_for_testing_only(BuildStatus::ANALYSIS_COMPLETE);
        tracker.execution_phase_started();
    } else {
        let _unused = tracker.analysis_complete();
    }
    tracker.progress_receiver_available(Arc::new(DummyExecutionProgressReceiver));
}

fn longest_line(output: &str) -> usize {
    output
        .split('\n')
        .map(|l| l.chars().count())
        .max()
        .unwrap_or(0)
}

fn write(tracker: &UiStateTracker, discard_highlight: bool, short_version: bool) -> String {
    let mut terminal_writer = LoggingTerminalWriter::new(discard_highlight);
    tracker
        .write_progress_bar(&mut terminal_writer, short_version, None)
        .unwrap();
    terminal_writer.get_transcript().to_owned()
}

fn secs(s: i64) -> i64 {
    s * 1000
}

// ---------------------------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------------------------

#[test]
fn test_strategy_ids_get_id_ids_are_bitmasks() {
    let strategy_ids = StrategyIds::new();
    let id1 = strategy_ids.get_id("foo");
    let id2 = strategy_ids.get_id("bar");
    let id3 = strategy_ids.get_id("baz");

    assert!(id1 > 0);
    assert!(id2 > 0);
    assert!(id3 > 0);

    assert_eq!(id1 & id2, 0);
    assert_eq!(id1 & id3, 0);
    assert_eq!(id2 & id3, 0);
}

#[test]
fn test_strategy_ids_get_id_ids_are_reused_if_already_exist() {
    let strategy_ids = StrategyIds::new();
    let id1 = strategy_ids.get_id("foo");
    let id2 = strategy_ids.get_id("bar");
    let id3 = strategy_ids.get_id("foo");

    assert_ne!(id1, id2);
    assert_eq!(id1, id3);
}

#[test]
fn test_strategy_ids_get_id_exhaust_ids() {
    let strategy_ids = StrategyIds::new();
    let mut ids = HashSet::new();
    let mut name = String::new();
    loop {
        name.push('a');
        let id = strategy_ids.get_id(&name);
        if id == strategy_ids.fallback_id() {
            break;
        }
        ids.insert(id);
    }
    assert_eq!(ids.len(), 32 - 1); // Minus 1 for FALLBACK_NAME.

    assert_eq!(strategy_ids.get_id("some"), strategy_ids.fallback_id());
    assert_eq!(strategy_ids.get_id("more"), strategy_ids.fallback_id());
}

#[test]
fn test_strategy_ids_format_names_fallback_exists_by_default() {
    let strategy_ids = StrategyIds::new();
    assert_eq!(
        strategy_ids.format_names(strategy_ids.fallback_id()),
        StrategyIds::FALLBACK_NAME
    );
}

#[test]
fn test_strategy_ids_format_names_one_has_no_comma() {
    let strategy_ids = StrategyIds::new();
    let id1 = strategy_ids.get_id("abc");
    assert_eq!(strategy_ids.format_names(id1), "abc");
}

#[test]
fn test_strategy_ids_format_names() {
    let strategy_ids = StrategyIds::new();
    let id1 = strategy_ids.get_id("abc");
    let id2 = strategy_ids.get_id("xyz");
    let id3 = strategy_ids.get_id("def");

    // Names are not sorted alphabetically but their order is stable based on prior getId calls.
    assert_eq!(strategy_ids.format_names(id1 | id2), "abc, xyz");
    assert_eq!(strategy_ids.format_names(id1 | id3), "abc, def");
    assert_eq!(strategy_ids.format_names(id2 | id3), "xyz, def");
    assert_eq!(strategy_ids.format_names(id1 | id2 | id3), "abc, xyz, def");
}

#[test]
fn test_loading_activity() {
    // During loading phase, state and activity, as reported by the PackageProgressReceiver,
    // should be visible in the progress bar.
    let loading_state = "42 packages loaded";
    let loading_activity = "currently loading //src/foo/bar and 17 more";
    let clock = Arc::new(ManualClock::default());
    for (_skymeld, state_tracker) in trackers(&clock, 0) {
        state_tracker.loading_started(Some(Arc::new(FixedPackageProgress(
            loading_state.into(),
            loading_activity.into(),
        ))));

        // When it is just loading packages.
        let loading_output = write(&state_tracker, true, false);
        assert!(loading_output.contains("Loading"), "{loading_output}");
        assert!(loading_output.contains(loading_state), "{loading_output}");
        assert!(
            loading_output.contains(loading_activity),
            "{loading_output}"
        );

        // When it is configuring targets.
        state_tracker.loading_complete(&[], Some(blaze_cmdline::repository_mapping::EMPTY.clone()));
        let additional_message = "5 targets";
        state_tracker.set_additional_message_for_testing(additional_message);
        let analysis_progress_string = "5 targets and 0 aspects configured";
        state_tracker.configuration_started(Some(Arc::new(FixedAnalysisProgress(
            analysis_progress_string.into(),
        ))));

        let output = write(&state_tracker, true, false);
        assert!(output.contains("Analyzing"), "{output}");
        assert!(output.contains(additional_message), "{output}");
        assert!(output.contains(loading_state), "{output}");
        assert!(output.contains(loading_activity), "{output}");
        // It should contain the analysis progress string along with the loading information.
        assert!(output.contains(analysis_progress_string), "{output}");
    }
}

#[test]
fn test_action_visible() {
    // If there is only one action running, it should be visible
    // somewhere in the progress bar, and also the short version thereof.
    let message = "Building foo";
    let clock = Arc::new(ManualClock::default());
    clock.advance_millis(120000);
    for (skymeld, state_tracker) in trackers(&clock, 0) {
        // Mimic being at the execution phase.
        simulate_execution_phase(skymeld, &state_tracker);
        state_tracker.action_started(&mock_action(message, "bar/foo"), 123456789);

        let output = write(&state_tracker, true, false);
        assert!(output.contains(message), "{output}");

        let output = write(&state_tracker, false, true);
        assert!(output.contains(message), "{output}");
    }
}

#[test]
fn test_completed_action_not_shown() {
    // Completed actions should not be reported in the progress bar, nor in the
    // short progress bar.
    let message_fast = "Running quick action";
    let message_slow = "Running slow action";
    let clock = Arc::new(ManualClock::default());
    clock.advance_millis(120000);
    for (skymeld, state_tracker) in trackers(&clock, 0) {
        let fast_action = mock_action(message_fast, "foo/fast");
        let slow_action = mock_action(message_slow, "bar/slow");
        // Mimic being at the execution phase.
        simulate_execution_phase(skymeld, &state_tracker);
        state_tracker.action_started(&fast_action, 123456789);
        state_tracker.action_started(&slow_action, 123456999);

        state_tracker.action_completion(&fast_action, &action_lookup_data());

        let output = write(&state_tracker, true, false);
        assert!(!output.contains(message_fast), "{output}");
        assert!(output.contains(message_slow), "{output}");

        let output = write(&state_tracker, false, true);
        assert!(!output.contains(message_fast), "{output}");
        assert!(output.contains(message_slow), "{output}");
    }
}

#[test]
fn test_oldest_action_visible() {
    // The earliest-started action is always visible somehow in the progress bar
    // and its short version.
    let message_old = "Running the first-started action";
    let clock = Arc::new(ManualClock::default());
    clock.advance_millis(120000);
    for (skymeld, state_tracker) in trackers(&clock, 0) {
        // Mimic being at the execution phase.
        simulate_execution_phase(skymeld, &state_tracker);
        state_tracker.action_started(&mock_action(message_old, "bar/foo"), 123456789);
        for i in 0..30 {
            state_tracker.action_started(
                &mock_action(
                    &format!("Other action {i}"),
                    &format!("some/other/actions/number{i}"),
                ),
                123456790 + i,
            );
        }

        let output = write(&state_tracker, true, false);
        assert!(output.contains(message_old), "{output}");

        let output = write(&state_tracker, true, true);
        assert!(output.contains(message_old), "{output}");
    }
}

#[test]
fn test_sample_size() {
    // Verify that the number of actions shown in the progress bar can be set as sample size.
    for skymeld in [false, true] {
        let clock = Arc::new(ManualClock::default());
        clock.advance_millis(secs(123));
        let state_tracker = trackers(&clock, 0).remove(skymeld as usize).1;
        // Mimic being at the execution phase.
        simulate_execution_phase(skymeld, &state_tracker);
        clock.advance_millis(secs(2));

        // Start 10 actions (numbered 0 to 9).
        for i in 0..10 {
            clock.advance_millis(secs(1));
            let action = mock_action(
                &format!("Performing action A{i}."),
                &format!("action_A{i}.out"),
            );
            state_tracker.action_started(&action, clock.nano_time());
        }

        // For various sample sizes verify the progress bar
        for i in 1..11 {
            state_tracker.set_progress_sample_size(i);
            let output = write(&state_tracker, true, false);
            assert!(output.contains(&format!("A{}.", i - 1)), "{output}");
            assert!(!output.contains(&format!("A{i}.")), "{output}");
            if i < 10 {
                assert!(output.contains("..."), "{output}");
            } else {
                assert!(!output.contains("..."), "{output}");
            }
        }
    }
}

#[test]
fn test_times_shown() {
    // For sufficiently long running actions, the time that has passed since their start is
    // shown. In the short version of the progress bar, this should be true at least for the
    // oldest action.
    for skymeld in [false, true] {
        let clock = Arc::new(ManualClock::default());
        clock.advance_millis(secs(123));
        let state_tracker = trackers(&clock, 0).remove(skymeld as usize).1;
        // Mimic being at the execution phase.
        simulate_execution_phase(skymeld, &state_tracker);
        clock.advance_millis(secs(2));

        state_tracker.action_started(&mock_action("First action", "foo"), clock.nano_time());
        clock.advance_millis(secs(7));
        state_tracker.action_started(&mock_action("Second action", "bar"), clock.nano_time());
        clock.advance_millis(secs(20));

        let output = write(&state_tracker, true, false);
        assert!(output.contains("27s"), "{output}");
        assert!(output.contains("20s"), "{output}");

        let output = write(&state_tracker, true, true);
        assert!(output.contains("27s"), "{output}");
    }
}

#[test]
fn initial_progress_bar_time_independent() {
    let clock = Arc::new(ManualClock::default());
    clock.advance_millis(secs(123));
    for (_skymeld, state_tracker) in trackers(&clock, 0) {
        state_tracker.build_started();
        assert!(
            !blaze_events::ui::UiStateTrackerView::progress_bar_time_dependent(&state_tracker),
            "Initial progress status should be time independent"
        );
    }
}

#[test]
fn running_action_time_independent() {
    let clock = Arc::new(ManualClock::default());
    clock.advance_millis(secs(123));
    for (skymeld, state_tracker) in trackers(&clock, 0) {
        clock.advance_millis(secs(1));
        simulate_execution_phase(skymeld, &state_tracker);
        state_tracker.action_started(&mock_action("Some action", "foo"), clock.nano_time());
        assert!(
            blaze_events::ui::UiStateTrackerView::progress_bar_time_dependent(&state_tracker),
            "Progress bar showing a running action should be time dependent"
        );
    }
}

fn label(s: &str) -> Label {
    Label::parse_canonical(s).unwrap()
}

#[test]
fn test_count_visible() {
    // The test count should be visible in the status bar, as well as the short status bar
    let clock = Arc::new(ManualClock::default());
    for (skymeld, state_tracker) in trackers(&clock, 0) {
        // Mimic being at the execution phase.
        simulate_execution_phase(skymeld, &state_tracker);
        let label_a = label("//foo/bar:baz");
        // targetB has no label (a mock).
        state_tracker.test_filtering_complete(Some(&[Some(label_a.clone()), None]));
        // The mocked TestSummary has no status (null): NO_STATUS counts the same.
        state_tracker.test_summary(&label_a, BlazeTestStatus::NoStatus);

        let output = write(&state_tracker, true, false);
        assert!(output.contains(" 1 / 2 tests"), "{output}");

        let output = write(&state_tracker, true, true);
        assert!(output.contains(" 1 / 2 tests"), "{output}");
    }
}

#[test]
fn test_passed_visible() {
    // The last test should still be visible in the long status bar, and colored as ok if it
    // passed.
    let clock = Arc::new(ManualClock::default());
    for (skymeld, state_tracker) in trackers(&clock, 0) {
        // Mimic being at the execution phase.
        simulate_execution_phase(skymeld, &state_tracker);
        let label_a = label("//foo/bar:baz");
        state_tracker.test_filtering_complete(Some(&[Some(label_a.clone()), None]));
        state_tracker.test_summary(&label_a, BlazeTestStatus::Passed);

        let output = write(&state_tracker, false, false);
        let expected = format!("{}{}", LoggingTerminalWriter::OK, label_a);
        assert!(output.contains(&expected), "{output}");
    }
}

#[test]
fn test_failed_visible() {
    // The last test should still be visible in the long status bar, and colored as fail if it
    // did not pass.
    let clock = Arc::new(ManualClock::default());
    for (skymeld, state_tracker) in trackers(&clock, 0) {
        // Mimic being at the execution phase.
        simulate_execution_phase(skymeld, &state_tracker);
        let label_a = label("//foo/bar:baz");
        state_tracker.test_filtering_complete(Some(&[Some(label_a.clone()), None]));
        state_tracker.test_summary(&label_a, BlazeTestStatus::Failed);

        let output = write(&state_tracker, false, false);
        let expected = format!("{}{}", LoggingTerminalWriter::FAIL, label_a);
        assert!(output.contains(&expected), "{output}");
    }
}

#[test]
fn test_sensible_shortening() {
    // Verify that in the typical case, we shorten the progress message by shortening
    // the path implicit in it, that can also be extracted from the label. In particular,
    // the parts
    for skymeld in [false, true] {
        let clock = Arc::new(ManualClock::default());
        let state_tracker = trackers(&clock, 70).remove(skymeld as usize).1;
        // Mimic being at the execution phase.
        simulate_execution_phase(skymeld, &state_tracker);
        let action = mock_action_with_owner(
            "Building some/very/very/long/path/for/some/library/directory/foo.jar (42 source files)",
            "some/very/very/long/path/for/some/library/directory/foo.jar",
            owner(
                Some("//some/very/very/long/path/for/some/library/directory:libfoo"),
                "dummy-mnemonic",
                "fedcba",
            ),
        );

        clock.advance_millis(secs(3));
        state_tracker.action_started(&action, clock.nano_time());
        clock.advance_millis(secs(5));

        let output = write(&state_tracker, true, false);
        assert!(output.contains("Building "), "{output}");
        assert!(output.contains("foo.jar (42 source files)"), "{output}");
    }
}

#[test]
fn test_action_strategy_visible() {
    // verify that, if a strategy was reported for a shown action, it is visible
    // in the progress bar.
    let strategy = "verySpecialStrategy";
    let clock = Arc::new(ManualClock::default());
    for (skymeld, state_tracker) in trackers(&clock, 0) {
        let action = mock_action_with_owner(
            "Some random action",
            "some/path/to/a/file",
            dummy_action_owner(),
        );
        // Mimic being at the execution phase.
        simulate_execution_phase(skymeld, &state_tracker);
        state_tracker.action_started(&action, clock.nano_time());
        state_tracker.running_action(&action, strategy);

        let output = write(&state_tracker, true, false);
        assert!(output.contains(strategy), "{output}");
    }
}

/// `createDummyAction(progressMessage)`.
fn create_dummy_action(progress_message: &str) -> ActionRef {
    mock_action_with_owner(
        progress_message,
        "some/path/to/a/file",
        dummy_action_owner(),
    )
}

#[test]
fn action_progress_visible() {
    let clock = Arc::new(ManualClock::default());
    for (skymeld, state_tracker) in trackers(&clock, 70) {
        let action = create_dummy_action("Some random action");
        // Mimic being at the execution phase.
        simulate_execution_phase(skymeld, &state_tracker);
        state_tracker.action_started(&action, clock.nano_time());
        state_tracker.action_progress(&action, "action-id", "action progress", false);

        let output = write(&state_tracker, true, false);
        assert!(output.contains("action progress"), "{output}");
    }
}

#[test]
fn set_target_width_affects_subsequent_progress_bar_rendering() {
    let clock = Arc::new(ManualClock::default());
    for (skymeld, state_tracker) in trackers(&clock, 70) {
        let action = create_dummy_action("Some random action");
        // Mimic being at the execution phase.
        simulate_execution_phase(skymeld, &state_tracker);
        state_tracker.action_started(&action, clock.nano_time());
        state_tracker.action_progress(&action, "action-id", "action progress", false);

        let output = write(&state_tracker, true, false);
        assert!(output.contains("action progress"), "{output}");

        blaze_events::ui::UiStateTrackerView::set_target_width(&state_tracker, 30);

        let output = write(&state_tracker, true, false);
        assert!(!output.contains("action progress"), "{output}");
    }
}

#[test]
fn action_progress_with_too_small_width_progress_skipped() {
    let clock = Arc::new(ManualClock::default());
    for (skymeld, state_tracker) in trackers(&clock, 30) {
        let action = create_dummy_action("Some random action");
        // Mimic being at the execution phase.
        simulate_execution_phase(skymeld, &state_tracker);
        state_tracker.action_started(&action, clock.nano_time());
        state_tracker.action_progress(&action, "action-id", "action progress", false);

        let output = write(&state_tracker, true, false);
        assert!(!output.contains("action progress"), "{output}");
    }
}

#[test]
fn action_progress_with_small_width_progress_shortened() {
    let clock = Arc::new(ManualClock::default());
    // The targetWidth needs to be small enough to cause shortening to occur.
    for (skymeld, state_tracker) in trackers(&clock, 40) {
        let action = create_dummy_action("some action");
        // Mimic being at the execution phase.
        simulate_execution_phase(skymeld, &state_tracker);
        state_tracker.action_started(&action, clock.nano_time());
        state_tracker.action_progress(&action, "action-id", "action progress", false);

        let output = write(&state_tracker, true, false);
        assert!(output.contains("action p..."), "{output}");
    }
}

#[test]
fn action_progress_multiple_progress_display_in_order() {
    let clock = Arc::new(ManualClock::default());
    for (skymeld, state_tracker) in trackers(&clock, 70) {
        let action = create_dummy_action("Some random action");
        // Mimic being at the execution phase.
        simulate_execution_phase(skymeld, &state_tracker);
        state_tracker.action_started(&action, clock.nano_time());
        state_tracker.action_progress(&action, "action-id1", "action progress 1", false);
        state_tracker.action_progress(&action, "action-id2", "action progress 2", false);

        let output = write(&state_tracker, true, false);
        assert!(output.contains("action progress 1"), "{output}");
        assert!(!output.contains("action progress 2"), "{output}");
    }
}

#[test]
fn test_multiple_action_strategies_visible_for_dynamic_scheduling() {
    let strategy1 = "strategy1";
    let strategy2 = "stratagy2";
    let clock = Arc::new(ManualClock::default());
    for (skymeld, state_tracker) in trackers(&clock, 0) {
        let action = mock_action_with_owner(
            "Some random action",
            "some/path/to/a/file",
            dummy_action_owner(),
        );
        // Mimic being at the execution phase.
        simulate_execution_phase(skymeld, &state_tracker);
        state_tracker.action_started(&action, clock.nano_time());
        state_tracker.running_action(&action, strategy1);
        state_tracker.running_action(&action, strategy2);

        let output = write(&state_tracker, true, false);
        assert!(
            output.contains(&format!("{strategy1}, {strategy2}")),
            "{output}"
        );
    }
}

#[test]
fn test_action_counts_with_dynamic_scheduling() {
    let clock = Arc::new(ManualClock::default());
    for (skymeld, state_tracker) in trackers(&clock, 0) {
        // Mimic being at the execution phase.
        simulate_execution_phase(skymeld, &state_tracker);

        let action1 = mock_action_with_owner(
            "First random action",
            "some/path/to/a/file",
            dummy_action_owner(),
        );
        state_tracker.action_started(&action1, clock.nano_time());
        let action2 = mock_action_with_owner(
            "First random action",
            "some/path/to/b/file",
            dummy_action_owner(),
        );
        state_tracker.action_started(&action2, clock.nano_time());

        state_tracker.running_action(&action1, "strategy1");
        state_tracker.scheduling_action(&action2, "strategy1");
        assert!(write(&state_tracker, true, false).contains("2 actions, 1 running"));

        state_tracker.running_action(&action1, "strategy2");
        assert!(write(&state_tracker, true, false).contains("2 actions, 1 running"));

        state_tracker.running_action(&action2, "strategy1");
        assert!(write(&state_tracker, true, false).contains("2 actions running"));

        state_tracker.running_action(&action2, "strategy2");
        assert!(write(&state_tracker, true, false).contains("2 actions running"));
    }
}

fn do_test_output_length(skymeld: bool, with_test: bool, actions: i32) {
    // If we target 70 characters, then there should be enough space to both,
    // keep the line limit, and show the local part of the running actions and
    // the passed test.
    let clock = Arc::new(ManualClock::default());
    let state_tracker = trackers(&clock, 70).remove(skymeld as usize).1;
    // Mimic being at the execution phase.
    simulate_execution_phase(skymeld, &state_tracker);

    let foobuild_action = mock_action(
        "Building //src/some/very/long/path/long/long/long/long/long/long/long/foo/foobuild.jar",
        "src/some/very/long/path/long/long/long/long/long/long/long/foo/foobuild.jar",
    );
    let bazbuild_action = mock_action(
        "Building //src/some/very/long/path/long/long/long/long/long/long/long/baz/bazbuild.jar",
        "src/some/very/long/path/long/long/long/long/long/long/long/baz/bazbuild.jar",
    );

    let bartest_label = label(
        "//src/another/very/long/long/path/long/long/long/long/long/long/long/long/bars:bartest",
    );

    if actions >= 1 {
        state_tracker.action_started(&foobuild_action, 123456789);
    }
    if actions >= 2 {
        state_tracker.action_started(&bazbuild_action, 123456900);
    }
    if with_test {
        state_tracker.test_filtering_complete(Some(&[Some(bartest_label.clone())]));
        state_tracker.test_summary(&bartest_label, BlazeTestStatus::Passed);
    }

    let output = write(&state_tracker, true, false);

    assert!(longest_line(&output) <= 70, "{output}");
    if actions >= 1 {
        assert!(output.contains("foobuild"), "{output}");
    }
    if actions >= 2 {
        assert!(output.contains("bazbuild"), "{output}");
    }
    if with_test {
        assert!(output.contains(":bartest"), "{output}");
    }
}

#[test]
fn test_output_length() {
    for skymeld in [false, true] {
        for i in 0..3 {
            do_test_output_length(skymeld, true, i);
            do_test_output_length(skymeld, false, i);
        }
    }
}

#[test]
fn test_status_shown() {
    // Verify that for non-executing actions, at least the first 3 characters of the
    // status are shown.
    // Also verify that the number of running actions is reported correctly, if there is
    // more than one active action and not all are running.
    let clock = Arc::new(ManualClock::default());
    clock.advance_millis(120000);
    for (skymeld, state_tracker) in trackers(&clock, 0) {
        // Mimic being at the execution phase.
        simulate_execution_phase(skymeld, &state_tracker);
        let action_foo = mock_action_with_owner("Building foo", "foo/foo", dummy_action_owner());
        let action_bar = mock_action_with_owner("Building bar", "bar/bar", dummy_action_owner());

        // Action foo being scanned.
        state_tracker.action_started(&action_foo, 123456700);
        state_tracker.scanning_action(&action_foo);

        let output = write(&state_tracker, true, false);
        assert!(output.contains("sca") || output.contains("Sca"), "{output}");

        // Then action bar gets scheduled.
        state_tracker.action_started(&action_bar, 123456701);
        state_tracker.scheduling_action(&action_bar, "bar-sandbox");

        let output = write(&state_tracker, true, false);
        assert!(output.contains("sch") || output.contains("Sch"), "{output}");
        assert!(output.contains("sca") || output.contains("Sca"), "{output}");
        assert!(output.contains("0 running"), "{output}");
        assert!(output.contains("2 actions"), "{output}");

        // Then foo starts.
        state_tracker.running_action(&action_foo, "xyz-sandbox");

        let output = write(&state_tracker, true, false);
        assert!(output.contains("xyz-sandbox"), "{output}");
        assert!(
            !(output.contains("ana") || output.contains("Ana")),
            "{output}"
        );
        assert!(output.contains("sch") || output.contains("Sch"), "{output}");
        assert!(output.contains("1 running"), "{output}");
        assert!(output.contains("2 actions"), "{output}");
    }
}

#[test]
fn test_timer_reset() {
    // Verify that a change in an action state (e.g., from scheduling to executing) resets
    // the time associated with that action.
    for skymeld in [false, true] {
        let clock = Arc::new(ManualClock::default());
        clock.advance_millis(secs(123));
        let state_tracker = trackers(&clock, 0).remove(skymeld as usize).1;
        // Mimic being at the execution phase.
        simulate_execution_phase(skymeld, &state_tracker);
        clock.advance_millis(secs(2));

        let action_foo = mock_action_with_owner("Building foo", "foo/foo", dummy_action_owner());
        let action_bar = mock_action_with_owner("Building bar", "bar/bar", dummy_action_owner());

        state_tracker.action_started(&action_foo, clock.nano_time());
        state_tracker.running_action(&action_foo, "foo-sandbox");
        clock.advance_millis(secs(7));
        state_tracker.action_started(&action_bar, clock.nano_time());
        state_tracker.scheduling_action(&action_bar, "bar-sandbox");
        clock.advance_millis(secs(21));

        let output = write(&state_tracker, true, false);
        assert!(output.contains("28s"), "{output}");
        assert!(output.contains("21s"), "{output}");

        state_tracker.running_action(&action_bar, "bar-sandbox");
        let output = write(&state_tracker, true, false);
        assert!(output.contains("28s"), "{output}");
        assert!(!output.contains("21s"), "{output}");

        clock.advance_millis(secs(30));
        let output = write(&state_tracker, true, false);
        assert!(output.contains("58s"), "{output}");
        assert!(output.contains("30s"), "{output}");
    }
}

#[test]
fn test_early_status_handled_gracefully() {
    // On the event bus, events sometimes are sent out of order; verify that we handle an
    // early message that an action is running gracefully.
    let clock = Arc::new(ManualClock::default());
    for (skymeld, state_tracker) in trackers(&clock, 0) {
        // Mimic being at the execution phase.
        simulate_execution_phase(skymeld, &state_tracker);
        let action_foo = mock_action_with_owner("Building foo", "foo/foo", dummy_action_owner());

        // Early status announcement
        state_tracker.running_action(&action_foo, "foo-sandbox");

        // Here we don't expect any particular output, just some description; in particular, we
        // do not expect the state tracker to hit an internal error.
        let output = write(&state_tracker, true, false);
        assert!(!output.is_empty(), "Expected at least some status bar");

        // Action actually started
        state_tracker.action_started(&action_foo, clock.nano_time());

        let output = write(&state_tracker, true, false);
        assert!(output.contains("foo-sandbox"), "{output}");
    }
}

#[test]
fn test_executing_actions_first() {
    // Verify that executing actions, even if started late, are visible.
    for skymeld in [false, true] {
        let clock = Arc::new(ManualClock::default());
        let state_tracker = trackers(&clock, 0).remove(skymeld as usize).1;
        // Mimic being at the execution phase.
        simulate_execution_phase(skymeld, &state_tracker);
        clock.advance_millis(120000);

        for i in 0..30 {
            let action = mock_action_with_owner(
                &format!("Takes long to schedule number {i}"),
                &format!("long/startup{i}"),
                dummy_action_owner(),
            );
            state_tracker.action_started(&action, 123456789 + i);
            state_tracker.scheduling_action(&action, "xyz-sandbox");
        }

        for i in 0..3 {
            let action = mock_action_with_owner(
                &format!("quickstart{i}"),
                &format!("pkg/quickstart{i}"),
                dummy_action_owner(),
            );
            state_tracker.action_started(&action, 123457000 + i);
            state_tracker.running_action(&action, "xyz-sandbox");

            let output = write(&state_tracker, true, false);
            assert!(output.contains(&format!("quickstart{i}")), "{output}");
            assert!(output.contains(&format!("{} running", i + 1)), "{output}");
        }
    }
}

#[test]
fn test_aggregation() {
    // Assert that actions for the same test are aggregated so that an action afterwards
    // is still shown.
    for skymeld in [false, true] {
        let clock = Arc::new(ManualClock::default());
        clock.advance_millis(secs(1234));
        let state_tracker = trackers(&clock, 80).remove(skymeld as usize).1;
        state_tracker.set_progress_sample_size(4);
        // Mimic being at the execution phase.
        simulate_execution_phase(skymeld, &state_tracker);

        let label_foo_test = label("//foo/bar:footest");
        let label_bar_test = label("//baz:bartest");
        let label_baz_test = label("//baz:baztest");

        state_tracker.test_filtering_complete(Some(&[
            Some(label_foo_test.clone()),
            Some(label_bar_test.clone()),
            Some(label_baz_test.clone()),
        ]));

        // First produce 10 actions for footest...
        for i in 0..10 {
            clock.advance_millis(secs(1));
            let action = mock_action_with_owner(
                &format!("Testing foo, shard {i}"),
                &format!("testlog_foo_{i}"),
                owner(Some("//foo/bar:footest"), "TestRunner", "abcdef"),
            );
            state_tracker.action_started(&action, clock.nano_time());
        }
        // ...then produce 10 actions for bartest...
        for i in 0..10 {
            clock.advance_millis(secs(1));
            let action = mock_action_with_owner(
                &format!("Testing bar, shard {i}"),
                &format!("testlog_bar_{i}"),
                owner(Some("//baz:bartest"), "TestRunner", "abcdef"),
            );
            state_tracker.action_started(&action, clock.nano_time());
        }
        // ...run a completely unrelated action..
        clock.advance_millis(secs(1));
        state_tracker.action_started(
            &mock_action("Other action", "other/action"),
            clock.nano_time(),
        );
        // ...and finally, run actions that are associated with baztest but are not a test.
        for i in 0..10 {
            clock.advance_millis(1_000);
            let action = mock_action_with_owner(
                &format!("Doing something {i}"),
                &format!("someartifact_{i}"),
                owner(Some("//baz:baztest"), "NonTestAction", "fedcba"),
            );
            state_tracker.action_started(&action, clock.nano_time());
        }
        clock.advance_millis(secs(1));

        let output = write(&state_tracker, true, false);
        assert!(output.contains(":footest"), "{output}");
        assert!(output.contains(":bartest"), "{output}");
        assert!(output.contains("Other action"), "{output}");
        assert!(!output.contains("Testing //baz:baztest"), "{output}");
        assert!(output.contains("Doing something"), "{output}");
    }
}

#[test]
fn test_suffix() {
    assert_eq!(UiStateTracker::suffix("foobar", 3), "bar");
    assert_eq!(UiStateTracker::suffix("foo", -2), "");
    assert_eq!(UiStateTracker::suffix("foobar", 200), "foobar");
}

#[test]
fn test_download_shown_during_loading() {
    // Verify that, whenever a single download is running in loading phase, it is shown in the
    // status bar.
    for skymeld in [false, true] {
        let clock = Arc::new(ManualClock::default());
        clock.advance_millis(secs(1234));
        let state_tracker = trackers(&clock, 80).remove(skymeld as usize).1;
        let url = "http://example.org/first/dep";

        state_tracker.build_started();
        state_tracker.download_progress(url, "", false);
        clock.advance_millis(secs(6));

        let output = write(&state_tracker, true, false);
        assert!(output.contains(url), "{output}");
        assert!(output.contains("6s"), "{output}");

        // Progress on the pending download should be reported appropriately
        clock.advance_millis(secs(1));
        // new DownloadProgressEvent(url, 256): getProgress() of 256 bytes read, total unknown.
        state_tracker.download_progress(url, "256 B (256B)", false);

        let output = write(&state_tracker, true, false);
        assert!(output.contains(url), "{output}");
        assert!(output.contains("7s"), "{output}");
        assert!(output.contains("256"), "{output}");

        // After finishing the download, it should no longer be reported.
        clock.advance_millis(secs(1));
        state_tracker.download_progress(url, "", true);

        let output = write(&state_tracker, true, false);
        assert!(!output.contains("example.org"), "{output}");
    }
}

#[test]
fn waiting_remote_cache_message_before_build_complete_invisible() {
    let clock = Arc::new(ManualClock::default());
    for (skymeld, state_tracker) in trackers(&clock, 0) {
        // Mimic being at the execution phase.
        simulate_execution_phase(skymeld, &state_tracker);
        state_tracker.action_upload_started();

        let output = write(&state_tracker, true, false);
        assert!(!output.contains("1 upload"), "{output}");
    }
}

#[test]
fn waiting_remote_cache_message_after_build_complete_visible() {
    for skymeld in [false, true] {
        let clock = Arc::new(ManualClock::default());
        let state_tracker = trackers(&clock, 0).remove(skymeld as usize).1;
        // Mimic being at the execution phase.
        simulate_execution_phase(skymeld, &state_tracker);
        state_tracker.action_upload_started();
        let _unused = state_tracker.build_complete_event(true);
        clock.advance_millis(secs(2));

        let output = write(&state_tracker, true, false);
        assert!(output.contains("1 upload"), "{output}");
    }
}

#[test]
fn waiting_remote_cache_message_multiple_upload_events_count_correctly() {
    for skymeld in [false, true] {
        let clock = Arc::new(ManualClock::default());
        let state_tracker = trackers(&clock, 0).remove(skymeld as usize).1;
        state_tracker.action_upload_started();
        let _unused = state_tracker.build_complete_event(true);
        state_tracker.action_upload_started();
        state_tracker.action_upload_started();
        state_tracker.action_upload_finished();
        clock.advance_millis(secs(2));

        let output = write(&state_tracker, true, false);
        assert!(output.contains("2 uploads"), "{output}");
    }
}

#[test]
fn test_multiple_build_event_protocol_transports() {
    // Verify that all announced transports are present in the progress bar
    // and that as transports are closed they disappear from the progress bar.
    // Verify that the wait duration is displayed.
    // Verify that after all transports have been closed, the build status is displayed.
    for skymeld in [false, true] {
        let clock = Arc::new(ManualClock::default());
        let transport1 = TransportId {
            id: 1,
            name: "BuildEventTransport1".into(),
        };
        let transport2 = TransportId {
            id: 2,
            name: "BuildEventTransport2".into(),
        };
        let transport3 = TransportId {
            id: 3,
            name: "BuildEventTransport3".into(),
        };
        clock.advance_millis(secs(1));

        let state_tracker = trackers(&clock, 80).remove(skymeld as usize).1;
        state_tracker.build_started();
        state_tracker.build_event_transports_announced(&[transport1.clone(), transport2.clone()]);
        state_tracker.build_event_transports_announced(&[transport3.clone()]);
        let _unused = state_tracker.build_complete_event(true);

        clock.advance_millis(secs(1));
        let output = write(&state_tracker, true, false);
        assert!(output.contains("1s"), "{output}");
        assert!(output.contains("BuildEventTransport1"), "{output}");
        assert!(output.contains("BuildEventTransport2"), "{output}");
        assert!(output.contains("BuildEventTransport3"), "{output}");

        clock.advance_millis(secs(1));
        state_tracker.build_event_transport_closed(&transport1);
        let output = write(&state_tracker, true, false);
        assert!(output.contains("2s"), "{output}");
        assert!(!output.contains("BuildEventTransport1"), "{output}");
        assert!(output.contains("BuildEventTransport2"), "{output}");
        assert!(output.contains("BuildEventTransport3"), "{output}");

        clock.advance_millis(secs(1));
        state_tracker.build_event_transport_closed(&transport3);
        let output = write(&state_tracker, true, false);
        assert!(output.contains("3s"), "{output}");
        assert!(!output.contains("BuildEventTransport1"), "{output}");
        assert!(output.contains("BuildEventTransport2"), "{output}");
        assert!(!output.contains("BuildEventTransport3"), "{output}");

        clock.advance_millis(secs(1));
        state_tracker.build_event_transport_closed(&transport2);
        let output = write(&state_tracker, true, false);
        assert!(!output.contains("3s"), "{output}");
        assert!(!output.contains("BuildEventTransport1"), "{output}");
        assert!(!output.contains("BuildEventTransport2"), "{output}");
        assert!(!output.contains("BuildEventTransport3"), "{output}");
        assert_eq!(java_split_lines(&output), 1, "{output}");
    }
}

#[test]
fn test_test_analyzed_event() {
    // The test count should be visible in the status bar, as well as the short status bar
    let clock = Arc::new(ManualClock::default());
    for (skymeld, state_tracker) in trackers(&clock, 0) {
        // Mimic being at the execution phase.
        simulate_execution_phase(skymeld, &state_tracker);
        let label_a = label("//foo:A");
        let label_b = label("//foo:B");

        state_tracker.single_test_analyzed(Some(&label_a));
        state_tracker.single_test_analyzed(Some(&label_b));
        state_tracker.test_summary(&label_a, BlazeTestStatus::NoStatus);

        let output = write(&state_tracker, true, false);
        assert!(output.contains(" 1 / 2 tests"), "{output}");

        let output = write(&state_tracker, true, true);
        assert!(output.contains(" 1 / 2 tests"), "{output}");
    }
}

#[test]
fn test_test_analyzed_event_repeated_no_duplicated_count() {
    let clock = Arc::new(ManualClock::default());
    for (skymeld, state_tracker) in trackers(&clock, 0) {
        // Mimic being at the execution phase.
        simulate_execution_phase(skymeld, &state_tracker);
        let label_a = label("//foo:A");

        state_tracker.single_test_analyzed(Some(&label_a));
        // Duplicate.
        state_tracker.single_test_analyzed(Some(&label_a));
        state_tracker.test_summary(&label_a, BlazeTestStatus::NoStatus);

        let output = write(&state_tracker, true, false);
        assert!(output.contains(" 1 / 1 tests"), "{output}");

        let output = write(&state_tracker, true, true);
        assert!(output.contains(" 1 / 1 tests"), "{output}");
    }
}

#[test]
fn test_get_oldest_action_prioritizes_running_over_scheduled() {
    for skymeld in [false, true] {
        let clock = Arc::new(ManualClock::default());
        let state_tracker = trackers(&clock, 0).remove(skymeld as usize).1;
        simulate_execution_phase(skymeld, &state_tracker);

        let scheduled_action =
            mock_action_with_owner("Scheduled action", "scheduled/action", dummy_action_owner());
        state_tracker.action_started(&scheduled_action, clock.nano_time());
        state_tracker.scheduling_action(&scheduled_action, "some-strategy");

        clock.advance_millis(1000);
        let running_action =
            mock_action_with_owner("Running action", "running/action", dummy_action_owner());
        state_tracker.action_started(&running_action, clock.nano_time());
        state_tracker.running_action(&running_action, "some-strategy");

        let output = write(&state_tracker, true, true);
        assert!(output.contains("Running action"), "{output}");
        assert!(output.contains("(2 actions, 1 running)"), "{output}");
        assert!(!output.contains("Scheduled action"), "{output}");
    }
}

#[test]
fn test_download_shown_during_main_repo_mapping_computation() {
    for skymeld in [false, true] {
        let clock = Arc::new(ManualClock::default());
        clock.advance_millis(secs(1234));
        let state_tracker = trackers(&clock, 80).remove(skymeld as usize).1;
        let url = "http://example.org/first/dep";

        state_tracker.main_repo_mapping_computation_started();
        state_tracker.download_progress(url, "", false);
        clock.advance_millis(secs(6));

        let output = write(&state_tracker, true, false);
        assert!(output.contains(url), "{output}");
        assert!(output.contains("6s"), "{output}");

        // Progress on the pending download should be reported appropriately
        clock.advance_millis(secs(1));
        // new DownloadProgressEvent(url, 256): getProgress() of 256 bytes read, total unknown.
        state_tracker.download_progress(url, "256 B (256B)", false);

        let output = write(&state_tracker, true, false);
        assert!(output.contains(url), "{output}");
        assert!(output.contains("7s"), "{output}");
        assert!(output.contains("256"), "{output}");

        // After finishing the download, it should no longer be reported.
        clock.advance_millis(secs(1));
        state_tracker.download_progress(url, "256 B (256B)", true);

        let output = write(&state_tracker, true, false);
        assert!(!output.contains("example.org"), "{output}");
    }
}

#[test]
fn test_download_output_length() {
    // Verify that URLs are shortened in a reasonable way, if the terminal is not wide enough
    // Also verify that the length is respected, even if only a download sample is shown.
    for skymeld in [false, true] {
        let clock = Arc::new(ManualClock::default());
        clock.advance_millis(secs(1234));
        let state_tracker = trackers(&clock, 60).remove(skymeld as usize).1;
        let url = "http://example.org/some/really/very/very/long/path/filename.tar.gz";

        state_tracker.build_started();
        state_tracker.download_progress(url, "", false);
        clock.advance_millis(secs(6));
        for i in 0..10 {
            state_tracker.download_progress(
                &format!(
                    "http://otherhost.example/another/also/length/path/to/another/download{i}.zip"
                ),
                "",
                false,
            );
            clock.advance_millis(secs(1));
        }

        let output = write(&state_tracker, true, false);
        assert!(longest_line(&output) <= 60, "{output}");
        assert!(output.contains("filename.tar.gz"), "{output}");
        assert!(output.contains("example.org"), "{output}");
    }
}

#[test]
fn test_build_event_transports_on_narrow_terminal() {
    // Verify that the progress bar contains useful information on a 60-character terminal.
    //   - Too long names should be shortened to reasonably long prefixes of the name.
    for skymeld in [false, true] {
        let clock = Arc::new(ManualClock::default());
        let transport1 = TransportId {
            id: 1,
            name: "A".repeat(61),
        };
        let transport2 = TransportId {
            id: 2,
            name: "BuildEventTransport".into(),
        };
        let state_tracker = trackers(&clock, 60).remove(skymeld as usize).1;
        state_tracker.build_started();
        state_tracker.build_event_transports_announced(&[transport1.clone(), transport2.clone()]);
        let _unused = state_tracker.build_complete_event(true);
        clock.advance_millis(secs(1));
        let output = write(&state_tracker, true, false);
        assert!(longest_line(&output) <= 60, "{output}");
        assert!(output.contains("1s"), "{output}");
        assert!(
            output.contains(&format!("{}...", "A".repeat(30))),
            "{output}"
        );
        assert!(output.contains("BuildEventTransport"), "{output}");

        clock.advance_millis(secs(1));
        state_tracker.build_event_transport_closed(&transport2);
        let output = write(&state_tracker, true, false);
        assert!(longest_line(&output) <= 60, "{output}");
        assert!(output.contains("2s"), "{output}");
        assert!(
            output.contains(&format!("{}...", "A".repeat(30))),
            "{output}"
        );
        assert!(!output.contains("BuildEventTransport"), "{output}");
        assert_eq!(java_split_lines(&output), 2, "{output}");
    }
}

#[test]
fn test_total_fetches_reported() {
    for skymeld in [false, true] {
        let clock = Arc::new(ManualClock::default());
        let state_tracker = trackers(&clock, 80).remove(skymeld as usize).1;

        state_tracker.build_started();
        for i in 0..30 {
            // FetchEvent: getProgress() is "working...".
            state_tracker.download_progress(&format!("@repoFoo{i}"), "working...", false);
        }
        clock.advance_millis(secs(7));

        let output = write(&state_tracker, true, false);
        assert!(output.contains("@repoFoo"), "{output}");
        assert!(output.contains("7s"), "{output}");
        assert!(output.contains("30 fetches"), "{output}");
    }
}

/// `output.split("\\n").length`: Java's `String.split` drops trailing empty strings.
fn java_split_lines(output: &str) -> usize {
    let mut parts: Vec<&str> = output.split('\n').collect();
    while parts.len() > 1 && parts.last() == Some(&"") {
        parts.pop();
    }
    parts.len()
}
