# Response to review 2026-11-18: blaze_skyframe

Date 2026-11-18. Answers every finding of `2026-11-18-skyframe.md`.

Tree: HEAD `88dbfa60f523aa6271a0f7407d5df20b980934f3` plus uncommitted changes. Mine are
`crates/blaze_skyframe/**`, the "Skyframe" section of `PORTING.md` and this file. Nothing is
committed. No file of `blaze_sky_loading` was edited (see "blaze_sky_loading" below). Other agents
have uncommitted work in the same tree; where a number below needed a tree without it, that is
said.

Test names are in `crates/blaze_skyframe/src/tests.rs` (`tests::<name>`) or in
`crates/blaze_skyframe/src/tests/<file>.rs` (`tests::<file>::<name>`).

## Summary

| finding | state |
|---|---|
| S1-1 keep_going catastrophe loses the bubbling path, `evaluate` panics | fixed |
| S1-2 every in-flight parent sees the exception, events reported per parent | fixed |
| S1-3 `in_error_bubbling()` always false | fixed |
| S2-1 error under a cycle never reports the cycle | fixed |
| S2-2 `SkyframeLookupResult` only knows its own call's keys | fixed |
| S2-3 failed thread spawn leaves a node in progress forever | fixed |
| S2-4 shape of bubbled errors (two corners) | fixed, both |
| S3-1 Java tests neither ported nor ignored | fixed: 140 ported, 124 ignored stubs, none unaccounted |
| S3-2 unported engine API without marker | fixed (markers) |
| S3-3 PORT comments that claim a false equivalence | fixed; the crash message is now ported instead of described |
| S4 `ErrorTransienceValue.KEY` not rejected | fixed |
| S4 interrupt after all work finished discards the result | deferred |
| S4 `child_error_infos` dedup by pointer | not changed (the review found no observable difference) |

No finding is disputed.

## What the engine does now

The three S1 findings and S2-1 and S2-4 have one cause: errors were handled "during normal
compute". That is replaced by a port of Java's two phases.

Normal evaluation (`Run::resolve`, a port of the part of `Evaluate.run` after `compute` returns,
`AbstractParallelEvaluator.java:473-703`):

- Without keep_going a dependency in error is `None` and sets `values_missing()`; no exception
  reaches the function (`process_dep_value` / `unwrap_or_throw`, ports of
  `SkyFunctionEnvironment.java:613-660, 728-792`).
- The first exception thrown (without keep_going, or catastrophic with it) is committed and
  recorded as Java's `SchedulerException.ofError(errorInfo, key, rdepsToBubbleUpTo)`. A function
  that returns null on a child in error records `ofError(childErrorInfo, childKey, {key})`. The
  first record wins, as in the visitor.
- A node whose function returned null stays unfinished, with the deps it declared.

Error bubbling (`Run::bubble_error_up`, a port of `ParallelEvaluator.bubbleErrorUp`, 290-487),
after every evaluator thread has stopped:

- Walks from the failed node to a top-level node, one requester per level
  (`Iterables.getFirst(reverseDeps)`), and runs each function once more with
  `Environment::create_for_error` (`SkyFunctionEnvironment.createForError`): `in_error_bubbling()`
  is true, requests compute nothing (bubbleErrorInfo, else the done node, else missing), exceptions
  reach `get_value_or_throw`, `check_interrupted()` fails once a dep in error was requested (Java's
  interrupt bit), stored events are reported after the run.
- A function that throws gets `fromChildErrors(key, {fromException(e, false)})`; one that does not
  gets `fromChildErrors(key, {error})` with `error` the leaf failure or the last thrown error.
- Reaching a node already in the map returns null; `construct_result` (a port of
  `constructResult`, 506-584) then runs the cycle detector.

What the blocking model needed on top, each marked `// PORT:` where it is:

1. Reverse deps of unfinished nodes (`NodeEntry.getInProgressReverseDeps`). Java registers a
   parent when its function has returned null (`enqueueChild`); here the request registers it
   (`Node::in_progress_rdeps`), before the function has ended. To get Java's set back:
   a node that commits takes its registrations on undone deps back (`Run::committed`); only
   requesters whose function did return null count (`DoneEntry::awaits_deps`; one that threw and
   was ignored, or was interrupted, registered nothing in Java); a node that commits leaves out
   requesters whose request closed a cycle through it (`Node::mark_cycle_rdep`).
