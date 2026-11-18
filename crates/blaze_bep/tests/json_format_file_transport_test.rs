//! Port of `buildeventstream/transports/JsonFormatFileTransportTest.java`.

#![allow(deprecated)]

use std::fs::File;
use std::io::{BufWriter, Write};
use std::sync::{Arc, Mutex};

use blaze_bep::buildeventstream::transports::JsonFormatFileTransport;
use blaze_bep::buildeventstream::{
    ArtifactGroupNamer, ArtifactGroupNode, BuildEvent, BuildEventContext, BuildEventTransport,
    Interrupted, LocalFilesArtifactUploader,
};
use blaze_optdefs::misc::build_event_protocol_options::BuildEventProtocolOptions;
use blaze_optdefs::misc::build_event_service_options::BesUploadMode;
use blaze_protos::JavaMessage;
use blaze_protos::build_event_stream::build_event::Payload;
use blaze_protos::build_event_stream::build_event_id::{ActionCompletedId, Id, NamedSetOfFilesId};
use blaze_protos::build_event_stream::{self as bes, BuildEventId};
use blaze_protos::google::protobuf::Any;
use blaze_protos::json::{JsonFormat, TypeRegistry};
use blaze_protos::tools::protos::SpawnExec;
use tempfile::TempDir;

/// `SPAWN_EXEC_TYPE_REGISTRY`.
fn spawn_exec_type_registry() -> TypeRegistry {
    TypeRegistry::new_builder().add::<SpawnExec>().build()
}

/// `Any.pack(message)`.
fn any_pack<M: JavaMessage>(message: &M) -> Any {
    Any {
        type_url: format!("type.googleapis.com/{}", M::FULL_NAME),
        value: message.to_byte_array(),
    }
}

/// The Mockito `@Mock BuildEvent buildEvent` with `asStreamProto(any())` stubbed.
#[derive(Debug, Default)]
struct MockBuildEvent {
    stream_proto: Mutex<bes::BuildEvent>,
}

impl MockBuildEvent {
    fn returning(proto: bes::BuildEvent) -> Arc<dyn BuildEvent> {
        Arc::new(MockBuildEvent {
            stream_proto: Mutex::new(proto),
        })
    }
}

impl BuildEvent for MockBuildEvent {
    fn get_event_id(&self) -> BuildEventId {
        // Unstubbed Mockito method: null.
        BuildEventId::default()
    }

    fn get_children_events(&self) -> Vec<BuildEventId> {
        Vec::new()
    }

    fn as_stream_proto(
        &self,
        _context: &dyn BuildEventContext,
    ) -> Result<bes::BuildEvent, Interrupted> {
        Ok(self.stream_proto.lock().unwrap().clone())
    }
}

/// The Mockito `@Mock ArtifactGroupNamer artifactGroupNamer` (unstubbed: returns null).
struct MockArtifactGroupNamer;

impl ArtifactGroupNamer for MockArtifactGroupNamer {
    fn apply(&self, _node: &ArtifactGroupNode) -> Option<NamedSetOfFilesId> {
        None
    }
}

/// The `@Before setUp()` state: a temporary output file and the transport writing to it.
struct Fixture {
    _tmp: TempDir,
    output: std::path::PathBuf,
}

impl Fixture {
    fn new() -> Fixture {
        let tmp = tempfile::tempdir().unwrap();
        let output = tmp.path().join("output");
        File::create(&output).unwrap();
        Fixture { _tmp: tmp, output }
    }

    fn output_stream(&self) -> Box<dyn Write + Send> {
        Box::new(BufWriter::new(File::create(&self.output).unwrap()))
    }

    fn under_test(&self, output_stream: Box<dyn Write + Send>) -> JsonFormatFileTransport {
        JsonFormatFileTransport::new(
            output_stream,
            Arc::new(BuildEventProtocolOptions::default()),
            Arc::new(LocalFilesArtifactUploader),
            Arc::new(MockArtifactGroupNamer),
            spawn_exec_type_registry(),
            BesUploadMode::WAIT_FOR_UPLOAD_COMPLETE,
        )
    }

    fn read_output(&self) -> String {
        std::fs::read_to_string(&self.output).unwrap()
    }
}

fn make_action_event_with_any_details(strategy_details: Any) -> bes::BuildEvent {
    bes::BuildEvent {
        payload: Some(Payload::Action(bes::ActionExecuted {
            exit_code: 1,
            strategy_details: vec![strategy_details],
            ..Default::default()
        })),
        ..Default::default()
    }
}

fn started_build_event() -> bes::BuildEvent {
    bes::BuildEvent {
        payload: Some(Payload::Started(bes::BuildStarted {
            command: "build".to_string(),
            ..Default::default()
        })),
        ..Default::default()
    }
}

