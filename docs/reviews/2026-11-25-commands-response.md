# Response: commands and option machinery (review of 2026-11-25, STOP at e8b79c0)

Implementer `commands-fix-1` (Opus 5.5), 2026-11-25. Answers `docs/reviews/2026-11-25-commands.md`
finding by finding.

Tree: HEAD `70f2eb4` plus uncommitted work. Other agents' uncommitted changes are in the same
working tree. Binary: `target/debug/blaze` of that tree, copied to
`~/.cache/blaze/work/commands-fix-1/bin/blaze`. The `run` comparison used the binary with sha256
`830a0264e9bdef32…`; the last rebuild (exit-code fix) is `68dcc33c273e7758…`. Oracle: the pinned
`bazel-9.3.2-linux-x86_64`.

Probes ran under `systemd-run --user --wait --pipe -p MemoryMax=12G` with `--batch`,
`--nohome_rc --nosystem_rc` and their own output user roots
(`~/.cache/blaze/work/commands-fix-1/our-{oracle,blaze}`), in workspaces under
`~/.cache/blaze/work/commands-fix-1/`. Raw outputs are in `…/out/<case>.{o,b}.{out,err}`.
"Filtered stderr" means without the oracle's JVM warning, Blaze's server-start and install
extraction lines, and the timer-driven progress lines (`Loading:`, `Computing main repo
mapping:`, `Analyzing:`, empty lines), with times replaced by `Ns`.

## F1 (S1): `run` printed `, Critical Path: 0.00s` after a validator failure

- Change: the lifecycle, not the line.
  - `crates/blaze_sky_top/src/build_tool.rs:516` adds the flag `analysis_and_execution_entered`.
  - `build_tool.rs:761` sets it where `buildTargets` enters
    `buildTargetsWithMergedAnalysisExecution` (Java `BuildTool.java:409`). That is after
    `evaluateTargetPatterns`, the validator and the project file. The accessor is at
    `build_tool.rs:543`.
  - `crates/blaze_runtime/src/commands/build.rs:822` and `:878` use the flag instead of the
    graph lookup. The graph lookup (`target_patterns_were_evaluated`) is deleted.
  - Java's `ExecutionTool` (and with it `BuildSummaryStatsModule.executorInit`) exists exactly
    when that method was entered (`BuildTool.java:613`).
- While there: `evaluateTargetPatterns` for `targetsForProjectResolution` now also runs the
  validator, as Java's does (`build_tool.rs:824`, `evaluate_target_patterns_for`; Java
  `BuildTool.java:315-329`).
- Evidence: `scripts/run/compare --binary …/bin/blaze --project all --label commands-fix-1-run`,
  raw outputs under `~/.cache/blaze/run-compare/commands-fix-1-run/`.
  - `not-executable`, `two-targets` and `no-target` are now KNOWN-GAP, differing only in
    `b1_server_start`.
  - Hand diff: `INFO: Elapsed time: Ns` on both sides; no `Critical Path` on the Blaze side.
    Exit code 2 on both.

## F2: order of repeatable sub-flags in an expanded policy

- Change: `crates/blaze_options/src/invocation_policy.rs:637`. With more than one repeatable
  sub-flag key, the code now fails loudly:
  `unported!("InvocationPolicyEnforcer.expandPolicy", …)`. The comment is corrected: the order
  is deterministic, a `HashMap` keyed by
  `OptionDescription.hashCode = Field.hashCode() + evaluatedExpansion.hashCode()`.
- Why not reproduce the order: `Field.hashCode` is the hash of the declaring class's binary name
  XOR the hash of the Java field name. Blaze's option definitions carry neither. They carry the
  Rust struct name and the snake_case field name, which is lossy. One key, the only case at the
  pin, behaves as before.

## F3: project-file flags

