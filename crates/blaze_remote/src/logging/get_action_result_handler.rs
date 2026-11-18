//! Port of `lib/remote/logging/GetActionResultHandler.java`.

use super::logging_handler::LoggingHandler;
use crate::reapi::remote_logging::{GetActionResultDetails, RpcCallDetails, rpc_call_details};
use crate::reapi::{ActionResult, GetActionResultRequest};

/// LoggingHandler for `google.devtools.remoteexecution.v1test.ActionCache.GetActionResult` gRPC
/// call.
// Java: com/google/devtools/build/lib/remote/logging/GetActionResultHandler.java:26 GetActionResultHandler
#[derive(Debug, Default)]
pub struct GetActionResultHandler {
    builder: GetActionResultDetails,
}

impl GetActionResultHandler {
    /// `new GetActionResultHandler()`.
    // Java: com/google/devtools/build/lib/remote/logging/GetActionResultHandler.java:26 GetActionResultHandler.<init>
    pub fn new() -> GetActionResultHandler {
        GetActionResultHandler::default()
    }
}

impl LoggingHandler<GetActionResultRequest, ActionResult> for GetActionResultHandler {
    // Java: com/google/devtools/build/lib/remote/logging/GetActionResultHandler.java:32 GetActionResultHandler.handleReq
    fn handle_req(&mut self, message: &GetActionResultRequest) {
        self.builder.request = Some(message.clone());
    }

    // Java: com/google/devtools/build/lib/remote/logging/GetActionResultHandler.java:37 GetActionResultHandler.handleResp
    fn handle_resp(&mut self, message: &ActionResult) {
        self.builder.response = Some(message.clone());
    }

    // Java: com/google/devtools/build/lib/remote/logging/GetActionResultHandler.java:42 GetActionResultHandler.getDetails
    fn get_details(&mut self) -> RpcCallDetails {
        RpcCallDetails {
            details: Some(rpc_call_details::Details::GetActionResult(
                self.builder.clone(),
            )),
        }
    }
}
