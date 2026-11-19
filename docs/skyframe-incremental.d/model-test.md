# I1-F: the model test (the real evaluator against the reference model)

Worker I1-F. Tree: worktree `i1-b` on 48b61bb plus uncommitted changes (nothing committed).
`J:` = `src/main/java/com/google/devtools/build/skyframe/`. Rust paths are relative to
`crates/blaze_skyframe/src/`.

The driver is `tests/model/driver.rs` (tests `model_driver`,
`model_driver_seed_22969_rebuilt_cycle_member_bubbles_to_its_parent`,
`model_driver_seed_3984_bubbling_parent_throws_its_own_exception`). The contract it implements is
the doc comment of `tests/model.rs`.

## Set-up

* Keys: `ModelKey(n)` (function `MODEL`, `values_equal` is `==`) for nodes whose
  `NodeSpec::comparable` is true, `OpaqueModelKey(n)` (function `MODEL_OPAQUE`, never equal)
  for the others; both `Debug` as `n{id}`, `Value = i64`, `HERMETICITY =
  Some(Nonhermetic)`. One `ModelFunction` serves both; it reads the current spec from shared
  state (`Shared::spec`, a mutex never held across a request), counts runs per node, and
  interrupts the evaluator (`Evaluator::interrupt` through a `Weak<Evaluator>`) when the run
  counter reaches the request's `interrupt_after_runs`. Programs run through the model's
  `run_program` with an adapter: one dep is `get_value_dyn` / `get_value_or_throw_dyn::<
  SomeErrorException>`, several are one `get_values_and_exceptions` read with `get` /
  `get_or_throw` in order. Completed runs report `Event::warn(warning_text)` and
  `Event::error(stored_event_text)`; then the value, the exception
  (`SomeErrorException(exception_message(n))`, persistent or transient) or `Ok(None)`.
* Evaluator: `keep_edges(true)`, a recording progress receiver (`dirtied`, `deleted`,
  `change_pruned` callbacks, `evaluated(SUCCESS_VERSION_UNCHANGED, directDeps != null)`), and
  `EventFilter::NO_STORAGE` or `FULL_STORAGE`.
* Edits: `SetLeaf` is `inject_dyn`; `SetProgram` puts the program into the shared spec, then
  `invalidate`; `Invalidate` is `invalidate`; `Delete` is `delete(|k, _| id in nodes)`;
  `DeleteDirty` is `delete_dirty(age)`; `InvalidateTransientErrors` is
  `invalidate_transient_errors()`. Each edit is also applied to the `ModelGraph`.
* Before each evaluation: `emitted_event_state().clear()`, a new `StoredEventHandler`, the
  run records reset. After it: `clear_interrupt()`. An evaluation is "interrupted" for the model
  (`ModelGraph::evaluate_interrupted`) when `evaluate` returned `Err(Interrupted)` or the function
  fired the interrupt; else `ModelGraph::evaluate`.
* Seeds: generator config by `seed % 3`: `GenConfig::default()` (exact), `GenConfig::wide()`
  (no-keep-going stops, interrupts), and `guarded_requests` with up to 14 nodes and 10 steps.
  Each scenario runs in four modes, each with its own evaluator and model: parallelism 1 or the
  request's parallelism raised to at least 2, times `NO_STORAGE` or `FULL_STORAGE`.

## What is compared, after every evaluation

Exactly:

