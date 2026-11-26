//! Ports of `lib/worker/WorkerProtocolImpl.java`, `ProtoWorkerProtocol.java` and
//! `JsonWorkerProtocol.java`: the two wire formats between Bazel and a persistent worker
//! (`requires-worker-protocol`).

use std::io::{self, Write};
use std::sync::Arc;

use blaze_protos::JavaMessage;
use blaze_protos::blaze::worker::{WorkRequest, WorkResponse};
use blaze_protos::json::JsonFormat;
use parking_lot::Mutex;

use super::error_message::JavaException;
use super::json_reader::{JsonReader, JsonReaderError};
use super::recording_input_stream::{InputStream, RecordingInputStream};

/// The worker's stdout as the protocols read it: the shared [`RecordingInputStream`].
#[derive(Clone)]
pub struct SharedRecordingStream(pub Arc<Mutex<RecordingInputStream>>);

impl InputStream for SharedRecordingStream {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.0.lock().read(buf)
    }

    fn available(&mut self) -> io::Result<usize> {
        self.0.lock().available()
    }
}

/// Represents the communication between Bazel and a persistent worker.
// Java: com/google/devtools/build/lib/worker/WorkerProtocolImpl.java:22 WorkerProtocolImpl
///
/// PORT: the methods take `&self`; the request side and the response side are locked
/// separately, as the multiplexer's sender and receiver threads use one protocol object at the
/// same time (Java's implementations have no locks; each side touches only its own stream).
pub trait WorkerProtocolImpl: Send + Sync {
    /// Writes the provided work request to the worker.
    fn put_request(&self, request: &WorkRequest) -> io::Result<()>;

    /// Reads a response from the worker. `Ok(None)` where Java returns null (the proto protocol
    /// at the end of the stream).
    fn get_response(&self) -> io::Result<Option<WorkResponse>>;

    /// `close()`.
    fn close(&self) -> io::Result<()>;
}

/// `CodedInputStream`'s `InvalidProtocolBufferException` messages.
const TRUNCATED_MESSAGE: &str = "While parsing a protocol message, the input ended unexpectedly in the middle of a field.  This could mean either that the input has been truncated or that an embedded message misreported its own length.";
const MALFORMED_VARINT: &str = "CodedInputStream encountered a malformed varint.";
const NEGATIVE_SIZE: &str = "CodedInputStream encountered an embedded string or message which claimed to have negative size.";
const INVALID_TAG: &str = "Protocol message contained an invalid tag (zero).";
const INVALID_WIRE_TYPE: &str = "Protocol message tag had invalid wire type.";
const INVALID_END_TAG: &str = "Protocol message end-group tag did not match expected tag.";
const INVALID_UTF8: &str = "Protocol message had invalid UTF-8.";

fn invalid_protocol_buffer(message: &str) -> io::Error {
    JavaException::new(
        "com.google.protobuf.InvalidProtocolBufferException",
        message,
    )
    .into_io_error()
}

/// A reader of one delimited message, byte by byte (`LimitedInputStream`).
struct Limited<'a> {
    input: &'a mut dyn InputStream,
    remaining: i64,
}

