# Response: extra actions and metrics (first round), 2026-11-26

Response to `docs/reviews/2026-11-26-extra-actions.md` (STOP at 374c631, blocking E-1 and
E-2), by the implementer (task `extra-actions-3`). The changes are in the working tree on top
of 374c631 (not committed; root commits). The oracle runs are under
`~/.cache/blaze/work/extra-actions/runs/` (workspace `ws-xa`, one reused output base per side,
`--max_idle_secs=60`, `--jobs=4`, servers shut down after each series).

## E-1 (S1): action templates treated as actions — fixed

- `crates/blaze_runtime/src/commands/print_action.rs`: both `add_extra_action_info` sites
  (`gatherActionsForTarget` and `gatherActionsForFile`) are guarded with
  `action.is_action_execution_metadata()` (Java: `if (action instanceof Action)`). The wrong
  comment ("every Blaze action is an `Action`") is gone.
- `crates/blaze_analysis/src/extra/extra_action_utils.rs` `maybe_add_extra_action`: returns
  early for an action that is not `is_action_execution_metadata()` (Java
  `ExtraActionsVisitor.java:48`, `if (original instanceof Action action)`).

Oracle evidence (new package `ws-xa/tt`: a rule that declares a directory of two `.cc` files,
a `cc_library` with that tree artifact in `srcs`, an `action_listener` on
`CppCompileActionTemplate` and `CppCompile`):

| case | oracle | Blaze |
| --- | --- | --- |
| `print_action //tt:lib` | `CppArchive`, `CppLink`; exit 0 | byte-identical stdout (77 lines); exit 0 |
| `build --experimental_action_listener=//tt:al //tt:lib` | no extra-action output; 1 process, 5 cache hits | same (file lists identical, empty) |

Runs: `pa-tt-{oracle,blaze}.out`, `xa-tt-{oracle,blaze}.err`, `xa-tt-files-*.txt`.

## E-2 (S1): `getInputFilesForExtraAction` of Starlark actions — fixed

`crates/blaze_actions/src/analysis/actions/spawn_action.rs`: `SpawnAction` overrides
`get_input_files_for_extra_action` by class, as the Java classes do:

- plain `SpawnAction`: the empty set (`SpawnAction` does not override the base);
- `StarlarkAction`: `getInputs()` (`StarlarkAction.java:126`);
- `EnhancedStarlarkAction`: `allStarlarkActionInputs`, joined (`createInputs`) with the
  shadowed action's set when there is a shadowed action, `None` when the shadowed action's is
  `None` (`StarlarkAction.java:454`).

Oracle evidence (new package `ws-xa/ua`: `ctx.actions.run(..., unused_inputs_list = ...)`
that prunes `ua/pruned.txt`, and an `extra_action` with `requires_action_output = True` whose
command reads `ua/pruned.txt`):

| binary | result |
| --- | --- |
| oracle | build succeeds; `.out` (`pruned`) and `.xa` produced |
| Blaze before the fix (`blaze-b5`, 374c631 era) | `cat: ua/pruned.txt: No such file or directory`, exit 1 |
| Blaze after the fix | build succeeds; `.out` and `.xa` byte-identical to the oracle's |

Runs: `xa-ua-{oracle,blaze,oldblaze}.err`, `xa-ua-files-*.txt`.

## E-3 (S2): `evaluatedValues` — recorded

- `docs/missing-features.md`: the `buildMetrics` paragraph now says only `memoryMetrics` is
  absent; `artifactMetrics`, `cumulativeMetrics` and the Skyframe graph statistics are sent,
  and `evaluatedValues` is Blaze's own count (one `compute()` per node, no restarts), a
  tool-specific measurement that comparison tools exclude by name.
- The ruling itself is in `docs/decisions.md` (root's entry of 2026-11-26, "Declared
  deviation: `buildMetrics.evaluatedValues` is Blaze's own count").

## S3 items

- E-5, part 1 (fixed): `SkyframeActionGraph::get_generating_action` in `print_action.rs` now
  resolves the generating action of any action lookup key Blaze has: configured targets,
  aspects (`AspectKey` → `AspectValue`), top-level aspects (`TopLevelAspectsKey` →
  `TopLevelAspectsValue`), action-template expansions (`ActionTemplateExpansionKey` →
  `ActionTemplateExpansionValue`, through the `ActionLookupValue` trait) and the workspace
  status. Another key kind fails loudly (`unported!`).
- E-5, part 2 (still `unported!`): `AliasConfiguredTarget.toString` in the "not a supported
  target kind" message. It needs the configured-target `toString` of an alias, which nothing
  else in Blaze prints yet.
- E-7 (still not ported): the non-merged `BuildView.addExtraActionsIfRequested` with
  `filterTransitiveExtraActions` for `--experimental_extra_action_top_level_only`. Bazel 9
  runs merged analysis and execution by default (Skymeld), where the `BuildDriverFunction`
  path is ported; the non-merged path is reached only with `--noexperimental_merged_skyframe_analysis_execution`.
  It stays listed as missing.
- E-4 and E-6: unchanged (E-6 accepted as argued).

## S4: per-mnemonic spec order

Java keeps the specs of one mnemonic in a `HashMultimap` (`computeMnemonicsToExtraActionMap`):
a hash set per key whose iteration order is that of `ExtraActionSpec.hashCode()`, which is the
identity hash (`Object.hashCode`). The identity hash is assigned per object per JVM run, so
Bazel's order of two listeners on one mnemonic varies between runs and cannot be reproduced.
Blaze keeps the first-insertion order and drops a spec already present (identity), as the set
does; the `PORT: java-hash-order` note at `MnemonicToExtraActionMap` says so. Nothing else
changed.

## Other work in this round (not findings)

Build Event Service client (`--bes_backend`), new crate `crates/blaze_bes`, and the profiler
sites of `docs/profiler-sites.md`; see the implementer's report. The BES request sequence was
compared with the oracle's through a logging BES server (`blaze-bes-logging-server`): equal
for a successful upload, for a stream the server fails once (retry), and for an unreachable
backend (console and exit code 38 identical).
