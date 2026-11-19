#![allow(clippy::field_reassign_with_default)]
//! Port of
//! `src/test/java/com/google/devtools/build/lib/remote/RemoteSpawnRunnerWithGrpcRemoteExecutorTest.java`.
//!
//! Java runs `RemoteSpawnRunner` + `GrpcRemoteExecutor` + `GrpcCacheClient` against an in-process
//! gRPC server on an `InMemoryFileSystem`. Here:
//! - the server is a fake server on localhost (`util::grpc_server`). The services of a test are
//!   registered first and the server is started (`t.fake_server.start()`) before the spawn runs;
//!   Java's anonymous `*ImplBase` subclasses are the `Fake*` services with handlers, and Java's
//!   Mockito spies (`spy(ExecutionImplBase.class)` with `doAnswer(...).doAnswer(...)`) are
//!   handlers that answer the i-th call with the i-th answer (the last one repeats, as Mockito
//!   does) and count the calls for the `verify(..., times(n))` checks;
//! - Java's `ServerInterceptor`s (`RequestHeadersValidator`, the header check of
//!   `extraHeaders`) run inside the handlers, on the request's metadata;
//! - the tests run on the real file system under a temporary directory;
//! - the mocked `RemoteOutputChecker` ("download nothing by default") is a real
//!   `RemoteOutputChecker` in `MINIMAL` mode without patterns; the tests that stub
//!   `shouldDownloadOutput(execPath, any())` to return true use one in `ALL` mode
//!   ([`Fixture::set_up_downloading_outputs`]). The `RemoteActionFileSystem` of `getSpawnContext`
//!   is the `RemoteOutputInjector` hook ([`RecordingInjector`]);
//! - Java's stack trace checks (`--verbose_failures` appends
//!   `Throwables.getStackTraceAsString`) have no counterpart: Rust has no stack traces and the
//!   port appends none (`util/utils.rs`, `grpc_aware_error_message`). Those assertions are left
//!   out, each with a `PORT:` comment.

mod util;

use std::io;
use std::sync::Arc;
use std::sync::atomic::{AtomicI32, Ordering};
use std::time::SystemTime;

use async_trait::async_trait;
use blaze_cmdline::Label;
use blaze_collect::{NestedSetBuilder, Order};
use blaze_exec::actions::{
    ActionExecutionMetadata, ActionInputRef, BasicActionInput, FakeOwner, PathMapper, ResourceSet,
    SimpleSpawn, Spawn, Status as SpawnStatus,
};
use blaze_exec::exec::SpawnRunner;
use blaze_exec::util::FileOutErr;
use blaze_optdefs::command::execution_options::ExecutionOptions;
use blaze_optdefs::misc::remote_options::RemoteOptions;
use blaze_optdefs::misc::remote_outputs_mode::RemoteOutputsMode;
use blaze_protos::JavaMessage;
use blaze_protos::failure_details::{self, failure_detail};
use blaze_remote::api_version::ApiVersion;
use blaze_remote::authandtls::google_auth_utils::new_call_credentials_provider;
use blaze_remote::authandtls::{NO_CREDENTIALS, new_exec_headers_interceptor};
use blaze_remote::chunker::Chunker;
use blaze_remote::common::remote_path_resolver::create_default;
use blaze_remote::common::{RemoteCacheClient, RemoteExecutionClient};
use blaze_remote::execution_status_exception;
use blaze_remote::grpc::channel::CapabilitiesFetcher;
use blaze_remote::grpc::stubs::execution_server::Execution;
use blaze_remote::grpc::{ClientInterceptor, ReferenceCountedChannel};
use blaze_remote::grpc_cache_client::GrpcCacheClient;
use blaze_remote::grpc_remote_executor::GrpcRemoteExecutor;
use blaze_remote::hooks::{RemoteHooks, RemoteOutputInjector};
use blaze_remote::reapi::bytestream::{
    QueryWriteStatusRequest, QueryWriteStatusResponse, ReadRequest, ReadResponse, WriteRequest,
    WriteResponse,
};
use blaze_remote::reapi::longrunning::{Operation, operation};
use blaze_remote::reapi::rpc::{self, PreconditionFailure, precondition_failure::Violation};
use blaze_remote::reapi::{
    ActionResult, Command, Digest, Directory, ExecuteRequest, ExecuteResponse,
    ExecutionCapabilities, FileNode, FindMissingBlobsRequest, FindMissingBlobsResponse,
    GetActionResultRequest, OutputDirectory, OutputFile, RequestMetadata, ServerCapabilities, Tree,
    WaitExecutionRequest, command, digest_function,
};
use blaze_remote::remote_execution_cache::RemoteExecutionCache;
use blaze_remote::remote_execution_service::RemoteExecutionService;
use blaze_remote::remote_output_checker::RemoteOutputChecker;
use blaze_remote::remote_retrier::{ExponentialBackoff, GRPC_RESULT_CLASSIFIER, RemoteRetrier};
use blaze_remote::remote_spawn_runner::RemoteSpawnRunner;
use blaze_remote::retrier::{
    ALLOW_ALL_CALLS, Backoff, BackoffSupplier, RejectedExecutionException, ResultClassifier,
    ResultClassifierResult, RetryScheduler, TokioRetryScheduler,
};
use blaze_remote::util::DigestUtil;
use blaze_remote::util::temp_path_generator::TempPathGenerator;
use blaze_remote::util::tracing_metadata_utils::{self, METADATA_KEY};
use blaze_vfs::{DigestHashFunction, Path, PathFragment, UnixFileSystem};
use futures::future::BoxFuture;
use indexmap::IndexMap;
use parking_lot::Mutex;
use prost::Message as _;
use tonic::metadata::MetadataMap;
use tonic::{Request, Response, Status, Streaming};

use util::fake_action_input_file_cache::FakeActionInputFileCache;
use util::fake_execution_service::any_pack;
use util::fake_spawn_execution_context::FakeSpawnExecutionContext;
use util::grpc_server::{
    FakeActionCache, FakeByteStream, FakeContentAddressableStorage, FakeServer, ResponseStream,
    hanging_response_stream, read_response, response_stream,
};

/// `TestConstants.WORKSPACE_NAME`.
const WORKSPACE_NAME: &str = "_main";

/// `BlazeVersionInfo.instance().getVersion()` of the test process (an unstamped build).
const BLAZE_VERSION: &str = "";

/// `DIGEST_UTIL`.
fn digest_util() -> DigestUtil {
    DigestUtil::new(DigestHashFunction::Sha256)
}

/// `DUMMY_OUTPUT`.
fn dummy_output() -> OutputFile {
    OutputFile {
        path: "dummy.txt".to_string(),
        digest: Some(Digest {
            hash: "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855".to_string(),
            size_bytes: 0,
        }),
        ..Default::default()
    }
}

/// `DUMMY_OUTPUT_TREE`.
fn dummy_output_tree() -> Tree {
    Tree {
        root: Some(Directory {
            files: vec![FileNode {
                name: dummy_output().path,
                digest: dummy_output().digest,
                is_executable: true,
                ..Default::default()
            }],
            ..Default::default()
        }),
        ..Default::default()
    }
}

/// `DUMMY_OUTPUT_DIRECTORY`.
fn dummy_output_directory() -> OutputDirectory {
    OutputDirectory {
        path: "dummy".to_string(),
        tree_digest: Some(digest_util().compute_message(&dummy_output_tree())),
        ..Default::default()
    }
}

/// `Operation.newBuilder().setDone(true).setResponse(Any.pack(response))` (with `name`).
fn done_operation(name: &str, response: &ExecuteResponse) -> Operation {
    Operation {
        name: name.to_string(),
        done: true,
        result: Some(operation::Result::Response(any_pack(response))),
        ..Default::default()
    }
}

/// `ExecuteResponse.newBuilder().setResult(actionResult).build()`.
fn execute_response(action_result: &ActionResult) -> ExecuteResponse {
    ExecuteResponse {
        result: Some(action_result.clone()),
        ..Default::default()
    }
}

/// `Status.X.asRuntimeException()` (no description).
fn status(code: tonic::Code) -> Status {
    Status::new(code, "")
}

// ---------------------------------------------------------------------------
// Fake services
// ---------------------------------------------------------------------------

type ExecuteHandler =
    Box<dyn Fn(Request<ExecuteRequest>) -> ResponseStream<Operation> + Send + Sync + 'static>;
