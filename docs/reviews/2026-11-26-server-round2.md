# Review, round 2: persistent server and client (Track S), 2026-11-26

Reviewer: Fable 5.2, read-only. Commit under review: **5f68157**. Round 1:
`2026-11-25-server.md` (STOP at e8b79c0: F1 S1, F2 S2 blocking; F3 S2). Response:
`2026-11-25-server-response.md`.

**Verdict: PASS**, bound to 5f68157. F1, F2 and F3 are fixed as round 1 asked. No S1 found. Two
S2 and the open items below do not block; O1 must be settled before the server default is
switched (package S5).

No probe was run: `root-suite-5f68157` was `active` for the whole review. The verdict rests on
the code at 5f68157, the pinned Java and the implementer's recorded runs (binary `blaze-2` of the
working tree before the commit, not `blaze-5f68157`). The two-command and two-target sequences
of the response were not repeated with `blaze-5f68157`.

## Scope

Read: `git diff e8b79c0 5f68157` of `blaze_runtime/src/dispatcher.rs`, `runtime.rs`,
`blaze_server` (all three files), `blaze_util/src/build_arena.rs`, `uncaught_exception.rs`,
`blaze_sky_loading/src/package_progress_receiver.rs`, `blaze_repo_rules/src/starlark/
repository_module.rs`, `repository/repo_rule.rs`, `setup.rs`; at 5f68157: `dispatcher.rs:255-345,
1008-1050, 1160-1176`, `command_environment.rs:895-930`, `bugreport.rs:361-422`,
`crates/blaze/src/main.rs:52-80`, `commands/query.rs:55-66`, the body of every thread spawn
outside tests and the client (list by grep), `remote_execution_service.rs:225-330`,
`remote_execution_cache.rs:160-190`; every process static that holds a collection or an optional
value (list by grep, 98 hits); every `unchecked_frozen_value` / empty-owner `OwnedFrozenValue`
site by file; every holder of `Arc<RepoRule>`. Java: `BlazeRuntime.java:750-838`, the callers of
`syncPackageLoading`.

