//! Port of `lib/remote/logging/LoggingInterceptor.java`.

use std::any::Any;
use std::sync::Arc;

use blaze_events::ui::Clock;
use blaze_protos::google::protobuf::Timestamp;
use prost::Message as _;
use tonic::metadata::MetadataMap;

use super::logging_handler::{RawLoggingHandler, raw};
use super::{
    ExecuteHandler, FindMissingBlobsHandler, GetActionResultHandler, GetCapabilitiesHandler,
    LogEntrySink, QueryWriteStatusHandler, ReadHandler, SpliceBlobHandler, SplitBlobHandler,
    UpdateActionResultHandler, WaitExecutionHandler, WriteHandler,
};
use crate::grpc::{CallListener, ClientInterceptor, Method, methods};
use crate::reapi::RequestMetadata;
use crate::reapi::remote_logging::LogEntry;
use crate::util::tracing_metadata_utils::METADATA_KEY;

/// An override of [`LoggingInterceptor::select_handler`] (Java: a subclass overriding the
/// `protected selectHandler`, as the tests do).
pub type HandlerSelector = Arc<dyn Fn(&Method) -> Option<Box<dyn RawLoggingHandler>> + Send + Sync>;

/// Client interceptor for logging details of certain gRPC calls.
// Java: com/google/devtools/build/lib/remote/logging/LoggingInterceptor.java:42 LoggingInterceptor
pub struct LoggingInterceptor {
    rpc_log_file: Arc<dyn LogEntrySink>,
    clock: Arc<dyn Clock>,
    /// PORT: the overriding `selectHandler` of a subclass, if any.
    select_handler_override: Option<HandlerSelector>,
}

impl LoggingInterceptor {
    /// Constructs a LoggingInterceptor which logs RPC calls to the given file.
    // Java: com/google/devtools/build/lib/remote/logging/LoggingInterceptor.java:49 LoggingInterceptor.<init>
    pub fn new(rpc_log_file: Arc<dyn LogEntrySink>, clock: Arc<dyn Clock>) -> LoggingInterceptor {
        LoggingInterceptor {
            rpc_log_file,
            clock,
            select_handler_override: None,
        }
    }

    /// A LoggingInterceptor whose `selectHandler` is overridden (Java: an anonymous subclass).
    // Java: com/google/devtools/build/lib/remote/logging/LoggingInterceptor.java:62 LoggingInterceptor.selectHandler
    pub fn with_select_handler(
        rpc_log_file: Arc<dyn LogEntrySink>,
        clock: Arc<dyn Clock>,
        select_handler: HandlerSelector,
    ) -> LoggingInterceptor {
        LoggingInterceptor {
            rpc_log_file,
            clock,
            select_handler_override: Some(select_handler),
        }
    }

    /// Returns a [`RawLoggingHandler`] to handle logging details for the specified method. If
    /// there is no handler for the given method, returns `None`.
    ///
    /// * `method`: Method to return handler for.
    // Java: com/google/devtools/build/lib/remote/logging/LoggingInterceptor.java:62 LoggingInterceptor.selectHandler
    pub fn select_handler(&self, method: &Method) -> Option<Box<dyn RawLoggingHandler>> {
        if let Some(select_handler) = &self.select_handler_override {
            return select_handler(method);
        }
        if *method == methods::EXECUTE {
            return Some(raw(ExecuteHandler::new())); // <ExecuteRequest, Operation>
        } else if *method == methods::WAIT_EXECUTION {
            return Some(raw(WaitExecutionHandler::new())); // <WaitExecutionRequest, Operation>
        } else if *method == methods::GET_ACTION_RESULT {
            return Some(raw(GetActionResultHandler::new())); // <GetActionResultRequest, ActionResult>
        } else if *method == methods::UPDATE_ACTION_RESULT {
            return Some(raw(UpdateActionResultHandler::new())); // <UpdateActionResultRequest, ActionResult>
        } else if *method == methods::FIND_MISSING_BLOBS {
            return Some(raw(FindMissingBlobsHandler::new())); // <FindMissingBlobsRequest, FindMissingBlobsResponse>
        } else if *method == methods::SPLIT_BLOB {
            return Some(raw(SplitBlobHandler::new())); // <SplitBlobRequest, SplitBlobResponse>
        } else if *method == methods::SPLICE_BLOB {
            return Some(raw(SpliceBlobHandler::new())); // <SpliceBlobRequest, SpliceBlobResponse>
        } else if *method == methods::READ {
            return Some(raw(ReadHandler::new())); // <ReadRequest, ReadResponse>
        } else if *method == methods::WRITE {
            return Some(raw(WriteHandler::new())); // <WriteRequest, WriteResponse>
        } else if *method == methods::QUERY_WRITE_STATUS {
            return Some(raw(QueryWriteStatusHandler::new())); // <QueryWriteStatusRequest, QueryWriteStatusResponse>
        } else if *method == methods::GET_CAPABILITIES {
            return Some(raw(GetCapabilitiesHandler::new())); // <GetCapabilitiesRequest, ServerCapabilities>
        }
        None
    }

    /// Get current time as a Timestamp.
    // Java: com/google/devtools/build/lib/remote/logging/LoggingInterceptor.java:103 LoggingInterceptor.getCurrentTimestamp
    fn get_current_timestamp(clock: &dyn Clock) -> Timestamp {
        // Instant.ofEpochMilli(millis): floorDiv / floorMod.
        let millis = clock.current_time_millis();
        Timestamp {
            seconds: millis.div_euclid(1000),
            nanos: (millis.rem_euclid(1000) * 1_000_000) as i32,
        }
    }
}