type WaitExecutionHandler =
    Box<dyn Fn(Request<WaitExecutionRequest>) -> ResponseStream<Operation> + Send + Sync + 'static>;

/// `new ExecutionImplBase() {...}`: a method without a handler answers UNIMPLEMENTED.
#[derive(Default)]
struct FakeExecution {
    execute: Option<ExecuteHandler>,
    wait_execution: Option<WaitExecutionHandler>,
}

#[tonic::async_trait]
impl Execution for FakeExecution {
    type ExecuteStream = ResponseStream<Operation>;
    type WaitExecutionStream = ResponseStream<Operation>;

    async fn execute(
        &self,
        request: Request<ExecuteRequest>,
    ) -> Result<Response<Self::ExecuteStream>, Status> {
        match &self.execute {
            Some(handler) => Ok(Response::new(handler(request))),
            None => Err(Status::unimplemented(
                "Method build.bazel.remote.execution.v2.Execution/Execute is unimplemented",
            )),
        }
    }

    async fn wait_execution(
        &self,
        request: Request<WaitExecutionRequest>,
    ) -> Result<Response<Self::WaitExecutionStream>, Status> {
        match &self.wait_execution {
            Some(handler) => Ok(Response::new(handler(request))),
            None => Err(Status::unimplemented(
                "Method build.bazel.remote.execution.v2.Execution/WaitExecution is unimplemented",
            )),
        }
    }
}

/// `new ExecutionImplBase() { execute(...) }`.
fn execution_service(
    execute: impl Fn(Request<ExecuteRequest>) -> ResponseStream<Operation> + Send + Sync + 'static,
) -> FakeExecution {
    FakeExecution {
        execute: Some(Box::new(execute)),
        wait_execution: None,
    }
}

/// `new ActionCacheImplBase() { getActionResult(...) }`.
fn action_cache(
    get_action_result: impl Fn(Request<GetActionResultRequest>) -> Result<ActionResult, Status>
    + Send
    + Sync
    + 'static,
) -> FakeActionCache {
    FakeActionCache {
        get_action_result: Some(Box::new(get_action_result)),
        ..Default::default()
    }
}

/// `new ContentAddressableStorageImplBase() { findMissingBlobs(...) }`.
fn cas(
    find_missing_blobs: impl Fn(
        Request<FindMissingBlobsRequest>,
    ) -> Result<FindMissingBlobsResponse, Status>
    + Send
    + Sync
    + 'static,
) -> FakeContentAddressableStorage {
    FakeContentAddressableStorage {
        find_missing_blobs: Some(Box::new(find_missing_blobs)),
        ..Default::default()
    }
}

/// `findMissingBlobs` answering `FindMissingBlobsResponse.getDefaultInstance()`: nothing is
/// missing.
fn nothing_missing(
    _request: Request<FindMissingBlobsRequest>,
) -> Result<FindMissingBlobsResponse, Status> {
    Ok(FindMissingBlobsResponse::default())
}

/// The future of a `ChannelConnectionWithServerCapabilitiesFactory`.
type CapabilitiesFuture = BoxFuture<'static, io::Result<ServerCapabilities>>;

/// The future of a `ByteStream.Write` handler.
type WriteFuture = BoxFuture<'static, Result<WriteResponse, Status>>;

/// A `ByteStream.Write` handler.
type WriteAnswer = Box<dyn Fn(Request<Streaming<WriteRequest>>) -> WriteFuture + Send + Sync>;

/// A `ByteStream.Read` handler.
type ReadAnswer = Box<dyn Fn(Request<ReadRequest>) -> ResponseStream<ReadResponse> + Send + Sync>;

/// An `Execute` / `WaitExecution` handler.
type OperationAnswer<Req> = Box<dyn Fn(Request<Req>) -> ResponseStream<Operation> + Send + Sync>;

/// `doAnswer(answers[0]).doAnswer(answers[1])...` on a spy: the i-th call gets the i-th answer,
/// every later call the last one. `calls` counts the calls (`verify(spy, times(n))`).
fn in_sequence<A: 'static, R: 'static>(
    calls: Arc<AtomicI32>,
    answers: Vec<Box<dyn Fn(A) -> R + Send + Sync>>,
) -> impl Fn(A) -> R + Send + Sync + 'static {
    move |a| {
        let i = calls.fetch_add(1, Ordering::SeqCst) as usize;
        let answer = &answers[i.min(answers.len() - 1)];
        answer(a)
    }
}

// Java: RemoteSpawnRunnerWithGrpcRemoteExecutorTest.answerWith
fn answer_with<Req: 'static>(op: Option<Operation>, status: Status) -> OperationAnswer<Req> {
    Box::new(move |_request: Request<Req>| {
        let mut items: Vec<Result<Operation, Status>> = Vec::new();
        if let Some(op) = &op {
            // responseObserver.onNext(op)
            items.push(Ok(op.clone()));
        }
        if status.code() != tonic::Code::Ok {
            // responseObserver.onError(status.asRuntimeException())
            items.push(Err(status.clone()));
        }
        // else responseObserver.onCompleted()
        response_stream(items)
    })
}

// Java: RemoteSpawnRunnerWithGrpcRemoteExecutorTest.blobWriteAnswer
fn blob_write_answer(data: &'static [u8]) -> WriteAnswer {
    let digest = digest_util().compute(data);
    Box::new(
        move |request: Request<Streaming<WriteRequest>>| -> WriteFuture {
            let digest = digest.clone();
            Box::pin(async move {
                let mut requests = request.into_inner();
                let mut response = None;
                // onNext(request)
                while let Some(request) = requests.message().await? {
                    assert!(
                        request
                            .resource_name
                            .contains(&DigestUtil::to_string(&digest)),
                        "{}",
                        request.resource_name
                    );
                    assert!(request.finish_write);
                    assert_eq!(&request.data[..], data);
                    response = Some(WriteResponse {
                        committed_size: request.data.len() as i64,
                    });
                }
                // onCompleted(): responseObserver.onCompleted()
                // PORT: Java answers with onNext(response) as soon as the request arrives; here the
                // response is returned when the client half-closes.
                Ok(response.unwrap_or_default())
            })
        },
    )
}

// Java: RemoteSpawnRunnerWithGrpcRemoteExecutorTest.blobWriteAnswerError
fn blob_write_answer_error() -> WriteAnswer {
    Box::new(|request: Request<Streaming<WriteRequest>>| -> WriteFuture {
        Box::pin(async move {
            let mut requests = request.into_inner();
            // onNext(request): responseObserver.onError(Status.UNAVAILABLE.asRuntimeException())
            requests.message().await?;
            Err(status(tonic::Code::Unavailable))
        })
    })
}

/// The `write` of the anonymous `ByteStreamImplBase`s that accept every write: `onNext` does
/// nothing, `onCompleted` completes the call.
fn accept_all_writes() -> WriteAnswer {
    Box::new(|request: Request<Streaming<WriteRequest>>| -> WriteFuture {
        Box::pin(async move {
            let mut requests = request.into_inner();
            while requests.message().await?.is_some() {}
            // PORT: Java completes without a response; a tonic handler must return one. No test
            // that uses this writes anything (nothing is missing).
            Ok(WriteResponse::default())
        })
    })
}

/// `TracingMetadataUtils.METADATA_KEY` of the request headers.
fn request_metadata(headers: &MetadataMap) -> RequestMetadata {
    let bytes = headers
        .get_bin(METADATA_KEY)
        .expect("no RequestMetadata header")
        .to_bytes()
        .expect("invalid RequestMetadata header");
    RequestMetadata::decode(bytes).expect("invalid RequestMetadata")
}

/// `RequestHeadersValidator`: capture the request headers from a client. Useful for testing
/// metadata propagation.
// Java: RemoteSpawnRunnerWithGrpcRemoteExecutorTest.RequestHeadersValidator.interceptCall
fn validate_request_headers(headers: &MetadataMap) {
    let meta = request_metadata(headers);
    assert_eq!(meta.correlated_invocations_id, "build-req-id");
    assert_eq!(meta.tool_invocation_id, "command-id");
    assert!(!meta.action_id.is_empty());
    let tool_details = meta.tool_details.unwrap_or_default();
    assert_eq!(tool_details.tool_name, "bazel");
    assert_eq!(tool_details.tool_version, BLAZE_VERSION);
}

// ---------------------------------------------------------------------------
// Fixture
// ---------------------------------------------------------------------------