2. "No restart after the failure" (`Run::entry_seen_in_normal_evaluation`). In Java a node that
   got null for a dep waits to be restarted, and after `preventNewEvaluations` it never is. Here
   that node's function is blocked in the request and resumes. A dep that was not done when asked
   for and is done once the evaluation has failed is therefore shown as not done to that run.
   This is also what keeps a keep_going parent of the catastrophic node from seeing the error in
   normal evaluation and committing (S1-1, "the ancestors of a wrapped catastrophe are Committed").
3. Two states Java cannot reach, because here a request evaluates the dep before the function
   goes on. (a) A function throws although a dep is missing, and that dep, below it, already
   asked for it (a cycle): the error is committed and bubbles up, not into the cycle below
   (`no_keep_going_function_that_throws_above_its_own_cycle_reports_its_error`). (b) The only
   requester of an unfinished node threw after the node failed under it: bubbling goes through
   that requester anyway, so that the error reaches a top-level node
   (`no_keep_going_requester_that_throws_after_its_dep_failed_is_still_bubbled_through`). In both
   the reported exception is the one Java reports (the thrower's own).

Remaining difference of the model, now stated in `lib.rs` and `PORTING.md`: requests are evaluated
one at a time, in order. Java collects all requests of a run and evaluates none before the
function returns. Example: without keep_going, a function that requests a dep not yet built and
then a dep whose error is cached fails, in Java, on the cached error and the first dep never runs;
here the first dep runs first and may fail first.

## Findings

### S1-1 fixed

- How: see above. The bubbling path is the walk of `bubble_error_up`, not a property of the child
  error. A thrower on the path always gets `fromChildErrors(key, {fromException(e, false)})`
  whatever `is_catastrophic()` says; nothing above the catastrophic node is committed; the walk
  ends at a top-level node, so the result has an error and `maybe_ensure_catastrophe` succeeds.
  `catastrophe` is set as in `waitForCompletionAndConstructResult` (keep_going and a failure, with
  the Java check that the failure is catastrophic).
- `cleanup` runs on every exit of `evaluate` (drop guard `Cleanup`), also when it panics.
- Where: `evaluator.rs` `Evaluator::evaluate`, `Run::resolve`, `Run::finish`,
  `Run::bubble_error_up`, `Run::construct_result`.
- Tests: `engine_test::keep_going_catastrophic_rethrow_reports_the_bubbling_root` (scenario A),
  `engine_test::keep_going_catastrophe_wrapped_below_a_batch_reports_the_root` (scenario B; both
  also assert the graph holds only the catastrophic node),
  `engine_test::crash_leaves_no_uncommitted_node_in_the_graph`,
  `parallel_evaluator_test::incremental_cycle_with_catastrophe_and_failed_bubble_up` (Java), and
  the existing `catastrophic_build`, `top_catastrophe`, `catastrophe_bubbles_into_non_catastrophe`.

### S1-2 fixed

- How: no parent sees the exception in normal evaluation, so no catch block runs there, and a run
  that ends with a missing dep reports no stored event (as before). `bubble_error_up` re-runs one
  parent per level and flushes that run's stored events and posts (`EnvListener::flush` after each
  re-run, Java 440-445 and 480-485).
- Tests: `engine_test::no_keep_going_reports_events_and_posts_of_one_chain_only` (two in-flight
  parents that emit an error event and a post in the catch block and throw; 60 rounds at
  parallelism 2 and 8; exactly one event, one post, one error, all of the same parent),
  `engine_test::no_keep_going_waiter_that_throws_is_not_rerun_in_error_bubbling`,
  and the Java tests `memoizing_evaluator_test::error_only_bubbles_to_requesting_parents`,
  `error_only_emitted_once`, `warning_and_error_on_fail_fast_build`,
  `warning_before_error_on_fail_fast_build`,
  `warning_and_error_on_fail_fast_build_after_keep_going_build`.
