# Response: review of package I3 (2026-11-26-skyframe-i3.md, STOP at ce81d5a)

Implementer: task `i3` / `i3-b`, 2026-11-26. Tree: HEAD `bbbc6fa` plus uncommitted work (this
task's and other agents'). Binary of the probes: `target/debug/blaze` of that tree, copied to
`~/.cache/blaze/work/i3/bin/blaze18` (labels `blaze-i3-b18-daemon`). Nothing of this response is
committed. `J:` = `~/.cache/blaze/src/bazel-9.3.2/src/main/java/com/google/devtools/build/lib/`.

## Status per finding

| Finding | State | Where |
| --- | --- | --- |
| S1-1 remote output metadata | **fail safe in place**; the three Java mechanisms ported; not yet reproduced against the oracle (no remote worker run yet), so the fail safe stays | below |
| S1-2 execution salt | **ported** (`setSaltAndDeleteActionsIfChanged`); not yet reproduced against the oracle (needs the remote worker); the fail safe covers it meanwhile | below |
| S2-1 conflict-finding nodes | **fixed** (Java's two deletions) | `skyframe_executor.rs`, `skyframe_build_view.rs` |
| S2-2 `--watchfs` live, document wrong | **fixed**: ported with its JUnit tests and oracle evidence (`watchfs_test`, all 4 functions as the oracle); document corrected | `i3-result.md` |
| S2-3 reset during a command undone by `put_back` | **fixed** (graph generation) | `incremental.rs` |
| S2-4 server state not tied to its graph | **fixed** (generation; Blaze-only discards reset the analysis-cache bookkeeping, the salt and the pending deletions) | `incremental.rs` |
| S2-5 randomized test | **open** (not extended yet) | |
| S3 `SequencedSkyframeExecutorTest`, `getDirtyActionValues` tests, `BatchStat` | **open** | |
| S3 `--notrack_incremental_state` without an executor | **fixed** | `command_environment.rs`, `incremental.rs` |
| S3 arena growth | **measured**, proposal below (root rules) | |
| S4 two wrong statements, stale comments | **fixed** | `i3-result.md`, `incremental.rs`, `skyframe_executor.rs` |

## 0. Fail safe for remote options (root's item 0)

`SkyframeIncrementalState::build_evaluator` (crates/blaze_sky_top/src/incremental.rs): a kept graph
is not reused by a command that has a remote cache, a disk cache or remote execution configured
(`RemoteOptions.isRemoteCacheEnabled`, which includes the disk cache and remote execution), nor
by any command when the command that gave the graph back had one (the kept graph records it).
The server log gets, at INFO from `SequencedSkyframeExecutor.sync`:
"Skyframe graph of the previous command discarded: this command | the previous command has a
remote cache, a disk cache or remote execution configured (remote output invalidation is not
ported)". The executors of one command still pass the graph to each other (the dispatcher's
main-repository-mapping executor to the command's), so a command's own events are not reported
twice.

## S1-1

Ported, behind the fail safe:

- `RemoteModule.lastRemoteOutputChecker` (`J:remote/RemoteModule.java:159,573,1120`):
  crates/blaze_remote/src/remote_module.rs keeps the previous command's checker in a
  process-wide cell (Java's module lives with the server), set by `afterCommand` and cleared at
  the end of a command whose module recorded none (Java's `remoteOutputChecker = null`,
  `end_command_for_last_remote_output_checker`, called for every command from
  `build_remote.rs::reset_repo_helpers_and_overlay_fs`).
- `remoteOutputChecker.maybeInvalidateSkyframeValues(evaluator)` (`J:remote/RemoteModule.java:574`)
  now has its caller: the new hook `RemoteHooks::skyframe_delete_by_function_name` queues the
  deletion on the server state (`SkyframeIncrementalState::delete_by_function_name`), applied to
  the kept graph when the next executor reuses it (Java's `delete` is itself pending until the
  next evaluation).
- `wasMaterializedAsToplevelOutput` / `setMaterializedAsToplevelOutput`
  (`J:actions/FileArtifactValue.java:140,149,888,893,992,997,1352,1357`): a shared flag in
  `FileArtifactValue::RemoteFile` (crates/blaze_exec/src/actions/file_artifact_value.rs), set by
  `RemoteImportantOutputHandler` (`J:remote/RemoteImportantOutputHandler.java:178`) and read and
  cleared by the output check (`J:skyframe/FilesystemValueChecker.java:539-553`,
  crates/blaze_sky_exec/src/output_checker.rs).

The fail safe stays until both sequences of the review (and the disk cache variant) are run
against the oracle with Bazel's remote worker and added to the probe: not done in this round.
`i3-result.md` no longer says that missing remote outputs are always treated as modified.

## S1-2

`SkyframeExecutor.setSaltAndDeleteActionsIfChanged` (`J:skyframe/SkyframeExecutor.java:2066`):
`ExecutorHandle::set_salt_and_delete_actions_if_changed` (incremental.rs) with
`last_execution_salt` in the server state, called where `ExecutionTool.prepareForExecution`
(`J:buildtool/ExecutionTool.java:332`) calls it: before the analysis with merged analysis and
execution (`ExecutionToolConstruction::before_merged_analysis`), at the start of the execution
otherwise (crates/blaze_runtime/src/commands/build.rs). The salt is the executor's
(`RemoteModule.computeActionExecutionSalt`, "" without one). Not reproduced against the oracle
yet (see S1-1).

## S2-1

`filterActionConflictsForConfiguredTargetsAndAspects` deletes the
`TOP_LEVEL_ACTION_LOOKUP_CONFLICT_FINDING` nodes after its evaluation, and
`resetActionConflictsStoredInSkyframe` (new, `J:skyframe/SkyframeExecutor.java:2611`) deletes the
`ACTION_LOOKUP_CONFLICT_FINDING` nodes; `SkyframeBuildView` calls it on the no-keep-going path as
Java does, and once its post-processing is done on the keep-going path (Java's other caller,
`filterActionConflictsForTopLevelArtifacts`, is not ported; marked PORT).

## S2-3, S2-4

A generation counter in the server state: every graph handed to an executor gets the next
generation; `reset_evaluator` and `decideKeepIncrementalState`'s reset advance it; `put_back`
drops a graph whose generation is not the current one ("put_back: stale graph not kept" in the
debug output). A reset during a command (`invalidate_repo_directories`) therefore drops the
command's graph at its end, and two executors of one command alive at once cannot leave a
graph with the other's state. Every Blaze-only discard (signature, still referenced, remote, a
failed sync, no kept graph) resets the analysis-cache bookkeeping (`has_configuration`,
`original_configuration_options`, `skyframe_analysis_was_discarded`,
`last_analysis_discarded`, the top-level options, the main repository mapping), the execution
salt and the pending deletions (`reset_graph_bookkeeping`).

## S2-2: `--watchfs`

Ported: `DiffAwareness`, `BrokenDiffAwarenessException`, `WorkspaceInfoFromDiff(Receiver)`,
`DiffAwarenessManager`, `LocalDiffAwareness` (with its `Factory` and `SequentialView`),
`WatchServiceDiffAwareness` (crates/blaze_sky_loading/src/diff_awareness.rs) over a port of the
JDK's Linux watch service (crates/blaze_sky_loading/src/watch_service.rs: inotify, a poller
thread, the JDK's event coalescing and Bazel's `maxEventsPerPoll=10000`,
`J:../../../../main/cpp/blaze.cc:444`), `FileSystemValueCheckerInferringAncestors`
(crates/blaze_sky_loading/src/filesystem_value_checker_inferring_ancestors.rs),
`handleDiffsWithCompleteDiffInformation`, `getDiff`, Java's guard of
`handleDiffsWithMissingDiffInformation` and its external-files-only branch, the evaluation of
`IgnoredSubdirectoriesValue` in `handleDiffs`, `BazelDiffAwarenessModule.workspaceInit` with a
`WorkspaceBuilder` (diff awareness factories only), `resetEvaluator`'s `diffAwarenessManager.reset()`.