/// `TestUtils.ZeroDelayListeningScheduledExecutorService`: schedules all tasks with zero delay.
// Java: com/google/devtools/build/lib/remote/util/TestUtils.java:69 TestUtils.ZeroDelayListeningScheduledExecutorService
struct ZeroDelayRetryScheduler;

#[async_trait]
impl RetryScheduler for ZeroDelayRetryScheduler {
    async fn schedule(&self, _delay_millis: i64) -> Result<(), RejectedExecutionException> {
        Ok(())
    }
}

/// `TestUtils.newRemoteRetrier(backoff, resultClassifier, retryScheduler)`.
// Java: com/google/devtools/build/lib/remote/util/TestUtils.java:46 TestUtils.newRemoteRetrier
fn new_remote_retrier(
    backoff: BackoffSupplier,
    result_classifier: Arc<dyn ResultClassifier>,
) -> Arc<RemoteRetrier> {
    Arc::new(RemoteRetrier::with_sleeper(
        backoff,
        Arc::new(move |e: &io::Error| {
            // `Status.fromThrowable(e)`: an ExecutionStatusException is a
            // StatusRuntimeException in Java.
            if execution_status_exception::status_from_throwable(e).map(|s| s.code())
                == Some(tonic::Code::Cancelled)
            {
                ResultClassifierResult::Success
            } else {
                result_classifier.test(e)
            }
        }),
        Arc::new(ZeroDelayRetryScheduler),
        ALLOW_ALL_CALLS.clone(),
        Arc::new(|_millis: i64| -> io::Result<()> {
            /* don't wait in tests */
            Ok(())
        }),
    ))
}

/// The `RemoteActionFileSystem` of `getSpawnContext`: accepts the injected outputs.
#[derive(Default)]
struct RecordingInjector {
    injected: Mutex<Vec<PathFragment>>,
}

impl RemoteOutputInjector for RecordingInjector {
    fn inject_remote_file(
        &self,
        path: &PathFragment,
        _digest: Vec<u8>,
        _size: i64,
        _expiration_time: SystemTime,
        _in_memory_output: bool,
    ) -> io::Result<()> {
        self.injected.lock().push(path.clone());
        Ok(())
    }
}

/// The fields of the Java test class, as `setUp` leaves them.
struct Fixture {
    _dir: tempfile::TempDir,
    /// `fakeServer` with its `serviceRegistry`.
    fake_server: FakeServer,
    exec_root: Path,
    simple_spawn: Arc<dyn Spawn>,
    fake_file_cache: Arc<FakeActionInputFileCache>,
    input_digest: Digest,
    cmd_digest: Digest,
    #[allow(dead_code)]
    command: Command,
    client: RemoteSpawnRunner,
    out_err: Arc<FileOutErr>,
    remote_options: RemoteOptions,
}

impl Fixture {
    // Java: RemoteSpawnRunnerWithGrpcRemoteExecutorTest.setUp
    fn set_up() -> Fixture {
        // remoteOutputChecker: download nothing by default.
        Fixture::set_up_with_outputs_mode(RemoteOutputsMode::MINIMAL)
    }

    /// `setUp` followed by
    /// `when(remoteOutputChecker.shouldDownloadOutput(execPath, any())).thenReturn(true)`.
    fn set_up_downloading_outputs() -> Fixture {
        Fixture::set_up_with_outputs_mode(RemoteOutputsMode::ALL)
    }

    fn set_up_with_outputs_mode(outputs_mode: RemoteOutputsMode) -> Fixture {
        // PORT: Java reads the version from BlazeVersionInfo.
        tracing_metadata_utils::set_tool_version(BLAZE_VERSION);
        // Use a mutable service registry for later registering the service impl for each test
        // case.
        let fake_server = FakeServer::new();

        Chunker::set_default_chunk_size_for_testing(1000); // Enough for everything to be one chunk.
        let dir = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(dir.path())
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();
        let fs = UnixFileSystem::create(DigestHashFunction::Sha256);
        let exec_root = Path::create_from_str(&format!("{root}/execroot/main"), &fs);
        exec_root.create_directory_and_parents().unwrap();
        // artifactRoot = ArtifactRoot.asDerivedRoot(execRoot, RootType.OUTPUT, "outputs");
        // PORT: the artifact root is only used by the RemoteActionFileSystem of getSpawnContext,
        // which is the RemoteOutputInjector hook here; its directory is created as in Java.
        exec_root
            .get_relative_str("outputs")
            .create_directory_and_parents()
            .unwrap();
        let temp_path_generator = Arc::new(TempPathGenerator::new(Path::create_from_str(
            &format!("{root}/execroot/_tmp/actions/remote"),
            &fs,
        )));
        let log_dir = Path::create_from_str(&format!("{root}/server-logs"), &fs);
        let fake_file_cache = Arc::new(FakeActionInputFileCache::new(exec_root.clone()));
        let input = ActionInputRef::new(BasicActionInput::from_path("input"));
        let mut inputs = NestedSetBuilder::stable_order();
        inputs.add(input.clone());
        let owner: Arc<dyn ActionExecutionMetadata> = Arc::new(FakeOwner::new(
            "Mnemonic",
            Some("Progress Message"),
            Some(Label::parse_canonical_unchecked("//dummy:label")),
        ));
        // Java: the 9-argument constructor (SimpleSpawn.java:72) with `tools` and
        // `mandatoryOutputs`.
        let simple_spawn: Arc<dyn Spawn> = Arc::new(SimpleSpawn::new(
            owner,
            vec!["/bin/echo".to_string(), "Hi!".to_string()],
            IndexMap::from([("VARIABLE".to_string(), "value".to_string())]),
            /* executionInfo= */ IndexMap::new(),
            /* inputs= */ inputs.build(),
            /* tools= */ NestedSetBuilder::empty_set(Order::STABLE_ORDER),
            // PORT: Java's anonymous ActionInputs with the exec paths "foo" and "bar" (neither
            // a directory nor a symlink).
            /* outputs= */
            vec![
                ActionInputRef::new(BasicActionInput::from_path("foo")),
                ActionInputRef::new(BasicActionInput::from_path("bar")),
            ],
            // `ImmutableSet.of()`: no output is mandatory (`SimpleSpawn.isMandatoryOutput`,
            // SimpleSpawn.java:146), unlike `null`, which makes every output mandatory.
            /* mandatoryOutputs= */
            Some(std::collections::HashSet::new()),
            Arc::new(ResourceSet::zero()),
            PathMapper::NOOP,
        ));

        let stdout = Path::create_from_str(&format!("{root}/tmp/stdout"), &fs);
        let stderr = Path::create_from_str(&format!("{root}/tmp/stderr"), &fs);
        stdout
            .get_parent_directory()
            .unwrap()
            .create_directory_and_parents()
            .unwrap();
        stderr
            .get_parent_directory()
            .unwrap()
            .create_directory_and_parents()
            .unwrap();
        let out_err = Arc::new(FileOutErr::new(stdout, stderr));
        let mut remote_options = RemoteOptions::default();

        remote_options.remote_headers = vec![
            ("CommonKey1".to_string(), "CommonValue1".to_string()),
            ("CommonKey2".to_string(), "CommonValue2".to_string()),
        ];
        remote_options.remote_exec_headers = vec![
            ("ExecKey1".to_string(), "ExecValue1".to_string()),
            ("ExecKey2".to_string(), "ExecValue2".to_string()),
        ];
        remote_options.remote_cache_headers = vec![
            ("CacheKey1".to_string(), "CacheValue1".to_string()),
            ("CacheKey2".to_string(), "CacheValue2".to_string()),
        ];

        let backoff_options = remote_options.clone();
        let retrier = new_remote_retrier(
            Arc::new(move || -> Arc<dyn Backoff> {
                Arc::new(ExponentialBackoff::from_options(&backoff_options))
            }),
            GRPC_RESULT_CLASSIFIER.clone(),
        );
        // InProcessChannelBuilder.forName(fakeServerName)
        //     .intercept(TracingMetadataUtils.newExecHeadersInterceptor(remoteOptions))
        let interceptors: Vec<Arc<dyn ClientInterceptor>> =
            vec![Arc::new(new_exec_headers_interceptor(&remote_options))];
        let capabilities_fetcher: CapabilitiesFetcher =
            Arc::new(|_channel: ReferenceCountedChannel| -> CapabilitiesFuture {
                Box::pin(async {
                    Ok(ServerCapabilities {
                        low_api_version: Some(ApiVersion::LOW.to_sem_ver()),
                        high_api_version: Some(ApiVersion::HIGH.to_sem_ver()),
                        execution_capabilities: Some(ExecutionCapabilities {
                            exec_enabled: true,
                            ..Default::default()
                        }),
                        ..Default::default()
                    })
                })
            });
        let channel = fake_server.channel(interceptors, Some(capabilities_fetcher));

        let executor: Arc<dyn RemoteExecutionClient> = Arc::new(GrpcRemoteExecutor::new(
            channel.retain(),
            NO_CREDENTIALS.clone(),
            retrier.clone(),
        ));
        let call_credentials_provider = new_call_credentials_provider(None);
        let cache_protocol: Arc<dyn RemoteCacheClient> = Arc::new(GrpcCacheClient::new(
            channel.retain(),
            call_credentials_provider,
            &remote_options,
            retrier,
            digest_util(),
        ));
        let remote_cache = Arc::new(RemoteExecutionCache::new(
            cache_protocol,
            /* diskCacheClient= */ None,
            /* symlinkTemplate= */ None,
            digest_util(),
            /* chunkingFunction= */ None,
        ));
        let shared_remote_options = Arc::new(remote_options.clone());
        let remote_execution_service = RemoteExecutionService::new(
            Arc::new(blaze_events::StoredEventHandler::new()),
            /* verbose_failures= */ true,
            exec_root.clone(),
            create_default(exec_root.clone()),
            "build-req-id",
            "command-id",
            WORKSPACE_NAME,
            digest_util(),
            shared_remote_options.clone(),
            Arc::new(ExecutionOptions::default()),
            /* combined_cache= */ None,
            Some(remote_cache),
            Some(executor),
            temp_path_generator,
            /* capture_corrupted_outputs_dir= */ None,
            Some(Arc::new(RemoteOutputChecker::new(
                "build",
                outputs_mode,
                Vec::new(),
            ))),
            Arc::new(Mutex::new(std::collections::HashSet::new())),
            /* scrubber= */ None,
        )
        // PORT: the RemoteActionFileSystem of getSpawnContext, and `/* wasRewound= */ action ->
        // false` is the default of the service.
        .with_hooks(RemoteHooks {
            remote_output_injector: Some(
                Arc::new(RecordingInjector::default()) as Arc<dyn RemoteOutputInjector>
            ),
        });
        let client = RemoteSpawnRunner::new(
            shared_remote_options,
            /* verbose_failures= */ true,
            /* cmdline_reporter= */ None,
            // retryService
            Arc::new(TokioRetryScheduler),
            log_dir,
            Arc::new(remote_execution_service),
            digest_util(),
        );

        let input_digest = fake_file_cache.create_scratch_input(&input, "xyz").unwrap();
        let command = Command {
            arguments: vec!["/bin/echo".to_string(), "Hi!".to_string()],
            environment_variables: vec![command::EnvironmentVariable {
                name: "VARIABLE".to_string(),
                value: "value".to_string(),
            }],
            output_paths: vec!["bar".to_string(), "foo".to_string()],
            ..Default::default()
        };
        let cmd_digest = digest_util().compute_message(&command);
        channel.release();

        Fixture {
            _dir: dir,
            fake_server,
            exec_root,
            simple_spawn,
            fake_file_cache,
            input_digest,
            cmd_digest,
            command,
            client,
            out_err,
            remote_options,
        }
    }

