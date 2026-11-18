//! Port of `runtime/BuildEventStreamerTest.java`.

#![allow(deprecated)]
#![allow(clippy::new_ret_no_self, clippy::field_reassign_with_default)]

use std::collections::HashSet;
use std::sync::atomic::{AtomicI32, AtomicUsize, Ordering};
use std::sync::{Arc, Barrier, Mutex};

use blaze_actions::{Artifact, ArtifactOwner, ArtifactRoot, BuildConfigurationEvent};
use blaze_bep::buildeventstream::build_event_id_util;
use blaze_bep::buildeventstream::{
    ArtifactGroupNamer, ArtifactGroupNode, BuildEvent, BuildEventArtifactUploader,
    BuildEventContext, BuildEventTransport, GenericBuildEvent, Interrupted, PathConverter,
    ProgressEvent, ReportedArtifacts, TransportCloseError, proto_chaining, proto_chaining_with,
};
use blaze_bep::events::actions_events::{
    ActionExecutedEvent, ActionFailure, ErrorTiming, ExecutedActionInfo, ExecutedActionOwner,
};
use blaze_bep::events::buildtool_events::{
    BuildResultData, BuildStartingData, BuildStartingEvent, build_complete_event,
};
use blaze_bep::events::completion_context::{CompletionContext, ImportantInputMap};
use blaze_bep::runtime::build_event_streamer::{
    BuildEventStreamer, Builder, OutErrProvider, construct_oom_exit_message, consume_as_pairs,
};
use blaze_bep::runtime::counting_artifact_group_namer::CountingArtifactGroupNamer;
use blaze_cmdline::label::Label;
use blaze_collect::nestedset::{NestedSet, NestedSetBuilder, Order};
use blaze_exec::actions::file_artifact_value::FileArtifactValue;
use blaze_optdefs::misc::build_event_protocol_options::{
    BuildEventProtocolOptions, OutputGroupFileModes,
};
use blaze_optdefs::misc::build_event_service_options::BesUploadMode;
use blaze_optdefs::misc::build_event_stream_options::BuildEventStreamOptions;
use blaze_protos::build_event_stream::aborted::AbortReason;
use blaze_protos::build_event_stream::build_event::Payload;
use blaze_protos::build_event_stream::build_event_id::{Id, NamedSetOfFilesId};
use blaze_protos::build_event_stream::{self as bes, BuildEventId};
use blaze_protos::failure_details::failure_detail::Category;
use blaze_protos::failure_details::{Crash, FailureDetail, Spawn, Throwable, crash, spawn};
use blaze_util::DetailedExitCode;
use blaze_vfs::{DigestHashFunction, FileSystemRef, Path, PathFragment, Root, UnixFileSystem};

const OOM_MESSAGE: &str = "Please build fewer targets.";

// ---------------------------------------------------------------------------------------------
// Test infrastructure
// ---------------------------------------------------------------------------------------------

/// `getTestBuildEventContext(artifactGroupNamer)`.
struct TestBuildEventContext<'a> {
    artifact_group_namer: &'a dyn ArtifactGroupNamer,
    options: &'a BuildEventProtocolOptions,
}

/// `Path::toString`.
struct PathToString;

impl PathConverter for PathToString {
    fn apply(&self, path: &Path) -> Option<String> {
        Some(path.get_path_string().to_string())
    }
}

impl BuildEventContext for TestBuildEventContext<'_> {
    fn path_converter(&self) -> &dyn PathConverter {
        &PathToString
    }

    fn artifact_group_namer(&self) -> &dyn ArtifactGroupNamer {
        self.artifact_group_namer
    }

    fn get_options(&self) -> &BuildEventProtocolOptions {
        self.options
    }
}

#[derive(Default)]
struct Recorded {
    events: Vec<Arc<dyn BuildEvent>>,
    events_as_protos: Vec<bes::BuildEvent>,
}

struct RecordingBuildEventTransport {
    recorded: Mutex<Recorded>,
    artifact_group_namer: Arc<CountingArtifactGroupNamer>,
    /// `Options.getDefaults(BuildEventProtocolOptions.class)`, computed once.
    options: BuildEventProtocolOptions,
}

impl RecordingBuildEventTransport {
    fn new(namer: Arc<CountingArtifactGroupNamer>) -> Self {
        RecordingBuildEventTransport {
            recorded: Mutex::new(Recorded::default()),
            artifact_group_namer: namer,
            options: BuildEventProtocolOptions::default(),
        }
    }

    fn get_events(&self) -> Vec<Arc<dyn BuildEvent>> {
        self.recorded.lock().unwrap().events.clone()
    }

    fn get_event_protos(&self) -> Vec<bes::BuildEvent> {
        self.recorded.lock().unwrap().events_as_protos.clone()
    }

    fn context(&self) -> TestBuildEventContext<'_> {
        TestBuildEventContext {
            artifact_group_namer: self.artifact_group_namer.as_ref(),
            options: &self.options,
        }
    }
}

impl BuildEventTransport for RecordingBuildEventTransport {
    fn name(&self) -> String {
        "RecordingBuildEventTransport".to_string()
    }

    fn may_be_slow(&self) -> bool {
        false
    }

    fn get_bes_upload_mode(&self) -> BesUploadMode {
        BesUploadMode::WAIT_FOR_UPLOAD_COMPLETE
    }

    // Java: synchronized.
    fn send_build_event(&self, event: &Arc<dyn BuildEvent>) {
        let mut recorded = self.recorded.lock().unwrap();
        recorded.events.push(event.clone());
        let proto = event
            .as_stream_proto(&self.context())
            .unwrap_or_else(|_| panic!("interrupts not supported in test instance"));
        recorded.events_as_protos.push(proto);
    }

    fn close(&self) -> Result<(), TransportCloseError> {
        Ok(())
    }

    fn get_uploader(&self) -> Option<Arc<dyn BuildEventArtifactUploader>> {
        panic!("IllegalStateException")
    }
}

/// The test class's fields: the namer, the recording transport and the streamer under test.
struct Fixture {
    artifact_group_namer: Arc<CountingArtifactGroupNamer>,
    transport: Arc<RecordingBuildEventTransport>,
    streamer: BuildEventStreamer,
}

impl Fixture {
    fn new() -> Fixture {
        let artifact_group_namer = Arc::new(CountingArtifactGroupNamer::new());
        let transport = Arc::new(RecordingBuildEventTransport::new(
            artifact_group_namer.clone(),
        ));
        let streamer = Builder::new()
            .artifact_group_namer(artifact_group_namer.clone())
            .build_event_transports(vec![transport.clone() as Arc<dyn BuildEventTransport>])
            .bes_stream_options(new_build_event_stream_options())
            .oom_message(OOM_MESSAGE.to_string())
            .build();
        Fixture {
            artifact_group_namer,
            transport,
            streamer,
        }
    }

    /// `getBepEvent(buildEventId)`.
    fn get_bep_event(&self, build_event_id: &BuildEventId) -> Option<bes::BuildEvent> {
        self.transport
            .get_event_protos()
            .into_iter()
            .find(|e| e.id.as_ref() == Some(build_event_id))
    }
}

/// `new BuildEventStreamOptions()`. PORT: Java's constructor leaves every field at its Java
/// default (`0`, `false`, `null`), unlike `Options.getDefaults`; of the fields the streamer reads,
/// only `maxNamedSetEntries` differs from the option default.
fn new_build_event_stream_options() -> BuildEventStreamOptions {
    let mut options = BuildEventStreamOptions::default();
    options.max_named_set_entries = 0;
    options.publish_all_actions = false;
    options
}

/// Java reference equality of two events (`assertThat(a).isEqualTo(b)` / `contains` on classes
/// that do not override `equals`).
fn same(a: &Arc<dyn BuildEvent>, b: &Arc<dyn BuildEvent>) -> bool {
    std::ptr::addr_eq(Arc::as_ptr(a), Arc::as_ptr(b))
}

fn contains(events: &[Arc<dyn BuildEvent>], event: &Arc<dyn BuildEvent>) -> bool {
    events.iter().any(|e| same(e, event))
}

fn test_id(opaque: &str) -> BuildEventId {
    build_event_id_util::unknown_build_event_id(opaque)
}

fn generic(id: BuildEventId, children: Vec<BuildEventId>) -> Arc<dyn BuildEvent> {
    Arc::new(GenericBuildEvent::new(id, children))
}

/// `proto.getProgress()` (the default instance if the payload is something else).
fn progress(proto: &bes::BuildEvent) -> bes::Progress {
    match &proto.payload {
        Some(Payload::Progress(p)) => p.clone(),
        _ => bes::Progress::default(),
    }
}

/// `proto.getNamedSetOfFiles()`.
fn named_set_of_files(proto: &bes::BuildEvent) -> bes::NamedSetOfFiles {
    match &proto.payload {
        Some(Payload::NamedSetOfFiles(n)) => n.clone(),
        _ => bes::NamedSetOfFiles::default(),
    }
}

/// `proto.hasAborted()`.
fn has_aborted(proto: &bes::BuildEvent) -> bool {
    matches!(proto.payload, Some(Payload::Aborted(_)))
}

/// `proto.getAborted()`.
fn aborted(proto: &bes::BuildEvent) -> bes::Aborted {
    match &proto.payload {
        Some(Payload::Aborted(a)) => a.clone(),
        _ => bes::Aborted::default(),
    }
}

/// `id.getNamedSet()`.
fn named_set_id(proto: &bes::BuildEvent) -> NamedSetOfFilesId {
    match proto.id.as_ref().and_then(|id| id.id.as_ref()) {
        Some(Id::NamedSet(n)) => n.clone(),
        _ => NamedSetOfFilesId::default(),
    }
}

/// `file.getUri()`.
fn uri(file: &bes::File) -> String {
    match &file.file {
        Some(bes::file::File::Uri(u)) => u.clone(),
        _ => String::new(),
    }
}

#[derive(Debug)]
struct GenericOrderEvent {
    id: BuildEventId,
    children: Vec<BuildEventId>,
    after: Vec<BuildEventId>,
}

