# Response to the rewinding review, first round

Date 2026-11-26. Answers `2026-11-26-rewinding.md` (STOP at dad60f0: RW-1, RW-2 blocking; RW-3,
RW-4 S2; S3; S4; RW-5 open). Written by the implementer of task `rewinding`.

Tree: HEAD `98d98c95` plus uncommitted changes (listed per finding). Nothing is committed.

Verification on that tree:

- `scripts/cargo-capped check -p blaze_sky_exec -p blaze_actions -p blaze_skyframe -p blaze
  --tests`: clean.
- `scripts/cargo-capped test -p blaze_sky_exec --lib`: 85 passed, 0 failed, 7 ignored (includes
  the new RW-1 test).
- `scripts/cargo-capped test -p blaze --test rewind_lost_inputs_test -- --ignored` (S3, against
  Bazel's remote worker): 1 passed.
- `rustfmt --edition 2024` on every changed file; `scripts/java-ref-lines check` on the new
  markers: clean.

## Summary

| finding | state | where | test |
|---|---|---|---|
| RW-1 (S1) | fixed | `completion_function.rs`: `completion_function_exception_for_top_level_output` | `top_level_output_exception_is_transient` |
| RW-2 (S1) | fixed | `skyframe_action_executor.rs` (`execute_action`, `ExecutionFailure::RestartNode`), `action_execution_function.rs` | end-to-end only (see below) |
| RW-3 (S2) | fixed | `Action::may_insensitively_propagate_inputs` (default false), overridden by the four actions | existing rewind-strategy tests |
| RW-4 (S2) | fixed | `RewindSignal::wait`, `wait_for_shared_result` | none (needs a panicking owner) |
| S3 | one end-to-end test added | `crates/blaze/tests/rewind_lost_inputs_test.rs` | 1 passed |
| S4 | comment fixed; successor order documented | `evaluator.rs`, `reset.rs` | — |
| RW-5 | nothing to change in code | — | — |

## RW-1

`CompletionFunctionException(TopLevelOutputException)` is now `Transience::Transient`
(`CompletionFunction.java:479`), in a function of its own so that it is testable. The two other
constructors stay persistent. Test: `completion_function::top_level_output_exception_tests::
top_level_output_exception_is_transient` (transient, not catastrophic).

## RW-2

A shared action's copy that waits for the primary action no longer loops inside
`execute_action`. When the owner's result is lost inputs and the owner's state becomes
`Obsolete`, the copy returns `ExecutionFailure::RestartNode`; `ActionExecutionFunction` turns it
into `Reset::self_only(ActionExecutionKey(<copy's key>))`. The evaluator then runs the copy's
`compute` from the start: it requests its inputs again, finds the rewound ones undone, waits for
them, and only then reaches `execute_action` (as owner or as copy of a new state). That is Java's
`scheduleRestart` / `UndonePreviouslyRequestedDeps` sequence; no execution with the stale input
metadata, no second rewind. The `PORT` note on `RewindSignal` says this now. The loop in
`execute_action` is gone.

Not covered by a unit test: the in-process harness for two nodes of one shared action is the one
the sweep agent is building for `RewindingTest` (its shared-action cases will cover this when
they are ported). The end-to-end test below has no shared action.

## RW-3

`may_insensitively_propagate_inputs` is a method of the `blaze_actions` `Action` trait with
Java's default `false` (`ActionExecutionMetadata.java:115`); `SymlinkAction`,
`SymlinkTreeAction`, `RunfilesTreeAction` and `SolibSymlinkAction` override it in their
`impl Action`. The rewind strategy calls the method; the downcasts are removed.

## RW-4

Both waits of a copy are interruptible: `RewindSignal::wait` and the wait on the result cell
(`wait_for_shared_result`) register a `ThreadInterrupt` listener and end with
`ExecutionFailure::Interrupted`. (`blaze_sky_exec` now depends on `blaze_shell` for it.)

## S3

`crates/blaze/tests/rewind_lost_inputs_test.rs::lost_remote_input_is_rewound` drives the built
`blaze` binary against Bazel's remote worker (`scripts/remote/worker`): `//a:foo` runs remotely,
`//a:bar` (`no-remote-exec`) consumes it locally under `--remote_download_minimal`; the worker is
restarted empty, `bar.in` changes, and the build with `--rewind_lost_inputs` must succeed with
"Build completed successfully, 2 total actions" and an execution log in which `//a:foo` ran again
on `remote` and `//a:bar` on `linux-sandbox`. The expected values were recorded from the oracle
with the same commands (the oracle's log shows the same two spawns). Ignored by default (needs the
worker jar); run with `--ignored`. `RewindingTest`'s 37 cases stay ignored until the in-process
multi-command fixture lands; I will port them onto it then.

## S4

`evaluator.rs`: the stale "resets are not ported" comment now says that `old_deps` is the union of
the old deps and the reset deps (taken in `evaluate_node`). The successor order of `RewindGraph`
(insertion order where Guava's is unspecified) is already documented as a `PORT` note in
`reset.rs`; it only orders the dirtying of rewound nodes, which nothing reports.

## RW-5

Agreed; root has withdrawn the "runs alone" ruling.
