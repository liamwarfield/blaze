# Review round 3: commands (R1, R2, R4, S4 re-check), 2026-11-26

Reviewer (Fable 5.2), read-only: no code, test, script, expectation or baseline edited, no
Cargo, no subagents. Commit under review: **e476a59**. Round 2:
`2026-11-25-commands-round2.md` (STOP at f71e692 on R1). Response: "Round 2" of
`2026-11-25-commands-response.md`.

## Verdict

**PASS**, bound to e476a59. No blocking finding. R1, R2 and R4 are fixed as the Java reads; R3
stays open by root's ruling (decisions, 2026-11-26, declared deviation 4).

## Scope

- Read: `git diff f71e692 e476a59` of `blaze_sky_top/src/{build_tool,analysis_phase_runner,
  build_view}.rs`, `blaze_runtime/src/project_options.rs`, and the hunks of
  `commands/{build,bep}.rs` the response names. Java: `SkyframeBuildView.java:618-730, 911-957`,
  `AnalysisOperationWatcher.java:70-93`, `BuildDriverFunction.java:395-408`,
  `AnalysisPhaseRunner.java:427-437`, `BuildTool.java:340-400, 500-560, 594-613`,
  `BlazeCommandDispatcher.java:779-800`.
- Not read: the other hunks of the shared files (worker options, exclusive-test order, extra
  actions, aquery dump after build, artifact metrics: other agents' work); `config.rs` beyond
  its outline; the `vendor` fix ("Found while working on Part B") beyond its diff.
- Probes (mine): workspace `~/.cache/blaze/work/review-commands-2/ws`, oracle against
  `~/.cache/blaze/work/extra-actions/bin/blaze-a2`, `--batch`, `--jobs=4`, one output user root
  per side. There is no binary of e476a59, so the Blaze side is secondary evidence. Outputs:
  `~/.cache/blaze/work/review-commands-2/out/` (left in place, 260 MB).

## R1 (S1, was blocking): fixed

- `analysis_phase_runner.rs:206, 244`: the event is posted when the request is not merged, or
  when `AnalysisResult::has_top_level_keys()`. That is Java's rule:
  `AnalysisPhaseRunner.java:429` posts always; with merged analysis and execution only
  `analysisFinishedCallback` posts it, and `AnalysisOperationWatcher.java:72` returns at once
  for an empty key set.
- `build_view.rs:568`: the count is configured-target keys plus top-level aspect keys, Java's
  `topLevelKeys` (`SkyframeBuildView.java:695`). Only zero against non-zero matters, so the
  missing de-duplication of `Sets.union` has no effect.
- `bep.rs:1704`: the collector's analysis-phase data (`onAnalysisPhaseComplete`, graph stats)
  is recorded under the same condition; `logAnalysisStartingEvent` stays unconditional, as
  `BuildView.update`.
- Probes, stderr after filtering progress lines and times, and exit codes:

| Command | Oracle | Blaze (`blaze-a2`) |
| --- | --- | --- |
| `build` (no targets) | no `Analyzed` line, exit 0 | identical |
| `build -- //pkg:all -//pkg:all` | no `Analyzed` line, exit 0 | identical |
| `build --nobuild -- //pkg:all -//pkg:all` | `INFO: Analyzed 0 targets (5 packages loaded, 6 targets configured).` | identical |

  In the build event stream of the second case neither side has `targetMetrics`,
  `packageMetrics` or `analysisPhaseTimeInMs`; in the third both have them. The event id
  sequences are equal in both cases.
- Not probed by me: `test`, `run`, `fetch`, `cquery`, `print_action` (the response's table).

## R2 (S1): fixed

- `build_tool.rs:886-891` calls `process_without_analysis` when the request does not analyze;
  `build.rs:1172` runs the same construction as `process` (`executorInit`, the construction
  checks). Java creates the tool outside `if (performAnalysisPhase)` (`BuildTool.java:520`).
- A merged request with `--noanalyze` now constructs it here too; the fallback at `build.rs:908`
  is guarded by `!collector.executor_initialized()`, so it is not initialised twice.
- Probe `build --noanalyze --nobuild //pkg:all`: both sides print
  `INFO: Elapsed time: Ns, Critical Path: Ns`, `INFO: 0 processes.`, exit 0.

## R4 (S2): fixed

- `project_options.rs:166-171` posts after the native and the Starlark flags are parsed and
  before `createBuildOptions`, the place of `BuildTool.java:375-390`.
  `bep.rs:1494` builds the event with the arguments of the Java call, `replaceable = false`.
- `project_evaluated(true)` no longer releases the tentative event (`bep.rs:2227`), as the
  `else` branch at `BuildTool.java:391-397`.
- `UpdateOptionsEvent`: the whole pinned tree names it in two files, its declaration and
  `BuildTool.java:391`. No subscriber. The claim holds.
- `cquery`/`aquery` with project flags have no applier and stop at `unported!`
  (`build_tool.rs:789-793`), so the hook cannot swallow the tentative event silently. S3.
- Not probed by me (needs a project file); the response's stream comparison stands as its own.

## S4: done

`config.rs` is 653 lines and takes the model and the text/JSON writers from
`blaze_build_config::config_output`; only the diff model, the selection logic and the writers
that module lacks remain.

## New findings

None in this area. One difference seen in the R1 probes belongs to `reporting` and is filed
there as N1 (`2026-11-26-reporting-round3.md`): with `--nobuild`, Blaze's `buildMetrics` lacks
the workspace status action.

## Open items, not blocking

- R3 (S2): open by ruling until the graph is kept.
- The fixer's evidence and mine come from binaries of uncommitted trees. Repeat the R1 table
  once with a binary of a checkpoint commit.
- `analysis_phase_runner.rs:200-203`: the event is posted after the keep-going analysis errors
  are reported, Java before. Stated in a PORT note; not re-examined.