impl GenericOrderEvent {
    fn new(
        id: BuildEventId,
        children: Vec<BuildEventId>,
        after: Vec<BuildEventId>,
    ) -> Arc<dyn BuildEvent> {
        Arc::new(GenericOrderEvent {
            id,
            children,
            after,
        })
    }

    fn with_children(id: BuildEventId, children: Vec<BuildEventId>) -> Arc<dyn BuildEvent> {
        Self::new(id, children.clone(), children)
    }
}

impl BuildEvent for GenericOrderEvent {
    fn get_event_id(&self) -> BuildEventId {
        self.id.clone()
    }

    fn get_children_events(&self) -> Vec<BuildEventId> {
        self.children.clone()
    }

    fn as_stream_proto(
        &self,
        _converters: &dyn BuildEventContext,
    ) -> Result<bes::BuildEvent, Interrupted> {
        Ok(proto_chaining(self))
    }

    fn posted_after(&self) -> Option<Vec<BuildEventId>> {
        Some(self.after.clone())
    }
}

#[derive(Debug)]
struct GenericArtifactReportingEvent {
    id: BuildEventId,
    children: Vec<BuildEventId>,
    artifacts: Vec<NestedSet<Artifact>>,
}

impl GenericArtifactReportingEvent {
    fn new(id: BuildEventId, artifacts: Vec<NestedSet<Artifact>>) -> Arc<dyn BuildEvent> {
        Arc::new(GenericArtifactReportingEvent {
            id,
            children: Vec::new(),
            artifacts,
        })
    }
}

impl BuildEvent for GenericArtifactReportingEvent {
    fn get_event_id(&self) -> BuildEventId {
        self.id.clone()
    }

    fn get_children_events(&self) -> Vec<BuildEventId> {
        self.children.clone()
    }

    fn reported_artifacts(
        &self,
        _output_group_file_modes: &OutputGroupFileModes,
    ) -> Option<ReportedArtifacts> {
        let mut important_input_map = ImportantInputMap::new();
        for artifact_set in &self.artifacts {
            for artifact in artifact_set.to_list().iter() {
                // This is good enough to make the tests pass because they don't care about the
                // metadata.
                important_input_map.put(artifact.clone(), FileArtifactValue::MissingFileMarker);
            }
        }
        Some(ReportedArtifacts::new(
            self.artifacts.clone(),
            Arc::new(CompletionContext::new(
                important_input_map,
                /* expand_filesets= */ false,
            )),
        ))
    }

    fn as_stream_proto(
        &self,
        converters: &dyn BuildEventContext,
    ) -> Result<bes::BuildEvent, Interrupted> {
        let mut builder = bes::NamedSetOfFiles::default();
        for artifactset in &self.artifacts {
            builder.file_sets.push(
                converters
                    .artifact_group_namer()
                    .apply(&ArtifactGroupNode::Artifacts(artifactset.to_node()))
                    .expect("NullPointerException: unnamed artifact set"),
            );
        }
        Ok(proto_chaining_with(self, Payload::NamedSetOfFiles(builder)))
    }
}

#[derive(Debug)]
struct GenericConfigurationEvent {
    id: BuildEventId,
    children: Vec<BuildEventId>,
    configurations: Vec<Arc<dyn BuildEvent>>,
}

impl GenericConfigurationEvent {
    fn new(id: BuildEventId, configuration: Arc<dyn BuildEvent>) -> Arc<dyn BuildEvent> {
        Arc::new(GenericConfigurationEvent {
            id,
            children: Vec::new(),
            configurations: vec![configuration],
        })
    }
}

impl BuildEvent for GenericConfigurationEvent {
    fn get_event_id(&self) -> BuildEventId {
        self.id.clone()
    }

    fn get_children_events(&self) -> Vec<BuildEventId> {
        self.children.clone()
    }

    fn get_configurations(&self) -> Option<Vec<Arc<dyn BuildEvent>>> {
        Some(self.configurations.clone())
    }

    fn as_stream_proto(
        &self,
        _converters: &dyn BuildEventContext,
    ) -> Result<bes::BuildEvent, Interrupted> {
        Ok(proto_chaining(self))
    }
}

#[derive(Debug)]
struct ReplaceableTestBuildEvent {
    inner: GenericBuildEvent,
    replaceable: bool,
}

impl ReplaceableTestBuildEvent {
    fn new(id: BuildEventId, replaceable: bool) -> Arc<dyn BuildEvent> {
        Arc::new(ReplaceableTestBuildEvent {
            inner: GenericBuildEvent::new(id, Vec::new()),
            replaceable,
        })
    }
}

impl BuildEvent for ReplaceableTestBuildEvent {
    fn get_event_id(&self) -> BuildEventId {
        self.inner.get_event_id()
    }

    fn get_children_events(&self) -> Vec<BuildEventId> {
        self.inner.get_children_events()
    }

    fn as_stream_proto(
        &self,
        context: &dyn BuildEventContext,
    ) -> Result<bes::BuildEvent, Interrupted> {
        self.inner.as_stream_proto(context)
    }

    fn replaceable(&self) -> Option<bool> {
        Some(self.replaceable)
    }
}

/// `mock(BuildEventStreamer.OutErrProvider.class)` with `when(getOut()).thenReturn(a)
/// .thenReturn(b)...`: answers in order, the last one repeated; calls are counted for
/// `verify(outErr, times(n))`.
struct MockOutErr {
    outs: Mutex<Vec<Vec<String>>>,
    errs: Mutex<Vec<Vec<String>>>,
    out_calls: AtomicUsize,
    err_calls: AtomicUsize,
}

impl MockOutErr {
    fn new(outs: Vec<Vec<&str>>, errs: Vec<Vec<&str>>) -> Arc<MockOutErr> {
        let own = |v: Vec<Vec<&str>>| -> Vec<Vec<String>> {
            v.into_iter()
                .map(|chunks| chunks.into_iter().map(str::to_string).collect())
                .collect()
        };
        Arc::new(MockOutErr {
            outs: Mutex::new(own(outs)),
            errs: Mutex::new(own(errs)),
            out_calls: AtomicUsize::new(0),
            err_calls: AtomicUsize::new(0),
        })
    }

    fn next_answer(answers: &Mutex<Vec<Vec<String>>>) -> Vec<String> {
        let mut answers = answers.lock().unwrap();
        if answers.len() > 1 {
            answers.remove(0)
        } else {
            answers[0].clone()
        }
    }

    fn out_calls(&self) -> usize {
        self.out_calls.load(Ordering::SeqCst)
    }

    fn err_calls(&self) -> usize {
        self.err_calls.load(Ordering::SeqCst)
    }
}

impl OutErrProvider for MockOutErr {
    fn get_out(&self) -> Option<Vec<String>> {
        self.out_calls.fetch_add(1, Ordering::SeqCst);
        Some(Self::next_answer(&self.outs))
    }

    fn get_err(&self) -> Option<Vec<String>> {
        self.err_calls.fetch_add(1, Ordering::SeqCst);
        Some(Self::next_answer(&self.errs))
    }
}

/// `CrashFailureDetails.detailedExitCodeForThrowable(new IllegalStateException("Unspecified
/// DetailedExitCode"))`, what `BuildResult.getDetailedExitCode()` returns when none was set.
/// PORT: the cause's stack trace is left empty; no test reads the failure detail.
fn unspecified_detailed_exit_code() -> DetailedExitCode {
    DetailedExitCode::of(FailureDetail {
        message: "Crashed: (java.lang.IllegalStateException) Unspecified DetailedExitCode"
            .to_string(),
        category: Some(Category::Crash(Crash {
            code: crash::Code::CrashUnknown as i32,
            causes: vec![Throwable {
                throwable_class: "java.lang.IllegalStateException".to_string(),
                message: "Unspecified DetailedExitCode".to_string(),
                stack_trace: Vec::new(),
            }],
            ..Default::default()
        })),
    })
}

/// `new BuildCompleteEvent(new BuildResult(0), children)`: no exit code set (so not a success),
/// no crash, no catastrophe, keep going.
fn new_build_complete_event(children: Vec<BuildEventId>) -> Arc<dyn BuildEvent> {
    Arc::new(build_complete_event(
        &BuildResultData {
            detailed_exit_code: unspecified_detailed_exit_code(),
            stop_time_millis: 0,
            has_unhandled_throwable: false,
            was_catastrophe: false,
            success: false,
            stop_on_first_failure: false,
        },
        children,
    ))
}

/// `buildCompleteEvent(detailedExitCode, stopOnFailure, crash, catastrophe, childrenEvents)`.
/// `crash` is whether a throwable is set.
fn build_complete_event_with(
    detailed_exit_code: DetailedExitCode,
    stop_on_failure: bool,
    crash: bool,
    catastrophe: bool,
    children_events: Vec<BuildEventId>,
) -> Arc<dyn BuildEvent> {
    let success = detailed_exit_code.is_success();
    Arc::new(build_complete_event(
        &BuildResultData {
            detailed_exit_code,
            stop_time_millis: 0,
            has_unhandled_throwable: crash,
            was_catastrophe: catastrophe,
            success,
            stop_on_first_failure: stop_on_failure,
        },
        children_events,
    ))
}

fn create_generic_detailed_exit_code() -> DetailedExitCode {
    DetailedExitCode::of(FailureDetail {
        category: Some(Category::Spawn(Spawn {
            code: spawn::Code::NonZeroExit as i32,
            ..Default::default()
        })),
        ..Default::default()
    })
}

fn file_system() -> FileSystemRef {
    Arc::new(UnixFileSystem::new(DigestHashFunction::Sha256))
}

/// `outputBase` of `FoundationTestCase`. PORT: on a Unix file system instead of the in-memory
/// one (not ported); nothing is created on disk.
fn output_base() -> Path {
    Path::create_from_str("/usr/local/google/_blaze_jrluser/FAKEMD5", &file_system())
}

