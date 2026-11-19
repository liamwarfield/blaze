# Review: remote wiring (RemoteModule in the `blaze` binary)

- Commit reviewed: `3353c77` against its parent `e297ee3`. All `file:line` references are at
  `3353c77`. Java references are relative to
  `~/.cache/blaze/src/bazel-9.3.2/src/main/java/com/google/devtools/build/lib/`.
- Reviewer: Fable 5.2, read-only. Read with `git show` / `git diff`; no Cargo, no oracle run, no
  build. Every finding below is from reading the Rust against the pinned Java; where a finding
  predicts oracle output, the oracle was not run and the fixer should confirm it.
- Scope: the diff of the files listed in the task (buildtool `execution_tool.rs`, runtime
  `build.rs` / `command_environment.rs` / `unported_options.rs`, the `blaze_exec` seams,
  `blaze_sky_exec` remote output tree, and the `blaze_remote` module, hooks, channel gate and
  adapters). The credential helper has its own reviewer and was skipped.
- Not covered: `blaze_remote` has no earlier review in `docs/reviews/`. This review checked only
  what the wiring changed. The download path (`remote_execution_service_download.rs`,
  `combined_cache.rs`, `grpc_cache_client.rs`, `disk/`) was spot-checked for the presence of the
  digest checks, not reviewed line by line as `docs/remote.md` item 7 says it is.

## Verdict

**STOP**, bound to `3353c77`. Blocking: B1, B2, B3, B4, B5 (two S1, three S2).

## Blocking findings

### B1 (S1) Build Event Protocol files carry `file://` URIs where Bazel writes `bytestream://`, silently

- Rust: `crates/blaze_remote/src/remote_module.rs:800` (`// PORT: not ported: ... the ByteStream
  build event artifact uploader`, no `unported!`); `crates/blaze_bep/src/bep_module.rs:291-295`
  (`local_file_uploader(_path_conversion)` always returns `LocalFilesArtifactUploader`);
  `crates/blaze_runtime/src/unported_options.rs:92-119` (refuses only `--bes_backend`).
- Java: `remote/RemoteModule.java:229` registers the only `BuildEventArtifactUploaderFactory`
  ("remote"); `runtime/BuildEventArtifactUploaderFactoryMap.java:33-40` selects the first one
  when `--experimental_build_event_upload_strategy` is unset; `remote/RemoteModule.java:906-916`
  initializes it with a `ByteStreamBuildEventArtifactUploaderFactory` on the gRPC cache path;
  `buildeventservice/BuildEventServiceModule.java:838,868,898` hand that uploader to the text,
  binary and JSON file transports; `remote/ByteStreamBuildEventArtifactUploader.java:481-483`
  ("Always use bytestream:// in MINIMAL mode", the default of
  `--remote_build_event_upload`) and `:520-530` format the URI. In that mode the uploader also
  uploads the stdout/stderr/test-log files it is asked for, which appear in the gRPC log.
- Failing scenario: `blaze build //:x --remote_cache=grpc://host:port
  --remote_download_outputs=all --build_event_json_file=bep.json` (also with
  `--remote_executor`). Bazel's `NamedSetOfFiles` and test log entries have
  `"uri": "bytestream://host:port/blobs/<hash>/<size>"`; Blaze writes `"uri": "file:///..."`.
  Nothing fails, nothing is logged, and `docs/remote.md` does not list it under gaps. The
  evidence table has no scenario with a BEP file and a gRPC cache.
- Needed: port the uploader, or `unported!` when a BEP file transport is combined with a gRPC
  cache, and list it in `docs/remote.md`.

### B2 (S1, disclosed) A rebuild without `clean` re-runs every C++ compile through the remote cache