impl ClientInterceptor for LoggingInterceptor {
    /// PORT: Java wraps the `ClientCall` in a `LoggingForwardingCall`, whose `start` runs when
    /// the call starts; the channel calls this method at that moment and drives the returned
    /// listener for the rest of the call.
    // Java: com/google/devtools/build/lib/remote/logging/LoggingInterceptor.java:90 LoggingInterceptor.interceptCall
    fn intercept_call(
        &self,
        method: &Method,
        headers: &mut MetadataMap,
    ) -> Option<Box<dyn CallListener>> {
        // handler matches method, but that type is inexpressible
        let handler = self.select_handler(method)?;
        let mut call = LoggingForwardingCall::new(
            self.rpc_log_file.clone(),
            self.clock.clone(),
            handler,
            method,
        );
        call.start(headers);
        Some(Box::new(call))
    }
}

/// `TracingMetadataUtils.requestMetadataFromHeaders(headers)`: extracts a [`RequestMetadata`]
/// from the headers of a call. If it does not exist, returns `None`.
// Java: com/google/devtools/build/lib/remote/util/TracingMetadataUtils.java:121 TracingMetadataUtils.requestMetadataFromHeaders
fn request_metadata_from_headers(headers: &MetadataMap) -> Option<RequestMetadata> {
    let value = headers.get_bin(METADATA_KEY)?;
    // `ProtoLiteUtils.MetadataMarshaller.parseBytes` throws an IllegalArgumentException for
    // bytes that are not the message.
    let bytes = value
        .to_bytes()
        .unwrap_or_else(|e| panic!("IllegalArgumentException: {e}"));
    match RequestMetadata::decode(bytes) {
        Ok(metadata) => Some(metadata),
        Err(e) => panic!("IllegalArgumentException: {e}"),
    }
}

/// Wraps client call to log call details by building a [`LogEntry`] and writing it to the RPC
/// log file.
// Java: com/google/devtools/build/lib/remote/logging/LoggingInterceptor.java:115 LoggingInterceptor.LoggingForwardingCall
struct LoggingForwardingCall {
    // PORT: the fields of the enclosing interceptor (a Java inner class reads them from
    // `LoggingInterceptor.this`).
    rpc_log_file: Arc<dyn LogEntrySink>,
    clock: Arc<dyn Clock>,
    handler: Box<dyn RawLoggingHandler>,
    entry_builder: LogEntry,
}

impl LoggingForwardingCall {
    // Java: com/google/devtools/build/lib/remote/logging/LoggingInterceptor.java:120 LoggingForwardingCall.<init>
    fn new(
        rpc_log_file: Arc<dyn LogEntrySink>,
        clock: Arc<dyn Clock>,
        handler: Box<dyn RawLoggingHandler>,
        method: &Method,
    ) -> LoggingForwardingCall {
        LoggingForwardingCall {
            rpc_log_file,
            clock,
            handler,
            entry_builder: LogEntry {
                method_name: method.full_method_name().to_string(),
                ..Default::default()
            },
        }
    }

    // Java: com/google/devtools/build/lib/remote/logging/LoggingInterceptor.java:130 LoggingForwardingCall.start
    fn start(&mut self, headers: &MetadataMap) {
        self.entry_builder.start_time =
            Some(LoggingInterceptor::get_current_timestamp(&*self.clock));
        let metadata = request_metadata_from_headers(headers);
        if let Some(metadata) = metadata {
            self.entry_builder.metadata = Some(metadata);
        }
    }
}

impl CallListener for LoggingForwardingCall {
    // Java: com/google/devtools/build/lib/remote/logging/LoggingInterceptor.java:168 LoggingForwardingCall.sendMessage
    fn on_request(&mut self, message: &(dyn Any + Send)) {
        self.handler.handle_req(message);
    }

    // Java: com/google/devtools/build/lib/remote/logging/LoggingInterceptor.java:140 LoggingForwardingCall.start (Listener.onMessage)
    fn on_response(&mut self, message: &(dyn Any + Send)) {
        self.handler.handle_resp(message);
    }

    /// This method must not throw any exceptions! Doing so will cause the wrapped call to
    /// silently hang indefinitely: https://github.com/grpc/grpc-java/pull/6107
    // Java: com/google/devtools/build/lib/remote/logging/LoggingInterceptor.java:150 LoggingForwardingCall.start (Listener.onClose)
    fn on_close(&mut self, status: &tonic::Status) {
        self.entry_builder.end_time = Some(LoggingInterceptor::get_current_timestamp(&*self.clock));
        self.entry_builder.status = Some(make_status_proto(status));
        self.entry_builder.details = Some(self.handler.get_details());
        // e.g. the log file is already closed.
        // PORT: skipped, logging (`logger.atWarning().withCause(e).log("Unable to write RPC log
        // entry for %s", entryBuilder.build())` goes to the Java log).
        let _ = self.rpc_log_file.write(&self.entry_builder);
    }
}

/// Converts io.grpc.Status to com.google.rpc.Status proto for logging.
///
/// PORT: `status.getCause()` is the source of the `tonic::Status` (a client-side cause; a
/// status received from the server has none) and `Throwable.toString()` is its `Display`.
// Java: com/google/devtools/build/lib/remote/logging/LoggingInterceptor.java:175 LoggingInterceptor.makeStatusProto
fn make_status_proto(status: &tonic::Status) -> crate::reapi::rpc::Status {
    // PORT: a null description is the empty message of a `tonic::Status`.
    let message = match std::error::Error::source(status) {
        Some(cause) => cause.to_string(),
        None => status.message().to_string(),
    };
    crate::reapi::rpc::Status {
        code: status.code() as i32,
        message,
        ..Default::default()
    }
}