Tests: `DiffAwarenessManagerTest` 11/11, `LocalDiffAwarenessTest` 15/15 (real inotify),
`FileSystemValueCheckerInferringAncestorsTest` 20/20 (the `fsvcThreads` parameter: the port is
sequential, the single-thread expectations hold).

Oracle evidence: `scripts/incremental/watchfs run` replays the four functions of
`src/test/shell/integration/watchfs_test.sh` (oracle first, then Blaze in daemon mode): every
assertion holds on both sides, 0 differences; in `test_command` the profiler of the second
`--watchfs` build has no "VFS stat" of the unchanged source, as on the oracle.

## Found on the way (not in the review)

- **The kept graph was discarded after every command that executed actions** (the regression
  the remote agent reported): checkpoint 39's `IncrementalPackageRoots` holds
  `SkyframeExecutor::get_root_for_done_package()`, whose lookup held the evaluator strongly; the
  artifact factory keeps the package roots and graph values reach the factory: an `Arc` cycle
  (the evaluator was also leaked). The lookup now holds the evaluator weakly
  (`DonePackageRootLookup`, skyframe_executor.rs). Tests: `kept_graph_references_test`
  (unit) and `scripts/incremental/reuse-check` (a daemon's no-op rebuild must print
  "0 packages loaded, 0 targets configured" and "1 process: 1 internal.").
