# Response: persistent server and client (Track S), review of 2026-11-25

Implementer: server-fix-1 (Opus 5.5). Responds to `docs/reviews/2026-11-25-server.md` (STOP at
e8b79c0). Tree: HEAD 3a006d7 plus uncommitted work of several agents (root's checkpoint 77eacad
already contains a first state of these changes, including a diagnostic line that the working
tree removes again: `TEMPDIAG release_build_arena` in `crates/blaze_runtime/src/dispatcher.rs`).
Binary of the evidence below: `~/.cache/blaze/work/server-fix-1/bin/blaze-2` (sha256
68455f52890ee5ef…, `scripts/cargo-capped build -p blaze` of that tree), unless a run names
`blaze-1` (00577a5e…, the same changes before the repository-rule leak fix and the doc edits).
Oracle: the pinned binary. Workspaces and outputs: `~/.cache/blaze/work/server-fix-1/`.

## F1 (S1): per-command state and the arena release

Changed:

- Package progress reset where Java resets it. `SkyframeExecutor.preparePackageLoading`
  (`SkyframeExecutor.java:1742`, run once per command by `CommandEnvironment.syncPackageLoading`)
  is `crates/blaze_runtime/src/commands/query.rs:63` `prepare_package_loading`, called at every
  site where a command syncs package loading: the dispatcher for commands that analyze
  (`dispatcher.rs:674`, `BlazeCommandDispatcher.java:651`), `query` (`commands/query.rs:327`,
  `QueryEnvironmentBasedCommand.java:111`), `canonicalize-flags` (`canonicalize.rs:113`), `mod`
  (`mod_command.rs:105`). The receiver no longer resets on the arena generation
  (`crates/blaze_sky_loading/src/package_progress_receiver.rs:1-17, 53`; module comment rewritten).
  `info` does not call it: `CommandEnvironment.syncPackageLoading` is still `unported!` there.
  The other resets of `preparePackageLoading` are fields of Blaze's per-command executor
  (comment at `query.rs:55-62`).
- The release is switched on: `BlazeCommandDispatcher::release_build_arena_after_commands`
  (`dispatcher.rs:97`), set only by `BlazeRuntime.serverMain` (`runtime.rs:379`); `exec` calls
  `release_build_arena` (`dispatcher.rs:333`, body and soundness argument at `:1160-1174`) after
  the command returned, still under the command lock, not after a crash or `shutdown`.
- Comments corrected: `execution_seam.rs:876` (`TopLevelAspectDrivers` is per build: reset by
  `note_top_level_aspects_analyzed_for_execution`), `dependency_resolver.rs:670`,
  `bzl_context.rs:65` (open item 5), `build_arena.rs` module comment (call site, per-command state
  independent of it, the no-self-ownership rule).
- The leak the release did not cure (about 36 MB per build, 11 MB per query). Found with
  jemalloc's heap profiler (the server started through `BLAZE_SERVER_MODE=jvm` and a
  `--server_javabase` whose `bin/java` preloads jemalloc; dumps after commands 3 and 10;
  symbolized with addr2line): all retained bytes were .bzl evaluation (frozen module heaps,
  bytecode, ASTs, rule classes, globals). Live-instance counters (temporary, removed) showed
  that after every release there were 0 `BzlLoadValue`s, 0 `StarlarkBuiltinsValue`s, 0
  `SkyframeExecutor`s, 0 leases. The holder is a reference cycle:
  `repository_module.rs` `get_repo_rule` caches the `RepoRule` in the `repository_rule` value,
  which lives on the defining module's heap, and `RepoRule.impl_` was an `OwnedFrozenValue`
  owning that same heap. The heap owned itself, so every module defining a repository rule (and
  every heap it references: loads, globals, builtins) stayed alive after every command. Fixed
  as the other keep-alive handles are (`StarlarkCallable`): the handle does not own the heap,
  the arena's BZL_LOAD keep-alive does (`crates/blaze_repo_rules/src/starlark/repository_module.rs:331-342`,
  `repository/repo_rule.rs:31`). After the fix: live heap delta over 7 `build --nobuild`
  commands under jemalloc -0.1 MB (RSS flat at about 320 MB); glibc in-use bytes after builds 1
  to 12: 4,732 kB to 4,933 kB (+18 kB per build; before: +35 MB per build).

