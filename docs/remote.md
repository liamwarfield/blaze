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
execution salt, `computeActionExecutionSalt`, for every build) do. With a cache the output
service is `RemoteOutputService` as far as the action cache goes: remote output metadata is
stored in it and checked with the `RemoteOutputChecker` (`shouldTrustMetadata`), so a rebuild
without `clean` has action cache hits for compiles whose `.d` stayed in memory. `afterCommand`
and its `afterCommandTask` run after the command, before the BES module, in Java's order: the
services shut down (waiting for `--remote_cache_async` uploads), the download temp directory
is deleted (a failure is ignored), the gRPC log is closed (a failure is `Partially wrote RPC log
file`, exit 36, which replaces the command's exit code unless that is an infrastructure
failure). `AbruptExitException`s carry Java's explicit exit code (`DetailedExitCode.of(exitCode,
failureDetail)`). A channel whose transport cannot be created (bad `--tls_certificate`, ...)
fails each call, not the command (`GoogleChannelConnectionFactory.create`).
`RemoteExecutionService.onBuildComplete/onBuildInterrupted` are called where the events are
posted.

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
for commands other than `build`/`test` (`blaze_runtime::unported_options`); `--bes_backend`;
a `--build_event_*_file` together with a gRPC cache or remote execution while
`--experimental_build_event_upload_strategy` is unset or `remote` (Bazel's
`ByteStreamBuildEventArtifactUploader` names the files `bytestream://...`; `local` works);
`--experimental_remote_cache_lease_extension` (`RemoteLeaseExtension`; with
`--rewind_lost_inputs` Java's warning is printed instead); reading the contents of a remote-only
file through the action file system (`RemoteActionFileSystem.getInputStream`).

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

Review round 1 (STOP on 3353c77) rows, same setup:

| scenario | oracle | Blaze |
|---|---|---|
| rebuild without `clean` after a cache build (`--remote_cache`, `--remote_executor`, `--disk_cache`) | `1 process: 20 action cache hit, 1 internal.`, no exec log entry, gRPC log: 1 `GetCapabilities` | same (gRPC log MATCH 1/1, 0/0 exec log entries) |
| `--remote_cache=grpcs://localhost:18981 --tls_certificate=/missing.pem` | `WARNING: Remote Cache: Failed to init TLS infrastructure using '/missing.pem' as root certificate: File does not contain valid certificates: /missing.pem`, 11 processwrapper-sandbox, exit 0 | same |
| same with `--remote_executor ... --remote_local_fallback` | 11 local, exit 0 | same |
| `--remote_grpc_log=/dev/full` | `ERROR: Partially wrote RPC log file`, exit 36 | same |
| stale `<action_temps>/remote` that cannot be deleted (parent read-only) | `ERROR: Failed to delete stale downloads: [unix_jni.cc:660] <path> (Permission denied)`, exit 36 | `ERROR: Failed to delete stale downloads: <path> (Permission denied)`, exit 36 (the text of the I/O error is blaze_vfs's, without the JNI location) |
| `--build_event_json_file` with `--remote_cache=grpc://` | NamedSetOfFiles URIs `bytestream://localhost:18981/blobs/<hash>/<size>` | fails loudly (`ByteStreamBuildEventArtifactUploader`) |

The compact execution log is written in every mode; its content was not compared (no `protoc`
or Python `zstandard` on the measuring machine for `scripts/execlog-compact`).

**Gaps (phase 1).** `RemoteOutputService`/`RemoteActionFileSystem` are the read view above and
the action cache configuration; no `RemoteActionInputFetcher` (nothing is remote with `all`).
On interrupt the background uploads are waited for, where Java cancels them
(`RemoteExecutionService.shutdown`, a `PORT:` note there). A `beforeCommand` failure returns
before the BES module is set up, so a requested BEP file is not written (Java still runs every
module's `beforeCommand`/`afterCommand`). `cquery`/`aquery` with remote options have no
`RemoteModule` (Bazel prints `Invocation ID` and creates `bazel-remote-logs`). The capability
gate and the remote output tree have no unit tests (end-to-end rows only). One HTTP/2 connection per endpoint (`--remote_max_connections`
does not size a pool). Disk cache garbage collection (an idle task) does not run. The process
count of a build that fails early can differ by the internal actions that finished before the
failure (a race in both tools).


### Evidence on a real RBE (Stairwell's Aspect Workflows deployment, 2026-11-19)

Binary: `target/release/blaze` built from 3353c77. Reference: the oracle (Bazel 9.3.2). Both run
through `scripts/remote/rbe-bench` (same flags: `grpcs://` cache and executor, BLAKE3 digests,
`--credential_helper=<host>=aspect`, `--remote_download_outputs=all`, `--bes_backend=`, the
depot's base exec properties, the `//bench_rbe` cc toolchain for the worker container; cold =
a new `BENCH_SALT` action env value, warm = same salt after `clean`). Run records:
`~/.cache/blaze/bench/runs/` on the measuring machine.

| project, mode | oracle summary | Blaze summary | outputs (sha256) | gRPC calls by method and status (`grpc-log-methods`) |
|---|---|---|---|---|
| brotli `build //...`, cold | 53 processes: 10 internal, 43 remote | same | 44/45 equal | equal: 43 Execute, 43 FindMissingBlobs, 43 GetActionResult (NOT_FOUND), 115 Read, 86 Write |
| brotli, warm | 43 remote cache hit, 10 internal | same | 44/45 equal | equal: 43 GetActionResult, 115 Read |
| re2 `test //:all //python:all`, cold | 716 processes: 165 internal, 1 processwrapper-sandbox, 550 remote; 21/21 tests pass | same | 576/599 equal | equal: 550 Execute, 571 FindMissingBlobs, 550 GetActionResult (NOT_FOUND), 782 Read, 1100 Write |
| re2, warm | 550 remote cache hit, 165 internal, 1 processwrapper-sandbox | same | 576/599 equal | equal: 550 GetActionResult, 782 Read |
| Blaze, warm, with the oracle's salt (entries written by Bazel) | - | brotli 43/43 and re2 550/550 remote cache hits | as above | equal to the oracle's warm run |

The files that differ are the `*.runfiles_manifest` files (1 in brotli, 23 in re2), which hold
absolute paths under each tool's output base; with the output base name replaced they are
identical. `compare-grpc-log` cannot decode these logs (the server attaches Buildbarn `Any`
types the dumper does not know), so the gRPC comparison here is by counts only. The Stairwell
depot itself is not measured: Bazel 9.3.2 cannot analyse it (docs/decisions.md 2026-11-19).

### Phase 2: Build without the Bytes (`--remote_download_outputs=toplevel|minimal`, 2026-11-19)

**Wiring.** `RemoteModule::executor_init` no longer refuses `toplevel`/`minimal`; with a cache it
creates the `RemoteActionInputFetcher` (`remote_action_input_fetcher.rs` over
`abstract_action_input_prefetcher.rs`, ports of the Java classes) and returns it with the salt
(`RemoteExecutorInit`). `ExecutionTool` hands it to the executor
(`SkyframeActionExecutor::with_remote_output_tree(tree, prefetcher)`), where it is the three
things Java makes of it: the executor's `ActionInputPrefetcher` (`blaze_exec::actions::
action_input_prefetcher`, reached by local and sandboxed runners through
`ActionExecutionContext::prefetch_files`, with `SpawnExecutionContextImpl.prefetchInputs`'s
`BulkTransferException` handler), `RemoteOutputService.finalizeAction` (called in
`actually_complete_action` after `check_outputs`; a failure is `unable to finalize action`,
`ACTION_FINALIZATION_FAILURE`) and the fetcher of the action file system
(`RemoteActionFileSystem.getInputStream` downloads a remote file, `downloadIfRemote`). The
`RemoteOutputChecker` learns the top-level outputs from `TopLevelTargetAnalysisWatcher`
(`blaze_runtime::commands::build`, registered on the command's event bus): each
`TopLevelTargetAnalyzedEvent` calls `CommandRemoteModule::after_top_level_target_analysis`
(important output groups of the requested `TopLevelArtifactContext`, the runfiles when
`_hidden_top_level_INTERNAL_` is requested), each `TestAnalyzedEvent`
`after_single_test_analysis` (`TestProvider` outputs). `--remote_download_regex` patterns are
built by `RemoteOutputChecker::patterns_to_download` in `beforeCommand` for both modes.

The build-wide remote output tree (`blaze_sky_exec::remote_output_tree`) now also holds the
remote outputs of actions that hit the action cache (`ActionOutputMetadataStore.injectFile`/
`injectTree` with remote metadata), standing in for the input metadata of Java's per-action
`RemoteActionFileSystem`; its `stat` follows local symlinks to remote files, its `readdir` merges
the remote entries (sorted, as `getDirectoryContents`), and directories that exist only remotely
stat as directories. An output that is a local symlink to a remote file (`SymlinkAction`,
`SolibSymlinkAction`) gets the remote metadata with its resolved path, as
`fileArtifactValueFromArtifact` does with a `FileStatusWithMetadata`. Remote tree artifacts are
ported: the store lists a tree's remote children, `TreeArtifactValue`'s metadata is
`TreeArtifactCompositeFileArtifactValue` (`FileArtifactValue::TreeArtifactComposite`) when the
tree is entirely remote, and cached remote tree metadata is rebuilt with
`SkyframeTreeArtifactFactory` (`TreeFileArtifact.createTreeOutput`).

**Evidence** (tree 3bb3adf + this change, `target/debug/blaze` vs the oracle, identical flags,
`--batch --nosystem_rc --nohome_rc`, one `--output_base` per tool and scenario; workspace
`~/.cache/blaze/bench/remote-ws-bwob`: the phase 1 workspace plus a `no-remote` genrule reading a
remote intermediate (`//:local_consumer`), a `ctx.actions.symlink` target consumed remotely and
locally (`//:use_sym`, `//:use_sym_local`), a tree artifact consumed remotely and locally
(`//:tree_consumer`, `//:tree_consumer_local`) and a top-level tree (`//:tree_top`); build targets
`//:hello //:local_consumer //:use_sym //:use_sym_local //:tree_top //:tree_consumer
//:tree_consumer_local`, test target `//:hello_test` (dynamically linked: `_solib_k8` symlink,
runfiles tree, `test.xml` generation spawn). Tools: `scripts/remote/bwob-matrix` (a fresh worker
per tool and scenario; cold, warm after `clean`, rebuild without `clean`), `bwob-run`,
`bwob-compare`. "files" is the set and sha256 of every file, symlink (target) and directory under
`bazel-out`, without `_tmp/actions` and `{stable,volatile}-status.txt` (host name, time), and
with `test.cache_status` compared by presence (it holds the test's start time); runs in
`~/.cache/blaze/bench/bwob-runs/m3`.)

| scenario | oracle summary (cold / warm / rebuild) | Blaze | files cold / warm / rebuild | exec log | gRPC log |
|---|---|---|---|---|---|
| `build --remote_cache --remote_download_outputs=toplevel` | 13 processwrapper-sandbox / 10 remote cache hit, 3 processwrapper-sandbox / 18 action cache hit (+ internal) | same | 38/38, 32/32, 32/32 | 13/13, 13/13, 0/0 | MATCH ×3 |
| same, `minimal` | same | same | 38/38, 26/26, 26/26 | same | MATCH ×3 |
| `build --remote_executor`, `toplevel` | 3 processwrapper-sandbox, 10 remote / 10 remote cache hit, 3 processwrapper-sandbox / 18 action cache hit | same | 32/32 ×3 | 13/13, 13/13, 0/0 | MATCH ×3 |
| same, `minimal` | same | same | 26/26 ×3 | same | MATCH ×3 |
| `test --remote_cache`, `toplevel` | 9 processwrapper-sandbox / 9 remote cache hit / 13 action cache hit; 1 test passes | same | 37/37, 26/26, 26/26 | 9/9, 9/9, 0/0 | MATCH ×3 |
| same, `minimal` | same | same | 37/37, 25/25, 25/25 | same | MATCH ×3 |
| `test --remote_executor`, `toplevel` | 9 remote / 9 remote cache hit / 13 action cache hit | same | 26/26 ×3 | same | MATCH ×3 |
| same, `minimal` | same | same | 25/25 ×3 | same | MATCH ×3 |
| `build --remote_executor --remote_download_outputs=minimal --remote_download_regex=.*/generated\.h --remote_download_regex=.*/tree_dir/sub/.*` | as `minimal` | same | 27/27 ×3 | 13/13, 13/13, 0/0 | MATCH ×3 |
| `test --remote_cache --remote_download_outputs=toplevel --remote_download_regex=.*/_objs/.*\.o` | as `toplevel` | same | 37/37, 28/28, 28/28 | 9/9, 9/9, 0/0 | MATCH ×3 |
| `build --disk_cache --remote_download_outputs=toplevel` | 13 processwrapper-sandbox / 13 disk cache hit / 18 action cache hit | same | 38/38, 27/27, 27/27 | 13/13, 13/13, 0/0 | - |
| `build //:sym --remote_executor`, `toplevel` (finalizeAction downloads the symlink's target) | 1 remote / 1 remote cache hit / 2 action cache hit | same | 7/7 ×3 | 1/1, 1/1, 0/0 | MATCH ×3 |
| `build //:hello`, then `build //:hello //:b.txt` without `clean` (`toplevel`, remote exec; a remote output becomes top-level) | 6 remote / 9 action cache hit, 1 remote cache hit | same | 16/16, 17/17 | 6/6, 1/1 | MATCH ×2 |
| regression: `build --remote_cache`, `test --remote_executor`, `--remote_download_outputs=all` | as phase 1 | same | 38/38, 35/35, 35/35; 33/33 ×3 | all identical | MATCH ×6 |

**Fails loudly (phase 2):** a remote cache that lost a blob before it was downloaded:
`LostInputsExecException (action rewinding for lost remote inputs)` (inputs of a local spawn) and
`LostInputsExecException (AbstractActionInputPrefetcher.finalizeAction: action rewinding for lost
remote outputs)` (top-level outputs); `--experimental_remote_cache_lease_extension` without
`--rewind_lost_inputs` (`RemoteModule.executorInit (RemoteLeaseExtension)`, unchanged);
`ArchivedTreeArtifact.createForTree` and `ActionCacheChecker.constructProxyTreeMetadata` when the
action cache rebuilds such tree metadata.

**Gaps (phase 2).** `RemoteImportantOutputHandler` is not ported: Blaze posts every
`TopLevelTargetAnalyzedEvent` before the first action runs, so the checker knows the top-level
outputs when they are produced; the handler's runfiles download at target completion is covered
by the checker's `addRunfiles` only. `FileArtifactValue.setContentsProxy` after a download or an
up-to-date check is not ported (Blaze's values are immutable): later checks hash the file again.
The prefetcher creates directories with `createDirectoryAndParents`, not through the
`ActionOutputDirectoryHelper` Java passes for every build (Blaze's executor owns its helper): a
file or symlink where a directory is needed (an output path component that changed from file to
directory between builds, served from the action cache) is an I/O error, where Java replaces it. Download progress events are not
posted. The prefetcher's `shutdown` is not called in `afterCommand` (Blaze's downloads are
synchronous, nothing is pending). An output symlink to a local input gets no resolved path (only
remote targets do). `--experimental_finalize_actions` is not read (true by default). Local tree
artifacts keep `HashedDirectory` metadata where Java has a non-remote
`TreeArtifactCompositeFileArtifactValue` with the same digest. `getInputStream` of a remote file
that is not an input of the action is downloaded (Java's `downloadIfRemote` only fetches the
action's inputs; the build-wide tree has no per-action inputs). Unit tests:
`tests/remote_action_input_fetcher_test.rs` ports 12 tests of `ActionInputPrefetcherTestBase`,
`RemoteActionInputFetcherTest` and `RemoteOutputCheckerTest`; 19 are `#[ignore]` (spied or
overlay file systems, interruption, rewinding, partial/resolved trees).

**Review round 2 follow-ups** (same binary, oracle first): R2-1, `build //:gen_a
--execution_log_json_file=a --execution_log_binary_file=b --spawn_strategy=bogus`: both print
`ERROR: 'bogus' was requested for explicit default strategies but no strategy with that
identifier was registered. ...`, exit 2 (the registry error now outranks the `SpawnLogModule`
exit). Without `--spawn_strategy`: both print `ERROR: Must specify at most one of
--execution_log_binary_file, --execution_log_json_file and --execution_log_compact_file`, exit 2,
but Bazel reports it after `Loading: 0 packages loaded` and Blaze after `INFO: Analyzed target
//:gen_a ...` / `INFO: Found 1 target...` (open: Blaze checks the ExecutionTool's errors after
analysis, `blaze_sky_top`'s `BuildTool` post-processing hook; the same holds for the registry
error). R2-2: `--remote_cache=grpc://... --build_event_json_file=f
--nobuild_event_json_file_path_conversion` builds and names `a.txt` `file://...` in both (Bazel
also lists its `command-*.profile.gz`, which Blaze does not write). R2-3:
`--experimental_stream_log_file_uploads` with a gRPC cache fails loudly in Blaze
(`ByteStreamBuildEventArtifactUploader (BlazeRuntime.newUploader:
--experimental_stream_log_file_uploads)`); Bazel streams the profile to the cache.

**Fix after the RBE run (re2, `minimal`, warm: 21 extra `TestXmlGeneration` spawns).** Java's
test strategy resolves a test's outputs on `actionExecutionContext.getExecRoot()`, the action file
system's exec root, so a `test.xml`, `test.shard` or undeclared output that stays remote exists
there. Blaze resolved them on the local exec root: with a test that writes its own `test.xml`
(googletest), `!xmlOutputPath.exists()` held and the generator spawn ran for every test; a sharded
test failed with `Sharding requested, but the test runner did not advertise support for it ...`.
`blaze_exec::exec::action_execution_context::exec_root_on_action_file_system` is Java's
`getExecRoot()`; the `StandaloneTestRunnerSpawn` (`prepareFileSystem`, `resolve(execRoot)` of
each attempt) and `getTestOutputsMapping` (also its test log and coverage paths, Java's
`pathResolver.toPath`) use it. The action file system's `delete` and `renameTo` (flaky test
attempts) also act on the remote output tree, as Java's. `TestRunnerAction.actionCacheHit` keeps
the local exec root (Java's action file system is empty on an action cache hit, see the TODO
there). Reproduced before the fix (`min-exec-test` below: the oracle has one `TestRunner` spawn for
`//:xml_test`, Blaze two; the shard test fails) and compared after it, with
`//:hello_test //:xml_test //:xml_shard_test` (`xml_test.cc` writes `$XML_OUTPUT_FILE`, touches
`$TEST_SHARD_STATUS_FILE` and writes an undeclared output; `xml_shard_test` has `shard_count = 2`;
contents differ per target and shard so that no blob is shared between test actions), runs in
`~/.cache/blaze/bench/bwob-runs/m6`:

| scenario | oracle summary (cold / warm / rebuild) | Blaze | files | exec log | gRPC log |
|---|---|---|---|---|---|
| `test --remote_executor`, `minimal` | 14 internal, 16 remote / 16 remote cache hit, 14 internal / 28 action cache hit; 3 tests pass | same | 60/60 ×3 | 16/16, 16/16, 0/0 | MATCH ×3 |
| same, `toplevel` | same | same | 72/72 ×3 | same | MATCH ×3 |
| same, `all` | same | same | 83/83 ×3 | same | MATCH ×3 |
| `test --remote_cache`, `minimal` | 16 processwrapper-sandbox / 16 remote cache hit / 28 action cache hit | same | 91/91, 60/60, 60/60 | same | MATCH ×3 |
| same, `toplevel` | same | same | 91/91, 72/72, 72/72 | same | MATCH ×3 |
| same, `all` | same | same | 91/91, 83/83, 83/83 | same | MATCH ×3 |

The summary total does not sum its parts in Bazel either (`29 processes: 14 internal, 16
remote.`): Blaze prints the same line; on the RBE the parts differed only by the extra spawns.

**Review round 1 of phase 2 (STOP on e7451bb, `docs/reviews/2026-11-19-remote-bwob.md`).** B1: the
remote children of a tree are merged with the local ones by parent-relative path (a remote `x.txt`
was dropped when a local child was `sub/x.txt`). B2: a tree artifact that is a symlink to a tree
(`ctx.actions.symlink` of a directory) is listed through its canonical path: its children come
from the local target directory and the remote output tree, and its value gets the resolved path
when the target is an input of the action (`ActionOutputMetadataStore.java:300`); the action file
system's `stat`/`readdir` canonicalize every component through local symlinks
(`resolveSymbolicLinks`). A side effect that follows Java: with any cache, a symlinked tree whose
target is an input now carries `resolvedPath` (also with `--remote_download_outputs=all`). Bench
additions: `//:dup_tree` (`x.txt`, `sub/x.txt`) with `cat_dir` consumers `//:dup_consumer`,
`//:dup_consumer_local`; `//:sym_tree` (symlink to `//:tree_for_sym`) with `//:sym_tree_consumer`,
`//:sym_tree_consumer_local` (runs in `~/.cache/blaze/bench/bwob-runs/m7`, all three steps MATCH on
exit, summary, files, exec log and gRPC log):

| scenario | files | exec log |
|---|---|---|
| dup consumers, `--remote_executor --remote_download_outputs=minimal --remote_download_regex=.*/sub/.*` | 10/10 ×3 | 3/3, 3/3, 0/0 |
| same with B1's old comparison (reproduction) | DIFF: `dup_tree_dir/x.txt` missing, `dup_consumer_local.txt` differs | 1/3 identical |
| sym consumers, `--remote_executor`, `minimal` | 11/11 ×3 | 3/3, 3/3, 0/0 |
| sym consumers, `--remote_cache`, `minimal` (warm: remote cache hits) | 12/12, 11/11, 11/11 | 3/3, 3/3, 0/0 |
| `//:sym_tree //:sym_tree_consumer`, `--remote_executor`, `toplevel` | 11/11 ×3 | 2/2, 2/2, 0/0 |
| all five new targets, `all`, remote exec / remote cache | 18/18 ×3 each | 6/6, 6/6, 0/0 |
| regression: `min-regex-exec-build`, `tl-cache-build` (phase 2 targets) | 27/27 ×3; 38/38, 32/32, 32/32 | 13/13, 13/13, 0/0 |

One `tl-cache-build` cold run had a `FindMissingBlobs` difference on `//:use_sym`: its output has
the contents of `a.txt`, which `//:gen_a` uploads concurrently, so whether the blob is reported
missing is a race (two oracle runs differ the same way); the repeated run matched.

**A lost remote blob is a sticky, loud failure.** When a remote or disk cache loses a blob that a
later build must download (eviction, a deleted or collected `--disk_cache`), Blaze fails loudly
(`LostInputsExecException (...)`: action rewinding is not ported). The action cache entries with
remote metadata stay, so every later build fails at the same place until `blaze clean`, or one
build with `--remote_download_outputs=all` (whose `RemoteOutputChecker` does not trust remote
metadata), restores the outputs. `RemoteActionInputFetcher.doDownloadFile` turns only errors of
the download itself into `CacheNotFoundException`: creating the temporary file's directory or an
empty file, and an interruption, are reported as they are.
