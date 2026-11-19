//! Port of the `runtime/UiEventHandlerTest.java` cases that need the state tracker
//! (`UiStateTracker` lives in this crate; the cases that do not are in
//! `blaze_events::ui::tests`).
//!
//! The Java test is parameterised over `skymeldMode`; each Rust test runs both. `uiOptions` is a
//! `new UiOptions()` object, whose fields hold Java's defaults for their types (false, 0), not
//! the options' declared defaults.

use std::any::Any;
use std::io;
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};

use blaze_actions::actions::action::{AbstractAction, Action, ActionRef, ComputeKeyError};
use blaze_actions::actions::platform_info::empty_platform_info;
use blaze_actions::{ActionKeyContext, ActionOwner, Artifact, ArtifactOwner, ArtifactRoot};
use blaze_buildtool::runtime::ui_state_tracker::UiStateTracker;
use blaze_cmdline::RepositoryMapping;
use blaze_collect::{NestedSetBuilder, Order};
use blaze_events::location::Location;
use blaze_events::ui::{UiEventHandler, UiEventHandlerOptions};
use blaze_events::{Clock, Event, EventHandler, EventKind, OutErr, OutputStream};
use blaze_util::Fingerprint;
use blaze_vfs::{DigestHashFunction, Path, PathFragment, Root, UnixFileSystem};
use indexmap::IndexMap;
use parking_lot::Mutex;

// Java: src/test/java/com/google/devtools/build/lib/runtime/UiEventHandlerTest.java UiEventHandlerTest.BUILD_DID_NOT_COMPLETE_MESSAGE
const BUILD_DID_NOT_COMPLETE_MESSAGE: &str =
    "\x1b[31m\x1b[1mERROR: \x1b[0mBuild did NOT complete successfully\n";

/// The escape sequence that clears the progress bar when curses is enabled.
// Java: src/test/java/com/google/devtools/build/lib/runtime/UiEventHandlerTest.java UiEventHandlerTest.CLEAR_PROGRESS_BAR
const CLEAR_PROGRESS_BAR: &str = "\x1b[1A\x1b[K";

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

// Java: src/test/java/com/google/devtools/build/lib/runtime/UiEventHandlerTest.java UiEventHandlerTest.FlushCollectingOutputStream
#[derive(Default)]
struct FlushCollectingOutputStream {
    state: Mutex<(Vec<String>, String)>,
}

impl OutputStream for FlushCollectingOutputStream {
    fn write(&self, bytes: &[u8]) -> io::Result<()> {
        self.state
            .lock()
            .1
            .push_str(std::str::from_utf8(bytes).unwrap());
        Ok(())
    }

    fn flush(&self) -> io::Result<()> {
        let mut st = self.state.lock();
        // Ignore inconsequential extra flushes.
        if !st.1.is_empty() {
            let written = std::mem::take(&mut st.1);
            st.0.push(written);
        }
        Ok(())
    }
}

impl FlushCollectingOutputStream {
    fn assert_flushed(&self, messages: &[&str]) {
        let st = self.state.lock();
        assert!(st.1.is_empty(), "written since flush: {:?}", st.1);
        assert_eq!(st.0, messages);
    }

    fn flushed(&self) -> Vec<String> {
        self.state.lock().0.clone()
    }

    fn written_since_flush(&self) -> String {
        self.state.lock().1.clone()
    }

    fn clear(&self) {
        self.state.lock().0.clear();
    }
}

/// Stands in for `mock(OutputStream.class)`.
struct DiscardingOutputStream;

impl OutputStream for DiscardingOutputStream {
    fn write(&self, _bytes: &[u8]) -> io::Result<()> {
        Ok(())
    }
}

/// `new UiOptions()`: the fields' Java defaults.
fn new_ui_options() -> UiEventHandlerOptions {
    UiEventHandlerOptions {
        show_progress: false,
        use_cursor_control: false,
        show_timestamp: false,
        experimental_ui_debug_all_events: false,
        attempt_to_print_relative_paths: false,
        filtered_event_kinds: blaze_events::EventKindSet::EMPTY,
        max_stdout_err_bytes: 0,
        show_progress_rate_limit: 0.0,
        terminal_columns: 0,
        progress_in_term_title: false,
        ui_actions_shown: 0,
    }
}

