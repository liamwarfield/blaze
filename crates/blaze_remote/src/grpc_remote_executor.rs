//! Port of `lib/remote/GrpcRemoteExecutor.java`.
//!
//! PORT: the task description of this file mentions an `Execution` inner class, a keepalive
//! (`--experimental_remote_execution_keepalive`) and a per-call deadline; those belong to
//! `ExperimentalGrpcRemoteExecutor`, which does not exist at the pin. `GrpcRemoteExecutor.java`
//! at the pin is ported here as it is: one `RemoteRetrier` around a loop of Execute /
//! WaitExecution streams, `Utils.refreshIfUnauthenticated` around the retrier, and no deadline on
//! the Execution stubs.

use std::io;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use blaze_protos::JavaMessage;
use parking_lot::Mutex;

use crate::authandtls::CallCredentialsProvider;
use crate::common::errors::{self, StatusRuntimeException, status_to_io_error};
use crate::common::{OperationObserver, RemoteActionExecutionContext, RemoteExecutionClient};
use crate::execution_status_exception::ExecutionStatusException;
use crate::grpc::{CallOptions, ReferenceCountedChannel, ResponseStream, methods};
use crate::reapi::longrunning::{Operation, operation};
use crate::reapi::rpc::Status;
use crate::reapi::{
    ExecuteRequest, ExecuteResponse, RequestMetadata, ServerCapabilities, WaitExecutionRequest,
};
use crate::remote_retrier::RemoteRetrier;
use crate::runtime::get_from_future;
use crate::util::utils;

/// A remote work executor that uses gRPC for communicating the work, inputs and outputs.
///
/// Thread-safe.
// Java: com/google/devtools/build/lib/remote/GrpcRemoteExecutor.java:49 GrpcRemoteExecutor
pub struct GrpcRemoteExecutor {
    channel: ReferenceCountedChannel,
    call_credentials_provider: Arc<dyn CallCredentialsProvider>,
    retrier: Arc<RemoteRetrier>,

    closed: AtomicBool,
}

impl GrpcRemoteExecutor {
    // Java: com/google/devtools/build/lib/remote/GrpcRemoteExecutor.java:57 GrpcRemoteExecutor.<init>
    pub fn new(
        channel: ReferenceCountedChannel,
        call_credentials_provider: Arc<dyn CallCredentialsProvider>,
        retrier: Arc<RemoteRetrier>,
    ) -> GrpcRemoteExecutor {
        GrpcRemoteExecutor {
            channel,
            call_credentials_provider,
            retrier,
            closed: AtomicBool::new(false),
        }
    }

    /// The decoration of `ExecutionGrpc.newBlockingStub(channel)`.
    // Java: com/google/devtools/build/lib/remote/GrpcRemoteExecutor.java:66 GrpcRemoteExecutor.execBlockingStub
    fn exec_blocking_stub(&self, metadata: &RequestMetadata) -> CallOptions {
        CallOptions::new()
            .with_request_metadata(metadata.clone())
            .with_call_credentials(self.call_credentials_provider.get_call_credentials())
    }

    /// PORT: Java throws the unchecked `ExecutionStatusException`; here it is the `Err` of an
    /// `io::Result` (`new IOException(executionStatusException)` once it leaves
    /// [`GrpcRemoteExecutor::execute_remotely`], as Java's final `catch` makes it).
    // Java: com/google/devtools/build/lib/remote/GrpcRemoteExecutor.java:72 GrpcRemoteExecutor.handleStatus
    fn handle_status(status_proto: &Status, resp: Option<&ExecuteResponse>) -> io::Result<()> {
        if status_proto.code == tonic::Code::Ok as i32 {
            return Ok(());
        }
        Err(ExecutionStatusException::new(status_proto.clone(), resp.cloned()).into())
    }

    // Java: com/google/devtools/build/lib/remote/GrpcRemoteExecutor.java:80 GrpcRemoteExecutor.getOperationResponse
    fn get_operation_response(op: &Operation) -> io::Result<Option<ExecuteResponse>> {
        if let Some(operation::Result::Error(error)) = &op.result {
            Self::handle_status(error, None)?;
        }
        if op.done {
            blaze_util::check_state!(
                op.result.is_some(),
                "Unexpected result of remote execution: result not set"
            );
            // `op.getResponse()` is the default `Any` when the result is an error.
            let response = match &op.result {
                Some(operation::Result::Response(any)) => any.clone(),
                _ => Default::default(),
            };
            let resp = unpack_execute_response(&response)?;
            if let Some(status) = &resp.status {
                Self::handle_status(status, Some(&resp))?;
            }
            blaze_util::check_state!(
                resp.result.is_some(),
                "Unexpected result of remote execution: no result"
            );
            return Ok(Some(resp));
        }
        Ok(None)
    }

