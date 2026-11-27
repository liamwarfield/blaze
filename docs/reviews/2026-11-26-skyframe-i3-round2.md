# Review, round 2: incremental builds, package I3 (kept graph, invalidation, `--watchfs`)

Reviewer role: read-only reviewer (Fable 5.2), 2026-11-26. Commit under review: **38cd662**
(checkpoint 41), read from `git archive 38cd662` (the working tree has later edits; not read).
Round 1: `2026-11-26-skyframe-i3.md` (STOP at ce81d5a). Response:
`2026-11-26-skyframe-i3-response.md`. `J:` =
`~/.cache/blaze/src/bazel-9.3.2/src/main/java/com/google/devtools/build/lib/`. No Cargo.
**No probe was run**: the unit `root-suite-5f68157` was active for the whole review. Every
finding is from reading the Rust against the Java (and the JDK's watch service from memory);
none was reproduced.

## Verdict

**STOP** at 38cd662. S1 3 (all blocking), S2 5, S3 2 (noted), S4 1.

Blocking:

- **S1-A** The per-command sync is keyed on the command id, which the user sets
  (`--invocation_id`, `BAZEL_INTERNAL_INVOCATION_ID`): two consecutive commands with the same id
  skip the whole sync (no file check, no fail-safe), so an edited source is not rebuilt.
- **S1-B** `--watchfs`: `put_key` is not Guava's `BiMap.put`; after a directory with a
  subdirectory is renamed, a stale reverse entry lets a later event cancel the watch of the
  moved subdirectory (edits there are never seen) or crash the server.
- **S1-C** `--watchfs`: on a new graph the diff view is marked processed without Java's check
  of the nodes that the `.bazelignore` / REPO.bazel evaluation just created; a write in that
  window (milliseconds) is never seen by a later `--watchfs` build.

What round 1 blocked on: the fail-safe is complete except through S1-A; the ports of S1-1 and
S1-2 read as the Java. They stay behind the fail-safe until the sequences in section 2 match.

## Scope read

- `blaze_sky_top/src/incremental.rs` (all), the diff of `skyframe_executor.rs`,
  `skyframe_build_view.rs` (conflict block), `incremental_package_roots.rs` (lookup path),
  `tests/kept_graph_references_test.rs`.
- `blaze_sky_loading`: `diff_awareness.rs` (all), `watch_service.rs` (all),
  `filesystem_value_checker_inferring_ancestors.rs` (all code), `repo_file.rs` and
  `ignored_subdirectories.rs` (dependency requests).
- `blaze_remote`: `remote_module.rs` (last checker, hooks), `remote_output_checker.rs:580-680`,
  `remote_important_output_handler.rs` (diff). `blaze_sky_exec/src/output_checker.rs` (diff).
  `blaze_exec/src/actions/file_artifact_value.rs` (diff).
- `blaze_runtime`: `commands/query.rs` (executor construction), `commands/build.rs` (salt call
  points, package roots), `commands/build_remote.rs` (hooks, end of command),
  `dispatcher.rs:670-720,985-1090`, `command_environment.rs` (command id, keep decision).
- `blaze_skyframe` diff (statistics, `Tagged`), `blaze_sky_analysis` and `blaze_build_config`
  diffs, `blaze_analysis/src/analysis_services.rs` (workspace status value).
- Java: `RemoteModule` (`beforeCommand`, `setup`, `afterCommand`, `computeActionExecutionSalt`),
  `RemoteOutputChecker`, `ExecutionTool.prepareForExecution`,
  `SkyframeExecutor` (`handleDiffs`, `handleDiffsWithMissingDiffInformation`,
  `setSaltAndDeleteActionsIfChanged`, conflict filtering, `makeWorkspaceStatusAction`),
  `WorkspaceStatusFunction`, `BazelWorkspaceStatusModule`, `DiffAwarenessManager`,
  `WatchServiceDiffAwareness`, `FileSystemValueCheckerInferringAncestors`,
  `CommandEnvironment.computeCommandId`, `RepoFileFunction`.
- `scripts/incremental/remote-seq` (head only).

## Findings

### S1

**S1-A. The sync of a command is skipped when the command id repeats.**

- Rust: `blaze_sky_top/src/incremental.rs:595-603` (`first_in_command =
  inner.synced_command != command.command_id`), `:627` (the fail-safe compares
  `kept.command_id`), `:750` (`decided_command`), `:870` (`outputs_checked_command`). The id is
  `CommandEnvironment::compute_command_id` (`blaze_runtime/src/command_environment.rs:553-590`):
  `--invocation_id`, else `BAZEL_INTERNAL_INVOCATION_ID` of the client environment, else random.
  Nothing clears `synced_command` at the end of a command.
- Java: `J:runtime/CommandEnvironment.java:607` computes the same id, but
  `SequencedSkyframeExecutor.sync` (`J:skyframe/SequencedSkyframeExecutor.java:262`) runs once
  per command call, whatever its id.
- Sequence (daemon, local build, no other flag):
  1. `build --invocation_id=11111111-1111-1111-1111-111111111111 //x`
  2. edit a source of `//x`
  3. the same command again.
  Step 3: `first_in_command` is false, `sync` goes straight to `sync_executor`: no
  `handle_diffs`, no environment injection, no deleted-packages check, no output check. Blaze
  reports `//x` up to date; Bazel rebuilds. A command without an executor in between (`info`,
  `version`) does not help. With `BAZEL_INTERNAL_INVOCATION_ID` exported in the shell every
  command has the same id.
- The same key bypasses the remote fail-safe (`kept.command_id != command.command_id` is
  false: a local build and a `--disk_cache` build with one id share the graph, with the
  unverified S1-1 and S1-2 ports live) and `decideKeepIncrementalState`
  (`--notrack_incremental_state` on the second command is ignored by the state).
- Present at ce81d5a (round 1 missed it); this round added two more uses of the key.
- Bazel's flag help says the caller must keep the id unique; Bazel's own correctness does not
  depend on it, so this is valid input with a different result.
- Fix expected: a token the server makes per command (a counter in the dispatcher), used for
  `synced_command`, `decided_command`, `outputs_checked_command` and `KeptGraph`; never the
  user's id.

**S1-B. `--watchfs`: the key and directory maps are not a bimap.**

- Rust: `blaze_sky_loading/src/diff_awareness.rs:557-566` `put_key`. When a key that is already
  bound gets a new directory, `watch_key_to_dir[key]` is replaced but the old
  `dir_to_watch_key[old dir]` entry stays.
- Java: `J:skyframe/WatchServiceDiffAwareness.java:329` `watchKeyToDirBiMap.put(key, path)`:
  `HashBiMap.put` on an existing key removes the old value from the inverse view.
- When it happens: inotify watches follow the inode, and the watch service returns the existing
  key for a directory that is already watched. After `mv a b`, the walk of `b` registers
  `b/sub` and gets the key that was bound to `a/sub`.
- Sequence, stale (workspace with `a/BUILD` using `a/sub/x.txt`; daemon; every build
  `build --watchfs //...`):
  1. build twice (the second has a baseline view).
  2. `mv a b`; build. Now `dir_to_watch_key["a/sub"]` still names the key of `b/sub`.
  3. `mkdir a`; build (registers `a`).
  4. `mkdir a/sub; rmdir a/sub`; build. The DELETE of `a/sub` finds the stale entry
     (`:618`), cancels that key (`inotify_rm_watch` on `b/sub`) and removes its mapping.
  5. edit `b/sub/x.txt`; build. No event arrives, the diff is complete and empty, the
     `FILE_STATE` node is not checked: Blaze reports up to date. Java: the inverse lookup in
     step 4 is null, nothing is cancelled, step 5 rebuilds.
- Sequence, crash: steps 1-2, then `mkdir -p a/sub`; build (`put_key(new key, "a/sub")` sees
  the stale entry and removes `watch_key_to_dir[key of b/sub]`, `:560-564`); edit
  `b/sub/x.txt`; build: `check_argument!(dir.is_some())` at `:589` fails
  (IllegalArgumentException, the server ends).
- Fix expected: `put_key` removes the reverse entry of the key's previous directory (and
  fails, as Guava does, when the directory is bound to another key); a test with a renamed
  directory tree.

