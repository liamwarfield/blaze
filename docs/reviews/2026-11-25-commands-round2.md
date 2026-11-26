# Review round 2: commands (F1-F6 re-check; `config`, `dump`, `print_action`)

Date 2026-11-26 (started 2026-11-25). Reviewer (Fable 5.2), read-only: no code, test, script,
expectation or baseline edited, no Cargo, no subagents, no commit. Area `commands`.

## Verdict

**STOP**, bound to `f71e692`. Blocking finding: **R1**.

F1 to F6 of round 1 and the exit-code fix are fixed as the Java reads; none of them blocks any
more. The STOP is for one wrong console line that the fixer's own `print_action` recording
shows and the response does not mention (R1), and it asks for one oracle probe of a second line
(R2).

## How I worked, and what I could not do

- I started no Bazel or Blaze process. The first session ended on a rate limit before any probe
  ran; in the second, `root-suite-5f68157` was `active`. Every statement is from reading the
  Rust at `f71e692` against the Java at the pin, or from the fixer's recordings under
  `~/.cache/blaze/work/commands-fix-1/out/` (binary built from an uncommitted tree: secondary
  evidence).
- There is no binary of `f71e692` on disk.
- I created `~/.cache/blaze/work/review-commands-2/` (a three-target workspace, no output root,
  nothing run in it). It can serve the re-check or be deleted.
- Paths: Rust relative to the repository, Java relative to `src/main/java` at the pin.

## Scope read

- The diff `e8b79c0..f71e692` of `blaze_sky_top/src/build_tool.rs`, `skyframe_executor.rs`,
  `blaze_runtime/src/commands/{build,run,fetch,mod_command}.rs` (the hunks of this fixer),
  `project_options.rs`, `bzlmod_cycle_reporters.rs`, `bazel_modules.rs` (debug module),
  `blaze_options/src/{invocation_policy,parser,parser_impl}.rs`, `rc/handler.rs`,
  `blaze_repo_rules/src/setup.rs`.
- In full: `commands/config.rs`, `commands/dump.rs`, `commands/print_action.rs`, against
  `ConfigCommand`, `ConfigCommandOutputFormatter`, `ConfigurationForOutput`,
  `FragmentForOutput`, `FragmentOptionsForOutput`, `DumpCommand`, the dump methods of
  `AbstractInMemoryMemoizingEvaluator`, `PrintActionCommand`.
- Java read for the re-check: `BuildTool.buildTargets`, both analysis/execution methods,
  `processRequest`, `stopRequest`, `BuildResult.setUnhandledThrowable`, `SkymeldModule`,
  `BuildSummaryStatsModule`, `BlazeCommandDispatcher` (option re-parse), `FetchCommand.exec`,
  `BzlmodRepoCycleReporter`, `BzlLoadCycleReporter`, `StarlarkRepositoryDebugModule`,
  `BazelRepositoryModule` (exit codes).

## Scope not read

- `commands/coverage.rs`, `bep.rs`, `exclusive_test_order.rs`, `dispatcher.rs`, `interrupt.rs`,
  `jvm_signals.rs`, `bugreport.rs` in the same range: other agents' work.
- The `HashMap` tree-bin port in `help.rs` and the `fetch --force` wiring beyond its diff (S3
  items of round 1; the response gives a JDK recording and an oracle probe for them).
- `CompactPersistentActionCache.dump` (`blaze_exec`), `PrintActionVisitor.java` itself (the Rust
  visitor was read against `BipartiteVisitor` semantics only from its comments), the option
  classes of the three commands in `blaze_optdefs` (unchanged in the range).

## Part 1: round-1 findings

| Finding | State | Notes |
| --- | --- | --- |
| F1 | fixed | see below; one neighbouring case open (R2) |
| F2 | closed as S3 | `unported!` when an expansion has more than one repeatable sub-flag |
| F3 | fixed | one leftover S2 (R4) |
| F4 | fixed | port matches the Java |
| F5 | closed as S3 | `unported!` on any unknown field |
| F6 | fixed | interrupt returns before the post |
| exit code | fixed | codes match `BazelRepositoryModule` and `CommandEnvironment` |