- Change: `crates/blaze_runtime/src/project_options.rs`.
  - The applier parses into a copy of the command's own parser (`OptionsParser` and
    `OptionsParserImpl` now derive `Clone`, `crates/blaze_options/src/parser.rs:249`,
    `parser_impl.rs:103`). Before, it used a fresh parser over the same classes.
  - It parses at `PriorityCategory.COMMAND_LINE` with the project file as source
    (`project_options.rs:118`), then the Starlark flags into the same parser, then calls
    `createBuildOptions(parser)`. The hand merge of values into `BuildOptions` is gone.
  - The copy starts from the parser `exec` received (`build.rs`:
    `Arc::new(options.clone())`), not from `env.get_options()`. The latter keeps the
    dispatcher's first parse, as Java's `env.getOptions()` does.
  - After `processRequest`, the updated parser goes to `RunCommand` through
    `RunBuildOutcome.updated_options` (`build.rs:212`, `:790`). `run.rs:205` reads the options
    after the build from it, as Java reads the parser it handed to `BuildTool`.
- Bad value: an `OptionsParsingException` now crashes with a message-less
  `IllegalStateException` (`project_options.rs:99`).
- Oracle probe (`wsproj`: PROJECT.scl buildable unit with `--compilation_mode=bogus`,
  `build //test:g`):
  - The oracle crashes, exit 37, `java.lang.IllegalStateException` with no message. The
    `catch (Throwable)` keeps the checked exception as `crash`. Then `stopRequest` calls
    `BuildResult.setUnhandledThrowable`, whose `checkState` (`BuildResult.java:126`) throws.
    This matches the reviewer's reading (crash, 37); the thrown exception is the `checkState`'s.
  - Blaze: exit 37, `FATAL: bazel crashed due to an internal error`, panic message
    `IllegalStateException`. The `INFO: Reading project settings…`, the `WARNING: This build
    uses a project file…` and `INFO: Applying flags…` lines are identical.
  - The stack trace text cannot match (see `bugreport.rs`).
- Valid values (`wsproj2`: `--compilation_mode=opt` and `--run_under=echo under` in the project
  file, `run //test:g -- a`): stdout `ran a` on both sides (run_under is read before the build
  in Java and in Blaze); filtered stderr identical; exit 0 on both.
- Not done: the `CanonicalCommandLineEvent` / `UpdateOptionsEvent` posts. The build event
  stream is another area; a PORT comment says so.

## F4: `mod` dropped the cycle report

- Change: the two reporters are ported.
  - `crates/blaze_runtime/src/bzlmod_cycle_reporters.rs` holds `BzlmodRepoCycleReporter`,
    `BzlLoadCycleReporter` and `RequestRepositoryInformationEvent`. They live there with their
    only Blaze user; a PORT comment gives the reason.
  - `StarlarkRepositoryDebugModule.beforeCommand` now registers a subscriber
    (`bazel_modules.rs:521`). It records `StarlarkRepositoryDefinitionLocationEvent`s and
    answers `RequestRepositoryInformationEvent`s.
  - `mod` reports cycles through `CyclesReporter` before the message
    (`mod_command.rs:145`; Java `ModCommand.java:295-303`).
- Oracle probe (`wscycle`: the setup of
  `mod_command_test.testModCommandWithCycleDoesNotCrash`, `mod graph`): stderr identical apart
  from the JVM warning. That is the `Circular definition of repositories generated by module
  extensions…` block with the drawn cycle, then
  `ERROR: Unexpected error during module graph evaluation.` Exit 37 on both sides.

## F5: unknown fields of a binary invocation policy

- Change: `invocation_policy.rs:142` and `:161`. When the parsed message, or any nested
  message, has a non-empty unknown field set (an unknown field number, or an unknown value of a
  closed enum), parsing fails loudly with `unported!("InvocationPolicyParser.parsePolicy", …)`.
  Before, those fields were dropped.
- Tests: `binary_policy_with_unknown_field_fails_loudly`,
  `binary_policy_with_nested_unknown_field_fails_loudly`,
  `binary_policy_without_unknown_fields_parses`.

## F6: an interrupted `fetch` posted `NoBuildRequestFinishedEvent`

- Change: `fetch_all` and `fetch_repos` return `Result<BlazeCommandResult, FetchInterrupted>`
  (`fetch.rs:173`). `exec` returns `Fetch interrupted: null` before the post, as Java's `catch`
  does (`fetch.rs:129`; Java `FetchCommand.java:133-136`). Not probed: it needs an interrupt at
  that point.

## S3 items

