# Review: rewinding (action rewinding), first round

- Commit under review: dad60f0 (checkpoint 42), read from `git archive dad60f0`, not from the
  working tree.
- Range: new since 374c631, limited to rewinding, in `crates/blaze_skyframe/src`,
  `crates/blaze_sky_exec/src`, `crates/blaze_exec/src/exec/rewound_action_synchronizer.rs`,
  `crates/blaze_remote/src` and `crates/blaze_runtime/src` (dispatcher retry).
- Reviewer: read-only. No Cargo, no probe (a lost-input probe needs a cache that evicts on
  demand; none was set up). Every finding below is from reading the code against the Java.
- Verdict: **STOP**, bound to dad60f0. Blocking: RW-1, RW-2.

## Scope read

Against the pinned Java, line by line:

- `blaze_skyframe/src/reset.rs` (whole file); `evaluator.rs:1813-2047` (`evaluate_node`,
  `evaluate_node_once`, `dirty_rewind_graph_and_reset_entry`, `reset_entry`), `:2440-2530`
  (registering deps that are found undone), `:2625-2650`, `:2905-2930`; the rewinding hunks of
  `incremental_in_memory_node_entry.rs`, `non_incremental_in_memory_node_entry.rs`,
  `dirty_building_state.rs`, `node_entry.rs`; the unsuccessfully-rewound keys in `progress.rs`
  and `evaluator.rs:649-655`. Java: `AbstractParallelEvaluator.java:565-580, 735-770, 838-920,
  975-1000`, `IncrementalInMemoryNodeEntry.java:285-333, 390-411, 486-535`,
  `NonIncrementalInMemoryNodeEntry.java:116-156`, `AbstractInMemoryMemoizingEvaluator.java:120-135`,
  `DirtyAndInflightTrackingProgressReceiver.java:140-195`.
- `blaze_sky_exec/src/rewinding/action_rewind_strategy.rs:1-1290` (everything except the two
  nested-set walks and the tests) against `ActionRewindStrategy.java` (whole file).
- `skyframe_action_executor.rs:244-430` (`RewindSignal`, `ActionExecutionState::obsolete`),
  `:1040-1275` (`was_rewound`, `prepare_for_rewinding`, `prepare_dep_for_rewinding`,
  `rewinding_failed`), `:1414-1542` (`execute_action`), `:1630-1715` (the synchronizer's two
  guards). Java: `SkyframeActionExecutor.java:505-615`, `ActionExecutionState.java:195-240`.
- `action_execution_function.rs:536-699` (`handle_lost_inputs` and its caller) against
  `ActionExecutionFunction.java:448-534`.
- `completion_function.rs:1196-1247, 1286-1392` against `CompletionFunction.java:385-490`.
- `blaze_remote/src/remote_rewound_action_synchronizer.rs:1-552` against
  `RemoteRewoundActionSynchronizer.java` (whole file); `lease_service.rs` against
  `LeaseService.java`; `concurrent_artifact_path_trie.rs` and
  `RemoteActionInputFetcher::handle_rewound_action_outputs` against their Java.
- `blaze_runtime/src/dispatcher.rs` retry (`:263-310, :895-912`) against
  `BlazeCommandDispatcher.java:286-320, 843-860`.

## Scope not read

`add_nested_set_chains_to_rewind_graph` / `add_nested_set_paths_to_rewind_graph` beyond their
first lines; `rewindable_graph_inconsistency_receiver.rs`; `artifact_function.rs` and
`build_driver_function.rs` hunks; the detection of lost inputs in the remote crate
(`remote_execution_service.rs`, `RemoteActionFileSystem.checkForLostInputs`);
`remote_important_output_handler.rs`; `reader_preferring_read_write_lock.rs`;
`cancellable_task.rs`; `ActionRewoundEvent` and the stats event's consumers; all tests (counted
only); the rest of the 14,884-line diff (chunking, HTTP cache, downloader: not rewinding).

## What was checked and found faithful

