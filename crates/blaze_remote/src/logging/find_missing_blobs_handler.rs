//! Port of `lib/remote/logging/FindMissingBlobsHandler.java`.

use super::logging_handler::LoggingHandler;
use crate::reapi::remote_logging::{FindMissingBlobsDetails, RpcCallDetails, rpc_call_details};
use crate::reapi::{FindMissingBlobsRequest, FindMissingBlobsResponse};

/// LoggingHandler for `google.devtools.remoteexecution.v1test.ContentAddressableStorage.FindMissingBlobs`
/// gRPC call.
// Java: com/google/devtools/build/lib/remote/logging/FindMissingBlobsHandler.java:26 FindMissingBlobsHandler
#[derive(Debug, Default)]
pub struct FindMissingBlobsHandler {
    builder: FindMissingBlobsDetails,
}

impl FindMissingBlobsHandler {
    /// `new FindMissingBlobsHandler()`.
    // Java: com/google/devtools/build/lib/remote/logging/FindMissingBlobsHandler.java:26 FindMissingBlobsHandler.<init>
    pub fn new() -> FindMissingBlobsHandler {
        FindMissingBlobsHandler::default()
    }
}

impl LoggingHandler<FindMissingBlobsRequest, FindMissingBlobsResponse> for FindMissingBlobsHandler {
    // Java: com/google/devtools/build/lib/remote/logging/FindMissingBlobsHandler.java:32 FindMissingBlobsHandler.handleReq
    fn handle_req(&mut self, message: &FindMissingBlobsRequest) {
        self.builder.request = Some(message.clone());
    }

    // Java: com/google/devtools/build/lib/remote/logging/FindMissingBlobsHandler.java:37 FindMissingBlobsHandler.handleResp
    fn handle_resp(&mut self, message: &FindMissingBlobsResponse) {
        self.builder.response = Some(message.clone());
    }

    // Java: com/google/devtools/build/lib/remote/logging/FindMissingBlobsHandler.java:42 FindMissingBlobsHandler.getDetails
    fn get_details(&mut self) -> RpcCallDetails {
        RpcCallDetails {
            details: Some(rpc_call_details::Details::FindMissingBlobs(
                self.builder.clone(),
            )),
        }
    }
}
