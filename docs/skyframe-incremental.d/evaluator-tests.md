# I1 phase 3: the evaluator tests (worker I1-E)

Tree: worktree `i1-a`, base 48b61bb plus the uncommitted phase-1/2 work plus this work; nothing
committed. Files under `crates/blaze_skyframe/` only. `J:` = `src/main/java/com/google/devtools/
build/skyframe/`, `JT:` the same under `src/test/java`. Rust paths are relative to
`crates/blaze_skyframe/src/`.

## Ledger

Counts are `@Test` methods at the pin (`@Test` lines; ParallelEvaluatorTest has 91, the 117 hits
of `grep -c @Test` include its `@TestParameter` fields and parameters). Every Java `@Test` has a
Rust test under its snake_case name with a `// Java: ...Class.method` marker; none is missing
(checked with a script that pairs every `@Test` with a marker and reads the `#[ignore]`).

| Java class | `@Test` | ported, passing | ported, ignored (body kept) | stub (not_ported.rs) |
| --- | --- | --- | --- | --- |
| `JT:MemoizingEvaluatorTest` | 155 | 134 | 5 | 16 |
| `JT:ParallelEvaluatorTest` | 91 | 71 | 1 | 19 |

Ported, ignored (body kept, each `#[ignore]` names the documented deviation):

* MemoizingEvaluatorTest `dirtyChildEnqueuesParentDuringCheckDependencies_ThrowDoesntEnqueue`,
  `..._NoThrow`: restart model (lib.rs `Model`): Java's top runs twice in the first build (asserted:
  `numTopInvocations == 2`); here once. Fails on that assertion (checked with `--ignored`).
* MemoizingEvaluatorTest `dirtyAndChangedValueIsChanged`: invalidation is sequential (lib.rs `Not
  ported`, `invalidation.rs`); the listener needs two invalidation threads, so it waits for itself
  until the 60 s timeout (checked).
* MemoizingEvaluatorTest `cachedErrorCausesRestart`: restart model; the listener waits in
  errorKey's `markClean` for topKey's second run, topKey's request blocks on errorKey: both wait
  until the timeouts (checked, 120 s).
* MemoizingEvaluatorTest `duplicateUnfinishedDeps_NoKeepGoing`: requests evaluate their dep (lib.rs
  `Model`): child's function panics by design ("shouldn't get here") because the request evaluates
  it; Java's first exception ends the evaluation first (checked).
* ParallelEvaluatorTest `declaresDifferentDepsAfterRestart`: restart model; the function expects
  `valuesMissing()` after each request (checked).

Stubs (`tests/not_ported.rs`, 35):

* by ruling: `SkyFunction.Reset` / rewinding (14: `resetSelfOnly_*` 6, `rewindOneDep`,
  `twoParentsRewindSameDep_*` 3, `depWithDoneParentEvaluatesToErrorAfterRewind_*`,
  `rewindTransitiveDep`, `evaluationAbortedWithRewoundNodeOnInvalidationPath_*` 2); partial
  reevaluation (5: `partialReevaluation*`); `SkyKeyComputeState` (4); `dependOnFuture` (4:
  `enqueueDoneFuture`, `enqueueBadFuture`, `dependsOnKeyAndFuture`, `externalDep`);
  `GraphTraversingHelper` (2: `declareDependenciesAndCheckIfValuesMissing`, `...NotValuesMissing`).
* re-examined, kept with a sharper reason:
  * `injectGraphTransformer_transformedGraphUsedForInMemoryGraph`: the graph is the concrete
    `InMemoryGraphImpl`; the transformer is ported as a listener hook, so `getInMemoryGraph` cannot
    return a substituted (mock) graph.
  * `validateExceptionTypeInDifferentPosition`, `validateExceptionTypeWithDifferentException`:
    `SkyFunctionException.validateExceptionType` checks Java exception classes;
    `get_value_or_throw` takes one concrete Rust type, there is no class to check.
  * `testParallelSkyFunctionComputation_runnablesOnBothCurrentAndExternalThreads`,
    `..._runnablesOnExternalThreadsOnly`: `Environment.getParallelEvaluationExecutor` and an
    injected `QuiescingExecutor` are not ported.
  * `customKeepGoingPredicate_differentPerKey`: the per-key keep_going predicate is not ported.

