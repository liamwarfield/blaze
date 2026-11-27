//! Port of `lib/worker/JsonWorkerMessageProcessor.java`: the worker side of the JSON worker
//! protocol, for workers written with [`super::work_request_handler::WorkRequestHandler`].

use std::io::{self, Write};

use blaze_protos::blaze::worker::{Input, WorkRequest, WorkResponse};
use blaze_protos::json::{JsonFormat, Printer};
use parking_lot::Mutex;

use super::error_message::JavaException;
use super::json_reader::{JsonReader, JsonReaderError};
use super::work_request_handler::WorkerMessageProcessor;

/// `new IOException(message)` as a reader error.
fn io_error(message: &str) -> JsonReaderError {
    JsonReaderError::Io(JavaException::io(message).into_io_error())
}

/// Implementation of the Worker Protocol using JSON to communicate with Bazel.
// Java: com/google/devtools/build/lib/worker/JsonWorkerMessageProcessor.java:34 JsonWorkerMessageProcessor
pub struct JsonWorkerMessageProcessor {
    /// Reader for reading the WorkResponse.
    reader: Mutex<JsonReader>,
    /// Printer for printing the WorkRequest.
    json_printer: Printer,
    /// Writer for writing the WorkRequest to the worker.
    json_writer: Mutex<Option<Box<dyn Write + Send>>>,
}

impl JsonWorkerMessageProcessor {
    /// Constructs a `JsonWorkerMessageProcessor` that reads and writes JSON.
    ///
    /// PORT: Java takes a Gson `JsonReader` over the worker's stdin and sets it lenient; Blaze's
    /// [`JsonReader`] is always lenient. The writer is Java's `BufferedWriter` (UTF-8).
    // Java: com/google/devtools/build/lib/worker/JsonWorkerMessageProcessor.java:45 JsonWorkerMessageProcessor.<init>
    pub fn new(
        reader: JsonReader,
        json_writer: Box<dyn Write + Send>,
    ) -> JsonWorkerMessageProcessor {
        JsonWorkerMessageProcessor {
            reader: Mutex::new(reader),
            json_printer: JsonFormat::printer()
                .omitting_insignificant_whitespace()
                .always_print_fields_with_no_presence(),
            json_writer: Mutex::new(Some(json_writer)),
        }
    }

    // Java: com/google/devtools/build/lib/worker/JsonWorkerMessageProcessor.java:53 JsonWorkerMessageProcessor.readArguments
    fn read_arguments(reader: &mut JsonReader) -> Result<Vec<String>, JsonReaderError> {
        reader.begin_array()?;
        let mut arguments = Vec::new();
        while reader.has_next()? {
            arguments.push(reader.next_string()?);
        }
        reader.end_array()?;
        Ok(arguments)
    }

    // Java: com/google/devtools/build/lib/worker/JsonWorkerMessageProcessor.java:63 JsonWorkerMessageProcessor.readInputs
    fn read_inputs(reader: &mut JsonReader) -> Result<Vec<Input>, JsonReaderError> {
        reader.begin_array()?;
        let mut inputs = Vec::new();
        while reader.has_next()? {
            let mut digest: Option<String> = None;
            let mut path: Option<String> = None;

            reader.begin_object()?;
            while reader.has_next()? {
                let name = reader.next_name()?;
                match name.as_str() {
                    "digest" => {
                        if digest.is_some() {
                            return Err(io_error("Input cannot have more than one digest"));
                        }
                        digest = Some(reader.next_string()?);
                    }
                    "path" => {
                        if path.is_some() {
                            return Err(io_error("Input cannot have more than one path"));
                        }
                        path = Some(reader.next_string()?);
                    }
                    _ => {
                        // As per https://bazel.build/docs/creating-workers#work-responses,
                        // unknown fields are ignored.
                        reader.skip_value()?;
                    }
                }
            }
            reader.end_object()?;
            let mut input = Input::default();
            if let Some(digest) = digest {
                // ByteString.copyFromUtf8(digest)
                input.digest = digest.into_bytes();
            }
            if let Some(path) = path {
                input.path = path;
            }
            inputs.push(input);
        }
        reader.end_array()?;
        Ok(inputs)
    }
}

