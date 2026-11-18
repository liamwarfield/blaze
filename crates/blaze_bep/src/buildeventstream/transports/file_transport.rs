//! Port of `FileTransport`, `JsonFormatFileTransport` and `BinaryFormatFileTransport`.

use std::io::Write;
use std::sync::Arc;

use blaze_optdefs::misc::build_event_context::OutputGroupFileMode;
use blaze_optdefs::misc::build_event_protocol_options::{
    BuildEventProtocolOptions, OutputGroupFileModes,
};
use blaze_optdefs::misc::build_event_service_options::BesUploadMode;
use blaze_protos::JavaMessage;
use blaze_protos::build_event_stream as bes;
use blaze_protos::json::{JsonFormat, Printer, TypeRegistry};
use blaze_protos::tools::protos::SpawnExec;
use parking_lot::Mutex;

use crate::buildeventstream::artifact_uploader::BuildEventArtifactUploader;
use crate::buildeventstream::build_event::BuildEvent;
use crate::buildeventstream::build_event_context::{
    ArtifactGroupNamer, BuildEventContext, PathConverter,
};
use crate::buildeventstream::transport::{BuildEventTransport, TransportCloseError};

/// How a file transport serializes one event.
pub trait SerializeEvent: Send + Sync {
    // Java: com/google/devtools/build/lib/buildeventstream/transports/FileTransport.java:271 FileTransport.serializeEvent
    fn serialize_event(&self, build_event: &bes::BuildEvent) -> Vec<u8>;
}

struct WriterState {
    out: Option<Box<dyn Write + Send>>,
    /// The first write failure; the remaining writes are dropped (`exitFailure`).
    failure: Option<TransportCloseError>,
}

/// Non-blocking file transport.
///
/// Implementors of this class need to implement [`SerializeEvent`], which serializes a build
/// event to bytes.
///
/// PORT: Java converts each event to its proto when it is sent (the local uploader's futures are
/// immediate) and hands the bytes to a `bep-local-writer` thread (`SequentialWriter`). Here the
/// proto is converted and written on the sending thread, under a lock, in send order; the bytes
/// are the same. The periodic flush (`EXPERIMENTAL_BEP_FILE_FLUSH_MILLIS`) is replaced by a
/// flush after each event.
// Java: com/google/devtools/build/lib/buildeventstream/transports/FileTransport.java:64 FileTransport
pub struct FileTransport<S: SerializeEvent> {
    options: Arc<BuildEventProtocolOptions>,
    uploader: Arc<dyn BuildEventArtifactUploader>,
    writer: Mutex<WriterState>,
    namer: Arc<dyn ArtifactGroupNamer>,
    bes_upload_mode: BesUploadMode,
    serializer: S,
    name: &'static str,
}

impl<S: SerializeEvent> FileTransport<S> {
    // Java: com/google/devtools/build/lib/buildeventstream/transports/FileTransport.java:78 FileTransport.<init>
    fn create(
        output_stream: Box<dyn Write + Send>,
        options: Arc<BuildEventProtocolOptions>,
        uploader: Arc<dyn BuildEventArtifactUploader>,
        namer: Arc<dyn ArtifactGroupNamer>,
        bes_upload_mode: BesUploadMode,
        serializer: S,
        name: &'static str,
    ) -> Self {
        FileTransport {
            options,
            uploader,
            writer: Mutex::new(WriterState {
                out: Some(output_stream),
                failure: None,
            }),
            namer,
            bes_upload_mode,
            serializer,
            name,
        }
    }

    // Java: com/google/devtools/build/lib/buildeventstream/transports/FileTransport.java:283 FileTransport.asStreamProto
    fn as_stream_proto(&self, event: &dyn BuildEvent) -> Option<bes::BuildEvent> {
        let converter = self
            .uploader
            .upload_referenced_local_files(&event.referenced_local_files());
        // Remote uploads are already resolved (see BuildEvent::remote_uploads).
        let context = FileTransportContext {
            path_converter: converter,
            namer: self.namer.as_ref(),
            options: &self.options,
            output_group_modes: self.options.get_output_group_file_modes_mapping(),
        };
        // An interrupted conversion yields null, which the writer skips.
        event.as_stream_proto(&context).ok()
    }