Partly ported (variants of a parameterized test): `ParallelEvaluatorTest` cycle tests with
`isCycleNodePartialReevaluation = true` (partial reevaluation; noted at the tests).

## Ported this phase

Moved out of `not_ported.rs` (all passing unless listed above):

* The 25 "left for the next part": `receiverToldOfVerifiedValueDependingOnCycle`,
  `changeCycle_NoKeepGoing`, `changeCycle_KeepGoing`, `cycleAndSelfEdgeWithDirtyValue`,
  `nodeInvalidatedThenDoubleCycle`, `incompleteDirectDepsAreClearedBeforeInvalidation`,
  `incompleteDirectDepsForDirtyValue`, `singleValueDependsOnManyDirtyValues`,
  `dirtyChildrenProperlyRemovedWith{Invalidate,Reevaluate}{Remove,Keep}Error` (4),
  `manyDirtyValuesClearChildrenOn{Error,Interrupt,SecondFail}` (3),
  `changedBuildAfterFailedThenSuccessfulBuild`, `failedDirtyBuildInBuilder`,
  `dirtyWithOwnErrorDependsOnTransientErrorTurningGood`,
  `dirtyWithRecoveryErrorDependsOnErrorTurningGood`, `changePruningWithIntermittentEvent`,
  `depEventPredicate`, `twoRailLeftRightDependenciesWithFailure`,
  `removedNodeComesBackAndInvalidates`, `removedNodeComesBackAndOtherInvalidates`,
  `removedInvalidatedNodeComesBackAndOtherInvalidates`.
* The 12 graph-listener tests: ParallelEvaluatorTest
  `interruptedEvaluatorThreadAfterEnqueueBeforeWaitForCompletionAndConstructResult`;
  MemoizingEvaluatorTest `alreadyAnalyzedBadTarget`, `cycleAndErrorAndReady`,
  `removeReverseDepFromRebuildingNode`, `raceClearingIncompleteValues`,
  `shutDownBuildOnCachedError_Done`, `shutDownBuildOnCachedError_Verified`,
  `cleanReverseDepFromDirtyNodeNotInBuild` (passing); `dirtyChildEnqueues...` (2),
  `dirtyAndChangedValueIsChanged`, `cachedErrorCausesRestart` (ignored, above).
* Graph transformers: `injectGraphTransformer_transformedGraphUsedForEvaluation`,
  `injectGraphTransformer_multipleTransformersAppliedInOrder`.
* Re-examined: `customKeepGoingPredicate_sameForAllKeys` (the constant predicate is the keep_going
  flag; both values); `declaresDifferentDepsAfterRestart`, `duplicateUnfinishedDeps_NoKeepGoing`
  (ignored, above).

`PORT` choices in these ports (each marked at the test): `incompleteDirectDeps*` and
`cycleAndErrorAndReady` request a group as one batch where Java's function gets null for the
first dep and goes on (a request blocks here, and the first dep waits for the second);
`interruptedEvaluatorThread...` uses the listener at B's top-level `addReverseDepAndCheckIfDone`
in place of Java's Mockito spy and returns instead of throwing (the evaluation then stops on the
interrupt); listener waits that Java ends with `InterruptedException` poll the thread's
`blaze_shell::ThreadInterrupt` (the evaluator's `interruptInFlightTasks`) or, for Java's main
thread, the evaluator's interrupt flag.

## Ported tests that asserted less than Java, completed

* `cycleAboveIndependentCycle`: the second half (leafKey changed, cKey invalidated) was left out;
  now ported (it re-asserts the first result, as Java does), with Java's keys (`nonHermeticKey`
  c and leaf) and `makeGraphDeterministic`.
* `depMessageBeforeNodeMessageOrNodeValue`: the graph listener (SET_VALUE of top after the dep
  warning) and the `DelegatingEventHandler` check were missing; now ported.
* `errorOnlyBubblesToRequestingParents`: was a fresh build with an extra latch; now Java's
  incremental scenario (first build without errors, then error and slow changed).
* `absentParent`: Java first builds absentParent on a good value and then turns it into an
  error; now ported as such.
