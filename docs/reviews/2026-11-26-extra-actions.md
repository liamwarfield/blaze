# Review: extra actions and metrics (first round), 2026-11-26

Commit under review: 374c631 (checkpoint 38). Reviewer role: extra-actions. Read-only; no Cargo,
no Bazel or Blaze process was started (the suite unit `root-suite-5f68157` was active for the
whole review, so the two S1 findings rest on code reading, not on a probe).

## Scope read (Rust against the pinned Java)

- `crates/blaze_analysis/src/extra/`: `extra_action_spec.rs`, `extra_action.rs`, `execution.rs`,
  `extra_action_info_file_write_action.rs`, `extra_action_utils.rs` (with `ExtraActionsVisitor`)
  against `analysis/extra/*.java`, `analysis/ExtraActionUtils.java`, `ExtraActionsVisitor.java`.
- `crates/blaze_rules/src/extra/action_listener.rs` against `rules/extra/ActionListener.java`.
- `getExtraActionInfo`: base (`AbstractAction.java:563`), `SpawnAction` (`:475`),
  `CppCompileAction` (`:1068`); `getInputFilesForExtraAction` and all its Java overrides.
- `crates/blaze_runtime/src/commands/print_action.rs` (whole file) against
  `PrintActionCommand.java`, `PrintActionVisitor`, `ActionGraphVisitor`,
  `SkyframeExecutor.getGeneratingAction`.
- Build wiring: `BuildDriverFunction.addExtraActionsIfRequested` and the filter
  (`build_driver_function.rs:108`, `execution_seam.rs:153`).
- `ctx.actions.do_nothing` against `StarlarkActionFactory.java:230`.
- Metrics: `crates/blaze_skyframe/src/evaluation_stats.rs` and `Evaluator::post_logging_stats`
  against `InMemoryMemoizingEvaluator.ProgressReceiver`; the inactivity watchdog and its monitor
  and reporter against `ActionExecutionInactivityWatchdog.java` and
  `ExecutionProgressReceiver.java:190-235`; `get_workspace_status_artifacts` against
  `SkyframeExecutor.java:1604` and its call in `BuildView.createResult`.

## Scope not read

- `extra_action_factory.rs`, `pseudo_action.rs`, `extra_action_map_provider.rs`,
  `extra_action_test.rs`; `JavaCompileAction.getExtraActionInfo` and the Python pseudo-action.
- `crates/blaze_protos` (the protos, the extension printing in `text_format/printer.rs`, the
  synthetic byte fixtures).
- `artifactMetrics` producers in `crates/blaze_sky_exec`, the cumulative metrics, the
  "VFS stat" profiler events, the BEP publication of extra-action outputs, the replay of the
  statistics into Java's hash order in `bep.rs`.

## Findings

### S1

E-1. Action templates are treated as actions by `print_action` and by the extra-action visitor.

- Rust: `crates/blaze_runtime/src/commands/print_action.rs:385` ("every Blaze action is an
  `Action`") and `:419`; `crates/blaze_analysis/src/extra/extra_action_utils.rs:157`
  ("every Blaze action is one").
- Java: `PrintActionCommand.java` `gatherActionsForTarget` and `gatherActionsForFile`
  (`if (action instanceof Action)`), `ExtraActionsVisitor.java:48`
  (`if (original instanceof Action action)`). `ActionTemplate` is an `ActionAnalysisMetadata`,
  not an `Action`.
- The comment is not true in Blaze: `CppCompileActionTemplate` implements the `Action` trait
  (`cpp_compile_action_template.rs:378`), sits in the configured target's action list, and
  answers `is_action_execution_metadata() == false` (`:443`); aquery already uses that guard
  (`action_graph_text_output_formatter_callback.rs:202`).
- Input: a `cc_library` whose `srcs` contain a tree artifact (a directory output of another
  rule), then `print_action //pkg:lib`. The visitor reaches the template through the archive's
  object-file tree input (same owner). Bazel prints the real actions only; Blaze also prints an
  `action { ... mnemonic: "CppCompileActionTemplate" }` entry built by the base
  `get_extra_action_info`. With `--experimental_action_listener` and a listener on mnemonic
  `CppCompileActionTemplate`, Blaze registers an `ExtraAction` that shadows the template; Bazel
  registers nothing.
- Fix: guard the three sites with `is_action_execution_metadata()`.

E-2. `getInputFilesForExtraAction` of Starlark actions is the base default (empty set).

- Rust: `crates/blaze_actions/src/actions/action.rs:603` is the only implementation besides
  `CppCompileAction`; `SpawnAction` (which carries `EnhancedStarlarkAction`) does not override
  it. Callers: `crates/blaze_analysis/src/extra/execution.rs:52` (`ExtraAction.discoverInputs`)
  and `crates/blaze_exec/src/analysis/actions/spawn_action.rs:513` (a Starlark action with
  `shadowed_action`).
- Java: `StarlarkAction.java:126` returns `getInputs()`; `EnhancedStarlarkAction` (`:454`)
  returns `allStarlarkActionInputs`, joined with the shadowed action's set when it has one.
- Input: a rule with `ctx.actions.run(..., unused_inputs_list = f)` (so `discoversInputs()` is
  true), an `action_listener` on its mnemonic with an `extra_action` that has
  `requires_action_output = 1`. The shadowed action runs first and prunes its inputs
  (`spawn_action.rs:247`, ported). `ExtraAction.discoverInputs` then builds
  `shadowed.getInputs() + inputFilesForExtraAction + extraActionInputs`: Bazel still gives the
  extra action every Starlark input, Blaze gives it the pruned set, so files named in the `.xa`
  file's `SpawnInfo.input_file` (which Blaze does fill from all inputs,
  `spawn_action.rs:407`) are missing from the extra action's inputs and sandbox. The same gap
  applies to a Starlark action whose `shadowed_action` is itself such an action.