**S1-C. `--watchfs` on a new graph: the nodes made before the view are never checked.**

- Rust: `incremental.rs:670-673` and `:1068-1092` `diff_awareness_on_new_graph`: `get_diffs`
  first evaluates `IGNORED_SUBDIRECTORIES` on the new graph (stats and reads `.bazelignore` and
  `REPO.bazel`, `ignored_subdirectories.rs:436-530`, `repo_file.rs:191-208`), then asks the
  diff awareness for its view, and marks the view processed. The comment says "the empty graph
  has nothing to invalidate"; the graph is not empty at that point.
- Java: `J:skyframe/SkyframeExecutor.java:3790-3830`: the same order, but a first view is
  `EVERYTHING_MODIFIED`, so `handleDiffsWithMissingDiffInformation` (`:3950`) re-stats every
  `FILE_STATE` node of the graph after the watches exist.
- Stale path: a write to `.bazelignore` or `REPO.bazel` after the evaluation read it and
  (new awareness) before `inotify_add_watch` of the root, or (awareness kept from an earlier
  command, graph discarded) before `collect_changes`, whose view then holds the event and is
  marked processed. The node keeps the old state and no later `--watchfs` command checks it.
- Every new graph of a first executor takes this path: server start, after `clean`, a
  signature change, and every command that the remote fail-safe gives a new graph.