- Rust: `crates/blaze_buildtool/src/buildtool/execution_tool.rs:491`
  (`set_store_output_metadata(false)`, "LocalOutputService: ... no remote output metadata in the
  action cache") while the same build keeps in-memory outputs out of the file system
  (`crates/blaze_sky_exec/src/remote_output_tree.rs`).
- Java: with a cache the output service is `RemoteOutputService`, the action cache stores
  remote output metadata, and `remote/RemoteOutputChecker.java:340-375` trusts an in-memory
  output by its TTL ("An in-memory output ... is never written to disk, so a 'should download'
  verdict must not force a re-execution").
- Failing scenario: `blaze build //:cc_binary --remote_cache=grpc://... --remote_download_outputs=all`
  twice, where the first build got its compiles from the cache. Bazel's second build has action
  cache hits for the compiles (no spawn, no RPC). Blaze's action cache entry cannot be validated
  (the `.d` is not on disk), so each compile runs again as a `remote cache hit`: a different
  `N processes` line, extra execution log entries, extra gRPC calls, object files rewritten.
- This is disclosed in `docs/remote.md` ("Gaps (phase 1)"), which is honest. It is still wrong
  behaviour on the most common valid input after a cold build, it is silent at run time, and
  `docs/decisions.md` has no ruling accepting it. Needed: the port, or a root ruling that records
  it as an accepted visible difference (as was done for `resource_set` on 2026-11-18) plus an
  oracle row for the no-clean rebuild so the difference is measured, not assumed.

### B3 (S2) `afterCommand` reports errors Java does not have, with other text and exit codes

- Rust: `crates/blaze_remote/src/remote_module.rs:813-848`, made the command's exit code by
  `crates/blaze_runtime/src/commands/build.rs:163-168`. The comments cite
  `RemoteModule.closeRpcLogFile` and `RemoteModule.deleteDownloadedInputs`, which do not exist
  at the pin.
- Java: `remote/RemoteModule.java:1140-1173` (`afterCommandTask`).
  1. Temp dir: Java `tempDir.deleteTree()` with `catch (IOException ignored)`. Rust returns an
     `AbruptExitException` "Failed to delete downloaded inputs in '<dir>': <e>"
     (`DOWNLOADED_INPUTS_DELETION_FAILURE`, exit 34).
  2. gRPC log: Java "Partially wrote RPC log file" with `ExitCode.LOCAL_ENVIRONMENTAL_ERROR`
     (36). Rust "Failed to close the remote gRPC log: <e>", exit 2 (see B4).
  3. Order: Java deletes the temp dir before closing the log; Rust closes the log first.
- Failing scenarios: (a) a successful build whose `<action_temps>/remote` cannot be removed
  (a root-owned or immutable entry left by a sandboxed action): Bazel exits 0, Blaze prints an
  error and exits 34. (b) `--remote_grpc_log` on a full disk: Bazel prints
  `ERROR: Partially wrote RPC log file` and exits 36, Blaze prints other text and exits 2.

### B4 (S2) `AbruptExitException` drops Java's explicit `ExitCode`; the exit code comes from the proto metadata

- Rust: `crates/blaze_remote/src/remote_module.rs:160-172` (`create_exit_exception` takes no
  exit code) and `crates/blaze_exec/src/exec/spawn_strategy_registry.rs:26-34`
  (`AbruptExitException` holds only the `FailureDetail`); `build.rs:150,167` turn it into
  `DetailedExitCode::of(failure_detail)`, which reads the code's `exit_code` metadata.
- Java: `remote/RemoteModule.java:1381-1390` `createExitException(message, exitCode, code)` uses
  `DetailedExitCode.of(exitCode, failureDetail)`; the explicit code wins over the metadata.
- Where the two differ (`src/main/protobuf/failure_details.proto:452-475`):
  - `RemoteModule.java:1225-1232` stale downloads cannot be deleted: Java 36, Rust
    (`remote_module.rs:895-911`) 34.
  - `RemoteModule.java:1167-1170` gRPC log close: Java 36, Rust 2.
  - `handleInitFailure` is always `COMMAND_LINE_ERROR` (2) in Java; it matches today only
    because the three codes passed happen to have metadata 2, except B5.
- Failing scenario: `<execroot>/../action_temps/remote` left over and not deletable at start:
  Bazel exits 36, Blaze exits 34.

### B5 (S2) A channel that cannot be created aborts the command at start; Java fails the calls instead

- Rust: `crates/blaze_remote/src/remote_module.rs:640-662`: the `create` closure maps any
  `create_channel` error to `handle_init_failure(.., RemoteExecutionUnknown)`, i.e.
  `ERROR: Error initializing RemoteModule` and exit 37 (by B4's metadata rule; Java's
  `handleInitFailure` would be 2). `create_channel` (`:994-1022`) fails eagerly because
  `google_auth_utils::new_channel` builds the TLS context and parses target and proxy up front.
- Java: `remote/RemoteModule.java:985-1019` `createChannel` cannot throw; there is no
  `handleInitFailure` call for channels (the only ones are at `:308, :619, :727, :840, :879`).
  `remote/GoogleChannelConnectionFactory.java:102-105` calls `channelFactory.newChannel` inside
  `Single.fromCallable`, so the `IOException` reaches each call's future.
- Failing scenario: `blaze build //:x --remote_cache=grpcs://host --tls_certificate=/missing.pem
  --remote_download_outputs=all`. Bazel builds locally, printing the `Remote Cache:` warning
  from `RemoteSpawnCache`, and exits 0 (with `--remote_executor` and `--remote_local_fallback`
  likewise). Blaze stops before loading with `Error initializing RemoteModule`, exit 37.

## Checked and found consistent with Java

- **Order of the hooks.** `beforeCommand` runs after the module loop (which holds
  `CredentialModule.beforeCommand`) and before the BES set-up (`build.rs:142-154`), as in
  `Bazel.BAZEL_MODULES`. `afterCommand` runs after the command and `command_precomplete`, before
  the BES `afterCommand` (`build.rs:155-170`), which is where `BlockWaitingModule` runs the task
  in Java. It runs on every return path of `run_build_with_bep`, failure and interrupt included,
  but not after a panic (`unported!`).
- **Remote strategy as default.** Unchanged `BazelStrategyModule` logic
  (`executor_setup.rs:405-411`); the module's registration (`remote_module.rs:851-866`) matches
  `RemoteModule.java:1176-1188`.
- **RemoteSpawnCache.** Registered only when a cache is available
  (`execution_tool.rs:567-572`). Java also registers the placeholder provider's cache; that one
  returns `NO_RESULT_NO_STORE` for every spawn, so the behaviour is the same. Recorded in
  `docs/decisions.md`.
- **Action execution salt.** `compute_action_execution_salt` (`remote_module.rs:870-892`) has
  Java's inputs in Java's order (`RemoteModule.java:1312-1331`); the exec properties map is
  sorted as `ImmutableSortedMap` is. It is applied whenever the command has a module, which is
  every `build` and `test`, and those are the only commands that execute actions today. The
  options passed are the unmutated copy (`execution_tool.rs:577`), which is harmless because
  `isRemoteCacheEnabled` also tests `remote_executor`.
- **Exit code precedence.** `build.rs:166` keeps an infrastructure failure of the command, as
  `BlazeRuntime.java:795` does.
- **Capability gate** (`grpc/channel.rs:364-373, 467-473`). All three call kinds go through
  `start_call`; `GetCapabilities` is exempt, so the fetch cannot wait on itself. The result is
  fetched once and cached, failure included, as `getAndVerifyServerCapabilitiesStarted` plus the
  `SettableFuture` do. The error is wrapped in `RemoteExecutionCapabilitiesException` like
  `GoogleChannelConnectionFactory.java:147-156`, and `status_to_io_error` hands the original
  exception back. The carrier status is UNKNOWN; none of the call sites that test a status code
  before converting (NOT_FOUND, UNIMPLEMENTED, ALREADY_EXISTS) can match it. Requirement `NONE`
  gets no gate, as in Java.
- **Remote output tree and concurrency.** An entry is keyed by an output's absolute path and
  each path has one generating action; shared actions run once behind `build_action_map`'s
  `OnceLock` (`skyframe_action_executor.rs:566-596`), and the clear sits inside that single run
  (`:673-678`), before `prepare`. So one action's clear cannot touch another action's entry, and
  an entry cannot be read before its own action injected it in this build. The tree is created
  per command (`execution_tool.rs:191`), so nothing crosses commands. Stale metadata after a
  remote attempt that injected and then fell back to local execution exists in Java too (same
  per-action file system, remote tree consulted before the local one,
  `RemoteActionFileSystem.java:690-699`). The metadata store prefers the tree at the point Java
  stats through the action file system (`skyframe_action_executor.rs:1826-1830`) and returns the
  value unchanged, as Java does for a `FileStatusWithMetadata` with a digest. The injector keeps
  Java's `isOutput` test (`execution_tool.rs:133`).
- **Refusal of `toplevel|minimal`.** `executor_init` (`remote_module.rs:260-275`) refuses any
  cache, disk only included, before the first action. Only `build` and `test` reach an executor,
  and both go through it.
- **Secrets in logs.** The gRPC log records only the `RequestMetadata` header
  (`logging/logging_interceptor.rs:133-141, 178-184`), as `LoggingInterceptor` does. No option
  struct holding `--remote_header` values is formatted into an event or panic in the files in
  scope.
- **Tests.** No test was weakened. The `should_panic` tests removed from `unported_options.rs`
  covered refusals that this commit lifts on purpose.

## Non-blocking findings

- **N1 (S3)** `--experimental_remote_cache_lease_extension` is accepted and does nothing; Java
  starts a `RemoteLeaseExtension` (`RemoteModule.java:1281-1297`) and, with
  `--rewind_lost_inputs`, prints a warning (`:1271-1278`) that Blaze never prints. Should be
  `unported!` when the flag is set. Not listed in `docs/remote.md`.
- **N2 (S3)** On interrupt Java cancels background uploads
  (`RemoteExecutionService.java:2118-2124`); Blaze waits for them
  (`remote_execution_service.rs:235-242`, a `PORT:` note). Not listed in `docs/remote.md`.
- **N3 (S3)** `RemoteActionFileSystem` (`remote_output_tree.rs`) answers `stat`/`exists` for a
  remote-only file but delegates `get_input_stream` to the local file system, so a reader gets
  "No such file" where Java downloads. Unreachable with today's rules (nothing reads a `.d` as
  an input); should be `unported!` at that point.
- **N4 (S3)** A `beforeCommand` failure returns at once (`build.rs:145-152`): the BES/BEP
  set-up and `afterCommand` are skipped, where Java still runs every module's `beforeCommand`
  and `afterCommand` (`BlazeCommandDispatcher`), so a requested BEP file is not written.
  Not checked against the oracle.
- **N5 (S3)** Commands other than `build`/`test` that carry remote options without executing
  (`cquery`, `aquery`) have no `RemoteModule`: Bazel prints `INFO: Invocation ID: ...` and
  creates `bazel-remote-logs`; Blaze accepts the flags silently.
- **N6 (S3)** The capability gate and the remote output tree, both new, have no unit test. The
  `RemoteModuleTest` capability tests are `#[ignore]` with empty bodies
  (`tests/remote_module_test.rs:105-151`); the only evidence is one end-to-end row.
- **N7 (S3)** `build_remote_action_with_scrubbing` is newly `#[ignore]`
  (`tests/remote_execution_service_test.rs:4463`); the reason given is honest (it was behind
  the `wip` feature before).
- **N8 (S3)** Disk cache garbage collection flags are accepted and do nothing. Disclosed in
  `docs/remote.md`.
- **N9 (S4)** Hook order inside `prepare_for_execution_with_action_cache` is
  strategies, contexts, `executor_init` (`execution_tool.rs:553-578`); Java's `ExecutionTool`
  runs `executorInit` first. No effect today, since the temp path generator is passed at
  construction.
- **N10 (S4)** `docs/remote.md` says `afterCommand` will "delete downloads" and that its
  exception "becomes the exit code"; both are true of the Rust but describe behaviour B3 shows
  is not Java's.

## `docs/remote.md` "Phases, evidence, gaps" against the code

- Wiring paragraph: matches the code. It does not match Java on the points in B3 to B5.
- "Fails loudly" list: accurate. Missing from it and from the gaps: B1, N1, N2, N5.
- Gaps paragraph: B2 is stated accurately.
- Evidence table: not re-measured (no oracle run). It has no row for a rebuild without `clean`
  (B2), none with a BEP file (B1), and none for an init failure (B3 to B5).

# Round 2

- Commit reviewed: `3bb3adf` against `3353c77` (`git diff 3353c77 3bb3adf`, `git show
  3bb3adf:<path>`). All `file:line` references in this section are at `3bb3adf`.
- Same reviewer role and rules: read-only, no Cargo, no build, no oracle run. The oracle rows
  the response cites (`docs/reviews/2026-11-19-remote-wiring-response.md`, `docs/remote.md`
  "Review round 1" table) were read, not re-measured.
- The credential implementer's edits in the same diff (`blaze_repo_fetch`,
  `blaze_remote/src/authandtls`, `commands/query.rs`) belong to another reviewer. Of them only
  `grpc/channel.rs` and `util/utils.rs` were read, for their effect on the wiring.

