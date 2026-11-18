# Response to the execution review of 2026-11-18

Analysis lead (also owner of blaze_exec, blaze_sky_exec, blaze_buildtool for this fix wave),
2026-11-18. Covers docs/reviews/2026-11-18-exec.md. State described: HEAD `1fdd650` plus the
uncommitted working tree of the analysis fix waves (binary `~/.cache/blaze/int-bin/blaze-fix-6`,
sha256 `257d0bcf7564`).

Several S1s were fixed by the integration lead before this wave; "at HEAD" below means a
sub-implementer re-read the Rust against the Java and found it fixed, not that I re-tested it in
isolation. The oracle scenarios (`scripts/w4/scenarios/`, run by `scripts/w4/probe`) are the
end-to-end check.

## S1

| id | disposition | where / test |
| --- | --- | --- |
| S1-1 cached test results not reported | fixed at HEAD | `notify_on_action_cache_hit` goes through `as_test_runner_action`. re2 `tests` surface 21/21. |
| S1-2 file output that is a directory | fixed at HEAD | `checkForUnsoundDirectoryOutput`; scenario `genrule` step `build //:dir_out` (oracle text). |
| S1-3 output directory creation | fixed at HEAD | `ActionOutputDirectoryHelper` port. No scenario of its own. |
| S1-4 missing-output error texts | fixed at HEAD | scenario `genrule` step `build //:missing_out`. The "is not a symlink" branch stays behind `unported!("FileArtifactValue.createForUnresolvedSymlink")`. |
| S1-5 missing source inputs | FIXED | `completion_function.rs` `handle_source_file_error` (with `recordExecutionError`); the action-level `ERROR: <loc> Action ... failed: N input file(s) do not exist` is `SkyframeErrorProcessor.rethrow`'s, ported in `blaze_runtime/src/commands/build.rs` (`process_execution_errors`, `rethrow`) with `AlreadyReportedActionExecutionException` in blaze_exec. Scenario `exec-source-errors`. |
| S1-6 shared actions | fixed at HEAD | `transform_for_shared_action`. No scenario (needs aspects). |

## S2

| id | disposition | where / test |
| --- | --- | --- |
| S2-1 stat fallback for inputs | FIXED | `ActionInputMetadata` is a port of `ActionInputMap` plus the three Java providers; `RunfilesArtifactValue` carries files, trees and metadata; unknown derived inputs return `None`; new `blaze_exec/src/exec/single_build_file_cache.rs`. 19 `action_input_map_tests`. One exception, loud when it matters: during input discovery a derived input outside the map is stat'ed, because the evaluator cannot be asked for a done value (request to blaze_skyframe). |
| S2-2 conflicting actions merged | FIXED | analysis side: `ArtifactConflictFinder` (analysis response S2-1). Execution side: `execute_action` panics "Cannot share ..." when the second action is not shareable (`Actions.canBeShared`). Scenario `top-conflicts`. |
| S2-3 tree artifact values | FIXED | ports of `TreeArtifactValue.visitTree`, `constructTreeArtifactValueFromFilesystem`; symlinks followed; Java's two error texts; every visited entry chmod'ed (directories 0555). Tree artifacts work with the action cache. Scenario `exec-tree` (stat of tree directories, cache hit on the second build, dangling symlink); 10 `visit_tree_test`s. |
| S2-4 action output directory | fixed at HEAD | `getActionTempsDirectory`. |
| S2-5 flags that do nothing | FIXED | `--subcommands`, `--output_filter`, `--check_up_to_date`, `--experimental_writable_outputs` wired at HEAD; this wave: `ExecutionToolConfig` carries the command's `RemoteOptions`, `DynamicExecutionOptions`, `WorkerOptions` (blaze_exec's guards were unreachable behind hard-coded defaults). `--archived_tree_artifact_mnemonics_filter` set to a non-default value is now `unported!` (`blaze_runtime/src/unported_options.rs`). |
| S2-6 sandbox seams | fixed at HEAD | `package_path_entries`, repo contents cache path, `SandboxModule.afterCommand`. |
| S2-7 compact execution log | FIXED | parameter files are recognised by type (`ParamFileActionInput`); runfiles symlink sets keep their nested-set nodes. A sub-set shared by two runfiles trees is still logged once per tree (Java: once). |
| S2-8 StringEncoding identity | FIXED | 15 sites go through `blaze_util::string_encoding`; no `PORT: StringEncoding identity` remains in the three crates. |

