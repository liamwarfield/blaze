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

### Phase 1: the `blaze` binary with `--remote_download_outputs=all` (2026-11-19)

**Wiring.** `blaze_remote` has no `wip` gate (the empty feature is kept for old command lines).
`blaze build` / `blaze test` run `RemoteModule.beforeCommand` before the Build Event Service
module (`blaze_runtime::commands::build::run_build` →
`blaze_buildtool::buildtool::execution_tool::remote_module_before_command`, which builds
`RemoteModuleEnvironment` from the command: options, reporter, paths, `env.getBuildRequestId()`,
`env.getCommandId()` (the BEP ids), product `bazel`, version 9.3.2, client env).
`ExecutionTool` (`prepare_for_execution_with_action_cache`) then does what the module's
`registerSpawnStrategies` (`"remote"` + the remote-local fallback identifier, through
`blaze_exec::exec::executor_setup::RemoteModuleSpawnStrategies`), `registerActionContexts`
(`RemoteSpawnCache` when a cache is available, else `NO_CACHE`) and `executorInit` (the action
execution salt, `computeActionExecutionSalt`, for every build) do. `afterCommand` (await
`--remote_cache_async` uploads, close the gRPC log, delete downloads) runs after the command,
before the BES module; its `AbruptExitException` becomes the exit code unless the command's is
an infrastructure failure. `RemoteExecutionService.onBuildComplete/onBuildInterrupted` are called
where the events are posted.

In-memory outputs (`internal-inline-outputs`, the `.d` files of C++ compiles) are never written
to disk, as in Bazel: their metadata goes to `blaze_sky_exec::remote_output_tree` (the remote
output tree of `RemoteActionFileSystem`), which the action's output metadata store reads first
and which the spawn log reads through `ActionExecutionContext.getActionFileSystem()`
(`RemoteActionFileSystem` read view). Calls on a channel wait for the verified server
capabilities (`GoogleChannelConnectionFactory.create`), so an incompatible server fails every
action with `Failed to query remote execution capabilities: ...`.

**Works** (`build` and `test`): `--disk_cache`, `--remote_cache=grpc[s]://`, `--remote_executor=
grpc[s]://` (`remote` becomes the first default strategy), `--remote_upload_local_results`,
`--remote_accept_cached`, `--remote_cache_async`, `--remote_local_fallback`,
`--remote_local_fallback_strategy`, `--remote_timeout`, `--remote_retries`,
`--remote_instance_name`, `--remote_default_exec_properties`, `--remote_header`,
`--remote_grpc_log`, `--remote_cache_compression` (its capability check; the local worker has no
zstd, so only the refusal path is compared), `--remote_max_connections` (accepted; one
multiplexed HTTP/2 connection, see `grpc::channel`), `--remote_verify_downloads`,
`--execution_log_json_file` / `--execution_log_compact_file`, the `N processes: ...` line.

**Fails loudly:** `--remote_download_outputs=toplevel|minimal` (Bazel 9's default is
`toplevel`) with any cache: `RemoteModule.executorInit (RemoteActionInputFetcher,
RemoteOutputService: Build without the Bytes): --remote_download_outputs=toplevel with a remote
or disk cache; pass --remote_download_outputs=all`; HTTP caches (`RemoteModule.createCredentials
(HTTP remote cache)` / `CombinedCacheClientFactory.createHttp`);
`--experimental_remote_output_service`, `--experimental_remote_downloader` and the remote flags
for commands other than `build`/`test` (`blaze_runtime::unported_options`); `--bes_backend`.

**Evidence** (tree e297ee3 + this change, `target/debug/blaze` vs
`~/.cache/blaze/oracle/bazel-9.3.2-linux-x86_64`, each with its own `--output_base`,
`--batch --nosystem_rc --nohome_rc --remote_download_outputs=all`; workspace
`~/.cache/blaze/bench/remote-ws`: 4 genrules, a cc_library, a cc_binary and a cc_test, 11
spawns (13 with `test`); server: Bazel's remote worker built from the pin,
`scripts/remote/worker start p1 18981`, fresh for every scenario; "warm" is after `clean`):

| scenario | oracle summary | Blaze summary | outputs (sha256) | JSON exec log entries equal (minus `metrics`) | gRPC log (`compare-grpc-log`) |
|---|---|---|---|---|---|
| `--disk_cache` cold | 10 internal, 11 processwrapper-sandbox | same | (in the exec log) | 11/11 identical | - |
| `--disk_cache` warm | 11 disk cache hit, 10 internal | same | (in the exec log) | 11/11 identical | - |
| disk cache written by the other tool | 11 disk cache hit | 11 disk cache hit | 15/15 same | 11/11 identical | - |
| `--remote_cache` cold | 10 internal, 11 processwrapper-sandbox | same | 22/22 | 11/11 | MATCH 71/71 |
| `--remote_cache` warm | 11 remote cache hit, 10 internal | same | 15/15 | 11/11 | MATCH 24/24 |
| remote cache written by the other tool (both ways) | 11 remote cache hit | 11 remote cache hit | 15/15 | 11/11 | - |
| `--remote_cache --remote_cache_async`, `test` cold / warm | 13 sandbox / 13 remote cache hit | same | 15/15 | 13/13 | MATCH 84/84, 28/28 |
| `--noremote_upload_local_results` cold / warm | 11 sandbox / 11 sandbox | same | same | 11/11 | MATCH |
| `--noremote_accept_cached` cold / warm | 11 sandbox / 11 sandbox | same | same | 11/11 | MATCH |
| `--remote_executor` cold | 10 internal, 11 remote | same | 15/15 | 11/11 | MATCH (317/317 digest items) |
| `--remote_executor` warm | 11 remote cache hit, 10 internal | same | 15/15 | 11/11 | MATCH 24/24 |
| `test --remote_executor --remote_instance_name --remote_header --remote_default_exec_properties --remote_timeout --remote_retries --remote_max_connections --remote_verify_downloads` cold / warm | 13 remote / 13 remote cache hit; PASSED / (cached) PASSED | same | 15/15 | 13/13 | MATCH (412/412 items), 28/28 |
| worker `--unavailable`, `--remote_executor --remote_local_fallback --remote_retries=0` | `WARNING: Remote Cache: UNAVAILABLE: null`, 11 local | same | - | - | - |
| `--remote_cache_compression` against the worker (no zstd) | `Failed to query remote execution capabilities: --remote_cache_compression requested but remote does not support compression` | same text | - | - | - |

The compact execution log is written in every mode; its content was not compared (no `protoc`
or Python `zstandard` on the measuring machine for `scripts/execlog-compact`).

**Gaps (phase 1).** `RemoteOutputService`/`RemoteActionFileSystem` are only the read view above:
the action cache does not store remote output metadata (`setStoreOutputMetadata(false)`,
`TRUST_LOCAL_ONLY`), so a rebuild without `clean` re-runs a C++ compile whose `.d` is in memory
through the remote cache, where Bazel has an action cache hit. No `RemoteActionInputFetcher`
(nothing is remote with `all`). One HTTP/2 connection per endpoint (`--remote_max_connections`
does not size a pool). Disk cache garbage collection (an idle task) does not run. The process
count of a build that fails early can differ by the internal actions that finished before the
failure (a race in both tools).

