//! Port of `lib/remote/logging/SpliceBlobHandler.java`.

use super::logging_handler::LoggingHandler;
use crate::reapi::remote_logging::{RpcCallDetails, SpliceBlobDetails, rpc_call_details};
use crate::reapi::{SpliceBlobRequest, SpliceBlobResponse};

/// LoggingHandler for `build.bazel.remote.execution.v2.ContentAddressableStorage.SpliceBlob` gRPC
/// call.
// Java: com/google/devtools/build/lib/remote/logging/SpliceBlobHandler.java:26 SpliceBlobHandler
#[derive(Debug, Default)]
pub struct SpliceBlobHandler {
    builder: SpliceBlobDetails,
}

impl SpliceBlobHandler {
    /// `new SpliceBlobHandler()`.
    // Java: com/google/devtools/build/lib/remote/logging/SpliceBlobHandler.java:26 SpliceBlobHandler.<init>
    pub fn new() -> SpliceBlobHandler {
        SpliceBlobHandler::default()
    }
}

impl LoggingHandler<SpliceBlobRequest, SpliceBlobResponse> for SpliceBlobHandler {
    // Java: com/google/devtools/build/lib/remote/logging/SpliceBlobHandler.java:31 SpliceBlobHandler.handleReq
    fn handle_req(&mut self, message: &SpliceBlobRequest) {
        self.builder.request = Some(message.clone());
    }

    // Java: com/google/devtools/build/lib/remote/logging/SpliceBlobHandler.java:36 SpliceBlobHandler.handleResp
    fn handle_resp(&mut self, message: &SpliceBlobResponse) {
        self.builder.response = Some(message.clone());
    }

    // Java: com/google/devtools/build/lib/remote/logging/SpliceBlobHandler.java:41 SpliceBlobHandler.getDetails
    fn get_details(&mut self) -> RpcCallDetails {
        RpcCallDetails {
            details: Some(rpc_call_details::Details::SpliceBlob(self.builder.clone())),
        }
    }
}
