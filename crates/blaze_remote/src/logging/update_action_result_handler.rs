//! Port of `lib/remote/logging/UpdateActionResultHandler.java`.

use super::logging_handler::LoggingHandler;
use crate::reapi::remote_logging::{RpcCallDetails, UpdateActionResultDetails, rpc_call_details};
use crate::reapi::{ActionResult, UpdateActionResultRequest};

/// LoggingHandler for `google.devtools.remoteexecution.v1test.ActionCache.UpdateActionResult` gRPC
/// call.
// Java: com/google/devtools/build/lib/remote/logging/UpdateActionResultHandler.java:26 UpdateActionResultHandler
#[derive(Debug, Default)]
pub struct UpdateActionResultHandler {
    update_actiont_result_details_builder: UpdateActionResultDetails,
}

impl UpdateActionResultHandler {
    /// `new UpdateActionResultHandler()`.
    // Java: com/google/devtools/build/lib/remote/logging/UpdateActionResultHandler.java:26 UpdateActionResultHandler.<init>
    pub fn new() -> UpdateActionResultHandler {
        UpdateActionResultHandler::default()
    }
}

impl LoggingHandler<UpdateActionResultRequest, ActionResult> for UpdateActionResultHandler {
    // Java: com/google/devtools/build/lib/remote/logging/UpdateActionResultHandler.java:33 UpdateActionResultHandler.handleReq
    fn handle_req(&mut self, message: &UpdateActionResultRequest) {
        self.update_actiont_result_details_builder.request = Some(message.clone());
    }

    // Java: com/google/devtools/build/lib/remote/logging/UpdateActionResultHandler.java:38 UpdateActionResultHandler.handleResp
    fn handle_resp(&mut self, message: &ActionResult) {
        self.update_actiont_result_details_builder.response = Some(message.clone());
    }

    // Java: com/google/devtools/build/lib/remote/logging/UpdateActionResultHandler.java:43 UpdateActionResultHandler.getDetails
    fn get_details(&mut self) -> RpcCallDetails {
        RpcCallDetails {
            details: Some(rpc_call_details::Details::UpdateActionResult(
                self.update_actiont_result_details_builder.clone(),
            )),
        }
    }
}
