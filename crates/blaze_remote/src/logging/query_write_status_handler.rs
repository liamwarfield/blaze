//! Port of `lib/remote/logging/QueryWriteStatusHandler.java`.

use super::logging_handler::LoggingHandler;
use crate::reapi::bytestream::{QueryWriteStatusRequest, QueryWriteStatusResponse};
use crate::reapi::remote_logging::{QueryWriteStatusDetails, RpcCallDetails, rpc_call_details};

/// LoggingHandler for `google.bytestream.QueryWriteStatus` gRPC call.
// Java: com/google/devtools/build/lib/remote/logging/QueryWriteStatusHandler.java:23 QueryWriteStatusHandler
#[derive(Debug, Default)]
pub struct QueryWriteStatusHandler {
    builder: QueryWriteStatusDetails,
}

impl QueryWriteStatusHandler {
    /// `new QueryWriteStatusHandler()`.
    // Java: com/google/devtools/build/lib/remote/logging/QueryWriteStatusHandler.java:23 QueryWriteStatusHandler.<init>
    pub fn new() -> QueryWriteStatusHandler {
        QueryWriteStatusHandler::default()
    }
}

impl LoggingHandler<QueryWriteStatusRequest, QueryWriteStatusResponse> for QueryWriteStatusHandler {
    // Java: com/google/devtools/build/lib/remote/logging/QueryWriteStatusHandler.java:28 QueryWriteStatusHandler.handleReq
    fn handle_req(&mut self, message: &QueryWriteStatusRequest) {
        self.builder.request = Some(message.clone());
    }

    // Java: com/google/devtools/build/lib/remote/logging/QueryWriteStatusHandler.java:33 QueryWriteStatusHandler.handleResp
    fn handle_resp(&mut self, message: &QueryWriteStatusResponse) {
        self.builder.response = Some(*message);
    }

    // Java: com/google/devtools/build/lib/remote/logging/QueryWriteStatusHandler.java:38 QueryWriteStatusHandler.getDetails
    fn get_details(&mut self) -> RpcCallDetails {
        RpcCallDetails {
            details: Some(rpc_call_details::Details::QueryWriteStatus(
                self.builder.clone(),
            )),
        }
    }
}
