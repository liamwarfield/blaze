//! Port of `buildeventstream/transports/BinaryFormatFileTransportTest.java`.

use std::collections::HashMap;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use blaze_bep::buildeventstream::build_event_id_util;
use blaze_bep::buildeventstream::transports::BinaryFormatFileTransport;
use blaze_bep::buildeventstream::{
    ArtifactGroupNamer, ArtifactGroupNode, BuildEvent, BuildEventArtifactUploader,
    BuildEventContext, BuildEventTransport, FileUriPathConverter, Interrupted, LocalFile,
    LocalFileType, LocalFilesArtifactUploader, NoConversion, PathConverter,
};
use blaze_optdefs::misc::build_event_protocol_options::BuildEventProtocolOptions;
use blaze_optdefs::misc::build_event_service_options::BesUploadMode;
use blaze_protos::build_event_stream::build_event::Payload;
use blaze_protos::build_event_stream::build_event_id::NamedSetOfFilesId;
use blaze_protos::build_event_stream::{self as bes, BuildEventId};
use blaze_protos::java::parse_delimited_from;
use blaze_vfs::{DigestHashFunction, FileSystemRef, Path, UnixFileSystem};
use tempfile::TempDir;

/// The Mockito `@Mock BuildEvent buildEvent`; `when(buildEvent.asStreamProto(any()))
/// .thenReturn(...)` is [`MockBuildEvent::set_stream_proto`].
#[derive(Debug, Default)]
struct MockBuildEvent {
    stream_proto: Mutex<bes::BuildEvent>,
}