Evidence (oracle with `--notrack_incremental_state --nokeep_state_after_build`, Blaze with
`BLAZE_SERVER_MODE=daemon`, each on one server, same workspace copy, fresh output roots, compared
exit status, stdout, stderr; timer-driven progress lines (`Analyzing:`, `Computing main repo
mapping:`, `[n / m] ...`) dropped):

| Case (small workspace `ws-small`) | Result |
| --- | --- |
| `build //a:x`, `build //b:y`, `build //b:y` again | equal; //b:y prints `(5 packages loaded, 8 targets configured)` both times, as Bazel |
| `query` (with an invalid `--jobs`), failing `build //a:fail`, good build | equal (exit 2, 1, 0) |
| `--host_jvm_args=-Dprobe=1 build`, then `shutdown` | equal: restart warning text, new server pid, exit 0 |
| `build`, `query //...`, Ctrl-C during a 30 s action, `build` | equal; interrupted build exit 8 with Bazel's text, same server pid after |
| `query //... \| head -1`, `build 2>&1 \| head -1`, then `build` | equal (exit 0; 141 for the client killed by SIGPIPE; next build normal, same server) |

Brotli, `scripts/server/seq run --project brotli` with `blaze-1` (report
`~/.cache/blaze/work/server/tools/seq/runs/20261125T105927-brotli-daemon-cold-3514433`): 17
steps, 8 identical, 9 different. Every `Loading:`, `Analyzed` and `N processes` line is equal
(the second build prints `Loading: 0 packages loaded` and `(89 packages loaded, 594 targets
configured)`, as Bazel). The differences: the allow-listed `buildMetrics` node counters (D9);
`actionsCreated` per mnemonic, which Bazel doubles on the second command of a server and Blaze
does not (present in the 2026-11-19 runs too; BEP metrics, not this area); Bazel's empty
`INFO: ` line before `Build Event Protocol files produced successfully.` (Blaze omits it in the
one-process mode as well); extra timer lines `Computing main repo mapping:` on the cold
start. The rerun with `blaze-2` was refused by the tool: less than 120 GB free on /home.

Soak, `scripts/server/soak` (brotli, `build //...`, one server, VmRSS after each command):

| Run | Command 1 | 10 | 30 | 50 | Growth, last 20 commands |
| --- | ---: | ---: | ---: | ---: | ---: |
| `blaze-1` (release on, cycle not fixed) | 250 MB | 676 MB | 1,355 MB | 2,033 MB | 34 MB per command |
| `blaze-2` | 240 MB | 418 MB | 562 MB | 622 MB | 3 MB per command |
| `blaze-2`, `c/common/constants.c` edited before each build | 241 MB | 405 MB | 545 MB | 608 MB | 3 MB per command |

All 150 commands exited 0 with the expected summary. What is left is not a leak (in-use bytes
are flat): it is glibc keeping freed memory (no `#[global_allocator]`, a new set of threads per
command, so new per-thread arenas); it decreases per command. `query` on brotli: before 11 MB per
command; after 17, 10, 7, 5, 4 MB for commands 2 to 6.

## F2 (S2): event bus lifecycle