## Verdict

**PASS**, bound to `3bb3adf`. B1 to B5 are resolved. No blocking finding. Five non-blocking
findings (R2-1 to R2-5) are listed below; R2-1 should be answered with oracle evidence in the
next round of this area.

## B1 to B5

- **B1: resolved by a loud refusal.** `remote_module.rs:851` sets the flag at the point Java
  calls `buildEventArtifactUploaderFactoryDelegate.init` (`RemoteModule.java:906`), which is
  after the disk/HTTP-only return, so a disk cache alone does not set it (Java's delegate
  returns `LocalFilesArtifactUploader` there, `:1362-1365`). `build.rs:157-178` refuses when the
  flag is set, a text, binary or JSON file is requested, and the strategy is unset or `remote`;
  that is `BuildEventArtifactUploaderFactoryMap.select` (`:33-42`: null picks the only
  registered factory, any other name falls back to local). Coverage asked for by the lead:
  - Every case where Bazel writes `bytestream://` into a BEP file is refused. I found none that
    slips through: the three file transports are the only consumers of the uploader in a
    `build`/`test` without `--bes_backend` (which is refused separately).
  - One case is refused where Bazel would not write `bytestream://`: see R2-2.
- **B2: resolved.** `execution_tool.rs:496-520` stores output metadata exactly when the module
  has a cache, which is when Java's output service is `RemoteOutputService`
  (`RemoteModule.java:694-697`, `RemoteOutputService.java:181-188`).
  `skyframe_action_executor.rs:1052-1057` checks entries with the module's `RemoteOutputChecker`
  and falls back to `TrustLocalOnly`. `setCheckMetadataTtl` has no caller in the pinned Java, so
  trusting in-memory outputs without a clock is Java's behaviour. The loader and the injection
  on a hit (`action_cache_checker.rs:723, 953-960`) follow `ActionCacheChecker.java:315`. The
  oracle row for the no-clean rebuild is in `docs/remote.md`.
