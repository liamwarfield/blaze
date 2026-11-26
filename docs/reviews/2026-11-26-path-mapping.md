# Review: path-mapping (execution under `strip`, analysis-failure testing, ValidateTarget, coverage follow-ups)

Date 2026-11-26. Reviewer: Fable 5.2, read-only. Commit under review: **ce81d5a** (checkpoint 39).
First round.

Verdict: **PASS**, bound to ce81d5a. No S1. Three S2 (P1, P2, P3), none blocking. Finding R1 of
`2026-11-26-java-config-round2.md` is closed.

No Blaze or Bazel process was started (`root-suite-5f68157` was active throughout), no Cargo.
Everything is from reading the Rust against the pinned Java. One small Java program was run against
Guava 33.5.0 (the jar in the repository cache) to settle the iteration order of
`ImmutableMap.Builder.buildKeepingLast` (P2).

## Scope

Read, Rust: `blaze_actions/src/analysis/actions/path_mappers.rs` and `stripping_path_mapper.rs`
(all), the six `path_mappers::create` execution sites (`blaze_exec/.../spawn_action.rs:91`,
`parameter_file_write_action.rs:333`, `cpp_compile_action.rs:1817` and `:2151`,
`java_compile_action.rs:492`, `java_header_compile_action.rs:324`), `KeyMetadataAdapter
.get_input_digest` (both copies), `blaze_exec/src/actions/path_mapper.rs`, `spawn_input_expander.rs`
`map_for_runfiles`, `sandbox_helpers.rs get_outputs`, `remote_path_resolver.rs create_mapped`,
`remote_spawn_cache.rs` lookup with the in-flight map, `remote_execution_service_download.rs`
`LocalExecution` and `wait_for_and_reuse_outputs`, `spawn_result.rs
delegate_with_runner_name_and_metrics`, `java_compile_action.rs create_full_output_deps` and the
fallback path, `config_util.rs modify_execution_info` and `modified_execution_info`,
`test/analysis_failure_info.rs` (all), the four errored paths and their callers in
`configured_target_factory.rs`, `rules_core/validate_target.rs`, `build_view.rs
create_top_level_aspect_class`, `build_tool.rs get_aspects`, `aspect_function.rs
native_aspect_factory`, `instrumented_files_info.rs` (accessors, `add_from_dependency`, `build`),
`test_action_builder.rs:440-460`, `remote_output_checker.rs coverage_artifacts_known`,
`commands/build.rs:2600-2650`.

Java: `PathMappers.java`, `StrippingPathMapper.java`, `RemotePathResolver.createMapped`,
`RemoteSpawnCache.lookup`, `RemoteExecutionService.waitForAndReuseOutputs`,
`JavaCompileAction` (spawns, fallback, `createFullOutputDeps`), `CppCompileAction
.getEffectiveEnvironment` and `createSpawn`, `ExecutionInfoModifier`, `BuildConfigurationValue
.modifiedExecutionInfo`, `AnalysisFailure*.java`, `ConfiguredTargetFactory` (the errored paths),
`ValidateTarget.java`, `BuildRequest.getAspects`, `InstrumentedFilesInfo/Collector`,
`RemoteOutputChecker.coverageArtifactsKnown`.

Not read: the Merkle tree computer beyond its two `getOutputPath` call sites,
`custom_command_line.rs` and `starlark_custom_command_line.rs` (round 2 of java-config read their
mapper lines), the worker files hash, the `get_input_metadata` implementations behind
`KeyMetadataAdapter` (see open item 1), the tests, the non-merged coverage path beyond its entry
(`non_merged_coverage_artifacts`), `OutputGroupInfo.determineOutputGroups` for the ASPECT mode,
`StarlarkAction`'s shadowed-action changes, the dynamic, docker and cgroups files of the diff
(other reviews).

## Confirmed

- **R1 closed.** All six sites pass `Some(&KeyMetadataAdapter(ctx.get_input_metadata_provider()))`;
  `have_same_digest` (`stripping_path_mapper.rs:360-373`) is StrippingPathMapper.java:365-385
  (null metadata, null digest or IOException answer false). `create_for_execution` is gone.
- When mapping is declined: `get_effective_output_paths_mode` (`path_mappers.rs:127-141`) is
  PathMappers.java:158-172 (`local`, `no-sandbox` with `no-remote`, the mnemonic list of 16,
  `supports-path-mapping`); a collision with different digests gives the no-op mapper, as Java.
- What is mapped: arguments and param-file contents and the param-file path (`command_lines.rs:184`),
  sandbox inputs (runfiles through `map_for_runfiles`) and outputs, worker file keys, Merkle tree
  and output paths through the mapped resolver (its inverse map and both checks as Java).
  An unmapped spawn keeps `bazel-out/<config>/` in its command, input root and output paths, so it
  cannot share an action key with a mapped one (`bazel-out/cfg/`) unless Java would as well.
- Deduplication: `create_if_deduplicatable` only for a mapped spawn; the `merge` runs under the
  map's lock; `register/unregister` as Java; "deduplicated" runner name and the new metrics.
  `wait_for_and_reuse_outputs` follows the Java line by line, including its overwrite of
  `inMemoryOutputContent` per iteration.
- `.jdeps` rewriting: first original path wins on a collision, unmapped transitive entries pass,
  the rewrite after a non-Turbine fallback is kept.
- `modify_execution_info`: `shift_remove` and `insert` on the caller's ordered map are
  `LinkedHashMap.remove/put`; `modified_execution_info` sorts by key as `ImmutableSortedMap`.
