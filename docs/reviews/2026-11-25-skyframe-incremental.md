# Review: incremental evaluator state machine (`blaze_skyframe`, package I1)

Date 2026-11-25. Reviewer: Fable 5.2, read-only. Commit under review: **e8b79c0** (code of
cc6c29a; `git diff e8b79c0 -- crates/blaze_skyframe` is empty). Baseline of the earlier review:
ef66f55 (`docs/reviews/2026-11-18-skyframe.md`).

`J:` = `~/.cache/blaze/src/bazel-9.3.2/src/main/java/com/google/devtools/build/skyframe/`,
`JT:` the same under `src/test/java`. Rust paths are relative to `crates/blaze_skyframe/src/`.

**Verdict: PASS**, bound to e8b79c0. No S1. Two S2, neither blocking. The verdict rests on reading
the Rust against the Java; I ran no test (see "Evidence I could not check").

## Scope

Read in full, against the Java named:

| Rust | Java |
| --- | --- |
| `incremental_in_memory_node_entry.rs` | `J:IncrementalInMemoryNodeEntry.java` (all but the reset states) |
| `dirty_building_state.rs` | `J:DirtyBuildingState.java`, `J:InitialBuildingState.java` |
| `abstract_in_memory_node_entry.rs` (trait) | `J:AbstractInMemoryNodeEntry.java` |
| `reverse_deps_utility.rs` | `J:ReverseDepsUtility.java` |
| `value_with_metadata.rs`, `version.rs`, `grouped_deps.rs` | `J:ValueWithMetadata.java`, `J:Version.java`, `J:IntVersion.java`, `J:NodeVersion.java`, `J:GroupedDeps.java` (equality, remove) |
| `in_memory_graph.rs`, `differencer.rs`, `invalidation.rs` | `J:InvalidatingNodeVisitor.java`, `J:SequencedRecordingDifferencer.java`, `J:RecordingDifferencer.java` |
| `evaluator.rs` (all 3,572 lines) | `J:AbstractInMemoryMemoizingEvaluator.java`, `J:ParallelEvaluator.java`, `J:AbstractParallelEvaluator.java` |
| `environment.rs` lines 30-1028 | `J:SkyFunctionEnvironment.java` 160-300, 300-415, 415-460, 460-665, 845-1016 |
| `simple_cycle_detector.rs` 120-633 | `J:SimpleCycleDetector.java` 112-553 |
| `progress.rs` 280-560 (`DirtyAndInflightTrackingProgressReceiver`) | `J:DirtyAndInflightTrackingProgressReceiver.java` |
| `lib.rs` (crate docs, `Key`, `DynKey`) | `J:DirtyBuildingState.java:214` |

Tests: 30 ported tests compared line by line with their Java source (list in "Tests"); the helper
`fail_build_and_remove_value`; the contract of `tests/model.rs` and the imports of
`tests/model/{spec,graph,scratch,error_model}.rs`; `docs/skyframe-incremental.md` and the five
notes under `docs/skyframe-incremental.d/`.

Not read: `non_incremental_in_memory_node_entry.rs`, `in_memory_node_entry.rs` and `node_entry.rs`
(the enum and trait that forward to the entries), so `keep_edges(false)` is reviewed only where
`evaluator.rs` branches on it; `progress.rs` outside the tracking receiver; `event_filter.rs`,
`graph_listener.rs`, `walkable_graph.rs`, `error_info.rs`, `cycle_info.rs`, `evaluation_result.rs`;
`simple_cycle_detector.rs` 1-120; the bodies of the model (`tests/model/graph.rs`, `driver.rs`,
`generate.rs`, `table.rs`); `docs/skyframe-incremental.d/i2-worklist.md`; every test not in the
sample. `J:InMemoryMemoizingEvaluator.java`, `J:EagerInvalidator.java`, `J:ErrorInfo.java`,
`J:InMemoryGraphImpl.java` were not opened (their ports are short or unchanged since ef66f55).

No Bazel or Blaze process was started: `root-suite-e8b79c0` was active throughout, and the area
is a library whose evidence is Cargo tests, which a reviewer does not run.

## What was checked, and the result