- **B3: resolved.** `remote_module.rs:866-887`: provider shutdown, temp directory deleted with
  the failure ignored, then the log close with `Partially wrote RPC log file`,
  `LOCAL_ENVIRONMENTAL_ERROR`, `RPC_LOG_FAILURE`; Java's order and text
  (`RemoteModule.java:1140-1173`). The citations of methods that do not exist are gone.
- **B4: resolved.** `spawn_strategy_registry.rs:26-51` carries the explicit code;
  `remote_module.rs:166-181` takes it; the four call sites pass Java's values
  (`handleInitFailure` 2; log directory, stale downloads and RPC log 36);
  `build.rs:217-229` uses it at both places a `RemoteModule` exception becomes the result.
- **B5: resolved.** `remote_module.rs:1058-1076` returns `ReferenceCountedChannel::failed`
  instead of an error. In `channel.rs` the creation error is returned before the capability
  gate (`:506-510`) and by `get_server_capabilities_async` (`:433-435`) unwrapped, so the text
  has no `Failed to query remote execution capabilities:` prefix, which matches a failure of
  `Single.fromCallable(newChannel)` before `getAndVerifyServerCapabilities`
  (`GoogleChannelConnectionFactory.java:102-105`). The eager fetch is skipped for such a
  channel. The carrier status is UNKNOWN with a `CallIoError` source, as for the gate, so no
  call site that tests a status code can mistake it.

