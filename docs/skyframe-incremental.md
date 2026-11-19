# Incremental evaluation in `blaze_skyframe` (package I1)

Status: I1 implemented, not reviewed (2026-11-19). Written by the Track I lead; the reviewer of
I1 reads this first. Plan: `docs/research/08-server-and-incrementality.md` sections 2, 4.4, 4.5, 5.2. Decisions:
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
functions run as when the closure was deleted. Confirmed by the regression guard (section 7).


## 4. Modules and Java classes

The tables below were written by the worker of each part when that part finished, and are kept
as written. Rust paths are relative to `crates/blaze_skyframe/src/`. A `file:line` in a Rust
file is from the tree of that part: later parts moved lines (function and type names are
stable); Java lines are at the pin and do not move. Each part's full notes are in
`docs/skyframe-incremental.d/` (`node-entry.md`, `support-and-model.md`, `engine.md`,
`evaluator-tests.md`, `model-test.md`). Where a phase-1 note says a module "is not used by the
evaluator yet", phase 2 (the engine) is what uses it.

### 4.1 Node entries, versions, reverse deps, `ValueWithMetadata` (phase 1)

| Java class | Java file:lines | Rust file | State | Tests |
| --- | --- | --- | --- | --- |
| `Version` (+ `MinimalVersion`, `ConstantVersion`) | `J:Version.java:23-103` | `src/version.rs` (`pub enum Version`) | ported | through the entry tests |
| `IntVersion` | `J:IntVersion.java:20-71` | `src/version.rs` (`pub struct IntVersion`) | ported (no interning) | through the entry tests |
| `NodeVersion` (+ `ChangePruned`) | `J:NodeVersion.java:22-55` | `src/version.rs` (`pub enum NodeVersion`) | ported | change-pruning tests |
| `NodeEntry` (+ `DependencyState`, `LifecycleState`, `DirtyType`, `MarkedDirtyResult`, `NodeValueAndRdepsToSignal`) | `J:NodeEntry.java:35-680` | `src/node_entry.rs` (trait `NodeEntry`, `pub enum DirtyType`) | ported; `resetEvaluationFromScratch`, `addExternalDep` documented as unported, `REWIND` fails | — |
| `InMemoryNodeEntry` | `J:InMemoryNodeEntry.java:24-73` | `src/in_memory_node_entry.rs` (`enum InMemoryNodeEntry`) | ported | — |
| `AbstractInMemoryNodeEntry` | `J:AbstractInMemoryNodeEntry.java:65-261` | `src/abstract_in_memory_node_entry.rs` (fields struct + trait) | ported; `addExternalDep` unported | `InMemoryNodeEntryTest` |
| `IncrementalInMemoryNodeEntry` (+ `IncrementalBuildingState`) | `J:IncrementalInMemoryNodeEntry.java:32-483` | `src/incremental_in_memory_node_entry.rs`; `IncrementalBuildingState` in `src/dirty_building_state.rs` | partly: rewinding, resets and Skyfocus unported | `InMemoryNodeEntryTest`, `IncrementalInMemoryNodeEntryTest` |
| `IncrementalInMemoryNodeEntry.ResetInitialBuildingState`, `ResetIncrementalBuildingState` | `J:IncrementalInMemoryNodeEntry.java:489-538` | — | not ported (rewinding) | ignored |
| `NonIncrementalInMemoryNodeEntry` (+ `NonIncrementalBuildingState`) | `J:NonIncrementalInMemoryNodeEntry.java:40-279` | `src/non_incremental_in_memory_node_entry.rs` | partly: rewinding, resets unported | `InMemoryNodeEntryTest` |
| `NonIncrementalInMemoryNodeEntry.RewoundNonIncrementalBuildingState` | `J:NonIncrementalInMemoryNodeEntry.java:286-303` | — | not ported (rewinding) | ignored |
| `DirtyBuildingState` | `J:DirtyBuildingState.java:38-306` | `src/dirty_building_state.rs` | ported (`addExternalDep` ported, its only caller is unported) | through the entry tests |
| `InitialBuildingState` | `J:InitialBuildingState.java:23-50` | `src/dirty_building_state.rs` (`DirtyBuildingStateKind::Initial`) | ported | through the entry tests |
| `ReverseDepsUtility` | `J:ReverseDepsUtility.java:59-412` | `src/reverse_deps_utility.rs` | ported (storage format simplified, see (b)) | `ReverseDepsUtilityTest` |
| `KeyToConsolidate` (+ `Op`) | `J:KeyToConsolidate.java:34-175` | `src/reverse_deps_utility.rs` | ported (no interning) | `ReverseDepsUtilityTest` |
| `ValueWithMetadata` (+ `ValueWithEvents`, `NotComparableValueWithEvents`, `ErrorInfoValue`) | `J:ValueWithMetadata.java:26-253` | `src/value_with_metadata.rs` | ported; `ErrorInfoValue.equals` unported | through the entry tests |
| `Reportable` as stored by a node | `lib/events/Reportable.java` | `src/value_with_metadata.rs` (`StoredReportable`, moved from `environment.rs`) | moved, `Clone + PartialEq + Eq + Hash` added | — |
| `GroupedDeps.appendGroups` / `appendNextGroup` | `J:GroupedDeps.java:149-179` | `src/grouped_deps.rs` | ported (`compress`/`decompress`/`WithHashSet` not: memory format / partial reevaluation) | `InMemoryNodeEntryTest.addTemporaryDirectDepsInGroups*` (GroupedDepsTest has no case for it) |
| `SkyValue.equals` hook (decision D10) | `J:DirtyBuildingState.java:214` | `src/lib.rs` `Key::values_equal`, `DynKey::dyn_values_equal` | added | change-pruning tests |