impl Limited<'_> {
    fn read_byte(&mut self) -> io::Result<u8> {
        if self.remaining <= 0 {
            return Err(invalid_protocol_buffer(TRUNCATED_MESSAGE));
        }
        let mut b = [0u8; 1];
        if self.input.read(&mut b)? == 0 {
            return Err(invalid_protocol_buffer(TRUNCATED_MESSAGE));
        }
        self.remaining -= 1;
        Ok(b[0])
    }

    fn at_end(&self) -> bool {
        self.remaining <= 0
    }

    // CodedInputStream.readRawVarint64
    fn read_varint64(&mut self) -> io::Result<u64> {
        let mut result: u64 = 0;
        let mut shift = 0;
        while shift < 64 {
            let b = self.read_byte()?;
            result |= ((b & 0x7F) as u64) << shift;
            if b & 0x80 == 0 {
                return Ok(result);
            }
            shift += 7;
        }
        Err(invalid_protocol_buffer(MALFORMED_VARINT))
    }

    fn read_bytes(&mut self, n: i64) -> io::Result<Vec<u8>> {
        if n < 0 {
            return Err(invalid_protocol_buffer(NEGATIVE_SIZE));
        }
        if n > self.remaining {
            // Java reads up to the limit and then fails.
            while self.remaining > 0 {
                self.read_byte()?;
            }
            return Err(invalid_protocol_buffer(TRUNCATED_MESSAGE));
        }
        let mut v = Vec::with_capacity(n as usize);
        for _ in 0..n {
            v.push(self.read_byte()?);
        }
        Ok(v)
    }

    // CodedInputStream.skipField(tag)
    fn skip_field(&mut self, tag: u32) -> io::Result<bool> {
        match tag & 7 {
            0 => {
                self.read_varint64()?;
                Ok(true)
            }
            1 => {
                self.read_bytes(8)?;
                Ok(true)
            }
            2 => {
                let len = self.read_varint64()? as i32;
                self.read_bytes(len as i64)?;
                Ok(true)
            }
            3 => {
                // skipMessage(), then checkLastTagWas(makeTag(fieldNumber, END_GROUP)).
                let end_tag = (tag & !7) | 4;
                loop {
                    if self.at_end() {
                        return Err(invalid_protocol_buffer(INVALID_END_TAG));
                    }
                    let t = self.read_tag()?;
                    if t == 0 || !self.skip_field(t)? {
                        if t != end_tag {
                            return Err(invalid_protocol_buffer(INVALID_END_TAG));
                        }
                        return Ok(true);
                    }
                }
            }
            4 => Ok(false),
            5 => {
                self.read_bytes(4)?;
                Ok(true)
            }
            // InvalidProtocolBufferException.invalidWireType() throws the subclass.
            _ => Err(JavaException::new(
                "com.google.protobuf.InvalidProtocolBufferException$InvalidWireTypeException",
                INVALID_WIRE_TYPE,
            )
            .into_io_error()),
        }
    }

    // CodedInputStream.readTag
    fn read_tag(&mut self) -> io::Result<u32> {
        let tag = self.read_varint64()? as u32;
        if (tag >> 3) == 0 {
            // If we actually read zero (or any tag number corresponding to field number zero),
            // that's not a valid tag.
            return Err(invalid_protocol_buffer(INVALID_TAG));
        }
        Ok(tag)
    }
}