    // The reply stream must be consumed here, while the pooled connection is still held, so the
    // connection's concurrency permit is not released before the stream completes.
    // Java: com/google/devtools/build/lib/remote/GrpcRemoteExecutor.java:187 GrpcRemoteExecutor.handleOperationStream
    async fn handle_operation_stream(
        &self,
        context: &RemoteActionExecutionContext<'_>,
        request: &ExecuteRequest,
        operation: &Mutex<Operation>,
        wait_execution: &AtomicBool,
        observer: &dyn OperationObserver,
    ) -> io::Result<Option<ExecuteResponse>> {
        // PORT: Java's blocking stub returns a lazy iterator, so a failure to start the call
        // surfaces at the first `replies.hasNext()`, inside the `try` below; `replies` holds that
        // failure here.
        let replies: Result<ResponseStream<Operation>, tonic::Status> =
            if wait_execution.load(Ordering::SeqCst) {
                let wr = WaitExecutionRequest {
                    name: operation.lock().name.clone(),
                };
                self.channel
                    .server_streaming::<WaitExecutionRequest, Operation>(
                        methods::WAIT_EXECUTION,
                        wr,
                        self.exec_blocking_stub(context.get_request_metadata()),
                    )
                    .await
            } else {
                // PORT: skipped, profiling ("send Execute request").
                self.channel
                    .server_streaming::<ExecuteRequest, Operation>(
                        methods::EXECUTE,
                        request.clone(),
                        self.exec_blocking_stub(context.get_request_metadata()),
                    )
                    .await
            };

        let mut replies = match replies {
            Ok(replies) => replies,
            Err(status) => {
                // catch (StatusRuntimeException e)
                let e = status_to_io_error(status);
                Self::on_status_runtime_exception(&e, wait_execution);
                return Err(e);
            }
        };

        let result = Self::consume_replies(&mut replies, operation, wait_execution, observer).await;
        // catch (StatusRuntimeException e)
        if let Err(e) = &result {
            Self::on_status_runtime_exception(e, wait_execution);
        }
        // finally:
        // The blocking streaming call closes correctly only when trailers and a Status are received
        // from the server so that onClose() is called on this call's CallListener. Under normal
        // circumstances (no cancel/errors), these are guaranteed to be sent by the server only if
        // replies.hasNext() has been called after all replies from the stream have been consumed.
        loop {
            match replies.message().await {
                Ok(Some(_)) => {}
                Ok(None) => break,
                // Cleanup: ignore exceptions, because the meaningful errors have already been
                // propagated.
                Err(_) => break,
            }
        }
        result
    }

    /// The body of the `try` of `handleOperationStream`.
    // Java: com/google/devtools/build/lib/remote/GrpcRemoteExecutor.java:207 GrpcRemoteExecutor.handleOperationStream
    async fn consume_replies(
        replies: &mut ResponseStream<Operation>,
        operation: &Mutex<Operation>,
        wait_execution: &AtomicBool,
        observer: &dyn OperationObserver,
    ) -> io::Result<Option<ExecuteResponse>> {
        // while (replies.hasNext())
        while let Some(o) = replies.message().await.map_err(status_to_io_error)? {
            *operation.lock() = o.clone();
            wait_execution.store(!o.done, Ordering::SeqCst);

            // Update execution progress to the caller.
            //
            // After called `execute` above, the action is actually waiting for an available gRPC
            // connection to be sent. Once we get a reply from server, we know the connection is up
            // and indicate to the caller the fact by forwarding the `operation`.
            //
            // The accurate execution status of the action relies on the server implementation:
            //   1. Server can reply the accurate status in `operation.metadata.stage`;
            //   2. Server may send a reply without metadata. In this case, we assume the action is
            //      accepted by the server and will be executed ASAP;
            //   3. Server may execute the action silently and send a reply once it is done.
            observer.on_next(&o)?;

            let r = Self::get_operation_response(&o)?;
            if let Some(r) = r {
                return Ok(Some(r));
            }
        }
        // The operation completed successfully but without a result.
        if !wait_execution.load(Ordering::SeqCst) {
            return Err(errors::io_exception(format!(
                "Remote server error: execution request for {} terminated with no result.",
                operation.lock().name
            )));
        }
        Ok(None)
    }

    /// `catch (StatusRuntimeException e) { if (NOT_FOUND) waitExecution.set(false); throw e; }`.
    ///
    /// PORT: `ExecutionStatusException` is a `StatusRuntimeException` in Java, so it is caught
    /// too; an `IOException` (also one that has a status exception as its cause) is not.
    // Java: com/google/devtools/build/lib/remote/GrpcRemoteExecutor.java:239 GrpcRemoteExecutor.handleOperationStream
    fn on_status_runtime_exception(e: &io::Error, wait_execution: &AtomicBool) {
        let status = match errors::downcast_ref::<ExecutionStatusException>(e) {
            Some(execution_status_exception) => Some(execution_status_exception.get_status()),
            None => errors::downcast_ref::<StatusRuntimeException>(e).map(|e| &e.status),
        };
        if let Some(status) = status
            && status.code() == tonic::Code::NotFound
        {
            // Operation was lost on the server. Retry Execute.
            wait_execution.store(false, Ordering::SeqCst);
        }
    }

