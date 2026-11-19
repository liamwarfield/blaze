# Response: remote wiring review (STOP on 3353c77)

- Responding to `docs/reviews/2026-11-19-remote-wiring.md`. Fixes are on top of `3353c77`
  (uncommitted when written; the lead commits them).
- Oracle: `~/.cache/blaze/oracle/bazel-9.3.2-linux-x86_64`; Blaze: `target/debug/blaze` of the
  fixed tree. Both `--batch --nosystem_rc --nohome_rc`, own `--output_base`, workspace
  `~/.cache/blaze/bench/remote-ws`, server Bazel's remote worker built from the pin
  (`scripts/remote/worker start p1 18981`). Every predicted Bazel behaviour below was run on the
  oracle first.

## B1 (S1) BEP `bytestream://` URIs: made loud (uploader not ported)

- Oracle confirms the finding: `build //... --remote_cache=grpc://localhost:18981
  --build_event_json_file=...` names every file `bytestream://localhost:18981/blobs/<hash>/<size>`;
  with `--experimental_build_event_upload_strategy=local` the URIs are `file://`.
- Fix: `ByteStreamBuildEventArtifactUploader` is not ported (about 550 lines of Java plus the BEP
  uploader seam). `RemoteModule` records whether `setup` reached the gRPC path, where Java calls
  `buildEventArtifactUploaderFactoryDelegate.init` (`remote_module.rs`,
  `uses_byte_stream_build_event_artifact_uploader`). `blaze_runtime::commands::build::run_build`
  fails with `unported!("ByteStreamBuildEventArtifactUploader
  (RemoteModule.BuildEventArtifactUploaderFactoryDelegate.create)", ...)` when that is true, a
  `--build_event_{text,binary,json}_file` is set and `--experimental_build_event_upload_strategy`
  is unset or `remote` (Java's `BuildEventArtifactUploaderFactoryMap.select`). Disk cache only and
  `--experimental_build_event_upload_strategy=local` keep working (Java's delegate returns a
  `LocalFilesArtifactUploader` there). Listed under "Fails loudly" in `docs/remote.md`.

## B2 (S1) remote output metadata in the action cache: fixed (phase 2 slice 1)

- `execution_tool.rs`: with a cache (Java: `RemoteOutputService`) the action cache checker is
  built with `setStoreOutputMetadata(true)`
  (`RemoteOutputService.shouldStoreRemoteOutputMetadataInActionCache`), and the executor checks
  cache entries with the module's `RemoteOutputChecker` (`RemoteOutputService.getOutputChecker`,
  `SkyframeActionExecutor.checkActionCache`; new `SkyframeActionExecutor::with_output_checker`,
  `TRUST_LOCAL_ONLY` otherwise). The checker's `shouldTrustMetadata` (in-memory outputs trusted
  by TTL, `RemoteOutputChecker.java:340`) and the action cache's remote metadata encoding
  (`inMemoryOutput` included) were already ported.
- Oracle row added: after a cache build, a rebuild without `clean` prints
  `INFO: 1 process: 20 action cache hit, 1 internal.` with no execution log entry and a gRPC log
  of one `GetCapabilities`, for `--remote_cache`, `--remote_executor` and `--disk_cache`; Blaze
  prints the same, the gRPC logs MATCH (1/1) and both execution logs are empty.

## B3 (S2) afterCommand: fixed

- `RemoteModule::after_command` now follows `RemoteModule.afterCommand` + `afterCommandTask`
  (`RemoteModule.java:1097, 1140`): provider shutdown, temp dir `deleteTree()` with the failure
  ignored, then the gRPC log close, whose failure is `createExitException("Partially wrote RPC log
  file", LOCAL_ENVIRONMENTAL_ERROR, RPC_LOG_FAILURE)`. The comments citing nonexistent methods are
  gone.
- Oracle: `--remote_grpc_log=/dev/full` prints `ERROR: Partially wrote RPC log file` after the
  build summary and exits 36; Blaze prints the same and exits 36.

## B4 (S2) explicit ExitCode: fixed

- `blaze_exec::exec::spawn_strategy_registry::AbruptExitException` gains `exit_code:
  Option<i32>` and `with_exit_code` (`DetailedExitCode.of(exitCode, failureDetail)`);
  `remote_module.rs`'s `create_exit_exception(message, exit_code, code)` takes Java's `ExitCode`
  (`handleInitFailure`: `COMMAND_LINE_ERROR`; stale downloads, log directory, RPC log:
  `LOCAL_ENVIRONMENTAL_ERROR`); `build.rs` turns it into `DetailedExitCode::of_exit_code` when
  set (`abrupt_exit_detailed_exit_code`).
- Oracle: a stale `<action_temps>/remote` that cannot be deleted (its parent made read-only):
  `WARNING: Found stale downloads from previous build, deleting...`, `ERROR: Failed to delete
  stale downloads: ... (Permission denied)`, exit 36; Blaze the same, exit 36. The I/O error text
  differs: Java's has the JNI location (`[unix_jni.cc:660] <path> (Permission denied)`), Blaze's
  is `<path> (Permission denied)` (blaze_vfs's error text, outside this review's scope).

## B5 (S2) lazy channel creation: fixed

- `create_channel` no longer fails: when `google_auth_utils::new_channel` returns an
  `IOException` the channel is `ReferenceCountedChannel::failed(target, e, ..)`, whose every call
  (and `getServerCapabilities`) fails with that exception, as the calls do when
  `GoogleChannelConnectionFactory.create`'s `Single.fromCallable(newChannel)` fails.
- Oracle: `--remote_cache=grpcs://localhost:18981 --tls_certificate=/missing.pem` builds locally
  with `WARNING: Remote Cache: Failed to init TLS infrastructure using '/missing.pem' as root
  certificate: File does not contain valid certificates: /missing.pem`, exit 0; with
  `--remote_executor=grpcs://... --remote_local_fallback`, `11 local`, exit 0. Blaze: identical
  lines, exit 0.

## Non-blocking

- N1: fixed. With a cache, `executor_init` prints Java's warning when
  `--experimental_remote_cache_lease_extension` and `--rewind_lost_inputs` are both set, and
  fails loudly (`RemoteModule.executorInit (RemoteLeaseExtension)`) when only the lease
  extension is set.
- N2: open, listed in `docs/remote.md` gaps.
- N3: fixed. `RemoteActionFileSystem::get_input_stream` of a remote-only file is
  `unported!("RemoteActionFileSystem.getInputStream (download of a remote-only file)")`.
- N4, N5, N6: open, listed in `docs/remote.md` gaps.
- N7: no change (the reason is accurate).
- N8: already listed.
- N9: no change; no effect until `executorInit` does more than the salt and the refusal.
- N10: fixed with the wording of the "Wiring" paragraph (follows B3/B4/B5).