impl MockBuildEvent {
    fn set_stream_proto(&self, proto: bes::BuildEvent) {
        *self.stream_proto.lock().unwrap() = proto;
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

/// `tmp.newFile()` and the `BufferedOutputStream` on it.
struct Output {
    _tmp: TempDir,
    output: std::path::PathBuf,
}

impl Output {
    fn new() -> Output {
        let tmp = tempfile::tempdir().unwrap();
        let output = tmp.path().join("output");
        File::create(&output).unwrap();
        Output { _tmp: tmp, output }
    }

    fn output_stream(&self) -> Box<dyn Write + Send> {
        Box::new(BufWriter::new(File::create(&self.output).unwrap()))
    }

    fn read(&self) -> Vec<u8> {
        std::fs::read(&self.output).unwrap()
    }
}

fn transport(
    output: &Output,
    uploader: Arc<dyn BuildEventArtifactUploader>,
) -> BinaryFormatFileTransport {
    BinaryFormatFileTransport::new(
        output.output_stream(),
        Arc::new(BuildEventProtocolOptions::default()),
        uploader,
        Arc::new(MockArtifactGroupNamer),
        BesUploadMode::WAIT_FOR_UPLOAD_COMPLETE,
    )
}

fn parse_next(input: &mut &[u8]) -> Option<bes::BuildEvent> {
    parse_delimited_from::<bes::BuildEvent>(input).unwrap()
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

/// A file system for the paths of [`WithLocalFilesEvent`]. PORT: Java mocks `Path` (only
/// `getBaseName` is stubbed); the paths here are never touched on disk.
fn file_system() -> FileSystemRef {
    Arc::new(UnixFileSystem::new(DigestHashFunction::Sha256))
}

#[test]
fn test_creates_file_and_writes_proto_binary_format() {
    let output = Output::new();

    let build_event = Arc::new(MockBuildEvent::default());
    let build_event_dyn: Arc<dyn BuildEvent> = build_event.clone();

    let started = started_build_event();
    build_event.set_stream_proto(started.clone());
    let transport = transport(&output, Arc::new(LocalFilesArtifactUploader));
    transport.send_build_event(&build_event_dyn);

    let progress = bes::BuildEvent {
        payload: Some(Payload::Progress(bes::Progress::default())),
        ..Default::default()
    };
    build_event.set_stream_proto(progress.clone());
    transport.send_build_event(&build_event_dyn);

    #[allow(deprecated)]
    let completed = bes::BuildEvent {
        payload: Some(Payload::Completed(bes::TargetComplete {
            success: true,
            ..Default::default()
        })),
        ..Default::default()
    };
    build_event.set_stream_proto(completed.clone());
    transport.send_build_event(&build_event_dyn);

    transport.close().unwrap();
    let bytes = output.read();
    let mut input: &[u8] = &bytes;
    assert_eq!(parse_next(&mut input), Some(started));
    assert_eq!(parse_next(&mut input), Some(progress));
    assert_eq!(parse_next(&mut input), Some(completed));
    assert_eq!(input.len(), 0);
}

#[test]
#[ignore = "needs asynchronous artifact uploads: BuildEventArtifactUploader.upload returns a \
            plain PathConverter in the port, so an upload cannot be cancelled \
            (Futures.immediateCancelledFuture)"]
fn test_cancelled_upload() {
    // Java: an uploader whose upload() returns an immediately cancelled future; close().get()
    // then throws an ExecutionException containing
    // "Unable to write all BEP events to file due to 'Task was cancelled.'" and the file is
    // empty.
}

#[test]
fn test_write_when_file_closed() {
    let output = Output::new();

    let build_event = Arc::new(MockBuildEvent::default());
    let build_event_dyn: Arc<dyn BuildEvent> = build_event.clone();
    build_event.set_stream_proto(started_build_event());

    let transport = transport(&output, Arc::new(LocalFilesArtifactUploader));

    transport.close().unwrap();

    // This should not throw an exception.
    transport.send_build_event(&build_event_dyn);
    transport.close().unwrap();

    // Also, nothing should have been written to the file
    assert_eq!(output.read().len(), 0);
}

#[test]
fn test_write_when_transport_closed() {
    let output = Output::new();

    let build_event = Arc::new(MockBuildEvent::default());
    let build_event_dyn: Arc<dyn BuildEvent> = build_event.clone();
    let started = started_build_event();
    build_event.set_stream_proto(started.clone());

    let transport = transport(&output, Arc::new(LocalFilesArtifactUploader));

    transport.send_build_event(&build_event_dyn);
    transport.close().unwrap();
    // This should not throw an exception, but also not perform any write.
    transport.send_build_event(&build_event_dyn);

    // There should have only been one write.
    let bytes = output.read();
    let mut input: &[u8] = &bytes;
    assert_eq!(parse_next(&mut input), Some(started));
    assert_eq!(input.len(), 0);
}

/// An uploader whose `upload` is the given closure (Java: an anonymous
/// `BuildEventArtifactUploaderWithRefCounting`).
struct TestUploader<F> {
    upload: F,
    may_be_slow: bool,
}

impl<F> BuildEventArtifactUploader for TestUploader<F>
where
    F: Fn(&HashMap<Path, LocalFile>) -> Box<dyn PathConverter> + Send + Sync,
{
    fn upload(&self, files: HashMap<Path, LocalFile>) -> Box<dyn PathConverter> {
        (self.upload)(&files)
    }

    fn may_be_slow(&self) -> bool {
        self.may_be_slow
    }
}

#[test]
fn test_writes_with_upload_delays() {
    // Test that events are written in order if the first event
    // has to wait a bit for local file uploads to finish.

    let fs = file_system();
    let file1 = Path::create_from_str("/file1", &fs);
    let file2 = Path::create_from_str("/file2", &fs);
    let event1: Arc<dyn BuildEvent> = Arc::new(WithLocalFilesEvent::new(vec![file1.clone()]));
    let event2: Arc<dyn BuildEvent> = Arc::new(WithLocalFilesEvent::new(vec![file2.clone()]));

    let uploader = Arc::new(TestUploader {
        upload: move |files: &HashMap<Path, LocalFile>| -> Box<dyn PathConverter> {
            if files.contains_key(&file1) {
                std::thread::sleep(Duration::from_millis(200));
            }
            Box::new(FileUriPathConverter)
        },
        may_be_slow: true,
    });
    let output = Output::new();
    let transport = transport(&output, uploader);
    transport.send_build_event(&event1);
    transport.send_build_event(&event2);
    transport.close().unwrap();

    let bytes = output.read();
    let mut input: &[u8] = &bytes;
    assert_eq!(parse_next(&mut input), Some(as_stream_proto_null(&event1)));
    assert_eq!(parse_next(&mut input), Some(as_stream_proto_null(&event2)));
    assert_eq!(input.len(), 0);

    // PORT: `verify(uploader).release()` is not ported: the uploader is not reference-counted
    // (see `BuildEventArtifactUploader`).
}

/// Regression test for b/207287675
#[test]
fn test_handles_duplicate_files() {
    let fs = file_system();
    let file1 = Path::create_from_str("/foo", &fs);
    let event1: Arc<dyn BuildEvent> =
        Arc::new(WithLocalFilesEvent::new(vec![file1.clone(), file1.clone()]));

    let uploader = Arc::new(TestUploader {
        upload: |_files: &HashMap<Path, LocalFile>| -> Box<dyn PathConverter> {
            Box::new(FileUriPathConverter)
        },
        may_be_slow: false,
    });
    let output = Output::new();
    let transport = transport(&output, uploader);
    transport.send_build_event(&event1);
    transport.close().unwrap();

    let bytes = output.read();
    let mut input: &[u8] = &bytes;
    assert_eq!(parse_next(&mut input), Some(as_stream_proto_null(&event1)));
    assert_eq!(input.len(), 0);
}

#[test]
fn test_close_waits_for_writes_to_finish() {
    // Test that .close() waits for all writes to finish.

    let fs = file_system();
    let file1 = Path::create_from_str("/file1", &fs);
    let event: Arc<dyn BuildEvent> = Arc::new(WithLocalFilesEvent::new(vec![file1]));

    // PORT: Java's upload returns a `SettableFuture` that is set to `PathConverter.NO_CONVERSION`
    // only after `close()` was called. The port's uploads complete synchronously, so the upload
    // returns `NO_CONVERSION` at once; the assertion that the event is in the file after close
    // is kept.
    let uploader = Arc::new(TestUploader {
        upload: |_files: &HashMap<Path, LocalFile>| -> Box<dyn PathConverter> {
            Box::new(NoConversion)
        },
        may_be_slow: false,
    });

    let output = Output::new();
    let transport = transport(&output, uploader);
    transport.send_build_event(&event);
    transport.close().unwrap();

    let bytes = output.read();
    let mut input: &[u8] = &bytes;
    assert_eq!(parse_next(&mut input), Some(as_stream_proto_null(&event)));
    assert_eq!(input.len(), 0);

    // PORT: `verify(uploader).release()` is not ported (no reference counting).
}

/// `event.asStreamProto(null)`: [`WithLocalFilesEvent`] ignores its context.
fn as_stream_proto_null(event: &Arc<dyn BuildEvent>) -> bes::BuildEvent {
    struct NullContext;
    impl BuildEventContext for NullContext {
        fn path_converter(&self) -> &dyn PathConverter {
            unreachable!("context is null")
        }
        fn artifact_group_namer(&self) -> &dyn ArtifactGroupNamer {
            unreachable!("context is null")
        }
        fn get_options(&self) -> &BuildEventProtocolOptions {
            unreachable!("context is null")
        }
    }
    event.as_stream_proto(&NullContext).unwrap()
}

#[derive(Debug)]
struct WithLocalFilesEvent {
    id: i32,
    files: Vec<Path>,
}

impl WithLocalFilesEvent {
    fn new(files: Vec<Path>) -> Self {
        WithLocalFilesEvent { id: 0, files }
    }
}

impl BuildEvent for WithLocalFilesEvent {
    fn referenced_local_files(&self) -> Vec<LocalFile> {
        self.files
            .iter()
            .map(|f| {
                LocalFile::new(
                    f.clone(),
                    LocalFileType::OutputFile,
                    /* artifact_metadata= */ None,
                )
            })
            .collect()
    }

    fn as_stream_proto(
        &self,
        _context: &dyn BuildEventContext,
    ) -> Result<bes::BuildEvent, Interrupted> {
        let base_names: Vec<&str> = self.files.iter().map(|f| f.get_base_name()).collect();
        Ok(bes::BuildEvent {
            id: Some(build_event_id_util::progress_id(self.id)),
            payload: Some(Payload::Progress(bes::Progress {
                stdout: format!("uploading: {}", base_names.join(", ")),
                ..Default::default()
            })),
            ..Default::default()
        })
    }

    fn get_event_id(&self) -> BuildEventId {
        build_event_id_util::progress_id(self.id)
    }

    fn get_children_events(&self) -> Vec<BuildEventId> {
        vec![build_event_id_util::progress_id(self.id + 1)]
    }
}