* `cachedChildErrorDepWithSiblingDepOnNoKeepGoingEval`: same (error turned on incrementally);
  the extra `singleton_error` assertions, which Java does not make, are dropped.
* `persistentErrorsNotRerun`: used `computed_count`; now `getEnqueuedValues()` sets as Java,
  through Java's helper `runTestPersistentErrorsNotRerun`.
* `sameDepInTwoGroups`: the listener that fails when leaf4 is considered in the second build
  was missing; now ported (the direct-deps assertions, not in Java, are dropped).
* `raceConditionWithNoKeepGoingErrors_InflightError` (MemoizingEvaluatorTest) and
  `raceConditionWithNoKeepGoingErrors_FutureError`, `errorBubblesToParentsOfTopLevelValue`
  (ParallelEvaluatorTest): used substitute latches for Java's graph listener; now the listener
  as in Java (Rust-only extra count assertions dropped).

Not re-audited line by line: the older ports of non-incremental tests (MemoizingEvaluatorTest
`smoke` .. `limitEvaluatorThreads`, ParallelEvaluatorTest cycle and error tests in `tests.rs` and
`parallel_evaluator_test.rs`) other than those with a `PORT` note about a missing listener or
incremental step; their markers are `// Java: Class.method` without a path and line.

## Engine and harness changes

No engine defect was found by the ported tests: every failure was a model difference (above) or
a test bug. The changes are the test hook and test-only determinism:

| File | Function / item | Change |
| --- | --- | --- |
| `graph_listener.rs` (new, `#[cfg(test)]`) | `EventType`, `Order`, `Reason`, `MarkDirtyAfterContext`, `Context`, `Listener`, `GraphListeners` | Port of `JT:NotifyingHelper.java` `Listener`/`EventType`/`Order`/`MarkDirtyAfterContext` and of the transformer stacking (`inject`: BEFORE to the last injected first, AFTER to the first injected first). |
| `lib.rs` | `notify_graph_listener!` macro, `#[cfg(test)] mod graph_listener` | The hook; expands to nothing outside `cfg(test)`. |
| `in_memory_graph.rs` | `InMemoryGraphImpl::listeners_for_testing` (`#[cfg(test)]` field), `create_if_absent` | Listener storage; `CREATE_IF_ABSENT` BEFORE. |
| `evaluator.rs` | `Run::eval` | `GET_BATCH` BEFORE with `PRE_OR_POST_EVALUATION` for each root (`J:ParallelEvaluator.java:645`). |
| `evaluator.rs` | `Run::eval_dep` | `ADD_REVERSE_DEP` / `CHECK_IF_DONE` BEFORE (before the node lock) and AFTER (after it is released, in each branch); `SIGNAL` around the requester's signal on `DONE`. Non-test change: the `AlreadyEvaluating` branch clones the outcome and drops the lock before returning (was: returned the clone with the lock held; same result). |
| `evaluator.rs` | `Run::evaluate_node` | `IS_READY` BEFORE where Java calls `isReadyToEvaluate` (the call itself stays dropped). |
| `evaluator.rs` | `Run::get_lifecycle_state` (new) | `getLifecycleState` with `GET_LIFECYCLE_STATE` BEFORE/AFTER; used in `evaluate_node` and `maybe_handle_dirty_node` (same single lock as before). |
| `evaluator.rs` | `Run::maybe_handle_dirty_node` | `REMOVE_REVERSE_DEP` (error transience, unused deps), `CHECK_IF_DONE`, `IS_READY`, `MARK_CLEAN` events; test-only `deterministic_mark_clean`. |
| `evaluator.rs` | `Run::register_done_deps` | `ADD_REVERSE_DEP`/`CHECK_IF_DONE` around each dep; `SIGNAL` BEFORE for the batch before the requester's lock and AFTER after it (`#[cfg(test)]` loops). |
| `evaluator.rs` | `Run::resolve` (child error edge), `Run::set_error` | `ADD_REVERSE_DEP`/`CHECK_IF_DONE`; `SIGNAL` for the error transience dep (BEFORE before the node lock). |
| `evaluator.rs` | `Run::commit_and_get_parents` | `REMOVE_REVERSE_DEP` per old dep; `SET_VALUE` BEFORE/AFTER around the `setValue` block (context: the plain value); test-only `deterministic_rdeps` on the returned reverse deps. |
| `evaluator.rs` | `Run::signal_parents_and_enqueue_if_ready`, `Run::signal_parents_on_abort` | `SIGNAL` BEFORE/AFTER per parent (on abort, BEFORE is under the parent's lock: the doneness check and the signal are one step). |
| `evaluator.rs` | `Evaluator::deterministic_rdeps`, `deterministic_mark_clean` (new, `#[cfg(test)]`) | `JT:DeterministicHelper.java` `DeterministicNodeEntry.setValue`/`markClean`: reverse deps to signal in alphabetical order when `deterministic_for_testing`. |
| `simple_cycle_detector.rs` | `maybe_handle_verified_clean_node` | Test-only `deterministic_rdeps` on `markClean`'s reverse deps. |
| `invalidation.rs` | `DirtyingNodeVisitor::visit` (dirtying path) | `IS_CHANGED`, `IS_DIRTY`, `MARK_DIRTY` BEFORE/AFTER. Non-test change: `isChanged` (read into `entry_is_changed`), `isDirty` and `markDirty` are separate lock acquisitions, as Java's separate `synchronized` calls (`J:InvalidatingNodeVisitor.java:532-556`); invalidation is sequential, so no other thread runs in between, and a `markDirty` that finds the node dirtied returns `None`, handled as before. |
| `tests.rs` | `Spec::computer` / `Computer`, `TestFunction::compute` | Java's `setComputedValue(ValueComputer)` (deps by key, in request order); `None` is `CONCATENATE` as before. |
| `tests.rs` | `Tester::set_chained_key`, `MemoizingTester::{try_eval_with_reporter, make_graph_deterministic, inject_graph_listener_for_testing}` | Harness additions for the ports. |

Not emitted (listed in `graph_listener.rs`): `EVALUATE`, `ADD_EXTERNAL_DEP`, `GET_VALUES`,
`GET_TEMPORARY_DIRECT_DEPS`, `GET_VALUE_WITH_METADATA`, `ADD_TEMPORARY_DIRECT_DEPS`,
`GET_ALL_DIRECT_DEPS_FOR_INCOMPLETE_NODE`, `RESET_FOR_RESTART_FROM_SCRATCH`, `REMOVE`,
`GET_BATCH` for dep requests, and the entry calls of value injection, error bubbling, the cycle
detector and `remove_undone_newly_requested_deps` (Java makes no `removeReverseDep` call there).
No ported test listens for them.

## Probes of the two delicate areas

(a) Thrower that is not the first error keeps registrations (`evaluator.rs` "Threw" path,
`J:AbstractParallelEvaluator.java:494-520`). No Java test builds this state directly (the closest
are `dirtyChildrenProperlyRemoved*`, `manyDirtyValuesClearChildrenOn*`, all passing). Probe
`engine_test::thrower_after_first_error_leaves_no_reverse_dep_after_in_flight_deletion`: p
requests d, d finishes after the first error (so p sees it not done), p throws. Checked during
development that the state is reached (after the evaluation p is in flight and d is done with
reverse dep p; Java's d would have none). The next evaluation deletes p (in flight) and removes
it from d's reverse deps: d's reverse deps do not contain p, invalidating d dirties nothing
stale, and p rebuilds to d's new value. Passing, 10 iterations per run.

(b) In-flight deletion of a dirty node (`J:InvalidatingNodeVisitor.java:229-400`). Covered by
the Java ports `cleanReverseDepFromDirtyNodeNotInBuild` (dirty top interrupted in flight, deleted;
its dirty dep keeps working), `manyDirtyValuesClearChildrenOnInterrupt`/`OnError`/`OnSecondFail`,
`dirtyChildrenProperlyRemoved*` (4), `incompleteDirectDepsForDirtyValue`; plus probe
`engine_test::in_flight_dirty_node_is_deleted_with_its_dirty_parents` (dirty mid interrupted while
rebuilding, its dirty parent top not requested: the next evaluation deletes both, leaf has no
reverse deps left, top rebuilds). All passing.

## Results

See the final report for the counts and the 30-run result.