- The window is milliseconds and I could not reproduce it without a hook; it is listed as S1
  by the rule of this review (a path that keeps a stale node). Root may rank it lower; the
  fix is small.
- Fix expected: run Java's handling on the new graph too (call `handle_diffs`, whose
  missing-diff check covers the few nodes), instead of only marking the view.

### S2

**S2-1. A dead weak reference answers "no root" silently.**
`skyframe_executor.rs:2267-2282` (`self.evaluator.upgrade()?`); the new test
`kept_graph_references_test.rs:36-40` pins that answer. In Java null means "package not
loaded". The weak reference itself is sound for the current command: the lookup is made from the
build's executor (`build.rs:1942`) and set on that executor's factory, and the evaluator lives
as long as the executor. But `build_reusing` makes a new `Arc` for every executor, so every
lookup of an earlier command is dead for good, and values of the kept graph hold the earlier
factory (`blaze_analysis/src/rule/analysis_environment.rs:56`) with its package roots (a cached
external-roots map and the dead fallback). I found no call from a kept value into that
factory's `resolve_source_artifact*`; the search was not exhaustive. Fix: fail loudly on a dead
reference (panic), and do not assert `None` in the test.

**S2-2. The S1-1 and S1-2 ports have no oracle evidence, and S1-2 is live without the
fail-safe.** The salt holds `--remote_default_exec_properties` also without any cache
(`J:remote/RemoteModule.java:1312-1332`), and `remote_configured` is false then: the deletion in
`incremental.rs:348-367` runs on a kept graph today. Reading: faithful (section 2). The local
sequence of section 2 must be run now, not at the lifting.

**S2-3. `BLAZE_INCREMENTAL_REMOTE_FAILSAFE=off` in the server's environment turns the ruled
fail-safe off** (`incremental.rs:160-165`), in the product binary, for any user whose
environment has it. Expected: remove it with the fail-safe, or accept it only together with
`BLAZE_INCREMENTAL_DEBUG`; say so in `decisions.md`.

**S2-4. Conflict-finding nodes: residual after the fix of round 1's S2-1.** The two deletions
are ported at Java's places (`skyframe_executor.rs:939-945,963-969`,
`skyframe_build_view.rs:402,457`). The functions still read a per-executor cell where Java
reads the precomputed `ACTION_CONFLICTS` (`J:skyframe/ActionLookupConflictFindingFunction.java:46`,
set at `J:skyframe/SkyframeExecutor.java:2548`). If the mini-phase is interrupted, the committed
`ACTION_LOOKUP_CONFLICT_FINDING` nodes stay in both; Java's are invalidated by the next
injection, Blaze's are read again by a later build with other conflicts. Needs an interrupt
inside the mini-phase and two different conflict sets; not blocking. Fix: delete both node
kinds before the evaluation as well.

**S2-5. A reset during a command is only applied at its end.** The generation counter is
correct (`incremental.rs:541-556,1343-1347`; no path keeps the graph). But inside the command
the executor goes on with its graph, where Java's `delete` of the `REPOSITORY_DIRECTORY` nodes
(`invalidate_repo_directories`, `build_remote.rs:343-360`) takes effect at the command's next
evaluation. That belongs to the remote repo contents cache (remote round 7, N1); noted here
because the hook is this package's.

### S3 (noted, not done, as the response says)

- Round 1's S2-5: the randomized test has none of the five edit kinds; it would not have found
  S1-A, S1-B or S1-C (it never repeats an id, never uses `--watchfs`, never renames a tree).
- `SequencedSkyframeExecutorTest`, the `getDirtyActionValues` tests, `BatchStat`.

### S4

- `i3-result.md` still describes the configuration key as its own node (correct for this
  commit, see section 4); the task text says otherwise.

## 1. The fail-safe

Complete, with S1-A as its one hole.