- **F1.** `build_tool.rs:761` sets the flag after `evaluateTargetPatterns` (validator included,
  both calls), after the project file is evaluated and applied, and before the analysis.
  `build.rs:822` and `:878` read it. Per exit path, against `BuildTool.java:286-415, 594-613`:
  - validator rejection, target-pattern errors, project-file errors, an interrupt before the
    analysis: flag unset, no `ExecutionTool` in Java either. Matches.
  - analysis error or interrupt in the merged path: flag set, Java has created the
    `ExecutionTool` (`BuildTool.java:613`). Matches.
  - `--nobuild` (not merged, `SkymeldModule.java:43-52`): the flag is not consulted; the
    statistics are initialised by `ExecutionToolConstruction` only when the analysis returned,
    as `BuildTool.java:511-520`. Matches, except `--noanalyze --nobuild` (R2).
  - Evidence for the three `run` cases is the fixer's `scripts/run/compare` run; I did not
    re-run it.
- **F2.** The reason given is true: `OptionDefinition` holds the Rust struct name without its
  package and the snake_case field name (`blaze_options/src/definition.rs:111-112, 172`), so
  `Field.hashCode()` cannot be computed. The loud path is reached only with two or more keys,
  where the order matters.
- **F3.** The applier parses into a clone of the parser `exec` received, at `COMMAND_LINE`
  priority with the project file as source, then the Starlark flags plus the skipped args, then
  `createBuildOptions`, as `BuildTool.java:352-400`. The fixer's statement that
  `env.getOptions()` keeps the first parse is right for analysing commands
  (`BlazeCommandDispatcher.java:706`: the second parse uses a new parser). The crash path
  (`catch (Throwable)`, then `checkState` in `setUnhandledThrowable`) reads as the response
  says; the oracle result (exit 37, bare `IllegalStateException`) is the fixer's probe, not
  mine.
- **F4.** Both reporters match the Java branch by branch: predicate sets, the filtered key list,
  the printer's cases, the path walk, the messages, the repository-definition request. The
  `ModuleFileValue.Key` text is the record's `toString` (`blaze_bzlmod/src/skyframe/keys.rs:66`).
- **F5.** `has_unknown_fields` covers nested messages; unknown values of closed enums land in
  the unknown field set of `DynamicMessage`. The loud path is wider than needed (a policy with
  an unknown field that is never printed would behave the same in Java); acceptable.

## Findings of this round

### S1

**R1. With an empty target set, Blaze prints `INFO: Analyzed 0 targets (...)`; Bazel prints no
such line.** Blocking.

- Evidence: `commands-fix-1/out/pa-none.{o,b}.err` (`print_action` with no targets).
  - oracle: `INFO: Found 0 targets...` directly after the repo-mapping line.
  - Blaze: `INFO: Analyzed 0 targets (4 packages loaded, 6 targets configured).` first.
  - The same recording has the `Usage: bazel build` warning that the response reports as
    fixed (`build.rs:414`); the response does not mention this second line.
- Java: the line is the UI's reaction to `AnalysisPhaseCompleteEvent`. In the merged path it is
  posted only from `SkyframeBuildView.analysisFinishedCallback` (`SkyframeBuildView.java:713,
  911-957`), which runs when the last top-level target concludes its analysis; with no
  top-level target it never runs.
- Where: the build driver shared by `build`, `test`, `run`, `fetch` and `print_action`, not
  `print_action.rs`. `build` with no targets very likely shows it too; no recording of that
  case exists (`work/cmds/results` has none).
- Limits: the Blaze side is the fixer's binary, built before its last rebuild; nothing in that
  rebuild touches the line.
- Fix expected: post the analysis-complete line only where Java posts the event; then record
  `build` and `print_action` with no targets against the oracle.

**R2. `build --noanalyze --nobuild`: no `Critical Path` on the summary line (by reading, not
probed).** Not blocking until probed.

- Java: not merged because of `--nobuild`; `BuildTool.java:520` creates the `ExecutionTool`
  outside `if (performAnalysisPhase)`, so `BuildSummaryStatsModule.executorInit` runs and the
  line ends `, Critical Path: 0.00s`.
- Rust: `build_tool.rs:771-786` calls the post-processor (the only initialiser in this path,
  `build.rs:1122`) only when `perform_analysis_phase`; the fallback at `build.rs:876` requires
  the merged path.
- Existed before the fix. One oracle probe settles it; if Bazel prints no critical path there,
  the finding is withdrawn.

### S2

**R3. `config` and the graph dumps report an empty server after a build.**