`after_command` now resets the reporter's output filter (`dispatcher.rs:952`,
`BlazeRuntime.java:756`), records `env.getBlazeWorkspace().clearEventBus()` (`:1008`,
`BlazeRuntime.java:810`; Blaze's executors are gone by then), and at Java's `:832` clears the
reporter's bus and drops the bus's subscribers (`:1036-1038`,
`CommandEventBus::clear_subscribers`, `command_environment.rs:916`, the port of
`BlazeWorkspace.clearEventBus`'s "release memory associated with handlers"). This breaks every
cycle through the bus (`ExecutionProgressReceiverSetup`, the test aggregator). Evidence: new test
`dispatcher_tests.rs:1201` `no_memory_leak_of_event_bus_through_its_subscribers` (a subscriber
holding the bus; both are freed after the command); a temporary counter in the daemon showed 0
live `CommandEventBus` after each brotli build.

## F3 (S2): panics on threads nobody joins

The default uncaught exception handler moved below every thread-starting crate:
`crates/blaze_util/src/uncaught_exception.rs` (`set_default_uncaught_exception_handler`,
`run_catching`, `run_catching_or_return_if_crash_in_progress`; without a handler it does what
the JVM's default does: print `Exception in thread "<name>" ...`, end the thread).
`blaze_server::uncaught_exception` keeps its API and its halt-37 fallback on top of it.
`BlazeRuntime.setupUncaughtHandlerAtStartup` (`runtime.rs:520-560`) installs `BugReport.handleCrash`
with `CrashContext.halt()` (Java's handler) and `haltOrReturnIfCrashInProgress()` for threads that
catch `Throwable` themselves; it is installed in daemon, batch and in-process mode alike. Thread
bodies whose Java counterpart is a plain `Thread` or an `execute`d task now run through it:
`cli-update-thread` (`blaze_events/src/ui.rs:1392`, with Java's
`haltOrReturnIfCrashInProgress`), `interrupt-signal-handler` (`interrupt.rs:147`), `SIGTERM
handler` (`jvm_signals.rs:73`), `collect-local-resources` (`collect_local_resource_usage.rs:308`),
the multiplexer sender and receiver (`worker_multiplexer.rs:356, 367`),
`worker-lifecycle-manager` (`worker_lifecycle_manager.rs:85`), `AsyncFinish-Worker-N`
(`worker_spawn_runner.rs:667`), the worker pipe reader (`recording_input_stream.rs:56`, whose
panic would otherwise leave its consumer waiting forever), the remote background task executor
(`remote_execution_service.rs:273`). Not wrapped, because their Java counterpart keeps the
exception in a future or the panic reaches a joiner: the evaluator's scoped workers, the
`subprocess-fork` executor, the consumer threads, the download threads, `bep-local-writer`
(already catches into its close future), `profile-writer-thread` (re-raised at `shutdown`),
tokio tasks. Not done: a remote-cache download thread (`remote_execution_cache.rs:172`) maps a
panic to "interrupted"; noted for the remote owner. Evidence: unit test
`escaping_panic_reaches_the_handler_with_its_kind` (blaze_util); no end-to-end panic was
injected. Comment of `bugreport.rs:14-23` rewritten.

## Open items and S4 notes

1. Early exits (`dispatcher.rs:919` `after_command_on_early_exit`, 6 sites): the early result is
   returned and `afterCommand`'s is dropped, as `BlazeCommandDispatcher.java:884-889`. Test
   `early_exit_keeps_its_result_over_after_command` (`dispatcher_tests.rs:1251`).
2. Bad cookie on `Cancel` / `UpdateTerminalSize`: grpc-java 1.66.0 (bytecode of the oracle's
   `A-server.jar`: `ServerCallImpl.closeInternal`, `NettyServerStream$Sink.cancel`) answers a
   unary call completed without a message with `RST_STREAM(CANCEL)`, which clients report as
   `CANCELLED`, not OK. `grpc_server_impl.rs:869` now answers `CANCELLED` without a message
   (tonic cannot reset the stream; PORT note there).
3. Shared output base with a live Bazel server in daemon mode: unchanged; it is the planned end
   state; to be stated in the S5 notes.
4. `waitForTimestampGranularity`: unchanged (Track I).
5. `ByFilename`: the arena is per command in every mode now; comment updated.
- F5: `clean.rs:222-228` comment rewritten for the daemon (fds 1 and 2 stay open; closing them
  would let a later `open` reuse the number). `server_args` (`crates/blaze/src/main.rs:59-67`)
  also requires `--add-opens=java.base/java.lang=ALL-UNNAMED`, which `GetServerExeArgs` always
  writes (`blaze.cc:390`), so `blaze --batch -jar x build` is a client invocation again; tests
  extended.
- F4 items not done (unchanged, marked): idle tasks, `--iff_heap_size_greater_than`, command
  extensions, terminal size monitor, SIGQUIT thread dump, `exec-server` without a server, JVM
  flags ignored.

## Verification commands

- `scripts/cargo-capped test -p blaze_runtime -p blaze_util -p blaze_server -p blaze -p
  blaze_repo_rules`: exit 0, no failure.
- `scripts/cargo-capped test -p blaze_sky_loading --lib package_progress` (3 passed),
  `-p blaze_events --lib` (89 passed, 1 ignored), `-p blaze_exec worker` (all filtered tests passed).
- `scripts/cargo-capped check --workspace --all-targets`: passes (one warning in another
  agent's test file).
