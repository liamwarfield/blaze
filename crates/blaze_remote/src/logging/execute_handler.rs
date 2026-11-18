//! Port of `lib/remote/logging/ExecuteHandler.java`.

use super::logging_handler::LoggingHandler;
use crate::reapi::ExecuteRequest;
use crate::reapi::longrunning::Operation;
use crate::reapi::remote_logging::{ExecuteDetails, RpcCallDetails, rpc_call_details};

/// LoggingHandler for google.devtools.remoteexecution.v1test.Execution.Execute gRPC call.
// Java: com/google/devtools/build/lib/remote/logging/ExecuteHandler.java:23 ExecuteHandler
#[derive(Debug, Default)]
pub struct ExecuteHandler {
    builder: ExecuteDetails,
}

impl ExecuteHandler {
    /// `new ExecuteHandler()`.
    // Java: com/google/devtools/build/lib/remote/logging/ExecuteHandler.java:23 ExecuteHandler.<init>
    pub fn new() -> ExecuteHandler {
        ExecuteHandler::default()
    }
}

impl LoggingHandler<ExecuteRequest, Operation> for ExecuteHandler {
    // Java: com/google/devtools/build/lib/remote/logging/ExecuteHandler.java:32 ExecuteHandler.handleReq
    fn handle_req(&mut self, message: &ExecuteRequest) {
        self.builder.request = Some(message.clone());
    }

    // Java: com/google/devtools/build/lib/remote/logging/ExecuteHandler.java:37 ExecuteHandler.handleResp
    fn handle_resp(&mut self, message: &Operation) {
        self.builder.responses.push(message.clone());
    }

    // Java: com/google/devtools/build/lib/remote/logging/ExecuteHandler.java:42 ExecuteHandler.getDetails
    fn get_details(&mut self) -> RpcCallDetails {
        RpcCallDetails {
            details: Some(rpc_call_details::Details::Execute(self.builder.clone())),
        }
    }
}
