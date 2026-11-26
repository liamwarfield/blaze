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
| `text_format` | re-export of `blaze_protos::text_format`: the port of protobuf-java's `TextFormat` reader for `Platform` (`remote_execution_properties`), shared with the execution log in `blaze_exec` |

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
probe-trees,probe-trees2,probe-inprocess,shard-probe,attempts-probe,evict-probe,caps-matrix,
tls-probe,platform-probe,pathcheck-probe,recreate-probe,outpath-symlink-probe,bep-uris,
bep-interrupt-probe,stale-probe,symlink-probe,srcdir-probe,cplookup-probe,sigpipe-probe,
reset-peer,regex-probe,discovery-probe}` (each prints `help`;
each takes `oracle` or a Blaze binary, so a result can be reproduced with another binary).
Every wrapper records the command's exit status inside its systemd unit
(`scripts/remote/unit-supervisor`): a death by signal is recorded as 128+N with the signal's
name, never as 0.
The table is the state after review round 1 (working tree on top of c74f487, binary recorded as
`blaze-c74f4870286a-dirty-6cb9e325`); what round 2 added, and the binary it was measured with,
is in docs/reviews/2026-11-19-remote-response.md, "Round 2 response".

| phase | what | evidence |
|---|---|---|
| P1 disk cache | DiskCacheClient, CombinedCache, UploadManifest, MerkleTreeComputer, RemoteExecutionService, RemoteSpawnCache | brotli: 43/43 action digests equal Bazel's; Bazel's cache read by Blaze `43 disk cache hit`, Blaze's cache read by Bazel `53 processes: 43 disk cache hit, 10 internal`; both caches hold the same 43 `ac` and 165 `cas` names, ActionResults equal except execution timestamps. re2 (`test`): both directions `716 processes: 550 disk cache hit, 165 internal, 1 linux-sandbox`, which is what Bazel prints on its own cache. |
| P2 remote cache | channel, capabilities, GrpcCacheClient, ByteStreamUploader, RequestMetadata, LoggingInterceptor | `--remote_grpc_log` vs Bazel's, RPC by RPC after normalisation: brotli cold 295/295, warm 51/51; re2 warm 600/600; re2 cold 3453/3535 (the other 82 RPCs all belong to the 21 TestRunner actions, whose outputs are timing-dependent). Interchange through the worker, both directions: brotli 43/43, re2 550/550 `remote cache hit`. |
| P3 remote execution | GrpcRemoteExecutor, RemoteExecutionCache, RemoteSpawnRunner | brotli: `53 processes: 10 internal, 43 remote` on both, outputs equal to local execution, log MATCH (504 RPCs each, attribution-insensitive). re2 with `build //:all //python:all` (Bazel's worker drops long test operations; the oracle itself fails `test` with `NOT_FOUND: Operation not found`): `695 processes: 165 internal, 1 linux-sandbox, 529 remote` on both, log MATCH (6095 RPCs each). |
| P4 build without the bytes | RemoteOutputChecker, AbstractActionInputPrefetcher, RemoteActionInputFetcher, RemoteActionFileSystem (a per-action overlay file system in `blaze_exec`), remote metadata in the action cache | brotli with an oracle-seeded disk cache, toplevel / minimal / all: build, build again, build after one source edit print the oracle's lines (`43 disk cache hit, 10 internal`; `1 process: 52 action cache hit, 1 internal`; `2 processes: 51 action cache hit, 1 internal, 1 linux-sandbox`) with identical `bazel-bin` after all 9 steps; tree artifacts (`probe-trees`), in-process readers (`probe-inprocess`), a remote-only `test.xml`: identical. |
| default path | no remote flag | referee unchanged; `scripts/gate/compare-scores corpus/scores/blaze-ef66f55e904e.referee2.json <new>`: rows 39, gained 0, regressed 0, PASS. One intended change: the action cache salt is Java's (review R16), so Blaze reuses a Bazel-written output base (52 action cache hits where 43 actions re-ran before). |

Gaps that are loud: see "Not ported (loud)". Deviations from Bazel that are not loud: see
"Known deviations".