/// `makeArtifact(pathString)`: a source artifact under `outputBase`.
fn make_artifact(path_string: &str) -> Artifact {
    let output_base = output_base();
    let path = output_base.get_relative(&PathFragment::create(path_string));
    let root = ArtifactRoot::as_source_root(Root::from_path(output_base));
    // ActionsTestUtil.createArtifact(root, path)
    let root_relative_path = root.get_root().relativize(&path);
    let exec_path = root.get_exec_path().get_relative(&root_relative_path);
    Artifact::new_source_artifact(root, exec_path, ArtifactOwner::NullOwner)
}

/// `ActionsTestUtil.DUMMY_ARTIFACT`. PORT: owned by `NULL_OWNER` instead of
/// `NULL_ARTIFACT_OWNER` (the test does not read the owner).
fn dummy_artifact() -> Artifact {
    Artifact::new_source_artifact(
        ArtifactRoot::as_source_root(Root::absolute_root(&file_system())),
        PathFragment::create("/dummy"),
        ArtifactOwner::NullOwner,
    )
}

/// What `ActionExecutedEvent` reads of `new ActionsTestUtil.NullAction()`. PORT: the
/// `BuildConfigurationEvent` of `NULL_ACTION_OWNER` has a default id and proto in Java; the
/// port's struct holds the checksum (empty) and the default `Configuration`.
fn null_action() -> ExecutedActionInfo {
    ExecutedActionInfo {
        mnemonic: "Null".to_string(),
        owner: Some(ExecutedActionOwner {
            label: Some(Label::parse_canonical_unchecked("//null/action:owner")),
            configuration_checksum: Some("dummy-configuration".to_string()),
            build_configuration_event: Some(BuildConfigurationEvent {
                event_id: String::new(),
                configuration: bes::Configuration::default(),
            }),
        }),
        arguments: None,
        is_extra_action: false,
    }
}

/// `SUCCESSFUL_ACTION_EXECUTED_EVENT`.
fn successful_action_executed_event() -> Arc<dyn BuildEvent> {
    let dummy = dummy_artifact();
    Arc::new(ActionExecutedEvent::new(
        dummy.get_exec_path().clone(),
        null_action(),
        /* exception= */ None,
        dummy.get_path(),
        dummy.clone(),
        Some(FileArtifactValue::MissingFileMarker),
        /* stdout= */ None,
        /* stderr= */ None,
        ErrorTiming::NoError,
        /* start_time= */ None,
        /* end_time= */ None,
    ))
}

/// The `failedActionExecutedEvent` of the action publishing tests: an
/// `ActionExecutionException("Exception", null, false, DetailedExitCode.of(Spawn
/// EXECUTION_DENIED))`.
fn failed_action_executed_event() -> Arc<dyn BuildEvent> {
    let dummy = dummy_artifact();
    let failure_detail = FailureDetail {
        category: Some(Category::Spawn(Spawn {
            code: spawn::Code::ExecutionDenied as i32,
            ..Default::default()
        })),
        ..Default::default()
    };
    let detailed_exit_code = DetailedExitCode::of(failure_detail.clone());
    Arc::new(ActionExecutedEvent::new(
        dummy.get_exec_path().clone(),
        null_action(),
        Some(ActionFailure {
            numeric_exit_code: detailed_exit_code.get_exit_code().get_numeric_exit_code(),
            failure_detail: Some(failure_detail),
        }),
        dummy.get_path(),
        dummy.clone(),
        /* primary_output_metadata= */ None,
        /* stdout= */ None,
        /* stderr= */ None,
        ErrorTiming::BeforeExecution,
        /* start_time= */ None,
        /* end_time= */ None,
    ))
}

/// `makeTestingBuildConfigurationValue().toBuildEvent()`. PORT:
/// `BuildConfigurationValue.createForTesting` is not reachable from this crate; a
/// `BuildConfigurationEvent` with the test's mnemonic stands in (the tests only compare it with
/// itself and look at its id kind). The checksum is a placeholder, not Bazel's.
fn make_testing_build_configuration_event() -> Arc<dyn BuildEvent> {
    Arc::new(BuildConfigurationEvent {
        event_id: "test-configuration-checksum".to_string(),
        configuration: bes::Configuration {
            mnemonic: "some_mnemonic".to_string(),
            ..Default::default()
        },
    })
}

/// `BuildConfigurationEvent.equals`: same event id and same proto.
fn assert_equal_configuration_events(
    transport: &RecordingBuildEventTransport,
    actual: &Arc<dyn BuildEvent>,
    expected: &Arc<dyn BuildEvent>,
) {
    assert_eq!(actual.get_event_id(), expected.get_event_id());
    let ctx = transport.context();
    assert_eq!(
        actual.as_stream_proto(&ctx).unwrap(),
        expected.as_stream_proto(&ctx).unwrap()
    );
}

// ---------------------------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------------------------

#[test]
fn test_simple_stream() {
    // Verify that a well-formed event is passed through and that completion of the
    // build clears the pending progress-update event. However, there is no guarantee
    // on the order of the flushed events.
    // Additionally, assert that the actual last event has the last_message flag set.
    //
    // PORT: the Java test also registers an `EventBusHandler` on the test's event bus and posts
    // `AnnounceBuildEventTransportsEvent` / `BuildEventTransportClosedEvent` to it itself; those
    // events are not ported and the streamer does not take part in that exchange, so only the
    // streamer assertions are kept.
    let f = Fixture::new();

    let start_event = generic(
        test_id("Initial"),
        vec![
            ProgressEvent::initial_progress_update(),
            build_event_id_util::build_finished(),
        ],
    );

    f.streamer.build_event(start_event.clone());

    assert!(!f.streamer.is_closed());
    let after_first_event = f.transport.get_events();
    assert_eq!(after_first_event.len(), 1);
    assert_eq!(
        after_first_event[0].get_event_id(),
        start_event.get_event_id()
    );

    f.streamer.build_event(new_build_complete_event(Vec::new()));

    assert!(f.streamer.is_closed());

    let final_stream = f.transport.get_events();
    assert_eq!(final_stream.len(), 3);
    assert_eq!(
        HashSet::from([
            final_stream[1].get_event_id(),
            final_stream[2].get_event_id()
        ]),
        HashSet::from([
            build_event_id_util::build_finished(),
            ProgressEvent::initial_progress_update()
        ])
    );

    // verify the "last_message" flag.
    let protos = f.transport.get_event_protos();
    assert!(!protos[0].last_message);
    assert!(!protos[1].last_message);
    assert!(protos[2].last_message);
}

#[test]
fn test_chaining() {
    // Verify that unannounced events are linked in with progress update events, assuming
    // a correctly formed initial event.
    let f = Fixture::new();
    let start_event = generic(
        test_id("Initial"),
        vec![ProgressEvent::initial_progress_update()],
    );
    let unexpected_event = generic(test_id("unexpected"), Vec::new());

    f.streamer.build_event(start_event.clone());
    f.streamer.build_event(unexpected_event.clone());

    assert!(!f.streamer.is_closed());
    let events_seen = f.transport.get_events();
    assert_eq!(events_seen.len(), 3);
    assert_eq!(events_seen[0].get_event_id(), start_event.get_event_id());
    assert_eq!(
        events_seen[2].get_event_id(),
        unexpected_event.get_event_id()
    );
    let link_event = &events_seen[1];
    assert_eq!(
        link_event.get_event_id(),
        ProgressEvent::initial_progress_update()
    );
    assert!(
        link_event
            .get_children_events()
            .contains(&unexpected_event.get_event_id()),
        "Unexpected events should be linked"
    );
}

#[test]
fn test_bad_initial_event() {
    // Verify that, if the initial event does not announce the initial progress update event,
    // the initial progress event is used instead to chain that event; in this way, new
    // progress updates can always be chained in.
    let f = Fixture::new();
    let unexpected_start_event = generic(test_id("unexpected start"), Vec::new());

    f.streamer.build_event(unexpected_start_event.clone());

    let events_seen = f.transport.get_events();
    assert_eq!(events_seen.len(), 2);
    assert_eq!(
        events_seen[1].get_event_id(),
        unexpected_start_event.get_event_id()
    );
    let initial = events_seen[0].clone();
    assert_eq!(
        initial.get_event_id(),
        ProgressEvent::initial_progress_update()
    );
    assert!(
        initial
            .get_children_events()
            .contains(&unexpected_start_event.get_event_id()),
        "Event should be linked"
    );

    // The initial event should also announce a new progress event; we test this
    // by streaming another unannounced event.

    let unexpected_event = generic(test_id("unexpected"), Vec::new());

    f.streamer.build_event(unexpected_event.clone());

    assert!(!f.streamer.is_closed());
    let all_events_seen = f.transport.get_events();
    assert_eq!(all_events_seen.len(), 4);
    assert_eq!(
        all_events_seen[3].get_event_id(),
        unexpected_event.get_event_id()
    );
    let second_link_event = &all_events_seen[2];
    assert!(
        initial
            .get_children_events()
            .contains(&second_link_event.get_event_id()),
        "Progress should have been announced"
    );
    assert!(
        second_link_event
            .get_children_events()
            .contains(&unexpected_event.get_event_id()),
        "Second event should be linked"
    );
}

#[test]
fn test_refer_past_event() {
    // Verify that, if an event is refers to a previously done event, that duplicated
    // late-referenced event is not expected again.
    let f = Fixture::new();
    let start_event = generic(
        test_id("Initial"),
        vec![
            ProgressEvent::initial_progress_update(),
            build_event_id_util::build_finished(),
        ],
    );
    let early_event = generic(test_id("unexpected"), Vec::new());
    let late_reference = generic(test_id("late reference"), vec![early_event.get_event_id()]);

    f.streamer.build_event(start_event);
    f.streamer.build_event(early_event.clone());
    f.streamer.build_event(late_reference);
    f.streamer.build_event(new_build_complete_event(Vec::new()));

    assert!(f.streamer.is_closed());
    let events_seen = f.transport.get_events();
    let early_event_count = events_seen
        .iter()
        .filter(|event| event.get_event_id() == early_event.get_event_id())
        .count();
    // The early event should be reported precisely once.
    assert_eq!(early_event_count, 1);
}