    // Java: com/google/devtools/build/lib/buildeventstream/transports/FileTransport.java:156 SequentialWriter.exitFailure
    fn exit_failure(state: &mut WriterState, e: &std::io::Error) {
        let message = format!("Unable to write all BEP events to file due to '{e}'");
        state.failure = Some(TransportCloseError {
            message,
            code: "BES_FILE_WRITE_IO_ERROR",
        });
        state.out = None;
    }
}

struct FileTransportContext<'a> {
    path_converter: Box<dyn PathConverter>,
    namer: &'a dyn ArtifactGroupNamer,
    options: &'a BuildEventProtocolOptions,
    output_group_modes: OutputGroupFileModes,
}

impl BuildEventContext for FileTransportContext<'_> {
    fn path_converter(&self) -> &dyn PathConverter {
        self.path_converter.as_ref()
    }
    fn artifact_group_namer(&self) -> &dyn ArtifactGroupNamer {
        self.namer
    }
    fn get_options(&self) -> &BuildEventProtocolOptions {
        self.options
    }
    fn get_file_mode_for_output_group(&self, output_group: &str) -> OutputGroupFileMode {
        self.output_group_modes.get_mode(output_group)
    }
}

impl<S: SerializeEvent> BuildEventTransport for FileTransport<S> {
    fn name(&self) -> String {
        self.name.to_string()
    }

    // Java: com/google/devtools/build/lib/buildeventstream/transports/FileTransport.java:257 FileTransport.sendBuildEvent
    fn send_build_event(&self, event: &Arc<dyn BuildEvent>) {
        if self.writer.lock().out.is_none() {
            return;
        }
        let Some(proto) = self.as_stream_proto(event.as_ref()) else {
            return;
        };
        let serialized = self.serializer.serialize_event(&proto);
        let mut state = self.writer.lock();
        let result = match state.out.as_mut() {
            None => return,
            Some(out) => out.write_all(&serialized).and_then(|()| out.flush()),
        };
        if let Err(e) = result {
            Self::exit_failure(&mut state, &e);
        }
    }

    // Java: com/google/devtools/build/lib/buildeventstream/transports/FileTransport.java:225 FileTransport.close
    fn close(&self) -> Result<(), TransportCloseError> {
        let mut state = self.writer.lock();
        if let Some(mut out) = state.out.take()
            && let Err(e) = out.flush()
        {
            // Java logs "Failed to close BEP file output stream." and still completes.
            let _ = e;
        }
        match &state.failure {
            Some(f) => Err(f.clone()),
            None => Ok(()),
        }
    }

    // Java: com/google/devtools/build/lib/buildeventstream/transports/FileTransport.java:330 FileTransport.mayBeSlow
    fn may_be_slow(&self) -> bool {
        self.uploader.may_be_slow()
    }

    // Java: com/google/devtools/build/lib/buildeventstream/transports/FileTransport.java:335 FileTransport.getBesUploadMode
    fn get_bes_upload_mode(&self) -> BesUploadMode {
        self.bes_upload_mode
    }

    // Java: com/google/devtools/build/lib/buildeventstream/transports/FileTransport.java:340 FileTransport.getUploader
    fn get_uploader(&self) -> Option<Arc<dyn BuildEventArtifactUploader>> {
        Some(self.uploader.clone())
    }
}

/// Serializes events as `JsonFormat` lines.
// Java: com/google/devtools/build/lib/buildeventstream/transports/JsonFormatFileTransport.java:38 JsonFormatFileTransport
pub struct JsonFormatSerializer {
    json_printer: Printer,
}

/// A simple transport that writes the JSON representation of the protocol-buffer representation
/// of the events to a file.
pub type JsonFormatFileTransport = FileTransport<JsonFormatSerializer>;

