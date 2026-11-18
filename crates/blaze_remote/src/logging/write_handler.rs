//! Port of `lib/remote/logging/WriteHandler.java`.

use indexmap::IndexSet;

use super::logging_handler::LoggingHandler;
use crate::reapi::bytestream::{WriteRequest, WriteResponse};
use crate::reapi::remote_logging::{RpcCallDetails, WriteDetails, rpc_call_details};

/// LoggingHandler for `google.bytestream.Write` gRPC call.
// Java: com/google/devtools/build/lib/remote/logging/WriteHandler.java:28 WriteHandler
#[derive(Debug, Default)]
pub struct WriteHandler {
    builder: WriteDetails,
    resources: IndexSet<String>,
    offsets: Vec<i64>,
    finish_writes: Vec<i64>,
    bytes_sent_in_sequence: i64,
    num_writes: i64,
    bytes_sent: i64,
}

impl WriteHandler {
    /// `new WriteHandler()`.
    // Java: com/google/devtools/build/lib/remote/logging/WriteHandler.java:28 WriteHandler.<init>
    pub fn new() -> WriteHandler {
        WriteHandler::default()
    }
}

impl LoggingHandler<WriteRequest, WriteResponse> for WriteHandler {
    // Java: com/google/devtools/build/lib/remote/logging/WriteHandler.java:38 WriteHandler.handleReq
    fn handle_req(&mut self, message: &WriteRequest) {
        self.resources.insert(message.resource_name.clone());
        let write_offset = message.write_offset;
        // `Iterables.getLast(offsets)` is only evaluated when `numWrites != 0`, and then the
        // list is not empty.
        if self.num_writes == 0
            || match self.offsets.last() {
                Some(last) => last + self.bytes_sent_in_sequence != write_offset,
                None => panic!("NoSuchElementException"),
            }
        {
            self.offsets.push(write_offset);
            self.bytes_sent_in_sequence = 0;
        }
        // `int size`, widened to long where it is added.
        let size = message.data.len() as i32;
        if message.finish_write {
            self.finish_writes.push(write_offset + size as i64);
        }

        self.num_writes += 1;
        self.bytes_sent += size as i64;
        self.bytes_sent_in_sequence += size as i64;
    }

    // Java: com/google/devtools/build/lib/remote/logging/WriteHandler.java:56 WriteHandler.handleResp
    fn handle_resp(&mut self, message: &WriteResponse) {
        self.builder.response = Some(*message);
    }

    // Java: com/google/devtools/build/lib/remote/logging/WriteHandler.java:61 WriteHandler.getDetails
    fn get_details(&mut self) -> RpcCallDetails {
        self.builder
            .resource_names
            .extend(self.resources.iter().cloned());
        self.builder.offsets.extend(self.offsets.iter().copied());
        self.builder
            .finish_writes
            .extend(self.finish_writes.iter().copied());
        self.builder.num_writes = self.num_writes;
        self.builder.bytes_sent = self.bytes_sent;
        RpcCallDetails {
            details: Some(rpc_call_details::Details::Write(self.builder.clone())),
        }
    }
}