- **Which nodes a plan resets.** `prepare_rewind_plan` follows Java step for step: the failed
  key; for each lost input that is not the failed action's own output, the artifact keys of the
  direct deps that are it or own it (parent tree, runfiles tree, one more level of owner); for
  each such artifact its generating action, and for a template's tree also the expansion's
  actions; for each newly visited action that insensitively propagates its inputs, all its
  derived inputs and their generating actions, recursively, plus its nested sets; then the
  nested-set chains and the nested-set paths from the failed key. A lost own output re-checks
  the failed action itself. Missing dependencies are ignored, as Java does with
  `allowSkyframeRestarts() == false`. `deps_to_rewind` is filled at the same points.
- **Node reset.** `dirty_rewind_graph_and_reset_entry` marks the children in depth-first
  post-order, notes `PARENT_FORCE_REBUILD_OF_CHILD` and `RESET_REQUESTED`, and resets the entry.
  `markDirty(REWIND)` is a no-op on a node that is not done or is in error, consolidates the
  reverse deps of a done node, and keeps the rewound value of a non-incremental node.
  `resetEvaluationFromScratch` accumulates the reset deps and builds the two `Reset*` states
  with Java's fields. The evaluator unites reset deps with the old deps where Java does
  (registration with `checkIfDoneForDirtyReverseDep`, removal at commit).
- **Kept graph.** Rewound keys are tracked apart from dirty keys and removed only when they
  finish successfully; the next evaluation deletes the in-flight and the unsuccessfully rewound
  keys with their reverse closure, as Java.
- **Limit and text.** 20 losses per key and record; the two messages, the codes, the
  `prepareForRewinding` call before the lost-input failure, and the disabled and fallback texts
  equal Java's.
- **Executor state.** `obsolete`, `prepare_for_rewinding`, `prepare_dep_for_rewinding`,
  `was_rewound` and the action-cache eviction follow Java, including the owner and non-owner
  branches and the template case.
- **Synchronizer.** The remote synchronizer at this commit is a full port: one coarse lock until
  the first rewound action, then one reader-preferring lock per generating action key (weakly
  held), the write lock from preparation to the end of execution, read locks on the keys of all
  inputs during execution and during top-level output processing, and cancellation of the
  action's registered uploads with all of them awaited before an interrupt is passed on. Both
  guards are taken in `run_action` in Java's order and released when execution ends; neither is
  held while a thread waits for a Skyframe node. So Java's deadlock proof carries over (a wait
  edge follows a dependency), and a consumer's read lock keeps a rewound producer out of
  `prepare` (which deletes) until the consumer's execution is over. See RW-5 on the ruling.
- **Interrupt.** Lock waits are interruptible; a reset node that sees the interrupt stays in
  flight and is deleted before the next evaluation; an interrupted plan signals the waiting
  shared copies (`rewinding_failed`).
- **Remote side read.** `LeaseService`, the trie, `handle_rewound_action_outputs`, and the
  dispatcher's retry (attempt counter, the "retrying the build" line, the retry limit) follow
  Java.

## Findings

### S1

- **RW-1. A failed top-level output is stored as a persistent error.**
  Rust `blaze_sky_exec/src/completion_function.rs:1246`
  (`SkyFunctionException::new(exception, Transience::Persistent)` for the
  `TopLevelOutputException`). Java `CompletionFunction.java:479-481`:
  `CompletionFunctionException(TopLevelOutputException e) { super(e, Transience.TRANSIENT); }`.
  Input (server mode): `build //x --remote_cache=... --remote_download_toplevel` where staging
  a top-level output fails (an `ImportantOutputException` from a download error, lost outputs
  with `--norewind_lost_inputs` and no retries, or `LOST_OUTPUT_TOO_MANY_TIMES`), then the same
  `build //x` in the same server. Bazel invalidates the transient error and evaluates the
  completion again, so the output is fetched; Blaze keeps the completion node in error and
  reports the stored failure without trying. Not probed. Fix: `Transience::Transient` at that
  one site (the two other constructors stay persistent), with a test.