- Build event artifact uploader (review R12, 2026-11-19): `byte_stream_build_event_artifact_uploader.rs`
  ports `ByteStreamBuildEventArtifactUploader(Factory)`; `RemoteModule::create_build_event_artifact_uploader`
  is the "remote" factory delegate, selected by `--experimental_build_event_upload_strategy`
  through `blaze_bep::runtime::BuildEventArtifactUploaderFactoryMap` (built per command in
  `blaze_runtime` `build_remote.rs`). With a gRPC cache the BEP files name their files
  `bytestream://<authority or --remote_bytestream_uri_prefix>[/<instance>]/blobs/[<fn>/]<hash>/<size>`;
  `--remote_build_event_upload=minimal|all` decides which files are uploaded. Evidence (brotli
  `build //...`, cold and warm, both modes): docs/reviews/2026-11-19-remote-response.md, R12.
  A transport's `close` returns a future and does not wait for the uploads (`FileTransport.java:225-274`),
  so an interrupted build cancels them (`RemoteExecutionService.shutdown`) and writes `file://`
  URIs, as Bazel (`scripts/remote/bep-interrupt-probe`).

## Security posture

Root ruling 2026-11-19 (docs/decisions.md): for output symlink paths, Tree node names and
server-log keys Blaze stays identical to Bazel 9.3.2, which does not confine them; Blaze does
not harden beyond Bazel. Each site is marked `// PORT: identical to Bazel, not hardened
(decision 2026-11-19)`, next to a `SECURITY:` comment that describes it. The sites:

| What the server controls | Rust site | Java |
|---|---|---|
| `ExecuteResponse.server_logs` keys: `..` and absolute keys escape the log directory | `remote_execution_service.rs` `maybe_download_server_logs` | `RemoteExecutionService.java:2075` |
| Tree node names (`FileNode`/`SymlinkNode`/`DirectoryNode.name`): a file can land anywhere under the exec root, not only under its tree root (outside the exec root is a crash in both) | `remote_execution_service_download.rs` `parse_directory`; checked only by `relative_to(exec_root)` in `download_outputs` | `RemoteExecutionService.java:1086-1200` |
| `OutputFile`/`OutputDirectory`/`OutputSymlink.path` (normalized `getRelative`) | `remote_execution_service_download.rs` `parse_action_result_metadata` | `RemoteExecutionService.java:1086-1200` |
| Output symlink paths and targets (absolute, `..`, dangling targets accepted; symlink path not compared with the exec root) | `remote_execution_service_download.rs` `parse_action_result_metadata`, `create_symlinks` | `RemoteExecutionService.java:950` `createSymlinks` |
| Inlined `OutputFile.contents` and inline stdout/stderr are not compared with their digests | `remote_execution_service_download.rs` `download_outputs` | `RemoteExecutionService.java` `downloadOutputs` |
| Symlink targets found in the output tree are re-planted verbatim by the prefetcher | `abstract_action_input_prefetcher.rs` `get_symlinks`, `plant_symlink` (`SECURITY:` comments) | `AbstractActionInputPrefetcher.java` |

The digest checks Java performs are all kept (rule 7 above). Metadata injected for a remote
output goes to the file system of the action that produced it and is read back only for that
action's declared outputs, as in Java (review R7, closed). To find the sites, grep for the
marker text; line numbers are not kept here.

Credential helpers (`--credential_helper`, `--credential_helper_timeout`,
`--credential_helper_cache_duration`; remote module, downloader and Bzlmod registries) are ported
in `blaze_repo_fetch::authandtls::credentialhelper` (round 4, docs/reviews/2026-11-19-remote-response.md);
the credential cache lives as long as the process (Bazel: the server), so without a Blaze server
a second command calls the helper again. `--experimental_remote_scrubbing_config` is parsed by
`blaze_optdefs`' `ScrubberConverter` and built by `scrubber.rs`.