struct Fixture {
    output: Arc<FlushCollectingOutputStream>,
    clock: Arc<ManualClock>,
    handler: Arc<UiEventHandler>,
    tracker: Arc<UiStateTracker>,
}

/// `createUiEventHandler(outputKind)`.
fn create_ui_event_handler(
    output_kind: EventKind,
    ui_options: &UiEventHandlerOptions,
    skymeld_mode: bool,
) -> Fixture {
    let output = Arc::new(FlushCollectingOutputStream::default());
    let clock = Arc::new(ManualClock::default());
    let out_err = match output_kind {
        EventKind::Stdout => OutErr::create(output.clone(), Arc::new(DiscardingOutputStream)),
        EventKind::Stderr => OutErr::create(Arc::new(DiscardingOutputStream), output.clone()),
        other => panic!("AssertionError: {other}"),
    };
    let handler = Arc::new(UiEventHandler::new(
        out_err,
        ui_options,
        /* quiet= */ false,
        clock.clone(),
        /* workspacePathFragment= */ None,
    ));
    let target_width = if ui_options.use_cursor_control {
        handler.get_progress_target_width()
    } else {
        0
    };
    let tracker = Arc::new(if skymeld_mode {
        UiStateTracker::new_skymeld(clock.clone(), target_width)
    } else {
        UiStateTracker::new(clock.clone(), target_width)
    });
    tracker.set_progress_sample_size(ui_options.ui_actions_shown);
    tracker.set_new_stats_summary(false);
    handler.set_state_tracker(tracker.clone());
    handler.main_repo_mapping_computation_started();
    handler.build_started();
    Fixture {
        output,
        clock,
        handler,
        tracker,
    }
}

/// `BUILD_COMPLETE_EVENT`: a `BuildResult` without a detailed exit code, not successful.
fn build_complete_event(f: &Fixture) {
    f.handler.build_complete(false);
}

/// One run per value of the `outputKind` and `skymeldMode` test parameters.
fn stdout_and_stderr(body: impl Fn(EventKind, bool)) {
    for kind in [EventKind::Stdout, EventKind::Stderr] {
        for skymeld in [false, true] {
            body(kind, skymeld);
        }
    }
}

// Java: src/test/java/com/google/devtools/build/lib/runtime/UiEventHandlerTest.java StdoutAndStderrTest.buildComplete_outputsBuildFailedOnStderr
#[test]
fn build_complete_outputs_build_failed_on_stderr() {
    stdout_and_stderr(|kind, skymeld| {
        let f = create_ui_event_handler(kind, &new_ui_options(), skymeld);
        build_complete_event(&f);

        if kind == EventKind::Stdout {
            f.output.assert_flushed(&[]);
        } else {
            f.output.assert_flushed(&[BUILD_DID_NOT_COMPLETE_MESSAGE]);
        }
    });
}

// Java: src/test/java/com/google/devtools/build/lib/runtime/UiEventHandlerTest.java StdoutAndStderrTest.buildComplete_flushesBufferedMessage
#[test]
fn build_complete_flushes_buffered_message() {
    stdout_and_stderr(|kind, skymeld| {
        let f = create_ui_event_handler(kind, &new_ui_options(), skymeld);
        f.handler.handle(&Event::of(kind, "hello"));
        build_complete_event(&f);

        if kind == EventKind::Stdout {
            f.output.assert_flushed(&["hello"]);
        } else {
            f.output
                .assert_flushed(&["hello", &format!("\n{BUILD_DID_NOT_COMPLETE_MESSAGE}")]);
        }
    });
}

// Java: src/test/java/com/google/devtools/build/lib/runtime/UiEventHandlerTest.java StdoutAndStderrTest.buildComplete_successfulBuild
#[test]
fn build_complete_successful_build() {
    stdout_and_stderr(|kind, skymeld| {
        let f = create_ui_event_handler(kind, &new_ui_options(), skymeld);
        f.handler.handle(&Event::of(kind, ""));
        f.handler.build_complete(true);

        if kind == EventKind::Stdout {
            f.output.assert_flushed(&[]);
        } else {
            f.output.assert_flushed(&[
                "\x1b[32mINFO: \x1b[0mBuild completed successfully, 0 total actions\n",
            ]);
        }
    });
}

