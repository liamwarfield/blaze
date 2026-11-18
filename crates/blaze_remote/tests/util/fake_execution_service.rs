//! Port of the test class `lib/remote/FakeExecutionService.java`.
//!
//! PORT: the service is registered in a fake server before the test programs it (the Rust fake
//! servers fix their services when they start), so it is a cheap handle over shared state:
//! register a clone (`service_registry.add_execution(service.clone())`) and program the
//! original.
#![allow(dead_code)]

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::sync::atomic::{AtomicI32, Ordering};

use blaze_protos::JavaMessage;
use blaze_remote::grpc::stubs::execution_server::Execution;
use blaze_remote::reapi::longrunning::{Operation, operation};
use blaze_remote::reapi::rpc;
use blaze_remote::reapi::{ExecuteRequest, ExecuteResponse, WaitExecutionRequest};
use parking_lot::Mutex;
use tonic::{Request, Response, Status};

use super::grpc_server::{ResponseStream, response_stream};

/// One reply of a call: `Supplier<Operation>` that returns an Operation or throws.
#[derive(Clone)]
enum OperationSupplier {
    /// `() -> operation`.
    Operation(Operation),
    /// `() -> { throw e; }`: the server's `onError(e)`.
    ///
    /// PORT: the exception is the `io.grpc.Status` the client receives for it
    /// (`Status.fromThrowable(e)` on the server).
    Error(Status),
}

// Java: com/google/devtools/build/lib/remote/FakeExecutionService.java:42 FakeExecutionService.OperationProvider
#[derive(Default)]
struct OperationProvider {
    // Map from the request to the list of operations to be returned for each instance of
    // that request, with Supplier used for either throwing an exception or returning an Operation.
    operation_provider: HashMap<String, VecDeque<Vec<OperationSupplier>>>,
}

impl OperationProvider {
    // Java: com/google/devtools/build/lib/remote/FakeExecutionService.java:49 FakeExecutionService.OperationProvider.append
    fn append(&mut self, name: String, suppliers: Vec<OperationSupplier>) {
        self.operation_provider
            .entry(name)
            .or_default()
            .push_back(suppliers);
    }

    // Java: com/google/devtools/build/lib/remote/FakeExecutionService.java:55 FakeExecutionService.OperationProvider.hasNext
    fn has_next(&self, name: &str) -> bool {
        match self.operation_provider.get(name) {
            Some(q) => !q.is_empty(),
            None => false,
        }
    }

    // Java: com/google/devtools/build/lib/remote/FakeExecutionService.java:60 FakeExecutionService.OperationProvider.next
    fn next(&mut self, name: &str) -> Vec<OperationSupplier> {
        self.operation_provider
            .get_mut(name)
            .and_then(|q| q.pop_front())
            .expect("NoSuchElementException")
    }
}

struct Inner {
    execute_operation_provider: Mutex<OperationProvider>,
    wait_execution_operation_provider: Mutex<OperationProvider>,
    exec_times: AtomicI32,
    wait_times: AtomicI32,
}

/// A fake implementation of the `ExecutionImplBase`.
// Java: com/google/devtools/build/lib/remote/FakeExecutionService.java:36 FakeExecutionService
#[derive(Clone)]
pub struct FakeExecutionService {
    inner: Arc<Inner>,
}

impl Default for FakeExecutionService {
    fn default() -> Self {
        FakeExecutionService::new()
    }
}

/// Which provider a builder appends to.
#[derive(Clone, Copy)]
enum ProviderKind {
    Execute,
    WaitExecution,
}

impl FakeExecutionService {
    /// `new FakeExecutionService()`.
    pub fn new() -> FakeExecutionService {
        FakeExecutionService {
            inner: Arc::new(Inner {
                execute_operation_provider: Mutex::new(OperationProvider::default()),
                wait_execution_operation_provider: Mutex::new(OperationProvider::default()),
                exec_times: AtomicI32::new(0),
                wait_times: AtomicI32::new(0),
            }),
        }
    }

    fn provider(&self, kind: ProviderKind) -> &Mutex<OperationProvider> {
        match kind {
            ProviderKind::Execute => &self.inner.execute_operation_provider,
            ProviderKind::WaitExecution => &self.inner.wait_execution_operation_provider,
        }
    }