## N1, N3, N10

- **N1: resolved.** `remote_module.rs:300-319` prints Java's warning text
  (`RemoteModule.java:1271-1278`) when both flags are set and a cache exists, and refuses the
  lease extension alone. The `LeaseService` without an extension is left out with a `PORT:`
  note; it only acts after lost inputs, which needs rewinding.
- **N3: resolved.** `remote_output_tree.rs:214-224`.
- **N10: resolved.** The "Wiring" paragraph of `docs/remote.md` now describes Java's order and
  codes. N2, N4, N5, N6 are listed under gaps.

## Interaction of the credential edits with the wiring

- `channel.rs:529-538`: the UNAUTHENTICATED status now carries the credential error as its
  source. `status_to_io_error` only unwraps a `CallIoError` source, so this status is not
  confused with a gate or creation error.
- Order in `start_call`: shutdown, creation error, capability gate, then credentials. Java
  acquires the connection before the call credentials run, so the order is right.
- `utils.rs:486-494` walks the status' cause chain for a credential helper message; gate and
  creation errors never reach it as a status (they are unwrapped first). No bad interaction
  found.

## Non-blocking findings

- **R2-1 (S2) The execution log option error is described as thrown by `ExecutionTool`'s
  constructor, and outranks the registry error.** `build.rs:414-427` turns the
  `SpawnLogModule` error into `executor_init_error`; `execution_tool.rs:576-590` gives it
  priority over the spawn strategy registry error; the comment says "SpawnLogModule.executorInit
  throws from ExecutionTool's constructor". Java does not throw there:
  `bazel/SpawnLogModule.java:85-97, 194-200` call `env.getBlazeModuleEnvironment().exit(...)`,
  the constructor goes on, and `spawnStrategyRegistryBuilder.build()`
  (`buildtool/ExecutionTool.java:207`) throws first if it fails; the pending exception is
  thrown at the next `env.throwPendingException()` (`AnalysisAndExecutionPhaseRunner.java:113`).
  Scenario: `--spawn_strategy=bogus --execution_log_json_file=a --execution_log_binary_file=b`.
  Bazel reports the strategy error, Blaze reports "Must specify at most one of ...". Two
  command-line errors are needed at once and both exit 2, so it does not block. The response
  does not mention this change and gives no oracle row for it. Wanted: the comment corrected,
  the priority reversed, and an oracle row for the single-error case (where the error appears
  relative to the analysis output).