Not read: `blaze_client` (the diff range has no change there), the other
`blaze_repo_rules` files of the range (other agents' work), the new tests' bodies,
`scripts/server/*`, the soak data. Not re-read: what round 1 found faithful.

## Round-1 findings

**F1 (S1): fixed.**
- Per-command reset where Java resets. `prepare_package_loading`
  (`commands/query.rs:63`, `SkyframeExecutor.java:1742`) is called at the four ported callers of
  `CommandEnvironment.syncPackageLoading`: `dispatcher.rs:674`
  (`BlazeCommandDispatcher.java:651`), `query.rs:327` (`QueryEnvironmentBasedCommand.java:111`),
  `canonicalize.rs:113` (`CanonicalizeCommand.java:173`), `mod_command.rs:105`
  (`ModCommand.java:282`). The fifth Java caller, `InfoCommand.java:284`, is still `unported!`
  (S3, honest). The receiver no longer reads the arena generation
  (`package_progress_receiver.rs:53`).
- The release: `dispatcher.rs:332-333`, after `exec_exclusively` returned, inside the closure
  that holds the command lock, skipped when the result is a shutdown (which includes a crash).
  Set only by `server_main` (`runtime.rs:377-380`).
- Does anything that outlives a command point into the arena? I found nothing. The long-lived
  objects are `BlazeRuntime` (modules, command map, options, `CrashEnv` cleared in
  `after_command`), `BlazeWorkspace` (directories only), the dispatcher, and the process statics.
  None of the 98 statics holds a frozen value except those that own their heap
  (`EMPTY_DEPSETS`, `instrumented_files_info::EMPTY`, `MODULE_BAZEL_ENVS`). The BES module's
  `current` is taken in `after_command` (`bazel_modules.rs:1035`). There is no persistent thread
  pool (no rayon, each command on a new `grpc-command-N` thread, evaluator workers scoped), so
  thread-locals die with the command.
- What guards a release while a detached thread of the finished command still runs? Only the
  lease count (`build_arena.rs:186`); leases are held by `SkyframeExecutor` and
  `BzlLoadFunction` and by nothing else. A detached thread is not counted. I read the bodies of
  the threads that can outlive `exec_exclusively` (`AsyncFinish-Worker-N`, the multiplexer
  threads, `worker-lifecycle-manager`, the worker pipe reader, the download threads, the process
  reaper, the remote cache download helper): each captures plain data (worker handles,
  `WorkRequest`, paths, URLs), no action, spawn or Starlark value. The remote background uploads
  are joined in `close()` (`remote_execution_service.rs:299-321`). So the release is sound at
  this commit, by inspection, not by construction: see O1.

**The leak fix (cached `RepoRule` handle): sound today, unchecked.**
`repository_module.rs:340-342` builds `impl_` as `OwnedFrozenValue::new(FrozenHeapRef::default(),
implementation)`: an unchecked pointer with an empty owner, not a weak handle. A use after the
defining heap is freed would be undefined behaviour, not a loud failure. It cannot happen at
this commit: the holders of `Arc<RepoRule>` are `RepoRuleData.repo_rule` (on the defining heap
itself), `RepoDefinition.repo_rule` (a Skyframe value of an evaluator that owns a
`BzlLoadFunction`, hence a lease), and `ModuleExtensionEvalThreadContext` (one evaluation). No
static, module or cache keeps a `RepoDefinition` or `RepoRule` across commands, and with a lease
held nothing is released. This is the same contract as `StarlarkCallable` (accepted under D7).
See O1.

**F2 (S2): fixed.** `after_command` resets the output filter (`dispatcher.rs:951`,
`BlazeRuntime.java:756`) and at Java's `:832` clears the reporter's bus and drops the bus's
subscribers (`dispatcher.rs:1032-1038`, `command_environment.rs:916-920`). The drop comes after
the modules' `commandComplete`, as in Java, so no event Java delivers is lost. The subscribers
are dropped outside the lock. The cycle bus → subscriber → bus is broken for every subscriber.

**F3 (S2): fixed.** The default handler is installed in all three modes (`runtime.rs:285`
`main`, `:502` `batch_main`, `:663` `server_main_single_command`) and is
`BugReport.handleCrash(..., CrashContext.halt())` (`runtime.rs:529-551`, `bugreport.rs:361-422`:
Java's message, exit code file, modules' `blazeShutdownOnCrash`, `_exit` with the crash's code,
37 for a bug). The ten thread bodies named in the response go through `run_catching`; the UI
thread uses `haltOrReturnIfCrashInProgress` as Java. Threads not wrapped, checked one by one:
`bep-local-writer` (catches into its close result, `file_transport.rs:135`), `profile-writer-thread`
(joined, re-raised, `json_trace_file_writer.rs:153`), consumers, downloads, subprocess-fork
(joined or re-raised), `idle-server-tasks-0` and `pid-file-watcher-0` (Java: scheduled executor,
the exception stays in the future), the evaluator's scoped workers, `Streaming Test Output Pump`
(scoped, joined), `async-file-writer` (Java catches `Exception` into the close future; an
`Error` would reach the handler, a Rust panic here does not: S4 weight), `process reaper` (no
code that can panic). One thread is still missed, declared by the response: S2-b.

**Bad cookie:** `grpc_server_impl.rs:869` answers `CANCELLED` without a message. I did not
re-read the grpc-java bytecode; the reasoning in the comment is consistent with
`ServerCallImpl.closeInternal` as I know it, and no correct client sends a bad cookie. Accepted.

**`--batch -jar`:** `server_args` (`crates/blaze/src/main.rs:63-80`) also requires
`--add-opens=java.base/java.lang=ALL-UNNAMED` before `-jar`, so `blaze --batch -jar x build` is
a client invocation and is rejected as an unknown startup option, as Bazel. Accepted.

Early exits (round-1 open item 1): `after_command_on_early_exit` returns the early result and
drops `afterCommand`'s, as `BlazeCommandDispatcher.java:884-889`. Accepted.

## New findings

### S2

**S2-a. The response's evidence was not made with the commit's binary.** The sequences and the
soak used `blaze-1` and `blaze-2` of an uncommitted tree; the brotli sequence with the final
code (`blaze-2`) was refused by the tool for disk space. Not a code deviation; the accepted
state needs the two-command and two-target sequences and the soak from `blaze-5f68157`
(`docs/state/README.md`). Masked: I expect them to pass.

**S2-b. A panic in the remote-cache download helper is reported as an interrupt.**
`crates/blaze_remote/src/remote_execution_cache.rs:172-183`: the helper thread is not wrapped;
if it panics the sender is dropped and the waiter returns `interrupted()`. Java
(`RemoteExecutionCache.java`, the `downloadIfRemote` future) would fail the future with the
exception and crash the build with exit 37. Input: none known that panics there; masked. Fix:
send the panic payload through the channel and re-raise it in the waiter. Remote owner.

### S3

- `info` does not reset the package progress (`InfoCommand.java:284`; `syncPackageLoading` is
  `unported!` there).
- Round-1 F4 items unchanged and marked: idle tasks, `--iff_heap_size_greater_than`, command
  extensions, terminal size monitor, SIGQUIT thread dump, `exec-server` without a server, JVM
  flags ignored.

### S4

`dispatcher.rs:1160-1163`: the comment says "the threads they join are gone"; the detached
threads listed above are not joined. Say which threads may still run and why that is safe.

## Open items, not blocking

- **O1. The arena release is safe by convention only.** Two rules keep it sound and nothing
  checks either: (1) no thread that outlives `exec_exclusively` may hold a frozen value, an
  action or a spawn; (2) every holder of an unowned frozen handle (`RepoRule.impl_`,
  `StarlarkCallable`, the other empty-owner sites, ten files by grep) lives in a leased evaluator or on the heap
  itself. A future cache that keeps a `RepoDefinition`, a rule class or an action across
  commands turns into a use after free in the server, silently. Before S5: record the arena
  generation in the unowned handles (at least `RepoRule`) and assert it where the value is used
  (`repository_fetch_function.rs:900, 1128`), or make the handle fail loudly some other way; and
  write rule (1) into `build_arena.rs`.
- O2. With a lease still held at the end of a command nothing is released and nothing is
  reported; the growth returns without a signal. Log or count it.
- O3. Round-1 items 3 and 4 (shared output base with a live Bazel server;
  `waitForTimestampGranularity`) unchanged.
- O4. Memory left after the fix (3 MB per build, glibc retention by the response's reading) was
  not examined here.

## Verdict

**PASS** at 5f68157 for the review of the server and client code. It is not the acceptance of
the server default: S2-a (evidence from the commit's binary) and O1 come first.
