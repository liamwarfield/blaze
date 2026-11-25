# I1 phase 2: the engine (worker I1-C)

Tree: worktree `i1-a`, base 48b61bb plus the uncommitted phase-1 work plus this work; nothing
committed. Files under `crates/blaze_skyframe/` only. `J:` = `src/main/java/com/google/devtools/
build/skyframe/`, `JT:` the same under `src/test/java`. Rust paths are relative to
`crates/blaze_skyframe/src/`; line numbers are as of this writing.

## (a) What was ported

| Java | Java file:lines | Rust | State |
| --- | --- | --- | --- |
| `InMemoryGraphImpl` (+ `EdgelessInMemoryGraphImpl`, `InMemoryGraph`) | `J:InMemoryGraphImpl.java:55-320`, `J:InMemoryGraph.java` | `in_memory_graph.rs` `InMemoryGraphImpl` (sharded map of `Arc<Node>`), `Node`, `NodeState` | ported; interning, `cleanupInterningPools`, `shrinkNodeMap`, `removeIfDone` not (memory / Skyfocus) |
| node as Java's `synchronized` entry | `J:InMemoryNodeEntry.java` | `in_memory_graph.rs` `Node::lock()` → `NodeState { entry, owner, snapshot, outcome, cycle_rdeps }` | blocking-engine state added (section b) |
| `AbstractInMemoryMemoizingEvaluator.evaluate`, `getNextGraphVersion`, `lastGraphVersion`, `keepEdges`, `pruneInjectedValues`, `injectValues`, `performInvalidation`, `delete`, `deleteDirty`, `getValues`, `getDoneValues`, `getExistingValue` | `J:AbstractInMemoryMemoizingEvaluator.java:60-503` | `evaluator.rs` `Evaluator::evaluate`, `get_next_graph_version`, `prune_injected_values`, `inject_values`, `perform_invalidation`, `delete`, `delete_dirty`, `get_values`, `get_done_values`, `get_existing_value`; `IncrementalState` | ported (Skyfocus, dump commands, stats not) |
| `InMemoryMemoizingEvaluator` (wrapping the receiver) | `J:InMemoryMemoizingEvaluator.java:147-180` | `EvaluatorBuilder::build` | ported; `ProgressReceiver` statistics not (logging) |
| `Differencer`, `Diff`, `DiffWithDelta.Delta`, `ImmutableDiff`, `RecordingDifferencer`, `SequencedRecordingDifferencer`, `Injectable` | `J:Differencer.java`, `J:ImmutableDiff.java`, `J:RecordingDifferencer.java`, `J:SequencedRecordingDifferencer.java`, `J:Injectable.java` | `differencer.rs` | ported |
| `InvalidatingNodeVisitor` (`InvalidationType`, `InvalidationState`, `DirtyingInvalidationState`, `DeletingInvalidationState`, `DeletingNodeVisitor`, `DirtyingNodeVisitor`) | `J:InvalidatingNodeVisitor.java:51-570` | `invalidation.rs` | ported, sequential, not interruptible (section b) |
| `EagerInvalidator` | `J:EagerInvalidator.java:27-90` | `invalidation.rs` `EagerInvalidator` | ported |
| `ParallelEvaluator.eval`, `doMutatingEvaluation`, `injectErrorTransienceValue`, `waitForCompletionAndConstructResult`, `informProgressReceiverThatValueIsDone`, `injectValues`, `bubbleErrorUp` (dirty parent, replay), `constructResult` | `J:ParallelEvaluator.java:96-700` | `evaluator.rs` `Run::eval`, `do_mutating_evaluation`, `inject_error_transience_value`, `wait_for_completion_and_construct_result`, `inform_progress_receiver_that_value_is_done`, `Evaluator::inject_values_into_graph`, `bubble_error_up`, `construct_result` | ported |
| `AbstractParallelEvaluator.Evaluate.run`, `enqueueChild`, `invalidatedByErrorTransience`, `maybeHandleDirtyNode`, `handleKnownChildrenForDirtyNode`, `maybeHandleRegisteringNewlyDiscoveredDepsForDoneEntry`, `maybeHandleUndoneDepForDoneEntry`, `maybeMarkRebuilding`, `replay` | `J:AbstractParallelEvaluator.java:145-860, 966-1143` | `evaluator.rs` `Run::evaluate_node`, `eval_dep`, `invalidated_by_error_transience`, `maybe_handle_dirty_node`, `eval_keys`, `maybe_handle_registering_newly_discovered_deps_for_done_entry`, `register_done_deps`, `maybe_mark_rebuilding`, `replay`, `resolve` | ported to the blocking model (section b); restarts, partial reevaluation, resets, external deps not |
| `SkyFunctionEnvironment.create`, `createForError`, `batchPrefetch`, `reportEventsAndGetEventsToStore`, `addTransitiveEventsFromDepValuesForDoneNode`, `setError`, `removeUndoneNewlyRequestedDeps`, `commitAndGetParents`, `isAny*DepErrorTransitivelyTransient` | `J:SkyFunctionEnvironment.java:164-1016` | `environment.rs` `Environment::create`, `create_for_error`, `batch_prefetch`, `report_events_and_get_events_to_store`; `evaluator.rs` `Run::set_error`, `remove_undone_newly_requested_deps`, `commit_and_get_parents`, `resolve` | ported |
| `ErrorInfoManager.UseChildErrorInfoIfNecessary.getErrorInfoToUse` | `J:ErrorInfoManager.java:55-66` | `Run::commit_and_get_parents` | ported |
| `ParallelEvaluatorContext.signalParentsOnAbort`, `signalParentsAndEnqueueIfReady`, replaying visitor, `NestedSetEventReceiver` | `J:ParallelEvaluatorContext.java:91, 108-130, 193-205` | `Run::signal_parents_on_abort`, `signal_parents_and_enqueue_if_ready`, `replay_visit`, `report_stored` | ported |
| `NodeEntryVisitor.preventNewEvaluations`, `innerEnqueueEvaluation` (progress part) | `J:NodeEntryVisitor.java:190, 233` | `Run::prevent_new_evaluations`, `enqueue_evaluation` | ported |
| `EmittedEventState` | `J:EmittedEventState.java:29-50` | `evaluator.rs` `pub struct EmittedEventState` (`clear`, `need_to_visit` as `NestedSetVisitorVisitedState`), `Evaluator::emitted_event_state()` | ported |
| `SimpleCycleDetector` (all, with `maybeHandleVerifiedCleanNode`, dirty block, `removeDescendantsOfCycleValue` signalling, `removeIncompleteChildForCycle`, commit through `setError` / `commitAndGetParents` / `signalParentsOnAbort`) | `J:SimpleCycleDetector.java:41-553` | `simple_cycle_detector.rs` | ported |
| `DelegatingWalkableGraph.getDirectDeps`, `getReverseDeps` | `J:DelegatingWalkableGraph.java:122-153` | `walkable_graph.rs` | now on the entry |
| `DirtyAndInflightTrackingProgressReceiver` sets | `J:DirtyAndInflightTrackingProgressReceiver.java` | `progress.rs` `ConcurrentSet` | changed: sharded sets (phase 1 had one mutex) |
| `GroupedDeps.numElements` | `J:GroupedDeps.java:248` | `grouped_deps.rs` | changed: cached count (constant time) |
| `ValueWithMetadata.NO_EVENTS` | `J:ValueWithMetadata.java:29` | `value_with_metadata.rs` `no_events()` | changed: one shared empty set |

