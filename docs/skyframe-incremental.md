# Incremental evaluation in `blaze_skyframe` (package I1)

Status: in progress. Written by the Track I lead as the work goes; the reviewer of I1 reads this
first. Plan: `docs/research/08-server-and-incrementality.md` sections 2, 4.4, 4.5, 5.2. Decisions:
`docs/decisions.md` 2026-11-19 (D5, D10). `J:` is
`~/.cache/blaze/src/bazel-9.3.2/src/main/java/com/google/devtools/build/skyframe/`.

Sections: 1 what I1 is; 2 how Java's node protocol maps onto the blocking engine; 3 public API
and compatibility; 4 modules and Java classes; 5 `// PORT:` deviations; 6 not ported; 7 tests;
8 invariants SkyFunctions must keep (input to I2); 9 decisions for root.

## 1. What I1 is

The blocking evaluator stays (D5). It gains Java's node-entry state machine, ported class by
class, and uses it as its node: graph versions, `IncrementalInMemoryNodeEntry` with
`DirtyBuildingState`, `ReverseDepsUtility`, `InvalidatingNodeVisitor` (dirtying and deleting),
the differencer (`inject`, `invalidate`), change pruning, stored events with replay, and the
progress receivers. Nothing outside `crates/blaze_skyframe` changes in I1, and every other crate
compiles unchanged and behaves as before when it does not use the new API (section 3).

## 2. Java's node protocol in the blocking engine

Java restarts a function when a dep is missing; Blaze blocks in the request (crate docs,
`src/lib.rs`). The node entry is Java's and is driven with Java's calls in Java's order; only
"enqueue and return, be re-enqueued when the last dep signals" becomes "evaluate the deps on
this thread (or wait for the thread that has them), then signal".

| Java (restarting) | Blaze (blocking) |
| --- | --- |
| `graph.createIfAbsentBatch` + `addReverseDepAndCheckIfDone(parent)` / `checkIfDoneForDirtyReverseDep(parent)` returns `DONE`, `ALREADY_EVALUATING` or `NEEDS_SCHEDULING` (`J:IncrementalInMemoryNodeEntry.java:166-192, 221-229`) | the same call, made by the requesting thread. `DONE`: read the value. `ALREADY_EVALUATING`: wait for the owner (after the wait-for-graph cycle check). `NEEDS_SCHEDULING`: this thread owns the node and evaluates it now (inline, or on a fresh thread past `max_inline_depth`) |
| `Evaluate.run`: `maybeHandleDirtyNode` (`J:AbstractParallelEvaluator.java:233-380`) | the same loop on the owning thread. Per dep group of the last build: error-transience check, the no-keep-going error check, `addTemporaryDirectDepGroup`, then the group is evaluated with the code path of `get_values_and_exceptions` (`Run::eval_keys`) and each done dep is signalled with its version (`signalDep`) |
| a dep that never signals (aborting evaluation, cycle, interrupt): the node stays in flight | the node ends "undone" for this evaluation (`DoneKind::Aborted`, `InCycle`, `Interrupted`); its entry stays not done with its temporary direct deps |
| `VERIFIED_CLEAN`: `markClean`, replay events, `evaluated(versionChanged = false)`, signal parents with the unchanged version | the same, on the owning thread; waiters wake with the old value |
| `NEEDS_REBUILDING`: `markRebuilding`, environment created with the temporary direct deps (the groups already checked) as previously requested deps and `getAllRemainingDirtyDirectDeps` as `oldDeps`, function runs | the same; the function runs once and its requests block |
| commit: `commitAndGetParents` (`J:SkyFunctionEnvironment.java:916-1016`): events, `removeReverseDep` on old deps not requested again, `setValue(value, graphVersion)` with change pruning, `evaluated(...)` | the same |
| in-flight nodes at the end of a failed or interrupted evaluation stay in the graph and are deleted, with their reverse transitive closure, at the start of the next `evaluate` (`J:AbstractInMemoryMemoizingEvaluator.java:134-135`, `EagerInvalidator.delete`, `traverseGraph = keepEdges`) | the same (this replaces `Run::cleanup`, which dropped them at the end of the call) |
| error bubbling over a dirty parent (`J:ParallelEvaluator.java:395-403`) | the same |

The two differences the crate already documents for normal evaluation apply to dirty checks as
well: requests are evaluated when made, and under no-keep-going a dep whose error stopped the
evaluation is seen as "not done" by a function that was waiting for it.

## 3. Public API and compatibility

Rules: no existing signature changes; no new field on `EvaluationContext` (other crates build
it with struct literals); new trait items have defaults.

New, all additive:

- `Key::values_equal(old, new) -> bool`, default `false` (D10: Java identity equality or
  `NotComparableSkyValue`). Used only by `DirtyBuildingState.unchangedFromLastBuild` and
  `pruneInjectedValues`. Reached through `DynKey` so that it is found from a `SkyKey`.
- `Key::HERMETICITY: Option<FunctionHermeticity>`, default `None` ("not declared yet").
  `DirtyingNodeVisitor`'s check "Nodes with hermetic functions cannot be marked 'changed'"
  applies only to keys that declare `Some(Hermetic)`. I2 declares every key; see section 9.
- `EvaluatorBuilder::keep_edges(bool)` (default `true`: what Blaze stored before and what Bazel
  does by default), `event_filter(...)` (default `EventFilter::NO_STORAGE`: events are reported
  when the node commits and not stored, which is what the crate did before I1 and what Bazel
  does without incremental state, `J:../lib/skyframe/SequencedSkyframeExecutor.java:227`),
  `progress_receiver(...)` (default none).
- `Evaluator::invalidate(keys)`, `delete(predicate)`, `delete_dirty(version_age_limit)`,
  `get_values`, `get_done_values`, `emitted_event_state()` (with `clear()`).
- `Evaluator::inject` and `invalidate_transient_errors` keep their signatures and become Java's
  `Injectable.inject` and `RecordingDifferencer.invalidateTransientErrors`: recorded, applied at
  the start of the next `evaluate`. Values given to `EvaluatorBuilder::inject` are in the graph
  when `build()` returns, as before.

Behaviour for existing callers: with the defaults nothing is comparable, so a node dirtied by
an injection is rebuilt when requested and so is everything above it that is requested: the same
functions run as when the closure was deleted. (To be confirmed by the regression guard.)

## 4. Modules and Java classes

Filled in by each package part; one row per Java class: Rust file, Java file and lines, state
(ported / partly / not ported), tests.

(to be filled)

## 5. `// PORT:` deviations

(to be filled; each with the reason and the Java lines it deviates from)

## 6. Not ported

Rewinding (`DirtyType.REWIND`, `resetEvaluationFromScratch`, `SkyFunction.Reset`), partial
reevaluation, `SkyKeyComputeState`, `dependOnFuture`, `registerDependencies`, the per-key
keep_going predicate, `GraphInconsistencyReceiver`, max transitive source versions, Skyfocus.
Each site fails with `unported!` or is marked; list with file and line to be filled.

## 7. Tests

(to be filled: per Java test class ported / passing / ignored with reason; the model test)

## 8. Invariants SkyFunctions must keep

(to be filled; starts from plan section 2.5, G1 to G7)

## 9. Decisions for root

(to be filled)