/// `WorkResponse.parseDelimitedFrom(InputStream)`: `Ok(None)` at the end of the stream.
// Java: com/google/protobuf/AbstractParser.java AbstractParser.parsePartialDelimitedFrom
pub fn parse_delimited_work_response(
    input: &mut dyn InputStream,
) -> io::Result<Option<WorkResponse>> {
    let mut first = [0u8; 1];
    if input.read(&mut first)? == 0 {
        return Ok(None);
    }
    // CodedInputStream.readRawVarint32(firstByte, input)
    let size: i32 = {
        let first_byte = first[0];
        if first_byte & 0x80 == 0 {
            first_byte as i32
        } else {
            let mut result: i32 = (first_byte & 0x7f) as i32;
            let mut offset = 7;
            let mut b = [0u8; 1];
            let mut done = false;
            while offset < 32 {
                if input.read(&mut b)? == 0 {
                    return Err(invalid_protocol_buffer(TRUNCATED_MESSAGE));
                }
                result |= ((b[0] & 0x7f) as i32) << offset;
                if b[0] & 0x80 == 0 {
                    done = true;
                    break;
                }
                offset += 7;
            }
            if !done {
                // Keep reading up to 64 bits.
                while offset < 64 {
                    if input.read(&mut b)? == 0 {
                        return Err(invalid_protocol_buffer(TRUNCATED_MESSAGE));
                    }
                    if b[0] & 0x80 == 0 {
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
        }
    };
    let mut limited = Limited {
        input,
        // A negative size gives an empty LimitedInputStream in Java.
        remaining: size.max(0) as i64,
    };
    let mut response = WorkResponse::default();
    loop {
        if limited.at_end() {
            break;
        }
        let tag = limited.read_tag()?;
        match tag {
            8 => response.exit_code = limited.read_varint64()? as i32,
            18 => {
                let len = limited.read_varint64()? as i32;
                let bytes = limited.read_bytes(len as i64)?;
                // proto3 string: readStringRequireUtf8.
                response.output =
                    String::from_utf8(bytes).map_err(|_| invalid_protocol_buffer(INVALID_UTF8))?;
            }
            24 => response.request_id = limited.read_varint64()? as i32,
            32 => response.was_cancelled = limited.read_varint64()? != 0,
            _ => {
                if !limited.skip_field(tag)? {
                    // An end-group tag at the top level: checkLastTagWas(0) fails.
                    return Err(invalid_protocol_buffer(INVALID_END_TAG));
                }
            }
        }
    }
    Ok(Some(response))
}

/// An implementation of a Bazel worker using Proto to communicate with the worker process.
// Java: com/google/devtools/build/lib/worker/ProtoWorkerProtocol.java:26 ProtoWorkerProtocol
pub struct ProtoWorkerProtocol {
    /// The worker process's stdin, which we send requests to.
    workers_stdin: Mutex<Box<dyn Write + Send>>,
    /// The worker process's stdout, which we read responses from.
    workers_stdout: Mutex<Box<dyn InputStream>>,
}

impl ProtoWorkerProtocol {
    // Java: com/google/devtools/build/lib/worker/ProtoWorkerProtocol.java:34 ProtoWorkerProtocol.<init>
    pub fn new(
        workers_stdin: Box<dyn Write + Send>,
        workers_stdout: Box<dyn InputStream>,
    ) -> ProtoWorkerProtocol {
        ProtoWorkerProtocol {
            workers_stdin: Mutex::new(workers_stdin),
            workers_stdout: Mutex::new(workers_stdout),
        }
    }
}

impl WorkerProtocolImpl for ProtoWorkerProtocol {
    // Java: com/google/devtools/build/lib/worker/ProtoWorkerProtocol.java:40 ProtoWorkerProtocol.putRequest
    fn put_request(&self, request: &WorkRequest) -> io::Result<()> {
        let mut buf = Vec::new();
        request.write_delimited_to(&mut buf);
        let mut stdin = self.workers_stdin.lock();
        stdin.write_all(&buf)?;
        stdin.flush()
    }

    // Java: com/google/devtools/build/lib/worker/ProtoWorkerProtocol.java:46 ProtoWorkerProtocol.getResponse
    fn get_response(&self) -> io::Result<Option<WorkResponse>> {
        let interrupted = blaze_shell::thread_interrupt::interrupted();
        let result = parse_delimited_work_response(&mut **self.workers_stdout.lock());
        if interrupted {
            blaze_shell::thread_interrupt::interrupt_current();
        }
        result
    }

    // Java: com/google/devtools/build/lib/worker/ProtoWorkerProtocol.java:58 ProtoWorkerProtocol.close
    fn close(&self) -> io::Result<()> {
        Ok(())
    }
}

/// An implementation of a Bazel worker using JSON to communicate with the worker process.
// Java: com/google/devtools/build/lib/worker/JsonWorkerProtocol.java:33 JsonWorkerProtocol
pub struct JsonWorkerProtocol {
    /// Reader for reading the WorkResponse.
    reader: Mutex<JsonReader>,
    /// Writer for writing the WorkRequest to the worker.
    json_writer: Mutex<Option<Box<dyn Write + Send>>>,
}

impl JsonWorkerProtocol {
    // Java: com/google/devtools/build/lib/worker/JsonWorkerProtocol.java:41 JsonWorkerProtocol.<init>
    pub fn new(
        workers_stdin: Box<dyn Write + Send>,
        workers_stdout: Box<dyn InputStream>,
    ) -> JsonWorkerProtocol {
        JsonWorkerProtocol {
            reader: Mutex::new(JsonReader::new(workers_stdout)),
            json_writer: Mutex::new(Some(workers_stdin)),
        }
    }

    // Java: com/google/devtools/build/lib/worker/JsonWorkerProtocol.java:69 JsonWorkerProtocol.parseResponse
    fn parse_response(&self) -> io::Result<WorkResponse> {
        let mut reader = self.reader.lock();
        let reader = &mut *reader;
        let mut exit_code: Option<i32> = None;
        let mut output: Option<String> = None;
        let mut request_id: Option<i32> = None;
        let parsed: Result<(), JsonReaderError> = (|| {
            reader.begin_object()?;
            while reader.has_next()? {
                let name = reader.next_name()?;
                match name.as_str() {
                    "exitCode" => {
                        if exit_code.is_some() {
                            return Err(JsonReaderError::Io(
                                JavaException::io(
                                    "Work response cannot have more than one exit code",
                                )
                                .into_io_error(),
                            ));
                        }
                        exit_code = Some(reader.next_int()?);
                    }
                    "output" => {
                        if output.is_some() {
                            return Err(JsonReaderError::Io(
                                JavaException::io("Work response cannot have more than one output")
                                    .into_io_error(),
                            ));
                        }
                        output = Some(reader.next_string()?);
                    }
                    "requestId" => {
                        if request_id.is_some() {
                            return Err(JsonReaderError::Io(
                                JavaException::io(
                                    "Work response cannot have more than one requestId",
                                )
                                .into_io_error(),
                            ));
                        }
                        request_id = Some(reader.next_int()?);
                    }
                    _ => {
                        // As per https://bazel.build/docs/creating-workers#work-responses,
                        // unknown fields are ignored.
                        reader.skip_value()?;
                    }
                }
            }
            reader.end_object()?;
            Ok(())
        })();
        match parsed {
            Ok(()) => {}
            Err(
                e @ (JsonReaderError::Malformed(_)
                | JsonReaderError::Eof(_)
                | JsonReaderError::IllegalState(_)),
            ) => {
                return Err(JavaException::with_cause(
                    "java.io.IOException",
                    "Could not parse json work request correctly",
                    e.to_java(),
                )
                .into_io_error());
            }
            Err(JsonReaderError::Io(e)) => return Err(e),
            Err(JsonReaderError::NumberFormat(m)) => {
                // Not caught by JsonWorkerProtocol: an unchecked exception, which crashes Bazel.
                panic!("java.lang.NumberFormatException: {m}");
            }
        }

        let mut response = WorkResponse::default();
        if let Some(exit_code) = exit_code {
            response.exit_code = exit_code;
        }
        if let Some(output) = output {
            response.output = output;
        }
        if let Some(request_id) = request_id {
            response.request_id = request_id;
        }
        Ok(response)
    }
}

impl WorkerProtocolImpl for JsonWorkerProtocol {
    // Java: com/google/devtools/build/lib/worker/JsonWorkerProtocol.java:49 JsonWorkerProtocol.putRequest
    fn put_request(&self, request: &WorkRequest) -> io::Result<()> {
        // WorkRequests are serialized according to ndjson spec.
        // https://github.com/ndjson/ndjson-spec
        let mut text = String::new();
        JsonFormat::printer()
            .omitting_insignificant_whitespace()
            .append_to(request, &mut text)
            .map_err(|e| JavaException::io(e.get_message()).into_io_error())?;
        text.push('\n');
        let mut json_writer = self.json_writer.lock();
        let Some(writer) = json_writer.as_mut() else {
            return Err(JavaException::io("Stream closed").into_io_error());
        };
        // OutputStreamWriter(workersStdin, UTF_8).
        writer.write_all(text.as_bytes())?;
        writer.flush()
    }

    // Java: com/google/devtools/build/lib/worker/JsonWorkerProtocol.java:58 JsonWorkerProtocol.getResponse
    fn get_response(&self) -> io::Result<Option<WorkResponse>> {
        let interrupted = blaze_shell::thread_interrupt::interrupted();
        let result = self.parse_response();
        if interrupted {
            blaze_shell::thread_interrupt::interrupt_current();
        }
        result.map(Some)
    }

    // Java: com/google/devtools/build/lib/worker/JsonWorkerProtocol.java:120 JsonWorkerProtocol.close
    fn close(&self) -> io::Result<()> {
        // PORT: Java closes the reader first; a reader blocked in a read holds its lock here, so
        // the writer (the worker's stdin) is closed first, which ends such a read.
        // Closing the writer closes the worker's stdin.
        *self.json_writer.lock() = None;
        if let Some(mut reader) = self.reader.try_lock() {
            reader.close();
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::worker::recording_input_stream::ByteArrayInputStream;

    #[test]
    fn parse_delimited_round_trip() {
        let response = WorkResponse {
            exit_code: 1,
            output: "out".to_string(),
            request_id: 42,
            was_cancelled: false,
        };
        let mut buf = Vec::new();
        response.write_delimited_to(&mut buf);
        let mut input = ByteArrayInputStream::new(buf);
        assert_eq!(
            parse_delimited_work_response(&mut input).unwrap(),
            Some(response)
        );
        assert_eq!(parse_delimited_work_response(&mut input).unwrap(), None);
    }

    // From the oracle: a worker that prints text to stdout ("I'm a poisoned worker ...": a
    // length byte 'I', then the tag 0x27, wire type 7) gives the subclass.
    #[test]
    fn parse_delimited_invalid_wire_type_is_the_subclass() {
        let mut input = ByteArrayInputStream::new(
            b"I'm a poisoned worker and this is not a protobuf.\n".to_vec(),
        );
        let e = parse_delimited_work_response(&mut input).unwrap_err();
        let java = JavaException::from_io_error(&e);
        assert_eq!(
            java.to_string(),
            "com.google.protobuf.InvalidProtocolBufferException$InvalidWireTypeException: \
             Protocol message tag had invalid wire type."
        );
    }

    #[test]
    fn parse_delimited_truncated() {
        let mut input = ByteArrayInputStream::new(b"hello".to_vec());
        let e = parse_delimited_work_response(&mut input).unwrap_err();
        assert_eq!(e.to_string(), TRUNCATED_MESSAGE);
        assert_eq!(
            JavaException::from_io_error(&e).class_name,
            "com.google.protobuf.InvalidProtocolBufferException"
        );
    }

    #[test]
    fn json_round_trip() {
        let input = ByteArrayInputStream::new(
            b"{\"exitCode\":2,\"output\":\"x\",\"requestId\":7,\"other\":[1]}\n".to_vec(),
        );
        let p = JsonWorkerProtocol::new(Box::new(Vec::new()), Box::new(input));
        assert_eq!(
            p.get_response().unwrap(),
            Some(WorkResponse {
                exit_code: 2,
                output: "x".to_string(),
                request_id: 7,
                was_cancelled: false,
            })
        );
        let e = p.get_response().unwrap_err();
        let java = JavaException::from_io_error(&e);
        assert_eq!(
            java.to_string(),
            "java.io.IOException: Could not parse json work request correctly"
        );
        // After a first value Gson's doPeek gives PEEKED_EOF (not an EOFException), and
        // beginObject throws unexpectedTokenError.
        let cause = java.cause.unwrap();
        assert_eq!(cause.class_name, "java.lang.IllegalStateException");
        assert!(
            cause.get_message().starts_with(
                "Expected BEGIN_OBJECT but was END_DOCUMENT at line 2 column 1 path $"
            )
        );
    }
}