    // Java: RemoteSpawnRunnerWithGrpcRemoteExecutorTest.getSpawnContext
    fn get_spawn_context(&self, spawn: &Arc<dyn Spawn>) -> FakeSpawnExecutionContext {
        // PORT: the RemoteActionInputFetcher and RemoteActionFileSystem of Java's context are the
        // RemoteOutputInjector hook of the service (see set_up).
        FakeSpawnExecutionContext::new(
            spawn.clone(),
            self.fake_file_cache.clone(),
            self.exec_root.clone(),
            self.out_err.clone(),
        )
    }

    /// `getResourceName(remoteOptions.remoteInstanceName, digest, false, SHA256)`.
    fn resource_name(&self, digest: &Digest) -> String {
        GrpcCacheClient::get_resource_name(
            &self.remote_options.remote_instance_name,
            digest,
            false,
            digest_function::Value::Sha256,
        )
    }
}

impl Drop for Fixture {
    // Java: RemoteSpawnRunnerWithGrpcRemoteExecutorTest.tearDown
    fn drop(&mut self) {
        self.fake_server.shutdown_now();
    }
}

/// `new FakeImmutableCacheByteStreamImpl(d1, c1, d2, c2)`: serves a fixed set of blobs, each
/// only after a number of errors.
// Java: com/google/devtools/build/lib/remote/FakeImmutableCacheByteStreamImpl.java:30 FakeImmutableCacheByteStreamImpl
fn fake_immutable_cache_byte_stream_impl(contents: Vec<(Digest, &str)>) -> FakeByteStream {
    // Start returning the correct response after this number of errors is reached.
    const MAX_ERRORS: i32 = 3;
    let canned_replies: Vec<(ReadRequest, ReadResponse)> = contents
        .into_iter()
        .map(|(digest, data)| {
            (
                ReadRequest {
                    resource_name: format!("blobs/{}/{}", digest.hash, digest.size_bytes),
                    ..Default::default()
                },
                read_response(data),
            )
        })
        .collect();
    let num_errors: Mutex<Vec<i32>> = Mutex::new(vec![0; canned_replies.len()]);
    FakeByteStream {
        read: Some(Box::new(move |request: Request<ReadRequest>| {
            let request = request.into_inner();
            // assertThat(cannedReplies.keySet()).contains(request);
            let index = canned_replies
                .iter()
                .position(|(canned, _)| *canned == request)
                .unwrap_or_else(|| panic!("unexpected request {request:?}"));
            let mut num_errors = num_errors.lock();
            if num_errors[index] < MAX_ERRORS {
                num_errors[index] += 1;
                // Retriable error.
                response_stream(vec![Err(status(tonic::Code::Unavailable))])
            } else {
                response_stream(vec![Ok(canned_replies[index].1.clone())])
            }
        })),
        ..Default::default()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[test]
fn cache_hit() {
    let mut t = Fixture::set_up();
    t.fake_server
        .service_registry
        .add_action_cache(action_cache(|_| Ok(ActionResult::default())));
    t.fake_server.start();

    let context = t.get_spawn_context(&t.simple_spawn);

    let result = t.client.exec(&*t.simple_spawn, &context).unwrap();
    assert!(result.setup_success(), "{result:?}");
    assert!(result.is_cache_hit());
    assert_eq!(result.exit_code(), 0);
    assert!(!t.out_err.has_recorded_output());
    assert!(!t.out_err.has_recorded_stderr());
}

#[test]
fn failed_action() {
    let mut t = Fixture::set_up();
    t.fake_server
        .service_registry
        .add_action_cache(action_cache(|_| Err(status(tonic::Code::NotFound))));
    let action_result = ActionResult {
        exit_code: 1,
        ..Default::default()
    };
    t.fake_server
        .service_registry
        .add_execution(execution_service(move |_| {
            response_stream(vec![Ok(done_operation(
                "",
                &execute_response(&action_result),
            ))])
        }));
    t.fake_server
        .service_registry
        .add_content_addressable_storage(cas(nothing_missing));
    t.fake_server.start();

    let context = t.get_spawn_context(&t.simple_spawn);
    let result = t.client.exec(&*t.simple_spawn, &context).unwrap();
    assert_eq!(result.exit_code(), 1);
}

#[test]
fn no_outputs() {
    let mut t = Fixture::set_up();
    t.fake_server
        .service_registry
        .add_action_cache(action_cache(|_| Err(status(tonic::Code::NotFound))));
    let action_result = ActionResult::default();
    t.fake_server
        .service_registry
        .add_execution(execution_service(move |_| {
            response_stream(vec![Ok(done_operation(
                "",
                &execute_response(&action_result),
            ))])
        }));
    t.fake_server
        .service_registry
        .add_content_addressable_storage(cas(nothing_missing));
    t.fake_server.start();

    let context = t.get_spawn_context(&t.simple_spawn);
    t.client.exec(&*t.simple_spawn, &context).unwrap();
}

#[test]
fn cache_hit_with_output() {
    let mut t = Fixture::set_up();
    let std_out_digest = digest_util().compute_as_utf8("stdout");
    let std_err_digest = digest_util().compute_as_utf8("stderr");
    {
        let std_out_digest = std_out_digest.clone();
        let std_err_digest = std_err_digest.clone();
        t.fake_server
            .service_registry
            .add_action_cache(action_cache(move |_| {
                Ok(ActionResult {
                    output_files: vec![dummy_output()],
                    stdout_digest: Some(std_out_digest.clone()),
                    stderr_digest: Some(std_err_digest.clone()),
                    ..Default::default()
                })
            }));
    }
    t.fake_server
        .service_registry
        .add_byte_stream(fake_immutable_cache_byte_stream_impl(vec![
            (std_out_digest, "stdout"),
            (std_err_digest, "stderr"),
        ]));
    t.fake_server.start();

    let context = t.get_spawn_context(&t.simple_spawn);
    let result = t.client.exec(&*t.simple_spawn, &context).unwrap();

    assert!(result.setup_success(), "{result:?}");
    assert_eq!(result.exit_code(), 0);
    assert!(result.is_cache_hit());
    assert_eq!(t.out_err.out_as_latin1(), "stdout");
    assert_eq!(t.out_err.err_as_latin1(), "stderr");
}

#[test]
fn cache_hit_with_inline_output() {
    let mut t = Fixture::set_up();
    t.fake_server
        .service_registry
        .add_action_cache(action_cache(|_| {
            Ok(ActionResult {
                output_files: vec![dummy_output()],
                stdout_raw: b"stdout".to_vec(),
                stderr_raw: b"stderr".to_vec(),
                ..Default::default()
            })
        }));
    t.fake_server.start();

    let context = t.get_spawn_context(&t.simple_spawn);
    let result = t.client.exec(&*t.simple_spawn, &context).unwrap();

    assert!(result.setup_success(), "{result:?}");
    assert_eq!(result.exit_code(), 0);
    assert!(result.is_cache_hit());
    assert_eq!(t.out_err.out_as_latin1(), "stdout");
    assert_eq!(t.out_err.err_as_latin1(), "stderr");
}

#[test]
fn extra_headers() {
    let mut t = Fixture::set_up();
    t.fake_server
        .service_registry
        .add_action_cache(action_cache(|_| Err(status(tonic::Code::NotFound))));

    t.fake_server
        .service_registry
        .add_content_addressable_storage(cas(nothing_missing));

    // PORT: the ServerInterceptor of the Java test asserts on the server thread; here the
    // handler records the headers of each Execute call and the test asserts on them below.
    let exec_headers: Arc<Mutex<Vec<MetadataMap>>> = Arc::new(Mutex::new(Vec::new()));
    {
        let exec_headers = exec_headers.clone();
        t.fake_server
            .service_registry
            .add_execution(execution_service(move |request| {
                exec_headers.lock().push(request.metadata().clone());
                response_stream(vec![Ok(Operation::default())])
            }));
    }
    t.fake_server.start();

    let context = t.get_spawn_context(&t.simple_spawn);
    t.client.exec(&*t.simple_spawn, &context).unwrap();

    let exec_headers = exec_headers.lock();
    assert!(!exec_headers.is_empty(), "Execute was not called");
    // PORT: gRPC header names are lower case on the wire (Java's Metadata.Key.of lower-cases
    // them too).
    for metadata in exec_headers.iter() {
        let get = |key: &str| metadata.get(key).map(|v| v.to_str().unwrap().to_string());
        assert_eq!(get("commonkey1").as_deref(), Some("CommonValue1"));
        assert_eq!(get("commonkey2").as_deref(), Some("CommonValue2"));
        assert_eq!(get("execkey1").as_deref(), Some("ExecValue1"));
        assert_eq!(get("execkey2").as_deref(), Some("ExecValue2"));
        assert_eq!(get("cachekey1"), None);
        assert_eq!(get("cachekey2"), None);
    }
}

#[test]
fn remotely_execute() {
    let mut t = Fixture::set_up();
    t.fake_server
        .service_registry
        .add_action_cache(action_cache(|request| {
            validate_request_headers(request.metadata());
            Err(status(tonic::Code::NotFound))
        }));
    let action_result = ActionResult {
        output_files: vec![dummy_output()],
        stdout_raw: b"stdout".to_vec(),
        stderr_raw: b"stderr".to_vec(),
        ..Default::default()
    };
    t.fake_server
        .service_registry
        .add_execution(execution_service(move |request| {
            validate_request_headers(request.metadata());
            response_stream(vec![Ok(done_operation(
                "",
                &execute_response(&action_result),
            ))])
        }));
    {
        let cmd_digest = t.cmd_digest.clone();
        let input_digest = t.input_digest.clone();
        t.fake_server
            .service_registry
            .add_content_addressable_storage(cas(move |request| {
                validate_request_headers(request.metadata());
                let requested = &request.get_ref().blob_digests;
                assert!(requested.contains(&cmd_digest));
                assert!(requested.contains(&input_digest));
                Ok(FindMissingBlobsResponse {
                    missing_blob_digests: vec![input_digest.clone()],
                })
            }));
    }

    let write_calls = Arc::new(AtomicI32::new(0));
    {
        let write_answer = blob_write_answer(b"xyz");
        let write_calls = write_calls.clone();
        t.fake_server
            .service_registry
            .add_byte_stream(FakeByteStream {
                write: Some(Box::new(
                    move |request: Request<Streaming<WriteRequest>>| -> WriteFuture {
                        write_calls.fetch_add(1, Ordering::SeqCst);
                        validate_request_headers(request.metadata());
                        write_answer(request)
                    },
                )),
                ..Default::default()
            });
    }
    t.fake_server.start();

    let context = t.get_spawn_context(&t.simple_spawn);

    let result = t.client.exec(&*t.simple_spawn, &context).unwrap();
    assert!(result.setup_success(), "{result:?}");
    assert_eq!(result.exit_code(), 0);
    assert!(!result.is_cache_hit());
    assert_eq!(t.out_err.out_as_latin1(), "stdout");
    assert_eq!(t.out_err.err_as_latin1(), "stderr");
    // verify(mockByteStreamImpl).write(...)
    assert_eq!(write_calls.load(Ordering::SeqCst), 1);
}

#[test]
fn remotely_execute_retries() {
    // when(remoteOutputChecker.shouldDownloadOutput(execPath, any())).thenReturn(true);
    let mut t = Fixture::set_up_downloading_outputs();

    {
        let num_errors = AtomicI32::new(4);
        t.fake_server
            .service_registry
            .add_action_cache(action_cache(move |_| {
                // (numErrors-- <= 0 ? Status.NOT_FOUND : Status.UNAVAILABLE)
                if num_errors.fetch_sub(1, Ordering::SeqCst) <= 0 {
                    Err(status(tonic::Code::NotFound))
                } else {
                    Err(status(tonic::Code::Unavailable))
                }
            }));
    }
    let result_digest = digest_util().compute(b"bla");
    let action_result = ActionResult {
        stdout_raw: b"stdout".to_vec(),
        stderr_raw: b"stderr".to_vec(),
        output_files: vec![OutputFile {
            path: "foo".to_string(),
            digest: Some(result_digest),
            ..Default::default()
        }],
        ..Default::default()
    };
    let op_name = "operations/xyz";

    let execute_response_with_error = ExecuteResponse {
        status: Some(rpc::Status {
            code: tonic::Code::Internal as i32,
            ..Default::default()
        }),
        ..Default::default()
    };
    let operation_with_execute_error = done_operation(op_name, &execute_response_with_error);
    let unfinished_operation = Operation {
        name: op_name.to_string(),
        ..Default::default()
    };
    let op_success = done_operation(op_name, &execute_response(&action_result));

    let execute_calls = Arc::new(AtomicI32::new(0));
    let wait_execution_calls = Arc::new(AtomicI32::new(0));
    // Flow of this test:
    // - call execute, get retriable gRPC error
    // - retry: call execute, get retriable Operation error
    // - retry: call execute, get an Operation, then a retriable gRPC error
    // - retry: call waitExecute, get a retriable gRPC error
    // - retry: call waitExecute, get retriable Operation error
    // - retry: call execute, get successful operation, ignore further errors.
    t.fake_server.service_registry.add_execution(FakeExecution {
        execute: Some(Box::new(in_sequence(
            execute_calls.clone(),
            vec![
                answer_with::<ExecuteRequest>(None, status(tonic::Code::Unavailable)),
                answer_with::<ExecuteRequest>(
                    Some(operation_with_execute_error.clone()),
                    status(tonic::Code::Ok),
                ),
                answer_with::<ExecuteRequest>(
                    Some(unfinished_operation.clone()),
                    status(tonic::Code::Unavailable),
                ),
                // last status should be ignored.
                answer_with::<ExecuteRequest>(Some(op_success), status(tonic::Code::Unavailable)),
            ],
        ))),
        wait_execution: Some(Box::new(in_sequence(
            wait_execution_calls.clone(),
            vec![
                answer_with::<WaitExecutionRequest>(None, status(tonic::Code::Unavailable)),
                answer_with::<WaitExecutionRequest>(
                    Some(operation_with_execute_error),
                    status(tonic::Code::Ok),
                ),
            ],
        ))),
    });

    {
        let num_errors = AtomicI32::new(4);
        let cmd_digest = t.cmd_digest.clone();
        let input_digest = t.input_digest.clone();
        t.fake_server
            .service_registry
            .add_content_addressable_storage(cas(move |request| {
                if num_errors.fetch_sub(1, Ordering::SeqCst) > 0 {
                    return Err(status(tonic::Code::Unavailable));
                }

                let requested = &request.get_ref().blob_digests;
                assert!(requested.contains(&cmd_digest));
                assert!(requested.contains(&input_digest));
                Ok(FindMissingBlobsResponse {
                    missing_blob_digests: vec![input_digest.clone()],
                })
            }));
    }

    let write_calls = Arc::new(AtomicI32::new(0));
    let read_calls = Arc::new(AtomicI32::new(0));
    t.fake_server
        .service_registry
        .add_byte_stream(FakeByteStream {
            write: Some(Box::new(in_sequence(
                write_calls.clone(),
                vec![
                    blob_write_answer_error(), // Error on the input file.
                    blob_write_answer_error(), // Error on the input file again.
                    blob_write_answer(b"xyz"), // Upload input file successfully.
                ],
            ))),
            query_write_status: Some(Box::new(
                |_: Request<QueryWriteStatusRequest>| -> Result<QueryWriteStatusResponse, Status> {
                    Ok(QueryWriteStatusResponse {
                        committed_size: 0,
                        complete: false,
                    })
                },
            )),
            read: Some(Box::new(in_sequence(
                read_calls.clone(),
                vec![
                    Box::new(|_: Request<ReadRequest>| {
                        // Will retry.
                        response_stream(vec![Err(status(tonic::Code::Internal))])
                    }) as ReadAnswer,
                    Box::new(|_: Request<ReadRequest>| {
                        response_stream(vec![Ok(read_response("bla"))])
                    }),
                ],
            ))),
        });
    t.fake_server.start();

    let context = t.get_spawn_context(&t.simple_spawn);
    let result = t.client.exec(&*t.simple_spawn, &context).unwrap();

    assert!(result.setup_success(), "{result:?}");
    assert_eq!(result.exit_code(), 0);
    assert!(!result.is_cache_hit());
    assert_eq!(t.out_err.out_as_latin1(), "stdout");
    assert_eq!(t.out_err.err_as_latin1(), "stderr");
    assert_eq!(execute_calls.load(Ordering::SeqCst), 4);
    assert_eq!(wait_execution_calls.load(Ordering::SeqCst), 2);
    assert_eq!(read_calls.load(Ordering::SeqCst), 2);
    assert_eq!(write_calls.load(Ordering::SeqCst), 3);
}

#[test]
fn remotely_execute_retries_wait_result() {
    // when(remoteOutputChecker.shouldDownloadOutput(execPath, any())).thenReturn(true);
    let mut t = Fixture::set_up_downloading_outputs();

    // This test's flow is similar to the previous, except the result
    // will eventually be returned by the waitExecute function.
    t.fake_server
        .service_registry
        .add_action_cache(action_cache(|_| Err(status(tonic::Code::NotFound))));
    let result_digest = digest_util().compute(b"bla");
    let action_result = ActionResult {
        stdout_raw: b"stdout".to_vec(),
        stderr_raw: b"stderr".to_vec(),
        output_files: vec![OutputFile {
            path: "foo".to_string(),
            digest: Some(result_digest),
            ..Default::default()
        }],
        ..Default::default()
    };
    let op_name = "operations/xyz";

    let unfinished_operation = Operation {
        name: op_name.to_string(),
        ..Default::default()
    };
    let op_success = done_operation(op_name, &execute_response(&action_result));

    let execute_calls = Arc::new(AtomicI32::new(0));
    let wait_execution_calls = Arc::new(AtomicI32::new(0));
    // Flow of this test:
    // - call execute, get an Operation, then a retriable gRPC error
    // - retry: call waitExecute, get NOT_FOUND (operation lost)
    // - retry: call execute, get NOT_FOUND (operation lost)
    // - retry: call execute, get an Operation, then a retriable gRPC error
    // - retry: call waitExecute, get successful operation, ignore further errors.
    t.fake_server.service_registry.add_execution(FakeExecution {
        execute: Some(Box::new(in_sequence(
            execute_calls.clone(),
            vec![
                answer_with::<ExecuteRequest>(
                    Some(unfinished_operation.clone()),
                    status(tonic::Code::Unavailable),
                ),
                answer_with::<ExecuteRequest>(
                    Some(unfinished_operation.clone()),
                    status(tonic::Code::NotFound),
                ),
                answer_with::<ExecuteRequest>(
                    Some(unfinished_operation.clone()),
                    status(tonic::Code::Unavailable),
                ),
            ],
        ))),
        wait_execution: Some(Box::new(in_sequence(
            wait_execution_calls.clone(),
            vec![
                answer_with::<WaitExecutionRequest>(
                    Some(unfinished_operation),
                    status(tonic::Code::NotFound),
                ),
                // This error is ignored.
                answer_with::<WaitExecutionRequest>(
                    Some(op_success),
                    status(tonic::Code::Unavailable),
                ),
            ],
        ))),
    });

    {
        let cmd_digest = t.cmd_digest.clone();
        let input_digest = t.input_digest.clone();
        t.fake_server
            .service_registry
            .add_content_addressable_storage(cas(move |request| {
                let requested = &request.get_ref().blob_digests;
                assert!(requested.contains(&cmd_digest));
                assert!(requested.contains(&input_digest));
                Ok(FindMissingBlobsResponse {
                    missing_blob_digests: vec![input_digest.clone()],
                })
            }));
    }

    let write_calls = Arc::new(AtomicI32::new(0));
    let read_calls = Arc::new(AtomicI32::new(0));
    t.fake_server
        .service_registry
        .add_byte_stream(FakeByteStream {
            write: Some(Box::new(in_sequence(
                write_calls.clone(),
                vec![blob_write_answer(b"xyz")],
            ))),
            read: Some(Box::new(in_sequence(
                read_calls.clone(),
                vec![Box::new(|_: Request<ReadRequest>| {
                    response_stream(vec![Ok(read_response("bla"))])
                }) as ReadAnswer],
            ))),
            ..Default::default()
        });
    t.fake_server.start();

    let context = t.get_spawn_context(&t.simple_spawn);
    let result = t.client.exec(&*t.simple_spawn, &context).unwrap();

    assert!(result.setup_success(), "{result:?}");
    assert_eq!(result.exit_code(), 0);
    assert!(!result.is_cache_hit());
    assert_eq!(t.out_err.out_as_latin1(), "stdout");
    assert_eq!(t.out_err.err_as_latin1(), "stderr");
    assert_eq!(execute_calls.load(Ordering::SeqCst), 3);
    assert_eq!(wait_execution_calls.load(Ordering::SeqCst), 2);
    assert_eq!(read_calls.load(Ordering::SeqCst), 1);
    assert_eq!(write_calls.load(Ordering::SeqCst), 1);
}

#[test]
fn pass_unavailable_error_with_stack_trace() {
    let mut t = Fixture::set_up();
    t.fake_server
        .service_registry
        .add_action_cache(action_cache(|_| Err(status(tonic::Code::Unavailable))));
    t.fake_server.start();

    let context = t.get_spawn_context(&t.simple_spawn);
    let result = t.client.exec(&*t.simple_spawn, &context).unwrap();

    assert_eq!(
        result.status(),
        SpawnStatus::ExecutionFailedCatastrophically
    );
    // Ensure we also got back the stack trace due to verboseFailures=true
    // PORT: `assertThat(result.getFailureMessage()).contains("com.google.devtools.build.lib.remote")`
    // has no counterpart (no stack traces, see the file comment).
}

#[test]
fn pass_internal_error_with_stack_trace() {
    let mut t = Fixture::set_up();
    t.fake_server
        .service_registry
        .add_action_cache(action_cache(|_| Err(Status::internal("whoa"))));
    t.fake_server.start();

    let context = t.get_spawn_context(&t.simple_spawn);
    let result = t.client.exec(&*t.simple_spawn, &context).unwrap();

    assert!(
        result.get_failure_message().contains("whoa"),
        "{}",
        result.get_failure_message()
    ); // Error details.
    // Ensure we also got back the stack trace due to verboseFailures=true
    // PORT: `assertThat(result.getFailureMessage()).contains("com.google.devtools.build.lib.remote")`
    // has no counterpart (no stack traces, see the file comment).
}

#[test]
fn pass_cache_miss_error_with_stack_trace() {
    let mut t = Fixture::set_up();
    t.fake_server
        .service_registry
        .add_action_cache(action_cache(|_| Err(status(tonic::Code::NotFound))));
    let std_out_digest = digest_util().compute_as_utf8("bla");
    let action_result = ActionResult {
        output_files: vec![dummy_output()],
        stdout_digest: Some(std_out_digest.clone()),
        ..Default::default()
    };
    t.fake_server
        .service_registry
        .add_execution(execution_service(move |_| {
            response_stream(vec![Ok(done_operation(
                "",
                &execute_response(&action_result),
            ))])
        }));
    t.fake_server
        .service_registry
        .add_content_addressable_storage(cas(nothing_missing));
    let std_out_resource_name = t.resource_name(&std_out_digest);
    t.fake_server
        .service_registry
        .add_byte_stream(FakeByteStream {
            read: Some(Box::new(move |request: Request<ReadRequest>| {
                assert_eq!(request.get_ref().resource_name, std_out_resource_name);
                response_stream(vec![Err(status(tonic::Code::NotFound))])
            })),
            ..Default::default()
        });
    t.fake_server.start();

    let context = t.get_spawn_context(&t.simple_spawn);
    let result = t.client.exec(&*t.simple_spawn, &context).unwrap();

    assert_eq!(result.status(), SpawnStatus::RemoteCacheFailed);
    assert!(
        result
            .get_failure_message()
            .contains(&DigestUtil::to_string(&std_out_digest)),
        "{}",
        result.get_failure_message()
    );
    // Ensure we also got back the stack trace.
    // PORT: `assertThat(result.getFailureMessage()).contains(
    // "RemoteSpawnRunnerWithGrpcRemoteExecutorTest.passCacheMissErrorWithStackTrace")` has no
    // counterpart (no stack traces, see the file comment).
}

#[test]
fn pass_repeated_orphaned_cache_miss_error_with_stack_trace() {
    let mut t = Fixture::set_up();
    let std_out_digest = digest_util().compute_as_utf8("bloo");
    let action_result = ActionResult {
        output_files: vec![dummy_output()],
        stdout_digest: Some(std_out_digest.clone()),
        ..Default::default()
    };
    {
        let action_result = action_result.clone();
        t.fake_server
            .service_registry
            .add_action_cache(action_cache(move |_| Ok(action_result.clone())));
    }
    t.fake_server
        .service_registry
        .add_execution(execution_service(move |_| {
            response_stream(vec![Ok(done_operation(
                "",
                &execute_response(&action_result),
            ))])
        }));
    t.fake_server
        .service_registry
        .add_content_addressable_storage(cas(nothing_missing));
    let std_out_resource_name = t.resource_name(&std_out_digest);
    t.fake_server
        .service_registry
        .add_byte_stream(FakeByteStream {
            read: Some(Box::new(move |request: Request<ReadRequest>| {
                assert_eq!(request.get_ref().resource_name, std_out_resource_name);
                response_stream(vec![Err(status(tonic::Code::NotFound))])
            })),
            ..Default::default()
        });
    t.fake_server.start();

    let context = t.get_spawn_context(&t.simple_spawn);
    let result = t.client.exec(&*t.simple_spawn, &context).unwrap();

    assert_eq!(result.status(), SpawnStatus::RemoteCacheFailed);
    let spawn_code = match result.failure_detail().and_then(|d| d.category.as_ref()) {
        Some(failure_detail::Category::Spawn(spawn)) => spawn.code,
        other => panic!("not a Spawn failure detail: {other:?}"),
    };
    assert_eq!(
        spawn_code,
        failure_details::spawn::Code::RemoteCacheFailed as i32
    );
    assert!(
        result
            .get_failure_message()
            .contains(&DigestUtil::to_string(&std_out_digest)),
        "{}",
        result.get_failure_message()
    );
    // Ensure we also got back the stack trace because verboseFailures=true
    // PORT: `assertThat(result.getFailureMessage()).contains(
    // "passRepeatedOrphanedCacheMissErrorWithStackTrace")` has no counterpart (no stack traces,
    // see the file comment).
}

// The stack-trace assertions of the four `pass*WithStackTrace` tests: the failure message of a
// verbose failure contains `Throwables.getStackTraceAsString(e)` in Java. The tests above keep
// their other assertions; these hold the dropped ones (review 2026-11-19 R18).

#[test]
#[ignore = "Java stack traces (verboseFailures appends Throwables.getStackTraceAsString) have no counterpart: asserts the message contains \"com.google.devtools.build.lib.remote\""]
fn pass_unavailable_error_with_stack_trace_stack_trace() {}

#[test]
#[ignore = "Java stack traces (verboseFailures appends Throwables.getStackTraceAsString) have no counterpart: asserts the message contains \"com.google.devtools.build.lib.remote\""]
fn pass_internal_error_with_stack_trace_stack_trace() {}

#[test]
#[ignore = "Java stack traces (verboseFailures appends Throwables.getStackTraceAsString) have no counterpart: asserts the message contains \"RemoteSpawnRunnerWithGrpcRemoteExecutorTest.passCacheMissErrorWithStackTrace\""]
fn pass_cache_miss_error_with_stack_trace_stack_trace() {}

#[test]
#[ignore = "Java stack traces (verboseFailures appends Throwables.getStackTraceAsString) have no counterpart: asserts the message contains \"passRepeatedOrphanedCacheMissErrorWithStackTrace\""]
fn pass_repeated_orphaned_cache_miss_error_with_stack_trace_stack_trace() {}

#[test]
fn remotely_re_execute_orphaned_cached_actions() {
    let mut t = Fixture::set_up();
    let std_out_digest = digest_util().compute_as_utf8("stdout");
    let action_result = ActionResult {
        output_files: vec![dummy_output()],
        stdout_digest: Some(std_out_digest),
        ..Default::default()
    };
    {
        let action_result = action_result.clone();
        t.fake_server
            .service_registry
            .add_action_cache(action_cache(move |_| Ok(action_result.clone())));
    }
    {
        let first = Mutex::new(true);
        t.fake_server
            .service_registry
            .add_byte_stream(FakeByteStream {
                read: Some(Box::new(move |_: Request<ReadRequest>| {
                    // First read is a cache miss, next read succeeds.
                    let mut first = first.lock();
                    if *first {
                        *first = false;
                        response_stream(vec![Err(status(tonic::Code::NotFound))])
                    } else {
                        response_stream(vec![Ok(read_response("stdout"))])
                    }
                })),
                write: Some(accept_all_writes()),
                ..Default::default()
            });
    }
    let num_execute_calls = Arc::new(AtomicI32::new(0));
    {
        let num_execute_calls = num_execute_calls.clone();
        t.fake_server
            .service_registry
            .add_execution(execution_service(move |request| {
                num_execute_calls.fetch_add(1, Ordering::SeqCst);
                assert!(request.get_ref().skip_cache_lookup); // Action will be re-executed.
                response_stream(vec![Ok(done_operation(
                    "",
                    &execute_response(&action_result),
                ))])
            }));
    }
    // Nothing is missing.
    t.fake_server
        .service_registry
        .add_content_addressable_storage(cas(nothing_missing));
    t.fake_server.start();

    let context = t.get_spawn_context(&t.simple_spawn);
    let result = t.client.exec(&*t.simple_spawn, &context).unwrap();

    assert!(result.setup_success(), "{result:?}");
    assert_eq!(result.exit_code(), 0);
    assert!(!result.is_cache_hit());
    assert_eq!(t.out_err.out_as_latin1(), "stdout");
    assert_eq!(num_execute_calls.load(Ordering::SeqCst), 1);
}

#[test]
fn remotely_re_execute_orphaned_directory_cached_actions() {
    let mut t = Fixture::set_up();
    let action_result = ActionResult {
        output_directories: vec![dummy_output_directory()],
        ..Default::default()
    };
    {
        let action_result = action_result.clone();
        t.fake_server
            .service_registry
            .add_action_cache(action_cache(move |_| Ok(action_result.clone())));
    }
    let dummy_tree_resource_name =
        t.resource_name(dummy_output_directory().tree_digest.as_ref().unwrap());
    {
        let first = Mutex::new(true);
        t.fake_server
            .service_registry
            .add_byte_stream(FakeByteStream {
                read: Some(Box::new(move |request: Request<ReadRequest>| {
                    let resource_name = &request.get_ref().resource_name;
                    if *resource_name == dummy_tree_resource_name {
                        // First read is a cache miss, next read succeeds.
                        let mut first = first.lock();
                        if *first {
                            *first = false;
                            response_stream(vec![Err(status(tonic::Code::NotFound))])
                        } else {
                            response_stream(vec![Ok(read_response(
                                dummy_output_tree().to_byte_array(),
                            ))])
                        }
                    } else {
                        // responseObserver.onNext(ReadResponse.getDefaultInstance()) and the call is
                        // never completed.
                        hanging_response_stream(
                            vec![Ok(ReadResponse::default())],
                            Arc::new(std::sync::atomic::AtomicBool::new(false)),
                        )
                    }
                })),
                write: Some(accept_all_writes()),
                ..Default::default()
            });
    }
    let num_execute_calls = Arc::new(AtomicI32::new(0));
    {
        let num_execute_calls = num_execute_calls.clone();
        t.fake_server
            .service_registry
            .add_execution(execution_service(move |request| {
                num_execute_calls.fetch_add(1, Ordering::SeqCst);
                assert!(request.get_ref().skip_cache_lookup); // Action will be re-executed.
                response_stream(vec![Ok(done_operation(
                    "",
                    &execute_response(&action_result),
                ))])
            }));
    }
    // Nothing is missing.
    t.fake_server
        .service_registry
        .add_content_addressable_storage(cas(nothing_missing));
    t.fake_server.start();

    let context = t.get_spawn_context(&t.simple_spawn);
    let result = t.client.exec(&*t.simple_spawn, &context).unwrap();

    assert!(result.setup_success(), "{result:?}");
    assert_eq!(result.exit_code(), 0);
    assert!(!result.is_cache_hit());
    assert_eq!(num_execute_calls.load(Ordering::SeqCst), 1);
}

#[test]
fn retry_upload_and_execute_on_missing_inputs() {
    let mut t = Fixture::set_up();
    t.fake_server
        .service_registry
        .add_action_cache(action_cache(|_| Err(status(tonic::Code::NotFound))));
    t.fake_server
        .service_registry
        .add_byte_stream(FakeByteStream {
            read: Some(Box::new(|_: Request<ReadRequest>| {
                response_stream(vec![Ok(read_response("bla"))])
            })),
            write: Some(accept_all_writes()),
            ..Default::default()
        });
    let action_result = ActionResult {
        output_files: vec![dummy_output()],
        ..Default::default()
    };
    let num_execute_calls = Arc::new(AtomicI32::new(0));
    {
        let num_execute_calls = num_execute_calls.clone();
        t.fake_server
            .service_registry
            .add_execution(execution_service(move |request| {
                if num_execute_calls.fetch_add(1, Ordering::SeqCst) + 1 == 1 {
                    // Missing input.
                    let viol = Violation {
                        r#type: "MISSING".to_string(),
                        ..Default::default()
                    };
                    let status = rpc::Status {
                        code: tonic::Code::FailedPrecondition as i32,
                        details: vec![any_pack(&PreconditionFailure {
                            violations: vec![viol],
                        })],
                        ..Default::default()
                    };
                    response_stream(vec![Ok(done_operation(
                        "",
                        &ExecuteResponse {
                            status: Some(status),
                            ..Default::default()
                        },
                    ))])
                } else {
                    assert!(!request.get_ref().skip_cache_lookup);
                    response_stream(vec![Ok(done_operation(
                        "",
                        &execute_response(&action_result),
                    ))])
                }
            }));
    }
    let num_cache_uploads = Arc::new(AtomicI32::new(0));
    {
        let num_cache_uploads = num_cache_uploads.clone();
        t.fake_server
            .service_registry
            .add_content_addressable_storage(cas(move |_| {
                num_cache_uploads.fetch_add(1, Ordering::SeqCst);
                // Nothing is missing.
                Ok(FindMissingBlobsResponse::default())
            }));
    }
    t.fake_server.start();

    let context = t.get_spawn_context(&t.simple_spawn);
    let result = t.client.exec(&*t.simple_spawn, &context).unwrap();

    assert!(result.setup_success(), "{result:?}");
    assert_eq!(result.exit_code(), 0);
    assert!(!result.is_cache_hit());
    assert_eq!(num_cache_uploads.load(Ordering::SeqCst), 2);
    assert_eq!(num_execute_calls.load(Ordering::SeqCst), 2);
}

#[test]
fn exec_waits_on_unfinished_completion() {
    let mut t = Fixture::set_up();
    t.fake_server
        .service_registry
        .add_action_cache(action_cache(|_| Err(status(tonic::Code::NotFound))));

    let op_name = "operations/xyz";
    let result_digest = digest_util().compute(b"bla");
    t.fake_server
        .service_registry
        .add_byte_stream(FakeByteStream {
            read: Some(Box::new(|_: Request<ReadRequest>| {
                response_stream(vec![Ok(read_response("bla"))])
            })),
            ..Default::default()
        });
    let action_result = ActionResult {
        stdout_raw: b"stdout".to_vec(),
        stderr_raw: b"stderr".to_vec(),
        output_files: vec![OutputFile {
            path: "foo".to_string(),
            digest: Some(result_digest),
            ..Default::default()
        }],
        ..Default::default()
    };
    let unfinished_operation = Operation {
        name: op_name.to_string(),
        ..Default::default()
    };
    let complete_operation = Operation {
        done: true,
        result: Some(operation::Result::Response(any_pack(&execute_response(
            &action_result,
        )))),
        ..unfinished_operation.clone()
    };
    let wait_execution_request = WaitExecutionRequest {
        name: op_name.to_string(),
    };
    let execute_calls = Arc::new(AtomicI32::new(0));
    let wait_execution_calls = Arc::new(AtomicI32::new(0));
    // Flow of this test:
    // - call execute, get an unfinished Operation, then the stream completes
    // - call waitExecute, get an unfinished Operation, then the stream completes
    // - call waitExecute, get a finished Operation
    let wait_execution = in_sequence(
        wait_execution_calls.clone(),
        vec![
            answer_with::<WaitExecutionRequest>(
                Some(unfinished_operation.clone()),
                status(tonic::Code::Ok),
            ),
            answer_with::<WaitExecutionRequest>(Some(complete_operation), status(tonic::Code::Ok)),
        ],
    );
    t.fake_server.service_registry.add_execution(FakeExecution {
        execute: Some(Box::new(in_sequence(
            execute_calls.clone(),
            vec![answer_with::<ExecuteRequest>(
                Some(unfinished_operation),
                status(tonic::Code::Ok),
            )],
        ))),
        wait_execution: Some(Box::new(move |request: Request<WaitExecutionRequest>| {
            // .waitExecution(ArgumentMatchers.eq(waitExecutionRequest), any()): another request
            // reaches the spy's real method, which answers UNIMPLEMENTED.
            if *request.get_ref() == wait_execution_request {
                wait_execution(request)
            } else {
                response_stream(vec![Err(Status::unimplemented(
                    "Method build.bazel.remote.execution.v2.Execution/WaitExecution is unimplemented",
                ))])
            }
        })),
    });

    t.fake_server
        .service_registry
        .add_content_addressable_storage(cas(nothing_missing));
    t.fake_server.start();

    let context = t.get_spawn_context(&t.simple_spawn);
    let result = t.client.exec(&*t.simple_spawn, &context).unwrap();

    assert!(result.setup_success(), "{result:?}");
    assert_eq!(result.exit_code(), 0);
    assert!(!result.is_cache_hit());
    assert_eq!(execute_calls.load(Ordering::SeqCst), 1);
    assert_eq!(wait_execution_calls.load(Ordering::SeqCst), 2);
}