/// `UNKNOWN_ANY_TYPE_ERROR_EVENT`: Gson's rendering of `UnknownAnyProtoError`.
// Java: com/google/devtools/build/lib/buildeventstream/transports/JsonFormatFileTransport.java:41 JsonFormatFileTransport.UNKNOWN_ANY_TYPE_ERROR_EVENT
pub const UNKNOWN_ANY_TYPE_ERROR_EVENT: &str =
    "{\"id\":\"unknown\",\"exception\":\"InvalidProtocolBufferException\"}";

impl JsonFormatFileTransport {
    // Java: com/google/devtools/build/lib/buildeventstream/transports/JsonFormatFileTransport.java:46 JsonFormatFileTransport.<init>
    pub fn new(
        output_stream: Box<dyn Write + Send>,
        options: Arc<BuildEventProtocolOptions>,
        uploader: Arc<dyn BuildEventArtifactUploader>,
        namer: Arc<dyn ArtifactGroupNamer>,
        type_registry: TypeRegistry,
        bes_upload_mode: BesUploadMode,
    ) -> Self {
        let json_printer = JsonFormat::printer()
            .using_type_registry(type_registry)
            .omitting_insignificant_whitespace();
        FileTransport::create(
            output_stream,
            options,
            uploader,
            namer,
            bes_upload_mode,
            JsonFormatSerializer { json_printer },
            "JsonFormatFileTransport",
        )
    }

    /// The registry `BuildEventServiceModule` passes: `TypeRegistry.newBuilder().add(
    /// SpawnExec.getDescriptor()).build()`.
    // Java: com/google/devtools/build/lib/buildeventservice/BuildEventServiceModule.java BuildEventServiceModule.createBepTransports
    pub fn bep_type_registry() -> TypeRegistry {
        TypeRegistry::new_builder().add::<SpawnExec>().build()
    }
}

impl SerializeEvent for JsonFormatSerializer {
    // Java: com/google/devtools/build/lib/buildeventstream/transports/JsonFormatFileTransport.java:64 JsonFormatFileTransport.serializeEvent
    fn serialize_event(&self, build_event: &bes::BuildEvent) -> Vec<u8> {
        let proto_json_representation = match self.json_printer.print(build_event) {
            Ok(s) => s,
            // Logged in Java: "Failed to serialize to JSON due to Any type resolution failure".
            Err(_) => UNKNOWN_ANY_TYPE_ERROR_EVENT.to_string(),
        };
        // PORT: StringEncoding identity; getBytes(UTF_8) of the printed string.
        format!("{proto_json_representation}\n").into_bytes()
    }
}

/// Serializes events as length-delimited binary protos.
// Java: com/google/devtools/build/lib/buildeventstream/transports/BinaryFormatFileTransport.java:33 BinaryFormatFileTransport
pub struct BinaryFormatSerializer;

/// A simple transport that writes a varint delimited binary representation of
/// `BuildEvent` protocol buffers to a file.
pub type BinaryFormatFileTransport = FileTransport<BinaryFormatSerializer>;

impl BinaryFormatFileTransport {
    // Java: com/google/devtools/build/lib/buildeventstream/transports/BinaryFormatFileTransport.java:34 BinaryFormatFileTransport.<init>
    pub fn new(
        output_stream: Box<dyn Write + Send>,
        options: Arc<BuildEventProtocolOptions>,
        uploader: Arc<dyn BuildEventArtifactUploader>,
        namer: Arc<dyn ArtifactGroupNamer>,
        bes_upload_mode: BesUploadMode,
    ) -> Self {
        FileTransport::create(
            output_stream,
            options,
            uploader,
            namer,
            bes_upload_mode,
            BinaryFormatSerializer,
            "BinaryFormatFileTransport",
        )
    }
}

impl SerializeEvent for BinaryFormatSerializer {
    // Java: com/google/devtools/build/lib/buildeventstream/transports/BinaryFormatFileTransport.java:49 BinaryFormatFileTransport.serializeEvent
    fn serialize_event(&self, build_event: &bes::BuildEvent) -> Vec<u8> {
        let mut out = Vec::new();
        build_event.write_delimited_to(&mut out);
        out
    }
}