// Java: src/test/java/com/google/devtools/build/lib/runtime/UiEventHandlerTest.java StdoutAndStderrTest.buildComplete_emptyBuffer_outputsBuildFailedOnStderr
#[test]
fn build_complete_empty_buffer_outputs_build_failed_on_stderr() {
    stdout_and_stderr(|kind, skymeld| {
        let f = create_ui_event_handler(kind, &new_ui_options(), skymeld);
        f.handler.handle(&Event::of(kind, ""));
        build_complete_event(&f);

        if kind == EventKind::Stdout {
            f.output.assert_flushed(&[]);
        } else {
            f.output.assert_flushed(&[BUILD_DID_NOT_COMPLETE_MESSAGE]);
        }
    });
}

// Java: src/test/java/com/google/devtools/build/lib/runtime/UiEventHandlerTest.java StdoutAndStderrTest.handleOutputEvent_concatenatesInBuffer
#[test]
fn handle_output_event_concatenates_in_buffer() {
    stdout_and_stderr(|kind, skymeld| {
        let f = create_ui_event_handler(kind, &new_ui_options(), skymeld);
        f.handler.handle(&Event::of(kind, "hello "));
        f.handler.handle(&Event::of(kind, "there"));
        build_complete_event(&f);

        if kind == EventKind::Stdout {
            f.output.assert_flushed(&["hello there"]);
        } else {
            f.output.assert_flushed(&[
                "hello there",
                &format!("\n{BUILD_DID_NOT_COMPLETE_MESSAGE}"),
            ]);
        }
    });
}

/// `uiOptions.showProgress = true; uiOptions.useCursesEnum = UseCurses.YES`.
fn curses_options() -> UiEventHandlerOptions {
    UiEventHandlerOptions {
        show_progress: true,
        use_cursor_control: true,
        ..new_ui_options()
    }
}

// This test only exercises progress bar code when testing stderr output, since we don't make
// any assertions on stderr (where the progress bar is written) when testing stdout.
// Java: src/test/java/com/google/devtools/build/lib/runtime/UiEventHandlerTest.java StdoutAndStderrTest.noChangeOnUnflushedWrite
#[test]
fn no_change_on_unflushed_write() {
    stdout_and_stderr(|kind, skymeld| {
        let f = create_ui_event_handler(kind, &curses_options(), skymeld);
        if kind == EventKind::Stderr {
            assert_eq!(f.output.flushed().len(), 2, "{:?}", f.output.flushed());
            f.output.clear();
        }
        // Unterminated strings are saved in memory and not pushed out at all.
        assert!(f.output.flushed().is_empty());
        assert!(f.output.written_since_flush().is_empty());
    });
}

// Java: src/test/java/com/google/devtools/build/lib/runtime/UiEventHandlerTest.java StderrOnlyTest.buildCompleteMessageDoesntOverrideError
#[test]
fn build_complete_message_doesnt_override_error() {
    // StderrOnlyTest runs with the JUnit4 runner: `skymeldMode` keeps its default, false.
    for skymeld in [false] {
        let f = create_ui_event_handler(EventKind::Stderr, &curses_options(), skymeld);

        build_complete_event(&f);
        f.handler.handle(&Event::error("Show me this!"));
        f.handler.after_command();

        let flushed = f.output.flushed();
        assert_eq!(flushed.len(), 5, "{flushed:?}");
        assert!(flushed[3].contains("Show me this!"), "{flushed:?}");
        assert!(!flushed[4].contains(CLEAR_PROGRESS_BAR), "{flushed:?}");
    }
}

fn shared_fs() -> blaze_vfs::FileSystemRef {
    static FS: std::sync::LazyLock<blaze_vfs::FileSystemRef> =
        std::sync::LazyLock::new(|| UnixFileSystem::create(DigestHashFunction::Sha256));
    FS.clone()
}