    // Java: com/google/devtools/build/lib/remote/FakeExecutionService.java:65 FakeExecutionService.whenExecute
    pub fn when_execute(&self, request: &ExecuteRequest) -> OnetimeOperationSupplierBuilder {
        OnetimeOperationSupplierBuilder::new(self.clone(), ProviderKind::Execute, request.clone())
    }

    // Java: com/google/devtools/build/lib/remote/FakeExecutionService.java:69 FakeExecutionService.whenWaitExecution
    pub fn when_wait_execution(&self, request: &ExecuteRequest) -> OnetimeOperationSupplierBuilder {
        OnetimeOperationSupplierBuilder::new(
            self.clone(),
            ProviderKind::WaitExecution,
            request.clone(),
        )
    }

    // Java: com/google/devtools/build/lib/remote/FakeExecutionService.java:73 FakeExecutionService.ackOperation
    pub fn ack_operation(request: &ExecuteRequest) -> Operation {
        Operation {
            name: Self::get_resource_name(request),
            done: false,
            ..Default::default()
        }
    }

    // Java: com/google/devtools/build/lib/remote/FakeExecutionService.java:77 FakeExecutionService.doneOperation
    pub fn done_operation(request: &ExecuteRequest, response: &ExecuteResponse) -> Operation {
        Operation {
            name: Self::get_resource_name(request),
            done: true,
            result: Some(operation::Result::Response(any_pack(response))),
            ..Default::default()
        }
    }

    // Java: com/google/devtools/build/lib/remote/FakeExecutionService.java:155 FakeExecutionService.getResourceName
    pub fn get_resource_name(request: &ExecuteRequest) -> String {
        format!(
            "operations/{}",
            request
                .action_digest
                .as_ref()
                .map(|d| d.hash.as_str())
                .unwrap_or("")
        )
    }

    // Java: com/google/devtools/build/lib/remote/FakeExecutionService.java:172 FakeExecutionService.serve
    fn serve(
        name: &str,
        provider: &Mutex<OperationProvider>,
    ) -> Result<Response<ResponseStream<Operation>>, Status> {
        let suppliers = {
            let mut provider = provider.lock();
            if !provider.has_next(name) {
                return Err(Status::unimplemented(""));
            }
            provider.next(name)
        };
        let mut items: Vec<Result<Operation, Status>> = Vec::new();
        for supplier in suppliers {
            match supplier {
                OperationSupplier::Operation(operation) => items.push(Ok(operation)),
                OperationSupplier::Error(e) => {
                    // PORT: Java keeps calling `onNext` on the closed call (which throws);
                    // the stream ends at the first `onError`.
                    items.push(Err(e));
                    break;
                }
            }
        }
        // `onCompleted()` when nothing was thrown: the end of the stream.
        Ok(Response::new(response_stream(items)))
    }

    // Java: com/google/devtools/build/lib/remote/FakeExecutionService.java:185 FakeExecutionService.getExecTimes
    pub fn get_exec_times(&self) -> i32 {
        self.inner.exec_times.load(Ordering::SeqCst)
    }

    // Java: com/google/devtools/build/lib/remote/FakeExecutionService.java:189 FakeExecutionService.getWaitTimes
    pub fn get_wait_times(&self) -> i32 {
        self.inner.wait_times.load(Ordering::SeqCst)
    }
}

/// `Any.pack(message)`.
// Java: com/google/protobuf/Any.java Any.pack
pub fn any_pack<M: JavaMessage>(message: &M) -> blaze_protos::google::protobuf::Any {
    blaze_protos::google::protobuf::Any {
        type_url: format!("type.googleapis.com/{}", M::FULL_NAME),
        value: message.to_byte_array(),
    }
}

// Java: com/google/devtools/build/lib/remote/FakeExecutionService.java:84 FakeExecutionService.OnetimeOperationSupplierBuilder
pub struct OnetimeOperationSupplierBuilder {
    service: FakeExecutionService,
    provider: ProviderKind,
    request: ExecuteRequest,
    operations: Vec<OperationSupplier>,
}

impl OnetimeOperationSupplierBuilder {
    // Java: com/google/devtools/build/lib/remote/FakeExecutionService.java:90 FakeExecutionService.OnetimeOperationSupplierBuilder.<init>
    fn new(
        service: FakeExecutionService,
        provider: ProviderKind,
        request: ExecuteRequest,
    ) -> OnetimeOperationSupplierBuilder {
        OnetimeOperationSupplierBuilder {
            service,
            provider,
            request,
            operations: Vec::new(),
        }
    }