## S3 / S4

- S3-1 (tree artifacts with the action cache) fixed with S2-3. S3-2 to S3-14 unchanged and loud.
- S4-1 (`lookup_input` quadratic) and S4-2 (`create_for_testing` in production paths) fixed;
  S4-3 unchanged.
- blaze_sky_exec's 11 clippy warnings: fixed; `clippy --all-targets -D warnings` is clean on the
  twelve crates of this wave.

## Requests from other owners, done here

- Skyframe: every SkyFunction of these crates returns `Ok(None)` on a missing dep and tolerates
  the bubbling re-run (`BuildDriverFunction` posts each event once per key);
  `compute_artifact_nested_set` is a port of `ArtifactNestedSetFunction.java:94-140`; the
  catastrophic bit comes from `is_catastrophe()`.
- Actions: `ctx.actions.write(content = args)` with a directory uses
  `new_deterministic_writer_with_input_metadata_provider` (scenario `exec-write-args-dir`).

## Open

- `ActionExecutionException` root causes are `ActionFailed` only: the `LabelCause`s of missing
  inputs do not reach BEP.
- `addOutputToMetrics` (BEP artifact metrics) and action templates in `ArtifactFunction`.
- Critical path (`Critical Path:` in the summary line was a wall time, now removed; BEP
  `buildToolLogs` lacks the entry).

## Evidence for both responses

Tree: HEAD `1fdd650` + uncommitted working tree (dirty); binary
`~/.cache/blaze/int-bin/blaze-fix-6` (sha256 `257d0bcf7564`), built from that tree.

Referee, full run of both projects, label `blaze-fix-6` against `oracle-9.3.2`
(`scripts/oracle/scoreboard blaze-fix-6 --project brotli --project re2`, referee sha256
`26e4400ec7d2`):

| project | matching / compared | allowed | failed | extra | surfaces MATCH | DIFF |
| --- | --- | --- | --- | --- | --- | --- |
| brotli | 997 / 1001 | 3 | 4 | 0 | 12 of 13 | bep (4) |
| re2 | 12853 / 12857 | 55 | 4 | 1 | 11 of 13 | bep (4), build_stderr (1 extra) |

The order items match on both projects: `cquery_label_kind` `__order__ deps`, `cquery_jsonproto`
`__order__ results` / `configurations`, and every `aquery_actions` `__order__` item (action
order, action ids, depSetOfFiles, artifacts, pathFragments, ...). Before this wave
(`blaze-int`): the order was not compared and differed on every line.

What still differs there, and whose it is:
- bep, both projects: `buildToolLogs` lacks `command.profile.gz` and `critical path` (mine,
  deferred); `--install_md5` and `--restart_reason=no_daemon` in three command-line events
  (blaze_client).
- re2 build_stderr: the `compatibility_level` MODULE.bazel warning is printed by every Blaze
  command; the oracle prints it once per server (bzlmod / server model).

Scenario probes (`scripts/w4/probe compare <scenario>` with that binary, each run cold):
27 scenarios, 20 identical to the oracle transcript; `scripts/w4/flags-probe compare`: 71 of 71
cases identical.

| scenario | result | what differs |
| --- | --- | --- |
| cc-hash, cc-tags, ctx-args-snapshot, ctx-build-file-path, ctx-immutable, ctx-run-shell-order, exec-source-errors, exec-tree, exec-write-args-dir, flags-transitions, genrule, ids-depset-freeze, query-errors, query-map-each, query-order, query-output-file, query-rdeps, top-analysis-errors, top-bad-platform, ui-exec-errors | MATCH | - |
| ctx-errors | DIFF | steps 2-3: `getattr(ctx.fragments, "cpp", None)` / `hasattr` on an undeclared fragment (needs the vendored attribute hook) |
| ctx-toolchains-string | DIFF | step 6: toolchain key converted in a helper .bzl (needs the evaluator in the index operator) |
| graph-order | DIFF | one cquery line out of place (exec deps of the first requester of the exec platform) |
| incompat-config-setting, incompat-toplevel | DIFF | one line each (`Use --verbose_failures ...`, `Target //pkg:bad failed to build`) that depends on which targets Bazel had announced as analyzed when a fail-fast analysis stopped |
| top-conflicts | DIFF | `Use --verbose_failures ...` missing after a fail-fast conflict (4 steps, open); which of two conflicting targets the WARNING names (3 steps, racy on the oracle); one sorted-line position |
| ui-build-results | DIFF | step 32: `[unix_jni.cc:382] ` prefix of a native I/O error (blaze_vfs) |

