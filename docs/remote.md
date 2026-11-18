# Remote caching and execution (`blaze_remote`)

Port of `com.google.devtools.build.lib.remote` (+ the gRPC side of `lib/authandtls`) at the pin.
Status, evidence and gaps are at the end; the first sections are the contract every file in the
crate follows.

## Layout

Java package `lib/remote/X/Y.java` is `crates/blaze_remote/src/x/y.rs` (snake_case file per Java
class; top-level classes of `lib/remote` are top-level modules). Tests are
`crates/blaze_remote/tests/<java_test_name_snake>.rs`, one file per JUnit class.

| module | Java |
|---|---|
| `reapi` | short names for the generated protos (`crate::reapi::Digest`, `ActionResult`, ...; `reapi::bytestream`, `reapi::longrunning`, `reapi::rpc`, `reapi::remote_logging`) |
| `runtime` | the tokio runtime; `get_from_future` (`Utils.getFromFuture`) |
| `grpc::stubs` | generated clients and servers: `action_cache_client::ActionCacheClient`, `content_addressable_storage_client::…`, `capabilities_client::…`, `execution_client::…`, `byte_stream_client::…` and the `*_server` twins |
| `common` | `lib/remote/common` (`errors` holds the exception mapping) |
| `util` | `lib/remote/util` (`DigestUtil`, `DigestOutputStream`, `AsyncTaskCache`, `TracingMetadataUtils`, `Utils`) |
| `disk`, `merkletree`, `logging`, `zstd`, `circuitbreaker` | same-named Java packages |

## Conventions (on top of PORTING.md)

1. **Futures.** A Java method returning `ListenableFuture<T>` / `Single<T>` / `Completable` is
   `async fn … -> std::io::Result<T>` (`Void` → `()`, a future that completes with `null` →
   `Option<T>`). Trait methods use `#[async_trait::async_trait]`. A *blocking* Java method (one
   that calls `getFromFuture`, `waitForBulkTransfer`, or declares `throws IOException,
   InterruptedException` without returning a future) stays a plain `fn` and blocks with
   `crate::runtime::get_from_future(fut)`. Never block inside an `async fn`. Fan-out
   (`Futures.allAsList`, `Flowable.flatMap`, `whenAllComplete`) is `futures::future::join_all` /
   `futures::stream::iter(..).buffer_unordered(n)` on the calling task; do not `tokio::spawn`
   anything that borrows (contexts borrow the spawn). Local file I/O inside an `async fn` is done
   inline (`// PORT: Java runs this on an executor`).
2. **Exceptions.** `IOException` and all its remote subclasses are `std::io::Error`
   (`crate::common::errors`): build with `X::new(..).into()`, test with
   `errors::downcast_ref::<X>(&e)` (Java `instanceof`), `errors::status_from_throwable(&e)` is
   `io.grpc.Status.fromThrowable`, `errors::status_to_io_error(status)` is
   `new IOException(new StatusRuntimeException(status))`, `errors::io_exception(msg)` is
   `new IOException(msg)`, `InterruptedException` is `errors::interrupted()` (kind `Interrupted`).
   gRPC statuses are `tonic::Status` / `tonic::Code`. A method that throws `ExecException` as well
   as `IOException` returns `Result<T, blaze_exec::exec::action_execution_context::SpawnCacheError>`
   (it has `From` for both). Unchecked exceptions are panics with the Java text
   (`blaze_util::check_state!`, `check_argument!`).
3. **Protos.** Types come from `crate::reapi`. Digests of messages and every blob uploaded for a
   message use `blaze_protos::JavaMessage::to_byte_array()` (Java field order), never
   `prost::Message::encode`. Parsing is `prost::Message::decode`. `ByteString` is `bytes::Bytes`
   (or `Vec<u8>` where the generated field is one); map fields are `Vec<Entry>`.
4. **Blaze types.** `blaze_vfs::{Path, PathFragment, FileStatus, Symlinks, Dirent}`;
   `blaze_exec::actions::{Spawn, ActionInput, ActionInputRef, Artifact, VirtualActionInput,
   FileArtifactValue, InputMetadataProvider, SpawnResult, ExecException, spawns, …}`;
   `blaze_exec::exec::{SpawnRunner, SpawnExecutionContext, ProgressStatus}`;
   `blaze_exec::exec::action_execution_context::{SpawnCache, CacheHandle, SpawnCacheError}`;
   `blaze_exec::util::FileOutErr`; options are
   `blaze_optdefs::misc::remote_options::RemoteOptions` (and `common_remote_options`,
   `auth_and_tls_options`); events are `blaze_events`. Read the Rust definitions before use; do
   not edit other crates. When a needed method does not exist there, do not invent it: put the
   call behind a small private helper that calls `blaze_util::unported!("Class.method (missing
   hook: <what>)")` and list it in your report.
5. **Borrowing.** `RemoteActionExecutionContext<'a>` borrows the spawn and its
   `SpawnExecutionContext`; pass contexts as `&RemoteActionExecutionContext<'_>`; background work
   that outlives the spawn takes `context.detached()`.
6. **Not ported is loud.** Features outside the phases (HTTP cache, CDC chunking, scrubbing
   config parsing, output service, rewinding, path mapping) call `blaze_util::unported!` at the
   point Java would enter them, with the Java class and method in the message.
7. **Security rules** (reviewed line by line): keep every digest check Java performs
   (`Utils.verifyBlobContents`, `--remote_verify_downloads`, `DigestOutputStream`), every output
   path check, and never resolve a server-provided path outside the exec root unless Java does.

## Phases, evidence, gaps

(Filled in by the crate lead as phases land.)