#[test]
fn test_reordering() {
    // Verify that an event requiring to be posted after another one is indeed.
    let f = Fixture::new();
    let expected_id = test_id("the target");
    let start_event = generic(
        test_id("Initial"),
        vec![
            ProgressEvent::initial_progress_update(),
            expected_id.clone(),
        ],
    );
    let root_cause = generic(test_id("failure event"), Vec::new());
    let failed_target =
        GenericOrderEvent::with_children(expected_id, vec![root_cause.get_event_id()]);

    f.streamer.build_event(start_event.clone());
    f.streamer.build_event(failed_target.clone());
    f.streamer.build_event(root_cause.clone());

    assert!(!f.streamer.is_closed());
    let all_events_seen = f.transport.get_events();
    assert_eq!(all_events_seen.len(), 4);
    assert_eq!(
        all_events_seen[0].get_event_id(),
        start_event.get_event_id()
    );
    let link_event = &all_events_seen[1];
    assert_eq!(
        link_event.get_event_id(),
        ProgressEvent::initial_progress_update()
    );
    assert_eq!(all_events_seen[2].get_event_id(), root_cause.get_event_id());
    assert_eq!(
        all_events_seen[3].get_event_id(),
        failed_target.get_event_id()
    );
}

fn index_ordered_build_event(index: i32, after_index: i32) -> Arc<dyn BuildEvent> {
    GenericOrderEvent::new(
        test_id(&format!("Concurrent-{index}")),
        Vec::new(),
        if after_index == -1 {
            Vec::new()
        } else {
            vec![test_id(&format!("Concurrent-{after_index}"))]
        },
    )
}

#[test]
fn test_concurrency() {
    // Verify that we can blast the BuildEventStreamer with many build events in parallel without
    // violating internal consistency. The thread-safety under test is primarily sensitive to the
    // pendingEvents field constructed when there are ordering constraints, so we make sure to
    // include such ordering constraints in this test.
    let f = Fixture::new();
    let start_event = generic(
        test_id("Initial"),
        vec![
            ProgressEvent::initial_progress_update(),
            build_event_id_util::build_finished(),
        ],
    );
    f.streamer.build_event(start_event.clone());

    let num_threads = 12;
    let num_events_per_thread = 10_000;
    let total_events = num_threads * num_events_per_thread;
    let id_index = AtomicI32::new(0);

    std::thread::scope(|s| {
        for _ in 0..num_threads {
            s.spawn(|| {
                for _ in 0..num_events_per_thread {
                    let index = id_index.fetch_add(1, Ordering::SeqCst);
                    // Arrange for half of the events to have an ordering constraint on the
                    // subsequent event. The ordering graph must avoid cycles.
                    let after_index = if index % 2 == 0 {
                        (index + 1) % total_events
                    } else {
                        -1
                    };
                    f.streamer
                        .build_event(index_ordered_build_event(index, after_index));
                }
            });
        }
    });

    let late_id = test_id("late event");
    f.streamer
        .build_event(new_build_complete_event(vec![late_id.clone()]));
    assert!(!f.streamer.is_closed());
    f.streamer.build_event(generic(late_id, Vec::new()));
    assert!(f.streamer.is_closed());

    let events_seen = f.transport.get_events();
    assert_eq!(events_seen[0].get_event_id(), start_event.get_event_id());
    assert_eq!(events_seen.len(), 4 + total_events as usize * 2);
}

// Re-enable this "test" for ad-hoc benchmarking of many concurrent build events.
#[test]
#[ignore = "@Ignore in Java: ad-hoc benchmark"]
fn concurrency_benchmark() {
    let f = Fixture::new();
    let mut time = 0u128;
    for _iteration in 0..3 {
        let watch = std::time::Instant::now();

        let start_event = generic(
            test_id("Initial"),
            vec![
                ProgressEvent::initial_progress_update(),
                build_event_id_util::build_finished(),
            ],
        );
        f.streamer.build_event(start_event);

        let num_threads = 12;
        let num_events_per_thread = 100_000;
        let total_events = num_threads * num_events_per_thread;
        let id_index = AtomicI32::new(0);

        std::thread::scope(|s| {
            for _ in 0..num_threads {
                s.spawn(|| {
                    for _ in 0..num_events_per_thread {
                        let index = id_index.fetch_add(1, Ordering::SeqCst);
                        // Arrange for half of the events to have an ordering constraint on the
                        // subsequent event. The ordering graph must avoid cycles.
                        let after_index = if index % 2 == 0 {
                            (index + 1) % total_events
                        } else {
                            -1
                        };
                        f.streamer
                            .build_event(index_ordered_build_event(index, after_index));
                    }
                });
            }
        });

        time += watch.elapsed().as_millis();

        let late_id = test_id("late event");
        f.streamer
            .build_event(new_build_complete_event(vec![late_id.clone()]));
        assert!(!f.streamer.is_closed());
        f.streamer.build_event(generic(late_id, Vec::new()));
        assert!(f.streamer.is_closed());
    }

    eprintln!();
    eprintln!("=============================================================");
    eprintln!("Concurrent performance of BEP build event processing: {time}ms");
    eprintln!("=============================================================");
}

#[test]
fn test_missing_prerequisites() {
    // Verify that an event where the prerequisite is never coming till the end of
    // the build still gets posted, with the prerequisite aborted.
    let f = Fixture::new();
    let expected_id = test_id("the target");
    let start_event = generic(
        test_id("Initial"),
        vec![
            ProgressEvent::initial_progress_update(),
            expected_id.clone(),
            build_event_id_util::build_finished(),
        ],
    );
    let root_cause_id = test_id("failure event");
    let failed_target = GenericOrderEvent::with_children(expected_id, vec![root_cause_id.clone()]);

    f.streamer.build_event(start_event.clone());
    f.streamer.build_event(failed_target.clone());
    f.streamer.build_event(new_build_complete_event(Vec::new()));

    assert!(f.streamer.is_closed());
    let all_events_seen = f.transport.get_events();
    assert_eq!(all_events_seen.len(), 6);
    assert_eq!(
        all_events_seen[0].get_event_id(),
        start_event.get_event_id()
    );
    assert_eq!(
        all_events_seen[1].get_event_id(),
        build_event_id_util::build_finished()
    );
    let link_event = &all_events_seen[2];
    assert_eq!(
        link_event.get_event_id(),
        ProgressEvent::initial_progress_update()
    );
    assert_eq!(all_events_seen[3].get_event_id(), root_cause_id);
    assert_eq!(
        all_events_seen[4].get_event_id(),
        failed_target.get_event_id()
    );
}

#[test]
fn test_very_first_event_needs_to_wait() {
    // Verify that we can handle an first event waiting for another event.
    let f = Fixture::new();
    let initial_id = test_id("Initial");
    let wait_id = test_id("Waiting for initial event");
    let start_event = generic(
        initial_id.clone(),
        vec![ProgressEvent::initial_progress_update(), wait_id.clone()],
    );
    let waiting_for_start = GenericOrderEvent::new(wait_id, Vec::new(), vec![initial_id]);

    f.streamer.build_event(waiting_for_start.clone());
    f.streamer.build_event(start_event.clone());

    assert!(!f.streamer.is_closed());
    let all_events_seen = f.transport.get_events();
    assert_eq!(all_events_seen.len(), 2);
    assert_eq!(
        all_events_seen[0].get_event_id(),
        start_event.get_event_id()
    );
    assert_eq!(
        all_events_seen[1].get_event_id(),
        waiting_for_start.get_event_id()
    );
}

#[test]
fn test_reported_artifacts() {
    // Verify that reported artifacts are correctly unfolded into the stream
    let f = Fixture::new();
    let start_event = generic(
        test_id("Initial"),
        vec![ProgressEvent::initial_progress_update()],
    );

    let a = make_artifact("path/a");
    let b = make_artifact("path/b");
    let c = make_artifact("path/c");
    let inner_group = NestedSetBuilder::<Artifact>::stable_order()
        .add(a.clone())
        .add(b.clone())
        .build();
    let group = NestedSetBuilder::<Artifact>::stable_order()
        .add_transitive(&inner_group)
        .add(c)
        .build();
    let reporting_artifacts = GenericArtifactReportingEvent::new(test_id("reporting"), vec![group]);

    f.streamer.build_event(start_event.clone());
    f.streamer.build_event(reporting_artifacts);

    assert!(!f.streamer.is_closed());
    let all_events_seen = f.transport.get_events();
    let event_protos = f.transport.get_event_protos();
    assert_eq!(all_events_seen.len(), 7);
    assert_eq!(
        all_events_seen[0].get_event_id(),
        start_event.get_event_id()
    );
    assert_eq!(
        all_events_seen[1].get_event_id(),
        ProgressEvent::initial_progress_update()
    );
    let first_set_directs = named_set_of_files(&event_protos[2]).files;
    assert_eq!(first_set_directs.len(), 2);
    assert_eq!(
        HashSet::from([uri(&first_set_directs[0]), uri(&first_set_directs[1])]),
        HashSet::from([
            a.get_path().get_path_string().to_string(),
            b.get_path().get_path_string().to_string()
        ])
    );
    let second_set_transitives = named_set_of_files(&event_protos[4]).file_sets;
    assert_eq!(second_set_transitives.len(), 1);
    assert_eq!(second_set_transitives[0], named_set_id(&event_protos[2]));
    let reported_artifact_sets = named_set_of_files(&event_protos[6]).file_sets;
    assert_eq!(reported_artifact_sets.len(), 1);
    assert_eq!(reported_artifact_sets[0], named_set_id(&event_protos[4]));
}