- The four errored paths, `dep_analysis_failures`, the output-file shortcut and the
  `AnalysisFailurePropagationException` text match ConfiguredTargetFactory.java:300-312, 547-620,
  762-815.
- `ValidateTarget` and its registration match rules/core; `get_aspects` matches
  BuildRequest.java:420-427; an unknown native aspect gives "Aspect '%s' is unknown".

## Findings

### S1

None.

### S2

**P1. A mapped C++ compile spawn (and a Starlark action that shadows one) gets an unmapped
environment.**

- Rust: `cpp_compile_action.rs:1902-1906` builds the spawn environment with
  `self.get_effective_environment(client_env)`, which expands the toolchain's `env_set`s with
  `CC_NOOP` (`:903-925`). The comment there ("path mapping is OFF, PathMappers.create returns NOOP
  for every supported mode") was true before this checkpoint and is false now: `execute` creates
  the real mapper (`:2151`). The trait method `Action::get_effective_environment`
  (`blaze_actions/src/actions/action.rs:393`) has no mapper parameter, so
  `EnhancedStarlarkAction` cannot pass one to its shadowed action either.
- Java: CppCompileAction.java:1776 `getEffectiveEnvironment(clientEnv, pathMapper)` and :1022
  `compileCommandLine.getEnvironment(pathMapper)`; StarlarkAction.java:490.
- Input: `--experimental_output_paths=strip --modify_execution_info=CppCompile=+supports-path-mapping`
  with a C++ toolchain whose `env_set` for a compile action expands a path variable (for example
  `env_entry("X", "%{output_file}")`). Bazel sets `X=bazel-out/cfg/...`, Blaze
  `X=bazel-out/k8-fastbuild/...`; the remote action key then differs from Bazel's and between
  configurations.
- Masked: the default Linux toolchain has no such `env_set`.
- Fix expected: give the environment the spawn's mapper (a mapper parameter on the trait method,
  as Java's signature), and delete the stale comment.

**P2. `coverage_environment`: a repeated key moves to its last position; Java keeps its first.**

- Rust: `instrumented_files_info.rs:337-341` does `shift_remove(k)` then `insert`, with the comment
  "buildKeepingLast: a repeated key takes its last position".
- Java: InstrumentedFilesCollector.java:281 and :319 (`putAll`, `buildKeepingLast()`). Guava keeps
  the last value at the position of the first occurrence. Run against guava-33.5.0-jre:
  `put(A,1) put(B,2) put(A,3) put(C,4)` prints `{A=3, B=2, C=4}`; Blaze's code gives
  `{B=2, A=3, C=4}`.
- Input: a target with dependencies `[cc, java, cc]` whose providers carry different key sets with
  an overlap. The value differs only in order.
- Masked: `TestActionBuilder` copies the map into a sorted map (TestActionBuilder.java:284,
  Rust `BTreeMap`), so the test action is not affected; the order is visible only through
  `InstrumentedFilesInfo.coverage_environment` in allow-listed `.bzl` files.
- Fix expected: plain `insert` (an `IndexMap` keeps the position and replaces the value).

**P3. `repr` of `AnalysisFailureInfo` is a guessed value.**

- Rust: `analysis_failure_info.rs:131-136` prints `AnalysisFailureInfo()`, with the comment
  "NativeInfo/StructImpl.repr with no struct fields".
- Java: `AnalysisFailureInfo implements Info, AnalysisFailureInfoApi` (AnalysisFailureInfo.java:28);
  it is not a `NativeInfo`, so `repr` is `StarlarkValue`'s default (StarlarkValue.java:50):
  `<unknown object com.google.devtools.build.lib.analysis.test.AnalysisFailureInfo>`.
- Input: `print(target[AnalysisFailureInfo])` or `str()` of it in an analysis test.
- The same comment and form are in `instrumented_files_info.rs:150-155`
  (`InstrumentedFilesInfo()`); that class is a `NativeInfo`, whose `repr` prints `struct(...)` of
  its legacy fields (NativeInfo.java:124-137), so `struct()`. Not checked further (whether another
  layer overrides the `Display` form).

### S3

- `delegate_spawn_result_with_in_memory_output` (`remote_execution_service_download.rs:1779`) is
  `unported!`. A locally executed spawn has no in-memory output today, so it is not reached.
- `StrippingPathMapper.mapCustomStarlarkArgs` is `unported!` for the Android Starlark mnemonics.
- The non-merged coverage path builds the coverage artifacts after the tests instead of in the
  targets' evaluation (declared `PORT` note in `commands/build.rs`).

### S4

- `stripping_path_mapper.rs:285` and `:377` cite line 170 for `StringStripper.strip` and `strip`
  (309 and 387 at the pin).

## Open items (not blocking)

1. `is_path_strippable` asks the provider for the digest of every colliding pair, and the list
   holds the action's outputs. Java's provider returns null for an artifact it does not know. I
   did not read the Rust implementations behind `get_input_metadata`; if one of them stops on an
   unknown artifact instead of returning `None`, an output that collides with an input would stop
   the build where Java declines mapping. Worth one probe when the suite ends.
2. Nothing here was probed against the oracle in this round. The suite family `path-mapping` (14
   functions) is the evidence to read once the measurement run ends.

## Verdict

**PASS**, bound to ce81d5a. P1, P2 and P3 go to the implementer as ordinary items.
