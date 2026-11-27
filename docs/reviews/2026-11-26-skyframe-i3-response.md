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