#[test]
fn test_artifact_sets_precede_reporting_event() {
    // Verify that reported artifacts appear as named_set_of_files before their ID is referenced
    // by a reporting event.
    let f = Fixture::new();
    let start_event = generic(
        test_id("Initial"),
        vec![ProgressEvent::initial_progress_update()],
    );

    // Prepare a dense NestedSet DAG with lots of shared references.
    let base_sets: Vec<NestedSet<Artifact>> = ["path/a", "path/b", "path/c", "path/d"]
        .iter()
        .map(|p| NestedSetBuilder::create(Order::STABLE_ORDER, [make_artifact(p)]))
        .collect();
    let next_depth = |sets: &[NestedSet<Artifact>]| -> Vec<NestedSet<Artifact>> {
        (0..sets.len())
            .map(|i| {
                NestedSetBuilder::<Artifact>::stable_order()
                    .add_transitive(&sets[i])
                    .add_transitive(&sets[(i + 1) % sets.len()])
                    .build()
            })
            .collect()
    };
    let depth2_sets = next_depth(&base_sets);
    let depth3_sets = next_depth(&depth2_sets);
    let depth4_sets = next_depth(&depth3_sets);
    let num_events = 20;
    let events_to_post: Vec<Arc<dyn BuildEvent>> = (0..num_events)
        .map(|i| {
            GenericArtifactReportingEvent::new(
                test_id(&format!("reporting{i}")),
                vec![depth4_sets[i % depth4_sets.len()].clone()],
            )
        })
        .collect();

    f.streamer.build_event(start_event.clone());
    // Publish `numEvents` different events that all report the same NamedSet of artifacts on
    // `numEvents` different threads. Use a barrier and a scope to ensure:
    //
    // 1. all threads have started, before:
    // 2. all threads send their event, before:
    // 3. verifying the recorded events.
    let ready_to_publish_latch = Barrier::new(num_events);
    std::thread::scope(|s| {
        for reporting_artifacts in &events_to_post {
            let ready_to_publish_latch = &ready_to_publish_latch;
            let streamer = &f.streamer;
            s.spawn(move || {
                ready_to_publish_latch.wait();
                streamer.build_event(reporting_artifacts.clone());
            });
        }
    });

    assert!(!f.streamer.is_closed());
    let all_events_seen = f.transport.get_events();
    let event_protos = f.transport.get_event_protos();
    // Each GenericArtifactReportingEvent and NamedArtifactGroup event has a corresponding
    // Progress event posted immediately before.
    assert_eq!(
        all_events_seen.len(),
        1 + ((num_events + base_sets.len() + depth2_sets.len() + depth3_sets.len()) * 2)
    );
    assert_eq!(
        all_events_seen[0].get_event_id(),
        start_event.get_event_id()
    );
    // Verify that each named_set_of_files event is sent before all of the events that report
    // that named_set.
    let mut seen_file_sets: HashSet<String> = HashSet::new();
    for build_event in event_protos.iter().skip(1) {
        match build_event.id.as_ref().and_then(|id| id.id.as_ref()) {
            Some(Id::NamedSet(named_set)) => {
                // These are the separately-posted contents of reported artifacts.
                seen_file_sets.insert(named_set.id.clone());
                for nested_set_id in named_set_of_files(build_event).file_sets {
                    assert!(seen_file_sets.contains(&nested_set_id.id));
                }
            }
            Some(Id::Unknown(_)) => {
                // These are the GenericArtifactReportingEvent that report artifacts.
                for nested_set_id in named_set_of_files(build_event).file_sets {
                    assert!(seen_file_sets.contains(&nested_set_id.id));
                }
            }
            _ => {}
        }
    }
}

#[test]
fn test_stdout_reported() {
    // Verify that stdout and stderr are reported in the build-event stream on progress
    // events.
    let f = Fixture::new();
    let stdout_msg = "Some text that was written to stdout.";
    let stderr_msg = "The UI text that bazel wrote to stderr.";
    let out_err = MockOutErr::new(vec![vec![stdout_msg]], vec![vec![stderr_msg]]);
    let start_event = generic(
        test_id("Initial"),
        vec![ProgressEvent::initial_progress_update()],
    );
    let unexpected_event = generic(test_id("unexpected"), Vec::new());

    f.streamer.register_out_err_provider(out_err.clone());
    f.streamer.build_event(start_event.clone());
    f.streamer.build_event(unexpected_event.clone());

    assert!(!f.streamer.is_closed());
    let events_seen = f.transport.get_events();
    assert_eq!(events_seen.len(), 3);
    assert_eq!(events_seen[0].get_event_id(), start_event.get_event_id());
    assert_eq!(
        events_seen[2].get_event_id(),
        unexpected_event.get_event_id()
    );
    let link_event = &events_seen[1];
    let link_event_proto = &f.transport.get_event_protos()[1];
    assert_eq!(
        link_event.get_event_id(),
        ProgressEvent::initial_progress_update()
    );
    assert!(
        link_event
            .get_children_events()
            .contains(&unexpected_event.get_event_id()),
        "Unexpected events should be linked"
    );
    assert_eq!(progress(link_event_proto).stdout, stdout_msg);
    assert_eq!(progress(link_event_proto).stderr, stderr_msg);

    // As there is only one progress event, the OutErrProvider should be queried
    // only once for stdout and stderr.
    assert_eq!(out_err.out_calls(), 1);
    assert_eq!(out_err.err_calls(), 1);
}

#[test]
fn test_stdout_reported_after_crash() {
    // Verify that stdout and stderr are reported in the build-event stream on progress
    // events.
    let f = Fixture::new();
    let stdout_msg = "Some text that was written to stdout.";
    let stderr_msg = "The UI text that bazel wrote to stderr.";
    let out_err = MockOutErr::new(vec![vec![stdout_msg]], vec![vec![stderr_msg]]);
    let start_event = generic(
        test_id("Initial"),
        vec![ProgressEvent::initial_progress_update()],
    );

    f.streamer.register_out_err_provider(out_err.clone());
    f.streamer.build_event(start_event.clone());
    // Simulate a crash with an abrupt call to #closeOnAbort().
    f.streamer.close_on_abort(AbortReason::Internal);
    assert!(f.streamer.is_closed());

    let events_seen = f.transport.get_events();
    assert_eq!(events_seen.len(), 2);
    assert_eq!(events_seen[0].get_event_id(), start_event.get_event_id());
    let link_event = &events_seen[1];
    let link_event_proto = &f.transport.get_event_protos()[1];
    assert_eq!(
        link_event.get_event_id(),
        ProgressEvent::initial_progress_update()
    );
    assert_eq!(progress(link_event_proto).stdout, stdout_msg);
    assert_eq!(progress(link_event_proto).stderr, stderr_msg);

    // As there is only one progress event, the OutErrProvider should be queried
    // only once for stdout and stderr.
    assert_eq!(out_err.out_calls(), 1);
    assert_eq!(out_err.err_calls(), 1);
}

type Pairs<T> = Vec<(Option<T>, Option<T>)>;

fn consume_to_lists<T: Clone>(left: &[T], right: &[T]) -> (Pairs<T>, Pairs<T>) {
    let mut consumer_builder: Pairs<T> = Vec::new();
    let mut last_consumer_builder: Pairs<T> = Vec::new();

    consume_as_pairs(
        left,
        right,
        &mut |t1: Option<T>, t2: Option<T>| consumer_builder.push((t1, t2)),
        &mut |t1: Option<T>, t2: Option<T>| last_consumer_builder.push((t1, t2)),
    );

    (consumer_builder, last_consumer_builder)
}

#[test]
fn test_consume_as_pairs() {
    assert_eq!(
        consume_to_lists(&[1, 2, 3], &[4, 5, 6]),
        (
            vec![
                (Some(1), None),
                (Some(2), None),
                (Some(3), Some(4)),
                (None, Some(5))
            ],
            vec![(None, Some(6))]
        )
    );

    assert_eq!(
        consume_to_lists::<i32>(&[], &[]),
        (vec![], vec![(None, None)])
    );

    assert_eq!(
        consume_to_lists(&[1], &[2]),
        (vec![], vec![(Some(1), Some(2))])
    );

    assert_eq!(
        consume_to_lists(&[1], &[2, 3]),
        (vec![(Some(1), Some(2))], vec![(None, Some(3))])
    );

    assert_eq!(
        consume_to_lists(&[1, 2], &[]),
        (vec![(Some(1), None)], vec![(Some(2), None)])
    );

    assert_eq!(consume_to_lists(&[], &[1]), (vec![], vec![(None, Some(1))]));
}

#[test]
fn test_reported_configurations() {
    // Verify that configuration events are posted, but only once.
    let f = Fixture::new();
    let start_event = generic(
        test_id("Initial"),
        vec![ProgressEvent::initial_progress_update()],
    );
    let configuration = make_testing_build_configuration_event();
    let first_with_configuration =
        GenericConfigurationEvent::new(test_id("first"), configuration.clone());
    let second_with_configuration =
        GenericConfigurationEvent::new(test_id("second"), configuration.clone());

    f.streamer.build_event(start_event.clone());
    f.streamer.build_event(first_with_configuration.clone());
    f.streamer.build_event(second_with_configuration.clone());

    assert!(!f.streamer.is_closed());
    let all_events_seen = f.transport.get_events();
    assert_eq!(all_events_seen.len(), 7);
    assert_eq!(
        all_events_seen[0].get_event_id(),
        start_event.get_event_id()
    );
    assert_eq!(
        all_events_seen[1].get_event_id(),
        ProgressEvent::initial_progress_update()
    );
    assert_equal_configuration_events(&f.transport, &all_events_seen[2], &configuration);
    assert_eq!(
        all_events_seen[3].get_event_id(),
        build_event_id_util::progress_id(1)
    );
    assert!(same(&all_events_seen[4], &first_with_configuration));
    assert_eq!(
        all_events_seen[5].get_event_id(),
        build_event_id_util::progress_id(2)
    );
    assert!(same(&all_events_seen[6], &second_with_configuration));
}