- The test is `RemoteOptions.is_remote_cache_enabled()` on the parsed options of the command
  (`incremental.rs:171-175`): rc files, `--config` and the client's default flags are in the
  parser; project files set build options only. It equals Java's
  (`J:remote/options/RemoteOptions.java:856`): `--remote_cache`, `--disk_cache` (non-empty),
  `--remote_executor`. Java's `setup` enables nothing outside it: HTTP and gRPC caches come
  from `--remote_cache`, the downloader and the output service are refused without a cache.
  Build Event Service flags and the repo contents cache startup flag do not trigger it; a
  cache-less command on the overlay file system resets the graph through
  `notify_no_cache_available`, and its predecessor with a cache had `kept.remote`.
- Every executor is created from `env.get_options()` (`commands/query.rs:215-251`; ten call
  sites, one options object per command), so all executors of a command agree on `remote`, and
  the graph a command leaves records it.
- The discard happens in `build_evaluator` before `build_reusing`, the pending deletions and
  the sync: nothing of the kept graph is read first.
- In between: a command without an executor leaves `kept.remote` in place; a command with one
  and without remote options (`query`, `cquery` without a cache) discards the remote graph and
  leaves a new one; the salt and the pending deletions are reset with every new graph
  (`reset_graph_bookkeeping`).
- Within one command the first executor already has a new graph, so the later executors hold
  no `ACTION_EXECUTION` or completion node of an earlier command; the salt of a new graph is
  unset and the output check sees no action. The sharing cannot hit S1-1 or S1-2.

## 2. S1-1 and S1-2 as ported

Read against the Java; no difference found.

- Last checker: `remote_module.rs:131-158,783-799,1216-1222`; set in `afterCommand`, cleared at
  the end of a command without one (`dispatcher.rs:1007`), as `J:remote/RemoteModule.java:1120`.
  The chain of earlier checkers grows as Java's does.
- `maybe_invalidate_skyframe_values` (`remote_output_checker.rs:590-609`) equals
  `J:remote/RemoteOutputChecker.java:382`; the deletion is queued and applied when the next
  executor reuses the graph (`incremental.rs:682-685`), before its sync. The module is set up
  before the build's executor is created, so the order holds.
- Materialization record: `file_artifact_value.rs` (shared flag), set at
  `remote_important_output_handler.rs:135-137`, read and cleared at `output_checker.rs:174-192`
  as `J:skyframe/FilesystemValueChecker.java:539-553`. It works only if the metadata the handler
  gets are clones of the graph's values; not traced.
- Salt: `build.rs:1313-1324`, called at `:1347` (merged, before analysis) and `:2449`
  (otherwise), as `J:buildtool/ExecutionTool.java:332`; deletion as
  `J:skyframe/SkyframeExecutor.java:2066`.

Sequences that must match the oracle (exit code, outputs present, process counts) before the
fail-safe is lifted, each on one server per side:

1. `remote-seq` s1-1-grpc, s1-1-disk, s1-1b-grpc, s1-1b-disk, s1-2 (as written).
2. Salt without a cache (run now, S2-2): `build --remote_default_exec_properties=a=1 //app`,
   then `a=2`: every action runs again.
3. Cache on and off: local build, `--disk_cache` build, local build; and the same with
   `--remote_cache`: all actions deleted at each change, no output missing at the end.
4. Command mode: `build`, `test`, `build` with `--remote_download_toplevel` (completion nodes
   deleted); `build`, `run`, `build` (not deleted).
5. A command without remote options in between: `build --remote_download_minimal //app`,
   `query //...`, `build --remote_download_toplevel //app` (last checker is null in Java: no
   deletion; compare what is downloaded).
6. s1-1 with a tree artifact output and with a runfiles tree; s1-1 where the top-level output
   is removed twice (the record is cleared after the first restore).
7. A failed remote build between two good ones.
8. s1-1 and s1-2 with the same `--invocation_id` on every command, after S1-A is fixed.

## 3. The Arc cycle and the weak reference

- The fix is right for the cycle: `DonePackageRootLookup` was the only strong path from the
  factory back to the evaluator. Dead reference: S2-1.