- **`builtValues` after `mod graph`** (referee): the evaluator's statistics move with the kept
  graph and only the build command reset them. `SkyframeExecutor.notifyCommandComplete`
  (`J:skyframe/SkyframeExecutor.java:1196`, called from `BlazeRuntime.afterCommand`) is now
  ported for every command (`SkyframeIncrementalState::notify_command_complete`, posted to the
  command's build event stream as Java's `SkyframeGraphStatsEvent`).
- **`BUILD_INFO` depended on `BUILD_ID`** (my I3 port): Java's `WorkspaceStatusFunction` has no
  dependency (`SkyFunctions.BUILD_INFO` is hermetic); the 27 configured targets that read it were
  analyzed again by every command. Fixed.

## Not done in this round

S2-5 (the five edit kinds, oracle comparison, C++ targets, other commands, remote and disk cache
flags), the S1 oracle sequences with the remote worker, `SequencedSkyframeExecutorTest`, the
`getDirtyActionValues` tests, `BatchStat`. They stay on the list in this order.

# Round 2 (2026-11-26-skyframe-i3-round2.md, STOP at 38cd662)

Implementer: task `i3-c`. Tree: HEAD `b2166031` plus uncommitted work. Binaries:
`~/.cache/blaze/work/i3/bin/blaze21` (before the round-2 fixes), `blaze24` (`target/debug/blaze`
of the tree with the round-2 fixes, before the server-log lines and the holder check below).
Sequences: `scripts/incremental/seq-probe` (new; oracle first, then Blaze, one server per side
and sequence; it replaces `remote-seq`, whose five sequences it contains). Nothing is committed.

| Finding | State |
| --- | --- |
| S1-A sync keyed on the user's id | **fixed**: a server token per command |
| S1-B `put_key` not a bimap | **fixed**: `HashBiMap.put`; two unit tests, two sequences |
| S1-C `--watchfs` on a new graph | **fixed**: `handle_diffs` runs on every graph |
| S2-1 dead weak reference silent | **fixed**: fails loudly; test asserts the panic |
| S2-2 salt and the eight groups | **run** (below); the fail-safe stays on, see "Lifting" |
| S2-3 switch in the product binary | **fixed**: debug builds only |
| S2-4 conflict cell | **fixed**: the `ACTION_CONFLICTS` precomputed |
| S2-5 reset inside a command | **fixed**: a targeted `delete` on the command's graph |
| S3 randomized test | **extended** (`scripts/incremental/fuzz`), run in progress |
| S4 configuration-key text | `i3-result.md` describes this tree (the node is its own); unchanged |

## S1-A