/// `actionWithProgressMessage(progressMessage, outputPath)`: a `NullAction` with that output and
/// progress message.
#[derive(Debug)]
struct NullAction {
    base: AbstractAction,
    progress_message: String,
}

impl Action for NullAction {
    fn abstract_action(&self) -> &AbstractAction {
        &self.base
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn get_mnemonic(&self) -> String {
        "Null".into()
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

fn action_with_progress_message(progress_message: &str, output_path: &str) -> ActionRef {
    let root = ArtifactRoot::as_source_root(Root::from_path(Path::create(
        PathFragment::create("/base/exec"),
        &shared_fs(),
    )));
    let output: Artifact = Artifact::new_source_artifact(
        root,
        PathFragment::create(output_path),
        ArtifactOwner::NullOwner,
    );
    Arc::new(NullAction {
        base: AbstractAction::new(
            ActionOwner::create_dummy(
                None,
                Location::new("dummy-file", 0, 0),
                "",
                "",
                "",
                None,
                false,
                Some(empty_platform_info()),
                Vec::new(),
                IndexMap::new(),
            ),
            NestedSetBuilder::empty_set(Order::STABLE_ORDER),
            vec![output],
        ),
        progress_message: progress_message.to_owned(),
    })
}

// Java: src/test/java/com/google/devtools/build/lib/runtime/UiEventHandlerTest.java StderrOnlyTest.temporarilyDisableProgress
#[test]
fn temporarily_disable_progress() {
    // StderrOnlyTest runs with the JUnit4 runner: `skymeldMode` keeps its default, false.
    for skymeld in [false] {
        let options = UiEventHandlerOptions {
            show_progress_rate_limit: 1.0,
            ui_actions_shown: 2,
            ..curses_options()
        };
        let f = create_ui_event_handler(EventKind::Stderr, &options, skymeld);
        let action1 = action_with_progress_message("Executing action 1", "action1.out");
        let action2 = action_with_progress_message("Executing action 2", "action2.out");
        // uiEventHandler.loadingComplete(...): the tracker, then refresh().
        f.tracker
            .loading_complete(&[], Some(blaze_cmdline::repository_mapping::EMPTY.clone()));
        f.handler.refresh_progress();
        f.handler.analysis_complete();
        f.output.clear();

        // Showing progress, running actions shown.
        f.clock.advance_millis(2000);
        // uiEventHandler.runningAction(...): the tracker, then refresh().
        f.tracker.running_action(&action1, "local");
        f.handler.refresh_progress();
        let flushed = f.output.flushed();
        assert_eq!(flushed.len(), 1, "{flushed:?}");
        assert!(flushed[0].contains("Executing action 1;"), "{flushed:?}");

        // Disable progress, progress bar cleared.
        assert!(f.handler.disable_progress().unwrap());
        let flushed = f.output.flushed();
        assert_eq!(flushed.len(), 2, "{flushed:?}");
        assert!(flushed[1].ends_with(CLEAR_PROGRESS_BAR), "{flushed:?}");

        // Another action starts running, still no progress updates.
        f.clock.advance_millis(2000);
        f.tracker.running_action(&action2, "local");
        f.handler.refresh_progress();
        assert_eq!(f.output.flushed().len(), 2);

        // Enable progress again, progress bar written with both running actions.
        f.handler.enable_progress().unwrap();
        let flushed = f.output.flushed();
        assert_eq!(flushed.len(), 3, "{flushed:?}");
        assert!(flushed[2].contains("2 actions running"), "{flushed:?}");
        assert!(flushed[2].contains("Executing action 1;"), "{flushed:?}");
        assert!(flushed[2].contains("Executing action 2;"), "{flushed:?}");
    }
}

// Java: src/test/java/com/google/devtools/build/lib/runtime/UiEventHandlerTest.java StderrOnlyTest.terminalSizeChangeRefreshesProgressBarWithNewWidth
#[test]
#[ignore = "needs TerminalSizeMonitor (not ported: the terminal width is --terminal_columns)"]
fn terminal_size_change_refreshes_progress_bar_with_new_width() {}
