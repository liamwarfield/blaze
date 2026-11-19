//! Port of `buildeventstream/transports/TextFormatFileTransportTest.java`.

use std::fs::File;
use std::io::{BufWriter, Write};
use std::sync::{Arc, Mutex};

use blaze_bep::buildeventstream::transports::TextFormatFileTransport;
use blaze_bep::buildeventstream::{
    ArtifactGroupNamer, ArtifactGroupNode, BuildEvent, BuildEventContext, BuildEventTransport,
    Interrupted, LocalFilesArtifactUploader,
};
use blaze_optdefs::misc::build_event_protocol_options::BuildEventProtocolOptions;
use blaze_optdefs::misc::build_event_service_options::BesUploadMode;
use blaze_protos::build_event_stream::build_event::Payload;
use blaze_protos::build_event_stream::build_event_id::NamedSetOfFilesId;
use blaze_protos::build_event_stream::{self as bes, BuildEventId};
use blaze_protos::text_format;

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

/// `trimLines`: replace CRLF with LF and trim leading and trailing spaces.
fn trim_lines(text: &str) -> String {
    let text = text.replace('\r', "");
    // `replaceAll(" *\n *", "\n")`.
    let mut out = String::new();
    for (i, line) in text.split('\n').enumerate() {
        if i > 0 {
            out.push('\n');
        }
        let mut l = line;
        if i > 0 {
            l = l.trim_start_matches(' ');
        }
        if i + 1 < text.split('\n').count() {
            l = l.trim_end_matches(' ');
        }
        out.push_str(l);
    }
    out
}

#[test]
fn test_creates_file_and_writes_proto_text_format() {
    let tmp = tempfile::tempdir().unwrap();
    let output = tmp.path().join("output");
    File::create(&output).unwrap();
    let output_stream: Box<dyn Write + Send> =
        Box::new(BufWriter::new(File::create(&output).unwrap()));

    let started = bes::BuildEvent {
        payload: Some(Payload::Started(bes::BuildStarted {
            command: "build".to_string(),
            ..Default::default()
        })),
        ..Default::default()
    };
    let transport = TextFormatFileTransport::new(
        output_stream,
        Arc::new(BuildEventProtocolOptions::default()),
        Arc::new(LocalFilesArtifactUploader),
        Arc::new(MockArtifactGroupNamer),
        BesUploadMode::WAIT_FOR_UPLOAD_COMPLETE,
    );
    transport.send_build_event(&MockBuildEvent::returning(started.clone()));

    let progress = bes::BuildEvent {
        payload: Some(Payload::Progress(bes::Progress::default())),
        ..Default::default()
    };
    transport.send_build_event(&MockBuildEvent::returning(progress.clone()));

    let completed = bes::BuildEvent {
        payload: Some(Payload::Completed(bes::TargetComplete {
            success: true,
            ..Default::default()
        })),
        ..Default::default()
    };
    transport.send_build_event(&MockBuildEvent::returning(completed.clone()));

    transport.close().get().unwrap();
    // `Joiner.on("\n").join(Files.readLines(output, UTF_8))`.
    let raw = std::fs::read_to_string(&output).unwrap();
    let contents = trim_lines(&raw.lines().collect::<Vec<_>>().join("\n"));

    for event in [&started, &progress, &completed] {
        let text = trim_lines(&text_format::printer().print_to_string(event));
        assert!(
            contents.contains(&text),
            "{contents:?} does not contain {text:?}"
        );
    }
}
