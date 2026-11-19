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

All evidence is against Bazel 9.3.2 (the oracle) and Bazel's own remote worker
(`scripts/remote/worker`), through the real `blaze` binary unless a line says "harness"
(`blaze-remote-replay`, which replays the spawns of an oracle exec log through `blaze_remote`).
Tools: `scripts/remote/{oracle-run,worker,replay,compare-grpc-log,matrix,second-build,ws-run,
probe-trees,probe-inprocess,evict-probe,caps-matrix,bep-uris,stale-probe,symlink-probe,
srcdir-probe,cplookup-probe}` (each prints `help`). Numbers are from the working tree on top of
c74f487 (binary recorded as `blaze-c74f4870286a-dirty-6cb9e325`) unless noted.

| phase | what | evidence |
|---|---|---|
| P1 disk cache | DiskCacheClient, CombinedCache, UploadManifest, MerkleTreeComputer, RemoteExecutionService, RemoteSpawnCache | brotli: 43/43 action digests equal Bazel's; Bazel's cache read by Blaze `43 disk cache hit`, Blaze's cache read by Bazel `53 processes: 43 disk cache hit, 10 internal`; both caches hold the same 43 `ac` and 165 `cas` names, ActionResults equal except execution timestamps. re2 (`test`): both directions `716 processes: 550 disk cache hit, 165 internal, 1 linux-sandbox`, which is what Bazel prints on its own cache. |
| P2 remote cache | channel, capabilities, GrpcCacheClient, ByteStreamUploader, RequestMetadata, LoggingInterceptor | `--remote_grpc_log` vs Bazel's, RPC by RPC after normalisation: brotli cold 295/295, warm 51/51; re2 warm 600/600; re2 cold 3453/3535 (the other 82 RPCs all belong to the 21 TestRunner actions, whose outputs are timing-dependent). Interchange through the worker, both directions: brotli 43/43, re2 550/550 `remote cache hit`. |
| P3 remote execution | GrpcRemoteExecutor, RemoteExecutionCache, RemoteSpawnRunner | brotli: `53 processes: 10 internal, 43 remote` on both, outputs equal to local execution, log MATCH (504 RPCs each, attribution-insensitive). re2 with `build //:all //python:all` (Bazel's worker drops long test operations; the oracle itself fails `test` with `NOT_FOUND: Operation not found`): `695 processes: 165 internal, 1 linux-sandbox, 529 remote` on both, log MATCH (6095 RPCs each). |
| P4 build without the bytes | RemoteOutputChecker, AbstractActionInputPrefetcher, RemoteActionInputFetcher, RemoteActionFileSystem (a per-action overlay file system in `blaze_exec`), remote metadata in the action cache | brotli with an oracle-seeded disk cache, toplevel / minimal / all: build, build again, build after one source edit print the oracle's lines (`43 disk cache hit, 10 internal`; `1 process: 52 action cache hit, 1 internal`; `2 processes: 51 action cache hit, 1 internal, 1 linux-sandbox`) with identical `bazel-bin` after all 9 steps; tree artifacts (`probe-trees`), in-process readers (`probe-inprocess`), a remote-only `test.xml`: identical. |
| default path | no remote flag | referee unchanged; `scripts/gate/compare-scores corpus/scores/blaze-ef66f55e904e.referee2.json <new>`: rows 39, gained 0, regressed 0, PASS. One intended change: the action cache salt is Java's (review R16), so Blaze reuses a Bazel-written output base (52 action cache hits where 43 actions re-ran before). |

Gaps that are loud: see "Not ported (loud)". Gaps that are not loud are listed in
docs/reviews/2026-11-19-remote-response.md ("Open after this round"): action rewinding (Bazel's
default answer to an input lost from the remote cache under build without the bytes: Blaze
matches Bazel only with `--norewind_lost_inputs`, and fails with `unported!` otherwise),
`RemoteLeaseExtension`, exit code 39 after exhausted eviction retries not reproduced end to end,
`argv[0]` conversion for a Windows execution platform (needs the platform's OS constraint),
remote flags for commands other than build/test, the connection pool
(`--remote_max_connections`: one multiplexed HTTP/2 connection).

- Build event artifact uploader (review R12, 2026-11-19): `byte_stream_build_event_artifact_uploader.rs`
  ports `ByteStreamBuildEventArtifactUploader(Factory)`; `RemoteModule::create_build_event_artifact_uploader`
  is the "remote" factory delegate, selected by `--experimental_build_event_upload_strategy`
  through `blaze_bep::runtime::BuildEventArtifactUploaderFactoryMap` (built per command in
  `blaze_runtime` `build_remote.rs`). With a gRPC cache the BEP files name their files
  `bytestream://<authority or --remote_bytestream_uri_prefix>[/<instance>]/blobs/[<fn>/]<hash>/<size>`;
  `--remote_build_event_upload=minimal|all` decides which files are uploaded. Evidence (brotli
  `build //...`, cold and warm, both modes): docs/reviews/2026-11-19-remote-response.md, R12.

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
| Build event artifacts on a `RemoteActionFileSystem` path (`isRemoteFile`; Blaze's build events refer to output-base paths, so it is not reached) | `byte_stream_build_event_artifact_uploader.rs` `is_remote_file` |
| `--remote_max_connections` / `--remote_max_concurrency_per_connection` other than 100 (one HTTP/2 connection per endpoint, no `DynamicConnectionPool`) | `remote_module.rs` setup, gRPC path |
| `--experimental_remote_cache_lease_extension` (without `--rewind_lost_inputs`) | `remote_module.rs` setup (Java: `executorInit`) |
| Disk cache garbage collection (`--experimental_disk_cache_gc_max_size` / `_max_age`, with `_idle_delay`): a server idle task, and Blaze has no server | `remote_module.rs` setup |
| Path mapping, lost inputs and action rewinding, persistent-worker signatures | `common/remote_path_resolver.rs`, `common/bulk_transfer_exception.rs`, `remote_execution_service.rs` |

Known silent deviations that cannot be made loud from this crate are listed in
docs/reviews/2026-11-19-remote-response.md (argv[0] policy of a Windows execution platform:
`PlatformInfo` carries no constraints).
