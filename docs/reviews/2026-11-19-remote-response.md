# Response: remote review 2026-11-19 (all fixers, merged)

- Reviewer's verdict: STOP at `ef66f55` (docs/reviews/2026-11-19-remote.md).
- This document merges the dispositions of the three fixers of the round: "internals"
  (`blaze_remote` outside the download/BwoB files: R3, R4, R8 Merkle half, R9-R15, R18 crate
  part), "integration" (R1, R2, R5, R6, R7, R16, R17, R18 part) and the follow-up fixer (the
  items the first two left open because they crossed file boundaries: R6 leftovers, R8 download
  half, R11, R12 call site, R15 exit codes, the `byte_stream_uploader_test` flake, S4
  `FileArtifactValue` equality and stale-downloads cleanup). Each finding lists state, files,
  Java, tests, oracle evidence and what stays open.
- Tree: `c74f487` (Checkpoint 11, which contains all three fixers' changes). Expected values
  always come from the Java tests or the oracle (Bazel 9.3.2).
- Tests on `c74f487`: `scripts/cargo-capped test -p blaze_remote --no-fail-fast`: 554 passed,
  0 failed, 63 ignored (43 binaries); `-p blaze_exec`: 448 passed, 0 failed, 76 ignored (37
  binaries). Earlier in the round (integration fixer, its tree): `-p blaze_sky_exec` 45/0/11,
  `-p blaze_buildtool` 77/0/44, `-p blaze_vfs` 90/0/8. `clippy --all-targets --no-deps -D
  warnings` clean for blaze_remote, blaze_exec, blaze_runtime (and earlier blaze_vfs,
  blaze_sky_exec, blaze_buildtool).
- Binaries used for evidence (copied before each run, under `~/.cache/blaze/work/remote/`):
  `rx-blaze-1724`, `rx-blaze-r6b` (integration), `target/debug/blaze` of the internals fixer
  (`ry-*` runs), `rz-blaze-1` (follow-up fixer, built from the tree of `c74f487`). "Before"
  binary: `sb-bin/blaze-final`.
- Tools (each prints `help`): `scripts/remote/caps-matrix`, `probe-trees`, `probe-inprocess`,
  `evict-probe`, `second-build`, `ws-run`, `worker`.

## R1. Tree artifact outputs lose their remote-only children: FIXED

- Files: `blaze_sky_exec/src/skyframe_action_executor.rs`
  (`construct_tree_artifact_value_from_filesystem` lists the action's own remote output tree,
  through the action file system's `readdir`), `blaze_exec/src/exec/remote_action_file_system.rs`.
- Java: `remote/RemoteExecutionService.java:1334-1358`, `remote/RemoteActionFileSystem.java:667-706,831`.
- Tests: `blaze_exec/tests/skyframe_tree_artifact_value_test.rs` (7 pass, 1 ignored),
  `remote_action_file_system_test.rs` `readdir_from_remote_output_tree*`.
- Oracle: `scripts/remote/probe-trees` oracle (`pt-rxo`) vs Blaze `rx-blaze-1724` (`pt-rxb2`):
  run1 `4 processes: 2 disk cache hit, 1 internal, 1 linux-sandbox`, run2 `1 process: 3 action
  cache hit, 1 internal`, run3 `1 process: 1 action cache hit, 1 internal`, exit 0 each,
  bazel-bin identical in all three steps (5 files, sha256).
- Open: none.

## R2. `finalizeAction` never called: FIXED

- Files: `blaze_sky_exec/src/skyframe_action_executor.rs` (`finalize_action` after the output
  check when `--output_tree_tracking` and the remote output service are on; the symlink output's
  metadata from the action file system's stat).
- Java: `skyframe/SkyframeActionExecutor.java:1340-1343`, `remote/RemoteOutputService.java:170-174`,
  `remote/AbstractActionInputPrefetcher.java:849`, `ActionOutputMetadataStore.java:478,580`.
- Oracle: re2 `build //:all //python:all --remote_executor=grpc://localhost:9301`, fresh worker
  each, oracle (`rx-r2-oracle`) vs Blaze (`rx-r2-blaze2`): ActionCache/GetActionResult 529,
  ByteStream/Read 93, ByteStream/Write 4414, Capabilities 1, FindMissingBlobs 9636, Execute 529,
  items 15202, `RESULT: MATCH`; bazel-bin identical (3951 entries, sha256 of files and symlink
  targets).
- Open: none.

## R3. Capabilities gate: FIXED

- Files: `grpc/channel.rs` (`ReferenceCountedChannel::with_server_capabilities_gate`;
  `start_call` obtains the connection first: every call except `GetCapabilities` waits for the
  verified capabilities and fails with their error before any interceptor runs; the failure is
  the source of the call's status, `errors::call_not_started`, and `status_to_io_error` gives
  back the `IOException` itself); `remote_module.rs` `create_channel` (gate when the
  requirement is not NONE; `capabilities_failure_cause`); `errors::find_cause` walks through
  `RemoteExecutionCapabilitiesException`; `grpc_cache_client.rs` `get_authority`.
- Java: `GoogleChannelConnectionFactory.java:101-121,146`, `ReferenceCountedChannel.java:77,131`,
  `RemoteSpawnCache.java:189-197`, `RemoteSpawnRunner.java:591`.
- Tests: `tests/remote_module_test.rs`: the five `testVerifyCapabilities_*` that do not need the
  downloader; `verify_capabilities_missing_compressor_fails_every_call`,
  `verify_capabilities_unimplemented_capabilities_fail_with_status_message`.
- Oracle (`scripts/remote/caps-matrix`, runs `ry-caps-o-*` vs `ry-caps-b2-*`, re-run with
  `rz-blaze-1` as `ry-caps-rzb-*`: all nine exit codes equal again):

| scenario | oracle | blaze | lines |
|---|---|---|---|
| unreachable `--remote_cache` | 34 | 34 | differ only in the transport text (`finishConnect(..) failed: Connection refused: localhost/127.0.0.1:9499` vs `Connection refused (os error 111)`) |
| same + `--remote_local_fallback --incompatible_remote_local_fallback_for_remote_cache` | 0 | 0 | identical |
| unreachable `--remote_executor` | 34 | 34 | transport text, as above |
| same + `--remote_local_fallback` | 0 | 0 | identical |
| `--remote_cache_compression`, worker without ZSTD (cache) | 34 | 34 | identical |
| same (executor) | 34 | 34 | identical |
| worker with `--expected_authorization_token`, no credentials | 34 | 34 | identical |
| `grpc://` to an HTTP/1 server | 34 | 34 | transport text (`INTERNAL: http2 exception` vs `INTERNAL: h2 protocol error: http2 error`) |
| `grpcs://` + unreadable `--tls_certificate` (R14) | 0 | 0 | identical |

- Open: a wrong `--remote_instance_name` and an API-version mismatch are not reachable with
  Bazel's worker.

## R4. Interrupts: FIXED

- Files: `runtime.rs` (`get_from_future` and `block_on_interruptibly` interruptible,
  `runtime::sleep`), `retrier.rs` (interruptible sleep, `Thread.interrupted()` check),
  `remote_retrier.rs` (CANCELLED permanent on an interrupted thread), `util/utils.rs`
  (`waitForBulkTransfer`), `remote_execution_service.rs` (semaphore), `combined_cache.rs`
  (`awaitTermination`), `util/async_task_cache.rs` (`shutdownNow` cancels running executions),
  `BackgroundTaskExecutor`.
- Java: `Utils.getFromFuture`, `Retrier.java:262`, `RemoteRetrier.java:54`,
  `RemoteExecutionService.java:421,2119-2125`, `CombinedCache.java:881`, `Utils.java:555`.
- Tests: `retrier_test::interrupts_should_not_be_retried_flag` un-ignored;
  `interrupt_during_backoff_sleep_ends_the_retries`,
  `remote_retrier_test::cancelled_is_permanent_on_an_interrupted_thread`,
  `utils_test::wait_for_bulk_transfer_interrupted_cancels_transfers`,
  `combined_cache_test::shutdown_now_cancel_in_progress_uploads`,
  `async_task_cache_test::shutdown_now_cancel_in_progress_tasks`.
- Oracle: `ws-run --sigint-on-glob` on a 30 s genrule with `--remote_executor` (worker `ry-int`):
  oracle exit 8 after 0.4 s, Blaze exit 8 after 0.0 s; ERROR and summary lines identical.
- Open: none.

## R5. In-process actions read remote-only files locally: FIXED

- Design: a per-action `RemoteActionFileSystem` (Java `remote/RemoteActionFileSystem.java`) as an
  overlay `blaze_vfs::FileSystem` in `blaze_exec/src/exec/remote_action_file_system.rs`, created
  per action execution from the action's input metadata, `updateContext(action)` before
  execution; `ActionExecutionContext::get_input_path` / `convert_path` and
  `blaze_exec::analysis::actions::get_input_path` (`TransformResolver.toPath`) return paths on
  it (identity without one: default path unchanged).
- Files: `blaze_exec/src/exec/{remote_action_file_system,action_execution_context,spawn_runner,abstract_spawn_strategy,standalone_test_strategy}.rs`,
  `analysis/actions/{mod,local_template_expansion_strategy}.rs`,
  `blaze_sky_exec/src/{skyframe_action_executor,action_execution_function}.rs`.
- Java: `SymlinkAction.java:302-323`, `RemoteActionFileSystem.java:382`,
  `StandaloneTestStrategy.java:857`.
- Tests: `blaze_exec/tests/remote_action_file_system_test.rs` (35 pass, 2 ignored: 27 ports of
  `RemoteActionFileSystemTest(Base)`, 3 in-process reader tests run with and without the action
  file system, 1 isOutput test, 2 prefetch tests).
- Oracle, (a)+(b): `scripts/remote/probe-inprocess` (disk cache seeded by the oracle, fresh
  output base, default flags): oracle `10 processes: 3 disk cache hit, 7 internal`, exit 0;
  Blaze `rx-blaze-r6b` identical, bazel-bin identical; before: "is not a file" and "failed to
  load template file", exit 1.
- Oracle, (c) (this round): workspace `~/.cache/blaze/work/remote/rz-txml-src` (a Starlark test
  rule whose test writes its own `$XML_OUTPUT_FILE`); disk cache seeded by the oracle
  (`rz-txml-seed`: `7 processes: 6 internal, 1 linux-sandbox`), then `test //:t --disk_cache=..
  --remote_download_outputs=minimal` in a fresh output base: oracle (`rz-txml-o`) and Blaze
  `rz-blaze-1` (`rz-txml-b`) both `//:t (cached) PASSED`, `7 processes: 1 disk cache hit, 6
  internal`, exit 0, `testlogs/t/` has the same entries (test.cache_status, test.log,
  test.outputs, test.outputs_manifest, test.raw_splitlogs) and no local `test.xml` in either:
  no XML-generating spawn runs.
- Open: `parseTestResult(xmlOutputPath)` still reads the local file (Java reads through the
  action file system, which does not download outputs either; same answer).

## R6. A blob lost from the remote cache: PARTIAL (no-rewinding path ported with retry; rewinding loud)

- Oracle finding: Bazel 9.3.2's default is `--rewind_lost_inputs=true`; exit 39 and the
  dispatcher retry are the `--norewind_lost_inputs` path. Action rewinding is not ported: with
  the default flag an action that loses inputs fails loudly
  (`unported!("ActionRewindStrategy.prepareRewindPlanForLostInputs", ...)`).
- Ported (integration): `AbstractSpawnStrategy.prefetchInputs` catch
  (`blaze_exec/src/exec/action_execution_context.rs`, `BulkTransferError` seam filled by
  `remote_action_input_fetcher.rs`); `LostInputsExecException` / `LostArtifacts` /
  `LostInputsActionExecutionException` (`blaze_exec/src/actions/{exec_exception,action_execution}.rs`;
  PORT: carried on `EnvironmentalExecException.lost_inputs`);
  `ActionRewindStrategy.checkRewindingEnabled` in `skyframe_action_executor.rs::handle_lost_inputs`;
  `LostInputsEvent` → `RemoteExecutionService.onLostInputs` + `LeaseService.onLostInputs`
  (`blaze_remote/src/lease_service.rs`); the `BlazeCommandDispatcher` retry
  (`blaze_runtime/src/dispatcher.rs`); `knownMissingCasDigests` kept for the process
  (`build_remote.rs`).
- This round:
  - (a) `buildRequestIdOverride`: a retried attempt keeps the build request id of the attempt it
    retries (`--build_request_id` still wins; a new `exec` starts without override). Files:
    `dispatcher.rs` (catch sets the override, `exec` resets it), `build_remote.rs`
    (`build_request_id`, `BUILD_REQUEST_ID_OVERRIDE`, `LAST_BUILD_REQUEST_ID`; PORT: Blaze's
    `CommandEnvironment` has no build request id, its only reader is the remote module). Java:
    `BlazeCommandDispatcher.java:292,316-319`, `CommandEnvironment.java:335-339`. Test:
    `build_remote::tests::retried_attempt_keeps_the_build_request_id`. Evidence: the eviction
    probe still retries and succeeds (`evict-probe run rz-blaze-1 rz-b1 9520
    --norewind_lost_inputs`: "Linking brotli failed: Lost inputs no longer available remotely:
    ...", `52 processes: 42 remote cache hit, 10 internal`, "Found transient remote cache error,
    retrying the build...", `39 processes: 14 action cache hit, 35 remote cache hit, 2 internal,
    2 linux-sandbox`, exit 0); the id itself is not visible end to end (each attempt rewrites
    `--remote_grpc_log`, in Java too).
  - (b) stale-downloads cleanup: Java runs `getTempPathGenerator` in `RemoteModule.setup` for
    every command that has `RemoteOptions`, and `RemoteOptions` is a common command option
    (`RemoteModule.getCommonCommandOptions`, `RemoteModule.java:1341`): every command, with or
    without remote flags; `afterCommandTask` deletes the directory again. Blaze now does both for
    `build`/`test` without remote flags (`build_remote::before_command`,
    `placeholder_after_command`; `remote_module::get_temp_path_generator` is public). Oracle
    (`rz-stale-o` vs `rz-stale-b`, stale `<execroot>/bazel-out/_tmp/actions/remote/junk`, no
    remote flags): both `WARNING: Found stale downloads from previous build, deleting...`, the
    directory is gone afterwards, exit 0. Not done for other commands (see "Open").
  - (c) R5(c) oracle run: see R5.
  - (d) `FileArtifactValue` equality: see S4 below.
- Oracle (integration, `scripts/remote/evict-probe`): `--norewind_lost_inputs`: oracle
  (`rx-ev-o2`) and Blaze (`rx-ev-b5`) identical first attempt, retry, exit 0, bazel-bin
  identical (55 entries); second attempt oracle `6 action cache hit` vs Blaze `14` (Bazel's
  server keeps Skyframe; PORT: skipped, incremental). `--experimental_remote_cache_eviction_retries=0`:
  both "Unexpected lost inputs (pass --rewind_lost_inputs to enable recovery): ...", exit 1.
  Default flags: oracle exit 0 (rewinding), Blaze loud `unported!`.
- Open: action rewinding; `RemoteLeaseExtension`; exit 39 after exhausted retries not
  reproduced end to end; the stale-downloads cleanup for commands other than build/test.

## R7. (security) Build-wide remote output tree: FIXED

- Injection goes into the spawn's own `RemoteActionFileSystem`
  (`SpawnExecutionContext::get_remote_action_file_system`, `remote_execution_service_download.rs::inject_remote_file`),
  read back only by that action's output metadata store for its declared outputs.
- Java: `RemoteExecutionService.java:1238`, `RemoteActionFileSystem.injectRemoteFile` :308.
- Test: `blaze_sky_exec` `injected_metadata_for_another_actions_output_does_not_replace_its_digest`.
- Open: none.

## R8. Strings seam: FIXED

- Merkle half (internals): `merkletree/merkle_tree_computer.rs` `internal_to_unicode` through
  the seam (`MerkleTreeComputer.java:629,639,659`). Test
  `merkle_tree_computer_test::build_for_files_non_ascii_names_are_unicode_on_both_sides_of_the_seam`.
- Download half (this round): `remote_execution_service_download.rs` wraps every proto string
  where Java does, checked against the Java calls: `unicodeToInternal` for `FileNode.name`,
  `SymlinkNode.name`/`.target`, `DirectoryNode.name` (`RemoteExecutionService.java:1086,1096,
  1097,1101`), `OutputDirectory.path` (:1134), `OutputFile.path` (:1177), `OutputSymlink.path`
  and `.target` (:1195-1196), the reused outputs of `waitForAndReuseOutputs` (:1625);
  `internalToUnicode` for the output path handed to `CombinedCache.downloadFile` and to the
  `DownloadProgressReporter` (:872, :878). The `HashMap.hash` of a path (:116) hashes the
  internal bytes and needs no change. The test fixture's `newSpawnFromResult` now calls
  `unicodeToInternal` as Java's does (`RemoteExecutionServiceTest.java:3205`).
- Test: `remote_execution_service_test::download_outputs_non_ascii_names_are_unicode_on_both_sides_of_the_seam`
  (output file, output directory with a file and a symlink, output symlink; runs with
  `strings_are_internal` false and true; with the old identity the internal run fails with
  "mandatory output ... was not created").
- Open: none (end to end only after the strings switch).

## R9. `argv[0]` for a Windows execution platform: NOT FIXED (needs other owners' files)

- `blaze_exec::actions::PlatformInfo` is filled in `blaze_exec/src/actions/action_adapters.rs`
  `convert_platform` from `blaze_actions::PlatformInfo`, whose trait exposes only `label()`,
  `add_to(fp)` and `exec_properties()`: the constraints cannot be read without changing
  `blaze_actions` (checked this round; there is no downcast or other accessor).
- Exact change needed:
  1. `crates/blaze_actions/src/actions/platform_info.rs`, trait `PlatformInfo`: add
     `fn constraint_value(&self, setting: &Label) -> Option<Label>;` (Java
     `ConstraintCollection.get(ConstraintSettingInfo)`), implemented by every implementor
     (`PlatformInfoData`: look `setting` up in `constraints`, then in `constraints_parent`).
  2. `crates/blaze_exec/src/actions/action_execution_metadata.rs` `PlatformInfo`: add
     `pub os_constraint: Option<Label>` (the value for `@platforms//os:os`); fill it in
     `action_adapters.rs` `convert_platform` with
     `p.constraint_value(&Label::parse_canonical("@platforms//os:os"))` (blaze_exec owner can
     do this once step 1 exists).
  3. `crates/blaze_remote/src/remote_execution_service.rs` (~:1132, Java
     `RemoteExecutionService.java:312-316`, `ConstraintConstants.getOsFromConstraintsOrHost`):
     when the spawn's execution platform has `os_constraint == @platforms//os:windows`, call
     `blaze_util::unported!("WindowsOsPathPolicy.postProcessPathStringForExecution")`.
- Listed in docs/remote.md.

## R10. Upload resume at the end of a blob: FIXED

- `chunker.rs` `seek_or_null_pointer`; `byte_stream_uploader.rs` `Writer::write` turns it into
  `onError(CANCELLED "Failed to call onReady.")`, which is retried and then fails the upload
  (grpc-java `ClientCallImpl`).
- Test: `byte_stream_uploader_test::resume_at_end_of_blob_fails_with_cancelled_on_ready`.

## R11. `AsyncTaskCache`: FIXED

- `util/async_task_cache.rs` rewritten on Java's contract: an execution is a
  `futures::future::Shared` over the task (the design of `remote_execution_cache.rs`); every
  `execute` that joins it is a subscriber holding a clone and driving it; the cache keeps only a
  `WeakShared`. The execution keeps running while any subscriber remains; a subscriber that goes
  away (its future dropped, Java `ExecutionDisposable.dispose()`) neither restarts nor cancels
  it; the last one removes it from `inProgress` and drops the task (`Execution.remove`,
  `upstreamDisposable.dispose()`); `shutdownNow` aborts every running execution, whose
  subscribers get `CancellationException: cancelled`; `awaitTermination` waits for TERMINATED
  (also in ACTIVE); `getSubscriberCount` counts live subscribers. Java:
  `AsyncTaskCache.java:142-372,458-492`.
- Tasks no longer borrow from their caller (`F: Future + Send + 'static`):
  - `common/remote_cache_client.rs`: `uploadFile` / `uploadBlob` / `uploadBytes` moved from the
    trait's provided methods to `RemoteCacheClientUploads`, implemented for
    `Arc<T: RemoteCacheClient + ?Sized>`: the task owns a clone of the client `Arc`,
    `context.detached()`, the digest and an `Arc<dyn Blob>`; `dedup_upload` requires a
    `'static` upload.
  - `combined_cache.rs`: `upload_blob` / `upload_blob_with_force` take `Arc<dyn Blob>`;
    `upload_manifest.rs` adapted.
  - `abstract_action_input_prefetcher.rs`: the download and symlink-planting tasks of
    `downloadCache` own their inputs (input, metadata, final path, tree root, symlink, exec
    root, an `ActionMetadataSnapshot` of the action for `doDownloadFile`'s request metadata)
    and the prefetcher handle (`AbstractActionInputPrefetcherBase::self_ref`, set by the new
    `register_prefetcher`, which `remote_module.rs` and the tests use to create a prefetcher).
  - `remote_execution_cache.rs` keeps its own shared upload map on top of the client (PORT
    comment updated: the client's `casUploadCache` sees one subscriber per digest there).
- Tests restored to Java's expectations: `async_task_cache_test::execute_dispose_when_multiple_subscriptions_not_cancelled`
  un-ignored (18 passed, 0 ignored); `remote_action_input_fetcher_test::prefetch_files_multiple_threads_download_is_not_cancelled_by_other_threads`
  un-ignored with Java's `download.isCancelled()` assertion, the adapted duplicate
  `..._remaining_caller_gets_the_file` removed (both files run 10 times: all pass);
  `combined_cache_test` 9 passed, 4 ignored (CDC only); `remote_execution_cache_test` 7 passed.
- Open: none.

## R12. Build event artifact uploader: FIXED (loud, Java's condition)

- `blaze_bep` takes no uploader from outside (`bep_module.rs` `local_file_uploader` always
  returns `LocalFilesArtifactUploader`), so Blaze fails loudly exactly where Bazel would use the
  ByteStream uploader.
- Java's selection: `BuildEventServiceModule.beforeCommand` selects the factory named by
  `--experimental_build_event_upload_strategy` (`BuildEventServiceModule.java:366,393`);
  `BuildEventArtifactUploaderFactoryMap.select` returns the alphabetically first registered
  factory for null and the local one for an unknown name (`BuildEventArtifactUploaderFactoryMap.java:33`);
  the only registered factory is RemoteModule's "remote" (`RemoteModule.java:229`), whose
  `create` gives the ByteStream uploader with a gRPC cache (`RemoteModule.java:905,1350`);
  `createBepTransports` calls the supplier for each file transport whose
  `--build_event_*_file_path_conversion` is on (`BuildEventServiceModule.java:827-900`).
- Files: `blaze_runtime/src/commands/build_remote.rs` `create_bep_file_transport_uploaders`
  (same condition), called from `build.rs` right after the BEP transports are created; it calls
  `RemoteModule::create_build_event_artifact_uploader` (internals fixer), which is `unported!`
  with a gRPC cache. `--remote_build_event_upload` other than `minimal` and
  `--remote_bytestream_uri_prefix` stay loud in setup.
- Oracle (worker `rz-bep` on 9510, `build //:a --remote_cache=grpc://localhost:9510
  --build_event_json_file=...`): oracle (`rz-bep-o`) writes
  `bytestream://localhost:9510/blobs/<sha256>/<size>` URIs (default `minimal` mode: every file
  with a digest, test and build logs uploaded); Blaze (`rz-bep-b`) exits with `blaze: not ported
  yet: ByteStreamBuildEventArtifactUploaderFactory.create: build event artifacts with a gRPC
  remote cache (bytestream:// URIs)` (it wrote local `file://` URIs before). With
  `--experimental_build_event_upload_strategy=local` both write `file://.../bazel-bin/a.txt`
  (`rz-bep-local-o`, `rz-bep-local-b`; the oracle also references its profile, which Blaze
  does not write: not a remote matter).
- Tests: `remote_module_test` `remote_build_event_upload_all_fails_loudly`,
  `build_event_artifact_uploader_with_grpc_cache_fails_loudly`,
  `build_event_artifact_uploader_without_grpc_cache_is_local`.
- Open: the port itself. The hook `blaze_bep` needs (owner of `crates/blaze_bep`):
  `BepCommandSupport`/`bep_module.rs` takes an optional uploader factory,
  `Option<Arc<dyn Fn() -> Arc<dyn BuildEventArtifactUploader> + Send + Sync>>` (Java
  `ThrowingBuildEventArtifactUploaderSupplier`, memoized), used instead of
  `local_file_uploader(..)` at `bep_module.rs:242,260` (and the text transport) when the path
  conversion is on; `BuildEventArtifactUploader::upload` must be allowed to be asynchronous
  (Java returns a `ListenableFuture<PathConverter>`; `mayBeSlow() == true` for the ByteStream
  uploader). Then `blaze_remote` ports `ByteStreamBuildEventArtifactUploader(Factory)`
  (`readPathMetadata`, `queryCombinedCache` through `findMissingDigests`, `uploadLocalFiles`,
  `PathConverterImpl` with `bytestream://<authority or prefix>/[<instance>/]blobs/<hash>/<size>`).

## R13. Flags accepted and ignored: FIXED (loud)

- `remote_module.rs` setup: `--remote_max_connections` / `--remote_max_concurrency_per_connection`
  other than 100 (gRPC path); `--experimental_remote_cache_lease_extension` without
  `--rewind_lost_inputs` (`RemoteModule.java:1268`; with it, `LeaseService::new`); a disk cache
  GC policy (`DiskCacheGarbageCollectorIdleTask.java:58`).
- Tests: `disk_cache_gc_policy_fails_loudly`, `remote_max_connections_fails_loudly`,
  `remote_cache_lease_extension_fails_loudly`, `disk_cache_garbage_collection_idle_task_disabled`.

## R14. Eager channel construction: FIXED

- `ChannelFactory.newChannel` failures make a `ReferenceCountedChannel::failed_to_create`
  whose calls fail with that `IOException` (`GoogleChannelConnectionFactory.java:101`); setup no
  longer exits 37. Test `invalid_tls_certificate_fails_calls_not_setup`; oracle row
  `badcert-cache` in R3.

## R15. `afterCommand` and setup texts/codes: FIXED

- Texts (internals): `after_command` follows `afterCommandTask` (`RemoteModule.java:1139-1171`):
  provider, then the temp dir (deletion failure ignored), then the RPC log ("Partially wrote RPC
  log file", `RPC_LOG_FAILURE`).
- Exit codes (this round): `blaze_exec::exec::spawn_strategy_registry::AbruptExitException` has
  `exit_code: Option<i32>` and `with_exit_code(exit_code, failure_detail)` (Java
  `DetailedExitCode.of(exitCode, failureDetail)`, `DetailedExitCode.java:78`);
  `remote_module.rs` `create_exit_exception` sets it (Java `createExitException`,
  `RemoteModule.java:1381`): `handleInitFailure` → `COMMAND_LINE_ERROR` (2), log-dir cleanup,
  "Partially wrote RPC log file" and "Failed to delete stale downloads" →
  `LOCAL_ENVIRONMENTAL_ERROR` (36); `createOptionsExitException` and the digest-function error
  keep no explicit code. `blaze_runtime/src/commands/build.rs` `abrupt_exit_detailed_exit_code`
  (Java `AbruptExitException.getDetailedExitCode`) is used for the module's `beforeCommand` and
  `afterCommand` exceptions instead of deriving the code from the failure detail.
- Tests: `remote_module_test::stale_downloads_deletion_failure_exits_with_local_environmental_error`
  (warning, message prefix, `exit_code == Some(36)`),
  `options_exit_exception_has_no_explicit_exit_code`.
- Oracle: none of the nine `caps-matrix` scenarios depends on an explicit code (their codes are
  equal to the metadata), and all nine still match (R3). The stale-downloads failure
  (`<execroot>/bazel-out/_tmp/actions` mode 555 with `remote/junk` inside; with and without
  `--disk_cache`): oracle (`rz-stale-o` run4/run5) and Blaze `rz-blaze-1` (`rz-stale-b`) both
  `WARNING: Found stale downloads from previous build, deleting...`, `ERROR: Failed to delete
  stale downloads: <path> (Permission denied)`, exit 36 (Blaze exited 34 before). The message
  differs only in the `[unix_jni.cc:660] ` prefix of Java's native error text.
- Not changed: `after_command` is skipped when an `unported!` panic unwinds; Java's crash path
  (`BlazeRuntime.cleanUpForCrash`) does not run module `afterCommand` either. "Partially wrote
  RPC log file" is not reproducible end to end (needs a failing close).

## R16. Action execution salt: FIXED

- `build_remote.rs::placeholder_contributions` + `build.rs`: `computeActionExecutionSalt` for
  every command with `RemoteOptions` (`RemoteModule.java:526-531,1241,1308,1312-1332`);
  `execution_tool.rs` comment updated. The default path changes only here (local action cache
  entry digests).
- Oracle (brotli, one output base, `rx-r16b`): oracle fresh `53 processes: 10 internal, 43
  linux-sandbox`; Blaze on that output base `1 process: 52 action cache hit, 1 internal`; Blaze
  with `--remote_default_exec_properties=OSFamily=linux` re-runs 53 (as Java); oracle with that
  flag on Blaze's cache `52 action cache hit`. Before: Blaze on an oracle output base re-ran
  everything.

## R17. Remote metadata across invocations: FIXED

- Remote output metadata is stored in the action cache (`set_store_output_metadata` for the
  remote output service) and checked with the `RemoteOutputChecker` in `checkActionCache`; the
  TTL check is inactive in Java 9.3.2 too (no caller of `setCheckMetadataTtl`).
- Oracle `scripts/remote/second-build` (tag rx17, brotli): toplevel and minimal, all three steps
  identical (`53 processes: 43 disk cache hit, 10 internal`; `1 process: 52 action cache hit, 1
  internal`; `2 processes: 51 action cache hit, 1 internal, 1 linux-sandbox`), bazel-bin
  identical. Test `blaze_sky_exec`
  `remote_output_metadata_in_the_action_cache_makes_a_second_build_a_cache_hit`.
- This round: the action cache round trip of the expiration time is still asserted explicitly
  (`compact_persistent_action_cache_test::put_and_get_saves_remote_file_metadata_with_expiration_time`,
  passes) now that equality ignores it (S4 below).

## R18. Tests: FIXED

- `tests/remote_module_test.rs` (new): RemoteModuleTest.java's 20 tests: 12 pass, 8 ignored
  with reasons; plus Blaze-only cases for R3, R12, R13, R14 and (this round) R15: 23 passed, 8
  ignored.
- `tests/rx_utils_test.rs` (new, `util/rx_utils.rs`; `RemoteExecutionCache` collects results in
  completion order as Java): 7 pass, 2 ignored. `tests/remote_output_checker_test.rs`:
  `test_should_download_output` (1 pass).
- False ignore reasons fixed: `grpc_cache_client_test::download_blob_idle_timeout_is_retried_with_progress`
  (passes), `remote_execution_service_test::build_remote_action_with_scrubbing` (passes).
- Dropped assertions: the four `pass_*_with_stack_trace` tests keep their other assertions; the
  stack-trace assertions are companion `*_stack_trace` tests, ignored ("Java stack traces have
  no counterpart"). The R11 adapted assertion is restored (R11).
- Flaky `byte_stream_uploader_test::available_permits_open_file_semaphore_fewer_permits_than_uploads_end_with_all_permits`
  (this round): cause in the fake server, not the client. The h2 crate (0.4.20) closes a server
  connection with GOAWAY(ENHANCE_YOUR_CALM) when the small DATA frames it buffers for streams
  whose handlers have not read them yet exceed its "DATA frame budget" (half the connection
  window, at least 25,600 bytes, each frame under 256 bytes counted as 256; h2
  `proto/connection.rs` `DataFrameBudget`). The test sends 1000 concurrent uploads in 10-byte
  chunks; when the handlers fall behind, the budget runs out and every stream on the connection
  fails with "operation was canceled: connection closed" (the "onError should never be called"
  panics in the log are the server observers of those cancelled calls, already caught). Fix:
  the test server is built with `initial_connection_window_size(1 << 30)` (budget 512 MiB), as
  grpc-java's in-process server buffers without limit; the client and the Java assertions are
  unchanged. Before: 2 of 30 whole-file runs failed; after: 0 of 90 (30 + 60) runs.

## Security ruling

- `// PORT: identical to Bazel, not hardened (decision 2026-11-19)` at the server-log keys
  (`remote_execution_service.rs` `maybe_download_server_logs`), the Tree node names and the two
  output-symlink sites (`remote_execution_service_download.rs`); the prefetcher's re-planted
  symlink targets carry `SECURITY:` comments. All listed in docs/remote.md "Security posture".

## S3/S4

- S4 `FileArtifactValue` equality (this round): Java's remote values compare digest, size and
  location index only (`FileArtifactValue.java:756,903`; the Java test
  `FileArtifactValueTest.testEqualsAndHashCode` puts two values with different expiration times
  in one equality group). `blaze_exec/src/actions/file_artifact_value.rs`: the expiration time
  and the in-memory flag of `RemoteFile` are `NotInEquality` fields (readers in
  `metadata_digest_utils.rs`, `remote_output_checker.rs` adapted). PORT: Java's class check is
  asymmetric (`RemoteFileArtifactValue.equals` accepts a value with materialization data, not the
  reverse); the merged variant compares symmetrically. Test: new
  `blaze_exec/tests/file_artifact_value_test.rs::test_equals_and_hash_code` (equality groups of
  the Java test; `hashCode` has no counterpart). The action cache round trip still keeps the
  expiration time (R17).
- S4 stale-downloads cleanup without remote flags: done for build/test (R6 (b)).
- Left (listed): `NEEDS(channel)` comments in `byte_stream_uploader.rs`;
  `grpc_cache_client.rs:723` sends `inline_output_files` sorted (only visible in the gRPC log);
  invalid `--remote_header` value; public `channel::testing`; `disk_cache_client.rs:497`
  `sync_file`; `SubTreeCache::INITIAL_PURGE_AT`; the three omitted `DigestUtil` methods.
- S3 action rewinding: still unported (loud at the point of use, R6).

## Open after this round

1. R6: action rewinding (the default `--rewind_lost_inputs`) and `RemoteLeaseExtension` stay
   `unported!`; exit 39 after exhausted retries not reproduced end to end.
2. R6 (b): the stale-downloads cleanup for commands other than build/test (Java runs it for
   every command; oracle `query //:a` with a stale `<execroot>/bazel-out/_tmp/actions/remote`
   warns and deletes it, Blaze leaves it). Exact change (owner of the non-retry part of
   `crates/blaze_runtime/src/dispatcher.rs`): in `env_before_command`, for every command except
   `build`/`test` (which do it in `commands/build_remote.rs`), when the options contain
   `RemoteOptions` and no remote option is set, call
   `blaze_remote::remote_module::get_temp_path_generator(&directories.get_action_temps_directory(&exec_root), reporter)`
   and exit with its `AbruptExitException` (through `build.rs`'s
   `abrupt_exit_detailed_exit_code`); in `after_command`, delete
   `<action temps>/remote` ignoring errors (`build_remote::placeholder_after_command` does both
   and can be made `pub(crate)` for this). Commands without a workspace have no exec root: Java
   still computes the path from the output base; check against the oracle first.
3. R9: needs the three-step change above (blaze_actions, blaze_exec, blaze_remote).
4. R12: needs the `blaze_bep` hook above, then the port of the ByteStream uploader.
5. R15: "Partially wrote RPC log file" has unit coverage only.
6. `blaze_vfs` error texts lack Java's `[unix_jni.cc:NNN] ` prefix (seen in R15's message);
   not a remote matter, listed for the vfs owner.
7. S4 leftovers listed above.

## Round 3 (BEP fixer): R12 and the stale-downloads cleanup

### R12. Build event artifact uploader: FIXED (ported)

- `blaze_bep` (additive): `BuildEventArtifactUploader::upload` returns a `PathConverterFuture`
  (`ListenableFuture<PathConverter>`: immediate, cancelled, failed, or completed later through a
  `PathConverterPromise`), plus `retain`/`release`; `runtime::build_event_artifact_uploader_factory`
  ports `BuildEventArtifactUploaderFactory`, `LOCAL_FILES_UPLOADER_FACTORY` and
  `BuildEventArtifactUploaderFactoryMap` (`select(null)` = alphabetically first, unknown = local);
  `BuildEventStreamerBuilder::from_options_with_uploader_factory` ports
  `ThrowingBuildEventArtifactUploaderSupplier` (created once, retained per further transport;
  transports without path conversion keep `LocalFilesArtifactUploader`). `FileTransport` keeps
  its inline write while every future is complete and the uploader is not slow (the default
  path, unchanged); otherwise a `bep-local-writer` thread waits for each event's uploads in
  send order (`SequentialWriter`), failures end the writer with Java's text and code
  (`Task was cancelled.` → `BES_FILE_WRITE_CANCELED`), and the writer releases the uploader.
- `blaze_remote`: `byte_stream_build_event_artifact_uploader.rs` ports
  `ByteStreamBuildEventArtifactUploader` (`readPathMetadata`, `shouldUpload` with the
  `MINIMAL` test/build-log patterns, `queryCombinedCache` = one FindMissingBlobs, then the uploads
  with `REMOTE_CACHE_ONLY`, `reportUploadError` warnings "Uploading BEP referenced local file
  <path> <digest>: <grpc message>", the `bytestream://` `PathConverterImpl` incl. the digest
  function component and `--remote_bytestream_uri_prefix`, ref counting) and its factory.
  `RemoteModule.setup` installs the factory for every gRPC cache (the loud checks of
  `--remote_build_event_upload`/`--remote_bytestream_uri_prefix` are gone); `afterCommand`
  resets it. Remaining loud: `isRemoteFile` for a path on a `RemoteActionFileSystem`
  (`blaze_vfs::FileSystem` has no downcast; not reached by Blaze's build events).
- `blaze_runtime`: `build_remote::build_event_artifact_uploader_factory_map` registers the
  "remote" delegate (`RemoteModule.serverInit`); `BepCommandSupport::before_command` selects with
  `--experimental_build_event_upload_strategy`. `create_bep_file_transport_uploaders` is removed.
- Tests: `tests/byte_stream_build_event_artifact_uploader_test.rs` 12 passed, 2 ignored
  (`custom_hash_function_uploaded`: no BLAKE3 file system, Java's `assumeNotNull` skips; the URI
  format is unit-tested; `remote_file_should_not_be_uploaded_action_fs`: no FileSystem
  downcast), 10 of 10 runs green; 4 unit tests in the module; `remote_module_test` R12 tests now
  assert the real uploader (23 passed, 8 ignored); `binary_format_file_transport_test` 7 passed,
  0 ignored (`test_cancelled_upload` un-ignored, release verified).
- Oracle (fresh `scripts/remote/worker` per side, brotli `build //...`
  `--remote_cache=grpc://localhost:PORT --build_event_json_file --remote_grpc_log`; warm = second
  run after `clean`; compared with the new `scripts/remote/bep-uris` and
  `scripts/remote/compare-grpc-log`):

  | run | BEP `uri` equal | gRPC log matched |
  |---|---|---|
  | minimal cold (`rb-bep-mo`/`rb-bep-mb` run1) | 115/116 | 295/296 |
  | minimal warm (run2) | 115/116 | 51/52 |
  | all cold (`rb-bep-ao`/`rb-bep-ab` run1) | 115/116 | 424/429 (427 candidate) |
  | all warm (run2) | 115/116 | 75/76 |

  The one missing URI and the unmatched profile FindMissingBlobs/Write are Bazel's
  `command.profile.gz` (Blaze writes no profile; not a remote matter). In `all` cold, one blob
  (`d62b16…/138772`) is written by `bes-upload` in Bazel and by its producing action in Blaze
  (the BEP upload races the action's upload; the FindMissingBlobs naming it are in both).
- Default path (no remote flags), `rb-bep-def` run1..4 alternating the binary before and after
  this change in one workspace: the URI lists are identical (115) and the event multisets are
  identical after masking ids, times and named-set numbering, except the wall-clock
  `BUILD_TIMESTAMP`/`FORMATTED_DATE`; the event order differs between two runs of the same
  binary as much as between the two binaries.

### R6 (b) stale downloads for every command: FIXED

- `blaze_runtime` `dispatcher.rs` `env_before_command` / `after_command` call
  `build_remote::dispatcher_before_command` / `dispatcher_after_command`: for every command other
  than build/test (which keep their own path) that has `RemoteOptions` (a common command option,
  `RemoteModule.getCommonCommandOptions`) and no remote option set, `getTempPathGenerator`
  (warning, deletion, `Failed to delete stale downloads` exit) and the deletion in
  `afterCommandTask`. PORT: a non-build command with a remote option set does not set the remote
  module up in Blaze, so it does not clean up either.
- Oracle (`scripts/remote/stale-probe`, stale `<output_base>/execroot/_main/bazel-out/_tmp/actions/remote/x/stale`
  before each command, no remote flags; `rb-stale-o`/`rb-stale-b`, `rb-stale-o2`/`rb-stale-b2`):
  `query //...`, `build //...`, `info`, `clean`, `info output_base`, `info release`: both print
  `WARNING: Found stale downloads from previous build, deleting...` right after the server start
  line, and the directory is gone afterwards in both. Exit codes equal except Blaze's bare `info`
  (101: `CommittedHeapSizeInfoItem.get: no JVM`, unported before this change).

### Open after round 3

- `isRemoteFile` on a `RemoteActionFileSystem` path (needs `FileSystem::as_any` in `blaze_vfs`).
- Bazel's `command.profile.gz` (no profiler in Blaze) is the only BEP file Blaze does not upload.

# Round 2 response

- Reviewer's verdict: STOP at `d294400` (docs/reviews/2026-11-19-remote.md, "# Round 2"): N1 to N4
  (S1), R9 open, N5 as the condition on R6.
- Lead of this round: a new remote lead (the previous one finished), with six Opus workers:
  A (overlay), B (trees, exec root, executor), C (internals), D (platform), E (lost inputs),
  F (triage of Bazel's own shell tests, read-only).
- Method, as before: literal ports with `// Java:` markers; expected values from the Java tests
  or from the oracle (Bazel 9.3.2); every behaviour finding shown through the real binary
  against Bazel's own worker with a tool under `scripts/remote/` that the reviewer can rerun
  (`<tool> help`). New tools: `probe-trees2`, `shard-probe`, `attempts-probe`, `platform-probe`,
  `pathcheck-probe`, `recreate-probe`, `outpath-symlink-probe`, `bep-interrupt-probe`;
  extended: `evict-probe` (`ws-seed`, `ws`, `disk`), `caps-matrix`, `stale-probe`.
- Rulings taken this round (docs/decisions.md): lost inputs without rewinding (N5); the disk
  cache's action-result integrity check stays on while rewinding is unported; X.509 v1
  certificates stay rejected (no hand-written verifier).
- Tree, binaries and test counts: see "Numbers" at the end of this section.

| Finding | State |
|---|---|
| N1 tree values through the action file system (S1) | fixed |
| N2 exec root through the action file system (S1) | fixed |
| N3 capabilities failure with NOT_FOUND (S1) | fixed |
| N4 platform `remote_execution_properties` (S1) | fixed: the TextFormat reader is ported |
| R9 `argv[0]` policy | fixed: keyed on the OS constraint; loud for a Windows execution platform |
| N5 / R6 lost inputs | fixed for every source without rewinding (ruling); rewinding stays loud; purge before failing |
| N6 `-DBAZEL_TRACK_SOURCE_DIRECTORIES` | fixed (by the workerd lead's worker) |
| N7 BEP upload failure text | fixed |
| N8 interrupt and BEP uploads | fixed (the transport's `close` blocked) |
| N9 overlay deviations | fixed: literal port with an in-memory remote tree |
| N10 tests | fixed: 124 of 124 `RemoteActionFileSystemTest(Base)` methods; traversal test assertions |
| N11 non-build commands | fixed for query, cquery, aquery, info; run and coverage stay loud (commands not ported) |
| N12 (S4) `hashString(.., ISO_8859_1)` | fixed in `file_artifact_value.rs` (the traversal site is not in my files) |
| R1, R2, R5 residuals | fixed (N1, N2, interrupt during finalize) |
| R3 transport texts | fixed as suggested; residue listed |
| R4 residuals | fixed |
| R13 partials | fixed (warning; disk cache with non-default max connections); the connection pool is not ported |
| S3/S4 lists | see "S3 and S4" |

## N1 (S1). Tree values through the action file system: FIXED

- `blaze_sky_exec/src/skyframe_action_executor.rs`: `construct_tree_artifact_value_from_filesystem`
  is `ActionOutputMetadataStore.java:236-320` line by line: `tree_dir = to_path(parent)` (new
  `to_path` / `artifact_path_resolver_to_path`, `ArtifactPathResolver.java:75,107`), `statIfFound(FOLLOW)`
  and `visitTree` on the action file system, chmod as Java, `setResolvedPath` (:309-317). The
  `get_files_under` merge is deleted (and the flat `RemoteOutputTree` copy behind it is gone from
  the overlay). `file_artifact_value_from_artifact` (:478-545) and `file_artifact_value_from_stat`
  (:566) are literal again (`pathNoFollow`/`realPath` on the resolver's file system,
  `createFromExistingWithResolvedPath`). `TreeArtifactValue::visit_tree` needed no change (it
  only uses `Path` methods).
- PORT: `stat instanceof FileStatusWithMetadata` is `RemoteActionFileSystem::stat_metadata` on
  the same path (`blaze_vfs::FileStatus` carries no metadata).
- `FileArtifactValue::create_from_existing_with_resolved_path` now has Java's three
  `checkArgument`s and unwraps a `ResolvedSymlink` delegate (`FileArtifactValue.java:444,951-962`).
- Tests (ports of `ActionOutputMetadataStoreTest`, module `action_output_metadata_store_tests`):
  `file_artifact_materialized_as_symlink_to_non_artifact`,
  `file_artifact_materialized_as_symlink_to_file_artifact` (4 parameter combinations),
  `tree_artifact_materialized_as_symlink_to_non_artifact`,
  `tree_artifact_materialized_as_symlink_to_another_tree_artifact` (both depths); Blaze-only
  `tree_with_relative_symlink_to_remote_child` (children from the oracle run below). In the
  overlay's test file: `tree_with_local_relative_symlink_to_remote_only_file`,
  `symlink_to_remote_only_input_tree_lists_its_children`.
- Oracle (`scripts/remote/probe-trees2`, disk cache seeded by the oracle, fresh output base,
  default flags):

| step | oracle | Blaze after | Blaze before (`ru-blaze-0`) |
|---|---|---|---|
| (i) `build //:libcons //:liblocal`: cached tree with `lib/libfoo.so -> libfoo.so.1`, not top-level | `4 processes: 2 disk cache hit, 1 internal, 1 linux-sandbox`, exit 0 | same | `child lib/libfoo.so is a dangling symbolic link`, exit 1 |
| (ii) `build //:symcons //:symlocal`: `ctx.actions.symlink` of a declared directory to a remote-only tree | `5 processes: 2 disk cache hit, 2 internal, 1 linux-sandbox`, exit 0 | same | consumers see zero children, exit 1 |
| `build //:libtree //:sym` | `1 process: 3 action cache hit, 1 internal`, exit 0 | same | |

  `bazel-bin` (sha256 of files and symlink targets) identical in all three steps; the consumers
  list `lib/libfoo.so`, `lib/libfoo.so.1` and `sym/a.txt`, `sym/sub/b.txt` on both sides.
- Found while fixing (not in the review): Java creates each action's output directories through
  the action file system, in the remote output tree as well (`SkyframeActionExecutor.java:1128-1158,
  1453-1509`, `ActionOutputDirectoryHelper.java:79-105`). Ported: `create_action_fs_output_directories`
  (blaze_exec `action_output_directory_helper.rs`, with Java's fallback), `setup_action_fs_file_out_err`,
  the messages and codes `ACTION_FS_OUTPUT_DIRECTORY_CREATION_FAILURE` /
  `ACTION_FS_OUT_ERR_DIRECTORY_CREATION_FAILURE`, `prepare` on the context's exec root. Without
  an action file system the old path is unchanged. And Java's `SymlinkTreeHelper` moves its paths
  to the host file system (`SymlinkTreeHelper.java:70-72`, `Path.forHostFileSystem`,
  `RemoteActionFileSystem.getHostFileSystem` :257): `blaze_vfs` got `FileSystem::get_host_file_system`
  and a real `Path::for_host_file_system`.

## N2 (S1). The action's exec root through the action file system: FIXED

- `ActionExecutionContextImpl` owns `exec_root`, computed as `ActionExecutionContext.java:369-373`
  (`actionFileSystem.getPath(execRoot.asFragment())`, else the executor's root). Callers checked
  against Java: `abstract_spawn_strategy.rs` (same result), `symlink_action.rs`, blaze_rules
  `cpp_compile_action.rs` (`createSpawn(ctx.getExecRoot())`, the `.d` file read) now on the
  action file system as Java; spawn runners keep the executor's local exec root.
- `standalone_test_strategy.rs`: premature-exit file and shard status file through
  `convertPath` (`StandaloneTestStrategy.java:687,706`); `runfilesDir`/`tmpDir` converted
  (:532-537, :142-145); the path resolver passed to `processTestAttempt` (:250); the
  `test.xml`/`test.log` check literal (:857). `test_runner_action.rs`:
  `get_test_outputs_mapping(resolver, exec_root)` (`TestRunnerAction.java:429`); the false
  "always the identity" comment is gone.
- Oracle (`scripts/remote/shard-probe`; Starlark test rules with `shard_count = 2` that touch
  `$TEST_SHARD_STATUS_FILE`, one also writing its own `$XML_OUTPUT_FILE` and a file in
  `$TEST_UNDECLARED_OUTPUTS_DIR`; oracle-seeded disk cache; `--remote_download_outputs=minimal`
  and, separately, `--remote_download_minimal`): oracle and Blaze both
  `15 processes: 6 disk cache hit, 11 internal`, `//:t (cached) PASSED`, `//:u (cached) PASSED`,
  `Executed 0 out of 2 tests: 2 tests pass.`, exit 0; same `bazel-testlogs` listing per shard
  (`test.cache_status`, `test.log`); same BEP `TestResult` files per shard (`test.log`,
  `test.xml`, for `u` also `test.outputs/u<N>.txt` and the manifest). Before: "Sharding
  requested, but the test runner did not advertise support for it...", `NO STATUS`, exit 3.
- Bazel's own tests, through `scripts/upstream-tests/run`:
  `test_shard_status_file_checked_remote_download_minimal`,
  `test_premature_exit_file_checked_remote_download_minimal` (`remote_execution_test.sh`) and
  `test_multiple_test_attempts` (`build_without_the_bytes_test.sh`; before: panic "Files are on
  different filesystems" in `rename_outputs`) pass. `scripts/remote/attempts-probe`
  (`--remote_executor --remote_download_minimal --flaky_test_attempts=2` on a failing test):
  both `//:f FAILED in 2 out of 2`, `7 processes: 1 remote cache hit, 6 internal, 3 remote`,
  exit 3, same testlogs and BEP. Left: the oracle prints a `FAILED: //:f (Summary)` block
  (`UiEventHandler.testSummary`), which Blaze's UI lacks (not remote).

## N3 (S1). Capabilities failure with NOT_FOUND: FIXED

- `common/errors.rs` `is_call_not_started`; `grpc_cache_client.rs` `handle_status` and
  `request_read` let the `RemoteExecutionCapabilitiesException` of a call that never started
  through instead of reading its cause's code (`GrpcCacheClient.java:377-386` catches only
  `StatusRuntimeException`); `upload_blob_impl` reads `e.getCause()` as Java;
  `byte_stream_uploader.rs` `Writer.on_error` skips the ALREADY_EXISTS mapping for such a call.
  Checked and left: `remote_retrier.rs`, `execute_retrier.rs`, `grpc_remote_executor.rs`
  (`Status.fromThrowable` walks the cause chain in Java too).
- Tests (`remote_module_test.rs`): `verify_capabilities_not_found_fails_download_action_result`,
  `verify_capabilities_not_found_fails_download_blob`,
  `verify_capabilities_already_exists_fails_upload_blob`,
  `verify_capabilities_invalid_argument_upload_names_the_blob`.
- Oracle: Bazel's worker cannot answer GetCapabilities with NOT_FOUND, so
  `blaze-remote-fake-caps` (new bin: GetCapabilities answers a given status, everything else
  UNIMPLEMENTED) serves both sides in new `caps-matrix` rows: `--remote_cache` and
  `--remote_executor`: oracle and Blaze `Failed to query remote execution capabilities:
  NOT_FOUND: instance not found`, exit 34; with the local-fallback flags both exit 0. Before:
  Blaze exit 0 (every lookup a miss).

## N4 (S1). Platform `remote_execution_properties`: FIXED (ported)

- The string is carried: `blaze_actions::PlatformInfo::remote_execution_properties()`
  (`PlatformInfo.java:130`), `convert_platform` passes it. Blaze's `PlatformInfo.Builder` was
  compared with `PlatformInfo.java:372-452` (parent merge, `{PARENT_REMOTE_EXECUTION_PROPERTIES}`):
  equal; the oracle agrees (a child with `parents` and no attribute sends no platform).
- The reader: `text_format.rs`, a literal port of protobuf-java 33.4 `TextFormat.java` (the
  version Bazel 9.3.2 pins): `Tokenizer` (UTF-16 line/column as Java), `Parser.merge` →
  `mergeField`, `consumeFieldValues`, unknown-field skipping (`guessFieldTypeAndSkip`, `skipField*`),
  `checkUnknownFields`, `unescapeBytes`/`unescapeText`, `parseInteger`; driven by descriptors for
  `Platform`/`Property`. `PlatformUtils.getPlatformProto` (`PlatformUtils.java:102-150`) is
  complete; a `ParseException` is the `UserExecException` with `INVALID_REMOTE_EXECUTION_PROPERTIES`.
  Two facts the review's fix text did not expect, both from Java and confirmed by the oracle: an
  unknown field is skipped and reported at the end ("Input contains unknown fields and/or
  extensions: ..."), and a repeated singular field overwrites (ALLOW_SINGULAR_OVERWRITES). The
  only `unported!` is `mergeAnyFieldValue` (no `Any` is parsed).
- The reader lives in `blaze_protos::text_format` (re-exported by `blaze_remote::text_format`),
  because the execution log needs it too: `blaze_exec` `spawn_log_context.rs` had a private
  `unported!` for the same parse, which became reachable once the string was carried. It now
  parses (`PlatformUtils.java:128-148`). `platform-probe --execlog` (default path, no remote
  flag): 7 cases identical to the oracle, including the two error cases (`Probe t.out failed:
  Failed to parse remote_execution_properties from platform //:p: 1:19: Expected ":".`, exit 1).
- Tests: `platform_utils_test.rs`: all 6 `PlatformUtilsTest` methods, 0 ignored
  (`test_parse_platform_legacy_options` un-ignored) + 7 oracle-derived; `text_format_test.rs`: 4
  ports of `TextFormatTest` + 16 oracle cases (the ~90 other `TextFormatTest` methods test the
  printer, `TestAllTypes`, maps and `Any`, none of which is ported; said in the file header).
- Oracle (`scripts/remote/platform-probe`: fresh worker per case, the Action and Command read
  back from the worker's CAS): 30 cases; 28 identical in platform properties, exit code, error
  text and processes line: parsing (one, unsorted, colon form, escapes, comment, `<>`, list,
  duplicate name), errors (unknown field, nested unknown, `1:19: Expected ":".`, broken string,
  EOF, identifier, unterminated, invalid value, bad unicode), inheritance (parent, substitution),
  every branch of `getPlatformProto` (`exec_properties` on the platform and on the target,
  `--remote_default_exec_properties` alone and with each of the others, empty text), OS linux
  and macos, and the sequence of upstream's
  `test_platform_default_properties_invalidation_with_platform_remote_execution_properties`
  (both send `foo=baz`; second build `2 processes: 1 remote cache hit, 1 internal`). The two
  that differ: `os-windows` (R9, intended) and `escapes` (the platforms are byte-identical; only
  the relayed server error prints non-Latin-1 characters differently: the strings seam).

## R9. `argv[0]` policy on the execution platform's OS: FIXED (loud for Windows)

- `blaze_exec::actions::PlatformInfo.os_constraint` from
  `blaze_actions::PlatformInfo::constraint_value(@platforms//os:os)`;
  `get_os_from_constraints_or_host` and the constraint table port `ConstraintConstants.java:39-94`;
  the argv[0] block (`RemoteExecutionService.java:312-316`, `OsPathPolicy.java:66`) applies the
  Unix policy for every OS but Windows and calls
  `unported!("WindowsOsPathPolicy.postProcessPathStringForExecution (Windows execution platform <label>)")`
  there.
- Tests: `build_remote_action_executable_path_conforms_to_platform` is one test per
  `@TestParameter OS` value (darwin, freebsd, openbsd, linux, unknown unchanged; windows
  `#[should_panic]` on the unported text).
- Oracle (`platform-probe os-windows`): Bazel sends argv[0] `tools\run.sh`; Blaze exits with
  `blaze: not ported yet: WindowsOsPathPolicy.postProcessPathStringForExecution (Windows
  execution platform //:p)` and sends no Action.

## N5 (S2) and R6. Lost inputs: FIXED for every source without rewinding; rewinding stays loud

Root ruling (docs/decisions.md): while action rewinding is unported, every lost-input source
ends in Bazel's `--norewind_lost_inputs` handling; the default path stays a loud `unported!`
that names the flag; stale entries are purged first.

- Sources. `LostArtifacts` has one definition (blaze_exec, as `ImportantOutputHandler.LostArtifacts`);
  `throw_if_not_empty` returns `LostInputsExecException` (`ImportantOutputHandler.java:155`); no
  `unported!` is left at the four sites: `remote_spawn_runner.rs` upload (through the retrier,
  not retried; `RemoteSpawnRunner.java:278-284`) and `exec_locally_and_upload_or_fail` (:580),
  `merkle_tree_computer.rs` (`MerkleTreeComputer.java:390-393`), `finalize_action`
  (`AbstractActionInputPrefetcher.java:886-896`; `ActionFinalizer` now returns
  `Result<(), SpawnCacheError>`).
- In-process reads. The overlay records lost inputs in `get_input_stream`
  (`RemoteActionFileSystem.java:380-393`) and merges them in `check_for_lost_inputs` (:914-923).
  The executor asks after a successful `Action.execute` (`SkyframeActionExecutor.java:1265`), in
  `maybe_signal_lost_inputs` (:1220-1250) at both catches, and in the finalize catch (:1344-1350);
  the `LostInputsCheck` of the context (:1614) is wired. Template expansion reads through the
  action file system's `getInputStream` (`Template.java:103`). PORT, marked: the discovery
  context has no action file system (:967, :976); `enrichLostInputsException` only feeds
  rewinding.
- Purge. With `--norewind_lost_inputs`: `LeaseService.finalizeExecution` → `handleMissingInputs`
  runs before the action cache is saved, also for a failed build (verified end to end). On the
  default path: `LostInputsPolicy.purge_before_unported_rewinding` applies the same predicate
  (`LeaseService.java:56-106`) to the persistent action cache and saves it, right before the
  `unported!` (PORT: Java rewinds here).
- Ruling on the disk cache: `checkDiskCacheActionResultIntegrity` is always true (Java:
  `!rewindLostInputs`, `RemoteModule.java:700-706`); listed as a deviation in docs/remote.md.
  `evict-probe disk` (one disk CAS blob and the consumer's AC entry deleted), default flags:
  oracle `3 processes: 1 disk cache hit, 1 internal, 1 linux-sandbox` (rewinds); Blaze
  `3 processes: 1 internal, 2 linux-sandbox`, exit 0 (before the ruling: the loud `unported!`
  on every run); with `--norewind_lost_inputs` both `3 processes: 1 internal, 2 linux-sandbox`.
- Tests: `remote_spawn_runner_test::lost_input_during_upload_fails_without_retries_or_execution`
  (`RemoteSpawnRunnerTest.java:1004`) un-ignored; blaze_sky_exec
  `lost_inputs_without_rewinding_fail_the_action_as_bazel_does`,
  `lost_inputs_with_rewinding_purge_the_action_cache_before_failing_loudly`; overlay
  `get_input_stream_records_lost_inputs`. Neither `ActionRewindStrategyTest` nor
  `LeaseServiceTest` exists at the pin.
- Oracle (`scripts/remote/evict-probe ws`, worker with `--noaction_cache_integrity_check`, one
  CAS blob deleted per scenario):

| source | `--norewind_lost_inputs` | `... --experimental_remote_cache_eviction_retries=0` | default flags |
|---|---|---|---|
| local spawn prefetch | identical: `Executing genrule //:b1 failed: Lost inputs no longer available remotely: a1.txt (0111f755…/3)`, `Found transient remote cache error, retrying the build...`, `3 processes: 1 internal, 2 linux-sandbox`, exit 0 | identical: `Unexpected lost inputs (pass --rewind_lost_inputs to enable recovery): a1.txt`, exit 1 | oracle exit 0 (rewinds); Blaze `unported!` naming `--norewind_lost_inputs` |
| remote execution (upload, Merkle tree) | identical lines, then `3 processes: 1 internal, 2 remote`, exit 0 | identical, exit 1 | as above |
| in-process read (template expansion) | identical: `Expanding template expanded.txt failed: Lost inputs no longer available remotely: tmpl.txt (bcfd0ecb…/13)`, retry, exit 0 | identical, exit 1 | as above |
| finalize (top-level symlink to an evicted blob) | identical: `Creating symlink l.txt failed: Lost inputs no longer available remotely: l.txt (16691bb6…/3)`, retry, exit 0 | identical, exit 1 | as above |

  Exit code 39 after exhausted retries (never reproduced before): two evicted blobs needed by
  two actions in sequence, `--experimental_remote_cache_eviction_retries=1`: oracle and Blaze
  print the same two "Lost inputs" errors and one retry line and exit 39. Not sticky: after the
  loud default-path failure, a default-flags rerun against a validating worker prints
  `3 processes: 1 internal, 2 linux-sandbox`, exit 0 (the binary before this round crashed
  again).
- Still not ported (S3, listed in docs/remote.md): action rewinding itself; `RemoteLeaseExtension`;
  `RemoteImportantOutputHandler` (`LostType.OUTPUT`, "Lost outputs no longer available
  remotely": Java registers it for a target that becomes top-level after it was executed, which
  needs the server's memory between commands; upstream's
  `test_remote_cache_eviction_retries_toplevel_artifacts` fails for that reason).

## N6. `-DBAZEL_TRACK_SOURCE_DIRECTORIES`: FIXED (by the workerd lead's worker)

`blaze_client/src/startup_options.rs` `unported_host_jvm_arg_error` refuses the property
(exit 2, "blaze: not ported yet: --host_jvm_args=-DBAZEL_TRACK_SOURCE_DIRECTORIES: ..."), test in
`startup_options_test.rs`. Verified by reading; not my change.

## N7. BEP upload failure text: FIXED

`errors::JavaException` / `errors::java_to_string` give the Java class of each error the remote
crate creates (`<class>: <cause.toString()>` for the capabilities exception;
`io.netty.channel.AbstractChannel$AnnotatedConnectException` for a refused connection); blaze_bep
`JavaThrowable` carries it to `UploadFailure::get_message` (`FileTransport.java:156-166`). Oracle
and Blaze, unreachable cache with `--build_event_json_file`: identical line, exit 38: `ERROR:
Unable to write all BEP events to file due to 'com.google.devtools.build.lib.remote.common.RemoteExecutionCapabilitiesException:
io.netty.channel.AbstractChannel$AnnotatedConnectException: finishConnect(..) failed: Connection
refused: localhost/127.0.0.1:9499'`.

## N8. Interrupt and BEP uploads: FIXED (the defect was in the transport, not in the order)

- The order in `blaze_runtime` `build.rs` is Java's (module order `Bazel.java:60,89,92`:
  RemoteModule, BlockWaitingModule, BuildEventServiceModule; now cited there, and the two exit
  codes are combined as `BlazeRuntime.java:780,795`). The defect: Blaze's `FileTransport::close`
  joined the writer thread, which waits for every upload, and the streamer closes its transports
  when the last event is posted, before `RemoteModule.afterCommand` can run `shutdownNow`. Java's
  `FileTransport.close()` returns the close future without waiting (`FileTransport.java:225-254,274`);
  only `BuildEventServiceModule.waitForBuildEventTransportsToClose` waits, after the remote module.
- Fix (blaze_bep): `BuildEventTransport::close` returns a `CloseFuture`; the streamer keeps the
  futures and `get_close_futures_map()` does the wait. `reportUploadError` returns early for a
  cancelled upload (`ByteStreamBuildEventArtifactUploader.java:326-328`).
- Oracle (`scripts/remote/bep-interrupt-probe`: three 400 MB outputs,
  `--remote_build_event_upload=all`, SIGINT during a slow action; 3 runs each):

| | exit | BEP artifact URIs | ByteStream/Write | SIGINT to exit |
|---|---|---|---|---|
| oracle | 8 | all `file://` | 1 to 4 calls, all CANCELLED | 0.4 to 0.6 s |
| Blaze before | 8 | 3 `bytestream://` | 3 OK | 1.5 s (after the uploads) |
| Blaze after | 8 | all `file://` | 0 to 3 calls, all CANCELLED | 0.0 to 0.1 s |

  ERROR and summary lines identical. Left: Bazel also lists `command.profile.gz` (no profiler in
  Blaze).

## N9. Overlay deviations: FIXED (the overlay is a literal port)

- `RemoteActionFileSystem.java` compared method by method, with `PathCanonicalizer.java` and the
  parts of `InMemoryFileSystem`/`InMemoryContentInfo`/`InMemoryDirectoryInfo`/`InMemoryLinkInfo`
  the remote tree uses. New `RemoteInMemoryFileSystem` (Java :925-1006, `InMemoryFileSystem.java:62-787`):
  an inode tree with directories, symlinks, file metadata, permissions and a clock;
  `pathWalkErrno` (ELOOP after 256), `renameTo` moving subtrees, `delete` with ENOTEMPTY,
  `readSymbolicLink` "is not a symlink". New `PathCanonicalizer` behind `resolve_symbolic_links`
  (`clear_prefix` on delete and rename, Java :363/369/782-783).
- The reviewer's four: `create_symbolic_link` and `create_directory(_and_parents)` write both
  trees; `read_symbolic_link` checks the remote tree first (:585-593); `rename_to` and `delete`
  work on subtrees and keep Java's exception pairing; `stat_from_metadata` no longer invents
  0555 (PORT: Java returns -1 and the caller asks `isExecutable()`; `blaze_vfs::FileStatus`
  has no -1, so the bits are what this file system's `isReadable/isWritable/isExecutable`
  answer). `get_host_file_system` is `localFs.getHostFileSystem()` (:257).
- Remaining PORT deviations, each marked in the code: `getDigest` errors where the metadata
  digest is null; prefetch priority MEDIUM instead of CRITICAL (the blocking prefetch seam
  carries no priority); suppressed exceptions are appended to the message; `getxattr`,
  `createReadWriteByteChannel`, `getFileSystemType` are not in Blaze's `FileSystem` trait.

## N10. Tests: FIXED

- `RemoteActionFileSystemTest(Base)`: 124 Java methods, 124 Rust tests of the same names,
  0 ignored, no catch-all stub (`@TestParameter` methods loop over the Java values); a table is
  at the top of `blaze_exec/tests/remote_action_file_system_test.rs`. The local file system sits
  behind a spy so that `getDigest_fromInputArtifactData_*` verify, as Java does, that
  `getDigest` is not called. `statAndExists_isNotDirectory` needed a `blaze_vfs` fix
  (`is_not_found` for ENOTDIR, `unix_jni.cc:659`).
- `ActionOutputDirectoryHelperTest`: all 8 Java methods have a counterpart in the new
  `blaze_exec/tests/action_output_directory_helper_test.rs`: 6 pass, 2 ignored (they need an
  exception-injecting `FileSystem`).
- `recursive_filesystem_traversal_function_test.rs`: `unbounded_symlink_expansion_error` and
  `symlink_chain_error` are Java's setups (:917, :932) and assert Java's texts ("Infinite
  symlink expansion", "Symlink cycle"); the digest of
  `source_directory_artifact_value_is_hash_of_contents` (no Java test asserts a value) is
  computed from Java's byte layout without Blaze's `Fingerprint` (`ArtifactFunction.java:364-369`,
  `Fingerprint.java:159,235`); `testTraversalOfSymlinkToFile` and
  `testTraversalOfTransitiveSymlinkToFile` (:509, :538) are ported and pass (the review's S3
  note on `SymlinkToFile` roots).

## N11. Non-build commands and `RemoteModule.setup`: FIXED for commands that do not execute actions

`build_remote.rs`: `before_command` is split into `set_up_module` (`RemoteModule.beforeCommand`,
any command) and the build-only part (`executorInit`); `dispatcher_before_command` runs the real
setup for `query`, `cquery`, `aquery`, `info` when a remote option is set, and
`dispatcher_after_command` returns the module's `AbruptExitException`. Oracle
(`scripts/remote/stale-probe remote-variants`: 4 commands × `--disk_cache` / unreachable
`--remote_cache` / both): all 12 rows equal: exit 0, `WARNING: Found stale downloads from
previous build, deleting...` before `INFO: Invocation ID`, the directory gone. Commands that
execute actions and are not ported themselves (`run`, `coverage`) keep the loud refusal
(`unported_options.rs`).

## Round-1 partials

- R1: closed by N1. R5: closed by N2 (and the template read, N5). R6: see N5.
- R2 residual: an interrupt during `finalizeAction` leaves as `ExecutionFailure::Interrupted`,
  unreported (`SkyframeActionExecutor.java:1340`).
- R3 transport texts: FIXED as the reviewer suggested, with a listed residue.
  `grpc/channel.rs` `transport_status` ports grpc-netty `Utils.statusFromThrowable`
  (`io/grpc/netty/Utils.java:327`) and `DnsNameResolver.java:285`, applied to every call error:
  refused connection and other I/O → `UNAVAILABLE: io exception` (cause rebuilt in Netty's form,
  `finishConnect(..) failed: Connection refused: <host>/<ip>:<port>`), h2 protocol error →
  `INTERNAL: http2 exception`, TLS alert → `UNAVAILABLE: ssl exception`, unresolvable host →
  `UNAVAILABLE: Unable to resolve host <host>`. `caps-matrix` (20 rows, 11 new): exit codes
  equal in all 20; lines identical in 17, including the HTTP/1 row and the per-action warning
  (`WARNING: Remote Cache: UNAVAILABLE: io exception`; before: `tcp connect error`). The three
  rows that differ: two where Bazel itself varies between `localhost/127.0.0.1` and
  `localhost/[0:0:0:0:0:0:0:1]` from run to run, one where a certificate is rejected locally
  (JDK/OpenSSL text vs rustls text). PORT: hyper and h2 are not dependencies of the crate, so
  their errors are recognised by their fixed texts.
- R4 residuals: `wait_for_bulk_transfer` restores an interrupt that was set on entry
  (`Utils.java:560,574-577`; test `wait_for_bulk_transfer_restores_the_interrupt_of_the_caller`);
  BEP uploads are cancelled (N8).
- R13: (a) `RemoteModule::executor_init` ports `RemoteModule.java:1241-1274`: the
  warning "--experimental_remote_cache_lease_extension has no effect since --rewind_lost_inputs
  is enabled, ..." (oracle and Blaze print the same line), `unported!("RemoteLeaseExtension")`
  only without rewinding. (b) The max-connections check moved into `create_channel`:
  `--disk_cache=... --remote_max_connections=200` builds on both (`2 processes: 1 internal,
  1 linux-sandbox`). Not done: `DynamicConnectionPool` (non-default values stay loud on the gRPC
  path).
- R7 doc: the stale "Blaze is weaker here" sentence is removed from docs/remote.md.

## Found by Bazel's own shell tests, not by the review

The remote shell and Python tests of the pinned source were run unmodified against Blaze
(`scripts/upstream-tests/run`; classification of every failure in docs/remote.md, "Bazel's own
remote tests"). Remote defects they showed and this round fixed:

- `RemotePathChecker` was a stub (`RemoteExecutionCache.java:99-127`): ported, and the Merkle
  tree now resolves paths with the spawn's path resolver (`MerkleTreeComputer.java:386`,
  `MerkleTree.java:171-195`, `TransformResolver`). `scripts/remote/pathcheck-probe`: oracle and
  Blaze `3 processes: 1 disk cache hit, 1 internal, 1 remote`; before: Exit 34. Upstream
  `test_combined_disk_remote_exec_with_flag_combinations` passes.
- `OutputService.startBuild` (`RemoteOutputService.java:125-150`, `LocalOutputService.java:59-85`,
  called at `ExecutionTool.java:291`): ported for both services. `scripts/remote/outpath-symlink-probe`
  (remote, local, local-moved): identical to the oracle; before: "Couldn't create action output
  directory: ... (File exists)", exit 36. Upstream `test_output_path_is_symlink` passes.
- Test attempts under `--remote_download_minimal` (N2 above).
- A regular file where a tree directory must be re-created (`test_prefetcher_recreate_tree_dir`):
  the cause was `blaze_vfs` (`statIfFound` must return null for ENOTDIR); fixed there.
  `scripts/remote/recreate-probe`: all four steps as the oracle; the upstream function passes.
- `actionInputFetcher.shutdown()` in `RemoteActionContextProvider.afterCommand`
  (`RemoteActionContextProvider.java:244-247`) had no caller; `AbstractActionInputPrefetcher.shutdown`
  ports :837-847 with the interrupt loop. Both ported; test `after_command_shuts_the_action_input_fetcher_down`.
- mTLS handshake failure downgraded to a warning: with a client certificate that rustls
  rejects when the TLS config is built, Blaze reported "Failed to init TLS infrastructure" per
  action and built locally with exit 0. Now the channel is created and every connection fails
  (a transport error), so the capabilities check fails the build as without a client certificate
  (test `new_channel_rejected_client_certificate_fails_connections_not_channel_creation`).
  `scripts/remote/tls-probe` with fresh X.509 v3 certificates: mTLS hit (both `1 remote cache
  hit`), client without its certificate (both 34), untrusted server with and without client
  certificate (both 34), with the fallback flags (both 0).
- `--experimental_remote_repo_contents_cache` silently ignored: now a loud
  `unported!("RemoteRepoContentsCacheImpl ...")` under Java's condition (startup flag set and a
  cache configured; the flag's default is false, `RemoteStartupOptions.java:28`).
- Remote flags on `run`: not done: `RunCommand` itself is not ported
  (`command_registry.rs`), so the loud refusal stays.

## Byte strings (the switch of 2026-11-19, commit 051c8c7)

The workspace moved to Bazel's internal string form during this round; the remote files were
converted by this round's workers (site list: docs/research/07-byte-strings-exec-sites.md).

- blaze_remote: the local seam wrappers are gone (`internal_to_unicode`/`unicode_to_internal`
  are the `blaze_util` functions, at the Java call sites only); `JavaHashMap::hash` runs over
  chars; `printErr` of a server message and `writeContentAsLatin1` use `latin1_encode_lossy`;
  OS paths go through `internal_to_platform`; `RemoteGrpcServiceConfig` is read as UTF-8
  (`RemoteGrpcServiceConfig.java:86`). `scrubber.rs` `translate_java_regex` keeps
  `java.util.regex.Pattern`'s ASCII semantics for `\w \d \s \b`, `.` and `(?i)` on Latin-1
  chars (28 rows of expectations produced by running Java's `Pattern`); the BEP uploader's log
  patterns got the same treatment. The Java inconsistency is kept: `Execute` converts with
  `internalToUnicode` where `lookupCache`'s `inline_output_files` does not
  (`RemoteExecutionService.java:788-789,2043`).
- blaze_exec: template content is read and written as ISO-8859-1 (`Template.java:107`,
  `LocalTemplateExpansionStrategy.java:51`; the `unported!` for a template that is not UTF-8 is
  gone); the execution logs convert exactly the fields Java converts (`CompactSpawnLogContext`
  every path except `empty_files`, `ExpandedSpawnLogContext` only the environment); the
  in-memory link size counts chars (`InMemoryLinkInfo.java:60`).
- Tests: the six `remote_execution_service_test` fixtures wrap their Unicode literals where
  Java's tests call `StringEncoding.unicodeToInternal`; the two "both sides of the seam" tests
  keep the internal side.
- Oracle: `scripts/remote/nonascii-probe` (non-ASCII source, output, argument, environment, tree
  child, symlink target): cold disk cache; each binary reading the other's disk cache (`4 disk
  cache hit, 1 internal` both ways, so the action digests are equal); remote execution with
  `compare-grpc-log` 39 of 39 RPCs MATCH; minimal mode and a local consumer;
  `--remote_header=x=aé` received as `a??` from both. `scripts/remote/strings-probe`: template
  bytes (valid and invalid UTF-8) identical; `readlink` of non-ASCII symlinks identical; JSON,
  binary and compact execution logs equal after normalisation, with the same converted and raw
  fields on both sides.

## S3 and S4

- Fixed: `wait_for_bulk_transfer` flag (R4); `await_in_progress_tasks` waits for a snapshot;
  `sync_file` reports the open error; `inline_output_files` is not sorted; `DigestOutputStream`
  hashes before writing; `--remote_header` values are sent as US-ASCII with `?`;
  `channel::testing` is behind `cfg(test)` or the `testing` feature; the omitted `DigestUtil`
  overloads carry PORT markers; stale `NEEDS(...)` and PORT comments are rewritten;
  `UnresolvedSymlinkAction` prints the raw target (oracle: `with target 'a//b/../xxx…'`);
  `is_remote_file` downcasts the file system; `FileSystem::get_host_file_system`; the missing
  marker at `OutputFile`/`OutputDirectory.path`; docs/remote.md (stale R7 and R9 sentences, the
  security table by function instead of line, the not-ported table).
- Ignore reasons: every `#[ignore]` in the modules touched was re-read; two were false after
  this round's ports and are un-ignored (`create_output_directories_after_missing_parent_delete`,
  `stat_and_exists_is_not_directory`), as are `test_parse_platform_legacy_options`,
  `lost_input_during_upload_fails_without_retries_or_execution` and the two `SymlinkToFile`
  traversal tests.
- Not done: the stale-downloads warning is still printed later than Java's replayed event for
  `build`/`test` (for query, cquery, aquery and info it now comes before `INFO: Invocation ID`,
  as the oracle); `SubTreeCache::INITIAL_PURGE_AT`; an invalid `--remote_header` name panics
  (the oracle crashes with exit 37); the second N12 site
  (`recursive_filesystem_traversal_function.rs`) belongs to another owner.

## Rulings and known deviations after this round

1. Action rewinding is not ported: the default path is loud and names `--norewind_lost_inputs`.
2. Disk cache action-result integrity check always on (Bazel: off under the default
   `--rewind_lost_inputs`).
3. X.509 v1 certificates are rejected (rustls/webpki); Bazel accepts them. Upstream's own test
   certificates are v1, so `remote_execution_tls_test` (2 functions) and the 4 functions of
   `remote_execution_mtls_test` fail on Blaze for this reason.
4. Windows execution platform: loud.
5. Transport error cause texts (Netty's address choice, TLS library texts): docs/remote.md,
   "Known deviations".

## Open after this round

1. Action rewinding, `RemoteLeaseExtension`, `RemoteImportantOutputHandler` (S3; rulings above).
2. `DynamicConnectionPool` (one HTTP/2 connection per endpoint; non-default values loud).
3. HTTP cache, credential helpers, remote downloader, remote repo contents cache, scrubbing
   config, disk cache GC, path mapping, CDC chunking, output service: loud, unchanged.
4. `run` and `coverage` with remote flags: the commands are not ported.
5. Lost inputs during input discovery are not checked (no action file system in the discovery
   context; marked in the code).
6. `Action::prepare` takes only the exec root (Java also passes the path resolver and the bulk
   deleter); cquery now runs `start_build` through the shared preparation path, which Java's
   cquery does not.
7. The unresolved-symlink digest (`hashString`) of a non-ASCII target has a unit test only: the
   oracle comparison needs `dump --action_cache`, which is not ported.
8. `ExecuteResponse.message` printing has no oracle probe (Bazel's worker never sets it).
9. Seen twice and not reproduced: a build that failed its capabilities check printed "Build
   did NOT complete successfully" and the recorded exit code was 0 (see "Numbers").
10. The `(Summary)` block of failed test attempts and `command.profile.gz` in the BEP are
    missing (UI and profiler, not remote).
11. `TextFormatTest`: 4 of about 95 methods ported (the rest test the printer and message types
    the reader does not handle); `ActionOutputDirectoryHelperTest`: 2 of 8 ignored (need an
    exception-injecting file system).

## Numbers

- Evidence binary: `~/.cache/blaze/work/remote/tl-blaze-1`, sha256 `43db45a1caf91dff…`, built
  from the working tree at `5a4e4a0` plus three then-uncommitted `blaze_remote` files
  (`byte_stream_build_event_artifact_uploader.rs`, `grpc/channel.rs`, `tests/scrubber_test.rs`;
  all committed since). One whitespace-only `rustfmt` change to `remote_spawn_runner.rs` was
  made after the binary was built. The per-finding tables above quote the workers' runs with
  their own binaries (`tb-`, `ta-`, `tc-`, `td-`, `te-`, `tg-`, `th-blaze-N`); every probe was
  then run again, oracle first, with the evidence binary by two workers who changed nothing
  (labels `p1-…`, `p2-…`, `tl-na-…`), and the statements below are from those runs.
- Probes on the evidence binary. Same as the oracle in the lines each tool compares, exit codes
  and outputs: `probe-trees` (3 steps), `probe-trees2` (3), `probe-inprocess`, `shard-probe`
  (2 cases), `symlink-probe` (7), `second-build` brotli toplevel and minimal (6),
  `pathcheck-probe`, `recreate-probe` (tree and non-tree, 8), `outpath-symlink-probe` (3),
  `nonascii-probe` (6 steps, `ALL SAME`), `strings-probe` (json, binary, compact, disk cache
  both ways), `stale-probe` (query, build, clean, and 12 remote variants), `srcdir-probe` (5),
  `cplookup-probe` (6), `evict-probe` (4 sources × `--norewind_lost_inputs` and × retries=0;
  `exit39`: exit 39 on both; `disk --norewind_lost_inputs`), `caps-matrix` (18 of 20 rows),
  `tls-probe` (mtls-ok, untrusted-mtls-fb), `platform-probe` (28 of 30 remote cases; 7 of 7
  execution-log cases), `attempts-probe` (processes line, exit code, testlogs, BEP).
- Differences on the evidence binary, all of them:
  - expected by the rulings: the four default-flags lost-input rows (oracle rewinds, exit 0;
    Blaze `blaze: not ported yet: ActionRewindStrategy.prepareRewindPlanForLostInputs: action
    rewinding (--rewind_lost_inputs, the default) for the lost inputs of action '…';
    --norewind_lost_inputs retries the build instead`, exit 101; the sticky sequence then
    rebuilds: `3 processes: 1 internal, 2 linux-sandbox`, exit 0); `evict-probe disk` with
    default flags (oracle `3 processes: 1 disk cache hit, 1 internal, 1 linux-sandbox`, Blaze
    `3 processes: 1 internal, 2 linux-sandbox`, both exit 0); `platform-probe os-windows`
    (loud);
  - transport texts, exit codes equal: `caps-matrix unreach-exec` (the oracle named
    `localhost/[0:0:0:0:0:0:0:1]:9499` in this run, Blaze `localhost/127.0.0.1:9499`),
    `tls-http-cache` (`not an SSL/TLS record: …` vs `received corrupt message of type
    InvalidContentType`), `tls-probe untrusted-*` (`General OpenSslEngine problem` vs `invalid
    peer certificate: UnknownIssuer`), and `tls-probe mtls-noclient`: oracle `UNAVAILABLE: ssl
    exception`, Blaze `received fatal alert: CertificateRequired` (the mapping to "ssl
    exception" covers an alert during the connect, not this one, which arrives after the TLS 1.3
    handshake; exit 34 on both);
  - not remote: Blaze prints no `Critical Path` in the elapsed-time line, no progress lines
    (`Loading:`, `Analyzing:`, ` no actions running`, which is also the only difference in the
    `platform-probe escapes` error line), no `FAILED: //:f (Summary)` block and no final
    `FAILED: ` / `INFO: ` line; it prints `INFO: Found 1 target...` in failing builds where
    Bazel does not; `dump --action_cache` and `info` (`CommittedHeapSizeInfoItem`) are not
    ported; the BEP lacks `command.profile.gz`.
  - OPEN, not explained: `tls-probe mtls-noclient`, first Blaze run: stderr ends with "ERROR:
    Build did NOT complete successfully" and the recorded exit code is 0
    (`~/.cache/blaze/work/remote/tls-p2-b-mtls-noclient/run1/{stderr,exit}`); two immediate
    reruns and ten later ones: exit 34, identical stderr. A worker saw the same once before, on
    the HTTP/1 row of `caps-matrix`. Both happened while other heavy jobs were running. 46
    targeted reruns (22 sequential, 24 in parallel against a closed port) all exited 34. The
    reviewer should treat it as an unresolved suspicion of a wrong exit code after a failed
    capabilities check.
- Default path: recording `blaze-remote-1` (brotli `build //...`, re2 `test //:all
  //python:all`; `scripts/oracle/record` with the evidence binary; score
  `~/.cache/blaze/work/remote/tl-score-1.json`): `scripts/gate/compare-scores
  corpus/scores/blaze-ef66f55e904e.referee2.json` prints `rows 39  gained 0  regressed 0`,
  `result: PASS (no surface lost ground)`; matching 13850 of 158988 on both sides (workerd is
  in the denominator and not recorded under either label).
- `scripts/remote/matrix` with the evidence binary: brotli `verdict: PASS` (D1 to D4, C1 to C5,
  E1, E2 as before: `43 disk cache hit` / `43 remote cache hit` / `43 remote`; C3 51 of 51, C4
  295 of 295, E2 1274 of 1274 items MATCH); re2 `verdict: PASS` (`550 disk cache hit`, `550
  remote cache hit`, `529 remote`; C3 600 of 600 MATCH; E2 15202 of 15202 items MATCH; C4 vs C1
  DIFFERENT under the allow-file entry `re2-cold-test-actions`, as in round 1).
- Bazel's own remote tests (`blaze-remote-1`): 115 of 253 oracle-passing functions (baseline
  binary: 110); 7 gained, 2 false passes now fail honestly; the 138 failures by cause: 39 remote
  (35 loud `unported!`, 4 the X.509 v1 ruling), 99 not remote. docs/remote.md, "Bazel's own
  remote tests".
- Rust tests, whole workspace (`scripts/gate/run-tests`, working tree at `cc7373c`, dirty with
  other agents' files): 6615 passed, 0 failed, 2021 ignored, 400 test executables, no problems.
  `scripts/gate/compare-tests` against the baseline: retained 5476, recovered 12, new 1127, lost
  0, absent 11; the 11 absent names are seam tests of the byte-strings switch in `blaze_actions`
  (7), `blaze_cc_features` (2), `blaze_repo_rules` (1) and `starlark_conformance` (1), none in a
  remote file. Per crate (passed / ignored): blaze_remote 630 / 62, blaze_exec 569 / 76,
  blaze_sky_exec 85 / 21, blaze_runtime 97 / 53, blaze_bep 99 / 26, blaze_buildtool 77 / 44,
  blaze_vfs 91 / 8, blaze_protos 26 / 0.
- `clippy --all-targets --no-deps -D warnings` at `780fceb`: clean for blaze_remote, blaze_exec,
  blaze_sky_exec, blaze_bep, blaze_buildtool, blaze_vfs; blaze_runtime fails on two lines of
  `commands/mod_command.rs` (another implementer's file in progress). `rustfmt --edition 2024
  --check` clean on the remote files.

# Round 3 response

- Reviewer's verdict: STOP at `e370bf9` (docs/reviews/2026-11-19-remote.md, "# Round 3"), one
  blocking finding (P1).
- Workers of this round (Opus): R1 (P4, P1, P5), R2 (P2, P7), R3 (P6, S4), R4 (P8). Same method
  as round 2. P3 is fixed by another implementer in `blaze_protos` (not in this response).
- Tree, binary and numbers: "Numbers" at the end.

| Finding | State |
|---|---|
| P1 SIGPIPE from Blaze's own sockets (S1) | observed, then fixed |
| P2 input discovery without the action file system | fixed; evidence is unit tests (the oracle cannot build the reviewer's scenario on this machine) |
| P3 TextFormat `from_utf8_lossy` | not mine (another implementer, `blaze_protos`) |
| P4 wrappers record `systemd-run --wait`'s status | fixed: status recorded inside the unit |
| P5 post-handshake TLS alert | fixed, and the race behind it (grpc-netty's shutdown status ported) |
| P6 regex dialect | fixed for every caller that compiles a user's Java pattern: one port of `java.util.regex.Pattern`'s parser |
| P7 lost-input residuals | fixed (both) |
| P8 tests | fixed: all 30 `ActionOutputMetadataStoreTest` methods have a Rust test (26 pass, 4 ignored for named missing ports); both helper tests un-ignored |
| S4 (8) | 7 fixed, 1 comment left for the file's owner; one new deviation found (completion path resolver) |

## P1 (S1). SIGPIPE: OBSERVED, FIXED

- Observed first, with the binary before the fix (`r1-blaze-0`) and the status recorded inside
  the unit (P4; `strace` is not installed on this machine):

| scenario (`scripts/remote/sigpipe-probe`) | result before the fix |
|---|---|
| `mtls-noclient` (TLS 1.3 server requiring a client certificate) | run 1: exit 34; run 2: `141 (killed by SIGPIPE)`, stderr identical to run 1 |
| `http-cache` (`grpc://` to an HTTP/1 server) | runs 1 and 2: exit 34; run 3: killed by SIGPIPE |
| `reset-peer` (new `scripts/remote/reset-peer`: a proxy in front of Bazel's worker that sends FIN then RST while the client writes; cache-only build) | 14 runs exit 0; run 15: "Build completed successfully" and killed by SIGPIPE |

  The reviewer's reading is confirmed, including why 0 had been recorded: `systemd-run --wait`
  returned 0 for the SIGPIPE death.
- Both sides read. C++ client (`src/main/cpp/blaze_util_posix.cc:141-229`, `blaze.cc:2082-2205`):
  the handler records SIGPIPE; when writing the server's output fails with EPIPE the client
  cancels and `PropagateSignalOrExit` raises the signal. Java server: the JVM ignores SIGPIPE;
  a reset remote socket is an `IOException`. Together: `bazel … | head -1` dies by SIGPIPE; a
  remote endpoint that resets does not touch the exit status.
- Fix. `blaze_client/src/signal_handler.rs`: SIGPIPE is ignored (`SIG_IGN`) instead of handled;
  new `note_broken_pipe()` sets the recorded signal (only when the handler is installed, as in
  Bazel, where batch mode has no handler); `propagate_signal_or_exit` is unchanged.
  `blaze_events/src/io.rs`: `set_broken_pipe_listener` and a wrapper around the process's
  stdout and stderr in `OutErr::system_out_err` that calls it when a write or flush fails with
  `BrokenPipe` (the writes that correspond to the C++ client's `fwrite` to fd 1 and 2).
  `crates/blaze/src/main.rs` wires the two. PORT notes at each site.
- Tests (`blaze_client/tests/signal_handler_test.rs`, each in a child process):
  `writev_to_a_reset_peer_does_not_kill_the_process`,
  `epipe_on_stdout_ends_the_process_with_sigpipe`, `broken_pipe_before_install_is_not_recorded`;
  `blaze_events` `std_out_err_writer_reports_broken_pipe`.
- Shown after the fix (in-unit status): `mtls-noclient` 30 of 30 exit 34 (and 50 of 50 with the
  final transport code, P5); `http-cache` 30 of 30 (50 of 50) exit 34; `reset-peer` 30 of 30
  exit 0 with one RST each (oracle: 10 of 10 each). Pipe closes, `${PIPESTATUS[0]}` inside the
  unit: `build 2>&1 | head -1` 141 on both; `build | head -1` 0 on both; `query //... | head -1`
  (5000 targets) 141 on both.
- Left for other owners: `blaze_client/src/util/file.rs` `write_to_std_out_err` (the separate
  client of the daemon mode) must call `note_broken_pipe()` in its EPIPE branch, and the PORT
  note at `blaze.rs:~610` is stale (both files were mid-edit by the client-daemon work; passed
  to root). Not remote: `info | head -1` is 37 on Blaze (full `info` crashes on
  `CommittedHeapSizeInfoItem.get`), and `query 2>&1 >/dev/null | head -1` is 0 (Blaze prints no
  progress lines to stderr).

## P4. Wrappers: FIXED

New `scripts/remote/unit-supervisor` runs as the unit's main process, forwards SIGINT, SIGTERM,
SIGHUP and SIGQUIT to the command, and writes `exit N` or `signal N NAME` to a status file;
`scripts/remote/unitstatus.py` is the shared reader. `ws-run`, `oracle-run`, `symlink-probe`,
`platform-probe`, `nonascii-probe`, `cplookup-probe`, `strings-probe` and `srcdir-probe` use it:
the `exit` file holds the plain code (128+N after a signal) and a `signal` file names the
signal; the printed line is `exit code: 141 (killed by SIGPIPE)`. No wrapper under
`scripts/remote` records `systemd-run --wait`'s own status any more (`worker` and `evict-probe`
start services without `--wait` and record none; `matrix` goes through `oracle-run` and
`ws-run`). Checked through each wrapper: exit 3, `kill -PIPE $$` (141), `kill -TERM $$` (143),
an oracle build that fails (1) and one that passes (0), and `--sigint-after` /
`--sigint-on-glob` (8).

## P5. Post-handshake alert: FIXED, with the race behind it

- The classification: `is_tls_alert` is used in all three arms of `classify_transport_error`.
  With that alone the oracle's line appeared in 26 of 30 runs; in 4 the alert was lost inside
  hyper and the waiting call failed as `CANCELLED: operation was canceled`.
- The race: grpc-netty gives every stream of a connection that failed the connection's shutdown
  status, and the first status wins (`ClientTransportLifecycleManager.notifyShutdown`,
  `NettyClientHandler.onConnectionError` / `channelInactive` / `createStream`, grpc-java 1.71.0);
  Netty also flushes preface, SETTINGS and the first request before the peer can have answered,
  so its first failure is what it reads. Ported in `grpc/channel.rs`: `ConnectionLifecycle`
  (first cause wins; `on_bytes` is `Http2ConnectionHandler.PrefaceDecoder.verifyFirstFrameIsSettings`
  with its text), `RecordingReads` and `DroppingRefusedWrites` around the connection's I/O, and
  `transport_status` gives a call that failed only because its connection went away the
  connection's status. A call cancelled by Blaze itself (deadline, interrupt), a connect or
  handshake failure and a status sent by the server are untouched. PORT: tonic applies its TLS
  out of reach of a wrapping layer, so `new_channel` validates the settings through tonic as
  before and then connects with the crate's own connector (the same rustls configuration, ALPN
  h2 required). Not ported: a clean EOF with no other failure records nothing (Netty: "Network
  closed for unknown reason"), which keeps the `closed-cache` row equal to the oracle.
- Tests (`channel.rs`): `tls_alert_after_the_handshake_is_unavailable_ssl_exception` (no relay),
  `http1_server_is_internal_http2_exception`, `plain_peer_reset_is_unavailable_io_exception`:
  200 sequential and 200 eight-way parallel runs each, 0 failures (the HTTP/1 test failed 3 of 15
  before); `only_a_call_cancelled_by_its_connection_gets_the_shutdown_status`,
  `call_cancelled_by_its_deadline_keeps_its_status`, `is_tls_alert_matches_rustls_alert_text_only`.
- Oracle: `mtls-noclient` 50 of 50 runs print `Failed to query remote execution capabilities:
  UNAVAILABLE: ssl exception` (oracle 10 of 10); `http-cache` 50 of 50 `… INTERNAL: http2
  exception`; `tls-probe` and `caps-matrix` otherwise unchanged (20 of 20 exit codes equal;
  texts differ only in the listed rustls/OpenSSL and address-choice rows).

## P2. Input discovery on the action file system: FIXED

- `skyframe_action_executor.rs` `discover_inputs` is `SkyframeActionExecutor.java:922-1004`: the
  discovery context is built on the action file system (exec root, input paths, out/err),
  `updateActionFileSystemContext`, the lost-input check after a successful discovery (:967) and
  after a failure (:976; a lost input wins over the discovery error), handled as in execution.
  `action_execution_function.rs` passes the file system of the output metadata store, the same
  instance execution uses in that call (PORT: no `SkyKeyComputeState`; a restart creates it
  again). `setFromInputDiscovery` only selects dependencies for the rewind plan (PORT comment).
  `CppCompileAction`'s modmap read already went through the context's input path.
- Evidence, stated plainly: the oracle cannot build C++20 modules on this machine (no clang; with
  gcc 16.2 `aggregate-ddi` fails with "Invalid JSON string" because the `.ddi` is not P1689
  JSON; `scripts/remote/discovery-probe oracle` shows it), and no other in-process discovery
  reader exists, so there is no oracle comparison. The evidence is two unit tests that fail when
  discovery is given no file system:
  `input_discovery_reads_a_remote_only_input_through_the_action_file_system` and
  `input_lost_during_discovery_fails_as_without_rewinding`, plus unchanged results of
  `probe-inprocess`, `probe-trees2`, `shard-probe` and `evict-probe … inprocess`.
- The false row in docs/remote.md "Known deviations" is removed.

## P7. Lost-input residuals: FIXED

- (a) `remote_action_file_system.rs` `download_if_remote`: the helper thread adopts the caller's
  interrupt status; `get_input_stream` restores the flag and reports "Received interrupt while
  fetching file '…'" (`RemoteActionFileSystem.java:381-385`). Test
  `get_input_stream_interrupted_while_download_in_flight` (fails without the adoption).
- (b) The purge before the rewinding `unported!` now works on the executor's own action cache
  (`ActionCacheChecker::get_action_cache`), under a lock that `update_action_cache` holds while
  it writes, and sets a flag after which nothing is put (PORT: Java has no such path); the
  second on-disk handle is gone. Test
  `lost_inputs_with_rewinding_purge_the_action_cache_before_failing_loudly`: an action that
  finishes after the purge does not bring an entry back, in the live cache or on disk.
  `evict-probe` sticky sequence, also with `--jobs=8` and five more cached actions in flight:
  the loud failure, then `8 processes: 3 remote cache hit, 3 internal, 2 linux-sandbox`, exit 0.

## P6. Regex dialect: FIXED (one translation for the whole tree)

- New `blaze_util::java_regex` (`crates/blaze_util/src/java_regex.rs`, with a case table generated
  from JDK 25): a port of JDK 25 `Pattern`'s parser with Java's cursor (`RemoveQEQuoting`, `expr`,
  `sequence`, `atom`, `escape`, `clazz`, `range`, `family`, `group0`, `addFlag`/`subFlag`,
  `closure`, …) that writes `regex`-crate syntax. Because the cursor is Java's, a syntax error
  carries Java's `PatternSyntaxException.getMessage()`.
- Translated exactly: ASCII `\w \d \s \b \B`; `\h \v \R`; Java's `.` line terminators;
  `(?i)` with ASCII folding, `(?iu)` and `(?U)` by Java's rules; `\Q..\E`; octal, `\x`, `\u`,
  `\c`, `\e`, `\a`; an escaped non-alphanumeric character is that literal (`\<` is `<`); class
  union, intersection and negation; POSIX, general-category and binary properties; flag scoping;
  `LITERAL`, `MULTILINE`, `DOTALL`.
- Loud (`unported!` naming the construct): backreferences, lookaround, `\G`, `\X`, `\b{g}`,
  `\N{..}`, Unicode blocks and scripts, `java*` properties, lone surrogates, the flags `(?d)`,
  `(?x)`, `(?c)`. Possessive quantifiers and atomic groups are translated only where nothing can
  follow them and the pattern is used with `find()` (the shape `RegexFilter.takeUnionOfRegexes`
  builds, `(?:(?>a)|(?>b))`, used by the default `--instrumentation_filter`); anywhere else they
  are loud. The old rewrite of `(?>` to `(?:` in `JavaPattern` was wrong (`(?>a*)a` never
  matches in Java) and is gone. Checked per input by `Pattern` (loud only when the input could
  make Java differ): `$` and `\Z` before a final line terminator, `(?m)` anchors around `\r`,
  U+0085, U+2028, U+2029, `\b` next to a non-spacing mark. Remaining deviation: a pattern the
  crate rejects after translation (a repetition count over its limit) gets the crate's text.
- Callers now on it: `blaze_options` `RegexPatternOption` (`Pattern.compile(internalToUnicode(input),
  DOTALL)` and `RegexUtil.asOptimizedMatchingPredicate`, with Java's conversions and literal
  optimisation), `blaze_optdefs` `JavaPattern` (`RegexFilter`), `blaze_query_engine`
  `compile_java_pattern` (also used by `aquery`), `blaze_cquery` `AqueryActionFilter`,
  `blaze_events` `OutputFilter`, `blaze_rules` and `blaze_packages` (`String.matches`,
  `PyBuiltins.regexMatch`), `blaze_cc_features` `CoptsFilter`, and `scrubber.rs` (its own
  translator is removed). Not routed: `blaze_repo_fetch` `UrlRewriterConfig` (needs the
  exception's description and index, and captures: two more functions in `blaze_util`, left for
  one later change), `UnixGlob` (Bazel builds that regex itself) and constant patterns written
  in crate syntax.
- This went beyond what the review asked (root accepted it, with conditions): eight caller
  crates changed, not the option converter alone, and constructs that the old untranslated or
  subset compilation accepted are now loud. `docs/java-regex.md` lists every construct with its
  state, every match-time condition with an example, the callers and the sites not routed.
- Conditions checked: a static scan of what Blaze really evaluates (Bazel's builtins and tools,
  the corpus checkouts, the 1,160 external repositories recorded for brotli, re2 and workerd, and
  the option defaults; `regex_match`, `nocopts`, query filters and every regex-valued option): 126
  uses, 49 distinct patterns, none with a syntax error, none containing a construct with a
  match-time condition, and one that is now loud: workerd's `.bazelrc:702`
  (`common:bazelflare-generated --remote_download_regex=bazel-out/(?![^/]*-exec)…`, negative
  lookahead; only with that `--config`; before, the crate rejected it as "Not a valid regular
  expression", so it never worked; Bazel accepts it): open item 3. workerd guards with the new
  code: `scripts/workerd/cquery-compare` 38,892 of 38,892 (order equal) and
  `scripts/w3/query-compare --project workerd` 46,371 of 46,371, both unchanged.
- Tests: `blaze_util/tests/java_regex_test.rs`: 381 rows generated by
  `tests/data/JavaRegexRows.java` on JDK 25 (about 296 match rows with span and `matches`, 50
  exact error messages, 35 that must be loud), with ASCII, Latin-1 and non-Latin-1 inputs.
  `RegexPatternConverterTest.unicodeClass` is un-ignored and passes.
- Oracle (`scripts/remote/regex-probe`: one action with outputs `out_a.txt`, `out_A.txt`,
  `out_ê.txt`, `out_<b>.txt`, `out_é.txt`; oracle-seeded disk cache;
  `--remote_download_outputs=minimal --remote_download_regex=P` for 8 patterns): the same files
  are materialised by the oracle and Blaze for all 8. The round-2 binary differs on 3 of them
  (`.*out_\w+\.txt` also fetched the é and ê outputs; `(?i).*OUT_Ê\.TXT` fetched `out_ê`;
  `.*_\<b\>\.txt` fetched nothing), so the probe detects the finding.

## P8. Tests: FIXED

- `ActionOutputMetadataStoreTest`: 30 Java methods, 30 Rust tests of the same names: 26 pass (7
  existing, 19 new in `blaze_sky_exec/src/action_output_metadata_store_tests.rs`, among them
  `inject_remote_artifact_metadata` and `inject_remote_tree_artifact_metadata`), 4 ignored, each
  with the Java body ported up to the missing call: `get_metadata_from_fileset_mapping`
  (`ActionInputMap.putFileset`, `ActionInputMap.java:388`), the two
  `file_artifact_value_from_artifact_compatible_with_get_metadata_*`
  (`FileArtifactValue.couldBeModifiedSince`, `FileArtifactValue.java:235`),
  `file_artifact_value_for_symlink_read_from_cache` (the `DigestUtils` digest cache). Java's
  `chmodCalls` is a recording file system, so those assertions are exact. No store deviation was
  found by the new tests; a table is at the top of the file.
- An existing port was flaky (36 failures in 200 runs): it built its expected value before the
  store's chmod changed the child's ctime; Java evaluates the store first. Reordered as Java;
  0 failures in 300 runs.
- `action_output_directory_helper_test.rs`: both ignored tests run now with a port of Java's
  exception-injecting `DelegateFileSystem` (8 passed, 0 ignored).
- Seen while porting and fixed (no Java test exercises them): `get_all_tree_artifact_data`
  returns the whole map including the missing-tree marker (`ActionOutputMetadataStore.java:144`;
  `MISSING_TREE_ARTIFACT` is a shared value recognised by identity, as `TreeArtifactValue.java:428`),
  and `ActionExecutionValue::create` has Java's checks in Java's order (`ActionExecutionValue.java:69-93`:
  "… should only be stored in a TreeArtifactValue", "Missing digest for …", the marker's
  `UnsupportedOperationException`, "Missing digest for file … in tree artifact …"); six tests.
  `ActionExecutionValueTest`'s two methods (`equality`, `serialization`) are ported and ignored
  (`equals` of the subclasses and the codecs are not ported).
- `PathCanonicalizerTest`: all 28 methods ported
  (`blaze_exec/src/exec/path_canonicalizer_tests.rs`): 26 pass, 2 ignored (Windows only: Java's
  `assumeTrue(OS.getCurrent() == OS.WINDOWS)`); the canonicalizer needed no change.
- Audit of the other `lib/remote` test classes (script: `@Test` method names against `fn
  <snake_name>` in the Rust files naming the class): 39 classes complete (among them
  `RemoteExecutionServiceTest` 70, `RemoteSpawnRunnerTest` 42, `GrpcCacheClientTest` 37,
  `RemoteActionFileSystemTest` 52 + 72, `DiskCacheClientTest` 28). Classes with no Rust tests
  are those of features that are not ported (chunking, HTTP cache, downloader, connection pool,
  output service, rewinding synchronizer, `RxFutures`, `ReaderPreferringReadWriteLock`), the
  integration test classes (`BuildWithoutTheBytesIntegrationTest`, `DiskCacheIntegrationTest`,
  `RemoteModuleIntegrationTest`: they need Bazel's Java integration harness, which has no
  counterpart; the probes under `scripts/remote/` stand in for them).

## S4: 7 FIXED, 1 LEFT

- `platform_utils.rs`: the properties keep the iteration order of the Java map each branch uses
  (a `JavaHashMap` port for the merge branch: `String.hashCode`, JDK resize and pre-sizing; a
  treeified bin is loud); 6 tests with names that collapse to U+FFFD, orders generated with the
  JDK (`tests/data/PlatformPropertyOrder.java`). The private copy of this code in `blaze_exec`
  `spawn_log_context.rs` still has the old order (not touched).
- `disk_cache_client.rs` `to_path_from_hash` takes the prefix by chars;
  `bulk_transfer_exception.rs` sorts by UTF-16 unit; `inline_output_files` is an `IndexSet`;
  `create_hard_link` is Java's call as written (the arguments end up swapped, so it fails with
  "File … does not exist" unless the link path exists); `stat_from_metadata` gives -1 for
  constant metadata.
- `blaze_bep` `completion_context.rs`: the comment was stale and the deviation behind it is
  real: Java resolves completion paths through a `RemoteActionFileSystem` over the important
  inputs when the remote output service is active (`SkyframeExecutor.java:659`,
  `RemoteOutputService.java:220`); Blaze uses the identity resolver. Now stated as not ported
  in the comment; open item 1 below.
- Left: the stale comment in `blaze_runtime` `bep.rs` `after_command` (the file is being edited
  by the build-event implementer; the wording is with root).

## Open after this round

1. `CompletionContext` uses the identity path resolver where Java resolves through a
   `RemoteActionFileSystem` over the important inputs when the remote output service is active
   (`SkyframeExecutor.java:659`, `RemoteOutputService.java:220`): a remote-only output is not
   found by the completion path (BEP file URIs under build without the bytes). Found by a worker
   this round, not in the review; not fixed (planned with the build-event owner).
2. P2 has no oracle comparison (see P2).
3. Negative lookahead is loud in `java_regex`; workerd's opt-in `--config=bazelflare-generated`
   uses it in `--remote_download_regex` (it did not work before either).
4. For the client-daemon owner: `blaze_client/src/util/file.rs` `write_to_std_out_err` must call
   `signal_handler::note_broken_pipe()` in its EPIPE branch; the PORT note at `blaze.rs:~610` is
   stale.
5. A special-file output (`createFromInjectedDigest(value, null)`,
   `skyframe_action_executor.rs:~2566`) would now trip "Missing digest for …" in
   `ActionExecutionValue::create`, exactly as Java's check would; not exercised by any test.
6. The private copy of the platform-property code in `blaze_exec` `spawn_log_context.rs` keeps
   a `HashMap`'s order for names that collapse to U+FFFD (the `blaze_remote` copy is fixed).
7. Unchanged from round 2: action rewinding, `RemoteLeaseExtension`,
   `RemoteImportantOutputHandler`, `DynamicConnectionPool`, and the loud features listed in
   docs/remote.md; a clean EOF from a peer with no other failure records no shutdown status.
8. The work was paused by root before the final evidence run was complete: see "Numbers" for
   exactly what was and was not run on the final binary.

## Numbers

State when the work was paused (2026-11-19, about 17:15), said exactly:

- Final binary: `~/.cache/blaze/work/remote/tl-blaze-2`, sha256 `018fa76d28cf3640…`, built from
  the working tree at `de5a476` with every round-3 change in it (all of them are committed in
  root's checkpoints since; HEAD at the pause: `c55ec7c`, `check --workspace --all-targets`
  passes).
- RUN on the final binary:
  - default path: recording `blaze-remote-2` (brotli, re2); `scripts/gate/compare-scores
    corpus/scores/blaze-ef66f55e904e.referee2.json ~/.cache/blaze/work/remote/tl-score-2.json`:
    `rows 39  gained 2  regressed 0`, `result: PASS (no surface lost ground)` (matching 13852 of
    158988; the 2 gained items come from other agents' work);
  - `scripts/remote/matrix brotli`: `verdict: PASS` (D1 to D4, C1 to C5, E1, E2; C3 51 of 51, C4
    295 of 295, E2 1274 of 1274 MATCH);
  - `scripts/remote/matrix re2`: INCOMPLETE: D1 to D4 and C1 to C3 as before (`550 disk cache
    hit`, `550 remote cache hit`, C3 600 of 600 MATCH); stopped during C4; C5, E1, E2 and the
    verdict were not produced.
- NOT RUN on the final binary: Bazel's own remote tests (label `blaze-remote-2` does not exist;
  the last full run is `blaze-remote-1`, 115 of 253, round 2); `scripts/gate/run-tests` for the
  whole workspace.
- Probes on the final binary (worker P3, stopped at the pause): steps 1 to 4 and the remote half of step 5 were run; equal to the oracle: `sigpipe-probe`
  (`mtls-noclient` 30 of 30 exit 34 with the oracle's exact line; `http-cache` 30 of 30;
  `reset-peer` 30 of 30 exit 0; `build 2>&1 | head -1` 141 and `build | head -1` 0, as the
  oracle), `tls-probe` mtls-ok / mtls-noclient / untrusted-mtls-fb, `caps-matrix` 17 of 20
  rows, `probe-trees`, `probe-trees2`, `probe-inprocess`, `shard-probe`, `attempts-probe`,
  `symlink-probe` (7 steps), `second-build` (6 steps), `pathcheck-probe`, `recreate-probe` (8
  steps), `outpath-symlink-probe` (3), `evict-probe` (4 sources × `--norewind_lost_inputs` and ×
  retries=0; `exit39`: 39 on both; `disk --norewind_lost_inputs`), `platform-probe` 28 of 30.
  Different, all known: the ruled rows (default-flags lost inputs: the loud failure, now exit 37
  through the crash handler with "FATAL: bazel crashed due to an internal error" before the
  `not ported yet` line; `disk` default flags: Blaze rebuilds; `os-windows`: loud, exit 37);
  texts with equal exit codes (`untrusted-*`, `tls-http-cache`, and the two `unreach` rows,
  where the oracle itself named the IPv6 address in 10 of 18 runs and the IPv4 one in 8); not
  remote (`info | head -1`: Blaze exits 37 in 2 of 3 runs because full `info` crashes on
  `CommittedHeapSizeInfoItem.get`; `query //... | head -1`: 141 on both, but Blaze does not
  print "Cannot write to standard output; exiting..."; `platform-probe escapes`: only the
  oracle's trailing progress text ` | [2 / 2] no actions running`). One row to note: the sticky
  sequence's third step is exit 0 on both, but prints `3 processes: 1 internal, 2 linux-sandbox`
  on Blaze against `2 processes: 1 action cache hit, 1 internal, 1 linux-sandbox` on the oracle
  (Blaze purged its action cache in step 2, where the oracle rewound). NOT run on the final
  binary: `platform-probe --execlog`, `regex-probe`, `nonascii-probe`, `strings-probe`,
  `stale-probe`, `bep-interrupt-probe`, `discovery-probe`.
- Per-finding evidence with the workers' own binaries (built from the tree at the time of each
  fix; all listed in the sections above): P1 `r1-blaze-0` (before) and `r1-blaze-1`/`r1-blaze-2`
  (after); P5 `r1-blaze-2`; P2/P7 `r2-blaze-1`; P6 `r3-blaze-1`, workerd guards `r3-blaze-2`
  (tree `f58ddb4`). `tl-blaze-2` is later than all of them.
- Rust tests, whole crates, working tree at `de5a476` (passed / failed / ignored): blaze_remote
  641 / 0 / 62 (three consecutive runs earlier: no flake), blaze_exec 673 / 0 / 157,
  blaze_sky_exec 112 / 0 / 27, blaze_client 161 / 0 / 9, blaze_events 83 / 0 / 1, blaze_util
  55 / 0 / 0, blaze_options 536 / 0 / 32, blaze_optdefs 424 / 0 / 25.
- clippy `-D warnings`: clean for blaze_remote, blaze_sky_exec, blaze_client, blaze_events,
  blaze_util (workers' runs); blaze_exec fails only on other agents' `worker/*` files.
- Disk: this round's run directories under `~/.cache/blaze/work/remote` cost about 6 GB (binary
  copies `r1-blaze-0/1/2`, `r2-blaze-1`, `r3-blaze-1/2`, `r3-query-dev-2`, `tl-blaze-2` at about
  0.36 GB each, the rest small); output user roots of R1's runs (about 12 GB) and of the
  matrix and probe runs remain under `~/.cache/blaze/oracle-out/ur-remote`. One worker (R2)
  deleted three output user roots that were not its own (`rx-r2-blaze`, `rx-r2-blaze2`,
  `rx-r2-oracle`; recorded results untouched); reported to root at the time.
- To finish the evidence when work resumes: rerun `scripts/remote/matrix re2
  ~/.cache/blaze/work/remote/tl-blaze-2 <new tag> 9730`; run Bazel's remote tests with the
  command in docs/remote.md as label `blaze-remote-2`; run the probes P3 did not reach; then
  `scripts/gate/run-tests`.


# Round 3 evidence on e8b79c0

- Worker: implementer `remote-r3` (Opus), 2026-11-25, on root's brief. Reviewed commit:
  `e8b79c0`, binary `~/.cache/blaze/work/root-runs/blaze-e8b79c0` (sha256 `7455128a0ccfc453…`).
  Oracle: `~/.cache/blaze/oracle/bazel-9.3.2-linux-x86_64`.
- The suite run of e8b79c0 showed defects in ported remote code; root asked me to fix them first
  (section 2). Every fix is now in root's checkpoint `77eacad`. The fixed binary is
  `~/.cache/blaze/work/remote-r3/blaze-4` (sha256 `9086f54cb15dbf0f…`), built at 11:14 from the
  shared working tree (these fixes and the other agents' uncommitted work of that hour; root's
  checkpoints `987d9b3` and `77eacad` were made minutes later and hold all of my changes but the
  two script path overrides noted under "Tools").
- Every comparison below was run twice, on the e8b79c0 binary and on `blaze-4`, unless it says
  otherwise. Each tool says which binary it ran in its log (`~/.cache/blaze/work/remote-r3/`).
- Incident (mine, reported plainly): at 11:45 a removal command I generated wrongly (a `sed` that
  read and appended the same file left a third line ending in `…/remote-r3/ur/`) deleted my own
  output-root directory while three of my runs used it. Nothing outside
  `~/.cache/blaze/work/remote-r3/` was touched. The damaged runs, all on the e8b79c0 binary, were
  rerun and only the reruns are used here: matrix step `all cold` (exit 36, replaced by runs 11/12),
  `evict-probe ws … remote` with retries=0 (exit 36, rerun as `r3e2-remote-0`), and `probe-trees`
  (step 1 lost its install base, rerun as `r3e2`).

## 1. Bazel's remote tests in the suite run `blaze-e8b79c0`

Root's suite run (label `blaze-e8b79c0`, binary sha256 `7455128a…`, all 237 default targets in
suite mode); `scripts/upstream-tests/compare corpus/upstream-tests/oracle-9.3.2.json
blaze-e8b79c0 --all`, restricted to the remote targets. Base: the functions that pass on the
oracle (flaky-in-base functions excluded; none of them is in these targets).

| target | oracle-passing functions | pass on blaze-e8b79c0 | lost | absent |
|---|---|---|---|---|
| `//src/test/py/bazel:bazel_remote_cache_decompression_test` | 1 | 0 | 1 | 0 |
| `//src/test/py/bazel:remote_repo_contents_cache_test` | 38 | 0 | 38 | 0 |
| `//src/test/shell/bazel/remote:build_without_the_bytes_test` | 63 | 44 | 19 | 0 |
| `//src/test/shell/bazel/remote:remote_build_event_uploader_test` | 20 | 16 | 0 | 4 |
| `//src/test/shell/bazel/remote:remote_downloader_test` | 4 | 0 | 4 | 0 |
| `//src/test/shell/bazel/remote:remote_execution_http_test` | 15 | 4 | 11 | 0 |
| `//src/test/shell/bazel/remote:remote_execution_mtls_test` | 4 | 2 | 2 | 0 |
| `//src/test/shell/bazel/remote:remote_execution_sandboxing_test` | 3 | 3 | 0 | 0 |
| `//src/test/shell/bazel/remote:remote_execution_test` | 96 | 82 | 14 | 0 |
| `//src/test/shell/bazel/remote:remote_execution_tls_test` | 4 | 2 | 2 | 0 |
| `//src/test/shell/bazel/remote:remote_execution_with_xattr_test` | 1 | 0 | 1 | 0 |
| `//src/test/shell/bazel:external_remote_file_test` | 8 | 8 | 0 | 0 |
| `//src/test/shell/bazel:unix_digest_hash_attribute_name_test` | 1 | 0 | 1 | 0 |
| total | 258 | 161 | 93 | 4 |

The 97 lost or absent functions by root's failure family
(`~/.cache/blaze/work/root-runs/e8b79c0-failures/by-family.tsv`): remote-repo-contents-cache 38,
http-cache 13, remote-flags-other-commands 12, unclassified 10 (section 2), tls-v1-cert 4 (fixed),
no-server 3, path-mapping 3, collateral-xml-overflow 3 (the suite aborted after unittest.bash's XML
writer overflowed; those functions did not run), credential-helper 3, modify-execution-info 2,
rewinding 2, xattr-digest 2 (fixed), include-scanning 1, scrubbing 1. These numbers are
e8b79c0's; the suite was not rerun on `blaze-4` (root runs it).

## 2. Defects the suite showed in ported code: fixed

Evidence for each: `scripts/remote/fixes-probe` (new; one case per defect, written as the upstream
test writes its workspace), oracle (label `o`) against `blaze-4` (label `b4`). Exit codes and the
lines the upstream test checks, compared:

| case (upstream test) | oracle | e8b79c0 (suite) | blaze-4 |
|---|---|---|---|
| `cc-tree-cache` (`test_cc_tree_remote_cache_download_minimal`) | exit 0, `5 processes: 1 internal, 4 linux-sandbox` | crash, NPE in the Merkle tree | equal to the oracle |
| `cc-tree-exec` (`test_cc_tree_remote_executor_download_all`) | exit 0, `5 processes: 1 internal, 4 remote` | crash | equal |
| `start-lines` (`test_downloads_minimal_hit_action_cache`) | 6 `START`/`FINISH` lines (`START <no location>: [1 / 3] Executing genrule //a:foo`, …) | none | the same 6 lines |
| `tls-v1` (`remote_execution_tls_test test_remote_grpcs_cache`) | exit 0, `2 processes: 1 internal, 1 linux-sandbox` | `invalid peer certificate: …UnsupportedCertVersion` | equal |
| `xattr` (`remote_execution_with_xattr_test`) | `VFS md5` file1 0, file2 1; `VFS xattr` file1 1, file2 2, BUILD 1 | `unported!` (`--unix_digest_hash_attribute_name`) | `VFS md5` file1 0, file2 1; `VFS xattr` file1 1, file2 **1**, BUILD 1 |
| `exec-log` (`test_upload_minimal_upload_compact_exec_log`) | buildToolLogs: elapsed time, critical path, process stats, `command.profile.gz` bytestream, `execution_log.binpb.zst` bytestream | no `execution_log.binpb.zst` | the same five entries in the same order |

- Merkle tree NPE (`test_cc_tree_*`, 3). `ActionInputMap.getInputMetadataChecked` looks up every
  `TreeFileArtifact` through its parent tree (`ActionInputMap.java:218`); the port did so only for
  `isChildOfDeclaredDirectory()`, so the children of a tree that an action template produced (the
  `.pic.o` files of a C++ tree) came back null. Fix:
  `blaze_sky_exec/src/skyframe_action_executor.rs:3437` (77eacad). Test
  `get_input_metadata_template_expansion_output_from_parent_tree` (:5166; no Java test covers it).
- `START … Executing genrule` (2). `SkyframeActionExecutor.ActionRunner` posts
  `reporter.startTask(null, prependExecPhaseStats(message))` and `finishTask` (Java :1113, :1216);
  the port had a "not ported" note. Ported: `ProgressSupplier`, `ActionCompletedReceiver`,
  `setActionExecutionProgressReportingObjects`, `prependExecPhaseStats`
  (`skyframe_action_executor.rs:381-450`), the two events (:1016, :1088);
  `ExecutionProgressReceiver` implements both (`blaze_buildtool/…/execution_progress_receiver.rs:136`);
  `ExecutionProgressReceiverSetup` hands the receiver to the executor
  (`blaze_runtime/src/commands/build.rs:665`; PORT: no `ActionExecutionStatusReporter` argument).
- `1 process: 1 internal.` (2, `test_downloads_toplevel*`). Not a remote defect: on the unchanged
  second build Blaze prints `1 process: 2 action cache hit, 1 internal.` because it evaluates
  every command anew (no graph kept between commands, ruling of 2026-11-19 on the server, D2);
  Bazel's server keeps the graph. Left to the incremental work; not changed.
- `tls-v1-cert` (4). Bazel's test certificates are X.509 v1 (`server.crt`, `client.crt`). The JDK's
  PKIX trust manager accepts a v1 end entity and checks the host name against the common name
  when there is no subjectAltName; webpki rejects any certificate that is not v3. New
  `blaze_remote/src/authandtls/jdk_server_cert_verifier.rs`: every v3 certificate goes to rustls'
  `WebPkiServerVerifier` unchanged; a v1/v2 end entity is checked as a one-certificate PKIX path
  (trust anchor itself, or issued by an anchor whose key verifies its signature), validity, and
  the most specific CN (`HostnameChecker.matchDNS`); handshake signatures with its raw key. Wired at
  `google_auth_utils.rs:429`. PORT: no path through an intermediate for a v1 certificate
  (`UnknownIssuer`); a wildcard CN in a v1 certificate is `unported!` (the JDK's public-suffix
  rule). 7 tests with Bazel's certificates (`blaze_remote/tests/data/test_tls_certificate/`).
- `xattr-digest` (2). Ported `UnixFileSystem.getxattr`, `getFastDigest` and `getDigest`'s
  `VFS_MD5` task (`blaze_vfs/src/unix_file_system.rs:82, 140, 153, 205, 216`; the native
  `getxattr_common`: ENODATA is null, ENOTSUP is null, other errors `<path> (<strerror>)`);
  `BazelFileSystemModule.getFileSystem` passes the startup option (`blaze_runtime/src/runtime.rs:966`).
  `DigestUtils.getDigestWithManualFallback` ported (`blaze_exec/src/actions/cache/digest_utils.rs:23`,
  no digest cache) and used where Java calls it: `FileArtifactValue.createFromStat` and
  `createForNormalFileUsingPath` (`file_artifact_value.rs:337, 436, 525`) and `DigestUtil.compute`
  (`blaze_remote/src/util/digest_util.rs:83`). 2 tests (`blaze_vfs/tests/unix_file_system_xattr_test.rs`).
  Remaining difference (the bold cell): the oracle reads file2's attribute twice (once for its
  `FileStateValue`, once in the fallback), Blaze once, because `ArtifactFunction.createSourceValue`
  stats the file instead of using its `FileValue` (an existing PORT in `artifact_function.rs`).
  What the upstream test checks (no `VFS md5` for file1, one for file2) is equal.
- Compact execution log not published (1). `SpawnLogModule.buildComplete` returned the file but
  `build.rs` dropped it, after the BEP had already posted `BuildToolLogs`. Now the file joins the
  collection after `BuildSummaryStatsModule`'s entries, the oracle's order
  (`bep.rs:1832, 1886`, `build.rs:932`).

## 3. re2 remote matrix

New run driver `scripts/remote/modes` (help prints the scenarios). One output user root per side,
`clean --expunge` before every step, Bazel's worker started empty per scenario, `--jobs=4`,
`--remote_grpc_log`. The project's command (`test //:all //python:all`); remote execution uses
`build` (as `scripts/remote/matrix`). Compared per step: exit code, `N processes:` line,
`scripts/remote/compare-grpc-log` (oracle the reference), and `bazel-bin.sha256`
(`second-build rehash`). Runs: oracle `r3-m-oracle`, e8b79c0 `r3-m-e8b` (`all` from runs 11/12,
see the incident), blaze-4 `r3-m-f`; full comparisons in `matrix-compare-e8b.txt` and
`matrix-compare-f.txt`.

| step | summary line (oracle = e8b79c0 = blaze-4) | gRPC calls by method, oracle = Blaze (both binaries) | unmatched RPCs | bazel-bin |
|---|---|---|---|---|
| remote cache, cold | `716 processes: 165 internal, 551 linux-sandbox` | GetActionResult 550, UpdateActionResult 550, Write 1884, GetCapabilities 1, FindMissingBlobs 550 | 81 each side, all TestRunner | same, 4917 |
| remote cache, warm | `716 processes: 550 remote cache hit, 165 internal, 1 linux-sandbox` | GetActionResult 550, Read 49, GetCapabilities 1 | 59, all TestRunner | same, 3865 |
| minimal, cold | as remote cache cold | as remote cache cold | 82, all TestRunner | same, 4917 |
| minimal, warm | as remote cache warm | GetActionResult 550, Read 23, GetCapabilities 1 | 39, all TestRunner | same, 3860 |
| all, cold | as remote cache cold | as remote cache cold | 82, all TestRunner | same, 4917 |
| all, warm | as remote cache warm | GetActionResult 550, Read 346, GetCapabilities 1 | 60, all TestRunner | same, 4388 |
| disk + remote cache, cold | as remote cache cold | as remote cache cold | 82, all TestRunner | same, 4917 |
| disk + remote cache, warm | `716 processes: 550 disk cache hit, 165 internal, 1 linux-sandbox` | GetCapabilities 1 | none, MATCH | same, 3865 |
| remote execution, cold | `695 processes: 165 internal, 1 linux-sandbox, 529 remote` | GetActionResult 529, Read 93, Write 4414, GetCapabilities 1, FindMissingBlobs 9636, Execute 529 | none, MATCH (attribution-insensitive, 15202 of 15202 items) | same, 3951 |
| remote execution, warm | `695 processes: 529 remote cache hit, 165 internal, 1 linux-sandbox` | GetActionResult 529, Read 93, GetCapabilities 1 | none, MATCH (623 of 623) | same, 3951 |

Exit code 0 on all 30 steps. Remote cache only is the toplevel mode (Bazel's default). The
unmatched RPCs, classified by their metadata, belong to the 21 TestRunner actions and to nothing
else: test results carry timings, so their action results and blobs differ between any two runs
(the matrix's existing allowance `re2-cold-test-actions`); each warm step reads its own side's
cache. Per-method counts are equal on all 39 rows for both binaries.

## 4. Probes, oracle against both binaries

Run under `BLAZE_REMOTE_WORK=~/.cache/blaze/work/remote-r3` (logs `battery-{o,e,f}.log`, compares
`compare-{e,f}.txt`). Equal to the oracle on both binaries unless listed:

- `probe-trees` (3 steps, bazel-bin listings identical), `probe-inprocess` (`10 processes: 3 disk
  cache hit, 7 internal`, same `final.txt`).
- `evict-probe`: 4 sources × `--norewind_lost_inputs` (lost-input line, retry line, `3 processes`,
  exit 0) and × retries=0 (exit 1, same text); `exit39` exit 39; `disk --norewind_lost_inputs`.
  Different, as ruled: the 4 default-flags rows (oracle rewinds, exit 0; Blaze the loud
  `ActionRewindStrategy.prepareRewindPlanForLostInputs` failure, exit 37) and `disk` with default
  flags (oracle `1 disk cache hit, 1 internal, 1 linux-sandbox`, Blaze rebuilds, both exit 0).
  New: in `exit39` the oracle prints a final `FAILED: ` progress line that Blaze does not
  (exit code and every other line equal; UI, not analysed).
- `second-build` brotli toplevel and minimal (6 steps): lines and bazel-bin hashes equal.
- `caps-matrix` (20 scenarios): exit codes 20 of 20 equal; texts equal except the listed ones
  (`unreach-*`: the oracle named `[0:0:0:0:0:0:0:1]`, Blaze `127.0.0.1`; `tls-http-cache`: Netty's
  `not an SSL/TLS record` against rustls' `received corrupt message`).
- The seven not run on the final binary in round 3: `platform-probe --execlog` 30 of 30 SAME;
  `regex-probe` 8 of 8 same; `nonascii-probe` ALL SAME; `strings-probe` json, binary, compact and
  dc equal (on e8b79c0 `dump --action_cache` fails loudly, `DumpCommand.exec` not ported; on
  blaze-4, built later, it is equal); `stale-probe` (query, build, clean and the 12 remote
  variants equal; `info` exits 37 on Blaze: `CommittedHeapSizeInfoItem.get: no JVM`, not remote);
  `bep-interrupt-probe` equal (uris, schemes, Write statuses); `discovery-probe` local step fails
  on both with the same aggregate-ddi failure (the oracle reports the action of `//:main`, Blaze
  the one of `//:base`; the oracle cannot build C++20 modules here, so no remote step).
- `sigpipe-probe` (P1): `mtls-noclient` 10 of 10 exit 34 on all three; `http-cache` 10 of 10 exit 34;
  `reset-peer` 10 of 10 exit 0, one reset each; `build 2>&1 | head -1` 141, `build | head -1` 0,
  `query //... | head -1` 141 (3 runs each), the same on all three.

## 5. The six `remote_execution_service_test` failures

The six (`build_remote_action_code_point_sorting`, `build_remote_action_golden_test_seed_{1,2,3}`,
`download_outputs_output_directories_with_unicode_filenames_works`,
`download_outputs_non_ascii_names_are_unicode_on_both_sides_of_the_seam`) failed in the gate
recorded at `051c8c7` (dirty tree, `~/.cache/blaze/work/bytes-lead/tests-3.json`), right after the
byte-strings switch: their fixtures used Unicode literals where the Java test calls
`StringEncoding.unicodeToInternal` (file and environment names of the sorting and golden tests,
the expected paths and symlink targets of the download tests), so the expectation was in the wrong
string form. `2574fef` wraps them as Java does; no product change was needed. They pass now:
`scripts/cargo-capped test -p blaze_remote --no-fail-fast` 648 passed, 0 failed, 62 ignored;
`-p blaze_exec --no-fail-fast` 857 / 0 / 157 (shared working tree on HEAD `3a006d7`, whole
crates).

## 6. Round-3 open items that are not blocking, at e8b79c0

| item | state | where |
|---|---|---|
| P2 input discovery without the action file system | fixed (unit tests only: the oracle cannot build C++20 modules here) | `blaze_sky_exec/src/skyframe_action_executor.rs:1754`; tests `action_execution_function.rs:2627, 2659` |
| P3 TextFormat `from_utf8_lossy` | fixed | `blaze_protos/src/text_format.rs:701-703, 1727-1730` (`java::new_string_utf8`); the one left at :1677 decodes eight already-checked hex digits |
| P4 wrappers record systemd-run's status | fixed | `scripts/remote/unit-supervisor`, `unitstatus.py`, `ws-run:109-142` |
| P5 post-handshake TLS alert | fixed | `blaze_remote/src/grpc/channel.rs:981, 1005, 1022, 1043`, `ConnectionLifecycle` :1082 |
| P6 regex dialect | fixed | `blaze_util/src/java_regex.rs:176`; `blaze_options/src/converters.rs:573-602`; `blaze_remote/src/scrubber.rs:9-59` |
| P7a interrupt of an in-process download | fixed | `blaze_exec/src/exec/remote_action_file_system.rs:401, 1037-1045`; test `remote_action_file_system_test.rs:3347` |
| P7b purge on a second cache handle | fixed | `blaze_runtime/src/commands/build_remote.rs:130-160`; `skyframe_action_executor.rs:354, 474-476`; test `action_execution_function.rs:2334` |
| P8 tests | fixed | `blaze_sky_exec/src/action_output_metadata_store_tests.rs:629, 703` (30 of 30 methods, 4 ignored); `action_output_directory_helper_test.rs` no ignore |
| S3 rewinding, `RemoteLeaseExtension`, `RemoteImportantOutputHandler`, `DynamicConnectionPool` | unchanged, loud (by ruling) | `docs/remote.md` |
| S4 U+FFFD property names keep a HashMap order | fixed in `blaze_remote`; **not fixed** in `blaze_exec`'s private copy | `blaze_remote/src/platform_utils.rs:113-140`; `blaze_exec/src/exec/spawn_log_context.rs:178` still a `HashMap` |
| S4 `to_path_from_hash` by bytes | fixed | `blaze_remote/src/disk/disk_cache_client.rs:407-416` |
| S4 `BulkTransferException` sort | fixed | `blaze_remote/src/common/bulk_transfer_exception.rs:156-157` |
| S4 `inline_output_files` HashSet | fixed (`IndexSet`) | `blaze_remote/src/common/remote_cache_client.rs:125` |
| S4 `create_hard_link` | fixed (Java's call as written) | `blaze_exec/src/exec/remote_action_file_system.rs:1103-1115` |
| S4 BEP close comment | **not fixed at e8b79c0**: `blaze_runtime/src/commands/bep.rs:1858-1859` still says the file transports close synchronously, while `get_close_futures_map` waits for each close future; the working tree's `bep.rs` has since been rewritten by its owner (:2044) | |
| S4 stale statements | fixed | `docs/remote.md:21`; `blaze_bep/src/events/completion_context.rs:64-71` now states the identity resolver as not ported |
| S4 `stat_from_metadata` mtime | fixed (-1 for constant metadata) | `blaze_exec/src/exec/remote_action_file_system.rs:134-140` |

Still open from the round-3 response's list: 1 (`CompletionContext` identity resolver), 3
(negative lookahead loud), 4 (`write_to_std_out_err` in the client daemon), 5 (special-file
output), 6 (the `spawn_log_context.rs` copy above), 7 (unchanged S3).

## 7. Tests and checks (shared working tree on HEAD `3a006d7`, 11:20 to 11:45)

- `scripts/cargo-capped test --no-fail-fast`, whole crates: blaze_remote 648 / 0 / 62,
  blaze_exec 857 / 0 / 157, blaze_sky_exec 115 / 0 / 27, blaze_vfs 93 / 0 / 8, blaze_buildtool
  139 / 0 / 48, blaze_runtime 182 / 0 / 89 (passed / failed / ignored).
- `scripts/cargo-capped check --workspace --all-targets`: passes.
- clippy `-D warnings`: clean for blaze_remote and blaze_vfs; blaze_sky_exec fails only on
  `action_execution_value.rs:605-606` (`default_constructed_unit_structs`, committed code that is
  not mine).

## 8. Not ported: next work (not started)

From the suite run, in functions of the remote targets: remote repository contents cache (38),
HTTP cache (13), remote flags on other commands (`RemoteModule.beforeCommand` for run, coverage,
the downloader: 18 across the suite), credential helper (10 across the suite), rewinding (2),
`Scrubber.parse` (1), disk cache GC, remote execution of repository rules (4).

## Tools

- New: `scripts/remote/modes` (run driver for the matrix above), `scripts/remote/fixes-probe`.
- Uncommitted (mine): the path overrides in `scripts/remote/caps-matrix` and
  `scripts/remote/sigpipe-probe`.
- `oracle-run` gained `--expunge`; `oracle-run`, `ws-run`, `worker`, `second-build`, `caps-matrix`,
  `sigpipe-probe` and the probes take `$BLAZE_REMOTE_WORK` / `$BLAZE_REMOTE_UR` (defaults
  unchanged); `compare-grpc-log` takes `$BLAZE_GRPC_LOG_DUMP`.
- Disk: my output roots are removed; `~/.cache/blaze/work/remote-r3/` keeps 4.8 GB of run results,
  workspace copies and worker caches.

# Round 4

Response to the round-4 review (STOP at `5f68157` on Q1). Code: the shared working tree, now in
root's checkpoint `f71fc675` (HEAD `18b4d4f6`, unchanged since for these files). Binary for the
end-to-end runs: `~/.cache/blaze/work/remote-4/blaze-1` (sha256 `4b67c340e0c968b8…`, release build
of the shared tree at 12:50 on 2026-11-26, which holds every change listed here).

## The referee for pre-v3 certificates: the oracle's own JDK

The JDK 25 sources are not on this machine (the review read JDK 11's `src.zip`). Instead of
reasoning from JDK 11, every check below was put to the JDK that Bazel 9.3.2 runs on: the
oracle's embedded Zulu 25.0.2 (`<install base>/embedded_tools/jdk`).

- `scripts/jdk-certs/generate.py` (new; `help`) builds 46 end-entity certificates by hand (DER
  written field by field, so that malformed structures can be made), signed by a test CA or by
  themselves, each with a host name to check.
- `scripts/jdk-certs/JdkCertProbe.java` runs, per certificate, what
  `X509TrustManagerImpl.checkTrusted` runs for a TLS client: `CertificateFactory("X.509")`
  (X509Factory, X509CertImpl, X509CertInfo), the PKIX trust manager over the case's roots
  (`checkServerTrusted`: PKIXValidator, EndEntityChecker), then
  `HostnameChecker.getInstance(TYPE_TLS).match(host, cert, AnchorCertificates.contains(anchor))`.
  `scripts/jdk-certs/run` records the verdicts in
  `crates/blaze_remote/tests/data/jdk_cert_fixtures/expected.tsv` (36 ACCEPT/REJECT lines with
  the JDK's exception text).
- `crates/blaze_remote/tests/jdk_server_cert_fixtures_test.rs` `verdicts_match_the_jdk` fails
  if Blaze accepts any case the JDK rejects, or rejects a case the JDK accepts that is not in
  its list of documented stricter cases. Result: 46 of 46 consistent (36 identical verdicts,
  10 stricter, listed below).

This also found a defect the review did not name: rustls' `ServerName` drops a trailing dot, so
the verifier saw `localhost` for `localhost.`, which the JDK refuses ("Illegal given domain
name"). The channel now hands the verifier the host as written
(`JdkServerCertVerifier::with_requested_host`, `google_auth_utils.rs` `rustls_client_config`).

## Q1 (S1). FIXED

- (a) `parse_pre_v3_certificate` (`jdk_server_cert_verifier.rs:499`) parses the whole
  `tbsCertificate`: nothing may follow the key of a v1 certificate (:562); a v2 certificate may
  have `[1]` then `[2]` and nothing else (:570-582). Cases `v1_extensions`,
  `v1_explicit_extensions` ("no more data allowed for version 1 certificate"), `v2_extensions`,
  `v2_unique_ids_reversed`, `v2_trailing_integer` ("Extensions not allowed in v2 certificate"),
  `v1_unique_id`: the JDK rejects, Blaze rejects; `v2_unique_ids`: both accept.
- (b) the inner and outer AlgorithmIdentifiers are compared (:587). `alg_mismatch` (inner
  sha384, outer and signature sha256) and `alg_mismatch_outer` (the reverse): "Signature
  algorithm mismatch" on the JDK, rejected by Blaze. The comparison is by encoding: the JDK
  takes absent parameters as equal to NULL (`alg_params_absent_inner`, JDK accepts), Blaze
  rejects (stricter).
- A pre-v3 certificate that fails to parse is now a `BadEncoding` failure; it no longer falls
  through to another path (`route`, :206, :230, :251, :271).

## Q2 (S2). FIXED

`parse_time` (:641) works on bytes: the last byte must be `Z` and every other byte an ASCII
digit before any field is read; seconds above 59 and days not in their month are rejected.
The full parser now runs only for a certificate whose version field is absent, 0 or 1
(`certificate_version`, :391, reads no further than the version); v3 certificates go to webpki
without being parsed here. No parser in the file indexes or slices without a bounds check, and
none unwraps certificate data. Tests: `malformed_times_are_rejected_without_panic` (the review's
12-byte UTCTime with `é` across the year, and 19 other malformed times);
`malformed_inputs_never_panic` (every truncation, three bit flips at every byte and an extension
of both Bazel test certificates through every parser and the three verifier entry points, plus
hand-made DER headers, integers, OIDs and BMPStrings); fixture `time_non_ascii` (JDK: "Parse UTC
time, invalid format"; Blaze: rejected).

## Q3 (S2) and the S4. FIXED

Direct trust compares the anchor's subject and the *contents* of its subjectPublicKeyInfo with
the certificate's (:99), and, as `PKIXValidator.engineValidate` returns `chain[0]` unchecked,
a directly trusted certificate gets no signature or validity check. Fixtures `direct` (issued by
an unknown issuer, itself the anchor), `direct_expired`, `self_signed`, `self_signed_expired`:
the JDK accepts all four, Blaze accepts all four; `self_signed_untrusted`: both reject.

## Q4 (S2). FIXED

`most_specific_common_name` (:701) takes the first commonName of the last RDN that has one.
Fixtures `two_cn_first` (one RDN, CN=localhost then CN=other: both accept `localhost`),
`two_cn_second` (CN=other then CN=localhost: both reject), `cn_last_rdn`, `cn_first_rdn`.
BMPString values are now read (`cn_bmp`, both accept).

## Every JDK check of a pre-v3 server certificate, and the Rust that does it

Java lines are JDK 11 (`sun.security`); "case" is the fixture that shows the JDK 25.0.2 verdict.
Rust lines are `crates/blaze_remote/src/authandtls/jdk_server_cert_verifier.rs` at `18b4d4f6`.

| Step | JDK | Case | Rust |
|---|---|---|---|
| DER lengths: definite, ≤ 4 length bytes, no leading zero, long form only above 127 | `DerInputStream.getLength` | `non_minimal_length` (JDK 25 accepts: stricter) | `read_tlv` :314 |
| Certificate: SEQUENCE of exactly tbs, AlgorithmIdentifier, BIT STRING | `X509CertImpl.java:1810-1836` | (all) | :499-514 |
| Bytes after the certificate | X509Factory reads one block | `trailing_bytes` (JDK accepts: stricter) | :505 |
| AlgorithmIdentifier: SEQUENCE { OID, one optional parameter }, no extra data; OID minimal | `AlgorithmId.java:397-421`, `ObjectIdentifier.check` | — | `algorithm_identifier` :451, `valid_oid` :434 |
| Signature BIT STRING | `DerValue.getBitString` | — | `bit_string_bits` :624 (no unused bits: stricter) |
| Version `[0] EXPLICIT INTEGER`, nothing after the INTEGER | `X509CertInfo.java:644`, `CertificateVersion.java:80` | `v1_explicit`, `v2` | :517-529 |
| Version above 2 | parsed as v2 | `version_5` (JDK accepts: stricter, webpki rejects) | `route` :206 |
| Serial: INTEGER, non-empty | `X509CertInfo.java:650`, `DerInputBuffer.java:150` | `serial_negative` (both accept), `serial_leading_zero` (JDK 25 accepts: stricter) | :534 |
| Inner AlgorithmIdentifier | `X509CertInfo.java:653` | — | :538 |
| Issuer Name: SEQUENCE OF SET OF SEQUENCE { OID, value }, no extra bytes | `X500Name.parseDER`, `RDN`, `AVA` | — | `name` :471 (empty RDN rejected: stricter) |
| Empty issuer | `X509CertInfo.java:656-660` | `empty_issuer` | :542 |
| Validity: SEQUENCE of exactly two UTCTime/GeneralizedTime | `CertificateValidity.java:77-101` | — | :546-552 |
| Time: digits, month 1-12, day 1-31, hour < 24, minute and second < 60 | `DerInputBuffer.getTime` | `time_non_ascii`, `time_second_60` | `parse_time` :641; `time_no_seconds`, `time_offset`, `time_fraction`, `time_feb_31` (JDK accepts: stricter) |
| Subject Name; empty subject in v1 | `X509CertInfo.java:666-671` | `empty_subject` (JDK rejects), `empty_subject_v2` (JDK rejects at the host step) | :553-556 |
| subjectPublicKeyInfo: SEQUENCE { AlgorithmIdentifier, BIT STRING }, no extra data | `X509Key.parse` ("excess subject key") | — | :558-559, `parse_spki_contents` :610 |
| v1: nothing after the key | `X509CertInfo.java:677-681` | `v1_extensions`, `v1_explicit_extensions`, `v1_unique_id` | :562 |
| v2: `[1]`, `[2]` in order, nothing else | `X509CertInfo.java:686-707` | `v2_unique_ids`, `v2_unique_ids_reversed`, `v2_extensions`, `v2_trailing_integer` | :570-582 |
| Inner and outer signature algorithm equal | `X509CertImpl.java:1842-1847` | `alg_mismatch`, `alg_mismatch_outer`, `alg_params_absent_inner` (stricter) | :587 |
| Directly trusted (same subject and key as a trust anchor): no further check | `PKIXValidator.java:256` | `direct`, `direct_expired`, `self_signed`, `self_signed_expired` | :99 |
| Chain of more than one certificate | ordering loop, partial validation, builder (`PKIXValidator.java:243-306`) | — | :108 (rejected: stricter) |
| Issued by an anchor (subject = issuer, anchor key verifies the signature) | `PKIXValidator.java:269-275`, BasicChecker | `v1`, `bad_signature`, `self_signed_untrusted` | :122 (`verify_with` :283; names by encoding: stricter) |
| Signature algorithm and key size allowed | AlgorithmChecker (`jdk.certpath.disabledAlgorithms`) | — | ring's set (SHA-256 and up, RSA ≥ 2048, P-256/P-384, Ed25519) is inside the JDK's |
| Validity at the current time | `CertificateValidity.java:271-275` | `expired` | :146, :149 (the notAfter second itself is expired: stricter by < 1 s) |
| KeyChecker, ConstraintsChecker, PolicyChecker, EndEntityChecker, revocation | nothing to check without extensions; revocation off in PKIXValidator | — | — |
| Host name a valid SNI host name (`new SNIHostName(name)`: LDH labels of 1-63, no leading or trailing hyphen, no trailing dot) | `HostnameChecker.java:195` | `underscore`, `trailing_dot_host` | `match_dns` :772, `is_sni_host_name` :756, requested host :157 |
| IP address host: needs an iPAddress subjectAltName | `HostnameChecker.java:100, 143` | `ip_host` | :162, :784 (all-digit names: stricter) |
| Most specific CN: first CN of the last RDN that has one, a string type | `X500Name.java:756-766`, `RDN.java:324-331` | `two_cn_first`, `two_cn_second`, `cn_last_rdn`, `cn_first_rdn`, `cn_bmp` | `most_specific_common_name` :701 |
| CN NFKC-normalized, a valid SNI host name, matched label by label ignoring case | `HostnameChecker.java:221-236, 271-310` | `v1_upper_host`, `wrong_host` | `is_matched` :808 (non-ASCII names: stricter) |
| Wildcard CN (`hasIllegalWildcard`, public suffix list for public CAs) | `HostnameChecker.java:290` | `wildcard` (JDK accepts) | :808 (not matched: stricter; no longer a crash) |
| Handshake signature with the certificate's key | done by the TLS engine | — | `verify_pre_v3_signature` :167, :264 |

The ten stricter cases are the `STRICTER` list of the fixture test. Each fails closed (a
handshake failure), none can accept what the JDK refuses.

## The two older S4 items

- `blaze_exec/src/exec/spawn_log_context.rs` `get_platform_proto`: the properties map is now
  Java's `HashMap` order. The `JavaHashMap` port moved from `blaze_remote/src/platform_utils.rs`
  to `blaze_util/src/java_hash_map.rs` (a crate both use); both ports of `getPlatformProto` build
  the map in the same steps.
- `blaze_runtime/src/commands/bep.rs:2053, 2091` (the comment on synchronous file transport
  close): not mine; reported to root for its owner.

## Tests (shared working tree, HEAD `18b4d4f6`, 13:00-13:40)

- `scripts/cargo-capped test -p blaze_remote --no-fail-fast`: 653 passed, 1 failed, 61 ignored.
  The failure is `remote_module_test::disk_cache_gc_policy_fails_loudly` (a `should_panic` on
  `DiskCacheGarbageCollectorIdleTask.create`): another agent has ported that idle task in the
  shared tree, so the test is stale; not changed by me.
- `blaze_exec` 1003 / 0 / 126; `blaze_util` 67 / 0 / 0; `blaze_optdefs` 424 / 0 / 25;
  `blaze_repo_fetch` 476 / 1 / 14 (the failure is `oracle_extraction_test`, which compares with
  a stored oracle extraction of a different `platforms` release; unrelated).
- `scripts/cargo-capped check --workspace --all-targets`: passes. clippy: clean for the changed
  files.

# Round 5

Response to the round-5 review (STOP at `e476a59`: T1-T5) under root's ruling D16 (Blaze does
not validate chains of X.509 v1/v2 certificates; the JDK's direct trust is the one extra rule).
Code: the shared working tree on HEAD `7ef5c861` (uncommitted). Binary for the end-to-end runs:
`~/.cache/blaze/work/remote-5/blaze-2` (sha256 `62c9c1e599498e29…`, release build of that tree;
the verifier's error-text change below came after it, see "Text").

## The verifier after D16

`crates/blaze_remote/src/authandtls/jdk_server_cert_verifier.rs` (868 lines, of which the tests
are the last 260) was rewritten. What is left beside rustls' webpki verifier:

| Step | Where |
|---|---|
| Direct trust: the end entity's whole DER equals a certificate of the `--tls_certificate` file (`trustedCerts.contains(cert)`, `PKIXValidator.java:234` at `jdk-25.0.2-ga`); no chain, no validity check, any version. The built-in roots give no direct trust. | `is_directly_trusted` :152, called first in `verify_server_cert` :220 |
| Host name after direct trust: v3 through rustls' `verify_server_name` (webpki's subjectAltName check); v1/v2 through `match_dns` :598 (SNI host name rules, IP literals refused, first CN of the last RDN that has one :527, wildcard refused); the host as the channel wrote it (trailing dot) | `check_host_name` :160 |
| Any other v1/v2 certificate: refused with Blaze's text `PRE_V3_CHAIN_NOT_SUPPORTED` :47 | :237 |
| Everything else: webpki (`None` when the trust file has no certificate webpki takes as an anchor: then only direct trust can succeed) | :127, :237-250 |
| Handshake signature of a directly trusted v1/v2 certificate: its key, read by `pre_v3_parts` :350 / `read_pre_v3_fields` :378 (well-formed DER up to the key or nothing) | `verify_pre_v3_signature` :190, `verify_tls13_signature` :273 |
| Signature algorithms (T4) | `oracle_signature_verification_algorithms` :82 |

Removed: the v1/v2 chain validation (signature by an issuer, validity, issuer matching), unique
identifiers, the OID and time parsers, AlgorithmIdentifier equality. T2 and T3 disappear with
them. The CN reader and the structure walk are kept; neither panics
(`malformed_inputs_never_panic`: every truncation, three bit flips per byte and an extension of
both Bazel test certificates through every reader and verifier entry point).

## T1 (S1). FIXED

Direct trust compares the whole certificate. Unit test `direct_trust_compares_the_whole_certificate`
(one signature bit flipped); fixture `direct_twin` (the reviewer's: same subject and key, other
bytes): JDK REJECT, Blaze refuses. End to end with the reviewer's listener and certificate
(`~/.cache/blaze/work/remote-5/probes/tls-certs/direct-twin.server.log`): `blaze-2` aborts the
handshake (the reviewer's log shows the oracle aborting it too).

## T2, T3. Gone with the code (D16)

`v2_uid_pad_nodata` and `subject_big_oid` are v1/v2 certificates issued by a CA: refused as
chains. They are not kept as fixtures: the decision point they tested no longer exists.

## T4. FIXED (provider subset)

Source of the oracle's list: `io/netty/handler/ssl/ExtendedOpenSslSession.class` in the oracle's
`A-server.jar` (netty-handler 4.1.119.Final, from its `pom.properties`), static initializer of
`LOCAL_SUPPORTED_SIGNATURE_ALGORITHMS` (read with `javap -c`): SHA512withRSA, SHA512withECDSA,
SHA384withRSA, SHA384withECDSA, SHA256withRSA, SHA256withECDSA, SHA224withRSA,
SHA224withECDSA, SHA1withRSA, SHA1withECDSA, RSASSA-PSS. ring's set (RSA PKCS#1 and RSA-PSS with
SHA-256/384/512, ECDSA P-256/P-384 with SHA-256/384, Ed25519) minus Ed25519 lies inside it. The
verifier builds its webpki verifier with that subset and advertises only its schemes
(`supported_verify_schemes`), so Ed25519 is refused for certificate signatures and handshake
signatures alike (`ed25519_is_not_offered`). Stricter than the oracle: SHA-1 and SHA-224
signatures (not in ring). End to end, a v3 chain from an Ed25519 root (fixture `v3_ed25519_ca`):
the oracle and `blaze-2` both abort the handshake (listener log `v3-ed25519-ca.server.log`: 3
failed handshakes, no success). The reviewer's `sig_ed25519` is a v1 chain and now refused by D16
first.

## T5. FIXED

`v3_self_ca_san` (self-signed, `CA:TRUE`, SAN, used as the trust file): fixture accepted by both;
end to end, `blaze-2` completes the handshake (`v3-self-ca-san.server.log`). `v3_self_expired`
and `v3_self_noca_san` are accepted too. Stricter and declared: `v3_self_ca_cn_only` and
`v3_self_noca_cn_only` (no SAN: webpki's name check has no CN fallback; the JDK matches the CN).

## T6. FIXED

`crates/blaze_remote/tests/jdk_server_cert_fixtures_test.rs` now runs, per fixture, a full TLS
handshake (TLS 1.3, then TLS 1.2, which must agree) between a rustls client with the verifier and
a rustls server presenting the fixture and signing with `keys/ee.key`, over an in-memory duplex;
so the end entity's key decides too. 44 fixtures (`scripts/jdk-certs/generate.py`, verdicts from
the oracle's JDK 25.0.2 by `scripts/jdk-certs/run`): direct trust identical (v1, v2, explicit
version, expired, upper-case host), host checks (wrong host, IP, trailing dot, underscore,
wildcard, the CN cases), the reviewer's cases that still have a decision point (`direct_twin`,
`cn_utf8`, `cn_t61`, `cn_ia5`, `cn_first_ava_other_type`, `rdn_unsorted`, the four `subject_*`,
`ee_key_alg_bad_null`, `ee_key_unknown_alg`, `ee_key_garbage_rsa`, the five `v3_self_*`), a v1
chain (`chain_v1`), webpki's route (`v3_issued_san`, `v3_issued_wrong_host`, `v3_p256_ca`,
`v3_ed25519_ca`). Result: 44 of 44 consistent, 6 declared stricter (`direct_wildcard`,
`ee_key_unknown_alg`, `chain_v1`, the two CN-only v3, `v3_ed25519_ca`). Not taken over from the
reviewer's 30: the v1/v2 chain cases (`v2_uid_*`, `subject_big_oid`, validity and time cases,
`sig_ecdsa_p256`, `ca_rsa_1024`, `sig_ed25519`, `ee_rsa_512`, `ee_rsa_1024`): their decision
point was the removed chain validation. `JdkCertProbe` itself is unchanged (no TLS engine): its
verdict on `v3_ed25519_ca` is ACCEPT, which is why that case is listed as stricter, with the end
to end run above showing the oracle refusing it.

## Declared gap

Bazel's TLS tests present `src/test/testdata/test_tls_certificate/server.crt` (v1, issued by
`ca.crt`) with `--tls_certificate=ca.crt`: a v1 chain. These now fail on Blaze with Blaze's text
(as they already did before round 3): `remote_execution_tls_test` `test_remote_grpcs_cache` and
`test_remote_grpc_cache`; `remote_execution_mtls_test` `test_remote_grpcs_cache` and
`test_remote_grpc_cache`. The other two functions of each target expect a failure and pass.
End to end with the same chain (fixture `chain_v1`, listener `cv1.log`): the oracle completes the
handshake; Blaze prints `Failed to query remote execution capabilities: ... X.509 v1/v2 server
certificate chains are not supported by Blaze: a v1/v2 server certificate is trusted only if it
is identical to a certificate in --tls_certificate` and exits 34.

Text: `blaze-2` wrapped that text in rustls' debug form of `CertificateError::Other`
(`invalid peer certificate: Other(OtherError(Custom { ... }))`); the tree now returns
`Error::Other`, printed `other error: X.509 v1/v2 ...` (unit and fixture tests; not rerun end to
end).

## C1. FIXED

The response reader keeps repeated member names (`blaze_bzlmod_model::gson::
parse_lenient_keeping_repeated_names`, a mode of the shared reader: an object keeps every member,
as the token stream shows them to a type adapter); `GetCredentialsResponse` puts the headers of
every `"headers"` member into one builder and fails as `ImmutableMap.Builder.build()` does
(`IllegalArgumentException`, Guava's text, the later entry first). As in Java the exception is
not a `JsonSyntaxException`: it leaves `CredentialHelper.getCredentials` unchanged
(`IoExceptionKind::IllegalArgumentException`, the tunnel for this one unchecked exception), is not
caught by the header function (which catches `IOException` only), and `HttpConnector.connect`
turns it into `UnrecoverableHttpException(e.getMessage())` (`HttpConnector.java:245`; the header
function now returns `Result`). On the gRPC path it fails the call as any credentials error does.
Compared with the oracle (a helper answering
`{"headers": {"Authorization": ["Bearer TOKEN"], "Authorization": ["Bearer OTHER"]}}`, a module
with an `http_archive` behind Bazel's `testing_server.py`): both exit 32 with
`ERROR: Error computing the main repository mapping: Error accessing registry
https://bcr.bazel.build/: Failed to fetch registry file <url>: Multiple entries with same key:
Authorization=[Bearer OTHER] and Authorization=[Bearer TOKEN]`, preceded by the same
`Error retrieving auth headers, continuing without: ... process was interrupted` warnings; the
registry file named differs (the oracle `protobuf/33.4`, Blaze `zlib/1.3.1.bcr.5`: the parallel
fetches race on both binaries). Test `repeated_header_name_is_illegal_argument`.

## C2. FIXED

`crates/blaze_repo_fetch/tests/command_line_path_factory_test.rs`: the 11 tests of
`CommandLinePathFactoryTest` (Java names in snake case), on the real file system under a
temporary directory. 11 pass.

## S4

The verifier's module comment now states the D16 design and its declared deviations.

## Tests (shared tree on HEAD `7ef5c861`)

- `scripts/cargo-capped test -p blaze_remote --no-fail-fast`: 652 passed, 0 failed, 60 ignored
  (`disk_cache_gc_policy_fails_loudly` is replaced by the port of the Java tests
  `diskCacheGarbageCollectionIdleTask_disabled/_enabled`, now that the server agent ported the
  idle task).
- `-p blaze_repo_fetch`: 488 passed, 1 failed, 14 ignored (the failure is
  `oracle_extraction_test`, which compares with a stored extraction of a different `platforms`
  release; not mine).
- clippy: clean for the changed files. `check --workspace --all-targets`: one error, in another
  agent's `blaze_exec/tests/analysis_actions_execute_test.rs:963`.