- Fix: override in `SpawnAction` as the two Java classes do.

### S2

E-3. The `evaluatedValues` decision is not recorded, and the document says the opposite.
`docs/missing-features.md` at this commit says the Skyframe graph statistics of
`buildGraphMetrics` are absent; the code now sends them. See the ruling below; the deviation
has to be written down (missing-features and a decisions entry) because the referee does not
compare `buildMetrics` sub-messages, so nothing else shows it.

### S3 (honest gaps)

- E-4. `cpp_compile_action.rs:814`: `getInputFilesForExtraAction` with include scanning is
  `unported!` (Java `CppCompileAction.java:1935`).
- E-5. `print_action.rs` `SkyframeActionGraph::get_generating_action`: artifacts owned by an
  aspect or an action-template expansion are `unported!` (Java evaluates any
  `ActionLookupKey`); `AliasConfiguredTarget.toString` in the "not a supported target kind"
  message is `unported!`.
- E-6. `ExtraAction.getCommandLineLimits` (`UNLIMITED`) is not ported; the note in
  `execution.rs` argues the spawn is the same because the single command line has no parameter
  file info. Accepted as argued.
- E-7. The non-merged path `BuildView.addExtraActionsIfRequested` (with
  `filterTransitiveExtraActions` for `--experimental_extra_action_top_level_only`) is not
  ported; only the `BuildDriverFunction` path is.

### S4

- Java iterates a `HashMultimap` of specs per mnemonic (identity-hash order); Blaze uses
  insertion order. Not observable against a fixed expectation.

## Ruling asked for: `evaluatedValues`

Leaving the field at Blaze's own count is honest; it should not be made absent.

- The count is taken at the same point as in Java: `stateEnding(key, COMPUTE)`, once per
  `compute()` call (`evaluation_stats.rs` `state_ending`, `evaluator.rs:1789`;
  Java `InMemoryMemoizingEvaluator.java:131`). It is the same measurement, not a substitute.
- The proto defines the field as "Number of evaluations to build SkyValues. This includes
  restarted evaluations ... Subtract built_values from this number to get the number of
  restarted evaluations." Blaze's number satisfies that definition for Blaze: fewer restarts
  give a smaller number, and `evaluated - built` is Blaze's true restart count. Bazel's own
  value varies between runs with scheduling.
- Absent would be worse: a consumer computing restarts would get a negative number.
- Condition: it is a tool-specific measurement like timings. It must be listed as such
  (finding E-3) and any comparison tool must exclude it by name, not match it by accident.

## What was checked in depth and found equal to the Java

`ExtraActionSpec.addExtraAction` (outputs, dummy output, `.xa` file, command expansion, script
name uniquifier, `getActionId` and owner digest fingerprints, output path layout),
`ExtraAction` construction, input discovery arithmetic, dummy-output touch and failure codes,
`ExtraActionInfoFileWriteAction` inputs and key, `createExtraActionProvider`, `action_listener`,
the three `getExtraActionInfo` implementations read, `print_action`'s control flow, messages,
exit codes and visitor order, the action-file filter, the watchdog loop with its monitor and
reporter, the five statistics multisets, `getWorkspaceStatusArtifacts`, `do_nothing`.

## Verdict

STOP, bound to 374c631. Blocking: E-1 and E-2.
