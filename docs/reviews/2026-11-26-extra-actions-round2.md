# Review: extra actions and metrics, round 2 (2026-11-26)

Commit under review: 38cd662 (checkpoint 41). Reviewer role: extra-actions. Read-only, no
Cargo. No Bazel or Blaze process was started: `root-suite-5f68157` was active for the whole
review. Everything here is from reading the Rust at 38cd662 against the pinned Java; the oracle
evidence cited is the implementer's (docs/reviews/2026-11-26-extra-actions-response.md).

Verdict: **PASS**.

## Scope

Read: `git diff 374c631 38cd662` of `print_action.rs`, `crates/blaze_analysis/src/extra/`,
both `spawn_action.rs` files, `actions/action.rs`, `build_view.rs`, `evaluation_stats.rs`,
`progress.rs`; `CppCompileAction`'s two extra-action input methods; `name_uuid_from_bytes`
and its use in `do_nothing`; against `StarlarkAction.java:126` and `:454`,
`ExtraAction.java:158`, `SkyframeExecutor.java:2702`, `BuildView.java:386-435`,
`InMemoryMemoizingEvaluator.java:99-135`, `StarlarkActionFactory.java:230`.

Not read: the rest of the `crates/blaze_skyframe/src` diff (action rewinding: `reset.rs`,
the node entries, `dirty_building_state.rs`, `environment.rs`, most of `evaluator.rs`; another
review's area); the cumulative metrics for commands without a build event transport (their
code is in `crates/blaze_bep`, outside the paths of this re-check); the inactivity watchdog and
the workspace status evaluation before analysis, which have no change in these paths since
374c631, where round 1 found them equal to the Java.

## Round-1 findings

| finding | state at 38cd662 |
|---|---|
| E-1 action templates | fixed |
| E-2 inputs of Starlark actions | fixed |
| E-3 `evaluatedValues` | recorded (missing-features, decisions entry) |

- E-1: the three sites are guarded with `is_action_execution_metadata()`
  (`print_action.rs:391` and `:428`, `extra_action_utils.rs:161`), as Java's
  `instanceof Action`. Oracle: `print_action //tt:lib` byte-identical, no extra action for the
  template.
- E-2: `SpawnAction::get_input_files_for_extra_action`
  (`crates/blaze_actions/src/analysis/actions/spawn_action.rs:673`) answers per class as the
  three Java classes do: empty for `SpawnAction`, `getInputs()` for `StarlarkAction`,
  `allStarlarkActionInputs` for `EnhancedStarlarkAction`, joined after the shadowed action's
  set, `None` when that is `None`; the join order is Java's. `ExtraAction.discoverInputs` now
  asks the context-taking form (`execution.rs:53`). Oracle: the `ua` case builds and its
  outputs are byte-identical.

## Other changes read

- `owner_actions` (`print_action.rs:541`): configured targets, aspects, top-level aspects,
  template expansions and the workspace status are read from the graph; a missing or failed
  owner gives `None`, as `result.hasError()`; any other key kind is `unported!`. Java evaluates
  the key, so it would compute a value that is missing; after the command's own analysis and
  build the values are done. Faithful by reading; no oracle case covers the aspect, top-level
  aspect and expansion branches (open item).
- `build_view.rs:433`: `set_artifact_roots` only without the execution phase, as the `else`
  branch of `BuildView.update`.
- Statistics receiver: `dirtied` (DIRTY, CHANGE, REWIND ignored), `evaluated` (built or
  cleaned, nothing without direct deps) and `stateEnding(COMPUTE)` are Java's. The new
  `remove_function` is Blaze-only and used only for the Blaze-only CQUERY_TRANSITION_RESOLVER
  (ruled, decisions.md).
- `do_nothing`: version 3 UUID over the MD5 of `"empty action <label>"`, version and variant
  bits as `UUID.nameUUIDFromBytes`.

## Findings

### S1, S2

None.

### S3 (honest gaps, unchanged or new)

- A `StarlarkAction` whose shadowed action is a `CppCompileAction` with include scanning calls
  the form without a context, which is `unported!` (`cpp_compile_action.rs:820`); the same in
  `EnhancedStarlarkAction.discoverInputs` (`blaze_exec .../spawn_action.rs:514`). Java passes
  the context through (`StarlarkAction.java:461`).
- `findMinimalSetOfRequiredModules` after `find_used_modules` (header modules) is not ported.
- E-5 part 2 (`AliasConfiguredTarget.toString`) and E-7 (the non-merged
  `addExtraActionsIfRequested`) as in round 1.

### S4

None.

## Open, not blocking

- An oracle case for `print_action` on an artifact owned by an aspect and by a template
  expansion.
- The cumulative metrics without a build event transport were not reviewed here.

## Verdict

**PASS**, bound to 38cd662.
