# Profiler: instrumentation points still to add

The profiler infrastructure is ported (`crates/blaze_util/src/profiler`, start and stop in
`crates/blaze_runtime/src/profiler_init.rs`; `command.profile.gz` is written and listed in
`buildToolLogs`). Scopes exist in blaze_runtime, blaze_buildtool and
`blaze_sky_top/build_tool.rs`. The sites below are where Java has a `Profiler` scope and Blaze
has none yet (list from the implementer, 2026-11-19). Each is a one-line addition at the
existing `// Java:` marker; do them crate by crate with the crate's owner.

Known gaps in the file itself: Blaze writes two of Bazel's six phase markers. "Launch Blaze"
and "Initialize command" are dropped because `blaze_events::ui::nano_time_of` starts its clock
at first use, so the command's start time is slightly negative and the profiler drops events
with a negative start (fix: a large positive origin, as Java's `nanoTime`). "Load, analyze
dependencies and build artifacts" belongs in `analysis_phase_runner.rs`. The command thread is
named "main" where Bazel's is "Main Thread" on a gRPC thread.

Done on 2026-11-26 (task `extra-actions-3`), in the order of their frequency in the oracle's
profiles (a cold command: about 550 `bazel module processing` events; a build: the action
events): blaze_bzlmod and blaze_bzlmod_model (every BZLMOD scope below except the extension
evaluation ones, which live in blaze_repo_rules), blaze_sky_exec (all three files), blaze_exec
(resource manager, local spawn runner, the two sandbox files, symlink tree helper, file write
action, and `AbstractSpawnStrategy.getInputMapping`), blaze_sky_loading, blaze_skyframe,
blaze_rules and the two analysis phase markers. On a clean build of three targets the action
categories now count as Bazel's (action processing, complete action execution, action count,
discover inputs, Skyframe evaluator, critical path). Still to do: blaze_repo_rules
(REPOSITORY_FETCH, STARLARK_REPOSITORY_FN, the extension evaluation BZLMOD scopes), the worker
notes, blaze_remote, `outputService.finalizeAction` (Blaze does not know `finalizeActions`
without a finalizing output service), the EXECUTE / PREPARE markers of the non-merged
`ExecutionTool`, `trashPath.deleteTree`, and the restart-driven counts (Java's `discovery` scope
repeats with every Skyframe restart of the resolution function, 13 times on that build; Blaze
computes once).

| Crate | File | Java sites |
| --- | --- | --- |
| blaze_sky_exec | `skyframe_action_executor.rs` | SkyframeActionExecutor.java:1103 (ACTION), 1129, 1257, 1284, 1341, 1423, 1634, 781 (ACTION_CHECK) |
| blaze_sky_exec | `action_execution_function.rs` | ActionExecutionFunction.java:736 DISCOVER_INPUTS |
| blaze_sky_exec | `recursive_filesystem_traversal_function.rs` | :183 FILESYSTEM_TRAVERSAL |
| blaze_exec | `actions/resource_manager.rs` | ResourceManager.java:98, 117 "Resources acquired" (feeds the "action count (local)" series), 381 ACTION_LOCK |
| blaze_exec | `sandbox/abstract_sandbox_spawn_runner.rs` | :102, 142, 151, 154, 162, 174 |
| blaze_exec | `sandbox/abstract_containerizing_sandboxed_spawn.rs` | :121, 131, 137, 140 |
| blaze_exec | `exec/local/local_spawn_runner.rs` | :135 LOCAL_EXECUTION, 354, 405 LOCAL_PROCESS_TIME |
| blaze_exec | `exec/symlink_tree_helper.rs` | :131, 160 |
| blaze_exec | `analysis/actions/abstract_file_write_action.rs` | :64 |
| blaze_exec | `worker/worker_spawn_runner.rs`, `worker/singleplex_worker.rs` | the WORKER_* notes |
| blaze_sky_loading | `package_function.rs` | PackageFunction.java:1084 CREATE_PACKAGE |
| blaze_repo_rules | `repository_fetch_function.rs` | RepositoryFetchFunction.java:156 REPOSITORY_FETCH, 676 STARLARK_REPOSITORY_FN |
| blaze_bzlmod | `skyframe/module_file_function.rs`, `module_file/mod.rs` | ModuleFileFunction.java:194, 659; BazelModuleResolutionFunction :135-195; BazelDepGraphFunction :70; RepoSpecFunction :58; YankedVersionsFunction :49; SingleExtensionEvalFunction :195; RegularRunnableExtension :295 |
| blaze_bzlmod_model | `index_registry.rs` | IndexRegistry.java:233 |
| blaze_skyframe | `evaluator.rs` | AbstractInMemoryMemoizingEvaluator.java:156; ParallelEvaluator.java:692 SKYFRAME_EVAL; AbstractParallelEvaluator.java:466 SKYFUNCTION |
| blaze_rules | `cpp/cpp_compile_action.rs` | CppCompileAction.java:1248 ACTION_UPDATE |
| blaze_sky_top | `analysis_phase_runner.rs` | AnalysisAndExecutionPhaseRunner.java:117; AnalysisPhaseRunner.java:140 (phase markers) |
| blaze_remote | `remote_spawn_runner.rs`, `remote_execution_service*.rs`, `grpc_remote_executor.rs`, `remote_execution_cache.rs`, `merkle_tree_computer.rs`, `abstract_action_input_prefetcher.rs`, `remote_repository_remote_executor.rs` | about 20 `PORT: skipped, profiling` notes |

Not ported anywhere yet: IncrementalArtifactConflictFinder (CONFLICT_CHECK), PackageFactory
STARLARK_PARSER, the Starlark user and builtin function events (interpreter hooks), the
Skyframe-counts collector (loud), `scripts/profile/compare` (not written), Bazel's own shell
tests that read the profile (not run).
