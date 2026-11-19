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
