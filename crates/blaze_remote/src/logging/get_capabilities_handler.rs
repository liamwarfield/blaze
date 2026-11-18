//! Port of `lib/remote/logging/GetCapabilitiesHandler.java`.

use super::logging_handler::LoggingHandler;
use crate::reapi::remote_logging::{GetCapabilitiesDetails, RpcCallDetails, rpc_call_details};
use crate::reapi::{GetCapabilitiesRequest, ServerCapabilities};

/// LoggingHandler for `google.devtools.remoteexecution.v1test.ActionCache.GetCapabilities` gRPC
/// call.
// Java: com/google/devtools/build/lib/remote/logging/GetCapabilitiesHandler.java:26 GetCapabilitiesHandler
#[derive(Debug, Default)]
pub struct GetCapabilitiesHandler {
    builder: GetCapabilitiesDetails,
}

impl GetCapabilitiesHandler {
    /// `new GetCapabilitiesHandler()`.
    // Java: com/google/devtools/build/lib/remote/logging/GetCapabilitiesHandler.java:26 GetCapabilitiesHandler.<init>
    pub fn new() -> GetCapabilitiesHandler {
        GetCapabilitiesHandler::default()
    }
}

impl LoggingHandler<GetCapabilitiesRequest, ServerCapabilities> for GetCapabilitiesHandler {
    // Java: com/google/devtools/build/lib/remote/logging/GetCapabilitiesHandler.java:32 GetCapabilitiesHandler.handleReq
    fn handle_req(&mut self, message: &GetCapabilitiesRequest) {
        self.builder.request = Some(message.clone());
    }

    // Java: com/google/devtools/build/lib/remote/logging/GetCapabilitiesHandler.java:37 GetCapabilitiesHandler.handleResp
    fn handle_resp(&mut self, message: &ServerCapabilities) {
        self.builder.response = Some(message.clone());
    }

    // Java: com/google/devtools/build/lib/remote/logging/GetCapabilitiesHandler.java:42 GetCapabilitiesHandler.getDetails
    fn get_details(&mut self) -> RpcCallDetails {
        RpcCallDetails {
            details: Some(rpc_call_details::Details::GetCapabilities(
                self.builder.clone(),
            )),
        }
    }
}
