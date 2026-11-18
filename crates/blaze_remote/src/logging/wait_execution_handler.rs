//! Port of `lib/remote/logging/WaitExecutionHandler.java`.

use super::logging_handler::LoggingHandler;
use crate::reapi::WaitExecutionRequest;
use crate::reapi::longrunning::Operation;
use crate::reapi::remote_logging::{RpcCallDetails, WaitExecutionDetails, rpc_call_details};

/// LoggingHandler for `build.bazel.remote.execution.v2.WaitExecution` gRPC call.
// Java: com/google/devtools/build/lib/remote/logging/WaitExecutionHandler.java:23 WaitExecutionHandler
#[derive(Debug, Default)]
pub struct WaitExecutionHandler {
    builder: WaitExecutionDetails,
}

impl WaitExecutionHandler {
    /// `new WaitExecutionHandler()`.
    // Java: com/google/devtools/build/lib/remote/logging/WaitExecutionHandler.java:23 WaitExecutionHandler.<init>
    pub fn new() -> WaitExecutionHandler {
        WaitExecutionHandler::default()
    }
}

impl LoggingHandler<WaitExecutionRequest, Operation> for WaitExecutionHandler {
    // Java: com/google/devtools/build/lib/remote/logging/WaitExecutionHandler.java:27 WaitExecutionHandler.handleReq
    fn handle_req(&mut self, message: &WaitExecutionRequest) {
        self.builder.request = Some(message.clone());
    }

    // Java: com/google/devtools/build/lib/remote/logging/WaitExecutionHandler.java:32 WaitExecutionHandler.handleResp
    fn handle_resp(&mut self, message: &Operation) {
        self.builder.responses.push(message.clone());
    }

    // Java: com/google/devtools/build/lib/remote/logging/WaitExecutionHandler.java:37 WaitExecutionHandler.getDetails
    fn get_details(&mut self) -> RpcCallDetails {
        RpcCallDetails {
            details: Some(rpc_call_details::Details::WaitExecution(
                self.builder.clone(),
            )),
        }
    }
}