- `fetch --force <targets>`:
  - `RunBuild` carries the command's injected `RepoPrecomputedValues` (`build.rs`, the
    `repo_precomputed` field). The build's executor is created with them through
    `query.rs` `create_skyframe_executor_with_execution_and_repo_precomputed`. `fetch.rs:99`
    passes the FORCE_FETCH value.
  - Probe on brotli, `fetch --force //:brotlicommon`: exit 0 on both sides, stdout identical,
    filtered stderr identical except two `currently loading:` progress lines on the oracle side.
- `help flags-as-proto` tree bins:
  - `JavaHashMap` (`help.rs`) now models `HashMap.TreeNode`: `treeify` (`help.rs:843`),
    `putTreeVal` (`:891`), `balanceInsertion`, the rotations, `moveRootToFront`, and the split on
    resize (keep the tree, rebuild it, or flatten at 6 nodes or fewer).
  - Test `java_hash_map_tree_bins_follow_java_order` (`help.rs:1220`) uses 404 keys: 64 with one
    `String.hashCode` and 40 equal modulo 64. Its expected order was recorded with
    `java.util.HashMap` by `~/.cache/blaze/work/commands-fix-1/hashprobe/HashOrder.java`
    (OpenJDK 25; HashMap's tree code is unchanged since JDK 8). It passes.
  - `help flags-as-proto` stdout is still identical to the oracle's.
- Left as reported: line breaking outside the listed ranges; `run` with the remote output
  service; the crash-detail tests; the `RemoteRequestedInfoItemHandlerTest` fixture.

## S4

- `help.rs:1025`, `license.rs:91`: the comments now name the real reason. No build event stream
  is created for commands outside `ALLOWED_COMMANDS`.
- `rc/handler.rs:349`: `handleProjectFiles` does nothing in Bazel, because no module provides a
  `ProjectFile.Provider`.

## The thirteen `run` cases the review left to the tool, diffed by hand

The 2026-11-19 recordings are stale (other binaries), so the cases were rerun with
`scripts/run/compare` on the binary above and the oracle. The raw files were then diffed by
hand.

For every one of the 13, stderr differs only in lines that come from Bazel's warm server or
from progress:

- the `Starting local Bazel server (9.3.2)…` line (Blaze only);
- the package and target counts in `Analyzing:` and `INFO: Analyzed …` (the oracle reuses its
  server's graph);
- `INFO: 1 process: 1 internal.` against Blaze's `… 6 action cache hit, 1 internal.`;
- the oracle's `WARNING: Build option --run_under has changed, discarding analysis cache`;
- timer-driven `Analyzing:` repeats and empty lines (`test-target`: Blaze, cold, prints one more
  progress pair);
- elapsed times.

Stdout is identical, or differs only in the side's own paths: workspace copy, output user root,
output-base hash in `BUILD_WORKSPACE_DIRECTORY` / `BUILD_WORKING_DIRECTORY` /
`BUILD_EXECROOT` / `RUNFILES_DIR` / `TEST_SRCDIR`, and `--run_under`'s path. `subcommands` also
differs in the `BUILD_ID` UUID in the printed command. Exit codes are equal.

| Case | stdout | stderr beyond warm-server and progress lines |
| --- | --- | --- |
| exit-code | identical | none |
| env | paths only | none |
| subdir | paths only | none |
| run-in-cwd | paths only | none |
| show-args | identical | none |
| run-under-cmd | paths only | none |
| run-under-label | paths only | none |
| script-path | identical | none |
| run-false | identical | none |
| subcommands | identical | paths and `BUILD_ID` in the printed command only |
| test-target | paths only | none |
| build-fails | identical | none (the gcc error line differs only in the workspace path) |
| no-target | identical | none |

The tool classes `args` and `re2-test` as IDENTICAL, and `batch`, `re2-test-args` and
`re2-runs-per-test` as DIFFERENT. I did not re-read those three beyond the tool's verdict.

## Changes outside the files the review names

- `crates/blaze_repo_rules/src/setup.rs` adds `RepositorySetupError` with Java's
  `ExternalRepository` code. `crates/blaze_sky_top/src/skyframe_executor.rs:301` maps it, and
  `tests/setup_test.rs` gets one new test.
- Why: the repository setup errors lost their code and exited 36. The oracle exits 2 for
  `--repo_contents_cache` inside the workspace (`BAD_REPO_CONTENTS_CACHE`); Blaze now exits 2.
- What remains: the oracle prints an extra empty line after that message. Java raises it in
  `BazelRepositoryModule.beforeCommand`, before the UI handler exists; Blaze raises it at
  executor creation. Not changed.
- `build.rs:414`: the `Usage: … build <options> <targets>` warning is `BuildCommand.exec`'s
  only. Before, `print_action` with no targets printed it; the oracle does not.

## Round 2

Implementer `commands-fix-2` (Opus 5.5), 2026-11-26. Answers
`docs/reviews/2026-11-25-commands-round2.md` (STOP at `f71e692` on R1).

Tree: HEAD `316720a` plus uncommitted work; other agents' uncommitted changes are in the same
working tree. Binary: `target/debug/blaze` of that tree, copied to
`~/.cache/blaze/work/commands-fix-2/bin/blaze`.

- The R1, R2 and R4 recordings were taken on sha256 `18a546a439e744d3…`.
- They were recorded again on `d54ff2b8cbd98d87…` (after the last change of this round) with
  the same results. The one exception is the `fetch` case below.
- Oracle: the pinned `bazel-9.3.2-linux-x86_64`, with `--batch` and one output user root per
  side.
- "Filtered stderr" drops:
  - the oracle's JVM warning;
  - Blaze's server-start and install-extraction lines;
  - the timer-driven progress lines (`Loading:`, `Computing main repo mapping:`,
    `Analyzing:`, `currently loading`, empty lines);
  - the times (replaced by `Ns`).
- Raw files: `~/.cache/blaze/work/commands-fix-2/out/`.

### R1 (S1): `INFO: Analyzed 0 targets (...)` with an empty target set

The event, and everything that hangs on it, is now posted only where Java posts it.

- `crates/blaze_sky_top/src/build_view.rs:92`: `AnalysisResult::has_top_level_keys` (new).
  `build_view.rs:568` records the number of top-level keys: configured-target keys plus
  top-level aspect keys, Java's `topLevelKeys` (`SkyframeBuildView.java:695`).
- `crates/blaze_sky_top/src/analysis_phase_runner.rs:244`:
  `analysis_phase_complete_event_posted`.
  - Without merged analysis and execution (`--nobuild`, `cquery`), always: as
    `AnalysisPhaseRunner.java:429`.
  - With it, only when there is a top-level key. `AnalysisOperationWatcher.java:70-86` calls
    `analysisFinishedCallback` (which posts `AnalysisGraphStatsEvent` and
    `AnalysisPhaseCompleteEvent`, `SkyframeBuildView.java:911-957`) only when the last expected
    key concludes. With an empty set it returns at once and never calls it.
  - `analysis_phase_runner.rs:206` posts the event (the UI's `Analyzed ...` line) under that
    condition.
- `crates/blaze_runtime/src/commands/bep.rs:1704`: the metrics collector's analysis-phase data
  (`onAnalysisPhaseComplete`) and the graph stats are recorded under the same condition. The
  `AnalysisPhaseStartedEvent` (`BuildView.update`) is still always posted.

Evidence: workspace `commands-fix-1/ws`. Each command was run with no targets and with
`-- //pkg:all -//pkg:all` (a pattern that expands to nothing).

| Command | Exit (oracle = Blaze) | stdout | filtered stderr |
| --- | --- | --- | --- |
| build | 0, 0 | identical | identical |
| test | 4, 4 | identical | identical |
| run | 2, 2 | identical | identical |
| fetch | 0, 0 | identical | identical (see the note) |
| cquery | 2, 0 | identical | identical |
| print_action | 1, 1 | identical | identical |

- Neither side prints `Analyzed` for `build`, `test` or `print_action`.
- Both print `INFO: Analyzed 0 targets (...)` for `fetch` with a pattern and for `cquery` with
  a pattern. Those requests are not merged (`fetch` injects `--nobuild`), so
  `AnalysisPhaseRunner` posts the event, as the Java reads.
- Note: on the re-recording, `fetch` with no targets differs by two `DEBUG:` lines about
  `pybind11`. The oracle's output base fetched that repository on its first run; Blaze's
  output base had fetched it in the first recording, which was identical. This is a difference
  of output-base state, not of the code.

### R2: `build --noanalyze --nobuild` printed no `Critical Path`

- Oracle probe: `INFO: Elapsed time: 0.904s, Critical Path: 0.00s`, then `INFO: 0 processes.`
  The finding stands.
- Change:
  - `crates/blaze_sky_top/src/build_tool.rs:421`: `AnalysisPostProcessor::process_without_analysis`
    (new, default no-op).
  - `build_tool.rs:828` calls it when the request does not analyze, as
    `new ExecutionTool(env, request)` runs outside `if (performAnalysisPhase)`
    (`BuildTool.java:520`).
  - `crates/blaze_runtime/src/commands/build.rs:1172`: `ExecutionToolConstruction` implements
    it with the same construction `process` does (`BuildSummaryStatsModule.executorInit` and the
    construction checks).
- Evidence: same probe on Blaze. `INFO: Elapsed time: 0.171s, Critical Path: 0.00s`, exit 0
  on both sides, filtered stderr identical, stdout identical (empty).

### R4: project-file flags posted no `CanonicalCommandLineEvent`

- `crates/blaze_runtime/src/commands/bep.rs:1494`: `post_project_canonical_command_line`.
  - It posts a non-replaceable `CanonicalCommandLineEvent` of the updated parser, as
    `BuildTool.java:375-390`. The streamer drops the dispatcher's tentative one.
  - `UpdateOptionsEvent` has no subscriber at the pin (searched: only its declaration and
    `BuildTool` name it), so it is not posted.
- `crates/blaze_runtime/src/project_options.rs:67`: the applier calls that poster after the
  native and Starlark flags are parsed. `build.rs` builds the poster when the command has a
  stream.
- The `project_evaluated` hook (`build_tool.rs`, `bep.rs:2227`, `cquery.rs`) now takes whether
  project flags were applied. It releases the tentative event only when none were
  (`ReleaseReplaceableBuildEvent`, `BuildTool.java:391-397`). Before, it always released it.
- Evidence:
  - Workspace `commands-fix-1/wsproj2`: PROJECT.scl buildable unit with
    `--compilation_mode=opt` and `--run_under=echo under`; `build //test:g
    --build_event_json_file=...`.
  - Both streams have the same 25 events in the same order (ids compared).
  - The canonical command line's sections are identical after the side's paths and install
    hash. The command options end in `--compilation_mode=opt`, `--run_under=echo under` on both
    sides; the only other difference is `--startup_time` (2 against 6 ms).

### R3

Ruled by root: `config` and the graph dumps keep reporting an empty server until the kept graph
(package I3) exists. No change.

### S4: two ports of the `config` output classes

They are equivalent for every reachable input, so the duplicate is gone. The differences, all
unreachable:

- Mine failed on a duplicate user-defined key: Starlark labels and `--define:` keys cannot
  collide.
- `blaze_build_config` checks the simple class name `CoreOptions` where mine compared the full
  name: there is one such class.
- Mine sorted by UTF-16 code units: the names are ASCII.

`crates/blaze_runtime/src/commands/config.rs` now uses
`blaze_build_config::config_output` (`ConfigurationForOutput`, `FragmentForOutput`,
`FragmentOptionsForOutput`, `text_write_configuration_ids`, `text_write_configuration`,
`json_write_configuration`, `gson_string`). It keeps only what that module lacks:

- the JSON list of configurations;
- the JSON ids object;
- the text and JSON diff writers;
- the selection and diff logic of `ConfigCommand`.

`config.rs` went from 1,094 to 654 lines.

### Found while working on Part B

On the non-merged path (`--nobuild`), Blaze never set the result's configuration, actual
targets and test targets. Java sets them once the analysis returns
(`BuildTool.java:540-542`).

- Change: `crates/blaze_runtime/src/commands/build.rs:1159`.
- Before the fix, `vendor <targets>` (`TargetFetcher` injects `--nobuild`) vendored nothing.
  This was 6 of the 19 functions of `bazel_vendor_test.py`.