**Node-entry dirty lifecycle.** `markDirty`, `addReverseDepAndCheckIfDone`,
`checkIfDoneForDirtyReverseDep`, `getNextDirtyDirectDeps`, `signalDep`, `setValue` (change pruning
by `unchangedFromLastBuild`, version `NodeVersion.of(lastChanged, graphVersion)`), `markClean`
(its five checks in Java's order), `forceRebuild`, `getAllDirectDepsForIncompleteNode`,
`getAllReverseDepsForNodeBeingDeleted`: statement for statement the Java, with the same check
texts. `signalDep`'s split between entry and state (`dirty_building_state.rs:244`) keeps the
order `isEvaluating` check, increment, `isChanged` return, version comparison. Rewinding fails
with `unported!` at every entry point, including Java's no-op cases, which is stricter than Java
and honest.

**Reverse deps.** `ReverseDepsUtility` keeps the delayed-operation list, the bare-operation
encoding, the consolidation threshold and all consistency checks. The `Vec` replacing Java's bare
key / array / list is equivalent because every Java writer stores exactly one element bare, so
`len() == 1` is `raw instanceof SkyKey`. `swap_remove` reproduces `CompactHashSet.remove`.

**Reverse-dep bookkeeping in the blocking engine.** This is where the port departs from Java's
shape, so I traced every path that adds a dep to a node's temporary direct deps and every path
that registers the node on the dep, and looked for one without the other:

- `Environment::lookup` (`environment.rs:479`): done deps go to `pending` and are registered when
  the function returns or before the next request that blocks (`register_pending_deps`, groups
  first, then rdeps, then signals); a request with a not-done dep adds its group, registers the
  done ones, then `eval_dep` registers each not-done one before evaluating or waiting.
- `maybe_handle_dirty_node` (`evaluator.rs:1710`): the group is added, then every key goes
  through `eval_dep` with `checkIfDoneForDirtyReverseDep`; the no-keep-going child-error path adds
  the one dep, re-registers on it and removes the rdep from the rest of the group, as
  `J:AbstractParallelEvaluator.java:267-297`.
- `remove_undone_newly_requested_deps` (`:2128`): takes back both sides; for an old dep the rdep
  is left for `commit_and_get_parents` (old deps minus temporary deps), as in Java, and if the
  node then does not commit the dep is still in `getAllRemainingDirtyDirectDeps`, so deletion
  finds it.
- commit (`:2585`): `oldDeps − temporaryDirectDeps` lose the rdep before `setValue`, as
  `J:SkyFunctionEnvironment.java:960-993`.
- deletion (`invalidation.rs:204`): rdeps first (marked pending at once, as Java's synchronous
  part of `visit`), then direct deps filtered by visited/pending, done deps deferred to
  `removeReverseDepsFromDoneEntryDueToDeletion`.

I found no path that leaves a dangling rdep or a dep without the rdep. A signal reaches a parent
exactly once: a requester that locks the child before `setValue` is in the set `setValue`
returns and is signalled by the committer before the outcome is published; one that locks it
after sees `DONE` and signals itself (`evaluator.rs:1414`). The child's lock is released before
the parent's is taken on every path.

**Dirty check and change pruning in the engine.** Error-transience check, no-keep-going error
check, `VERIFIED_CLEAN` (replay, `evaluated(versionChanged = false)`, no signalling when the
clean node is the first error), `changePruned` only when the entry ends `DONE`, `oldDeps` from
`getAllRemainingDirtyDirectDeps`, previously requested deps prefetched into the environment so
that a re-request does not register twice: as Java. The window in which a parent can see a child
`DONE` with an error between the child's `markClean` and its `preventNewEvaluations` exists in
Java in the same place (`J:AbstractParallelEvaluator.java:350-365`); not a deviation.

**Errors across evaluations.** Fail-fast, keep-going, catastrophe (`should_fail_fast`), error
transience (`setError` with `checkIfDoneForDirtyReverseDep` when the key was an old dep,
`forceRebuild` plus rdep removal when it is newer), bubbling over a dirty parent
(`CHECK_DEPENDENCIES` → `signalDep(graphVersion)` → `markRebuilding`), cached errors at the roots:
as Java. In-flight nodes are deleted with their reverse closure at the start of the next
`evaluate` (`evaluator.rs:516`, `J:AbstractInMemoryMemoizingEvaluator.java:134`); every claim of a
node goes through `enqueue_evaluation`, so the in-flight set is complete.

**Deleting vs dirtying.** `delete` (dirty nodes and predicate), `deleteDirty` (version threshold
over the unenqueued dirty keys), `pruneInjectedValues` (equal value and no dep → dropped; deps →
dirtied instead; `keep_edges(false)` → removed), `injectValues` (`noDepsLastBuild` check): as
Java.

**Thread safety.** Java's `synchronized` entry is `Node::lock()`. The done snapshot is refreshed
under that lock at every change of doneness (all five sites: `evaluator.rs:665`, `:1926`, `:2686`,
`simple_cycle_detector.rs:426`, `invalidation.rs:495`). Compound sequences on an entry
(lifecycle read, then `markClean`) are made only by the owning thread; other threads only signal
or register. Lock order `failure` → node is kept. `outcome` and `cycle_rdeps` are cleared when a
node is claimed; a dirtied node drops its `outcome` in `refresh_snapshot`. Invalidation and
deletion run on the calling thread with no evaluation running.

## Findings

### S1

None.

### S2

**S2-1. A missing child of a dirty node is created instead of reported.** Not blocking.
`evaluator.rs:1369` (`eval_dep` calls `graph.create_if_absent` for every request), reached from
the dirty check (`:1877`) and from `environment.rs:512` for an old dep the function requests
again. Java looks the child up and fails:
`J:AbstractParallelEvaluator.java:393-400` (`handleKnownChildrenForDirtyNode`:
"Dirty parent had missing child (child=%s, parent=%s %s)") and `:1022-1028`
("Missing already declared dep %s (parent=%s)"). Input: a dirty node whose last-build dep is not
in the graph, evaluated with keep_going (without it the pre-check at `:1773` panics first, as
Java's does). Why masked: a consistent graph cannot be in that state (deletion takes the reverse
closure), and the fresh entry later fails in `consolidateDataAndReturnNewElements`
(`reverse_deps_utility.rs:334`, a `CHECK` on an initial build), so it is still a crash, with
another text and after the missing child's function has run. Fix expected: when
`requester.old_deps` contains the key, use `get_if_present` and panic with Java's text.

**S2-2. `evaluate` fails with `Interrupted` where Java returns cached results.** Not blocking;
the line predates this work (it is in ef66f55) but now sits after invalidation.
`evaluator.rs:547`: after the invalidation step, a set interrupt flag returns `Err(Interrupted)`
before `Run::eval`. Java has no interrupt check on the path where every root is done
(`J:ParallelEvaluator.java:641-665`), so with a pending interrupt, nothing to invalidate and
cached roots it returns the result. Input: `ev.interrupt()`, then `evaluate` of roots that are
all done. Why masked: callers treat an interrupt as fatal for the command and clear the flag.
Fix expected: drop the unconditional return and let `Run::eval` decide (it already returns
`Interrupted` when work was interrupted), or mark it `// PORT:` with the reason.

### S3 (honestly marked; the important ones)

- No key outside the crate opts into `Key::values_equal`, so change pruning never happens in a
  real build until I2: a dirtied node that is requested always re-runs, and so does everything
  above it that is requested. Results are right; the saving is not there yet.
- `Key::HERMETICITY` defaults to `None`, so "Nodes with hermetic functions cannot be marked
  'changed'" is not checked for any real key until I2 declares them.
- Invalidation is sequential and not interruptible; the four interruption cases of
  `EagerInvalidatorTest` (× two suites) and `interruptDuringRewind_parentAlreadySignaled` are
  ignored, as is `dirtyAndChangedValueIsChanged` (needs concurrent invalidation).
- Rewinding, `SkyFunction.Reset`, partial reevaluation, `SkyKeyComputeState`, `dependOnFuture`,
  `registerDependencies`, the per-key keep_going predicate, max transitive source versions,
  Skyfocus, `GraphInconsistencyReceiver` (always `THROWING`), `ShortCircuitingCycleDetector`: by
  ruling (docs/decisions.md 2026-11-19), each `unported!` or absent.
- Six tests ported with their body and ignored for the restart model; 35 stubs in
  `tests/not_ported.rs`; four interning tests of `InMemoryGraphTest`.

### S4

`PORTING.md` "Skyframe" still says `EvaluationProgressReceiver` and `EventFilter` are absent
(protected; root's item 9.7). The Rust line numbers in `engine.md` and section 5.3 have drifted
(for example `maybe_handle_dirty_node` is at `evaluator.rs:1710`, cited as `:1577`); names are
right.

## Decisions that depart from the Java, and whether each is sound

From `docs/skyframe-incremental.md` sections 5 and 9 and docs/decisions.md 2026-11-19.

| Decision | Sound? |
| --- | --- |
| D5: blocking engine drives Java's node entry; "enqueue and return" becomes "evaluate or wait, then signal" (section 2) | Yes. The entry sees Java's calls in Java's order; signalling is exactly once (above). |
| D10: value equality is `Key::values_equal`, default never equal; no identity shortcut (`value_with_metadata.rs:205`, `dirty_building_state.rs:287`) | Yes, conservative: it can only prune less than Java (a function returning the same object is pruned in Java, not here). It cannot prune where Java does not unless a key opts in wrongly, which is I2's burden. |
| 9.1: `HERMETICITY` is `Option`, `None` unchecked (`invalidation.rs:394`) | Yes as an interim: the check is an assertion, not behaviour. Until I2 a wrongly invalidated hermetic key goes unnoticed. |
| 9.2: default event filter `NO_STORAGE` | Yes for a graph that is discarded (Bazel's own choice there). With a kept graph it must change in I3, or the events of nodes that are not re-run are lost on the next command. |
| 9.3: in-flight nodes stay until the next `evaluate` | This is Java; the earlier behaviour was the deviation. |
| 9.4: `Evaluator::inject` deferred to the next `evaluate` | This is Java. `EvaluatorBuilder::inject` injecting at `build()` (`evaluator.rs:392`) does depart: sound, the version is the first evaluation's and nothing runs in between. |
| 9.5: invalidation sequential, on the calling thread, not interruptible | Sound for results: marking and deleting are idempotent per key and Java's order is arbitrary. Cost: an interrupt waits for the whole dirtied closure; measure before the kept graph is on by default. |
| 9.6: a thrower that is not the first error keeps its registrations on deps it saw not done (`evaluator.rs:2235`) | Yes. Both sides of each edge stay registered, the node stays in flight, and the next `evaluate` deletes it with its closure; within the evaluation bubbling skips it unless it is the only way up, where it re-runs and may throw. |
| Done snapshot, `owner`, `outcome`, `cycle_rdeps` on the node (`in_memory_graph.rs:148`) | Yes: refreshed or cleared at every transition, under the node's lock. |
| A dep that became done after the evaluation failed is shown as not done to a function that was waiting (`entry_seen_in_normal_evaluation`) | Yes: Java's parent is signalled but never re-enqueued; same end state. Kept from the reviewed engine and applied to dirty checks. |
| All requested deps done and the function returned null: commit child errors without a second run (`evaluator.rs:2468`) | Sound under invariant 7 of section 8, which I2 must hold functions to. |
| Reverse deps always a `Vec`; done deps not compressed; no interning | Yes, memory format only. |
| `MarkedDirtyResult` holds a copy of the rdeps | Yes: the visitor reads them once, and nothing evaluates during invalidation. |
| `analyzeDepsDoneness` is `NO_INFORMATION` | Yes: that is the in-memory graph's answer in Java. |
| Cached error keys in root order (Java `HashSet`) | Yes: the result is a map by key. |
| `BugReport.sendBugReport` sites in the cycle detector dropped | Yes: non-crashing reports. |
| `rdepsToBubbleUpTo` without requesters that closed a cycle through the node; `never_evaluated` | Kept from the reviewed engine. The stale-`cycle_rdeps` defect the model found is fixed at `evaluator.rs:1451`; I found no other per-evaluation field that survives a claim. |

## Tests

Thirty ported tests were compared with the Java source; all keep the Java's set-up, steps and
expectations. `containsExactly` on unordered collections is a set or sorted comparison; a
`WeakReference` cleared by the GC is an `Arc` whose weak count is checked.

- `MemoizingEvaluatorTest`: `changePruning`, `changePruningWithDoneValue`,
  `changePruningAfterParentPrunes`, `dirtyDepIgnoresChildren`, `deleteDirtyCleanedValue`,
  `failedDirtyBuild`, `failedDirtyBuildInBuilder`, `changedBuildAfterFailedThenSuccessfulBuild`,
  `incompleteDirectDepsAreClearedBeforeInvalidation`, `deleteOldNodesTest`, `transientPruning`,
  `dirtyThenDeleted`, `cycleAndSelfEdgeWithDirtyValue`, `nodeInvalidatedThenDoubleCycle`,
  `changePruningWithIntermittentEvent`, `removedNodeComesBackAndInvalidates`,
  `errorTransienceDepCleared`, `valueInjectionOverExistingEqualEntryMarkedForInvalidation`,
  `deleteInvalidatedValue`.
- `EagerInvalidatorTest`: `receiverWorks`, `receiverIsNotifiedAboutNodesInError`,
  `invalidateValuesNotInGraph`, `invalidatedValuesAreGCedAsExpected`, `reverseDepsConsistent`.
- `IncrementalInMemoryNodeEntryTest`: `pruneBeforeBuild`, `pruneAfterBuild`, `checkDepsOneByOne`,
  `pruneWhenDepGroupReordered`.
- `ParallelEvaluatorTest`: `incrementalCycleWithCatastropheAndFailedBubbleUp`,
  `runPartialResultOnInterruption` (latch count 2 instead of 3: one run of "fast" where Java has
  two; the assertions are Java's).

Differences seen, all sound: `changePruningWithDoneValue` omits Java's
`getOrCreate(mid, /*markAsModified=*/ false)` (a no-op for an existing node);
`runPartialResultOnInterruption` clears the recording receiver where Java installs a new one.

**The model test is independent of the implementation.** `tests/model/spec.rs`, `scratch.rs`,
`error_model.rs` import nothing from the crate; `graph.rs` imports only the `DirtyType` enum. The
model simulates Java's algorithm per node entry and its expected values do not come from the
evaluator; the driver also compares the incremental evaluator with a fresh one. The six narrowed
comparisons are disclosed with counts in `model-test.md`; none hides a value or error
comparison. Its stated gaps are real: no catastrophes, no `keep_edges(false)`, no hermetic keys,
interrupts only from inside a function, cycle content compared only as "a real cycle" when the
incremental and the fresh evaluator disagree.

## Evidence I could not check

- I ran no Cargo, so the counts in `docs/skyframe-incremental.md` section 7 (426 passed, 0
  failed, 153 ignored; model seeds 0..32,000) are the implementer's. They were measured on
  `de5a476` plus uncommitted work, not on e8b79c0; the doc says so. The committed crate has 547
  `#[test]` and 147 `#[ignore]` attributes before macro expansion, which fits 579 tests, but that
  is not a run.
- The paths the model does not generate (catastrophe after a kept graph, `keep_edges(false)`,
  interrupt from another thread during a dirty check) are covered by my reading and by ported
  Java tests only.

## Open items (not blocking)

1. Re-measure the crate's tests at the commit that gets accepted and cite that tree; run the
   model's long mode once on it (item 9.9: pass the two `BLAZE_SKYFRAME_MODEL_*` variables
   through `scripts/cargo-capped`).
2. S2-1 and S2-2.
3. Add catastrophes and `keep_edges(false)` to the model generator, or a table test for each,
   before the kept graph is switched on.
4. I3 must set the storing event filter together with the kept graph (decision 9.2).
5. Measure sequential invalidation on a real dirtied closure before the default changes (9.5).
6. Root: update `PORTING.md` "Skyframe" (9.7).
7. A reviewer for I2 should read `non_incremental_in_memory_node_entry.rs` and `node_entry.rs`,
   which this round did not.

## Verdict

**PASS** at e8b79c0. Findings: S1 0, S2 2 (not blocking), S3 5 groups, S4 1.
