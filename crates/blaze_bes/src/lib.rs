//! Port of `lib/buildeventservice`: the Build Event Service client (`--bes_backend`).
//!
//! - [`client`]: the client interface, the request builders and the gRPC client.
//! - [`uploader`]: the event loop that publishes the lifecycle events and streams the build
//!   events with retries and acknowledgements.
//! - [`transport`]: the `BuildEventTransport` over the uploader.
//! - [`module`]: the module's client cache, keywords, results URL and transport creation.

pub mod availability_event;
pub mod client;
pub mod module;
pub mod transport;
pub mod uploader;

/// Stubs of `google.devtools.build.v1.PublishBuildEvent` (client and server).
#[allow(clippy::all, clippy::pedantic, missing_docs, unused_qualifications)]
pub mod stubs {
    include!(concat!(
        env!("OUT_DIR"),
        "/google.devtools.build.v1.PublishBuildEvent.rs"
    ));
}
