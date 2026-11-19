# I1-B: support classes and the reference model

Worker I1-B, package I1 phase 1. Tree: worktree `i1-b` on 48b61bb plus uncommitted changes.
`J:` = `src/main/java/com/google/devtools/build/skyframe/`, `JT:` = the same under `src/test/java`.
Rust paths are relative to `crates/blaze_skyframe/src/`.

## (a) Classes

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

## (b) `// PORT:` deviations

| Site | Why |
| --- | --- |
| `progress.rs:102` `EvaluationState::get` | Java takes the value maybe wrapped in `ValueWithMetadata` and tests `justValue != null`; no `ValueWithMetadata` port yet, so the caller passes the just-value. |
| `progress.rs:301`, `:355`, `:370`, `:383` `DirtyAndInflightTrackingProgressReceiver` | Rewinding is not ported (skyframe-incremental.md section 6): no `unsuccessfullyRewoundKeys` set; `getAndClearUnsuccessfullyRewoundKeys` returns the empty set; `addToDirtySet(REWIND)` fails with `unported!`; `removeFromDirtySet`'s fallback removal is a no-op. |
| `progress.rs:484` `CompoundEvaluationProgressReceiverBase` | Java's constructor is protected; Rust has no subclassing, so it is `pub` and subclasses compose. |
| `tests/invalidation_progress_receiver.rs:19` | Java's abstract class with final `dirtied`/`deleted` becomes a trait (`invalidated`) plus an adapter implementing `EvaluationProgressReceiver`. |

Not a PORT but worth knowing: `EvaluationState::FailVersionUnchanged.version_changed()` returns
true, as Java constructs `FAIL_VERSION_UNCHANGED(false, true)` (J:EvaluationProgressReceiver.java:32).

## (c) `unported!` sites

* `progress.rs:371` `DirtyAndInflightTrackingProgressReceiver.addToDirtySet (DirtyType.REWIND)`.

## (d) The model (`tests/model.rs`, `tests/model/`)

Files: `spec.rs` (spec, edits, requests, interpreter), `error_model.rs` (`ErrorInfo` reduced),
`scratch.rs` (from-scratch semantics), `graph.rs` (`ModelGraph`, Java's incremental algorithm),
`generate.rs` (PRNG, generator), `self_tests.rs`, `table.rs`. The driver contract is the doc
comment of `tests/model.rs`.

### Rules, with Java

* Programs: steps of dep groups (one dep = `getValue`/`getValueOrThrow`, several = one
  `getValuesAndExceptions`), `IfEven` branches on an earlier dep's value (a missing dep stops the
  run), then `if (env.valuesMissing()) return null`, then events (warning: not stored for replay,
  J:../lib/events/Event.java:75; error event: stored), then throw (persistent/transient, always
  or when the result equals r) or return `(bias + sum) mod m`. Reads follow
  J:SkyFunctionEnvironment.java:613-660 (`processDepValue`) and :767-791 (`handleError`),
  including: a dep whose error has a cycle sets valuesMissing even when its exception is thrown
  to the function.
* Errors: `ErrorInfo.fromChildErrors` representative choice (J:ErrorInfo.java:63-90), children in
  request order; values with child errors carry `ErrorInfo.withValue` (J:ErrorInfoManager.java:
  55-66); thrown errors are transitively transient if any dep error is
  (J:AbstractParallelEvaluator.java:527-534); a directly transient error adds the
  error-transience dep (J:SkyFunctionEnvironment.java:422-447).
* Start of evaluate (J:AbstractInMemoryMemoizingEvaluator.java:118-152, 432-493): version 0, +1;
  in-flight keys deleted (:134); differencer lists (`SequencedRecordingDifferencer`, last inject
  of a key wins); `pruneInjectedValues` (uses `equals` on the just-value); injected keys
  invalidated; delete (reverse transitive closure, J:InvalidatingNodeVisitor.java:229-400), then
  dirty (CHANGE given keys, DIRTY reverse deps, :402-570; markChanged on a DIRTY node without
  propagation, J:IncrementalInMemoryNodeEntry.java:287-333); inject (J:ParallelEvaluator.java:
  599-632). `delete(pred)` takes every not-done node too (:212-225); `deleteDirty(age)` takes
  unenqueued dirty keys with version <= lastGraphVersion - age (:243-255).
* `ParallelEvaluator.eval` (J:ParallelEvaluator.java:641-700): all roots done: nothing runs; no
  keep_going and a root done with an error: only those errors are returned; else the
  error-transience node is injected when not done (:187-202) — not at every evaluation.
* Dirty node: J:AbstractParallelEvaluator.java:233-380 and J:DirtyBuildingState.java:126-219 as
  listed in the `graph.rs` module docs (groups in order, error-transience group compared with
  `lastChanged`, `signalDep` with `lastEvaluated`, no-keep-going error check before the group,
  `markClean` keeps both versions).
* Commit: J:SkyFunctionEnvironment.java:916-1016 and J:IncrementalInMemoryNodeEntry.java:139-163
  (`unchangedFromLastBuild`: comparable, no error on either side).