`IncrementalCommand.command_token` (crates/blaze_sky_top/src/incremental.rs): a token the server
allocates for each `CommandEnvironment` (`NEXT_INCREMENTAL_COMMAND_TOKEN`,
crates/blaze_runtime/src/command_environment.rs), which the user cannot set. `synced_command`,
`decided_command`, `outputs_checked_command` and `KeptGraph` hold it; the command id is no longer
read by the incremental state. Java syncs once per command call
(`J:skyframe/SequencedSkyframeExecutor.java:262`); the token is the port of "one call".

Oracle evidence (`seq-probe --group id`). The reviewer's expectation ("Bazel rebuilds") does not
hold at the pin: with the same `--invocation_id` and only a source edit, Bazel 9.3.2 prints
"Found 0 targets..." and runs nothing (the output stays stale). Java's sync does run (a BUILD
edit under the same id is analyzed again, sequence `id-reuse-build-edit`), but Skymeld's
`BuildDriverFunction` is re-evaluated only through its dependency on `BUILD_ID`
(`J:skyframe/BuildDriverFunction.java:163-165`), which an equal id leaves clean.

| Sequence | blaze21 (before) | blaze24 (after) |
| --- | --- | --- |
| `id-reuse-build-edit` (BUILD edit, same id) | stale output, "Analyzed  (0 packages ...)" | as the oracle |
| `id-reuse-notrack` (`--notrack_incremental_state` second) | graph kept, stale output | as the oracle |
| `id-reuse`, `id-reuse-env` (source edit, same id) | stale, other message text | rebuilds; the oracle stays stale |
| `id-reuse-disk` | graph shared with a disk-cache build | the fail-safe discards the graph (counts only) |

The remaining `id-reuse` difference belongs to the Skymeld port (decisions.md, 2026-11-26): Blaze
does not run its `BuildDriverFunction` (which has the `BUILD_ID` dependency) yet; with Skymeld
the same mechanism gives Bazel's result.

## S1-B

`WatchServiceDiffAwareness::put_key` (crates/blaze_sky_loading/src/diff_awareness.rs) follows
`HashBiMap.put`: an equal mapping is a no-op, a directory bound to another key fails
(`IllegalArgumentException: value already present`), a key's previous directory leaves the inverse
map. Tests (crates/blaze_sky_loading/src/tests/local_diff_awareness_test.rs):
`renamed_directory_keeps_watching_its_subdirectory` and
`recreated_path_of_renamed_directory_does_not_lose_the_moved_one`; both fail with the old
`put_key` (the crash and the lost event), 17/17 pass with the fix.

Oracle evidence (`seq-probe --group watchfs`, every build `build --watchfs -- //... -//app:bad`):
blaze21: `watchfs-stale` misses the edit of `b/sub/x.txt` (x-1 where the oracle has x-2),
`watchfs-crash` ends the server (panic at diff_awareness.rs:589); blaze24: 0 differences in the
three sequences (also `watchfs-recreate`, a rename back).

## S1-C

The new-graph path of `build_evaluator` calls `handle_diffs` (Java's order: the ignored
subdirectories are evaluated, then the diff awareness's view is taken; a first view is
`EVERYTHING_MODIFIED`, so the missing-diff check stats every node made before the watches).
`diff_awareness_on_new_graph` is removed. Not reproducible without a hook (the window is
milliseconds), as the reviewer says; the `--watchfs` sequences and the fuzz run go through it.

Found with it: Java logs "About to scan skyframe graph checking for filesystem nodes of types
[OUTPUT, EXTERNAL_REPO]" (and "About to scan %d external files") to the server log in
`handleDiffsWithMissingDiffInformation` (`J:skyframe/SkyframeExecutor.java:4022,4048`);
`check_external_files_test#test_check_all_flags_fast` counts it. Ported (the `EnumSet` text in
declaration order); the oracle prints one line after the first `--watchfs` build and none after
the second.

## S2-1