#[test]
fn test_reported_configurations_concurrent() {
    // Verify that configuration events are posted, but only once.
    let f = Fixture::new();
    let start_event = generic(
        test_id("Initial"),
        vec![ProgressEvent::initial_progress_update()],
    );
    let configuration = make_testing_build_configuration_event();

    let num_events = 100;
    let events_to_post: Vec<Arc<dyn BuildEvent>> = (0..num_events)
        .map(|i| {
            GenericConfigurationEvent::new(
                test_id(&format!("has_config_{i}")),
                configuration.clone(),
            )
        })
        .collect();

    f.streamer.build_event(start_event.clone());
    // Publish `numEvents` different events that all report the same configuration on
    // `numEvents` different threads. Use a barrier and a scope to ensure:
    //
    // 1. all threads have started, before:
    // 2. all threads send their event, before:
    // 3. verifying the recorded events.
    let ready_to_publish_latch = Barrier::new(num_events);
    std::thread::scope(|s| {
        for has_config_event in &events_to_post {
            let ready_to_publish_latch = &ready_to_publish_latch;
            let streamer = &f.streamer;
            s.spawn(move || {
                ready_to_publish_latch.wait();
                streamer.build_event(has_config_event.clone());
            });
        }
    });

    assert!(!f.streamer.is_closed());

    let all_events_seen = f.transport.get_events();

    // Two events for each GenericConfigurationEvent: a progress event announcing it and the
    // actual GenericConfigurationEvent itself.
    assert_eq!(all_events_seen.len(), 3 + (num_events * 2));
    assert_eq!(
        all_events_seen[0].get_event_id(),
        start_event.get_event_id()
    );
    assert_eq!(
        all_events_seen[1].get_event_id(),
        ProgressEvent::initial_progress_update()
    );
    assert_equal_configuration_events(&f.transport, &all_events_seen[2], &configuration);
    for event in &all_events_seen[3..] {
        assert!(!matches!(
            event.get_event_id().id,
            Some(Id::Configuration(_))
        ));
    }
}

#[test]
fn test_early_flush() {
    // Verify that the streamer can handle early calls to flush() and still correctly
    // reports stdout and stderr in the build-event stream.
    let f = Fixture::new();
    let first_stdout_msg = "Some text that was written to stdout.";
    let first_stderr_msg = "The UI text that bazel wrote to stderr.";
    let second_stdout_msg = "More text that was written to stdout, still before the start event.";
    let second_stderr_msg = "More text written to stderr, still before the start event.";
    let out_err = MockOutErr::new(
        vec![vec![first_stdout_msg], vec![second_stdout_msg]],
        vec![vec![first_stderr_msg], vec![second_stderr_msg]],
    );
    let start_event = generic(
        test_id("Initial"),
        vec![ProgressEvent::initial_progress_update()],
    );

    f.streamer.register_out_err_provider(out_err.clone());
    f.streamer.flush();
    f.streamer.flush();
    f.streamer.build_event(start_event.clone());

    assert!(!f.streamer.is_closed());
    let events_seen = f.transport.get_events();
    assert_eq!(events_seen.len(), 3);
    assert_eq!(events_seen[0].get_event_id(), start_event.get_event_id());
    let progress_event = &events_seen[1];
    assert_eq!(
        progress_event.get_event_id(),
        ProgressEvent::initial_progress_update()
    );
    let protos = f.transport.get_event_protos();
    let progress_event_proto = &protos[1];
    assert_eq!(progress(progress_event_proto).stdout, first_stdout_msg);
    assert_eq!(progress(progress_event_proto).stderr, first_stderr_msg);
    let second_progress_event_proto = &protos[2];
    assert_eq!(
        progress(second_progress_event_proto).stdout,
        second_stdout_msg
    );
    assert_eq!(
        progress(second_progress_event_proto).stderr,
        second_stderr_msg
    );

    // As there is only one progress event, the OutErrProvider should be queried
    // only once per flush() for stdout and stderr.
    assert_eq!(out_err.out_calls(), 2);
    assert_eq!(out_err.err_calls(), 2);
}

#[test]
fn test_chunked_flush() {
    // Verify that the streamer calls to flush() that return multiple chunked buffers.
    let f = Fixture::new();
    let first_stdout_msg = "Some text that was written to stdout.";
    let first_stderr_msg = "The UI text that bazel wrote to stderr.";
    let second_stdout_msg = "More text that was written to stdout, still before the start event.";
    let second_stderr_msg = "More text written to stderr, still before the start event.";
    let out_err = MockOutErr::new(
        vec![vec![first_stdout_msg, second_stdout_msg]],
        vec![vec![first_stderr_msg, second_stderr_msg]],
    );
    let start_event = generic(
        test_id("Initial"),
        vec![ProgressEvent::initial_progress_update()],
    );

    f.streamer.register_out_err_provider(out_err.clone());
    f.streamer.build_event(start_event.clone());
    f.streamer.flush();

    assert!(!f.streamer.is_closed());
    let events_seen = f.transport.get_events();
    assert_eq!(events_seen.len(), 4);
    assert_eq!(events_seen[0].get_event_id(), start_event.get_event_id());

    // Expect to find 3 progress messages: (firstStdout, ""), (secondStdout, firstStderr),
    // ("", secondStdErr). Assuming UIs display stdout first, this maintains ordering.
    let progress_event = &events_seen[1];
    assert_eq!(
        progress_event.get_event_id(),
        ProgressEvent::initial_progress_update()
    );
    let protos = f.transport.get_event_protos();
    let progress_event_proto = &protos[1];
    assert_eq!(progress(progress_event_proto).stdout, first_stdout_msg);
    assert!(progress(progress_event_proto).stderr.is_empty());

    let second_progress_event_proto = &protos[2];
    assert_eq!(
        progress(second_progress_event_proto).stdout,
        second_stdout_msg
    );
    assert_eq!(
        progress(second_progress_event_proto).stderr,
        first_stderr_msg
    );

    let third_progress_event_proto = &protos[3];
    assert!(progress(third_progress_event_proto).stdout.is_empty());
    assert_eq!(
        progress(third_progress_event_proto).stderr,
        second_stderr_msg
    );

    // The OutErrProvider should be queried only once per flush().
    assert_eq!(out_err.out_calls(), 1);
    assert_eq!(out_err.err_calls(), 1);
}

#[test]
#[ignore = "needs SynchronizedOutputStream (runtime/SynchronizedOutputStream.java, not ported) \
            and @TestParameter maxBufferedLength/maxChunkSize in {5, 30, 10000}"]
fn test_flush_preserves_stdout_stderr_order() {
    // Java: for each (maxBufferedLength, maxChunkSize), stdout/stderr SynchronizedOutputStreams
    // registered with the streamer receive the interleaved genrule output below; after
    // streamer.close(), concatenating (stderr, stdout) of every progress event reproduces it:
    //
    // [0 / 3] 3 actions running
    // INFO: From Executing genrule //:1:
    // Hello from genrule //:1 on stdout
    // Hello from genrule //:1 on stderr
    // [1 / 3] 2 actions running
    // INFO: From Executing genrule //:2:
    // Hello from genrule //:2 on stderr
    // Hello from genrule //:2 on stdout
    // [2 / 3] 1 actions running
    // INFO: From Executing genrule //:3:
    // Hello from genrule //:3 on stdout
    // Hello from genrule //:3 on stderr
    // Hello again from genrule //:3 on stdout
    // INFO: Build completed successfully, 3 total actions
}

#[test]
fn test_noop_flush() {
    // Verify that the streamer ignores a flush, if neither stream produces any output.
    let f = Fixture::new();
    let stdout_msg = "Some text that was written to stdout.";
    let stderr_msg = "The UI text that bazel wrote to stderr.";
    let out_err = MockOutErr::new(
        vec![vec![stdout_msg], vec![]],
        vec![vec![stderr_msg], vec![]],
    );
    let start_event = generic(
        test_id("Initial"),
        vec![ProgressEvent::initial_progress_update()],
    );

    f.streamer.register_out_err_provider(out_err);
    f.streamer.build_event(start_event.clone());
    assert_eq!(f.transport.get_events().len(), 1);
    f.streamer.flush(); // Output, so a new progress event has to be added
    assert_eq!(f.transport.get_events().len(), 2);
    f.streamer.flush(); // No further output, so no additional event should be generated.
    assert_eq!(f.transport.get_events().len(), 2);

    assert!(same(&f.transport.get_events()[0], &start_event));
    let protos = f.transport.get_event_protos();
    assert_eq!(progress(&protos[1]).stdout, stdout_msg);
    assert_eq!(progress(&protos[1]).stderr, stderr_msg);
}

#[test]
fn test_early_flush_bad_initial_event() {
    // Verify that an early flush works correctly with an unusual start event.
    // In this case, we expect 3 events in the stream, in that order:
    // - an artificial progress event as initial event, to properly link in
    //   all events
    // - the unusual first event we have seen, and
    // - a progress event reporting the flushed messages.
    let f = Fixture::new();
    let stdout_msg = "Some text that was written to stdout.";
    let stderr_msg = "The UI text that bazel wrote to stderr.";
    let out_err = MockOutErr::new(vec![vec![stdout_msg]], vec![vec![stderr_msg]]);

    let unexpected_start_event = generic(test_id("unexpected start"), Vec::new());

    f.streamer.register_out_err_provider(out_err.clone());
    f.streamer.flush();
    f.streamer.build_event(unexpected_start_event.clone());

    assert!(!f.streamer.is_closed());

    let events_seen = f.transport.get_events();
    assert_eq!(events_seen.len(), 3);

    let initial = &events_seen[0];
    assert_eq!(
        initial.get_event_id(),
        ProgressEvent::initial_progress_update()
    );
    let protos = f.transport.get_event_protos();
    let initial_proto = &protos[0];
    assert!(progress(initial_proto).stdout.is_empty());
    assert!(progress(initial_proto).stderr.is_empty());

    assert_eq!(
        events_seen[1].get_event_id(),
        unexpected_start_event.get_event_id()
    );
    assert!(
        initial
            .get_children_events()
            .contains(&unexpected_start_event.get_event_id()),
        "Unexpected event should be linked"
    );

    let progress_proto = &protos[2];
    assert_eq!(progress(progress_proto).stdout, stdout_msg);
    assert_eq!(progress(progress_proto).stderr, stderr_msg);
    assert!(
        initial
            .get_children_events()
            .contains(&events_seen[2].get_event_id()),
        "flushed progress should be linked"
    );

    assert_eq!(out_err.out_calls(), 1);
    assert_eq!(out_err.err_calls(), 1);
}