- The old `no_keep_going_reports_one_bubbled_root_and_drops_aborted_roots` asserted that the first
  root in request order reports the error. Which parent registered first is a race in Java too;
  the test now accepts either parent.

### S1-3 fixed

- How: `Environment::in_error_bubbling()` returns `bubble_error_info.is_some()`
  (`SkyFunctionEnvironment.java:1036`), true exactly in the re-run. I did not take the review's
  shortcut (`!keep_going && dep_error.is_some()`): the function is really re-run, as the task
  asked, so Java code after a guard runs in the same state as in Java.
- `check_interrupted()` in that run returns `Err` once a dep in error was requested.
- PORTING.md "Skyframe" and the doc of `in_error_bubbling` are rewritten. The test
  `in_error_bubbling_is_always_false` is replaced by
  `in_error_bubbling_is_true_only_in_the_rerun_after_a_failure`.
- Tests: `parallel_evaluator_test::run_dep_on_error_halts_no_keep_going_build_eagerly` (Java, all
  four parameter combinations), `race_condition_with_no_keep_going_errors_future_error` (Java),
  `engine_test::check_interrupted_fails_in_error_bubbling_once_a_dep_in_error_was_requested`,
  `engine_test::get_temporary_direct_deps_in_error_bubbling_are_the_deps_of_the_normal_run`,
  `memoizing_evaluator_test::bubble_race` (Java).
- `blaze_sky_loading/src/target_pattern_function.rs:968-977` is a literal port and needed no
  edit: the guard now is true in the re-run.

### S2-1 fixed

- How: the literal port. `bubble_error_up` returns `None` when the next parent is already in the
  map; `construct_result` then adds every unfinished root to the cycle roots.
