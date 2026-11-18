//! Port of `lib/remote/logging/SplitBlobHandler.java`.

use super::logging_handler::LoggingHandler;
use crate::reapi::remote_logging::{RpcCallDetails, SplitBlobDetails, rpc_call_details};
use crate::reapi::{SplitBlobRequest, SplitBlobResponse};

/// LoggingHandler for `build.bazel.remote.execution.v2.ContentAddressableStorage.SplitBlob` gRPC
/// call.
// Java: com/google/devtools/build/lib/remote/logging/SplitBlobHandler.java:26 SplitBlobHandler
#[derive(Debug, Default)]
pub struct SplitBlobHandler {
    builder: SplitBlobDetails,
}

impl SplitBlobHandler {
    /// `new SplitBlobHandler()`.
    // Java: com/google/devtools/build/lib/remote/logging/SplitBlobHandler.java:26 SplitBlobHandler.<init>
    pub fn new() -> SplitBlobHandler {
        SplitBlobHandler::default()
    }
}

impl LoggingHandler<SplitBlobRequest, SplitBlobResponse> for SplitBlobHandler {
    // Java: com/google/devtools/build/lib/remote/logging/SplitBlobHandler.java:31 SplitBlobHandler.handleReq
    fn handle_req(&mut self, message: &SplitBlobRequest) {
        self.builder.request = Some(message.clone());
    }

    // Java: com/google/devtools/build/lib/remote/logging/SplitBlobHandler.java:36 SplitBlobHandler.handleResp
    fn handle_resp(&mut self, message: &SplitBlobResponse) {
        self.builder.response = Some(message.clone());
    }

    // Java: com/google/devtools/build/lib/remote/logging/SplitBlobHandler.java:41 SplitBlobHandler.getDetails
    fn get_details(&mut self) -> RpcCallDetails {
        RpcCallDetails {
            details: Some(rpc_call_details::Details::SplitBlob(self.builder.clone())),
        }
    }
}