- Other strong holders of the evaluator at this commit: `SkyframeExecutor`,
  `SkyframePackageManager` (`package_manager.rs:156`; cloned into the cycle reporters and the
  query environments), `KeptGraph`. All belong to a command, none is stored in a value or a
  function I read. `BaseTargetPrerequisitesSupplier` holds it weakly and answers `None` when
  dead (same remark as S2-1; it is used only during its own executor's analysis).
- A holder that outlives its command makes `Arc::try_unwrap` fail and the graph is discarded
  (`incremental.rs:659-663`): slow, not stale, and visible only with `BLAZE_INCREMENTAL_DEBUG`.
- Not searched: reporters, event buses and action executors stored in values (a `git grep` for
  the evaluator and executor types only).

## 4. Other changes

- Evaluation statistics (`notify_command_complete`, `incremental.rs:514-522`,
  `dispatcher.rs:989-996`): metrics only, no effect on what is dirtied.
- `BUILD_INFO` without `BUILD_ID` (`skyframe_executor.rs:563-571`): as
  `J:skyframe/WorkspaceStatusFunction.java:38` (no dependency). The value holds the two
  artifacts and an action with user and host name only (`analysis_services.rs:135-151`); the
  status command and `--embed_label` are read at execution from the command's context. What
  re-runs a stamped action: the status action is volatile, so its `ACTION_EXECUTION` node
  depends on `BUILD_ID` (`action_execution_function.rs:426`) and runs every build;
  `stable-status.txt` changes by content; `volatile-status.txt` has constant metadata. As Java.
  The removed dependency had no purpose beyond re-running the function.
- "The configuration-key producer runs inside each configured target": **not in 38cd662**.
  `BUILD_CONFIGURATION_KEY` is still its own node (`blaze_build_config/src/skyframe.rs:47-50`),
  and the diff of `blaze_sky_analysis` and `blaze_build_config` in this range has no such
  change. Not reviewed; when it lands, each configured target must itself depend on
  `PLATFORM_MAPPING` and on what the producer reads.
- Conflict nodes: S2-4. Reset during a command: S2-5.
- `--notrack_incremental_state` without an executor (`command_environment.rs:479-489`,
  `incremental.rs:748-785`): as Java, except through S1-A.

## 5. `--watchfs`

Read line by line: `WatchServiceDiffAwareness`, `LocalDiffAwareness`, `DiffAwarenessManager`,
the watch service, `FileSystemValueCheckerInferringAncestors`, `handleDiffs` and both branches
of `handleDiffsWithMissingDiffInformation`. Differences: S1-B and S1-C only.

- Overflow: `IN_Q_OVERFLOW` signals `OVERFLOW` on every valid key
  (`watch_service.rs:388-401`), a key with 10,000 pending events is replaced by one `OVERFLOW`
  (`:104-118`); `collect_changes` throws, the awareness is closed and dropped, the entry is
  `EVERYTHING_MODIFIED` and the new awareness starts with a full check. As Java.
- Directories created later: registered in the walk of a CREATE event, before their children
  are listed, and their contents returned as changed. As Java.
- Renames: `IN_MOVED_FROM` is DELETE (the key of a moved directory is cancelled), `IN_MOVED_TO`
  is CREATE (walked). As Java, except S1-B.
- Symlinks: the walk does not follow them; a target outside the roots is an external file and
  is found by the graph scan that runs whenever `--experimental_check_external_repository_files`
  (default) or seen external files ask for it. As Java. `register` uses `lstat` where the JDK
  follows the link; this only turns a directory swapped for a symlink during the walk into a
  broken awareness (full check).
- Events between the view and the build's reads, and events the poller thread has not yet
  delivered when the view is taken: the same window as Java's (the JDK's poller is a thread
  too); a late event is in the next view.
- Changes while no command runs are queued by the kernel and the poller; a command without an
  executor does not consume them.
- First command of a server: new awareness, first view, baseline set: S1-C.
- `--watchfs` off after on: awareness closed and dropped, full check; on after off: new
  awareness, full check on the kept graph. A changed `.bazelignore` makes a new awareness
  (the old one stays open, as in Java).
- A failed sync (interrupt, stat failure) leaves the view unmarked; the next view is out of
  sequence and the entry is `EVERYTHING_MODIFIED`.
- `floor` over `RootedPath` in the deleted-directory sweep: I did not compare the Rust order of
  `RootedPath` with Java's string order.

## Not read

- `LocalDiffAwarenessTest`, `DiffAwarenessManagerTest`,
  `FileSystemValueCheckerInferringAncestorsTest` ports; `scripts/incremental/watchfs`,
  `reuse-check`, `stress`; the body of `remote-seq`.
- The JDK source (`LinuxWatchService`, `AbstractWatchKey`) is not on this machine; compared
  from memory.
- The rest of the range: rewinding, the remote repo contents cache, the Build Event Service
  client, `prepare_deps.rs`, `package_metrics.rs`, the query changes.
- `invalidation.rs`, `node_entry.rs`, `Evaluator::delete` (I1's ground), the completion
  functions, `ActionCacheChecker`, where the handler's metadata come from.
- The Rust `BazelWorkspaceStatusAction` body.
- No test and no probe was run. S1-A and S1-B are deterministic by the code and should
  reproduce at once; S1-C needs a hook.