- **RW-2. A shared action's second node runs the action with stale inputs instead of
  restarting.** Rust `skyframe_action_executor.rs:1535-1538`: after the owner's lost inputs the
  waiting copy gets `RewindOutcome::Obsolete` and `continue`s the loop of `execute_action`,
  becomes the owner of a new state (`:1453-1462`) and calls `run_action` at once with the input
  metadata it collected before the loss. Java (`ActionExecutionState.java:87-143`,
  `Obsolete.get` → `scheduleRestart`) returns null: the node's `compute` starts over, the
  rewound dep is found undone (`UndonePreviouslyRequestedDeps`) and the copy waits for it to be
  rebuilt before it executes. Input: two nodes with a shareable action (the same action from a
  target and an aspect, or from two configurations), a consumer-side lost input of that action
  under `--rewind_lost_inputs`. Blaze executes the action one more time against the input that
  is still lost, which fails again, plans a second rewind from the copy's key (a second
  `ActionRewoundEvent`, `lostInputsCount` 2 instead of 1, a second count toward the limit of
  20), and only then waits; if the producer has already finished again by then it is rewound
  and run a second time. The `PORT` note at `:250-262` calls this Java's `scheduleRestart`; it
  is not. Fix: the copy's `compute` returns a restart of its own node (a `Reset::self_only`
  gives exactly Java's sequence in this engine) instead of looping inside `execute_action`.
  Not probed.

### S2

- **RW-3. `mayInsensitivelyPropagateInputs` is a list of four downcasts**
  (`action_rewind_strategy.rs`, `may_insensitively_propagate_inputs`: `SymlinkAction`,
  `SymlinkTreeAction`, `RunfilesTreeAction`, `SolibSymlinkAction`). These are Java's four
  overriders today, so it is right now; an action type added later that propagates inputs
  answers `false` silently, and its lost outputs are then "rewound" without their inputs. Fix:
  a method on the action trait with the default `false`.
- **RW-4. Waits of a shared copy are not interruptible.** `RewindSignal::wait` and `cell.wait()`
  (`skyframe_action_executor.rs:287, 1516`) block without an interrupt check, where Java's node
  is not blocked at all. The owner always signals (success, failure, interrupt), so this shows
  only if the owner's thread panics between its lost-inputs result and the signal; then the
  copy hangs and Ctrl-C does not end it.

### S3

- `RewindingTest` (37 cases, `blaze_sky_exec/tests/rewinding_test.rs`) is ignored for lack of an
  in-process build harness; the rewinding cases of `MemoizingEvaluatorTest` in
  `blaze_skyframe/src/tests/not_ported.rs:185-219` are ignored; four engine cases run
  (`rewinding_evaluator_test.rs`). No end-to-end run of a rewind exists (no oracle comparison).
- Not ported, marked: `bugReporter` reports, remote analysis caching
  (`allowSkyframeRestarts`), filesets as owners, `stateCache.invalidate` and
  `cancelExternalDeps`, `--experimental_remote_cache_lease_extension` (`unported!`).

### S4

- `evaluator.rs:3102` still says "resets are not ported"; `RewindGraph` iterates successors in
  insertion order where Guava's order is unspecified (no reported effect).

## Open items that do not block

- **RW-5. The ruling of 2026-11-26 in `docs/decisions.md` (a rewound action "runs alone" under
  one write lock and waits for its uploads) does not describe this commit.** The code switches
  to per-action locks and cancels the uploads, as Java. Nothing to fix in the code; root may
  append a line that the deviation is withdrawn. The questions asked about the coarse lock
  (deadlock, read during delete) were answered for the code as it is, above.
- RW-3, RW-4; a probe with a cache that drops one blob (lost input) and one top-level output,
  on both sides, once such a tool exists under `scripts/`.

## Verdict

**STOP** at dad60f0. Blocking: RW-1 (top-level output failure stored as persistent, Java
transient), RW-2 (a shared action's second node re-executes with stale inputs instead of
restarting its node).