`DonePackageRootLookup` panics ("IllegalStateException: getRootForDonePackage(..) after the
command's Skyframe executor is gone"), as does the rewinding synchronizer's graph accessor;
`kept_graph_references_test` asserts the panic.

## S2-2: the salt and the eight groups

`seq-probe --group remote --failsafe off` with blaze24 (the kept graph is used with remote
options; the switch exists in debug builds only):

| Group | Sequences | Result |
| --- | --- | --- |
| 1 | s1-1-grpc, s1-1-disk, s1-1b-grpc, s1-1b-disk, s1-2 | as the oracle |
| 2 | salt-local (`--remote_default_exec_properties` a=1, a=1, a=2, no cache) | as the oracle |
| 3 | cache-onoff-disk, cache-onoff-grpc | outputs and process counts as the oracle; adding the cache: Blaze analyzes 6 packages / 10 targets, the oracle 2 / 4 (open) |
| 4 | mode-test, mode-run | as the oracle |
| 5 | no-remote-between | as the oracle |
| 6 | s1-1-tree, s1-1-runfiles, s1-1-twice | as the oracle |
| 7 | failed-between | as the oracle (the probe now normalises the workspace path) |
| 8 | s1-1-grpc-same-id, s1-2-same-id | outputs as the oracle; the oracle runs nothing on the repeated id (Skymeld, see S1-A) |

## S2-3

`remote_fail_safe_enabled()` reads `BLAZE_INCREMENTAL_REMOTE_FAILSAFE=off` only when
`cfg!(debug_assertions)`; a release binary always has the fail-safe.

## S2-4

`ACTION_CONFLICTS` is a `Precomputed<ActionConflictsValue>` (crates/blaze_sky_top/
action_lookup_conflict_finding_function.rs) injected by
`filter_action_conflicts_for_configured_targets_and_aspects` (`J:skyframe/SkyframeExecutor.java:2548`)
and read by `ActionLookupConflictFindingFunction` through Skyframe (`J:skyframe/
ActionLookupConflictFindingFunction.java:46`), so a new set of conflicts invalidates the verdicts
an interrupted mini-phase left. Equality is `ImmutableMap.equals` (actions by identity; the
exceptions by message, since the port copies them: PORT note). I did not add the deletion before
the evaluation the reviewer suggests: Java has none, and with the precomputed Java's own
mechanism (invalidation by injection) covers the interrupted case.

## S2-5

`SkyframeIncrementalState::delete` applies a `delete` to the graph the current executor holds
(recorded with `attach` as a weak reference when the executor is built), or else to the kept
graph. `invalidate_repo_directories` (crates/blaze_runtime/src/commands/build_remote.rs) now
deletes the `REPOSITORY_DIRECTORY` nodes of the named repos, as
`J:remote/RemoteExternalOverlayFileSystem.java:195`, instead of resetting the whole graph, and
`RemoteOutputChecker.maybeInvalidateSkyframeValues` goes through the same call (the pending
deletion list is gone).

## Holders of the evaluator (root, after checkpoint 42)

- `SkyframeIncrementalState::check_kept_graph_unshared`, called by the dispatcher after every
  command: once the command and its executors are gone, the kept graph must be the evaluator's
  only strong holder. A debug build panics (so any test that runs a command fails in the crate
  that adds a holder); a release build logs it to the server log.
- `crates/blaze_runtime/tests/kept_graph_unshared_test.rs`: builds that execute a genrule, a
  Starlark action, a tree artifact and a runfiles tree, a no-op build, an incremental build, a
  cquery and a failed build; after each, the kept evaluator's strong count is exactly the kept
  graph's (plus the test's clone).
- `scripts/incremental/reuse-check run --blaze B` (interface unchanged; exit 1 on any failure,
  errors of the tool included): scenario `basic` as before, and `cc-test` (a cc_binary with a
  library, a genrule running it, a Starlark test with a runfiles tree: build, test, build,
  test), with expectations taken from the oracle (the oracle passes the tool). blaze24 passes;
  blaze22 (the tree with the action executor's strong graph) fails both scenarios. A rewound
  action without a remote cache is not in it (rewinding needs lost inputs: remote or
  build-without-the-bytes outputs; `seq-probe` has those).

## Lifting the fail-safe

Not lifted yet. With it off, every sequence of the eight groups has the oracle's outputs, and
the differences left are counts: group 3 (adding a cache analyzes more in Blaze) and groups 1/8
through Skymeld. I propose to lift it once group 3 is explained, and to record the decision
with these sequences as its evidence.
