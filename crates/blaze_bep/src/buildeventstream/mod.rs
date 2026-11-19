//! Port of `lib/buildeventstream`.

pub mod artifact_uploader;
pub mod build_completing_event;
pub mod build_event;
pub mod build_event_context;
pub mod build_event_id_util;
pub mod generic_build_event;
pub mod transport;
pub mod transports;

pub use artifact_uploader::{
    BuildEventArtifactUploader, JavaThrowable, LocalFilesArtifactUploader, PathConverterFuture,
    PathConverterPromise, UploadFailure,
};
pub use build_completing_event::{BuildCompletingEvent, BuildToolLogs, FetchEvent, LogFileEntry};
pub use build_event::{
    BuildCompleteResultFacts, BuildEvent, BuildEventClass, Interrupted, LocalFile,
    LocalFileCompression, LocalFileType, ReportedArtifacts,
};
pub use build_event_context::{
    ArtifactGroupNamer, ArtifactGroupNode, BuildEventContext, FileUriPathConverter, NoConversion,
    PathConverter,
};
pub use generic_build_event::{
    AbortedEvent, GenericBuildEvent, LastBuildEvent, NullConfiguration, ProgressEvent,
    proto_chaining, proto_chaining_with,
};
pub use transport::{BuildEventTransport, CloseFuture, TransportCloseError};