- **R2-2 (S3) The BEP refusal is wider than Bazel's use of the uploader.** With
  `--nobuild_event_json_file_path_conversion` (and the text/binary twins) Java gives that
  transport a `LocalFilesArtifactUploader` (`BuildEventServiceModule.java:838, 868, 898`) and
  writes `file://`. `build.rs:157-178` does not read the path conversion options and refuses.
  Loud, with a documented way around it, so honest; it should not refuse when every requested
  file has path conversion off.
- **R2-3 (S3) Other users of the same uploader are not covered by the refusal.**
  `--experimental_stream_log_file_uploads` routes the profile and the Skyframe dump through
  `getBuildEventArtifactUploaderFactoryMap().select(...)` (`BuildTool.java:733-739, 790-796`).
  Blaze has no reader of that option outside `blaze_optdefs`, so with a gRPC cache it is a
  silent no-op. Outside the BEP file case B1 was about.
- **R2-4 (S4) `build.rs:862-875` drops `AbruptExitException.exit_code`.** The
  `ExecutionTool` path computes the exit code from the failure detail only. No exception with
  an explicit code reaches it today.
- **R2-5 (S3) The action cache's remote metadata path has end-to-end evidence for files only.**
  `load_cached_output_metadata` also handles tree artifacts and archived trees; the bench
  workspace has none, and with `--remote_download_outputs=all` only in-memory files are remote,
  so that branch is not reached today.

Round 1's N2, N4, N5, N6, N7, N8 and N9 stand as written. N9's description of Java was
imprecise: `ExecutionTool.<init>` runs `executorInit`, `registerActionContexts` and
`registerSpawnStrategies` per module, in that order (`ExecutionTool.java:170-185`).