1. The roots' results (contract mapping to `RootResult`) pass `eval_from_scratch(spec, roots,
   keep_going).check`, for the incremental evaluator and for a fresh evaluator (built from the
   current spec, every leaf injected, same roots, keep_going, parallelism and event filter).
2. Where the from-scratch outcome is unique (`Expected::unique`): incremental equals fresh per
   root: value, error presence, exception message and `isTransitivelyTransient`, cycle infos as
   sets (narrowed, see below); and `hasError`. Where it is not unique, both lie in the allowed set
   (item 1).
3. Where the model is exact (`ModelRun::exact`): results and `hasError` equal the model's.
   `NO_STORAGE`: the set of nodes whose function ran equals `run.ran`; the `changePruned`
   callbacks equal `run.marked_clean`; the version-unchanged `evaluated` keys equal
   `run.change_pruned`; reported warnings equal `run.warnings` and error-kind events equal
   `run.stored_events`.
4. While the model was certain before the evaluation: the `dirtied` and `deleted` callback sets
   equal `run.dirtied` / `run.deleted` (`MKey::Et` is `ErrorTransienceValue.KEY`), interrupted
   evaluations included.
5. The keys in flight at the end of an evaluation are all `deleted` at the start of the next
   (J:AbstractInMemoryMemoizingEvaluator.java:134).
6. `FULL_STORAGE`, when the evaluation completed, is keep_going or has a unique outcome, and the
   from-scratch closure has no node stuck in a cycle: the set of error-kind (stored) events
   reported equals the set a fresh `FULL_STORAGE` evaluator reports for the same roots
   ("replay for nodes that are not re-run"), and the warnings reported are a subset of the
   fresh evaluator's. `blaze_events` stores every event kind but `PROGRESS`, `WARNING`, `INFO`
   and `DEBUG` (`Event::store_for_replay`, Event.java:75), so warnings are reported only by
   functions that run, and only error events are compared as sets.
7. Graph invariants (read through crate-internal access; reverse deps computed from the stored
   list and the delayed operations without consolidating): every reverse dep is in the graph; a
   done or dirty node `p` listed as reverse dep of `d` has `d` among its deps (dirty: last-build
   deps); every dep of a done node is in the graph, done, has the node as reverse dep, and has
   `lastChanged <= ` the node's `lastEvaluated`; done and dirty nodes have `lastChanged <=
   lastEvaluated <= graph version`; every last-build dep of a dirty node has it as reverse dep;
   the nodes in flight (not done, evaluating) are exactly the keys the
   `DirtyAndInflightTrackingProgressReceiver` holds in flight.
8. While the model is certain after the evaluation: the whole graph against the model's: every
   key present in one is present in the other, same state (done, dirty, in flight), same
   `lastChanged` / `lastEvaluated`, same dep groups (done) or last-build groups (dirty), same
   value and error class (exception origin and transience, or cycle) for done nodes.

By bounds:

* `ran` when the model is not exact (`NO_STORAGE`): `ran ⊆ run.ran_upper`.
* `ran` with `FULL_STORAGE` (always): `run.ran_upper ⊇ ran`, and `ran ⊇ run.ran` when the model
  is exact and no `DeleteDirty` ran yet in the scenario (a node the model re-runs is changed or
  has a dep whose value changed; fewer nodes pruned keeps that so).
* `NO_STORAGE`, inexact: every reported warning and error event names a node whose function ran.

## Narrowed comparisons (each with its reason and how often it applied)

Counts are from the long run below (seeds 0..24000, four modes each).

1. **Cycle infos, incremental vs fresh** (`compare_with_fresh`, counter `cycle_infos_differ`).
   Java's `SimpleCycleDetector` reports cycles that depend on the graph an evaluation leaves:
   it skips done nodes and takes their stored errors (J:SimpleCycleDetector.java:196-199, and
   `getChildrenErrorsForCycleChecking` :253-257), walks each in-flight node's temporary direct
   deps (:286-305), which for a dirty node are the last-build groups re-requested so far
   (J:AbstractParallelEvaluator.java:233-380), and in keep_going drops the unvisited children of
   a node found on a cycle (:238-248 `removeDescendantsOfCycleValue`). An incremental evaluation
   that finds a dep done with a cycle error from an earlier evaluation, or a dirty node checking
   old groups, reports other cycles than a fresh one (seen: fewer cycles, more cycles, a longer
   or shorter cycle through the same nodes). When the sets differ both must still be non-empty,
   and every reported cycle must be a real cycle of the current spec reached from the root (each
   edge of path and cycle is a dep some branch of the program may request). Applied: see the
   counters table.
2. **Exception with cycles**: an error that has cycle infos is compared by "has cycles" only
   (not its exception or transience), as the model does (`ErrClass::Cycle`; model notes,
   "Known gaps": it depends on how far a dirty node got before it was stuck). Every comparison
   of an error with cycles.
3. **FULL_STORAGE and the model graph** (`check_graph_against_model`, counter
   `full_storage_graph_differs`): the model does not simulate `NestedSet.shallowEquals`
   (J:ValueWithMetadata.java:105-124), so with `FULL_STORAGE` a node the model prunes or marks
   clean may re-run, and versions differ; a dirty node the model leaves in a dirty check stuck on
   a cycle may be rebuilt and record the current program's deps (seed 1656: n3's stored event
   disappeared, so n3 counted as changed and n4 was rebuilt with a third group). Then only values
   and error classes of nodes done in both graphs are compared.
4. **FULL_STORAGE and `deleteDirty`** (counter `full_storage_invalidation_not_compared`):
   `deleteDirty` picks nodes by version (J:AbstractInMemoryMemoizingEvaluator.java:243-255,
   `getVersion().atMost(threshold)`), and FULL_STORAGE versions differ from the model's (item 3),
   so from the first `DeleteDirty` of a scenario on, the `dirtied` / `deleted` sets and the lower
   bound of `ran` are not compared in FULL_STORAGE mode (seeds 3792, 7475, 10922).
5. **No keep_going, a root done with an error** (counter `cached_error_stop`): the evaluation
   returns the cached error roots only (J:ParallelEvaluator.java:668-690). The fresh-evaluator
   comparison (item 2) is skipped; the result is compared with the model (exact there) and with
   the allowed set. (With a unique from-scratch outcome this cannot happen: no node can be in
   error.) `ModelRun::cached_error_stop` is an annotation added for this count.
6. **Interrupt fired, evaluation returned normally** (counter `interrupt_fired_but_completed`):
   the last function fired the interrupt after its last request. The result is checked against
   the from-scratch semantics and the fresh evaluator; the model is told "interrupted" (bounds
   only from then on). Java likewise returns normally when no work notices the interrupt.

Not compared: the set of keys reported `evaluated` with a changed version (the model has no such
set); stored events of no-keep-going evaluations that stop at an error, and of closures with a
cycle (in a cycle a dirty node's cached deps are not replayed the way a fresh evaluation reports
them; the contract excludes them).

## Engine fix

* `evaluator.rs` `Run::eval_dep` (NEEDS_SCHEDULING branch, line 1358):
  `state.cycle_rdeps.clear()` when a thread claims a node. `NodeState::cycle_rdeps` (blocking
  engine state, the requesters whose request closed a cycle through the node) was never
  cleared. Before incremental evaluation a node with such requesters never survived the
  evaluation; now one committed with a cycle error by a keep_going evaluation stays in the graph.
  When it was later rebuilt and threw the first error of a no-keep-going evaluation, its stale
  cycle requesters were filtered out of `rdeps_to_bubble_up_to` (`evaluator.rs` `resolve`, the
  fail-fast commit), so the error did not bubble to its dirty parent and `bubble_error_up`
  panicked "IllegalStateException: Current key n3 has to be a top-level key: {n4}" (Java's
  check, J:ParallelEvaluator.java:330-335; Java's `rdepsToBubbleUpTo` is
  `env.commitAndGetParents`, J:AbstractParallelEvaluator.java:537-541, which includes the dirty
  parent that re-registered with `checkIfDoneForDirtyReverseDep`). Found by 5 seeds of 24000
  (8827, 20410, 21538, 22969, 23257; wide config, sequential, NO_STORAGE); regression test
  `model_driver_seed_22969_rebuilt_cycle_member_bubbles_to_its_parent`.
* `progress.rs` `DirtyAndInflightTrackingProgressReceiver::inflight_keys_for_testing`
  (`#[cfg(test)]`, line 464): read-only snapshot of the in-flight set for invariant 7.

