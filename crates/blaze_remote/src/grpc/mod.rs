//! gRPC plumbing: the generated service stubs (clients and, for the in-process fake servers of the
//! tests, servers). Java: the generated `ActionCacheGrpc`, `ContentAddressableStorageGrpc`,
//! `CapabilitiesGrpc`, `ExecutionGrpc` and `ByteStreamGrpc` classes.

/// Stubs of `build.bazel.remote.execution.v2` services and `google.bytestream.ByteStream`.
#[allow(clippy::all, clippy::pedantic, missing_docs, unused_qualifications)]
pub mod stubs {
    include!(concat!(
        env!("OUT_DIR"),
        "/build.bazel.remote.execution.v2.Execution.rs"
    ));
    include!(concat!(
        env!("OUT_DIR"),
        "/build.bazel.remote.execution.v2.ActionCache.rs"
    ));
    include!(concat!(
        env!("OUT_DIR"),
        "/build.bazel.remote.execution.v2.ContentAddressableStorage.rs"
    ));
    include!(concat!(
        env!("OUT_DIR"),
        "/build.bazel.remote.execution.v2.Capabilities.rs"
    ));
    include!(concat!(env!("OUT_DIR"), "/google.bytestream.ByteStream.rs"));
}
