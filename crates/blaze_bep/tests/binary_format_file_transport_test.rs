//! Port of `buildeventstream/transports/BinaryFormatFileTransportTest.java`.

use std::collections::HashMap;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use blaze_bep::buildeventstream::build_event_id_util;
use blaze_bep::buildeventstream::transports::BinaryFormatFileTransport;
use blaze_bep::buildeventstream::{
    ArtifactGroupNamer, ArtifactGroupNode, BuildEvent, BuildEventArtifactUploader,
    BuildEventContext, BuildEventTransport, FileUriPathConverter, Interrupted, LocalFile,
    LocalFileType, LocalFilesArtifactUploader, NoConversion, PathConverter, PathConverterFuture,
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

    transport.close().get().unwrap();
    let bytes = output.read();
    let mut input: &[u8] = &bytes;
    assert_eq!(parse_next(&mut input), Some(started));
    assert_eq!(parse_next(&mut input), Some(progress));
    assert_eq!(parse_next(&mut input), Some(completed));
    assert_eq!(input.len(), 0);
}

#[test]
fn test_cancelled_upload() {
    let fs = file_system();
    let file1 = Path::create_from_str("/foo", &fs);
    let event1: Arc<dyn BuildEvent> = Arc::new(WithLocalFilesEvent::new(vec![file1]));

    let uploader = Arc::new(TestUploader::new(
        |_files: &HashMap<Path, LocalFile>| PathConverterFuture::immediate_cancelled(),
        false,
    ));

    let output = Output::new();
    let transport = transport(&output, uploader);
    transport.send_build_event(&event1);

    let expected = transport.close().get().unwrap_err();
    assert!(
        expected
            .message
            .contains("Unable to write all BEP events to file due to 'Task was cancelled.'"),
        "{}",
        expected.message
    );

    let bytes = output.read();
    let mut input: &[u8] = &bytes;
    assert_eq!(parse_next(&mut input), None);
    assert_eq!(input.len(), 0);
}

#[test]
fn test_write_when_file_closed() {
    let output = Output::new();

    let build_event = Arc::new(MockBuildEvent::default());
    let build_event_dyn: Arc<dyn BuildEvent> = build_event.clone();
    build_event.set_stream_proto(started_build_event());

    let transport = transport(&output, Arc::new(LocalFilesArtifactUploader));

    transport.close().get().unwrap();

    // This should not throw an exception.
    transport.send_build_event(&build_event_dyn);
    transport.close().get().unwrap();

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
    transport.close().get().unwrap();
    // This should not throw an exception, but also not perform any write.
    transport.send_build_event(&build_event_dyn);

    // There should have only been one write.
    let bytes = output.read();
    let mut input: &[u8] = &bytes;
    assert_eq!(parse_next(&mut input), Some(started));
    assert_eq!(input.len(), 0);
}

/// An uploader whose `upload` is the given closure (Java: an anonymous
/// `BuildEventArtifactUploaderWithRefCounting`, spied on for `release()`).
struct TestUploader<F> {
    upload: F,
    may_be_slow: bool,
    releases: AtomicUsize,
}

impl<F> TestUploader<F> {
    fn new(upload: F, may_be_slow: bool) -> Self {
        TestUploader {
            upload,
            may_be_slow,
            releases: AtomicUsize::new(0),
        }
    }
}

impl<F> BuildEventArtifactUploader for TestUploader<F>
where
    F: Fn(&HashMap<Path, LocalFile>) -> PathConverterFuture + Send + Sync,
{
    fn upload(&self, files: HashMap<Path, LocalFile>) -> PathConverterFuture {
        (self.upload)(&files)
    }

    fn may_be_slow(&self) -> bool {
        self.may_be_slow
    }

    fn release(&self) {
        self.releases.fetch_add(1, Ordering::SeqCst);
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

    let uploader = Arc::new(TestUploader::new(
        move |files: &HashMap<Path, LocalFile>| {
            if files.contains_key(&file1) {
                std::thread::sleep(Duration::from_millis(200));
            }
            PathConverterFuture::immediate(Box::new(FileUriPathConverter))
        },
        true,
    ));
    let output = Output::new();
    let transport = transport(&output, uploader.clone());
    transport.send_build_event(&event1);
    transport.send_build_event(&event2);
    transport.close().get().unwrap();

    let bytes = output.read();
    let mut input: &[u8] = &bytes;
    assert_eq!(parse_next(&mut input), Some(as_stream_proto_null(&event1)));
    assert_eq!(parse_next(&mut input), Some(as_stream_proto_null(&event2)));
    assert_eq!(input.len(), 0);

    assert_eq!(uploader.releases.load(Ordering::SeqCst), 1);
}

/// Regression test for b/207287675
#[test]
fn test_handles_duplicate_files() {
    let fs = file_system();
    let file1 = Path::create_from_str("/foo", &fs);
    let event1: Arc<dyn BuildEvent> =
        Arc::new(WithLocalFilesEvent::new(vec![file1.clone(), file1.clone()]));

    let uploader = Arc::new(TestUploader::new(
        |_files: &HashMap<Path, LocalFile>| {
            PathConverterFuture::immediate(Box::new(FileUriPathConverter))
        },
        false,
    ));
    let output = Output::new();
    let transport = transport(&output, uploader);
    transport.send_build_event(&event1);
    transport.close().get().unwrap();

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

    let (upload, upload_future) = PathConverterFuture::create();
    let upload_future = Mutex::new(Some(upload_future));
    let uploader = Arc::new(TestUploader::new(
        move |_files: &HashMap<Path, LocalFile>| {
            upload_future.lock().unwrap().take().expect("one upload")
        },
        false,
    ));

    let output = Output::new();
    let transport = transport(&output, uploader.clone());
    transport.send_build_event(&event);
    // PORT: Java's close() returns a future and the test completes the upload after it;
    // the port's close() blocks, so the upload is completed from another thread while close()
    // waits.
    let setter = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(100));
        upload.set(Box::new(NoConversion));
    });
    transport.close().get().unwrap();
    setter.join().unwrap();

    let bytes = output.read();
    let mut input: &[u8] = &bytes;
    assert_eq!(parse_next(&mut input), Some(as_stream_proto_null(&event)));
    assert_eq!(input.len(), 0);

    assert_eq!(uploader.releases.load(Ordering::SeqCst), 1);
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