    /// `getRetrier()` (package-private in Java).
    // Java: com/google/devtools/build/lib/remote/GrpcRemoteExecutor.java:268 GrpcRemoteExecutor.getRetrier
    pub fn get_retrier(&self) -> &Arc<RemoteRetrier> {
        &self.retrier
    }
}

/// `any.unpack(ExecuteResponse.class)`.
// Java: com/google/protobuf/Any.java Any.unpack
fn unpack_execute_response(
    any: &blaze_protos::google::protobuf::Any,
) -> io::Result<ExecuteResponse> {
    // Java: com/google/protobuf/Any.java Any.getTypeNameFromTypeUrl
    let type_name = match any.type_url.rfind('/') {
        None => "",
        Some(pos) => &any.type_url[pos + 1..],
    };
    if type_name != ExecuteResponse::FULL_NAME {
        // InvalidProtocolBufferException (an IOException).
        return Err(errors::io_exception(
            "Type of the Any message does not match the given class.",
        ));
    }
    // PORT: the message of an InvalidProtocolBufferException from the parser is prost's.
    <ExecuteResponse as prost::Message>::decode(any.value.as_slice())
        .map_err(|e| errors::io_exception(e.to_string()))
}

impl RemoteExecutionClient for GrpcRemoteExecutor {
    // Java: com/google/devtools/build/lib/remote/GrpcRemoteExecutor.java:100 GrpcRemoteExecutor.getServerCapabilities
    fn get_server_capabilities(&self) -> io::Result<ServerCapabilities> {
        self.channel.get_server_capabilities()
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    // Execute has two components: the Execute call and (optionally) the WaitExecution call.
    // This is the simple flow without any errors:
    //
    // - A call to Execute returns streamed updates on an Operation object.
    // - We wait until the Operation is finished.
    //
    // Error possibilities:
    // - An Execute call may fail with a retriable error (raise a StatusRuntimeException).
    //   - If the failure occurred before the first Operation is returned, we retry the call.
    //   - Otherwise, we call WaitExecution on the Operation.
    // - A WaitExecution call may fail with a retriable error (raise a StatusRuntimeException).
    //   In that case, we retry the WaitExecution call on the same operation object.
    // - A WaitExecution call may fail with a NOT_FOUND error (raise a StatusRuntimeException).
    //   That means the Operation was lost on the server, and we will retry to Execute.
    // - Any call can return an Operation object with an error status in the result. Such Operations
    //   are completed and failed; however, some of these errors may be retriable. These errors
    //   should trigger a retry of the Execute call, resulting in a new Operation.
    // Java: com/google/devtools/build/lib/remote/GrpcRemoteExecutor.java:123 GrpcRemoteExecutor.executeRemotely
    fn execute_remotely(
        &self,
        context: &RemoteActionExecutionContext<'_>,
        request: ExecuteRequest,
        observer: &dyn OperationObserver,
    ) -> io::Result<ExecuteResponse> {
        // Will be modified by the retried handler.
        let operation: Mutex<Operation> = Mutex::new(Operation::default());
        // Whether we should call WaitExecution.
        let wait_execution = AtomicBool::new(false);
        // PORT: Java's final `catch (StatusRuntimeException e) { throw new IOException(e); }` is
        // implicit: every status exception already travels as the payload of an `io::Error`.
        utils::refresh_if_unauthenticated(
            || {
                self.retrier.execute(|| {
                    // Retry calls to Execute()/WaitExecute() "infinitely" if the server terminates
                    // one of them status OK and an Operation that does not have done=True set.
                    // This is legal according to the remote execution protocol i.e. if the
                    // execution takes longer than a connection timeout. This is not an error
                    // condition and is thus handled outside of the retrier.
                    loop {
                        // PORT: `channel.withChannelBlocking(channel -> ...)` leases a pooled
                        // connection; the port's channel multiplexes one connection
                        // (grpc/channel.rs), so the call goes straight to it.
                        let response = get_from_future(self.handle_operation_stream(
                            context,
                            &request,
                            &operation,
                            &wait_execution,
                            observer,
                        ))?;
                        if let Some(response) = response {
                            return Ok(response);
                        }
                    }
                })
            },
            &*self.call_credentials_provider,
        )
    }

    // Java: com/google/devtools/build/lib/remote/GrpcRemoteExecutor.java:261 GrpcRemoteExecutor.close
    fn close(&self) {
        if self.closed.swap(true, Ordering::SeqCst) {
            return;
        }
        self.channel.release();
    }
}
