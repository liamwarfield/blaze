//! Port of `lib/worker/ProtoWorkerMessageProcessor.java`: the worker side of the proto worker
//! protocol, for workers written with [`super::work_request_handler::WorkRequestHandler`].

use std::io::{self, Write};

use blaze_protos::JavaMessage;
use blaze_protos::blaze::worker::{WorkRequest, WorkResponse};
use parking_lot::Mutex;
use prost::Message;

use super::error_message::JavaException;
use super::recording_input_stream::InputStream;
use super::work_request_handler::WorkerMessageProcessor;

/// `CodedInputStream`'s `InvalidProtocolBufferException.truncatedMessage()` text.
const TRUNCATED_MESSAGE: &str = "While parsing a protocol message, the input ended unexpectedly in the middle of a field.  This could mean either that the input has been truncated or that an embedded message misreported its own length.";
/// `InvalidProtocolBufferException.malformedVarint()`.
const MALFORMED_VARINT: &str = "CodedInputStream encountered a malformed varint.";

fn invalid_protocol_buffer(message: &str) -> io::Error {
    JavaException::new(
        "com.google.protobuf.InvalidProtocolBufferException",
        message,
    )
    .into_io_error()
}

/// Reads one byte; `Ok(None)` at the end of the stream.
fn read_byte(input: &mut dyn InputStream) -> io::Result<Option<u8>> {
    let mut b = [0u8; 1];
    Ok((input.read(&mut b)? == 1).then_some(b[0]))
}

/// `WorkRequest.parseDelimitedFrom(InputStream)`: `Ok(None)` at the end of the stream.
///
/// PORT: the message body is decoded by prost; a body prost rejects is an
/// `InvalidProtocolBufferException` carrying prost's text (Java's `CodedInputStream` names the
/// defect in its own words).
// Java: com/google/protobuf/AbstractParser.java AbstractParser.parsePartialDelimitedFrom
pub fn parse_delimited_work_request(
    input: &mut dyn InputStream,
) -> io::Result<Option<WorkRequest>> {
    let Some(first_byte) = read_byte(input)? else {
        return Ok(None);
    };
    // CodedInputStream.readRawVarint32(firstByte, input)
    let size: i32 = if first_byte & 0x80 == 0 {
        first_byte as i32
    } else {
        let mut result: i32 = (first_byte & 0x7f) as i32;
        let mut offset = 7;
        let mut done = false;
        while offset < 32 {
            let b = read_byte(input)?.ok_or_else(|| invalid_protocol_buffer(TRUNCATED_MESSAGE))?;
            result |= ((b & 0x7f) as i32) << offset;
            if b & 0x80 == 0 {
                done = true;
                break;
            }
            offset += 7;
        }
        if !done {
            // Keep reading up to 64 bits.
            while offset < 64 {
                let b =
                    read_byte(input)?.ok_or_else(|| invalid_protocol_buffer(TRUNCATED_MESSAGE))?;
                if b & 0x80 == 0 {
                    done = true;
                    break;
                }
                offset += 7;
            }
            if !done {
                return Err(invalid_protocol_buffer(MALFORMED_VARINT));
            }
        }
        result
    };
    // A negative size gives an empty LimitedInputStream in Java.
    let mut body = vec![0u8; size.max(0) as usize];
    let mut filled = 0;
    while filled < body.len() {
        let n = input.read(&mut body[filled..])?;
        if n == 0 {
            return Err(invalid_protocol_buffer(TRUNCATED_MESSAGE));
        }
        filled += n;
    }
    WorkRequest::decode(body.as_slice())
        .map(Some)
        .map_err(|e| invalid_protocol_buffer(&e.to_string()))
}

/// Implementation of the Worker Protocol using Proto to communicate with Bazel.
// Java: com/google/devtools/build/lib/worker/ProtoWorkerMessageProcessor.java:25 ProtoWorkerMessageProcessor
pub struct ProtoWorkerMessageProcessor {
    /// This worker's stdin.
    stdin: Mutex<Box<dyn InputStream>>,
    /// This worker's stdout. Only `WorkRequest`s should be written here.
    stdout: Mutex<Box<dyn Write + Send>>,
}

impl ProtoWorkerMessageProcessor {
    /// Constructs a `WorkRequestHandler` that reads and writes Protocol Buffers.
    // Java: com/google/devtools/build/lib/worker/ProtoWorkerMessageProcessor.java:35 ProtoWorkerMessageProcessor.<init>
    pub fn new(
        stdin: Box<dyn InputStream>,
        stdout: Box<dyn Write + Send>,
    ) -> ProtoWorkerMessageProcessor {
        ProtoWorkerMessageProcessor {
            stdin: Mutex::new(stdin),
            stdout: Mutex::new(stdout),
        }
    }
}

impl WorkerMessageProcessor for ProtoWorkerMessageProcessor {
    // Java: com/google/devtools/build/lib/worker/ProtoWorkerMessageProcessor.java:40 ProtoWorkerMessageProcessor.readWorkRequest
    fn read_work_request(&self) -> io::Result<Option<WorkRequest>> {
        parse_delimited_work_request(&mut **self.stdin.lock())
    }

    // Java: com/google/devtools/build/lib/worker/ProtoWorkerMessageProcessor.java:45 ProtoWorkerMessageProcessor.writeWorkResponse
    fn write_work_response(&self, work_response: &WorkResponse) -> io::Result<()> {
        let mut buf = Vec::new();
        work_response.write_delimited_to(&mut buf);
        let mut stdout = self.stdout.lock();
        let result = stdout.write_all(&buf);
        // finally
        let flushed = stdout.flush();
        result.and(flushed)
    }

    // Java: com/google/devtools/build/lib/worker/ProtoWorkerMessageProcessor.java:53 ProtoWorkerMessageProcessor.close
    fn close(&self) -> io::Result<()> {
        Ok(())
    }
}