Server certificates (ruling D16, round 5): rustls' webpki verifier plus the JDK's direct trust
(`authandtls/jdk_server_cert_verifier.rs`): a server certificate byte-identical to one in
`--tls_certificate` is trusted without a chain, then its host name is checked. Blaze does not
validate chains of X.509 v1/v2 certificates (refused with Blaze's own text; Bazel accepts them),
so `remote_execution_tls_test` and `remote_execution_mtls_test` lose `test_remote_grpcs_cache`
and `test_remote_grpc_cache` (declared gap). Signature algorithms are the oracle's Netty list as
far as ring has them (no Ed25519). Fixtures: `tests/jdk_server_cert_fixtures_test.rs` (full
handshakes, verdicts from the oracle's JDK by `scripts/jdk-certs/run`).

Where Blaze is stricter than Bazel (ruling 2026-11-19, docs/decisions.md): TLS for remote
endpoints uses rustls with the webpki verifier and no hand-written verifier, so an X.509 v1
certificate (server or client) is rejected, where Bazel (JDK/Netty) accepts it. Effect: an
endpoint with a v1 certificate fails with "Failed to query remote execution capabilities:
invalid peer certificate: ... UnsupportedCertVersion" (the build fails unless the local-fallback
flags are set). Bazel's own test certificates (`src/test/testdata/test_tls_certificate`) are
v1, so `remote_execution_tls_test` and `remote_execution_mtls_test` each fail 2 functions on
Blaze for this reason. With X.509 v3 certificates TLS and mTLS behave as Bazel
(`scripts/remote/tls-probe`).

## Not ported (loud)

Each of these fails with `unported!` (a crash with `blaze: not ported yet: ...`) at the point
where Bazel would enter it; none is silently ignored.

| Feature / flag | Where it fails |
|---|---|
| Google credentials (`--google_credentials`, `--google_default_credentials`) | `authandtls/google_auth_utils.rs` |
| HTTP/HTTPS remote cache (`--remote_cache=http(s)://`) | `combined_cache_client_factory.rs` `createHttp`, `remote_module.rs` `createCredentials` |
| Content-defined chunking (`--experimental_remote_cache_chunking`) | `combined_cache.rs` `Chunking.*`, `uploadChunked`; `grpc_cache_client.rs` split/splice |
| Remote output service (`--experimental_remote_output_service`); the "must be used with a cache" exit and the `--disk_cache` warning are ported | `remote_module.rs` setup, at Java's channel creation for it |
| Remote downloader (`--experimental_remote_downloader`) | `remote_module.rs` setup; `blaze_runtime` `unported_options.rs` |
| `--remote_max_connections` / `--remote_max_concurrency_per_connection` other than 100 with a gRPC endpoint (one HTTP/2 connection per endpoint, no `DynamicConnectionPool`); ignored for `--disk_cache` alone, as in Java | `remote_module.rs` `create_channel` |
| `--experimental_remote_cache_lease_extension` with `--norewind_lost_inputs` (`RemoteLeaseExtension`); with the default `--rewind_lost_inputs` Java's "has no effect" warning is printed | `remote_module.rs` `executor_init` |
| Disk cache garbage collection (`--experimental_disk_cache_gc_max_size` / `_max_age`, with `_idle_delay`): a server idle task, and Blaze has no server | `remote_module.rs` setup |
| Action rewinding (`--rewind_lost_inputs`, Bazel's default) when an input is lost from the remote cache: the message names `--norewind_lost_inputs`, with which every lost-input source behaves as Bazel (retry of the invocation, exit 39 when the retries are used up); the stale local action cache entries are purged before the failure | `blaze_sky_exec` `skyframe_action_executor.rs` `handle_lost_inputs` |
| `argv[0]` conversion for a Windows execution platform (`@platforms//os:windows` on the execution platform) | `remote_execution_service.rs`, `WindowsOsPathPolicy.postProcessPathStringForExecution` |
| Remote flags on commands that execute actions other than `build` and `test` (`run`, `coverage`, ...: the commands themselves are not ported). `query`, `cquery`, `aquery`, `info` run `RemoteModule.beforeCommand`/`afterCommand` as Bazel | `blaze_runtime` `unported_options.rs` |
| Path mapping, persistent-worker signatures | `common/remote_path_resolver.rs`, `remote_execution_service.rs` |

## Known deviations (not loud; each by a ruling or listed for one)

Work in progress (remote-5, not reviewed): the remote repo contents cache (startup
`--experimental_remote_repo_contents_cache`) is ported up to the overlay file system
(`remote_external_overlay_file_system.rs`), the cache (`remote_repo_contents_cache_impl.rs`) and
its wiring; a cached repository is served from memory for loading (BUILD and .bzl files). NOT
ported and NOT loud yet: the remote metadata of a source file of an in-memory repo
(`FileStatusWithMetadata` through `FileStateValue` and the source artifact's value), with which
Java prefetches such a file for a local action. A local action that reads a file of a repository
served from the cache fails (the file is not on disk), where Bazel downloads it first.

| What | Bazel 9.3.2 | Blaze | Why |
|---|---|---|---|
| Disk cache action-result integrity check | skipped under the default `--rewind_lost_inputs` (`RemoteModule.java:700-706`) | always on | Ruling 2026-11-19: rewinding is not ported, so a disk-cache action result whose blobs are not in the disk CAS must be a miss, not a crash. Visible where a disk cache was filled under build without the bytes together with a remote cache: Bazel `disk cache hit`, Blaze re-executes. A disk-cache-only setup is unchanged (brotli, re2). Restore Java's expression when rewinding is ported. |
| X.509 v1 certificates | accepted | rejected | Ruling 2026-11-19, see "Security posture". |
| Transport error causes | Netty/JDK texts (`finishConnect(..) failed: Connection refused: localhost/127.0.0.1:9499`; `General OpenSslEngine problem`) | the status line is Bazel's (`UNAVAILABLE: io exception`, `INTERNAL: http2 exception`, `UNAVAILABLE: ssl exception`, `Unable to resolve host <host>`, ported from grpc-netty `Utils.statusFromThrowable`); the connect cause is rebuilt in Netty's form, but Netty's choice between `127.0.0.1` and `[0:0:0:0:0:0:0:1]` for `localhost` varies between runs of Bazel itself, a certificate rejected locally prints rustls' text. A call whose connection failed gets the connection's first failure as its status, as grpc-netty's shutdown status (`mtls-noclient`: `UNAVAILABLE: ssl exception` on both) | exit codes are equal; not reproducible |
| Lost top-level outputs (`RemoteImportantOutputHandler`, "Lost outputs no longer available remotely") | reported when a target becomes top-level after its action ran in an earlier command of the same server | not reached: Blaze has no server, top-level outputs are downloaded when their action is finalized or its cached result is checked | PORT: skipped, incremental |
| Connection pool | one connection per 100 concurrent calls (`DynamicConnectionPool`) | one HTTP/2 connection per endpoint | performance only with default flags; non-default values are loud |
| `-1` ("unknown") permissions of a remote file's stat | `RemoteFileStatus.getPermissions() == -1` | the bits this file system's `isReadable/isWritable/isExecutable` answer | `blaze_vfs::FileStatus` has no unknown value |

Java patterns (`--remote_download_regex`, the scrubber, every regex-valued option) are compiled
through `blaze_util::java_regex`, a port of JDK 25 `Pattern`'s parser: docs/java-regex.md lists
what is translated, what is loud and what is checked per input.

SIGPIPE: the process ignores it (its sockets must not kill it, as Bazel's server ignores it);
a broken pipe on Blaze's own stdout or stderr is recorded and ends the process by SIGPIPE at
exit, as Bazel's client does (`blaze_client` `signal_handler.rs`, `blaze_events` `io.rs`).

## Bazel's own remote tests

Bazel's own shell and Python integration tests for remote caching and execution (pinned source,
unmodified) run against Blaze with `scripts/upstream-tests/run` (method and oracle baseline:
`corpus/upstream-tests/README.md`, `oracle-9.3.2.json`):

```
scripts/upstream-tests/run --binary <blaze> --label <label> --fresh --test-jobs 4 \
  --file 'remote*' --file '*disk_cache*' --file '*build_without_the_bytes*' \
  --file '*bazel_remote_cache_decompression*'
```

12 targets, 253 functions that pass on the oracle. Runs: `blaze-remote-0` (binary before the
round-2 fixes, tree on top of 750317c): 110 pass; `blaze-remote-1` (binary sha256
`43db45a1caf91dff…`, tree 5a4e4a0 plus three uncommitted `blaze_remote` files): 115 pass. Between
the two: 7 functions gained (`test_multiple_test_attempts`, `test_output_path_is_symlink`,
`test_prefetcher_recreate_tree_dir`, `test_combined_disk_remote_exec_with_flag_combinations`,
`test_platform_default_properties_invalidation_with_platform_remote_execution_properties`,
`test_premature_exit_file_checked_remote_download_minimal`,
`test_shard_status_file_checked_remote_download_minimal`), and 2 that passed for the wrong
reason now fail honestly (`remote_execution_mtls_test` `test_remote_grpc_cache` and
`test_remote_grpcs_cache`: the build used to continue locally after the TLS failure).

`blaze-remote-1`, per test file (results:
`~/.cache/blaze/upstream-test-results/blaze-remote-1/results.json`; a function is counted under
the first failure in its log; every failure was read in the log, not guessed from the name):

| test file | oracle pass | Blaze pass | Blaze fail | causes (ids below) |
|---|---:|---:|---:|---|
| bazel_remote_cache_decompression_test | 1 | 0 | 1 | R-http 1 |
| remote_repo_contents_cache_test | 38 | 0 | 38 | N-mod 33, R-repocc 4, R-gate 1 |
| build_without_the_bytes_test | 63 | 31 | 32 | N-bep 7, R-gate 6, N-cctpl 6, N-java 4, N-server 3, N-ui 2, N-pathmap 2, N-incl 1, N-unused 1 |
| remote_build_event_uploader_test | 20 | 0 | 20 (not run) | N-toollogs 20 |
| remote_downloader_test | 4 | 0 | 4 | R-dl 4 |
| remote_execution_http_test | 15 | 4 | 11 | R-http 11 |
| remote_execution_mtls_test | 4 | 2 | 2 | R-tlsv1 2 |
| remote_execution_sandboxing_test | 3 | 3 | 0 | |
| remote_execution_test | 96 | 71 | 25 | N-bep 6, N-fetch 4, R-cred 3, N-excl 3, N-unused 3, R-gate 2, R-http 1, R-scrub 1, N-server 1, N-modexec 1 |
| remote_execution_tls_test | 4 | 2 | 2 | R-tlsv1 2 |
| remote_execution_with_xattr_test | 1 | 0 | 1 | N-xattr 1 |
| disk_cache_test | 4 | 2 | 2 | R-gc 1, N-sibling 1 |
| **total** | 253 | 115 | 138 | remote 39, not remote 99 |

Remote causes (39 functions). Every one is either a feature that fails loudly (see "Not ported
(loud)") or the TLS ruling; none is a silent wrong result:

| id | functions | cause |
|---|---:|---|
| R-http | 13 | HTTP remote cache (`--remote_cache=http://...`): `RemoteModule.createCredentials (HTTP remote cache)` |
| R-gate | 9 | remote flags on `run` (5) and `coverage` (4): the commands themselves are not ported |
| R-repocc | 4 | remote repo contents cache (`--experimental_remote_repo_contents_cache`); these would first hit N-mod |
| R-dl | 4 | `--experimental_remote_downloader` |
| R-tlsv1 | 4 | Bazel's test certificates are X.509 v1, which rustls rejects (ruling: fail closed, "Security posture") |
| R-cred | 3 | `--credential_helper` |
| R-scrub | 1 | `--experimental_remote_scrubbing_config` |
| R-gc | 1 | disk cache garbage collection |

Causes outside the remote subsystem (99 functions), for their owners:

| id | functions | missing |
|---|---:|---|
| N-mod | 33 | `mod dump_repo_mapping` (called by the test class's `RepoDir` helper) |
| N-toollogs | 20 | BEP `buildToolLogs` file entries (`command.profile.gz`, the compact execution log). The first function fails on the missing profile entry, prints the whole BEP into its log, and `unittest.bash` then dies with "Argument list too long", so no function of the target gets a result. The upload itself works (`bytestream://` URIs for the outputs) |
| N-bep | 13 | `--build_event_text_file` (TextFormat transport) |
| N-cctpl | 6 | `CcStarlarkInternal.createCppCompileActionTemplate` |
| N-java | 4 | `java_common_internal_do_not_use.expand_java_opts` |
| N-server | 4 | no persistent server: Bazel keeps executed actions in memory between two commands of a test, Blaze re-checks the action cache (expected `1 process: 1 internal`, got `2 action cache hit, 1 internal`), and "Lost outputs no longer available remotely" needs that memory |
| N-unused | 4 | Starlark action with `unused_inputs_list`: panic "Not an input-discovering action" (`blaze_exec` `action_execution.rs`), not an `unported!` |
| N-fetch | 4 | `fetch` command |
| N-excl | 3 | exclusive tests (`BuildTool.runExclusiveTests`) |
| N-ui | 2 | `--experimental_ui_debug_all_events` (`UiStateTracker.writeProgressBar`) |
| N-pathmap | 2 | path stripping (`StrippingPathMapper.tryCreate`) |
| N-incl | 1 | include scanning (`CppCompileAction.discoverInputs`) |
| N-modexec | 1 | `--modify_execution_info` |
| N-xattr | 1 | `--unix_digest_hash_attribute_name` |
| N-sibling | 1 | external repository links under `--experimental_sibling_repository_layout` (`IncrementalPackageRoots`) |

Fixed in round 2 after this run found them (they were wrong behaviour, and the independent review
had not found them): the `RemotePathChecker` stub, `OutputService.startBuild`, test attempts
under `--remote_download_minimal`, a regular file where a tree directory belongs, the mTLS
handshake failure downgraded to a warning, the silently ignored remote repo contents cache
(now loud). Details: docs/reviews/2026-11-19-remote-response.md, "Round 2 response".