#[test]
fn test_early_abort() {
    // For a build that is aborted before a build-started event is generated,
    // we still expect that, if a build-started event is forced by some order
    // constraint (e.g., CommandLine wants to come after build started), then
    // that gets sorted to the beginning.
    let f = Fixture::new();
    let order_event = GenericOrderEvent::new(
        test_id("event depending on start"),
        Vec::new(),
        vec![build_event_id_util::build_started_id()],
    );

    f.streamer.build_event(order_event.clone());
    f.streamer.build_event(new_build_complete_event(Vec::new()));

    assert!(f.streamer.is_closed());
    let events_seen = f.transport.get_events();
    assert_eq!(events_seen.len(), 4);
    assert_eq!(
        events_seen[0].get_event_id(),
        build_event_id_util::build_started_id()
    );
    assert_eq!(events_seen[1].get_event_id(), order_event.get_event_id());
    assert_eq!(
        HashSet::from([events_seen[2].get_event_id(), events_seen[3].get_event_id()]),
        HashSet::from([
            build_event_id_util::build_finished(),
            ProgressEvent::initial_progress_update()
        ])
    );
    assert!(f.transport.get_event_protos()[3].last_message);
}

#[test]
fn test_event_after_build_complete_event() {
    let f = Fixture::new();
    let late_id = test_id("late");
    let start_event = generic(
        test_id("initial"),
        vec![
            ProgressEvent::initial_progress_update(),
            build_event_id_util::build_finished(),
        ],
    );
    let late_event = generic(late_id.clone(), vec![test_id("nonexistent")]);
    let finished_event = new_build_complete_event(vec![late_id]);

    f.streamer.build_event(start_event);
    f.streamer.build_event(finished_event);
    f.streamer.build_event(late_event);
    assert!(f.streamer.is_closed());
    let protos = f.transport.get_event_protos();
    assert_eq!(protos.len(), 4);
    assert!(protos[3].last_message);
}

#[test]
fn test_final_events_late() {
    // Verify that we correctly handle late events (i.e., events coming only after the
    // BuildCompleteEvent) that are sent to the streamer after the BuildCompleteEvent.
    let f = Fixture::new();
    let start_event = generic(
        test_id("Initial"),
        vec![
            ProgressEvent::initial_progress_update(),
            build_event_id_util::build_finished(),
        ],
    );
    let late_id = test_id("late event");
    let finished_event = new_build_complete_event(vec![late_id.clone()]);

    f.streamer.build_event(start_event.clone());
    f.streamer.build_event(finished_event);
    assert!(!f.streamer.is_closed());
    f.streamer.build_event(generic(late_id.clone(), Vec::new()));
    assert!(f.streamer.is_closed());

    let events_seen = f.transport.get_events();
    assert_eq!(events_seen.len(), 4);
    assert_eq!(events_seen[0].get_event_id(), start_event.get_event_id());
    assert_eq!(
        events_seen[1].get_event_id(),
        build_event_id_util::build_finished()
    );
    assert_eq!(
        HashSet::from([events_seen[2].get_event_id(), events_seen[3].get_event_id()]),
        HashSet::from([late_id, ProgressEvent::initial_progress_update()])
    );
}

#[test]
fn test_final_events_early() {
    // Verify that we correctly handle late events (i.e., events coming only after the
    // BuildCompleteEvent) that are sent to the streamer before the BuildCompleteEvent,
    // but with an order constraint to come afterwards.
    let f = Fixture::new();
    let start_event = generic(
        test_id("Initial"),
        vec![
            ProgressEvent::initial_progress_update(),
            build_event_id_util::build_finished(),
        ],
    );
    let late_id = test_id("late event");
    let finished_event = new_build_complete_event(vec![late_id.clone()]);

    f.streamer.build_event(start_event.clone());
    f.streamer.build_event(GenericOrderEvent::new(
        late_id.clone(),
        Vec::new(),
        vec![build_event_id_util::build_finished()],
    ));
    f.streamer.build_event(finished_event);
    assert!(f.streamer.is_closed());

    let events_seen = f.transport.get_events();
    assert_eq!(events_seen.len(), 4);
    assert_eq!(events_seen[0].get_event_id(), start_event.get_event_id());
    assert_eq!(
        events_seen[1].get_event_id(),
        build_event_id_util::build_finished()
    );
    assert_eq!(
        HashSet::from([events_seen[2].get_event_id(), events_seen[3].get_event_id()]),
        HashSet::from([late_id, ProgressEvent::initial_progress_update()])
    );
}

#[test]
fn test_progress_after_final_events() {
    let f = Fixture::new();
    let early_stdout = "Stdout before finishing.";
    let early_stderr = "Stderr before finishing.";
    let middle_stdout = "Stdout *while* finishing.";
    let middle_stderr = "Stderr *while* finishing.";
    let late_stdout = "Stdout after finishing.";
    let late_stderr = "Stderr after finishing.";
    let out_err = MockOutErr::new(
        vec![vec![early_stdout], vec![middle_stdout], vec![late_stdout]],
        vec![vec![early_stderr], vec![middle_stderr], vec![late_stderr]],
    );
    f.streamer.register_out_err_provider(out_err);

    // Verify that we correctly handle progress events that are sent to the streamer after the
    // BuildCompleteEvent.
    let start_event = generic(
        test_id("Initial"),
        vec![
            ProgressEvent::initial_progress_update(),
            build_event_id_util::build_finished(),
        ],
    );
    let late_id = test_id("late event");
    let finished_event = build_complete_event_with(
        DetailedExitCode::success(),
        /* stop_on_failure= */ false,
        /* crash= */ false,
        /* catastrophe= */ false,
        vec![late_id.clone()],
    );

    f.streamer.build_event(start_event.clone());
    f.streamer.flush();
    f.streamer.build_event(finished_event);
    // Flushing after the finished event should discard stdout/stderr, not post progress events.
    f.streamer.flush();
    assert!(!f.streamer.is_closed());
    f.streamer.build_event(generic(late_id.clone(), Vec::new()));
    assert!(f.streamer.is_closed());

    let events_seen = f.transport.get_events();
    let formatter = f.transport.context();
    // Verify that the event IDs are as expected.
    assert_eq!(events_seen.len(), 5);
    assert_eq!(events_seen[0].get_event_id(), start_event.get_event_id());
    assert_eq!(
        events_seen[1].get_event_id(),
        ProgressEvent::initial_progress_update()
    );
    assert_eq!(
        events_seen[2].get_event_id(),
        build_event_id_util::build_finished()
    );
    assert_eq!(
        events_seen[3].get_event_id(),
        ProgressEvent::progress_update_empty(1).get_event_id()
    );
    // Progress events received after the build is finished do not have an incremented progress
    // ID.
    assert_eq!(events_seen[4].get_event_id(), late_id);

    // Verify that the progress events have the correct stdout/stderr and that the last event has
    // the "last_message" bit set true.
    let early_progress_proto = progress(&events_seen[1].as_stream_proto(&formatter).unwrap());
    assert_eq!(early_progress_proto.stdout, early_stdout);
    assert_eq!(early_progress_proto.stderr, early_stderr);
    let middle_progress_proto = progress(&events_seen[3].as_stream_proto(&formatter).unwrap());
    assert_eq!(middle_progress_proto.stdout, middle_stdout);
    assert_eq!(middle_progress_proto.stderr, middle_stderr);
    assert!(
        events_seen[4]
            .as_stream_proto(&formatter)
            .unwrap()
            .last_message
    );
}

#[test]
fn test_lots_of_progress_after_final_events() {
    let f = Fixture::new();
    let early_stdout = "Stdout before finishing.";
    let early_stderr = "Stderr before finishing.";
    let middle_stdout = "Stdout *while* finishing.";
    let middle_stderr = "Stderr *while* finishing.";
    let late_stdout = "DoneStdout";
    let late_stderr = "DoneStderr";
    let late_stdouts: Vec<String> = (1..=3).map(|i| format!("{late_stdout}{i}")).collect();
    let late_stderrs: Vec<String> = (1..=3).map(|i| format!("{late_stderr}{i}")).collect();
    let out_err = MockOutErr::new(
        vec![
            vec![early_stdout],
            vec![middle_stdout],
            late_stdouts.iter().map(String::as_str).collect(),
        ],
        vec![
            vec![early_stderr],
            vec![middle_stderr],
            late_stderrs.iter().map(String::as_str).collect(),
        ],
    );
    f.streamer.register_out_err_provider(out_err);

    // Verify that we correctly handle progress events that are sent to the streamer after the
    // BuildCompleteEvent.
    let start_event = generic(
        test_id("Initial"),
        vec![
            ProgressEvent::initial_progress_update(),
            build_event_id_util::build_finished(),
        ],
    );
    let late_id = test_id("late event");
    let finished_event = build_complete_event_with(
        DetailedExitCode::success(),
        /* stop_on_failure= */ false,
        /* crash= */ false,
        /* catastrophe= */ false,
        vec![late_id.clone()],
    );

    f.streamer.build_event(start_event);
    f.streamer.flush();
    f.streamer.build_event(finished_event);
    f.streamer.flush();
    assert!(!f.streamer.is_closed());
    f.streamer.build_event(generic(late_id, Vec::new()));
    assert!(f.streamer.is_closed());

    let events_seen = f.transport.get_events();
    let formatter = f.transport.context();
    // Verify that the last event has the "last_message" bit set true.
    assert!(
        events_seen
            .last()
            .unwrap()
            .as_stream_proto(&formatter)
            .unwrap()
            .last_message
    );
}

#[test]
fn test_successful_actions_are_not_published_by_default() {
    let f = Fixture::new();
    let successful_action_executed_event = successful_action_executed_event();
    let failed_action_executed_event = failed_action_executed_event();

    f.streamer
        .build_event(successful_action_executed_event.clone());
    f.streamer.build_event(failed_action_executed_event.clone());

    let transported_events = f.transport.get_events();

    assert!(!contains(
        &transported_events,
        &successful_action_executed_event
    ));
    assert!(contains(&transported_events, &failed_action_executed_event));
}

