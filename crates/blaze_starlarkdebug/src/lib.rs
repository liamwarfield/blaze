//! Port of `com.google.devtools.build.lib.starlarkdebug.server`: the Starlark debug server of
//! `--experimental_skylark_debug` (a socket speaking `starlark_debugging.proto`), on top of the
//! interpreter hooks of `blaze_starlark::debug` (Java `net.starlark.java.eval.Debug`).

pub mod event_helper;
pub mod serialization;
pub mod server;
pub mod thread_handler;
pub mod transport;

pub use server::{DebugCallback, NoopCallback, StarlarkDebugServer};

// PORT: skipped, a Java exception type (the error travels as a String): ConditionalBreakpointException.<init>

/// `Message.toString()`: the text format, as the verbose log prints requests and events.
pub(crate) fn text_format<M: blaze_protos::JavaMessage>(message: &M) -> String {
    blaze_protos::text_format::message_to_string(message)
}