#[test]
fn test_creates_file_and_writes_proto_json_format() {
    let fixture = Fixture::new();
    let under_test = fixture.under_test(fixture.output_stream());

    // Arrange: Prepare a simple BEP event that can be round-tripped.
    let started = started_build_event();
    let build_event = MockBuildEvent::returning(started.clone());

    // Act: Send the simple BuildStarted event.
    under_test.send_build_event(&build_event);
    under_test.close().unwrap();

    // Assert: Read back the BEP event and confirm it round-tripped.
    let parser = JsonFormat::parser();
    let mut builder = bes::BuildEvent::default();
    parser.merge(&fixture.read_output(), &mut builder).unwrap();
    assert_eq!(builder, started);
}

#[test]
fn expands_known_any_type() {
    let fixture = Fixture::new();
    let under_test = fixture.under_test(fixture.output_stream());

    // Arrange: Prepare an Any event that is recognized by the JSON formatter.
    let spawn_exec_any = any_pack(&SpawnExec {
        exit_code: 1,
        mnemonic: "Javac".to_string(),
        ..Default::default()
    });
    let action = make_action_event_with_any_details(spawn_exec_any);
    let build_event = MockBuildEvent::returning(action.clone());

    // Act: Write the event and close the transport to force flushing.
    under_test.send_build_event(&build_event);
    under_test.close().unwrap();

    // Assert: Confirm we get the SpawnExec back.
    let parser = JsonFormat::parser().using_type_registry(spawn_exec_type_registry());
    let mut builder = bes::BuildEvent::default();
    parser.merge(&fixture.read_output(), &mut builder).unwrap();
    assert_eq!(builder, action);
}

#[test]
fn rejects_unknown_any_type() {
    let fixture = Fixture::new();
    let under_test = fixture.under_test(fixture.output_stream());

    // Arrange: Prepare a BuildEvent that cannot be serialized due to an unrecognized Any type.
    let bogus_any_proto = any_pack(&BuildEventId {
        id: Some(Id::ActionCompleted(ActionCompletedId {
            label: "//:foo".to_string(),
            ..Default::default()
        })),
    });
    let action = make_action_event_with_any_details(bogus_any_proto);
    let build_event = MockBuildEvent::returning(action);

    // Act: Send the event with a bogus Any value.
    under_test.send_build_event(&build_event);
    under_test.close().unwrap();

    // Assert: Special invalid event message is written in JSON.
    let output = fixture.read_output();
    let json_line = output.lines().next().expect("one line");
    // Java: `new GsonBuilder().create().fromJson(jsonLine, UnknownAnyProtoError.class)` is not
    // null, i.e. the line is a JSON object.
    let error: serde_json::Value = serde_json::from_str(json_line).unwrap();
    assert!(error.is_object(), "{json_line}");
}

/// A thin wrapper around an output stream that counts number of bytes written and verifies
/// flushes.
struct WrappedOutputStream {
    out: Box<dyn Write + Send>,
    counts: Arc<Mutex<Counts>>,
}

#[derive(Default)]
struct Counts {
    byte_count: u64,
    flush_count: u32,
}

impl Write for WrappedOutputStream {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        let n = self.out.write(b)?;
        self.counts.lock().unwrap().byte_count += n as u64;
        Ok(n)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.out.flush()?;
        self.counts.lock().unwrap().flush_count += 1;
        Ok(())
    }
}

#[test]
fn test_flushes_stream_after_small_writes() {
    let fixture = Fixture::new();
    // Arrange: JsonFormatFileTransport writes to a wrapped output stream to verify flushing.
    let counts = Arc::new(Mutex::new(Counts::default()));
    let wrapped_output_stream = WrappedOutputStream {
        out: fixture.output_stream(),
        counts: counts.clone(),
    };
    let under_test = fixture.under_test(Box::new(wrapped_output_stream));

    let started = started_build_event();
    let build_event = MockBuildEvent::returning(started);

    // Act: Write an event, then wait for three flush intervals.
    under_test.send_build_event(&build_event);
    // PORT: the Rust file transport writes and flushes on the sending thread (see the PORT note
    // of `FileTransport`), so it has no `getFlushInterval()` to wait for.

    // Assert: Confirm BEP events were written even though the file transport is not closed.

    // Some users, e.g. Tulsi, use JSON build event output for interactive use and expect the
    // stream to be flushed at regular short intervals.
    {
        let counts = counts.lock().unwrap();
        assert!(counts.flush_count > 0);

        // We know that large writes get flushed; test is valuable only if we check small writes,
        // meaning smaller than 8192, the default buffer size used by BufferedOutputStream.
        assert!(counts.byte_count < 8192);
        assert!(counts.byte_count > 0);
    }

    under_test.close().unwrap();
}