#[test]
fn test_successful_actions_can_be_published() {
    let f = Fixture::new();

    let mut options = new_build_event_stream_options();
    options.publish_all_actions = true;

    let streamer = Builder::new()
        .artifact_group_namer(f.artifact_group_namer.clone())
        .bes_stream_options(options)
        .build_event_transports(vec![f.transport.clone() as Arc<dyn BuildEventTransport>])
        .build();

    let successful_action_executed_event = successful_action_executed_event();
    let failed_action_executed_event = failed_action_executed_event();

    streamer.build_event(successful_action_executed_event.clone());
    streamer.build_event(failed_action_executed_event.clone());

    let transported_events = f.transport.get_events();

    assert!(contains(
        &transported_events,
        &successful_action_executed_event
    ));
    assert!(contains(&transported_events, &failed_action_executed_event));
}

/// The start event of the abort tests: build started, announcing `abort_expected`.
fn abort_test_start_event(build_event_id: &BuildEventId) -> Arc<dyn BuildEvent> {
    generic(
        build_event_id_util::build_started_id(),
        vec![
            build_event_id.clone(),
            ProgressEvent::initial_progress_update(),
            build_event_id_util::build_finished(),
        ],
    )
}

#[test]
fn test_build_incomplete() {
    let f = Fixture::new();
    let build_event_id = test_id("abort_expected");
    let start_event = abort_test_start_event(&build_event_id);
    let build_complete_event = build_complete_event_with(
        create_generic_detailed_exit_code(),
        true,
        false,
        false,
        Vec::new(),
    );

    f.streamer.build_event(start_event);
    f.streamer.build_event(build_complete_event);
    f.streamer.close();

    let aborted_event = f.get_bep_event(&build_event_id).expect("aborted event");
    assert!(has_aborted(&aborted_event));
    assert_eq!(
        aborted(&aborted_event).reason,
        AbortReason::Incomplete as i32
    );
    assert!(aborted(&aborted_event).description.is_empty());
}

#[test]
fn test_build_crash() {
    let f = Fixture::new();
    let build_event_id = test_id("abort_expected");
    let start_event = abort_test_start_event(&build_event_id);
    let build_complete_event = build_complete_event_with(
        create_generic_detailed_exit_code(),
        true,
        /* crash= new RuntimeException() */ true,
        false,
        Vec::new(),
    );

    f.streamer.build_event(start_event);
    f.streamer.build_event(build_complete_event);
    f.streamer.close();

    let aborted_event = f.get_bep_event(&build_event_id).expect("aborted event");
    assert!(has_aborted(&aborted_event));
    assert_eq!(aborted(&aborted_event).reason, AbortReason::Internal as i32);
    assert!(aborted(&aborted_event).description.is_empty());
}

#[test]
fn test_build_catastrophe() {
    let f = Fixture::new();
    let build_event_id = test_id("abort_expected");
    let start_event = abort_test_start_event(&build_event_id);
    let build_complete_event = build_complete_event_with(
        create_generic_detailed_exit_code(),
        true,
        false,
        true,
        Vec::new(),
    );

    f.streamer.build_event(start_event);
    f.streamer.build_event(build_complete_event);
    f.streamer.close();

    let aborted_event = f.get_bep_event(&build_event_id).expect("aborted event");
    assert!(has_aborted(&aborted_event));
    assert_eq!(aborted(&aborted_event).reason, AbortReason::Internal as i32);
    assert!(aborted(&aborted_event).description.is_empty());
}

#[test]
fn test_build_catastrophe_oom_test_command() {
    let f = Fixture::new();
    let aborted_event_id = build_event_id_util::target_pattern_expanded(&["//foo:bar".to_string()]);
    // Java: BuildStartingEvent.create("tmpfs", true, request, null, "/tmp/build") for a "test"
    // request with targets ["//foo:bar"], a random id and start time 10842.
    // PORT: the request's id is random in Java; a fixed UUID is used here.
    let start_event: Arc<dyn BuildEvent> =
        Arc::new(BuildStartingEvent::create(BuildStartingData {
            uuid: "00000000-0000-0000-0000-000000000000".to_string(),
            start_time_millis: 10842,
            command_name: "test".to_string(),
            pwd: "/tmp/build".to_string(),
            workspace: None,
            targets: vec!["//foo:bar".to_string()],
            ..Default::default()
        }));
    let build_complete_event = build_complete_event_with(
        DetailedExitCode::of(FailureDetail {
            category: Some(Category::Crash(Crash {
                code: crash::Code::CrashOom as i32,
                ..Default::default()
            })),
            ..Default::default()
        }),
        true,
        false,
        true,
        Vec::new(),
    );

    f.streamer.build_event(start_event);
    f.streamer.build_event(build_complete_event);
    f.streamer.close();

    let aborted_event = f.get_bep_event(&aborted_event_id).expect("aborted event");
    assert!(has_aborted(&aborted_event));
    assert_eq!(
        aborted(&aborted_event).reason,
        AbortReason::OutOfMemory as i32
    );
    assert_eq!(
        aborted(&aborted_event).description,
        construct_oom_exit_message(OOM_MESSAGE)
    );
}

#[test]
fn test_stream_aborted_with_timeout() {
    let f = Fixture::new();
    let build_event_id = test_id("abort_expected");
    let start_event = abort_test_start_event(&build_event_id);

    f.streamer.build_event(start_event);
    f.streamer.close_on_abort(AbortReason::TimeOut);

    let aborted0 = f.get_bep_event(&build_event_id).expect("aborted event");
    assert!(has_aborted(&aborted0));
    assert_eq!(aborted(&aborted0).reason, AbortReason::TimeOut as i32);
    assert!(aborted(&aborted0).description.is_empty());

    let aborted1 = f
        .get_bep_event(&build_event_id_util::build_finished())
        .expect("aborted event");
    assert!(has_aborted(&aborted1));
    assert_eq!(aborted(&aborted1).reason, AbortReason::TimeOut as i32);
    assert!(aborted(&aborted1).description.is_empty());
}

#[test]
fn test_build_failure_multiple_reasons() {
    let f = Fixture::new();
    let build_event_id = test_id("abort_expected");
    let start_event = abort_test_start_event(&build_event_id);
    let build_complete_event = build_complete_event_with(
        create_generic_detailed_exit_code(),
        false,
        /* crash= new RuntimeException() */ true,
        false,
        Vec::new(),
    );

    f.streamer.build_event(start_event);
    // Java: streamer.noAnalyze(new NoAnalyzeEvent())
    f.streamer.no_analyze();
    f.streamer.build_event(build_complete_event);
    f.streamer.close();

    let aborted_event = f.get_bep_event(&build_event_id).expect("aborted event");
    assert!(has_aborted(&aborted_event));
    assert_eq!(aborted(&aborted_event).reason, AbortReason::Internal as i32);
    assert_eq!(
        aborted(&aborted_event).description,
        "Multiple abort reasons reported: [NO_ANALYZE, INTERNAL]"
    );
}

#[test]
fn non_oom_abort_reason_does_not_include_oom_message() {
    let f = Fixture::new();
    let build_event_id = test_id("abort_expected");
    let start_event = abort_test_start_event(&build_event_id);

    f.streamer.build_event(start_event);
    f.streamer.close_on_abort(AbortReason::Internal);

    assert_eq!(
        aborted(&f.get_bep_event(&build_event_id).expect("aborted event")),
        bes::Aborted {
            reason: AbortReason::Internal as i32,
            ..Default::default()
        }
    );
}

#[test]
fn oom_abort_reason_includes_oom_message() {
    let f = Fixture::new();
    let build_event_id = test_id("abort_expected");
    let start_event = abort_test_start_event(&build_event_id);

    f.streamer.build_event(start_event);
    f.streamer.close_on_abort(AbortReason::OutOfMemory);

    assert_eq!(
        aborted(&f.get_bep_event(&build_event_id).expect("aborted event")),
        bes::Aborted {
            reason: AbortReason::OutOfMemory as i32,
            description: construct_oom_exit_message(OOM_MESSAGE),
        }
    );
}

#[test]
fn replaceable_event_does_not_post_becauseis_replaced() {
    let f = Fixture::new();
    let build_event_id = test_id("replaceable_event");
    let replaceable =
        ReplaceableTestBuildEvent::new(build_event_id.clone(), /* replaceable= */ true);
    let replaced_by = ReplaceableTestBuildEvent::new(build_event_id, /* replaceable= */ false);

    f.streamer.build_event(replaceable.clone());
    assert!(f.transport.get_events().is_empty());
    f.streamer.build_event(replaced_by.clone());
    assert!(!contains(&f.transport.get_events(), &replaceable));
    assert!(contains(&f.transport.get_events(), &replaced_by));
}

#[test]
fn replaceable_event_posts_becauseis_not_replaced_and_build_aborts() {
    let f = Fixture::new();
    let build_event_id = test_id("replaceable_event");
    let replaceable = ReplaceableTestBuildEvent::new(build_event_id, /* replaceable= */ true);

    f.streamer.build_event(replaceable.clone());
    assert!(f.transport.get_events().is_empty());
    // Java: streamer.noAnalyze(new NoAnalyzeEvent())
    f.streamer.no_analyze();
    f.streamer.close();
    assert!(contains(&f.transport.get_events(), &replaceable));
}

#[test]
fn replaceable_event_posts_becauseis_not_replaced_and_build_completes() {
    let f = Fixture::new();
    let build_event_id = test_id("replaceable_event");
    let replaceable = ReplaceableTestBuildEvent::new(build_event_id, /* replaceable= */ true);

    f.streamer.build_event(replaceable.clone());
    assert!(f.transport.get_events().is_empty());
    f.streamer.build_event(new_build_complete_event(Vec::new()));
    assert!(contains(&f.transport.get_events(), &replaceable));
}