    // Java: com/google/devtools/build/lib/remote/FakeExecutionService.java:96 FakeExecutionService.OnetimeOperationSupplierBuilder.thenAck
    pub fn then_ack(mut self) -> OnetimeOperationSupplierBuilder {
        let operation = FakeExecutionService::ack_operation(&self.request);
        self.operations
            .push(OperationSupplier::Operation(operation));
        self
    }

    /// `thenDone()`.
    // Java: com/google/devtools/build/lib/remote/FakeExecutionService.java:102 FakeExecutionService.OnetimeOperationSupplierBuilder.thenDone
    pub fn then_done_without_response(mut self) {
        let operation = Operation {
            name: FakeExecutionService::get_resource_name(&self.request),
            done: true,
            ..Default::default()
        };
        self.operations
            .push(OperationSupplier::Operation(operation));
        self.finish();
    }

    /// `thenDone(response)`.
    // Java: com/google/devtools/build/lib/remote/FakeExecutionService.java:109 FakeExecutionService.OnetimeOperationSupplierBuilder.thenDone
    pub fn then_done(mut self, response: &ExecuteResponse) {
        let operation = FakeExecutionService::done_operation(&self.request, response);
        self.operations
            .push(OperationSupplier::Operation(operation));
        self.finish();
    }

    /// `thenError(com.google.rpc.Code code)`.
    // Java: com/google/devtools/build/lib/remote/FakeExecutionService.java:115 FakeExecutionService.OnetimeOperationSupplierBuilder.thenError
    pub fn then_error(mut self, code: tonic::Code) {
        // From REAPI Spec:
        // > Errors discovered during creation of the `Operation` will be reported
        // > as gRPC Status errors, while errors that occurred while running the
        // > action will be reported in the `status` field of the `ExecuteResponse`. The
        // > server MUST NOT set the `error` field of the `Operation` proto.
        let operation = FakeExecutionService::done_operation(
            &self.request,
            &ExecuteResponse {
                status: Some(rpc::Status {
                    code: code as i32,
                    ..Default::default()
                }),
                ..Default::default()
            },
        );
        self.operations
            .push(OperationSupplier::Operation(operation));
        self.finish();
    }

    /// `thenError(RuntimeException e)`: `e` is the status the client receives
    /// (`Status.X.asRuntimeException()` is that status; any other exception is
    /// `Status.UNKNOWN`).
    // Java: com/google/devtools/build/lib/remote/FakeExecutionService.java:130 FakeExecutionService.OnetimeOperationSupplierBuilder.thenError
    pub fn then_error_exception(mut self, e: Status) {
        self.operations.push(OperationSupplier::Error(e));
        self.finish();
    }

    // Java: com/google/devtools/build/lib/remote/FakeExecutionService.java:138 FakeExecutionService.OnetimeOperationSupplierBuilder.finish
    pub fn finish(self) {
        let name = FakeExecutionService::get_resource_name(&self.request);
        self.service
            .provider(self.provider)
            .lock()
            .append(name, self.operations);
    }
}

#[tonic::async_trait]
impl Execution for FakeExecutionService {
    type ExecuteStream = ResponseStream<Operation>;
    type WaitExecutionStream = ResponseStream<Operation>;

    // Java: com/google/devtools/build/lib/remote/FakeExecutionService.java:159 FakeExecutionService.execute
    async fn execute(
        &self,
        request: Request<ExecuteRequest>,
    ) -> Result<Response<Self::ExecuteStream>, Status> {
        self.inner.exec_times.fetch_add(1, Ordering::SeqCst);
        let name = FakeExecutionService::get_resource_name(request.get_ref());
        FakeExecutionService::serve(&name, &self.inner.execute_operation_provider)
    }

    // Java: com/google/devtools/build/lib/remote/FakeExecutionService.java:165 FakeExecutionService.waitExecution
    async fn wait_execution(
        &self,
        request: Request<WaitExecutionRequest>,
    ) -> Result<Response<Self::WaitExecutionStream>, Status> {
        self.inner.wait_times.fetch_add(1, Ordering::SeqCst);
        FakeExecutionService::serve(
            &request.get_ref().name,
            &self.inner.wait_execution_operation_provider,
        )
    }
}
