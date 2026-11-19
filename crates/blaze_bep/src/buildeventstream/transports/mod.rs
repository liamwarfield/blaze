//! Port of `lib/buildeventstream/transports`: the file transports.

pub mod file_transport;

pub use file_transport::{
    BinaryFormatFileTransport, FileTransport, JsonFormatFileTransport, TextFormatFileTransport,
};