Unit tests (`scripts/cargo-capped test`, the twelve crates of this wave: blaze_analysis,
blaze_rules, blaze_sky_analysis, blaze_sky_top, blaze_cquery, blaze_build_config, blaze_config,
blaze_runtime, blaze_sky_exec, blaze_buildtool, blaze_exec, blaze_bep): 1026 passed, 0 failed,
839 ignored (most ignored tests are Java test names that need a `BuildViewTestCase`). The three
crates touched outside (blaze_cmdline, blaze_sky_loading, blaze_starlark): 694 passed, 0 failed,
126 ignored. `clippy --all-targets -D warnings` and `rustfmt --check`: clean on the twelve crates.

workerd (stress project): `cquery 'deps(//src/workerd/util:entropy)' --output=label_kind` prints
1,074 lines, all present in the oracle's `deps(//src/workerd/server:workerd)` record, with the
oracle's target configuration `13e714b` and exec configuration `1d8533e`. The full
`deps(//src/workerd/server:workerd)` now stops at a loud `unported!` for aspects
(`AspectResolutionHelpers.computePropagatingAspects`, protobuf's upb aspect); the earlier
blockers (Starlark flags, flag scoping, `HeaderInfo` hashing, genrule, analysis-error panic,
`ctx.build_file_path`) are gone.

---

# Round 2 response

Execution implementer, 2026-11-18. Covers "Round 2" of docs/reviews/2026-11-18-exec.md (STOP,
bound to 12ca29e). State described: HEAD `c68645949d3f` plus the uncommitted working tree (dirty;
other agents were editing blaze_analysis, blaze_sky_top and blaze_remote at the same time),
binary `~/.cache/blaze/int-bin/blaze-execfix-1` (sha256 `5c6616b9582f`). Not committed.

## Findings

| id | disposition | where | test / evidence |
| --- | --- | --- | --- |
| N1 (S1) `--notest_keep_going` | FIXED | blaze_exec: `UserExecException.test_exec_exception` (set by `test_exec_exception`, the port of `TestExecException.<init>`), `ExecException::is_test_exec_exception`, `ActionExecutionException::is_caused_by_test_exec_exception` (set by `fromExecException`, kept by `AlreadyReportedActionExecutionException`). build.rs: `rethrow` returns `RethrownException::TestExec` for `innerCause instanceof TestExecException` (`SkyframeErrorProcessor.java:978`); `execute_build` shows the results (the `finally`) and reports `ERROR: Test failed, aborting` with a successful build (`BuildTool.java:974-978`), so `TestCommand.doTest` returns 3. | unit `rethrow_of_a_test_exec_exception_throws_it`; scenario `test-no-keep-going` (new, oracle-recorded): exit 3 / 0 / 3 and every ERROR line equal to the oracle. |
| N3 (S1) unknown strategy | FIXED | execution_tool.rs: no `format!("{e:?}")`; the registry's `AbruptExitException` is kept in `PreparedExecution::spawn_strategy_registry_error` and thrown by build.rs `ExecutionToolConstruction` (where Java runs `new ExecutionTool`), as `BuildToolError::Exit` with the failure detail's exit code; catastrophe set; no build result shown (Java throws before the `try`). | scenario `exec-strategy-errors` (new): 7 of 7 exit codes equal (2, 2, 2, 2, 1 for a bad pattern, 2 plus `Couldn't start the build. Unable to run tests` for `test`, 0), every ERROR line equal including Java's HashMap order of `Valid values are: [...]`. Deviation, documented in the code: with Skymeld Java constructs the tool before the analysis, Blaze after it (blaze_sky_top's `BuildTool` has no hook between target patterns and analysis), so Blaze also prints `INFO: Analyzed target ...` / `INFO: Found 1 target...` (and would report an analysis error first). Request to the blaze_sky_top owner: an additive hook before `analysis_phase_runner::execute`. Audit: every other `{:?}` in blaze_exec, blaze_sky_exec, blaze_buildtool, blaze_runtime is a panic/`check_*!`/test message; the two replay tools now print `getMessage()`. |
| N4 catastrophe | FIXED | build.rs `rethrow` carries `isCatastrophe()` (`SkyframeErrorProcessor.java:992`); `execute_build` calls `set_catastrophe()` (`BuildTool.java:938`). | unit `rethrow_keeps_the_catastrophe`. |
| N5 interrupt during `test` | FIXED | build.rs `execute_build`: an interrupted evaluation goes through the `finally` of `buildTargetsWithMergedAnalysisExecution` (`BuildTool.java:620-688`): no action cache save (`buildCompleted` false), successful targets and test targets set, results shown, then `BuildToolError::Interrupted`. | Manual oracle comparison (the probe cannot send signals): `--batch test //:pass_test //:slow_test` (slow_test's data is a 30 s genrule), SIGINT after 15 s. Oracle and Blaze both print `//:pass_test PASSED`, `Executed 1 out of 2 tests: 1 test passes and 1 was skipped.`, `ERROR: build interrupted`, exit 8. Two new findings (not fixed here): Blaze does not cancel running spawns on interrupt (it waits for the genrule: 33 s against 15 s; blaze_exec/blaze_shell), and without `--batch` Blaze installs no SIGINT handler (killed, exit 130; blaze_runtime `capture_sigint` runs only in `batch_main`). |
| N6 spawn log `remote_options` | FIXED | build.rs passes `options.get_options::<RemoteOptions>()` as Java's `env.getOptions().getOptions(RemoteOptions.class)` (`SpawnLogModule.java:115,146`). | `build //:one --remote_default_exec_properties=OSFamily=linux --remote_default_exec_properties=pool=big --execution_log_json_file=...`: oracle and Blaze both log `platform {OSFamily=linux, pool=big}`; ported `SpawnLogContextTestBase` platform tests unchanged. |
| N7 stack size | FIXED | New `blaze_exec::exec::evaluation_stack::on_evaluation_stack`: runs `Action.execute` (`SkyframeActionExecutor`), the action key of the cache check (`CacheCheckActionAdapter::get_key`) and `Actions.canBeShared` on a thread with `EVAL_STACK_SIZE` (via `run_on_evaluation_thread`; inline when already on one; interrupt status carried back; panics resumed). Evaluator threads stay at 16 MB. Why this does not bring back the fork ENOMEM: the fork accounts every reserved stack of the process; the evaluator may hold `max_threads` (512) plus forced threads, which at 256 MB exceeds RAM + swap (measured on this machine: 400 threads x 256 MB makes `fork` fail with ENOMEM, 200 do not), while the 256 MB threads now exist only for actions in flight (bounded by `--jobs`), e.g. 16 x 256 MB = 4 GB. | 4 unit tests in `evaluation_stack` (a 64 MB recursion from a 1 MB thread, nesting, interrupt status, panics); brotli referee run and all exec scenarios below run through it. |
| S2-5 / N2 `--output_filter` at the Reporter | FIXED | build.rs sets `env.get_reporter().set_output_filter(...)` at `BuildStartingEvent` (`OutputFilteringModule.java:78`); `--auto_output_filter=none/all` set at the same point (the events before `TargetParsingCompleteEvent` carry no tag, so this is equivalent); `packages`/`subpackages` stay `unported!` (they need the labels of a `TargetParsingCompleteEvent` blaze_sky_top does not post). | scenario `ui-output-filter` (new): the tagged analysis WARNINGs (`depends on deprecated target`) and action output are filtered exactly as on the oracle in all 5 steps (`^//a2`, no match, `all`, `none`); DEBUG is never filtered on either side. |
| S2-7 compact log | FIXED | blaze_exec `to_action_input_set` memoizes per `NestedSet<Artifact>` node for the process, so every spawn holding a shared depset holds the same converted node (Java upcasts the same object) and `CompactSpawnLogContext` logs it once. | unit `a_shared_sub_set_is_one_node_in_every_converted_set`; new tool `scripts/execlog-compact` (record, dump, canon, compare): oracle and Blaze compact logs of brotli have identical multisets of canonical entries (354: 205 file, 100 input_set, 43 spawn, 5 symlink_action, 1 invocation; the same 3 duplicated input sets on both sides). The streams are not byte-identical: entry ids follow the order in which spawns finish, and spawn metrics are timings. No pre-fix log to show the difference (the 12ca29e binary is gone). |
| blaze_repo_fetch `no_proxy` split | FIXED | `proxy_helper.rs` `java_split_comma`: Java `split("\\s*,\\s*")` (leading empty entry, outer whitespace kept, trailing empties dropped, `[]` for `","`). | `java_split_comma_trims_around_separators` (restored, in blaze_repo_fetch; expectations printed by OpenJDK 25.0.5 for each input) and `no_proxy_entry_with_leading_whitespace_does_not_match`. The name-map entry under "No equivalent" can now map to `blaze_repo_fetch::downloader::proxy_helper::tests::java_split_comma_trims_around_separators`. |
| S1-3 scenario | ADDED | scenario `exec-output-dirs`: a file where a later build needs a directory, a read-only tree artifact below which a later build writes, a symlink where a later build needs a directory, and back. | 21 steps; every `stat`/`cat` step matches the oracle (file 555 → dir 755, tree 555 → 755, symlink → dir 755). |
| S1-6 scenario | ADDED | scenario `exec-shared-actions` (Bazel's `RewindingTestsHelper.setUpParallelTrackSharedActionPackage`, plus a shared tree artifact): two targets register equal actions for `A-shared.out` and `shared-tree`. | 7 steps; outputs, tree stats and exit codes match. |

## Evidence

Unit tests (`scripts/cargo-capped test`, blaze_exec, blaze_sky_exec, blaze_buildtool,
blaze_runtime, blaze_repo_fetch, dirty tree above): 987 passed, 0 failed, 195 ignored.
`clippy --no-deps --all-targets -D warnings` on the same five crates: clean (`--no-deps`
because blaze_sky_loading and blaze_analysis, edited by others, fail clippy in the working tree).
`rustfmt --edition 2024 --check` on every changed file: clean.

Referee, brotli, label `blaze-execfix-1` (binary above): 997 of 1001 compared items, 3 allowed,
4 failed, 12 of 13 surfaces MATCH, bep DIFF (4): identical to `corpus/scores/blaze-12ca29ee95e0.md`
for brotli. The execlog surface (44 items) matches.

Probe scenarios (`BLAZE=~/.cache/blaze/int-bin/blaze-execfix-1 scripts/w4/probe compare <s>`,
each run cold; `compare-all` was killed twice by concurrent probe runs of other agents, so the
execution scenarios were run one by one). The probe was switched to full stdout/stderr
transcripts by another agent during this work, and in that mode no scenario matches step for
step: every run step differs on lines no fix here touches, namely `Starting local Bazel server`,
`N packages loaded` and action-cache-hit counts after the first step (the oracle keeps its
server between steps; Blaze runs one process per command), `Critical Path: ...` (not ported),
and the position of `INFO: Found N targets...` and other stderr lines (Skymeld interleaving,
parallel actions). The table counts steps that differ on anything else (the global lines above
removed, the remaining lines compared as multisets):

| scenario | steps | differ beyond the global lines | what |
| --- | ---: | ---: | --- |
| test-no-keep-going (new, N1) | 3 | 0 | |
| exec-strategy-errors (new, N3) | 7 | 0 | (the extra `Analyzed`/`Found` lines of the N3 deviation are among the global lines) |
| exec-output-dirs (new, S1-3) | 21 | 0 | |
| exec-shared-actions (new, S1-6) | 7 | 0 | |
| ui-output-filter (new, S2-5) | 5 | 4 | `DEBUG: analysis of //dep:old` is printed by every Blaze command; the oracle's server analyzed it once (server model) |
| exec-source-errors | 8 | 0 | |
| exec-tree | 22 | 0 | |
| exec-write-args-dir | 7 | 0 | |
| genrule | 32 | 0 | |
| query-map-each | 5 | 0 | |
| ui-build-results | 49 | 5 | analysis: an extra `File "<builtin>", in <toplevel>` traceback line after `fail()` (4); blaze_vfs `[unix_jni.cc:382] ` prefix (1); both known, not execution |
| ui-exec-errors | 19 | 2 | the same `File "<builtin>"` traceback line |

Scenario `ORACLE.txt` files of the five new scenarios were recorded with the oracle binary in the
full-transcript format (`scripts/w4/probe record`).

## Open after round 2

- Running spawns are not cancelled on interrupt (found while testing N5): Blaze finishes the
  running genrule before it stops; Bazel cancels it. blaze_exec / blaze_shell (interrupt of the
  thread that waits for the child); deferred, not a regression.
- Blaze installs its SIGINT handler only in `batch_main` (blaze_runtime `capture_sigint`):
  without `--batch` a Ctrl-C kills the process (exit 130) instead of "build interrupted" (8).
  Owner of the client / dispatcher.
- N3 with Skymeld: the `ExecutionTool` exception comes after the analysis (needs a hook in
  blaze_sky_top's `BuildTool` before `analysis_phase_runner::execute`). `cquery` and `aquery`
  (which construct an `ExecutionTool` in Java's `--nobuild` path) never check the strategies in
  Blaze, so `cquery --spawn_strategy=bogus` succeeds where Bazel exits 2.
- `--auto_output_filter=packages|subpackages`: `unported!` (needs the
  `TargetParsingCompleteEvent` labels at target parsing time).
- Compact execution log bytes: equal entries, different ids/order (spawn completion order).

# Round 3 response

Execution fixer, 2026-11-18. Covers "Round 3" of docs/reviews/2026-11-18-exec.md (STOP, bound to
2a85ea2). State described: HEAD `b3c53fc` plus the uncommitted working tree (dirty: another worker
was integrating blaze_remote, and others edited blaze_analysis and the vendored starlark at the
same time). Binary `target/debug/blaze` built from that tree. Not committed.

## Findings

| id | disposition | where | test / evidence |
| --- | --- | --- | --- |
| R1 (S1) running spawns never interrupted | FIXED | blaze_skyframe `evaluator.rs`: Java's `AbstractQueueVisitor.jobs` (`Evaluator::add_job`, counted per thread, registered before the `interrupted`/`aborting` check as `WrappedRunnable.run` does) and `interruptInFlightTasks` (every job thread except the current one gets `ThreadInterrupt::interrupt`). Called when the evaluation stops (fail-fast error committed, child error seen without keep_going, crash: `Run::stop_in_flight_jobs`, once), by `Evaluator::interrupt()`, and by the evaluating thread, which now waits for its root workers on a condvar and polls the `interrupt_flag()` every 20 ms (`IN_FLIGHT_INTERRUPT_POLL`; the flag is the `AtomicBool` blaze_runtime's `interrupt_command_thread` sets). blaze_shell `thread_interrupt`: `ThreadInterrupt::adopt` (a thread stands in for another's interrupt status); blaze_exec `evaluation_stack::on_evaluation_stack`: the 256 MB action thread adopts the evaluator thread's status, so the interrupt reaches `LocalSpawnRunner`/`AbstractSandboxSpawnRunner` waits, which already `destroyAndWait` (SIGTERM to process-wrapper / linux-sandbox, whose own kill delay applies) and clean the sandbox. blaze_exec `ActionExecutionException::is_interrupted` (set by `fromExecException` for `ExecException::Interrupted`) and blaze_sky_exec `SkyframeActionExecutor` (execute, discoverInputs): an interrupted action is rethrown as `InterruptedException`, not reported as `Executing genrule ... failed: interrupted`. | skyframe: `interrupted_function` now uses a thread-interruptible sleep (the Java test's `Thread.sleep`), plus `in_flight_function_is_interrupted_when_another_fails_fast`, `..._not_interrupted_by_an_error_with_keep_going`, `setting_the_interrupt_flag_interrupts_in_flight_functions`, `a_function_waiting_on_an_inline_dependency_is_still_interrupted`; blaze_shell 5 `thread_interrupt` tests; blaze_exec `an_interrupt_of_the_caller_reaches_a_wait_on_the_evaluation_stack`. End to end, new `scripts/w4/interrupt-probe` (oracle vs Blaze): `sigint`, `sigterm`, `sigint-twice`: exit 8 within 5 s, no `sleep` left, same lines as the oracle except the known global ones (`INFO: Found 1 target...` position under Skymeld, `Critical Path`); `fail-fast` (`//:bad` fails while `//:slow` sleeps 30 s): both stop in 5-25 s from a cold start, same lines; `fail-fast-keep`: both wait for `//:slow` (>= 25 s). |
| R2 (S1) no SIGINT handler without --batch | FIXED | blaze_client `signal_handler.rs`: port of `SignalHandler::Install` / `handler` / `PropagateSignalOrExit` (blaze_util_posix.cc:144-229) and the `BlazeServer` cancel thread (blaze.cc:1842; `CancelThreadAction`), installed in `run_client_server_mode` before `Communicate` as blaze.cc:1254 does. SIGINT 1 and 2 and SIGTERM print Bazel's lines and cancel; SIGINT 3 prints the "third interrupt signal; server killed" line and `_exit(1)`s (the server is this process); SIGPIPE is recorded and re-raised at exit; SIGQUIT prints its line (no JVM to dump; not re-sent to ourselves). `InProcessServer::canceller` (crates/blaze/src/main.rs: `interrupt_command_thread`, i.e. `CommandManager.doCancel`). blaze_runtime `server_main_single_command` and blaze_shell `set_start_children_in_new_session`: the in-process server's children start with `setsid`, as the daemonized server's do (daemonize.cc:270), so a terminal Ctrl-C reaches only the client. `--batch` is unchanged (`captureSigint`; children share the terminal, as with Bazel's exec'd JVM). | blaze_client `signal_handler_test` (SIGINT and SIGTERM cancel; a panicking command does not hang the cancel thread); blaze_shell `children_start_in_a_new_session_only_when_asked`. interrupt-probe `sigint` (SIGINT to the client's process group, like a terminal): oracle and Blaze both print `Bazel caught interrupt signal; cancelling pending invocation.` and `ERROR: build interrupted`, exit 8; `sigint-batch`: both print `Bazel received an interrupt` and, because linux-sandbox shares the terminal's group in batch mode, the same `Executing genrule //:slow failed: (Interrupt): ...` line, exit 8. Third Ctrl-C not probed (the command stops after the first). |
| R3 (S2) cquery/aquery do not check strategies | NOT DONE (files not mine) | Needs blaze_cquery and blaze_runtime `commands/{cquery,aquery}.rs`; change described in the report to root (below). | Oracle, `cquery`/`aquery --spawn_strategy=bogus //:g`: `INFO: Analyzed target`, `INFO: Found 1 target...`, `ERROR: 'bogus' was requested for explicit default strategies but no strategy with that identifier was registered. Valid values are: [...]`, `INFO: Elapsed time`, `INFO: 0 processes.`, `ERROR: Build did NOT complete successfully`, exit 2, no query output. Blaze: cquery prints the result, exit 0. |
| R5 (S2) 256 MB action threads scale with --jobs | FIXED | blaze_shell `java_process.rs`: children start with glibc `posix_spawn` (`CLONE_VM | CLONE_VFORK`), like the JDK 21 default launch mechanism (`POSIX_SPAWN`), instead of std's `fork` + `pre_exec`. `childproc.c`'s steps are file actions (dup2 of stdin/stdout/stderr, `chdir`, `closefrom(3)`), the signal mask is emptied and SIGPIPE reset (what std did); `JDK_execvpe`'s PATH search and ENOEXEC `/bin/sh` fallback run in the parent, one spawn per candidate, so the same program and the same errno result. Reaping is `waitpid` on the pid (pidfd still used to wait). Why this and not a pool: a spawn no longer copies or accounts the parent's address space, so neither the 256 MB action threads nor the evaluator's 16 MB threads can make it fail, at any `--jobs`; a pool sized apart from `--jobs` would cap concurrent actions (also remote ones, which wait on the same thread). | Measured on this machine (C, 256 MB stacks): `fork` fails with ENOMEM at 200, 400 and 800 parked threads, `posix_spawn` succeeds at 800. blaze_shell `children_start_while_hundreds_of_large_stacks_are_reserved` (400 x 256 MB, then a spawn). interrupt-probe `jobs400` (`build --jobs=400` of 400 genrules): oracle and Blaze exit 0, `401 processes: 1 internal, 400 linux-sandbox.`, `Build completed successfully, 401 total actions`. All blaze_shell process tests (exec errors, PATH search, redirections, timeouts, signals) pass unchanged. |
| R6 (S2) input-set memo ignores Order | FIXED | blaze_exec `analysis/actions/mod.rs`: the memo key is `(node, order)`; the lock is held only for the lookup and the publish (first publisher wins, so sharing is kept), not across the conversion (the S4). | `sets_that_share_a_node_keep_their_own_order` (empty sets of 4 orders, singletons of 3, a compile-order set wrapping a stable one) plus the existing sharing test. |
| discover_inputs stack | CHECKED, no change | The only ported `discoverInputs` is `CppCompileAction`'s (blaze_rules): compiler options from the feature configuration, C++20 module files, include path checks. No Starlark callable runs there (no `call_frozen`/`map_each` in blaze_rules/src/cpp or blaze_cc_features). `StarlarkAction.discoverInputs` is not ported (the default panics loudly). | - |

## Public interrupt API (for blaze_remote)

On the thread that runs a spawn runner (the evaluation-stack thread of `Action.execute`; it
stands in for the Skyframe worker):

- `blaze_shell::thread_interrupt::is_current_interrupted()`: `Thread.currentThread().isInterrupted()`;
  true once the action's evaluator thread was interrupted (Ctrl-C/cancel, or the build stopping on
  a fail-fast error). `interrupted()` is Java's `Thread.interrupted()` (reads and clears).
- `blaze_shell::ThreadInterrupt::current().add_interrupt_listener(Arc<dyn Fn() + Send + Sync>)`
  returns an `InterruptRegistration` (unregisters on drop). The listener runs on the interrupting
  thread right after the flag is set (and at once if the thread is already interrupted); it must
  not block. `blaze_shell::thread_interrupt::with_interrupt_listener(listener, || ...)` scopes it.
  Use it to cancel an in-flight gRPC call or wake a wait that is not blaze_shell's; then throw
  `InterruptedException` (`ExecException::Interrupted`) as Java does.
- A thread the runner starts itself can share the status with `ThreadInterrupt::adopt()`.

## Evidence

Unit tests (`scripts/cargo-capped test -p <crate>`, tree above): blaze_shell 97 passed, 0 failed,
10 ignored; blaze_skyframe 204 / 0 / 124; blaze_client 146 / 0 / 13; blaze_exec 401 / 0 / 73;
blaze_sky_exec 42 / 0 / 11. `clippy --no-deps --all-targets -D warnings` on blaze_shell,
blaze_skyframe, blaze_client, blaze_exec, blaze_sky_exec, blaze: clean. `rustfmt --edition 2024`
on every changed file. Probe: `scripts/w4/interrupt-probe both <scenario>` (help: `interrupt-probe
help`), scenarios sigint, sigint-batch, sigterm, sigint-twice, fail-fast, fail-fast-keep, jobs400;
every remaining difference is `INFO: Found N targets...` (Skymeld position) or `Critical Path`
(not ported), plus the JVM's `-Xverify:none` warning in batch mode.

## Open after round 3

- R3 (needs blaze_cquery and blaze_runtime cquery/aquery owners).
- R4 unchanged (strategy error after analysis; needs the blaze_sky_top hook).
- Third Ctrl-C: Bazel's `KillServerProcess` kills the server's process group; in-process,
  `_exit(1)` ends the server and linux-sandbox (PR_SET_PDEATHSIG), but a process-wrapper started in
  its own session outlives the client until its child ends.
- `INFO: Found N targets...` is printed before execution; with Skymeld Bazel prints it after the
  evaluation (also absent when interrupted). blaze_sky_top.