### 4.2 Progress receivers, event filter, hermeticity (phase 1)

| Java class | Java file:lines | Rust | State | Tests |
| --- | --- | --- | --- | --- |
| `EvaluationProgressReceiver` (+ `NULL`) | J:EvaluationProgressReceiver.java:22-146 | `progress.rs` (trait, `static NULL`) | ported, not wired | `tests/progress_receiver_test.rs` |
| `EvaluationProgressReceiver.EvaluationState` | :28-74 | `progress.rs` `EvaluationState` | ported (`get` takes the just-value, PORT) | `evaluation_state_get_and_flags` |
| `EvaluationProgressReceiver.NodeState` | :77-86 | `progress.rs` `NodeState` | ported | via compound test |
| `InflightTrackingProgressReceiver` | J:InflightTrackingProgressReceiver.java:19-38 | `progress.rs`, `pub(crate) trait` | ported | via the two below |
| `InflightOnlyTrackingProgressReceiver` | J:InflightOnlyTrackingProgressReceiver.java:31-119 | `progress.rs`, `pub(crate)` | ported, not wired | `inflight_only_tracks_inflight` |
| `DirtyAndInflightTrackingProgressReceiver` | J:DirtyAndInflightTrackingProgressReceiver.java:27-190 | `progress.rs`, `pub(crate)` | ported except rewinding (PORT) | `dirty_and_inflight_*` (4) |
| `CompoundEvaluationProgressReceiverBase` | J:CompoundEvaluationProgressReceiverBase.java:24-85 | `progress.rs` | ported | `compound_receiver_forwards_to_each_in_order` |
| `CompoundEvaluationProgressReceiver` | J:CompoundEvaluationProgressReceiver.java:22-28 | `progress.rs` (newtype over the base) | ported | same |
| `NodeEntry.DirtyType` | J:NodeEntry.java:95-135 | `node_entry.rs` (placeholder file, replaced by I1-A's at merge) | ported | used by the receiver tests |
| `EventFilter` (+ `FULL_STORAGE`, `NO_STORAGE`) | J:EventFilter.java:17-59 | `event_filter.rs`; constants are `<dyn EventFilter>::FULL_STORAGE` / `NO_STORAGE` | ported, not wired | 3 tests in `event_filter.rs` |
| `FunctionHermeticity` | J:FunctionHermeticity.java:28-51 | `function_hermeticity.rs` | ported | `key_hermeticity_is_declared_per_key_type` |
| `SkyFunctionName.getHermeticity` | J:SkyFunctionName.java:84 | `lib.rs` `Key::HERMETICITY` (default `None`), `DynKey::hermeticity`, `SkyKey::hermeticity` | ported per key type | same |
| `ErrorTransienceValue` function name hermeticity | J:ErrorTransienceValue.java:25 | `evaluator.rs` `ErrorTransienceKey::HERMETICITY = Some(Nonhermetic)` | ported | same |
| `GraphTester.NODE_TYPE` hermeticity | JT:GraphTester.java:53, J:SkyFunctionName.java:32 | `tests.rs` `T::HERMETICITY = Some(Hermetic)` | ported | same |
| `TrackingProgressReceiver` | JT:TrackingProgressReceiver.java:24-96 | `tests/tracking_progress_receiver.rs` | ported | `tracking_receiver_check_evaluation_results` |
| `InvalidationProgressReceiver` | JT:InvalidationProgressReceiver.java:24-46 | `tests/invalidation_progress_receiver.rs` (trait + adapter, PORT) | ported | used in phase 2 |

Bazel has no JUnit class for these receivers (they are exercised by ParallelEvaluatorTest,
MemoizingEvaluatorTest and EagerInvalidatorTest, which come with the evaluator in phase 2); the
tests listed are written from the Java methods.

Shared files touched: `lib.rs` (mod lines 145-146, 148-149; `pub use` 161-162, 164-168;
`Key::HERMETICITY` 208-211; `DynKey::hermeticity` 217-218 and its blanket impl 228-230;
`SkyKey::hermeticity` 255-260), `evaluator.rs` 50-52 (`ErrorTransienceKey::HERMETICITY`),
`tests.rs` 326-329 (`T::HERMETICITY`) and the `mod` lines 1831, 1833, 1836-1837. The crate docs
in `lib.rs` ("Not ported": `EvaluationProgressReceiver`, `EventFilter`) are left for the lead to
update when the receivers are wired.

### 4.3 Graph, differencer, invalidation, evaluator, environment, cycle detector (phase 2)

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

### 4.4 Test-only graph listener (phase 3)

`graph_listener.rs` (`#[cfg(test)]`): port of `NotifyingHelper` (`EventType`, `Order`, `Reason`,
`Listener`, stacked listeners) as hooks (`notify_graph_listener!`, which expands to nothing
outside tests) at the entry calls of `evaluator.rs`, `in_memory_graph.rs` and `invalidation.rs`.
Events not emitted are listed in the module docs and in `evaluator-tests.md`. One non-test change
came with it: in the dirtying visitor `isChanged`, `isDirty` and `markDirty` take the node lock
separately, as Java's three `synchronized` calls do (`J:InvalidatingNodeVisitor.java:532-556`).

## 5. `// PORT:` deviations

### 5.1 Node-entry layer

| Where | Java | Deviation and why |
| --- | --- | --- |
| `version.rs:12` | `Version.java:23` | Interface with three implementations is a closed enum; singleton identity is variant equality. |
| `version.rs:100` | `Object.toString` | `MinimalVersion`/`ConstantVersion` print their class name (Java's identity hash is not reproducible). |
| `version.rs:111` | `IntVersion.java:22-26` | No interner; equality was by value anyway. |
| `version.rs:155` | `NodeVersion.java:22` | Interface implemented by `Version` and `ChangePruned` is a two-variant enum. |
| `value_with_metadata.rs:15` | `Reportable` | A `Postable` compares by identity (`Arc::ptr_eq`, pointer hash); a Java `Postable` overriding `equals` would compare by value. |
| `value_with_metadata.rs:72` | `ValueWithMetadata.java:29` | `NO_EVENTS` is a new empty set per call (`Order::empty_set`); all empty sets are `shallowEquals`. |
| `value_with_metadata.rs:81` | `ValueWithMetadata.java:46-58, 225-252` | Java's bare `SkyValue` (no error, no events) is the `Bare` variant of the one enum; the static helpers take that enum. |
| `value_with_metadata.rs:88`, `:151` | `ValueWithMetadata.java:84-91, 140-145` | No `NotComparableValueWithEvents` variant: comparability is the key's `values_equal`. |
| `value_with_metadata.rs:205` | `ValueWithMetadata.java:109-124, 182-201` | `equals` takes the node's key (value equality is the key's, D10); Java's `this == o` shortcut is dropped (Rust cannot see `NotComparableSkyValue`, and a stored value is never compared with itself); a bare value is never equal to a wrapped one. |
| `value_with_metadata.rs:281` | `toString`s | A `SkyValue` (`dyn Any`) prints as `<SkyValue>` in crash messages. |
| `node_entry.rs:111` | `NodeEntry.java:95-136` | `DirtyType::Rewind` kept; marking a node `Rewind` fails with `unported!`. |
| `node_entry.rs:135` | `NodeEntry.java:182-185` | `MarkedDirtyResult` holds a copy of the reverse deps (same content and order), not a live view. |
| `node_entry.rs:203` | `NodeEntry.java:35` | `synchronized` is the caller's lock (graph `Mutex`); no `InterruptedException`; methods take `&self`/`&mut self`. |
| `node_entry.rs:334`, `incremental_in_memory_node_entry.rs:163`, `non_incremental_in_memory_node_entry.rs:82` | `NodeEntry.java:341` | `max_transitive_source_version` accepted and ignored (both in-memory entries ignore it at the pin). |
| `node_entry.rs:511` | `NodeEntry.java:561` | `get_temporary_direct_deps` takes `&mut self` (lazy creation). |
| `node_entry.rs:537`, `dirty_building_state.rs:375` | `NodeEntry.java:627`, `DirtyBuildingState.java:270` | `getResetDirectDeps` is the base `ImmutableSet.of()`; reset states not ported. |
| `dirty_building_state.rs:26` | `DirtyBuildingState.java:38`, `InitialBuildingState.java:23`, `IncrementalInMemoryNodeEntry.java:446` | Abstract class + subclasses are one struct with a `DirtyBuildingStateKind`. |
| `dirty_building_state.rs:244` | `DirtyBuildingState.java:178-183` | `signalDep` cannot borrow the entry that owns it: the entry makes the first check (`checkState(isEvaluating(), "%s %s", entry, child)`, `incremental_in_memory_node_entry.rs:356`, `non_incremental_in_memory_node_entry.rs:205`) and passes `getNumTemporaryDirectDeps()` (a pure read) instead of itself. |
| `dirty_building_state.rs:287` | `DirtyBuildingState.java:214-219` | `unchangedFromLastBuild` takes the node's key (D10). `instanceof NotComparableSkyValue` is "is an `ErrorInfoValue`"; for other values the key's `values_equal` answers. |
| `in_memory_node_entry.rs:21` | `InMemoryNodeEntry.java:24`, `InMemoryGraphImpl.java:195,316` | The interface with two engine implementations is an enum chosen by `keep_edges`. |
| `in_memory_node_entry.rs:70`, `incremental_in_memory_node_entry.rs:32`, `:129` | `IncrementalInMemoryNodeEntry.java:46, 102-106, 119` | Done deps are not compressed (memory format). |
| `incremental_in_memory_node_entry.rs:42`, `reverse_deps_utility.rs:144`, `:214`, `:522` | `ReverseDepsUtility.java:47-57, 114-138, 362-373` | Reverse deps are always a `Vec` (Java: bare key / array / ArrayList by size). "Single reverse dep" is `len() == 1`, exactly Java's condition since every writer stores one element bare. Order kept. |
| `reverse_deps_utility.rs:56` | `KeyToConsolidate.java:34-175` | The bare encoding is kept (its meaning depends on `opToStoreBare` at decode time); interning and subclass layout dropped. |
| `reverse_deps_utility.rs:144` | `CompactHashSet` | Sets are `IndexSet`; a removal is `swap_remove`, which is `CompactHashSet.remove`'s order (last element moves into the hole, `lib/collect/compacthashset/CompactHashSet.java:372-430`). |
| `reverse_deps_utility.rs:270` | `ReverseDepsUtility.java:170-178` | Kept Java's bug (collects first occurrences, not duplicates); the `HashSet` order of that crash message is not reproduced (java-hash-order, crash text only). |
| `reverse_deps_utility.rs:294` | `ReverseDepsUtility.java:188-281` | `consolidateDataAndReturnNewElements(entry, mutateObject)` split into a pure computation and the mutation, so `returnNewElements` takes `&self`. Same checks, same order. |
| `reverse_deps_utility.rs:563` | `ReverseDepsUtility.java:403-409` | `toString` prints the `Vec` (Java prints the raw object; a `SkyKey[]` prints an identity hash). `singleReverseDep` is always false, as in Java (it passes the entry). |
| `abstract_in_memory_node_entry.rs:32` | `AbstractInMemoryNodeEntry.java:65` | Abstract class = fields struct (`base`) + trait with provided methods (template methods `getNumTemporaryDirectDeps`, `getTemporaryDirectDeps`). |
| `abstract_in_memory_node_entry.rs:89` | same | `synchronized` is the caller's lock. |
| `abstract_in_memory_node_entry.rs:133` | `AbstractInMemoryNodeEntry.java:98-103` | `key.supportsPartialReevaluation()` is false (no such method on Rust keys; partial reevaluation not ported). |
| `abstract_in_memory_node_entry.rs:261` | `AbstractInMemoryNodeEntry.java:208-219` | `newGroupedDeps` is always a plain `GroupedDeps` (`skipsBatchPrefetch` is not on Rust keys; Java default false). |
| `non_incremental_in_memory_node_entry.rs:321` | `NonIncrementalInMemoryNodeEntry.java:240` | `NonIncrementalBuildingState` wraps an `InitialBuildingState` (`base`) instead of extending it. |
| `non_incremental_in_memory_node_entry.rs:348` | `NonIncrementalInMemoryNodeEntry.java:246-251` | The entry passes a new empty `GroupedDeps` instead of itself (`entry.newGroupedDeps()`); used only when none exists. |
| `incremental_in_memory_node_entry.rs:392`, `non_incremental_in_memory_node_entry.rs:173` | `IncrementalInMemoryNodeEntry.java:291-292, 302-308, 328-330`, `NonIncrementalInMemoryNodeEntry.java:145-156` | Every `markDirty(REWIND)` fails with `unported!`, including Java's no-op cases (not done, or done in error). |
| `tests/in_memory_node_entry_test.rs:30` | `InMemoryNodeEntryTest.IntegerValue` | Test values' equality is on the test key (`TestKey::values_equal`, `IntegerValue` by value; an anonymous `new SkyValue() {}` is another type, never equal). |
| `tests/in_memory_node_entry_test.rs:124` | `ErrorInfo.equals` | `getErrorInfo()).isEqualTo(errorInfo)` compares `Debug` output (no `ErrorInfo.equals` port). |
| `tests/incremental_in_memory_node_entry_test.rs:1117` | `GroupedDeps.decompress` | Identity (no compression). |

Not a `// PORT:` site but worth knowing: `IntVersion::next` uses `wrapping_add` (Java `long` overflow).

### 5.2 Support classes

| Site | Why |
| --- | --- |
| `progress.rs:102` `EvaluationState::get` | Java takes the value maybe wrapped in `ValueWithMetadata` and tests `justValue != null`; no `ValueWithMetadata` port yet, so the caller passes the just-value. |
| `progress.rs:301`, `:355`, `:370`, `:383` `DirtyAndInflightTrackingProgressReceiver` | Rewinding is not ported (skyframe-incremental.md section 6): no `unsuccessfullyRewoundKeys` set; `getAndClearUnsuccessfullyRewoundKeys` returns the empty set; `addToDirtySet(REWIND)` fails with `unported!`; `removeFromDirtySet`'s fallback removal is a no-op. |
| `progress.rs:484` `CompoundEvaluationProgressReceiverBase` | Java's constructor is protected; Rust has no subclassing, so it is `pub` and subclasses compose. |
| `tests/invalidation_progress_receiver.rs:19` | Java's abstract class with final `dirtied`/`deleted` becomes a trait (`invalidated`) plus an adapter implementing `EvaluationProgressReceiver`. |

Not a PORT but worth knowing: `EvaluationState::FailVersionUnchanged.version_changed()` returns
true, as Java constructs `FAIL_VERSION_UNCHANGED(false, true)` (J:EvaluationProgressReceiver.java:32).

### 5.3 Engine

| Where | Java | Deviation and why |
| --- | --- | --- |
| `in_memory_graph.rs:43` `DoneEntry` | `NodeEntry.getValueMaybeWithMetadata` | A done node's value is kept as an immutable snapshot (`Arc<DoneEntry>`, refreshed at `setValue`, `markClean`, `markDirty`): a request clones one `Arc` under the node lock, nothing per dep list. |
| `in_memory_graph.rs:150-162` `owner`, `outcome`, `cycle_rdeps` | — | Blocking engine: the owning thread (wait-for graph), how the node ended in this evaluation (what waiters wait for; set after the parents were signalled), requesters that closed a cycle through it. A node that ends "undone" keeps its entry evaluating (in flight) as in Java. |
| `in_memory_graph.rs:242`, `:317`, `:359` | `InMemoryGraphImpl` | Sharded map; `getAllNodeEntries` is a copy; `parallelForEach` sequential; no interning. |
| `differencer.rs:68` | `Differencer.getDiff(fromGraph, ...)` | The graph and versions are not passed (the one implementation ignores them). |
| `differencer.rs:142` | `SequencedRecordingDifferencer.valuesToInject` | java-hash-order: insertion order; only progress receivers see the injection order. |
| `invalidation.rs:4`, `:156`, `:176`, `:428` | `InvalidatingNodeVisitor` | Sequential work-list traversal on the calling thread instead of a fork-join pool; not interruptible (the evaluator checks its interrupt after invalidation). Effects, checks and the pending set are Java's; the pending set is always empty after a run. |
| `invalidation.rs:375` | `InvalidatingNodeVisitor.java:462-470` | Hermeticity check only for keys that declare `Some(Hermetic)`; `None` (not declared yet) is not checked (design doc section 3). |
| `evaluator.rs:322`, `:390` | `Injectable.inject` | Values given to `EvaluatorBuilder::inject` are injected by `build()` at the first evaluation's version, not at the first `evaluate`. |
| `evaluator.rs:362` | `InMemoryMemoizingEvaluator.ProgressReceiver` | Statistics (`postLoggingStats`) not ported. |
| `evaluator.rs:507`, `:566`, `:1074`, `:2360` | various | Skyfocus, max transitive source versions, `SkyKeyComputeState`, reset deps not ported (null / empty, Java's values without those features). |
| `evaluator.rs:571` | `AbstractInMemoryMemoizingEvaluator.java:441` | `newValue.equals(prev)` is `Key::values_equal` (D10). |
| `evaluator.rs:1050` | `ParallelEvaluator.java:668` | Cached error keys in root order (Java: `HashSet`). Kept from before. |
| `evaluator.rs:1510` | `Evaluate.run:408-413` | `isReadyToEvaluate` check dropped (partial reevaluation only). |
| `evaluator.rs:1577` `maybe_handle_dirty_node` | `AbstractParallelEvaluator.java:233-380` | A dep group is evaluated with the request code path (`eval_keys`, registering with `checkIfDoneForDirtyReverseDep`); a dep that does not finish ends the node "undone" with the dep's kind (still dirty, in flight). |
| `evaluator.rs:1698` | `analyzeDepsDoneness` | `NO_INFORMATION` (in-memory graph): no minimal-version signals. |
| `evaluator.rs:1802` `entry_seen_in_normal_evaluation` | — | Kept from the reviewed engine (masking after abort); also applies to dirty checks. |
| `evaluator.rs:1830` `register_pending_deps`, `environment.rs:48` `EnvState::pending` | `addTemporaryDirectDepsTo` + `enqueueChild`; `maybeHandleRegisteringNewlyDiscoveredDepsForDoneEntry` | A done dep read by a function is registered (temporary direct deps, reverse dep, `signalDep`) when the function returns, as in Java; when a request finds a dep not done, the deps requested so far and the request's group are registered first (what Java does when the function returns null there), then the dep is evaluated. Rdep order on a dep is Java's for single-threaded runs. |
| `evaluator.rs:2425` signalling | `signalParentsAndEnqueueIfReady` | The committer signals the reverse deps `setValue` returns before it publishes the node's outcome; a requester that registers on a done node signals itself (`enqueueChild` `DONE`). "Enqueue" is the waiting thread waking up. |
| `evaluator.rs:1904` `remove_undone_newly_requested_deps` | `SkyFunctionEnvironment.java:853` | The deps a function saw not done were registered by its requests; the registration is taken back (temporary deps and reverse dep; for an old dep `commitAndGetParents` removes the reverse dep, as in Java). A dep done by now (it signalled the node) stays. |
| `evaluator.rs:1950` | `AbstractParallelEvaluator.java:973` | The `removeUndoneNewlyRequestedDeps` step is done by the caller first. |
| `evaluator.rs:2020` | `AbstractParallelEvaluator.java:494-520` | A thrower that is not the first error keeps the registrations of deps it saw not done (state 3(b) of the 2026-11-18 review: Java's function would have thrown before evaluating them); its done deps are registered as in Java. |
| `evaluator.rs:2090` | `rdepsToBubbleUpTo` | Without requesters that closed a cycle through the node (kept from the reviewed engine). |
| `evaluator.rs:2232` | `Evaluate.run:640-656` | All requested deps done and the function returned null: Java restarts it (no new deps) and commits the child errors; the blocking engine commits without a second run. |
| `evaluator.rs:2415` | `commitAndGetParents` `evaluated(..., temporaryDirectDeps)` | The direct deps are copied only when the caller gave a progress receiver. |
| `evaluator.rs:2779-2860` bubbling | `ParallelEvaluator.bubbleErrorUp` | Kept from the reviewed engine (`never_evaluated`, `awaits_deps`); now on entries; the `isInflight` check and the dirty-parent signalling (`:395-403`) are ported. |
| `environment.rs:177` `batch_prefetch` | `SkyFunctionEnvironment.java:234-290` | `UndonePreviouslyRequestedDeps` cannot happen (no rewinding, nothing dirties a node during an evaluation); an undone dep panics with the `THROWING` inconsistency text. |
| `environment.rs:285` | `getTemporaryDirectDeps` | In normal evaluation: previously requested deps followed by this run's requests. |
| `environment.rs:967` | `addTransitiveEventsFromDepValuesForDoneNode` | `MANUALLY_REGISTERED_MARKER` cannot occur (`registerDependencies` not ported). |
| `simple_cycle_detector.rs:225`, `:430` | `SimpleCycleDetector.java:204-215, 341-352` | `BugReport.sendBugReport` (non-crashing) not ported. |
| `walkable_graph.rs:107` | `DelegatingWalkableGraph.getReverseDeps` | Reverse deps are the entry's (see (d)). |
| `progress.rs:348` | `Sets.newConcurrentHashSet` | Sharded set. |
| `grouped_deps.rs:16-20` | `GroupedDeps.numElements` | Cached count (Java: array size); `Debug` output unchanged. |
| `value_with_metadata.rs:71` | `ValueWithMetadata.NO_EVENTS` | One shared empty set (phase 1 allocated one per call). |
| `tests/*`: `NotComparableKey` (`tests.rs`), `failBuildAndRemoveValue`, `deletingDirtyNodes`, `changePruningWithEvent`, `runPartialResultOnInterruption`, `fullEventStorage_*`, `deepGraph`, `GraphTest` batch sampling | test harness | Java's `NotComparableStringValue` is a key type here (D10); restart counts are one run; a new `EmittedEventState` per Java evaluator is a `clear()`; the deep chain is built bottom-up in 16-level steps (1024 nested threads otherwise); a fixed sample instead of a seeded `Random`. |

Removed: the four `// PORT: skipped, incremental` comments (`evaluator.rs` bubbling dirty
handling and stored-event replay, `simple_cycle_detector.rs` dirty block and
`maybeHandleVerifiedCleanNode`/`maybeMarkRebuilding`), `Run::cleanup`, `Node::rdeps`,
`Node::in_progress_rdeps`, `delete_reverse_transitive_closure`.

Added in phase 3 (`evaluator.rs`, `Run::eval_dep`, `NEEDS_SCHEDULING`): the requesters that
closed a cycle through a node (`cycle_rdeps`, a blocking-engine field) are cleared when the node
is claimed again. Found by the model test (5 of 24,000 seeds): a node committed with a cycle
error by a keep_going evaluation and later rebuilt as the first error of a no-keep-going
evaluation dropped its dirty parent from `rdepsToBubbleUpTo` and `bubble_error_up` failed Java's
check "Current key ... has to be a top-level key" (`J:ParallelEvaluator.java:330-335`).

### 5.4 Behaviour changes visible to other crates

* `Evaluator::inject` and `invalidate_transient_errors` are recorded and applied at the start of
  the next `evaluate` (`J:AbstractInMemoryMemoizingEvaluator.java:134-152`); before, `inject`
  changed the graph at once. `get_existing_value` right after `inject` returns the old value until
  the next `evaluate`. Audit: no caller outside the crate reads back after `inject`
  (`blaze_sky_loading/src/precomputed.rs:112` and `blaze_bzlmod/src/skyframe/precomputed.rs:132`
  `set_on` have no callers). `inject` no longer panics during an evaluation (Java records it).
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

## 6. Not ported

By ruling (docs/decisions.md 2026-11-19; plan section 5.5): rewinding (`DirtyType.REWIND`,
`resetEvaluationFromScratch`, `SkyFunction.Reset`), partial reevaluation, `SkyKeyComputeState`,
`dependOnFuture`, `registerDependencies`, the per-key keep_going predicate, max transitive
source versions, Skyfocus. Also not ported: `GraphInconsistencyReceiver` (an inconsistency
panics with the text of Java's `THROWING` receiver), `ShortCircuitingCycleDetector`, key
interning, the evaluator statistics of `InMemoryMemoizingEvaluator.ProgressReceiver`, a parallel
and interruptible invalidation (it is sequential on the calling thread).

### 6.1 Node-entry layer

- `src/value_with_metadata.rs:240` `ErrorInfoValue.equals` (needs `ErrorInfo.equals`; unreachable from `unchangedFromLastBuild`, which returns false first for an error value).
- `src/abstract_in_memory_node_entry.rs:205` `AbstractInMemoryNodeEntry.addExternalDep`.
- `src/incremental_in_memory_node_entry.rs:395` `IncrementalInMemoryNodeEntry.markDirty(DirtyType.REWIND)`.
- `src/incremental_in_memory_node_entry.rs:541` `IncrementalInMemoryNodeEntry.resetEvaluationFromScratch`.
- `src/incremental_in_memory_node_entry.rs:556` `IncrementalInMemoryNodeEntry.clearDirectDepsForSkyfocus`.
- `src/non_incremental_in_memory_node_entry.rs:158` `NonIncrementalInMemoryNodeEntry.resetEvaluationFromScratch`.
- `src/non_incremental_in_memory_node_entry.rs:175` `NonIncrementalInMemoryNodeEntry.markDirty(DirtyType.REWIND)` (after Java's `checkArgument(dirtyType == REWIND, "Unexpected dirty type: %s")`).

### 6.2 Support classes

* `progress.rs:371` `DirtyAndInflightTrackingProgressReceiver.addToDirtySet (DirtyType.REWIND)`.

### 6.3 Engine

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
