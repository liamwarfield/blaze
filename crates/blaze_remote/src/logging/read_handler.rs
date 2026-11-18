//! Port of `lib/remote/logging/ReadHandler.java`.

use super::logging_handler::LoggingHandler;
use crate::reapi::bytestream::{ReadRequest, ReadResponse};
use crate::reapi::remote_logging::{ReadDetails, RpcCallDetails, rpc_call_details};

/// LoggingHandler for `google.bytestream.Read` gRPC call.
// Java: com/google/devtools/build/lib/remote/logging/ReadHandler.java:23 ReadHandler
#[derive(Debug, Default)]
pub struct ReadHandler {
    builder: ReadDetails,
    num_reads: i64,
    bytes_read: i64,
}

impl ReadHandler {
    /// `new ReadHandler()`.
    // Java: com/google/devtools/build/lib/remote/logging/ReadHandler.java:23 ReadHandler.<init>
    pub fn new() -> ReadHandler {
        ReadHandler::default()
    }
}

impl LoggingHandler<ReadRequest, ReadResponse> for ReadHandler {
    // Java: com/google/devtools/build/lib/remote/logging/ReadHandler.java:29 ReadHandler.handleReq
    fn handle_req(&mut self, message: &ReadRequest) {
        self.builder.request = Some(message.clone());
    }

    // Java: com/google/devtools/build/lib/remote/logging/ReadHandler.java:34 ReadHandler.handleResp
    fn handle_resp(&mut self, message: &ReadResponse) {
        self.num_reads += 1;
        self.bytes_read += message.data.len() as i64;
    }

    // Java: com/google/devtools/build/lib/remote/logging/ReadHandler.java:40 ReadHandler.getDetails
    fn get_details(&mut self) -> RpcCallDetails {
        self.builder.num_reads = self.num_reads;
        self.builder.bytes_read = self.bytes_read;
        RpcCallDetails {
            details: Some(rpc_call_details::Details::Read(self.builder.clone())),
        }
    }
}