New public API (all additive): `EvaluatorBuilder::{keep_edges, progress_receiver, event_filter}`,
`Evaluator::{inject_dyn, invalidate, delete, delete_dirty, get_values, get_done_values,
emitted_event_state}`, `pub struct EmittedEventState` (`new`, `clear`), `impl EventFilter for
&'static dyn EventFilter`, `impl EvaluationProgressReceiver for &'static dyn
EvaluationProgressReceiver`. `keep_edges(false)` is ported (it fell out of the Java): constant
versions, the non-incremental entry, deletion without traversal, `graph.remove` in
`pruneInjectedValues`.

## (b) `// PORT:` deviations added or changed

| Where | Java | Deviation and why |
| --- | --- | --- |
| `in_memory_graph.rs:43` `DoneEntry` | `NodeEntry.getValueMaybeWithMetadata` | A done node's value is kept as an immutable snapshot (`Arc<DoneEntry>`, refreshed at `setValue`, `markClean`, `markDirty`): a request clones one `Arc` under the node lock, nothing per dep list. |
| `in_memory_graph.rs:150-163` `owner`, `outcome`, `cycle_rdeps` | — | Blocking engine: the owning thread (wait-for graph), how the node ended in this evaluation (what waiters wait for; set after the parents were signalled), requesters that closed a cycle through it. A node that ends "undone" keeps its entry evaluating (in flight) as in Java. |
| `in_memory_graph.rs:242`, `:330`, `:372` | `InMemoryGraphImpl` | Sharded map; `getAllNodeEntries` is a copy; `parallelForEach` sequential; no interning. |
| `differencer.rs:68` | `Differencer.getDiff(fromGraph, ...)` | The graph and versions are not passed (the one implementation ignores them). |
| `differencer.rs:142` | `SequencedRecordingDifferencer.valuesToInject` | java-hash-order: insertion order; only progress receivers see the injection order. |
| `invalidation.rs:4`, `:156`, `:176`, `:425` | `InvalidatingNodeVisitor` | Sequential work-list traversal on the calling thread instead of a fork-join pool; not interruptible (the evaluator checks its interrupt after invalidation). Effects, checks and the pending set are Java's; the pending set is always empty after a run. |
| (removed in I2) `invalidation.rs` | `InvalidatingNodeVisitor.java:462-470` | The hermeticity check ran only for keys that declared `Some(Hermetic)`. Since I2 every key declares `Key::HERMETICITY` (required) and the check runs for all, as in Java. |
| `evaluator.rs:323`, `:391` | `Injectable.inject` | Values given to `EvaluatorBuilder::inject` are injected by `build()` at the first evaluation's version, not at the first `evaluate`. |
| `evaluator.rs:546` (I2, review S2-2) | `AbstractInMemoryMemoizingEvaluator.java:481` `performInvalidation` | Java's invalidators throw `InterruptedException` for a pending interrupt and `injectValues` is skipped; here invalidation runs to completion, then the pending interrupt returns `Interrupted` before the injection. With nothing to invalidate there is no check, and `ParallelEvaluator.eval` returns cached roots (`ParallelEvaluator.java:641`). |
| `evaluator.rs` `eval_dep`, `environment.rs` `lookup` (I2, review S2-1) | `AbstractParallelEvaluator.java:394`, `:1022` | A dirty node's old dep missing from the graph fails with Java's texts ("Dirty parent had missing child", "Missing already declared dep"); the second fails at the request, before the missing dep is computed, where Java fails when it commits the node. |
| `evaluator.rs:363` | `InMemoryMemoizingEvaluator.ProgressReceiver` | Statistics (`postLoggingStats`) not ported. |
| `evaluator.rs:508`, `:578`, `:1127`, `:2658` | various | Skyfocus, max transitive source versions, `SkyKeyComputeState`, reset deps not ported (null / empty, Java's values without those features). |
| `evaluator.rs:583` | `AbstractInMemoryMemoizingEvaluator.java:441` | `newValue.equals(prev)` is `Key::values_equal` (D10). |
| `evaluator.rs:1103` | `ParallelEvaluator.java:668` | Cached error keys in root order (Java: `HashSet`). Kept from before. |
| `evaluator.rs:1643` | `Evaluate.run:408-413` | `isReadyToEvaluate` check dropped (partial reevaluation only). |
| `evaluator.rs:1737` `maybe_handle_dirty_node` | `AbstractParallelEvaluator.java:233-380` | A dep group is evaluated with the request code path (`eval_keys`, registering with `checkIfDoneForDirtyReverseDep`); a dep that does not finish ends the node "undone" with the dep's kind (still dirty, in flight). |
| `evaluator.rs:1895` | `analyzeDepsDoneness` | `NO_INFORMATION` (in-memory graph): no minimal-version signals. |
| `evaluator.rs:2017` `entry_seen_in_normal_evaluation` | — | Kept from the reviewed engine (masking after abort); also applies to dirty checks. |
| `evaluator.rs:2035` `register_pending_deps`, `environment.rs:48` `EnvState::pending` | `addTemporaryDirectDepsTo` + `enqueueChild`; `maybeHandleRegisteringNewlyDiscoveredDepsForDoneEntry` | A done dep read by a function is registered (temporary direct deps, reverse dep, `signalDep`) when the function returns, as in Java; when a request finds a dep not done, the deps requested so far and the request's group are registered first (what Java does when the function returns null there), then the dep is evaluated. Rdep order on a dep is Java's for single-threaded runs. |
| `evaluator.rs:2753` signalling | `signalParentsAndEnqueueIfReady` | The committer signals the reverse deps `setValue` returns before it publishes the node's outcome; a requester that registers on a done node signals itself (`enqueueChild` `DONE`). "Enqueue" is the waiting thread waking up. |
| `evaluator.rs:2155` `remove_undone_newly_requested_deps` | `SkyFunctionEnvironment.java:853` | The deps a function saw not done were registered by its requests; the registration is taken back (temporary deps and reverse dep; for an old dep `commitAndGetParents` removes the reverse dep, as in Java). A dep done by now (it signalled the node) stays. |
| `evaluator.rs:2193` | `AbstractParallelEvaluator.java:973` | The `removeUndoneNewlyRequestedDeps` step is done by the caller first. |
| `evaluator.rs:2263` | `AbstractParallelEvaluator.java:494-520` | A thrower that is not the first error keeps the registrations of deps it saw not done (state 3(b) of the 2026-11-18 review: Java's function would have thrown before evaluating them); its done deps are registered as in Java. |
| `evaluator.rs:2333` | `rdepsToBubbleUpTo` | Without requesters that closed a cycle through the node (kept from the reviewed engine). |
| `evaluator.rs:2495` | `Evaluate.run:640-656` | All requested deps done and the function returned null: Java restarts it (no new deps) and commits the child errors; the blocking engine commits without a second run. |
| `evaluator.rs:2743` | `commitAndGetParents` `evaluated(..., temporaryDirectDeps)` | The direct deps are copied only when the caller gave a progress receiver. |
| `evaluator.rs:3145-3369` bubbling | `ParallelEvaluator.bubbleErrorUp` | Kept from the reviewed engine (`never_evaluated`, `awaits_deps`); now on entries; the `isInflight` check and the dirty-parent signalling (`:395-403`) are ported. |
| `environment.rs:177` `batch_prefetch` | `SkyFunctionEnvironment.java:234-290` | `UndonePreviouslyRequestedDeps` cannot happen (no rewinding, nothing dirties a node during an evaluation); an undone dep panics with the `THROWING` inconsistency text. |
| `environment.rs:285` | `getTemporaryDirectDeps` | In normal evaluation: previously requested deps followed by this run's requests. |
| `environment.rs:978` | `addTransitiveEventsFromDepValuesForDoneNode` | `MANUALLY_REGISTERED_MARKER` cannot occur (`registerDependencies` not ported). |
| `simple_cycle_detector.rs:225`, `:432` | `SimpleCycleDetector.java:204-215, 341-352` | `BugReport.sendBugReport` (non-crashing) not ported. |
| `walkable_graph.rs:107` | `DelegatingWalkableGraph.getReverseDeps` | Reverse deps are the entry's (see (d)). |
| `progress.rs:349` | `Sets.newConcurrentHashSet` | Sharded set. |
| `grouped_deps.rs:12-20` | `GroupedDeps.numElements` | Cached count (Java: array size); `Debug` output unchanged. |
| `value_with_metadata.rs:71` | `ValueWithMetadata.NO_EVENTS` | One shared empty set (phase 1 allocated one per call). |
| `tests/*`: `NotComparableKey` (`tests.rs`), `failBuildAndRemoveValue`, `deletingDirtyNodes`, `changePruningWithEvent`, `runPartialResultOnInterruption`, `fullEventStorage_*`, `deepGraph`, `GraphTest` batch sampling | test harness | Java's `NotComparableStringValue` is a key type here (D10); restart counts are one run; a new `EmittedEventState` per Java evaluator is a `clear()`; the deep chain is built bottom-up in 16-level steps (1024 nested threads otherwise); a fixed sample instead of a seeded `Random`. |

Removed: the four `// PORT: skipped, incremental` comments (`evaluator.rs` bubbling dirty
handling and stored-event replay, `simple_cycle_detector.rs` dirty block and
`maybeHandleVerifiedCleanNode`/`maybeMarkRebuilding`), `Run::cleanup`, `Node::rdeps`,
`Node::in_progress_rdeps`, `delete_reverse_transitive_closure`.

## (c) `unported!` sites

Unchanged from before: `ShortCircuitingCycleDetector.checkForCycles` (`evaluator.rs`
`construct_result`), `SkyFunctionEnvironment.registerDependencies`, `getState`, `dependOnFuture`,
`SkyframeLookupResult.queryDep` (`environment.rs`); phase 1's in the entry modules. No new one:
`keep_edges(false)` is ported. Graph inconsistencies (Java's `GraphInconsistencyReceiver.THROWING`)
panic with Java's text ("Unexpected inconsistency: key, [dep], BUILDING_PARENT_FOUND_UNDONE_CHILD").

Unused (with a targeted `#[allow(dead_code)]` and the reason at the item): `Delta.old_value`,
`Injectable::inject_all`, `NodeEntry` trait methods for restarts / rewinding / external deps,
`MarkedDirtyResult::ResultForRewinding`, `IncrementalInMemoryNodeEntry::{reset_evaluation_from_scratch,
clear_direct_deps_for_skyfocus, consolidate_reverse_deps}`, `InflightOnlyTrackingProgressReceiver`,
`remove_from_inflight`, a few entry accessors used only by tests. The phase-1
`#![allow(dead_code)]` lines are removed except in `tests/model.rs` (the model driver is not
written yet).

## (d) Behaviour changes visible to other crates

* `Evaluator::inject` and `invalidate_transient_errors` are recorded and applied at the start of
  the next `evaluate` (`J:AbstractInMemoryMemoizingEvaluator.java:134-152`); before, `inject`
  changed the graph at once. `get_existing_value` right after `inject` returns the old value until
  the next `evaluate`. Audit: no caller outside the crate reads back after `inject`
  (`Precomputed::set_on`, now `blaze_skyframe/src/precomputed.rs:159`, has no caller outside
  tests). `inject` no longer panics during an evaluation (Java records it).
* Injecting over a node changes nothing when `Key::values_equal` says the value is equal; otherwise
  its reverse transitive closure is dirtied (not deleted) and re-checked when requested. With no
  key opting into `values_equal`, the same functions run as when the closure was deleted.
* Invalidating (`inject` over a done node, `invalidate`) a key that declares
  `HERMETICITY = Some(Hermetic)` panics with Java's "Nodes with hermetic functions cannot be
  marked 'changed'". No key outside this crate declares hermeticity yet.
* Nodes still in flight when an evaluation ends (abort, interrupt, cycles not committed) stay in
  the graph, not done, until the next `evaluate` deletes them: `get_all_keys_for_testing` lists
  them; `get_existing_value` / `WalkableGraph` do not.
* `WalkableGraph::get_reverse_deps` returns the entry's reverse deps
  (`getReverseDepsForDoneEntry`, `J:IncrementalInMemoryNodeEntry.java:251`,
  `J:ReverseDepsUtility.java:248`): registration order (Java's), compacted by the entry, instead
  of commit order. No caller outside the crate (`blaze_query_engine`'s `get_reverse_deps` is its
  own trait).
* `ErrorTransienceValue.KEY` is created by the first `evaluate` that evaluates something
  (`J:ParallelEvaluator.java:187-202`), not by `build()`.

## (e) Tests

`scripts/cargo-capped test -p blaze_skyframe` in the worktree: 385 passed, 0 failed, 189 ignored
(before this phase: 304 / 0 / 221). 30 consecutive full runs: no failure.
`scripts/cargo-capped clippy -p blaze_skyframe --all-targets -- -D warnings`: clean.

| Java test class | Rust | Ported (passing) | Ignored |
| --- | --- | --- | --- |
| `JT:EagerInvalidatorTest` (11 base cases × `DeletingInvalidatorTest` / `DirtyingInvalidatorTest`, + 3 subclass cases = 14 methods) | `tests/eager_invalidator_test.rs` (`<name>_deleting`, `<name>_dirtying`) | 8 base × 2 + 2 subclass = 18 (+ `default_invalidation_types`, a check of the suites' `defaultInvalidationType`) | 7: `interruptChild`, `interruptRecoversNextTime`, `interruptThreadInReceiver` × 2 (invalidation not interruptible), `interruptDuringRewind_parentAlreadySignaled` (rewinding) |
| `JT:GraphTest` (9 methods; `testDoneToDirty` × 2 `BatchMethod`) on `InMemoryGraphTest` and `EdgelessInMemoryGraphTest` | `tests/graph_test.rs` (`<name>`, `<name>_edgeless`) | 16 | 2: `testDoneToDirty` × 2 on the edgeless graph (Java's `assumeTrue(shouldTestIncrementality())`) |
| `JT:InMemoryGraphTest` own (4) | `tests/in_memory_graph_test.rs` | 0 | 4: key and label interning not ported |
| `JT:MemoizingEvaluatorTest` | `tests/memoizing_evaluator_test.rs` | 40 more this phase (list below), 16 earlier ports rewritten to Java's keys (see below) | remaining stubs in `not_ported.rs` |
| `JT:ParallelEvaluatorTest` | `tests/parallel_evaluator_test.rs` | 5 more this phase | remaining stubs in `not_ported.rs` |
| (no Java) | `tests/engine_test.rs` `keep_edges_false_memoizes_and_injects_without_edges` | 1 | — |

Ported this phase (moved out of `not_ported.rs`): MemoizingEvaluatorTest `invalidationWithNothingChanged`,
`deleteValues`, `deleteOldNodesTest`, `deleteDirtyCleanedValue`, `deleteNonexistentValues`,
`signalValueEnqueued`, `invalidationWithChangeAndThenNothingChanged`,
`noKeepGoingErrorAfterKeepGoingError`, `transientPruning`, `incrementalAddedDependency`,
`nodeIsChangedWithoutBeingEvaluated`, `dirtyThenDeleted`, `dirtyAndChanged`, `changePruning`,
`changePruningWithDoneValue`, `changePruningAfterParentPrunes`,
`changePruningFromOtherNodeAfterParentPrunes`, `changedChildChangesDepOfParent`,
`dirtyDepIgnoresChildren`, `dirtyBuildAfterFailedBuild`, `failedDirtyBuild` (dirty parent in
bubbling), `deletingDirtyNodes`, `deleteInvalidatedValue`, `valueInjectionOverExistingDirtyEntry`,
`valueInjectionOverExistingEntryMarkedForInvalidation`, `...MarkedForDeletion`,
`valueInjectionOverExistingEqualEntryMarkedForInvalidation`, `...EqualEntryMarkedForDeletion`,
`valueInjectionOverValueWithDeps`, `valueInjectionOverEqualValueWithDeps`,
`valueInjectionOverExistingEqualEntryDoesNotInvalidate` (pruneInjectedValues),
`notComparableNotPrunedNoEvent`, `notComparableNotPrunedEvent`, `changePruningWithEvent` (stored
events replayed), `errorChanged`, `cycleWithDirtyValue` (dirty nodes in a cycle), `breakCycle`,
`continueWithErrorDepTurnedGood`, `errorDepAlreadyThereThenTurnedGood` (error transience),
`dirtyDependsOnErrorTurningGood`; ParallelEvaluatorTest `runPartialResultOnInterruption`,
`fullEventStorage_unstoredEvent_reportedImmediately_notReplayed`,
`fullEventStorage_storedEvent_reportedAfterSkyFunctionCompletes_replayed`,
`signalValueEnqueuedAndEvaluated`, `topLevelKeyBuiltAsDepOfAnotherTopLevelKey_reportedToProgressReceiverOnce`.

Existing tests adjusted (Java evidence):

* `incrementalSimpleDependency`, `incrementalDiamondDependency`, `manyValuesDependOnSingleValue`,
  `singleValueDependsOnManyValues`, `twoRailLeftRightDependencies`, `continueWithErrorDep`,
  `doubleDepOnErrorTransienceValue`, `errorInDependencyGroup`, `valueInErrorWithGroups`,
  `errorTransienceDepCleared`, `sameDepInTwoGroups`: the earlier port injected into a hermetic `T`
  key in place of Java's `tester.set(nonHermeticKey(...)); tester.invalidate()`; they now use
  Java's non-hermetic key and set + invalidate (injecting into a hermetic key now panics, as in
  Java). `valueInjection`, `valueInjectionOverExistingEntry`, `valueInjectionOverValueWithErrors`,
  `valueInjectionInvalidatesReverseDeps`, `valueInjectionInterrupt`: Java's key is
  `nonHermeticKey`; they inject into it.
* `engine_test::graph_keys` and `tests::no_keep_going_reports_one_bubbled_root_and_drops_aborted_roots`
  checked that uncommitted nodes left the graph at the end of `evaluate`; Java keeps them in flight
  until the next `evaluate` deletes them (`J:AbstractInMemoryMemoizingEvaluator.java:134`). They
  now check "not done", and the second one also checks that the next evaluation deleted them.
* The test key `T` compares values (`values_equal`, Java's `StringValue.equals`).

Remaining stubs in `not_ported.rs` (77), by reason: not ported yet, left for the next part (25:
`changeCycle_*`, `cycleAndSelfEdgeWithDirtyValue`, `nodeInvalidatedThenDoubleCycle`,
`incompleteDirectDeps*`, `singleValueDependsOnManyDirtyValues`, `dirtyChildrenProperlyRemoved*`
(4), `manyDirtyValuesClearChildrenOn*` (3), `changedBuildAfterFailedThenSuccessfulBuild`,
`failedDirtyBuildInBuilder`, `dirtyWith*TurningGood` (2), `changePruningWithIntermittentEvent`,
`depEventPredicate`, `twoRailLeftRightDependenciesWithFailure`, `removed*ComesBack*` (3),
`receiverToldOfVerifiedValueDependingOnCycle`); graph listener `NotifyingHelper` (12); rewinding /
`SkyFunction.Reset` (14); `dependOnFuture` (4); `SkyKeyComputeState` (4); partial reevaluation (5);
graph transformers (3); per-key keep_going (2); `GraphTraversingHelper` (2); injected executor (2);
`validateExceptionType` (2); restart model (2).

Harness (`tests.rs`) for the next workers: `Tester` keyed by `SkyKey` with every
`GraphTester.TestFunction` setter (`*_key` variants), `get_or_create(key, mark_as_modified)`,
`get_modified_values`, `N` (`NonHermeticKey`), `NotComparableKey`, `nk`, `MemoizingTester`
(Java's `MemoizingEvaluatorTester`: `TrackingProgressReceiver`, `FULL_STORAGE`, event collector,
`initialize_reporter`, `invalidate`, `invalidate_transient_errors`, `delete`,
`reset_played_events`, `get_dirty_keys`, `get_deleted_keys`, `get_enqueued_values`,
`eval`/`try_eval`/`eval_and_get`/`eval_and_get_error`, `get_existing_value`), `Latch::get_count`.

## (f) Invariants a SkyFunction must keep

Met while porting; the engine relies on them as Java does.

1. Deterministic deps: given the same dep values, a function requests the same deps in the same
   order and groups. Change pruning (`markClean` checks the deps are those of the last build,
   `J:DirtyBuildingState.java:225`) and the dirty check (groups of the last build, in order)
   depend on it.
2. Nothing read outside Skyframe deps, unless the key is non-hermetic and is invalidated when that
   state changes. A node marked clean is never re-run.
3. A function that returns null with all its deps done must request nothing new on a re-run:
   Java restarts it and commits its child errors; Blaze commits without the re-run
   (`evaluator.rs:2495`).
4. Values of keys that opt into `values_equal` must be equal exactly when a re-run may be skipped
   for every dependent (an equal value keeps the node's version).
5. A function that throws although a dep is missing should throw the same exception when the dep
   is present (Java's comment at `J:AbstractParallelEvaluator.java:946-951`).
6. Deps are requested before work that must not be repeated (error bubbling re-runs the function;
   unchanged from before).
7. Hermetic keys (`HERMETICITY = Some(Hermetic)`) are never injected or invalidated.
