# Review round 2: coverage (V1 re-check), 2026-11-26

Reviewer (Fable 5.2), read-only. Commit under review: **e476a59**. Round 1:
`2026-11-26-coverage.md` (STOP at 5f68157 on V1).

## Verdict

**PASS**, bound to e476a59. No blocking finding. V1 is fixed; V2 is a declared deviation
(`docs/decisions.md`, 2026-11-26, item 1); V3 is open and queued (same file).

## Scope

- Read: `crates/blaze_runtime/src/commands/build.rs:2255-2314` (the coverage block of
  `RuntimeExecutionPhase::execute_build`), `blaze_sky_top/src/build_view.rs:1154-1198`
  (`construct_coverage_artifacts`), `bazel_coverage_report_module.rs:37-70`,
  `coverage_report_action_builder.rs:252-308, 380`, `execution_tool.rs:881-898`
  (`evaluate_artifacts`). Java: `BazelCoverageReportModule.java:44-110`,
  `CoverageReportActionBuilder.java:203-300`, `TestActionBuilder.java:494-508`,
  `SkyframeBuildView.java:796-820`.
- Not read again: everything round 1 read and did not fault.
- Probes (mine): a copy of the `sh_test` workspace of the coverage task in
  `~/.cache/blaze/work/review-coverage-2/ws`, oracle against
  `~/.cache/blaze/work/extra-actions/bin/blaze-a2` (not a binary of e476a59: secondary
  evidence), `--batch`, `--jobs=4`. Outputs in `~/.cache/blaze/work/review-coverage-2/out/`
  (left in place, 262 MB).

## V1 (S1, was blocking): fixed

The `unported!` now sits after `construct_coverage_artifacts` and fires only when the request
is not merged and the factory returned a wrapper or artifacts (`build.rs:2273-2286`).

Paths through the block for a request without merged analysis and execution:

| Case | Factory result | Blaze | Java |
| --- | --- | --- | --- |
| no test targets (`build`) | `None`: `targetsToTest` empty (`CoverageReportActionBuilder.java:217`) | nothing | nothing |
| tests, no `--collect_code_coverage` | `None`: no `CoverageParams`, so the generator is null (`TestActionBuilder.java:494-508`, `:241`) | nothing | nothing |
| `--combined_report=none` | `None` | nothing | nothing |
| coverage collected, artifacts exist | wrapper | `unported!` | builds the report with the targets |
| coverage collected, no coverage artifacts | `None`, after the error `Cannot generate coverage report ...` | the error, at the end of execution (C1) | the error, during analysis |
| evaluation error without `--keep_going`, or interrupt | block skipped | nothing (C1) | artifacts were made during analysis |

- The two conditions of the guard cannot disagree: artifacts come only from a wrapper
  (`build_view.rs:1166-1197`). The `||` is redundant and harmless.
- In the null-generator case the Rust short-circuits before the `expect` on the action owner
  (`coverage_report_action_builder.rs:294-304`), as Java's `reportGenerator == null ||`.
- What a build without coverage still does in this block that the non-merged Java path does
  not: the factory call (reads the test providers, no event, no Skyframe write) and one
  `evaluate_artifacts` with no roots. Neither is visible: the collector registration needs a
  wrapper, the `RemoteModule.coverageArtifactsKnown` stop needs artifacts, and the evaluation
  has nothing to build.
- Probes, with `--noexperimental_merged_skyframe_analysis_execution` and no coverage:

| Command | Oracle | Blaze (`blaze-a2`) |
| --- | --- | --- |
| `test //:compatible_test` | exit 0, 1 test passes | same lines after filtering progress and times |
| `test --keep_going //:all` (one incompatible test) | exit 0, 1 passes, 1 skipped | same lines |

  Root's probe of `build` on this path is in `docs/decisions.md` (2026-11-26, checkpoint 36).

## New findings

### S2

**C1. Without merged analysis and execution, two coverage paths pass the guard silently.**

- `build.rs:2262-2286`. Java makes the coverage artifacts during analysis
  (`BuildView.java:406, 656-659`); Blaze calls the factory after the execution, and only when
  the build had no error or `--keep_going` is set.
- Inputs, all with `--collect_code_coverage` and the non-merged path:
  - no coverage artifact collected: the `Cannot generate coverage report` error is printed
    after the execution phase instead of during analysis, and nothing stops.
  - `--nobuild`: the block is never reached; Java still creates the report actions.
  - a failing build without `--keep_going`: the block is skipped.
- Masked: coverage on the non-merged path is not ported, and the main case stops loudly. None
  of the three gives a wrong report. Not probed.
- Expected: when the non-merged path is ported, compute the artifacts where
  `BuildView.update` does; until then a line in the PORT note at `build.rs:2276`.

### S4

`build.rs:2274`: `actions_wrapper.is_some()` alone says what the guard means.

## Open items, not blocking

- V3 (`coverage_environment`, first against last of a repeated key): queued.
- The probes used a fixer's binary. One run of a non-merged `test` scenario with a binary of a
  checkpoint commit would close this.