## Model changes

* `tests/model/scratch.rs` `eval_from_scratch` (lines 349-376): without keep_going the allowed
  exception origins of a root were the keep_going throwers it reaches. Error bubbling re-runs each
  parent on the path from the failed node with the bubbled errors (J:ParallelEvaluator.java:
  290-470, `createForError` :414-420); `getValueOrThrow` hands it the bubbled exception
  (J:SkyFunctionEnvironment.java:464-467 and :767-791 `handleError`: "The exception may only
  propagate in keepGoing mode or during error bubbling"), and a parent that catches it and throws
  its own exception replaces the error (J:ParallelEvaluator.java:433-447). Such a parent need
  not throw in keep_going (there it may wait on a cycle through another dep, which bubbling never
  evaluates). Seed 3984 (fresh evaluator, parallelism 2): n4 = `[n2?0, n1] throws transient`, n2
  above a self-cycle and a thrower n3; the root's error was n4's own exception. Origins are now
  the keep_going throwers plus every node of the closure that can throw and reaches a keep_going
  thrower. The table tests and self-tests still pass. Regression test
  `model_driver_seed_3984_bubbling_parent_throws_its_own_exception`.
* `tests/model/graph.rs` `ModelRun::cached_error_stop` (line 176, set at line 687): an
  annotation (the cached-error branch of `evaluate`), not a rule change.
* `tests/model.rs`: `mod driver;`; the contract's doc comment points to the driver and names
  both environment variables; the module-wide `#![allow(dead_code)]` is gone (targeted allows on
  `generate::generate` and `Expected::throwers`, which no caller uses).

No generator change.

## Coverage counters and results

Tree: worktree `i1-b`, base 48b61bb plus the uncommitted I1 work and this work.

| Counter | Fixed seeds 0..500 | Long run 0..24000 |
| --- | ---: | ---: |
| scenario runs (seeds x 4 modes) | 2000 | 96000 |
| evaluations | 14656 | 704000 |
| exact (model) | 13552 | 654890 |
| edits: set_leaf / set_program / invalidate | 14192 / 6188 / 3868 | 684196 / 299260 / 183440 |
| edits: delete / delete_dirty / invalidate_transient_errors | 2524 / 1972 / 2968 | 121732 / 98072 / 146992 |
| evaluations with a change-pruned rebuild | 1469 | 81787 |
| evaluations with a marked-clean node | 434 | 22855 |
| evaluations that deleted in-flight nodes | 2123 | 104915 |
| evaluations with a dirty node in a cycle | 988 | 49646 |
| transient error re-runs | 112 | 5601 |
| injected-equal no-ops | 1244 | 61574 |
| no-keep-going stops | 2800 | 132641 |
| interrupted evaluations | 140 | 7754 |
| interrupt fired, evaluation completed (narrowing 6) | 0 | 170 |
| cached-error stops (narrowing 5) | 200 | 8460 |
| results compared with a fresh evaluator (unique) | 11716 | 563605 |
| of which cycle infos differed (narrowing 1) | 128 | 8500 |
| results checked against the allowed set only (not unique) | 2800 | 132641 |
| FULL_STORAGE stored-event sets compared with a fresh evaluator | 4422 | 211725 |
| graph compared with the model | 13552 | 654890 |
| of which FULL_STORAGE structure differed (narrowing 3) | 118 | 3702 |
| FULL_STORAGE invalidation not compared after a DeleteDirty (narrowing 4) | 2590 | 128324 |

Failures: none at the end (fixed seeds: 0; long run: 0 of 24000 seeds). Before the fixes the
long run found: the engine defect (5 seeds), the model's allowed set (1 seed), and the driver's
FULL_STORAGE comparisons that narrowings 3 and 4 now cover (seeds 2, 30, 80, 83, 1656, 2105,
2546, 3792, 7475, 10922).

The test asserts at least one change-pruned rebuild, marked-clean node, deleted in-flight node,
dirty node in a cycle, transient error re-run, injected-equal no-op, no-keep-going stop,
interrupt, unique result compared with a fresh evaluator, FULL_STORAGE event comparison, graph
comparison with the model, and every edit kind.

Whole crate, `scripts/cargo-capped test -p blaze_skyframe` 30 times in a row: every run 388
passed, 0 failed, 189 ignored (before this work: 385 / 0 / 189). `scripts/cargo-capped clippy -p
blaze_skyframe --all-targets -- -D warnings`: clean.

## Running

* Normal: `scripts/cargo-capped test -p blaze_skyframe model_driver` (500 seeds, about 10 s on
  an idle machine in a debug build; 45 s under the load of other agents).
* Long mode: `BLAZE_SKYFRAME_MODEL_SEEDS=<count>` and optionally
  `BLAZE_SKYFRAME_MODEL_SEED_START=<first>`; `BLAZE_SKYFRAME_MODEL_DUMPS=<n>` prints the
  scenario dumps of the first n failing seeds (default 1); `BLAZE_SKYFRAME_MODEL_TRACE=1`
  prints the graph before and the result after each evaluation. `scripts/cargo-capped` runs
  Cargo under `systemd-run`, which does not pass the environment through, so run the test binary
  (the path `scripts/cargo-capped test -p blaze_skyframe --no-run` prints) under
  `systemd-run --user --collect --pipe --wait -p MemoryMax=12G -p MemorySwapMax=0
  --setenv=BLAZE_SKYFRAME_MODEL_SEEDS=4000 --setenv=BLAZE_SKYFRAME_MODEL_SEED_START=0 <binary>
  tests::model::driver::model_driver --nocapture`.
* A failure prints the seed, generator config, mode, step, the failed check, and a
  deterministic dump of the scenario (spec one node per line, then per step the edits and the
  request). Replay: the same with `SEED_START=<seed>` and `SEEDS=1`, or add a one-line test
  calling `run_one_seed(<seed>)`.

## Known gaps

* Interrupts come only from inside a function (after N runs), not from another thread.
* `FULL_STORAGE` change pruning is compared by bounds only (the model does not simulate it).
* Cycle infos are compared with a fresh evaluator only as "real cycles of the spec" when they
  differ; nothing checks which cycles Java would report for a given incremental state.
* Catastrophes, rewinding, `keep_edges(false)` and key types that are hermetic are not
  generated.
