# I3 result: a persistent server keeps its Skyframe graph

Implementer: task `i3`, 2026-11-26. Tree: HEAD `5f68157`..`e476a59` plus the uncommitted work of
this task and of other agents. `J:` = `~/.cache/blaze/src/bazel-9.3.2/src/main/java/`.

## How it works

Java keeps one `SequencedSkyframeExecutor` per workspace. Blaze builds a `SkyframeExecutor` (its
functions and the services they capture) for every command, and more than one per command (the
dispatcher's, then the command's, `info`'s). What is kept is the graph:
`blaze_sky_top::incremental::SkyframeIncrementalState` (held by `BlazeWorkspace`) receives the
evaluator when an executor is dropped, strips its functions when the next executor starts being
built (so two generations of services and action caches are never alive at once), runs Java's
per-command `sync` on it, and the next executor builds its evaluator over that graph
(`EvaluatorBuilder::build_reusing`, blaze_skyframe; precomputed values the executor injects go
through the differencer, so equal ones are pruned).

Mode: `BLAZE_INCREMENTAL` in the server's environment (decision D6). `none` keeps nothing (every
command starts from an empty graph, the behaviour of decision D2); unset or any other value keeps
the graph. `BLAZE_INCREMENTAL_DEBUG` prints to the server log why a graph was not reused. Batch
and the in-process mode run one command per process and never reuse a graph.

Per command, in Java's order (`J:com/google/devtools/build/lib/skyframe/SequencedSkyframeExecutor.java:262`
`sync`, `SkyframeExecutor.java:3040` `sync`, `:3774` `handleDiffs`):

1. `decideKeepIncrementalState` (`--track_incremental_state`, `--batch` with
   `--discard_analysis_cache`): a change, or no tracking, resets the graph.
2. `preparePackageLoading`: the emitted-event state is cleared; the precomputed values are
   re-injected by the executor (pruned when equal); `BUILD_ID` gets the command id.
3. `setDeletedPackages`: `PACKAGE_LOOKUP` nodes of packages whose deleted state changed are deleted.
4. `invalidateTransientErrors`; `lastAnalysisDiscarded` (after `--discard_analysis_cache`):
   `dropConfiguredTargetsNow`.
5. `handleDiffs` without diff information (every package root): `applyInvalidation`, then
   `FilesystemValueChecker.getDirtyKeys` over the done values with `MissingDiffDirtinessChecker`
   and `ExternalDirtinessChecker` (`--experimental_check_external_repository_files`,
   `--experimental_check_external_other_files`, `--experimental_check_output_files`, the external
   files knowledge), `getExtraKeysToInvalidate` for externally modified repositories,
   `handleChangedFiles`.
6. `handleClientEnvironmentChanges`, `handleRepositoryEnvironmentChanges`.
7. For an executor that executes: `detectModifiedOutputFiles` (`getDirtyActionValues`).
8. At `BuildView.update`: `shouldDiscardAnalysisCache` (Java's message, `OptionsDiff`, the
   `TestConfiguration` option-diff predicate, `--max_config_changes_to_show`,
   `--allow_analysis_cache_discards`) and `setConfiguration` (`handleAnalysisInvalidatingChange`
   with `shouldDeleteOnAnalysisInvalidatingChange`).
9. At the end of the command: `--nokeep_state_after_build` resets the graph
   (`BlazeRuntime.afterCommand`); `clean` resets it (`BlazeWorkspace.resetEvaluator`); the
   `TimestampGranularityMonitor` wait.

Events: with tracking, the evaluator stores events with `DEFAULT_EVENT_FILTER_WITH_ACTIONS` and
replays them for nodes it does not re-run (decision 9.2 of docs/skyframe-incremental.md).

## Change sources