- `dump.rs:43` hands `config` and `dump` a new empty evaluator. After `build //x`,
  Bazel's `config` lists the configurations (exit 0), `dump --skyframe=summary` counts the
  graph and `dump --rules` lists rules. Blaze prints `No configurations found` (an error exit),
  `Node count: 0` and `No rules in Bazel server`.
- This follows from ruling D2 (no graph kept between commands), and the module comments and
  ignored tests say so. But the commands return a normal-looking result instead of failing
  loudly, and the `config` message advises a sequence that cannot work in Blaze.
- Not checked: whether D2's stated oracle (Bazel with `--notrack_incremental_state`) also
  answers `config` after a build. By my reading the evaluator is reset at the next command
  that syncs package loading, and `config` (`buildPhase = NONE`) does not; that would make
  Blaze differ from D2's own oracle. I am not sure of this.
- Expected: root rules whether these stay as they are or become `unported!` until the kept
  graph exists; either way list them in STATUS as not functional.

**R4. Project-file flags: no `CanonicalCommandLineEvent` / `UpdateOptionsEvent`.**

- `project_options.rs:153` and `build_tool.rs:736` (PORT comments). Java replaces the tentative
  canonical command line in the build event stream (`BuildTool.java:375-390`). Blaze's stream
  for a build with project flags and a build event file lacks the project's flags, silently.
- Masked: needs a project file with flags; none in the corpus. Belongs to the build-events
  area; `unported!` when a stream exists would make it loud.

### S3 (honestly marked)

- `print_action`: every path that prints an action is `unported!`
  (`print_action.rs:402`, `AbstractAction.getExtraActionInfo`); `--compile_one_dependency` fails
  earlier in the target pattern phase. Only the paths that print nothing work. The control flow
  around them matches `PrintActionCommand.java:144-318`, and the two recorded error cases
  (`pa-fg`, `pa-gen`: `ConfiguredTarget(<label>, <hash>) is not a supported target kind`) agree
  with the oracle.
- `dump`: any dump over a non-empty graph, `--memory`, and the two Skyfocus dumps are
  `unported!`; with the empty graph they are unreachable or print the empty form. The fixer's
  recordings of `dump` with no option, `--rule_classes`, `--rules`, `--packages`,
  `--skyframe=summary|count|deps|function_graph` and `--skylark_memory` have stdout identical
  to the oracle's (a fresh `--batch` server on both sides).
- `config`: ten `ConfigCommandTest` tests ignored (need the kept graph).
- F2 and F5 as above.

### S4

- `config.rs:475-745` is a second port of `ConfigurationForOutput`, `FragmentForOutput`,
  `FragmentOptionsForOutput` and the text and JSON writers. The first is
  `blaze_build_config/src/config_output.rs` (commit `778a9c1`, used by cquery's proto output
  and compared with the oracle's `config --output=json` in W4). One should go.

## `config.rs` formatting code, read against the Java

Nothing runs this code, so I compared it statement by statement. I found no difference.

- Model: the fragment list is the registry's, sorted by class name, with a second add of the
  same name dropped; fragment options sorted by class name plus `user-defined`; `define` left
  out of `CoreOptions`; Starlark options and `--define:<key>` entries with a duplicate key
  failing; `skyKey` as `BuildConfigurationKey[<checksum>]`. The helpers it uses (`as_map`,
  `java_string_of`, the Java class names, the key's text) are the ones the W4 port verified
  against the oracle.
- Selection: order by checksum, prefix match, the "not found" and "ambiguous" texts, the first
  failing id of a diff winning, `Too many config ids.` after the empty-graph check.
- Diff: rows only for fragments with a difference, sorted by fragment then option, null shown
  as `null`, the ids printed as the user typed them.
- Text writer: every format string and line end as `ConfigCommandOutputFormatter.java:63-125`.
- JSON writer: field order of Gson's reflective adapter, `Pair` as `first`/`second`, compact
  output, HTML-safe escaping (`<`, `>`, `&`, `=`, `'`, U+2028, U+2029, control characters).

## Open items that do not block

- Probe R2 and the D2 question of R3 once the suite run ends.
- Re-run `scripts/run/compare` and the F3 and F4 probes with a binary of the fix commit; the
  evidence in the response is from an uncommitted tree.
- `StarlarkRepositoryDebugSubscriber` was read, not exercised: the cycle probe has no
  repository definition to print.
