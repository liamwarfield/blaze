//! Short names for the generated protocol types the remote package uses.
//!
//! Java imports them from `build.bazel.remote.execution.v2.*`, `com.google.bytestream.*`,
//! `com.google.longrunning.*` and `com.google.rpc.*`.

pub use blaze_protos::build::bazel::remote::execution::v2::*;
pub use blaze_protos::build::bazel::semver::SemVer;

/// `com.google.bytestream.ByteStreamProto`.
pub mod bytestream {
    pub use blaze_protos::google::bytestream::*;
}

/// `com.google.longrunning`.
pub mod longrunning {
    pub use blaze_protos::google::longrunning::*;
}

/// `com.google.rpc` (`Status` and the error details).
pub mod rpc {
    pub use blaze_protos::google::rpc::*;
}

/// `com.google.devtools.build.lib.remote.logging.RemoteExecutionLog`.
pub mod remote_logging {
    pub use blaze_protos::remote_logging::*;
}
