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

## Security posture

Root ruling 2026-11-19 (docs/decisions.md): for output symlink paths, Tree node names and
server-log keys Blaze stays identical to Bazel 9.3.2, which does not confine them; Blaze does
not harden beyond Bazel. Each site is marked `// PORT: identical to Bazel, not hardened
(decision 2026-11-19)` (or by the `SECURITY:` comment that describes it, in files whose owner
has not added the marker yet). The sites:

| What the server controls | Rust site | Java |
|---|---|---|
| `ExecuteResponse.server_logs` keys: `..` and absolute keys escape the log directory | `remote_execution_service.rs` `maybe_download_server_logs` (marked) | `RemoteExecutionService.java:2075` |
| Tree node names (`FileNode`/`SymlinkNode`/`DirectoryNode.name`): a file can land anywhere under the exec root, not only under its tree root (outside the exec root is a crash in both) | `remote_execution_service_download.rs:378` (`SECURITY:`), checked only by `relative_to(exec_root)` at :1337/:1407/:1416 | `RemoteExecutionService.java:1086-1200` |
| `OutputFile`/`OutputDirectory`/`OutputSymlink.path` (normalized `getRelative`) | `remote_execution_service_download.rs:463` | `RemoteExecutionService.java:1086-1200` |
| Output symlink paths and targets (absolute, `..`, dangling targets accepted; symlink path not compared with the exec root) | `remote_execution_service_download.rs:589`, `:1139` | `RemoteExecutionService.java:950` `createSymlinks` |
| Inlined `OutputFile.contents` and inline stdout/stderr are not compared with their digests | `remote_execution_service_download.rs:1398` | `RemoteExecutionService.java` `downloadOutputs` |
| Symlink targets found in the output tree are re-planted verbatim by the prefetcher | `abstract_action_input_prefetcher.rs:1050`, `:1078` | `AbstractActionInputPrefetcher.java` |

The digest checks Java performs are all kept (rule 7 above). The one place where Blaze is weaker
than Bazel is the build-wide remote output tree (review R7, owned by the integration).

## Not ported (loud)

Each of these fails with `unported!` (a crash with `blaze: not ported yet: ...`) at the point
where Bazel would enter it; none is silently ignored.

| Feature / flag | Where it fails |
|---|---|
| TextFormat-dependent options: `exec_properties`/`remote_execution_properties` text protos of a platform; `--experimental_remote_scrubbing_config` | `platform_utils.rs` `PlatformUtils.getPlatformProto`; `scrubber.rs` `Scrubber.parse`, and `blaze_runtime` `build_remote.rs` before setup |
| Credential helpers (`--credential_helper`) and Google credentials (`--google_credentials`, `--google_default_credentials`) | `authandtls/google_auth_utils.rs` |
| HTTP/HTTPS remote cache (`--remote_cache=http(s)://`) | `combined_cache_client_factory.rs` `createHttp`, `remote_module.rs` `createCredentials` |
| Content-defined chunking (`--experimental_remote_cache_chunking`) | `combined_cache.rs` `Chunking.*`, `uploadChunked`; `grpc_cache_client.rs` split/splice |
| Remote output service (`--experimental_remote_output_service`); the "must be used with a cache" exit and the `--disk_cache` warning are ported | `remote_module.rs` setup, at Java's channel creation for it |
| Remote downloader (`--experimental_remote_downloader`) | `remote_module.rs` setup |
| Build event artifact uploader (`ByteStreamBuildEventArtifactUploader`) with a gRPC cache; `--remote_build_event_upload` (old name `--experimental_remote_build_event_upload`) other than `minimal`; `--remote_bytestream_uri_prefix` | `remote_module.rs` setup (flags), `RemoteModule::create_build_event_artifact_uploader`, called by `blaze_runtime` `build_remote.rs` `create_bep_file_transport_uploaders` for each BEP file transport with path conversion when the "remote" upload strategy is selected (the default) |
| `--remote_max_connections` / `--remote_max_concurrency_per_connection` other than 100 (one HTTP/2 connection per endpoint, no `DynamicConnectionPool`) | `remote_module.rs` setup, gRPC path |
| `--experimental_remote_cache_lease_extension` (without `--rewind_lost_inputs`) | `remote_module.rs` setup (Java: `executorInit`) |
| Disk cache garbage collection (`--experimental_disk_cache_gc_max_size` / `_max_age`, with `_idle_delay`): a server idle task, and Blaze has no server | `remote_module.rs` setup |
| Path mapping, lost inputs and action rewinding, persistent-worker signatures | `common/remote_path_resolver.rs`, `common/bulk_transfer_exception.rs`, `remote_execution_service.rs` |

Known silent deviations that cannot be made loud from this crate are listed in
docs/reviews/2026-11-19-remote-response.md (argv[0] policy of a Windows execution platform:
`PlatformInfo` carries no constraints).
