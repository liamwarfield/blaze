//! Rust types for the Bazel protocol buffers whose bytes or JSON Blaze must reproduce, and
//! encoders that match Java protobuf byte for byte.
//!
//! The `.proto` sources are vendored under `proto/` (Bazel 9.3.2, remote-apis as vendored by
//! Bazel, googleapis at the commit Bazel's MODULE.bazel pins) and compiled by `build.rs` with
//! `protox` and `prost-build`. Package `a.b` becomes module `a::b`
//! (`blaze_protos::blaze_query::Target`, `blaze_protos::tools::protos::SpawnExec`,
//! `blaze_protos::build_event_stream::BuildEvent`, ...).
//!
//! - Binary: use [`JavaMessage`] (`to_byte_array`, `write_to`, `write_delimited_to`,
//!   `write_message_field`), not `prost::Message::encode`. prost writes a oneof at the position
//!   of its lowest field number and drops default map keys and values; Java does neither.
//! - JSON: [`json::JsonFormat::printer`], a port of `com.google.protobuf.util.JsonFormat`.
//! - Text: [`text_format::printer`] and [`text_format::get_parser`], a port of
//!   `com.google.protobuf.TextFormat`, driven by the runtime descriptors of [`descriptors`].
//! - Map fields are `Vec<XxxEntry>` in insertion order (Java's `MapField` order); keep keys
//!   unique, as a Java map would.

pub mod descriptors;
pub mod dynamic_message;
pub mod java;
pub mod json;
pub mod text_format;

pub use java::JavaMessage;

#[allow(
    clippy::all,
    clippy::pedantic,
    missing_docs,
    rustdoc::all,
    non_camel_case_types,
    unused_qualifications
)]
mod generated {
    include!(concat!(env!("OUT_DIR"), "/_includes.rs"));
}

pub use generated::*;

#[allow(clippy::all, clippy::pedantic, deprecated)]
mod java_impls {
    include!(concat!(env!("OUT_DIR"), "/java_impls.rs"));
}

pub(crate) use java_impls::{any_entry, file_dependencies};
