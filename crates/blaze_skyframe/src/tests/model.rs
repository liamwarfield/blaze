//! Reference model for the randomized incremental-evaluation test.
//!
//! The finished test says: for random graphs, random edits and random keep_going, an incremental
//! evaluation equals a from-scratch evaluation of the same graph in values, errors and reported
//! events, and the set of functions that ran is the set Java's algorithm runs. This module is
//! everything that does not need the engine:
//!
//! * [`spec`]: the graph spec (leaves with injected integers, function nodes with programs), the
//!   [`Edit`]s, the [`EvalRequest`]s, and [`run_program`], the interpreter the driver's
//!   `SkyFunction` runs;
//! * [`generate`]: a seeded generator of [`Scenario`]s (its own PRNG, no new dependency);
//! * [`scratch`]: [`eval_from_scratch`], what a fresh evaluator must answer (with the cases where
//!   the answer is a set);
//! * [`graph`]: [`ModelGraph`], Java's incremental algorithm simulated node entry by node entry
//!   (versions, dirtying, deleting, change pruning, the dirty-node check, the cycle detector), with
//!   [`ModelRun`] (`ran`, `marked_clean`, `change_pruned`, results, events);
//! * self-tests: the model against [`eval_from_scratch`] for many seeds, minimality, no-op edits,
//!   and a table of hand-derived scenarios.
//!
//! Everything is derived from the Java at the pin (`J:` = `src/main/java/com/google/devtools/
//! build/skyframe/`), cited at each rule, never from what Blaze's evaluator does.
//!
//! # Driver contract
//!
//! Written against `docs/skyframe-incremental.md` section 3. The driver ([`driver`], notes in
//! `docs/skyframe-incremental.d/model-test.md`) does,
//! for each `Scenario` from [`generate::generate`] (or `generate_with`; the number of seeds comes
//! from the environment variables `BLAZE_SKYFRAME_MODEL_SEEDS` and `BLAZE_SKYFRAME_MODEL_SEED_START`):
//!
//! **Keys and functions.** Two key types, both `Debug` as `n{id}`, `Value = i64`,
//! `HERMETICITY = FunctionHermeticity::Nonhermetic` (Java's `GraphTester.NonHermeticKey`;
//! CHANGE-invalidating a hermetic key throws "Nodes with hermetic functions cannot be marked
//! 'changed'", J:InvalidatingNodeVisitor.java:467): `ModelKey(NodeId)` with
//! `values_equal(a, b) = a == b`, used for nodes whose `NodeSpec::comparable` is true, and
//! `OpaqueModelKey(NodeId)` with the default `values_equal` (never equal) for the others. Both
//! are served by one function that reads the current spec (shared with the driver, e.g.
//! `Arc<Mutex<GraphSpec>>`, never locked across a request):
//!
//! * leaf `n`: returns its current spec value (it runs only when the node is absent);
//! * function `n`: `run_program(program, &mut adapter)` where the adapter's `request(group)` is
//!   `env.get_value(k)` (dep without `catch`) or `env.get_value_or_throw::<_, SomeErrorException>(k)`
//!   (with `catch`; `Err` is [`DepRead::Exception`]) for a group of one, and one
//!   `env.get_values_and_exceptions(&keys)` read with `get` / `get_or_throw` in order for a group
//!   of several; `Ok(None)` is [`DepRead::Missing`]. `values_missing()` is
//!   `env.values_missing()`. On `completed: Some(pre)` the function reports, in this order,
//!   `Event::warn(warning_text(n, pre))` if `program.warning` and `Event::error(stored_event_text(
//!   n, pre))` if `program.stored_event`. Then `RunResult::Value(v)` returns `Ok(Some(v))`,
//!   `Throw { transient }` returns `Err(SkyFunctionException::new(SomeErrorException(
//!   exception_message(n)), transience))`, `Missing` returns `Ok(None)`.
//! * Every invocation (normal or error bubbling) adds `n` to this evaluation's `ran` set and
//!   increments a counter; with `interrupt_after_runs = Some(k)` the invocation that finds the
//!   counter at `k` calls `Evaluator::interrupt` before doing anything else.
//!
//! **Evaluator.** Built once per scenario with the function above, `keep_edges(true)`,
//! `event_filter(EventFilter::NO_STORAGE)` (the default) and a progress receiver recording, per
//! evaluation: `dirtied` keys, `deleted` keys, `change_pruned` keys (Java's callback, which is the
//! model's `marked_clean`), and keys whose `evaluated` has `direct_deps: Some(_)` and state
//! `SuccessVersionUnchanged` (the model's `change_pruned`; note `FailVersionUnchanged
//! .version_changed()` is true in Java, so compare states, not that flag).
//!
//! **Per step** `(edits, request)`, with `model: ModelGraph` and `model_spec` cloned from the
//! scenario's spec:
//!
//! 1. For each edit, in order, call `model.apply_edit(&mut model_spec, &edit)` and make the
//!    evaluator call: `SetLeaf { node, value }` → `inject(key(node), value)`;
//!    `SetProgram { node, program }` → put `program` in the shared spec, then
//!    `invalidate([key(node)])`; `Invalidate { node }` → `invalidate([key(node)])`;
//!    `Delete { nodes }` → `delete(|k, _| k is a model key whose id is in nodes)`;
//!    `DeleteDirty { age }` → `delete_dirty(age)`; `InvalidateTransientErrors` →
//!    `invalidate_transient_errors()`. The first step's edits inject every leaf.
//! 2. `emitted_event_state().clear()`; reset the per-evaluation records.
//! 3. `evaluate(roots, EvaluationContext { keep_going, parallelism, event_handler: <collector>,
//!    ..Default })`. If it returned `Err(Interrupted)`: `run = model.evaluate_interrupted(
//!    &model_spec, &request)`; else `run = model.evaluate(&model_spec, &request)`. Then
//!    `clear_interrupt()`.
//! 4. Compare (every step):
//!    * Unless interrupted: the actual result per distinct root (`Value(v)` from `get`;
//!      `Error(Cycle)` when the error has cycle infos; else `Error(Exception { origin, transient
//!      })` with `origin` parsed from the `SomeErrorException` message and `transient =
//!      is_transitively_transient()`; else `Absent`) passes
//!      `eval_from_scratch(&model_spec, &roots, keep_going).check(&actual, result.has_error())`.
//!    * If `run.exact`: actual results == `run.results`; `ran` == `run.ran`; the receiver's
//!      `change_pruned` keys == `run.marked_clean`; the unchanged-version `evaluated` keys ==
//!      `run.change_pruned`; `dirtied` == `run.dirtied` and `deleted` == `run.deleted` (with
//!      `MKey::Et` as `ERROR_TRANSIENCE_KEY`); the reported warnings == `run.warnings` and the
//!      reported error-kind events == `run.stored_events`.
//!    * Else: `run.ran_bounds()` = `(lower, upper)` with `lower ⊆ ran ⊆ upper`.
//!    * Optional second mode with `EventFilter::FULL_STORAGE`: compare values/errors as above
//!      and, for keep_going exact runs whose `eval_from_scratch` closure has no cycle, the set of
//!      reported error-kind events with `Expected::stored_events`; `ran` only against
//!      `ran_upper` (stored events make change pruning stricter: `ValueWithEvents.equals` needs
//!      `NestedSet.shallowEquals`, which the model does not simulate).
//!
//! [`Edit`]: spec::Edit
//! [`EvalRequest`]: spec::EvalRequest
//! [`run_program`]: spec::run_program
//! [`Scenario`]: spec::Scenario
//! [`DepRead::Exception`]: spec::DepRead::Exception
//! [`DepRead::Missing`]: spec::DepRead::Missing
//! [`eval_from_scratch`]: scratch::eval_from_scratch
//! [`ModelGraph`]: graph::ModelGraph
//! [`ModelRun`]: graph::ModelRun

mod driver;
pub(crate) mod error_model;
pub(crate) mod generate;
pub(crate) mod graph;
pub(crate) mod scratch;
pub(crate) mod spec;

mod self_tests;
mod table;