impl WorkerMessageProcessor for JsonWorkerMessageProcessor {
    // Java: com/google/devtools/build/lib/worker/JsonWorkerMessageProcessor.java:105 JsonWorkerMessageProcessor.readWorkRequest
    fn read_work_request(&self) -> io::Result<Option<WorkRequest>> {
        let mut arguments: Option<Vec<String>> = None;
        let mut inputs: Option<Vec<Input>> = None;
        let mut request_id: Option<i32> = None;
        let mut verbosity: Option<i32> = None;
        let mut sandbox_dir: Option<String> = None;
        let mut reader = self.reader.lock();
        let reader = &mut *reader;
        let parsed: Result<(), JsonReaderError> = (|| {
            reader.begin_object()?;
            while reader.has_next()? {
                let name = reader.next_name()?;
                match name.as_str() {
                    "arguments" => {
                        if arguments.is_some() {
                            return Err(io_error(
                                "WorkRequest cannot have more than one 'arguments' field",
                            ));
                        }
                        arguments = Some(Self::read_arguments(reader)?);
                    }
                    "inputs" => {
                        if inputs.is_some() {
                            return Err(io_error(
                                "WorkRequest cannot have more than one 'inputs' field",
                            ));
                        }
                        inputs = Some(Self::read_inputs(reader)?);
                    }
                    "requestId" => {
                        if request_id.is_some() {
                            return Err(io_error(
                                "WorkRequest cannot have more than one requestId",
                            ));
                        }
                        request_id = Some(reader.next_int()?);
                    }
                    "verbosity" => {
                        if verbosity.is_some() {
                            return Err(io_error(
                                "Work response cannot have more than one verbosity",
                            ));
                        }
                        verbosity = Some(reader.next_int()?);
                    }
                    "sandboxDir" => {
                        if sandbox_dir.is_some() {
                            return Err(io_error(
                                "Work response cannot have more than one sandboxDir",
                            ));
                        }
                        sandbox_dir = Some(reader.next_string()?);
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
            // catch (MalformedJsonException | IllegalStateException | EOFException e) {
            //   throw new IOException(e);
            Err(
                e @ (JsonReaderError::Malformed(_)
                | JsonReaderError::Eof(_)
                | JsonReaderError::IllegalState(_)),
            ) => {
                let cause = e.to_java();
                return Err(JavaException::with_cause(
                    "java.io.IOException",
                    cause.to_string(),
                    cause,
                )
                .into_io_error());
            }
            Err(JsonReaderError::Io(e)) => return Err(e),
            Err(JsonReaderError::NumberFormat(m)) => {
                // Not caught: an unchecked exception.
                panic!("java.lang.NumberFormatException: {m}");
            }
        }

        let mut request = WorkRequest::default();
        if let Some(arguments) = arguments {
            request.arguments.extend(arguments);
        }
        if let Some(inputs) = inputs {
            request.inputs.extend(inputs);
        }
        if let Some(request_id) = request_id {
            request.request_id = request_id;
        }
        if let Some(verbosity) = verbosity {
            request.verbosity = verbosity;
        }
        if let Some(sandbox_dir) = sandbox_dir {
            request.sandbox_dir = sandbox_dir;
        }
        Ok(Some(request))
    }

    // Java: com/google/devtools/build/lib/worker/JsonWorkerMessageProcessor.java:174 JsonWorkerMessageProcessor.writeWorkResponse
    fn write_work_response(&self, response: &WorkResponse) -> io::Result<()> {
        let mut text = String::new();
        self.json_printer
            .append_to(response, &mut text)
            .map_err(|e| JavaException::io(e.get_message()).into_io_error())?;
        let mut json_writer = self.json_writer.lock();
        let Some(writer) = json_writer.as_mut() else {
            return Err(JavaException::io("Stream closed").into_io_error());
        };
        writer.write_all(text.as_bytes())?;
        writer.flush()
    }

    // Java: com/google/devtools/build/lib/worker/JsonWorkerMessageProcessor.java:180 JsonWorkerMessageProcessor.close
    fn close(&self) -> io::Result<()> {
        // BufferedWriter.close flushes, then closes the stream.
        if let Some(mut writer) = self.json_writer.lock().take() {
            writer.flush()?;
        }
        Ok(())
    }
}