| Source | Handling |
| --- | --- |
| Files under the package roots (sources, BUILD, .bzl, glob matches, packages added/deleted, symlinks) | precise: `FILE_STATE` / `DIRECTORY_LISTING_STATE` checked by `FilesystemValueChecker` |
| Same-size rewrite, edit in the same second | precise (contents proxy: ctime, mtime, inode); `TimestampGranularityMonitor` waits at the end of a command that read a file with the command's start time |
| `MODULE.bazel`, `include()`d files, module files of non-registry overrides, patches, `MODULE.bazel.lock` | precise: now `FILE` dependencies as in Java (were direct reads) |
| `platform_mappings` | precise: now a `FILE` dependency (was a direct stat) |
| Source artifacts (action inputs) | precise: `ARTIFACT` now requests `FILE` as in Java (was a direct stat and digest) |
| Files in external repositories | precise: `ExternalDirtinessChecker`, re-fetch of modified repositories |
| Output files modified or deleted | precise: `detectModifiedOutputFiles` |
| Client environment (`--action_env` inheritance, `PATH`) | precise: per-variable injection; `ACTION_EXECUTION` now depends on `CLIENT_ENVIRONMENT_VARIABLE` of the action's inherited variables as in Java |
| Repository environment (`--repo_env`, `getenv`) | precise: per-variable injection |
| Build options (analysis) | precise: `BASELINE_CONFIGURATION` / `BASELINE_EXEC_CONFIGURATION` are now precomputed dependencies as in Java; Java's message and deletion on an analysis-invalidating change |
| `--define`, Starlark flags | as build options |
| Non-analysis options (`--keep_going`, `--jobs`, execution options, output options) | nothing to do (Java: nothing) |
| Starlark semantics (`BuildLanguageOptions`) | **discards the graph** (the services capture a copy; Java would re-run nearly everything anyway) |
| `RepositoryOptions` that decide results (registries, overrides, injections, dev deps, compatibility checks, lockfile mode, vendor dir, yanked versions, module mirrors) | **discards the graph** (captured by `BzlmodConfig`) |
| Working directory, when a module or repository override is given | **discards the graph** (relative override paths) |
| Package path | **discards the graph** (captured roots) |
| Deleted packages | precise (`setDeletedPackages`) |
| Main repository mapping | **deletes the analysis nodes** when it changes (the configured target and aspect functions read it from a cell) |
| Volatile actions, actions notified of cache hits (tests), workspace status, build drivers | precise: `BUILD_ID` dependency as in Java |
| Registry contents | Java's one-hour `LAST_INVALIDATION` |
| `--discard_analysis_cache` | Java's next-command deletion and message |
| Interrupt, failed build | in-flight nodes and their reverse closure deleted at the next evaluation (I1) |
| `--watchfs` | no diff awareness: every root is checked (Java's path without diff information); correct, not faster |

## Audit: reads outside the graph (root's S2-1 item)

Method: the six sections C of `i2-worklist.md` re-verified against the tree, plus
`grep -rnE 'std::env::var|env::var\(|current_dir\(|SystemTime::now|Instant::now'` and a grep of
every mutable `static`/`thread_local!` in the crates whose functions run in the evaluator
(blaze_sky_loading, blaze_bzlmod, blaze_bzlmod_model, blaze_repo_rules, blaze_build_config,
blaze_analysis, blaze_sky_analysis, blaze_sky_exec, blaze_sky_top, blaze_packages, blaze_rules,
blaze_actions, blaze_exec, blaze_starlark).

| Function (key) | What it read | Java dependency | State |
| --- | --- | --- | --- |
| ARTIFACT (source) `blaze_sky_exec/src/artifact_function.rs` `create_source_value` | stat + digest of the source file | `FileValue.key(path)` `J:.../skyframe/ArtifactFunction.java:279` | **fixed**: requests `FILE`, metadata from the `FileValue` (`FileArtifactValue::create_for_source_artifact`) |
| PLATFORM_MAPPING `blaze_build_config/src/skyframe.rs` | stat of the mappings file | `FileValue.key(rootedMappingPath)` `J:.../skyframe/config/PlatformMappingFunction.java:92` | **fixed**: `FileValueLookup` seam, implemented with `FILE` in blaze_sky_top |
| MODULE_FILE (root) `module_file_function.rs` | `exists` + read of `MODULE.bazel` | `FileValue.key(moduleFilePath)` `ModuleFileFunction.java:296` | **fixed**: `FileValueLookup` seam (blaze_bzlmod), implemented with `FILE` in blaze_repo_rules `setup` |
| MODULE_FILE (includes) | `is_file` + read | batch `FileValue` keys `ModuleFileFunction.java:494` | **fixed** |
| MODULE_FILE (non-registry override) | read of the module file | `FileValue.key(moduleFilePath)` `:766` | **fixed** |
| MODULE_FILE (patches) | `is_file` of each patch | `FileValue` keys `:916` | **fixed** |
| BAZEL_LOCK_FILE `lock_file.rs` | read of the lockfile | `FileValue.key(lockfilePath)` `BazelLockFileFunction.java:82` | **fixed** |
| ACTION_EXECUTION | client environment of inherited variables | `CLIENT_ENVIRONMENT_VARIABLE` keys `ActionExecutionFunction.java:221` | **fixed** (key moved to `blaze_skyframe::precomputed`, I2 item X2) |
| ACTION_EXECUTION | per-build re-execution of volatile actions | `BUILD_ID` `ActionExecutionFunction.java:216` | **fixed** |
| BUILD_DRIVER | per-build re-evaluation | `BUILD_ID` `BuildDriverFunction.java:165` | **fixed** |
| BUILD_INFO (workspace status) | per-build status | `WORKSPACE_STATUS_KEY` precomputed | **fixed** with `BUILD_ID` (same schedule; PORT) |
| BASELINE_OPTIONS, BUILD_CONFIGURATION_KEY_VALUE (project scopes) | baseline options from a cell | `BASELINE_CONFIGURATION` / `BASELINE_EXEC_CONFIGURATION` precomputed | **fixed**: precomputed dependencies |
| REGISTRY (via config) | `SystemTime::now()` at every executor | `LAST_INVALIDATION` with the one-hour interval `BazelRepositoryModule.java:744` | **fixed**: value kept in the graph, replaced after an hour |
| CONFIGURED_TARGET / ASPECT | main repository mapping from `AnalysisServices` cell | Java reads it through Skyframe | **fail safe**: analysis nodes deleted when the mapping changes |
| Loading and analysis functions | Starlark semantics copies in services | `STARLARK_SEMANTICS` precomputed | **fail safe**: graph discarded on change |
| bzlmod / repository functions | `RepositoryOptions` captured in `BzlmodConfig` | precomputed values of `BazelRepositoryModule` | **fail safe** for the options that decide results; cache locations etc. need nothing (Java: module state) |
| ACTION_EXECUTION input discovery `skyframe_action_executor.rs` `skyframe_input_metadata_provider_get_input_metadata_checked` | metadata of a generated input from a stat | the `ActionExecutionValue` in the graph (no dependency) | **kept, PORT**: equal to the graph value while outputs are checked (`detectModifiedOutputFiles` dirties the generating action first); differs only under `--noexperimental_check_output_files` |
| Exec transition memo `dependency_resolver.rs` `Loaded` (build arena) | builtins exec transition by flag text | BZL_LOAD of the builtins file (depends on semantics) | **fixed**: keyed by the semantics too |
| RuleContext `rule_context.rs:2118` | `exists()` of an input path for an error message | none in Java either | same as Java |
| DirectoryTreeDigest, RepoRecordedInput, repository fetching, rctx/mctx file operations, registry `file:` URLs | file system | Java reads the same paths (non-hermetic keys, marker files) | same as Java |
| `BAZEL_SH` (`action_environment.rs`, `bazel_rule_class_provider.rs`) | server environment | Java reads the server's too | same as Java |
| `Instant::now` (action timing, sandbox deadlines, build view timer) | clock for durations | same | not an input |
| Arena memos (`DepsetMemos`, `ConvertedTransitions`, `StarlarkTransitionCache`, `MapEachAdaptors`, `ActionInputSets`, `LoadingPhaseHeaderInfos`, `GeneratorNameHeaps`, `KeepAlive`, `ByFilename`) | keyed by frozen-value address or value, pure functions of the key | Java: identity caches / GC | correct while the arena outlives the graph (the kept graph holds a lease) |
| `TopLevelAspectDrivers` (arena) | per-build guard | n/a | reset per build |
| `PACKAGE_PROGRESS`, `ANALYSIS_PROGRESS` | counters | per-command resets | reset per command (unchanged) |
| Native aspect `DEFINITION_CACHE`, `MODULE_BAZEL_ENVS`, regex caches | pure caches | Java has the same | fine |

## Not ported (ranked)

1. `DiffAwareness`, `DiffAwarenessManager`, `LocalDiffAwareness`, `WatchServiceDiffAwareness`,
   `BrokenDiffAwarenessException`, `WorkspaceInfoFromDiff`, `FileSystemValueCheckerInferringAncestors`
   (`--watchfs`): every root is checked instead (Java's fallback). Their JUnit classes
   (`LocalDiffAwarenessTest`, `DiffAwarenessManagerTest`) are not ported.
2. `SequencedSkyframeExecutorTest` (35 tests, a full analysis/execution harness) not ported; the
   behaviour is covered by the oracle probe and the randomized test below.
3. `getDirtyActionValues` tests of `FilesystemValueCheckerTest` (tree artifacts, remote files,
   batch stat) not ported; `BatchStat` (no Bazel output service provides one) not ported; remote
   outputs missing locally are always treated as modified (re-evaluated; never stale).
4. The counters of `ExecutionFinishedEvent` (modified output files) and the incremental build
   monitor are not ported (metrics).

## Known count differences against Bazel (D9), outputs and spawns equal

- After an edit of `platform_mappings` Blaze configures fewer targets than Bazel (2 vs 11):
  Blaze maps options through a `BuildConfigurationKeyValue` node (pre-existing PORT); Java reads
  the mapping inside the configured target's state machine, so every configured target depends on
  it directly.
- After an interrupted or failed no-keep-going build, Bazel re-analyzes more (13 vs 3 targets,
  5 vs 0 packages after an interrupt): Java's parallel evaluator has more nodes in flight at the
  abort, which are deleted with their reverse closure; the set depends on thread timing.
- When loading fails, Bazel writes no `--execution_log_json_file` and Blaze writes an empty one
  (seen in the probe's `break` step; not checked whether it depends on the kept graph).

## Evidence (2026-11-26)

Binary: `target/debug/blaze` built from HEAD `b6566fa` plus uncommitted work (this task's and other
agents'), sha256 `d52e40c03bc68fe5…`, copied to `~/.cache/blaze/work/i3/bin/blaze6`.

- `scripts/incremental/stress probe` (generated workspace; one server per side, the oracle first;
  exit code, stdout, INFO/WARNING/ERROR lines, spawns of `--execution_log_json_file`, every file
  under `bazel-bin`): 28 steps, 25 identical. Different: `platform-mappings` and `fix` (counts
  only, see "Known count differences"), `break` (no log vs empty log). Identical among others:
  no-op rebuild (`0 packages loaded, 0 targets configured`, `1 process: 1 internal.`), touch,
  same-size rewrite, source/BUILD/.bzl edits, glob add/delete, package add/delete, `--define`
  change and back (Java's "discarding analysis cache" warning), `--keep_going`, `--action_env`
  change, `PATH` change, external module file edit, `--repo_env` change, output modified and
  deleted, `clean`.
- Interrupt (SIGINT 6 s into a 20 s action) then rebuild, oracle and Blaze: same exit code (8),
  messages and spawn set; counts differ (in-flight set, see above).
- Warnings and `print()` of a .bzl: same on the first build and on the no-op rebuild as Bazel.
- `scripts/incremental/stress random` (incremental server against `clean` + build on a second
  server with `BLAZE_INCREMENTAL=none`, same binary, same tree): seed 11, 320 rounds, 0 exit or
  output mismatches, 6 message differences, all `WARNING: Target pattern parsing failed.` missing
  on the incremental side for a second build of the same broken package — Bazel behaves the same
  (checked with the oracle); the comparison now excludes that line. Seed 23 (final binary): 320
  rounds, 0 mismatches. Edits per kind (seed 11): same-size rewrites 53, two rewrites in one
  millisecond 15, appends 20, touches 23, glob adds 21 / deletes 17, package adds 29 / deletes 11,
  BUILD rewrites, dep removals 4, .bzl edits 10, rule attribute edits 16, `platform_mappings` edits
  19, `--define` changes 13, `--action_env` changes 25, external module edits 12, syntax errors 16
  and repairs 16, outputs modified 12 / deleted 7; every round starts right after the previous
  builds (edits in the same second).
- `BLAZE_INCREMENTAL=none`: the second build reloads everything (12 packages, 30 targets), as
  before this task.
- `scripts/server/seq run --project brotli --reference default` refused to run: "less than 120 GB
  free on /home" (101 GB free). Root to run it after a target cleanup.