* Cycles: J:SimpleCycleDetector.java ported onto the model graph (keep_going commits; unfinished
  deps of a node on a cycle are dropped with their reverse-dep edge; unvisited stuck nodes stay
  in flight and are deleted next time; no keep_going: first unfinished root only).
* Reverse deps are derived from recorded deps (done deps; dirty last-build deps; in flight:
  temporary deps plus unchecked last-build groups).

### Where `ran` is exact and where bounded

`ModelRun::exact` is true, and `ran`, `marked_clean`, `change_pruned`, results, `dirtied`,
`deleted`, warnings and (NO_STORAGE) stored events are determined, when all hold:

1. keep_going, or no keep_going and the evaluation does not stop at an error (no function
   throws, no done dep in error is read or found in a dirty check, no verified-clean node has an
   error; J:ParallelEvaluator.java:205-238, J:AbstractParallelEvaluator.java:264-299, :360-369,
   :625-686). Then every requested node is evaluated, a node's reads depend only on its deps'
   outcomes, and a node is stuck exactly when it waits in a wait-for cycle, so the set of nodes
   whose function runs does not depend on scheduling or parallelism. The canonical depth-first
   schedule computes it.
2. no interrupt;
3. every earlier evaluation of the model was exact (else `ModelGraph::is_certain()` is false: a
   stopped evaluation leaves a schedule-dependent set of committed and in-flight nodes).

Otherwise `ran_bounds()` is `(∅, ran_upper)`: `ran_upper` is the set reachable from the roots
through every branch of the current programs, every dep the model graph records, and every such
edge recorded at an inexact evaluation. (For a no-keep-going evaluation stopped at an error the
lower bound is honestly empty: a cached error root, or the first node evaluated throwing, stops
it before anything else runs.)

Request order (crate docs "PORT: dependencies are evaluated in request order"): it can change
`ran` only in no-keep-going evaluations that stop at an error, for programs that request a group
after a step whose dep may not be done yet (Java's run asks for both and evaluates neither before
returning; Blaze evaluates the first before asking for the second). Those evaluations are never
exact here. `GenConfig::guarded_requests` generates only programs whose later steps are
`IfEven` on the first step's dep, where the two engines ask for the same deps in the same order.

### From-scratch uniqueness

keep_going: unique. No keep_going: unique only when the closure has no thrower and no cycle (all
values); otherwise a set (`Allowed::Loose`): with exceptions the first thrown wins; with cycles
only, a fresh evaluator reports the first unfinished root's cycle, while an incremental one that
finds a root done with a cycle error committed by an earlier keep_going evaluation returns exactly
the cached error roots (J:ParallelEvaluator.java:668-690) — a real Java difference the first
version of the model's from-scratch spec missed and the self-test found.

### Self-tests (run now)

* `model_matches_from_scratch_exact_mode`, `_wide_mode`, `_guarded_and_larger`: 300 / 300 / 100
  seeds; after every step the model's results pass `eval_from_scratch(...).check`; `ran ⊆
  ran_upper`, `change_pruned ⊆ ran`, `marked_clean ∩ ran = ∅`; exact keep_going warnings and
  events ⊆ a clean build's; the first evaluation runs exactly the function nodes of the closure
  and reports a clean build's events; minimality (a node that ran was absent at the start, never
  finished, or something in its from-scratch closure was injected, set, invalidated, or
  transient errors were invalidated with a transient thrower in it). Coverage asserts (marked
  clean, pruned, cycles, exceptions, transient, aborted, interrupted all occur). Checked once at
  6000 seeds per mode (all pass, 4 s).
* `no_op_edits_run_nothing` (all comparable): re-injecting every leaf's value runs and dirties
  nothing; setting a function's program to itself reruns only that node, which is pruned.
* `generator_is_deterministic`.
* `table.rs`: 17 tests, 19 hand-derived scenarios (leaf change pruned at the middle node, first
  group changed skips the second, transient error, equal injection, opaque injection,
  invalidate same value, opaque middle node, start/stop failing, caught error, cycle appears and
  disappears, delete closure, delete with empty predicate and its contrast, deleteDirty ages 1
  and 2, cached error root without keep_going, batch group, invalidated leaf with a cached error
  above, no-keep-going cycle nodes deleted and rerun).

### Known gaps

* Events with `EventFilter::FULL_STORAGE` are not simulated: change pruning then compares
  `ValueWithEvents` with `NestedSet.shallowEquals` (J:ValueWithMetadata.java:105-124), which
  makes pruning stricter; the driver must use NO_STORAGE for exact comparisons.
* Error bubbling is not simulated (it only happens in inexact evaluations); the exception of an
  error that also has cycles is not compared (it depends on how far a dirty node got before it
  got stuck, which a clean build does not reproduce).
* Exactness is global: after one inexact evaluation the model stays inexact for the scenario.
* No key is ever injected that has deps (`pruneInjectedValues`' "has deps" branch is modelled but
  not generated); catastrophes, rewinding, max transitive source versions are not modelled.
* Interrupts are modelled as "anything in the upper set may have happened"; the driver must tell
  the model whether the evaluation was interrupted.