- Tests: `engine_test::no_keep_going_error_below_a_cycle_through_the_root_reports_the_cycle` (the
  review's scenario, parallelism 1 and 4: `{A: CycleInfo([], [A, B])}`, no exception), and the
  three Java tests `parallel_evaluator_test::cycle_and_error_in_bubble_up`,
  `cycle_and_error_and_other_in_bubble_up`, `cycle_and_error_and_error`.
- Note for the reader of those three: they need Java's `DeterministicProcessableGraph`, which
  hands out reverse deps in alphabetical order ("cycle" before "top"). The evaluator has a test
  hook for that order (`Evaluator::deterministic_for_testing`, `#[cfg(test)]`). In production
  order (first registered first, as `InMemoryGraphImpl`) the bubbling of those graphs goes to
  "top" and reports the error, in Java and here.

### S2-2 fixed

- How: the environment's map of requested deps is shared with the results it hands out
  (`RequestedDeps`), so `get` / `get_or_throw` read every dep requested by any call, earlier or
  later (`maybeGetValueFromErrorOrDeps`). A key never requested still gives `None` without marking
  values missing; the comment now says that this, and only this, is Java's bug-report case.
- Where: `environment.rs` `SkyframeLookupResult::get_or_throw_internal`.
- Test: `engine_test::lookup_result_reads_deps_of_earlier_and_later_requests`.

### S2-3 fixed

- How: all spawns go through `Run::spawn`, which returns the error. Depth guard: the claimed key is
  finished (not done), the crash is recorded, `evaluate` panics with "OutOfMemoryError: unable to
  create evaluator thread for <key>: <os error>" (what happens to Java there). Batch helpers that
  cannot start are done without (the requester drains the queue). Root workers: the ones that
  started take all roots; with none, crash. The thread for error bubbling: crash.
- Tests (through the `#[cfg(test)]` hook `spawns_before_failure_for_testing`):
  `engine_test::spawn_failure_of_the_depth_guard_crashes_instead_of_hanging`,
  `spawn_failure_wakes_waiters_of_the_claimed_key`,
  `spawn_failure_of_batch_helpers_falls_back_to_the_requesting_thread`,
  `spawn_failure_of_every_root_worker_crashes`.
- Not changed: the number of live threads still grows with dependency depth
  (`max_inline_depth`); the review named it as the reason spawns can fail, not as a finding.

### S2-4 fixed

- Non-throwing parent: `fromChildErrors(key, {error})` with the leaf failure or the last thrown
  error. Test: `engine_test::bubbled_error_of_a_parent_that_does_not_throw_wraps_the_leaf_failure`
  (cached keep_going cycle error reused without keep_going: path `[grandparent]`).
- Function that throws while a dep in error was requested: committed as the first error, with
  `isTransitivelyTransient` from the deps' errors. Test:
  `engine_test::no_keep_going_function_that_throws_with_a_dep_in_error_commits_its_own_error`.

### S3-1 fixed

Counts are of `@Test` methods at the pin (a parameterized method counts once).

| Java class | methods | ported | `#[ignore]` stub |
|---|---|---|---|
| ParallelEvaluatorTest | 91 | 64 | 27 |
| MemoizingEvaluatorTest | 155 | 60 | 95 |
| GroupedDepsTest | 15 | 15 | 0 |
| ErrorTransienceValueTest | 3 | 1 | 2 |

- Every method of the review's list is ported, except those in the stub list for the reason given
  there.
- The stubs are in `tests/not_ported.rs` (and two in `grouped_deps_test.rs`), under the Java name,
  with the Java file and line. Each body is `unimplemented!("not ported: Class.method")`, so
  running one with `--ignored` fails. Reasons, by count: incremental (invalidation of changed
  nodes, dirty nodes, versions, change pruning) 58; `SkyFunction.Reset` / rewinding 14;
  `MemoizingEvaluator.delete` 7; `EvaluationProgressReceiver` 6; stored events and replay
  (`EventFilter`) 6; partial reevaluation 5; `SkyKeyComputeState` 4; `dependOnFuture` 4; graph
  transformers 3; six further reasons with one or two tests each.
- How the 58 "incremental" ones were classified: by a script over the Java bodies (and the helpers
  they call) that looks for `tester.invalidate()`, `markAsModified` true, `differencer`, graph
  listeners and receivers. The five that had none of these were ported. I read about ten of the
  58 in full, not all.
- Ported with a stated difference (`// PORT:` at the test):
  - Where Java counts two runs of a function with keep_going (null, then the restart), the function
    runs once here. Where Java counts the normal run and the error-bubbling run, so does the port.
  - Six tests make a parent request its deps as one batch where Java requests them one after the
    other and relies on the first returning null at once: `bubble_race`, the three
    `cycle_and_error_*`, `incomplete_direct_deps_are_cleared_before_keep_going`,
    `limit_evaluator_threads`.
  - Graph listeners (`NotifyingHelper`) are replaced by latches in the functions in
    `error_bubbles_to_parents_of_top_level_value`,
    `race_condition_with_no_keep_going_errors_future_error` and `_inflight_error`,
    `error_only_bubbles_to_requesting_parents`.
  - `tester.invalidate()` after a changed leaf is `Evaluator::inject` of the new leaf value in 11
    tests. The second halves of `memoizing_cycle_above_independent_cycle` and `absent_parent` need
    a changed function and are left out.
  - `cycle_with_multiple_unfinished_children`, `cycle_and_error_in_bubble_up`,
    `cycle_and_error_and_error`: only the `isCycleNodePartialReevaluation=false` variants.
- The engine-specific gaps of the review are covered by `tests/engine_test.rs` (S1-1 A and B, two
  parents with events, a batch whose rest contains the requester's own key in
  `batch_with_the_requesters_own_key_is_a_self_cycle`, spawn failure).
- `tests/engine_test.rs` has no Java test behind it. Each test states the Java lines its
  expectation was derived from. Three of them cover states Java does not reach (above, point 3)
  and say so.

### S3-2 fixed (markers)

- `lib.rs` has a "Not ported" section listing `EvaluationProgressReceiver`, `getState` /
  `SkyKeyComputeState`, `registerDependencies`, `queryDep`, `dependOnFuture`, `Reset` and
  rewinding, partial reevaluation, the per-key keep_going predicate, `EventFilter`,
  `ShortCircuitingCycleDetector`, `delete`.
- Where there is a Rust type to hang it on, the method exists and fails with `unported!`:
  `Environment::register_dependencies`, `Environment::get_state`,
  `Environment::depend_on_future`, `SkyframeLookupResult::query_dep`. Test:
  `engine_test::register_dependencies_is_not_ported`.
- PORTING.md "Skyframe" lists them.

### S3-3 fixed

- `environment.rs` / PORTING.md "equivalent": gone with S1-3.
- `lib.rs` `get_value_or_throw::<K, E>`: says it matches the concrete type `E` only, Java also
  subclasses, and points at `get_value_or_throw_with`. Same note on the method and in PORTING.md.
- `environment.rs` lookup miss: with S2-2.
- Crash message: ported, not just described. A panic in a function makes `evaluate` panic with
  `prepareCrashMessage`'s text and the cause: "Unrecoverable error while evaluating node '<key>'
  (requested by nodes '<a>', '<b>')\nCaused by: <panic message>". A payload that is not a message
  is rethrown as it is. Tests: `parallel_evaluator_test::test_function_crash_trace`,
  `unrecoverable_error`, `memoizing_evaluator_test::crash_after_interrupt_crashes` (all Java).

### S4

- `request` rejects `ErrorTransienceValue.KEY` with Java's text. Test:
  `engine_test::requesting_the_error_transience_key_is_rejected`.
- Deferred: an interrupt flag set after all work finished still discards the result
  (`evaluate` returns `Err(Interrupted)`). Java returns the result and leaves the interrupt bit
  set. Changing it changes what callers of `evaluate` in crates I do not own see; it needs the
  integration lead.
- `child_error_infos` dedup by `Arc::ptr_eq`: not changed.

## Changes the review did not ask for

- `GroupedDeps` (found by porting `GroupedDepsTest`): equality ignores the order inside a group
  (`GroupedDeps.equals`), `remove` panics on an absent key ("Requested removal of absent
  element(s)"), new `num_groups` and `get_dep_group`.
- A function that stops with `Interrupted` because the evaluation is aborting never commits. It
  used to commit its children's error with keep_going.
- The panic payload of `evaluate` after a function crash is a `String` with the Java text in
  front (above). Code that matches the old bare message by substring still matches.
- `Evaluator` has two `#[cfg(test)]` fields (the hooks named above).

Public API: additive. New methods on `Environment`, `SkyframeLookupResult`, `GroupedDeps`.
`get_value_or_throw_with` returns the same type, spelled with the existing alias.

## Verification

All Cargo through `scripts/cargo-capped`.

- `test -p blaze_skyframe` in the working tree (HEAD `88dbfa6` plus uncommitted work):
  197 passed, 0 failed, 124 ignored (the stubs above).
- The test binary of that build run 400 times in a loop: 0 failures. Earlier loops found two
  flaky ports, both in the test's timing and both fixed there: Java's exception latch is released
  after the failed node is committed (the port waited for the abort flag, which is set before);
  and `error_only_bubbles_to_requesting_parents` relies on a dirty node not getting past its first
  changed dep group (the port now holds "error" until "slow" has started). A third flake was the
  engine: a requester that throws after the failure was treated as registered (fixed, point 1).
- `clippy -p blaze_skyframe --all-targets -- -D warnings`: clean. `rustfmt --edition 2024` on every
  changed `.rs` file.
- `scripts/w3/query-compare --project brotli`: query_label_kind 162/162, query_jsonproto 162/162.
  `--project re2`: 1091/1091 and 1091/1091, exit codes equal to the oracle's (0, 3, 0, 3). Binary:
  `target/debug/blaze-query-dev` built from the working tree after the last change to
  `blaze_skyframe`.
- `test -p blaze_sky_loading`: the test target of the working tree does not compile at the moment
  (another agent's test files call `LoadedPackage::assert_contains_event`, which does not exist
  yet; 73 errors, none in or about `blaze_skyframe`). Measured instead on a clean copy of HEAD
  `88dbfa6` (`git archive`) with this `crates/blaze_skyframe` copied over it: 124 passed, 0 failed,
  147 ignored. Those tests hardly reach the evaluator's error paths; query-compare is the stronger
  check for loading.
- `check --lib` of the 13 crates that depend on `blaze_skyframe`, in the working tree:
  `blaze_repo_rules` (`setup.rs`: `LocalDownloadManagerFetcher`, `RepoServices` fields) and
  `blaze_analysis` (`starlark_transition.rs`: `StarlarkOptionList`) do not compile, for reasons
  in other agents' files, and the crates above them were not reached. No error names a
  `blaze_skyframe` item.
- Consumers, on the same clean copy of HEAD `88dbfa6` with this `crates/blaze_skyframe`:
  `test -p blaze_build_config -p blaze_sky_analysis -p blaze_sky_exec -p blaze_sky_top
  -p blaze_query -p blaze_cquery --no-fail-fast`: 106 passed, 1 failed, 372 ignored. The failure is
  `blaze_sky_exec` `action_execution_function_test::
  file_write_then_symlink_builds_and_sets_output_permissions` ("not ported yet:
  ConfiguredTargetAndData.getLocation"); it fails the same way on that copy with HEAD's own
  `blaze_skyframe`, so it is not from this change. These are HEAD's versions of those crates, not
  the integration lead's uncommitted ones.

## blaze_sky_loading

No edit was needed in the call sites the review names:

- `target_pattern_function.rs:968-977`: literal port of the `inErrorBubbling()` guards; live now.
- `file_state.rs:258-264` (catastrophic `InconsistentFilesystemException`) and
  `package_lookup.rs:477-484` (rethrown as persistent, non-catastrophic): the shape of S1-1
  scenario B, which the engine now handles.

I read every `get_value_or_throw` / `get_or_throw` call of the crate (about 30 sites) for code that
would misbehave when, without keep_going, the call returns `None` instead of the exception in the
normal run. All handle `None` as Java does.

## For the integration lead (crates I do not own)

Behaviour that changes for every SkyFunction, without keep_going: a dep in error is `None` plus
`values_missing()` in the normal run; the catch blocks run in a second run of the same function
(`env.in_error_bubbling()`), and only for the functions on one path to a top-level node. A function
must return `Ok(None)` when a dep it catches errors for is `None`, and must tolerate running twice
for one key. I read the 40 or so call sites in `blaze_sky_exec`, `blaze_sky_top`,
`blaze_sky_analysis`, `blaze_build_config` and `blaze_query` by their match arms only; all have an
`Ok(None)` arm. I did not check what they do between requests.

Call sites that need a change, with the Java they should follow:

1. `blaze_sky_exec/src/artifact_function.rs:205-225` (`compute_artifact_nested_set`). It reads
   children with `result.get(k)` and its PORT comment relies on "in_error_bubbling() is always
   false". Port `ArtifactNestedSetFunction.java:94-131`: `getOrThrow` for `SourceArtifactException`,
   `ActionExecutionException`, `ArtifactNestedSetEvalException`; collect; throw
   `ArtifactNestedSetFunctionException` whose `isCatastrophic()` is the OR of the children's
   (`:140`). Without it a catastrophic action failure loses the bit one level up.
2. `blaze_sky_exec/src/action_execution_function.rs:63` and `:211`: `SkyFunctionException::new(e,
   Persistent)`. Java `ActionExecutionFunctionException.isCatastrophic()` is
   `actionException.isCatastrophe()` (`ActionExecutionFunction.java:1329`). Set `catastrophic`.
3. `blaze_sky_exec/src/completion_function.rs:561` and `:573`: same, Java
   `CompletionFunctionException.isCatastrophic()` (`CompletionFunction.java:485`).
4. `blaze_sky_exec/src/completion_function.rs` near 458 (the failed `TargetCompleteEvent`): it is
   now posted once, from the error-bubbling run. Java guards that run at
   `CompletionFunction.java:248` (`env.inErrorBubbling() && Thread.interrupted()`): staging and
   posting must not be stopped by the interrupt. Here: do not let `env.check_interrupted()?`
   return early between the failed input and the post when `env.in_error_bubbling()`.
5. Nothing in those crates calls `in_error_bubbling()` yet. The other Java users are
   `IncludeParser.java:311-371` and `SkyValueRetrieverUtils.java:52`.
6. Anything that inspects the panic payload of `Evaluator::evaluate`: it is now the Java crash
   text followed by "\nCaused by: " and the original message.
7. S4, if wanted: let `evaluate` return the result when the interrupt arrives after all work.

Tests of those crates were not run against this engine in the working tree (two of their
dependencies do not compile there); see "Verification" for the run on HEAD's versions.
