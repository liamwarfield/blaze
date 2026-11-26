# Response: Java rules, configuration cluster, analysis test harness (2026-11-25)

Fixer: `java-fix-1` (implementer, Opus 5.5). Answers `docs/reviews/2026-11-25-java-config.md`
(verdict STOP at `e8b79c0`). Tree: working tree on top of `3a006d7` (root's checkpoint `77eacad`
took in part of this work as in-progress changes). Binary for the probes: `target/debug/blaze`
of that tree, copied to `~/.cache/blaze/work/java-fix-1/blaze`. Its sha256 prefix was
`14891b393c3f1781` for the aquery and build probes and `3e6f8251baf5ad88` for the stamp probe;
the only change between the two is the `BuildInfoFileWriteAction` execution port.

## 1. Findings

### S1-1 (blocking): fixed and checked on both binaries

- `crates/blaze_rules/src/java/java_header_compile_action.rs` `compute_key` now passes
  `NestedSetBuilder::empty_set(Order::STABLE_ORDER)` to `path_mappers::add_to_fingerprint`, as
  `SpawnAction.computeKey` does (`SpawnAction.java:410-416`). The module comment and the
  `computeKey` marker (`:396`) are corrected.
- While checking it, I found and fixed a second deviation of the same kind. The header action's
  `get_arguments` delegated to the inner `SpawnAction`, so `PathMappers.create(this, ...)` read
  the inner action's execution info and additional artifacts, not this class's overrides. It now
  calls `path_mappers::create(self, ...)` (inherited `SpawnAction.getArguments`, `:207`), and
  `get_starlark_argv` goes through it.
- The probe needed `--experimental_output_paths=strip` to get past `unported!`, so
  `StrippingPathMapper` is ported (section 3).
- Probe `aq-strip`: `aquery --output=jsonproto --experimental_output_paths=strip` over the
  Turbine, Javac and JavacTurbine actions of `deps(//:app) + deps(//x:uses_other)` (98 actions).
  All 10 Turbine actions have the same `actionKey` and `arguments` as the oracle (stripped
  `bazel-out/cfg/...` paths), and so do all Javac actions. The only differing keys are the 12
  `SourceSymlinkManifest`/`SymlinkTree` actions: the known runfiles-manifest key gap allowed in
  `scripts/oracle/allow.json`, not Java.

### S2-1: ported

`crates/blaze_build_config/src/function_transition_util.rs`: a Starlark dict for a native option
whose Java type is `Map` now follows `ImmutableMap.copyOf` followed by
`def.getType().isInstance` (`FunctionTransitionUtil.java:676-681, 724-725`). The map is stored in
the field (`map_option_value`). Java stores any entry classes there by heap pollution, which a
typed Rust field cannot hold, so the conversion goes as far as the field's type allows:

- `bytecode_optimizers` (`Map<String, Label>`): `String=Label` entries are stored.
- `test_timeout` (`Map<TestTimeout, Duration>`): the empty dict is stored.
- Any other entry classes call `unported!` with that reason.

The "changed" test compares a `LabelMap` order-insensitively, as `Map.equals` does
(`option_values_java_equal`). The wrong sentence in `starlark_transition.rs` is replaced.

Oracle evidence (probe `cq-tr`, `cquery --output=label 'deps(//tr:all) + deps(//tr_bo:all)'`):

- A transition returning
  `{"//command_line_option:bytecode_optimizers": {"Proguard": Label("//tr:pg")}, "//command_line_option:copt": [...]}`
  gives the same leaf configuration hash on both sides. Before the fix Blaze failed with
  "Invalid value type".
- On both sides, a dict whose values are all dicts is read as a split transition, so the probe
  needs one non-dict output.
- `//command_line_option:experimental_bytecode_optimizers` is rejected at load time with
  "Cannot transition on --experimental_* or --incompatible_* options", the same text on both
  sides (`cq-tr-old`).
- `test_timeout` on a non-test rule gives "transition outputs [//command_line_option:test_timeout]
  do not correspond to valid settings" on both sides (`cq-tr-tt`).

### S2-2: fixed

`crates/blaze_sky_top/src/testing/build_view_test_case.rs`: `get_generating_action_for_output`
now uses ported `findArtifactNamed` and `artifactNamed`: `name.equals(artifact.prettyPrint())`,
and a missing artifact panics with Java's `NoSuchElementException` text. The five call sites
(`starlark_subrule_test.rs` ×2, `template_variable_info_test.rs`,
`aspect_auto_exec_groups_test.rs` ×2) pass the same names as their Java tests. All of them still
pass, so none changed result.

### S2-3: fixed

The harness's `update*` functions now return `UpdateError`, which has
`ViewCreationFailed | InvalidConfiguration | Interrupted | TargetParsing(TargetParsingException)`.
The `TargetParsingException` of `loadTargetPatternsWithFilters` is no longer reported as
`ViewCreationFailed`.

- Callers moved to `UpdateError`: `visibility_test`, `aspect_test`,
  `registered_execution_platforms_function_test`, `starlark_aspect_attr_aspects_function_test`,
  `starlark_defined_aspects_test_1`, `starlark_aspects_propagation_predicate_test` (harness
  calls), `starlark_aspects_toolchain_propagation_test`.
- The last of these had a local copy of the same mapping in its own update helper, which is
  fixed too.
- `starlark_defined_aspects_test_2::update_expecting_failure` accepted any `Err`; it now checks
  the variant against the Java exception class the test names.

Tests that changed result: **two**, `starlark_defined_aspects_test_1::aspects_non_exported` and
`provider_non_exported`.

- With the fix they first failed with `TargetParsing`: the shared `expect_failure` helper caught
  only `ViewCreationFailed`.
- The Java tests catch `ViewCreationFailedException | TargetParsingException`
  (`StarlarkDefinedAspectsTest.java:693, 770`), so the port was weaker than Java there. Those two
  sites now use `expect_failure_or_target_parsing`, which catches both, and pass.
- Every other test that used `update` has the same result as before.
- `blaze_sky_top --features testing --lib`: 898 passed, 0 failed, 164 ignored.

### S4: fixed

All `// Java:` markers in `crates/blaze_rules/src/java/*.rs` were re-pointed to the
declaration's line at the pin (356 markers, e.g. `computeKey` 221, `getReducedClasspath` 263,
`GUID` 99, `createCompileAction` 140).

- For overloaded members I chose the declaration nearest the old line and checked each by hand;
  three were corrected by hand: `mergeWithoutJavaOutputs(a, b)` 218,
  `StarlarkInfoWrapper` 26, and `StarlarkInfo.getValue` for the one-argument form.
- `scripts/java-ref-lines check` now reports one marker, `CompilationType.mnemonic` →
  `JavaCompileAction.java:111`. It is a field, which the checker does not recognise.

## 2. Open items of section 4

Not changed, except as listed here:

- Item 2 (`get_output_group`) and item 6 (`from_utf8_lossy` note): unchanged.
- Item 1 (fail-fast handler): unchanged.
- The harness gained `get_generating_action_for_label`, `get_bin_artifact` /
  `getDerivedArtifact` (including the artifact-factory fallback) and `artifacts_to_strings`
  (`AnalysisTestUtil.artifactsToStrings` with `computeRootPaths`).

## 3. Changes outside the reviewed area

- `blaze_actions`:
  - New `analysis/actions/stripping_path_mapper.rs`: `StrippingPathMapper` with `tryCreate`,
    `isPathStrippable`, `StringStripper`, `getMapFn` (`structuredArgStripper`) and `mapRoot`.
    `haveSameDigest` with a provider, and `mapCustomStarlarkArgs` for the Android mnemonics,
    are `unported!`.
  - `path_mappers::create` returns the stripping mapper.
  - New `path_mappers::create_for_execution`: the same mapper, but `unported!("Spawn.getPathMapper")`
    when it is not the no-op. Running a path-mapped spawn (staging mapped inputs and outputs,
    `.jdeps` rewriting) is not ported, and the spawns are still built with
    `SpawnPathMapper::NOOP`, so execution under `strip` now stops loudly at this call instead of
    at `tryCreate`. It is used at the six execution sites (blaze_exec `spawn_action.rs`, header
    action, `JavaCompileAction`, `CppCompileAction` ×2, `ParameterFileWriteAction`).
  - `SpawnAction::get_arguments` uses `create`.
  - `custom_command_line.rs` applies `structuredArgStripper` to `String` values (as
    `mapHeuristically`).
  - `PathMapper.storeIn` / `loadFrom` around `map_each` callbacks: a scoped thread-local in
    `starlark_file.rs`; `apply_map_each` no longer stops.
- `BuildInfoFileWriteAction.execute` (root's `build-info` acceptance cases):
  - The action keeps the creating thread's `StarlarkSemantics`.
  - New `call_translation_callback` in blaze_actions; blaze_exec's `execute` does the parsing,
    the three failures with Java's texts and codes, and the template expansion.
  - The status values are handed over in Java's HashMap iteration order: `parseValues`' map,
    then `putAll` into a pre-sized map (`parse_values_in_insertion_order` +
    `java_hash_map_iteration_order`).
  - Probe `b-stamp` (`build --stamp --embed_label=toto //:hello_deploy.jar`): exit 0 on both
    sides. `build-data.properties` has the same keys in the same order (`build.target`,
    `main.class`, `build.label=toto`, `build.time`, `build.timestamp`,
    `build.timestamp.as.int`), and all values agree except the time values.

## 4. Probes of section 5 (and the Part B list of the task)

Workspace: `~/.cache/blaze/work/java-fix-1/ws`, a copy of `scripts/java/probe` plus:

- `other/`: a second module, through `local_path_override`.
- `x/`: external-repository dependency and resources, strict deps, resources outside the Maven
  layout and with `resource_strip_prefix`, a `generates_api` plugin named
  `dagger.internal.codegen.ComponentProcessor`.
- `toolchain/`: `package_configuration` and `jspecify_*` toolchains.
- `tr/`, `tr_bo/`, `tr_tt/`, `tr_bo_old/`: transitions.
- `tree/`: a C++ tree-artifact compile.
- `env/`: `--target_environment`.
- `pm/`: platform mapping files with each syntax error.

Each side has its own copy and output root, `--jobs=4`, and the shared repository cache. The
command list is `probes.txt` and the outputs are `out-{oracle,blaze}/`.

How each kind was compared:

- **aquery**: `jsonproto` through `scripts/oracle/surfaces.aquery_summary`, which keys actions
  by primary output and compares argv, inputs, outputs, environment, execution info, action
  key, configuration and owner.
- **textproto**: diffed raw.
- **stderr**: ERROR/WARNING lines, and the "INFO: From" action output as a set (completion
  order varies).
- **builds**: sha256 of every jar, `.jdeps` and launcher under `bazel-bin`, plus the zip entry
  lists.

| Probe | Result |
| --- | --- |
| `aq-main` (181 actions: app, `//x:all`, uses_api, fb_a, fb_e, greeter_test, hello_imported) | all fields equal except 20 runfiles-manifest keys (known gap) |
| classpath modes `off`, `javabuilder`, `bazel` (103 actions each) | equal except 12 runfiles-manifest keys |
| `--strict_java_deps=off` / `warn` | equal except the manifest keys |
| `--collect_code_coverage --instrumentation_filter=//...` | equal except 18 manifest keys |
| `--incompatible_auto_exec_groups` | equal except the manifest keys |
| `package_configuration` toolchain, `jspecify` toolchain (`--java_language_version=11`) | equal except 8 manifest keys |
| `--experimental_output_paths=strip` (S1-1) | equal except 12 manifest keys (all Turbine and Javac keys equal) |
| tree-artifact compile (`aq-tree`, 8 actions incl. keys) | equal |
| textproto output | differs only in **action order** (e.g. Bazel prints `//tree:srcs` before `//tree:lib`, Blaze the reverse) and the ids that follow from it: aquery dump order, outside the Java rules |
| `cquery` transitions (string_list, repeatable string, string_set from list and from set, label from string and from `Label`, `copt`, Map-typed option) | equal (30 lines, configuration hashes included) |
| the same flags on the command line | equal |
| platform mapping errors (9 syntax files, missing file, directory) | ERROR lines and exit code 2 equal |
| `--target_environment` with two targets | equal (after `--nobuild`) |
| `build` of everything (incl. deploy jars, `//x:other_bin`, `//tree:lib`) | exit 0; 134 jar / `.jdeps` / launcher files byte-identical; C++ tree outputs (13 files) identical |
| strict deps error / warn / off | exit 1/0/0 on both; JavaBuilder messages equal |
| `--experimental_java_classpath=bazel` (with and without in-memory jdeps), reduced-classpath fallback targets | exit 0; jars and `.jdeps` of fb_a and fb_e identical |
| bad tree artifact (non-source file) | exit 36, same error |
| persistent workers (no `--spawn_strategy`) | 16 worker spawns each; jars identical |
| `test //:greeter_test`, `run //x:other_bin` | equal (test time differs) |
| `build --experimental_output_paths=strip //:app` | oracle 0, Blaze 37: `unported!("Spawn.getPathMapper")`, as section 3 describes |
| `build --stamp` deploy jar | equal (section 3) |

The only stderr difference in every probe is Bazel's "discarding analysis cache" warning. Blaze's
server does not keep the analysis graph between commands (decision D2).

## 5. Java-rule JUnit tests

The 59 stubs moved from `blaze_rules/src/java/tests.rs` (which now points to the new files) to
the harness crate. `blaze_rules` cannot depend on `blaze_sky_top`, which depends on it.

| Java class | Rust file | run, pass | ignored |
| --- | --- | --- | --- |
| `JavaCompileActionBuilderTest` (7) | `blaze_sky_top/src/tests/java_compile_action_builder_test.rs` | 6 | 1 |
| `JavaRuntimeTest` (14) | `java_runtime_test.rs` | 14 | 0 |
| `JavaStarlarkApiTest` (38) | `java_starlark_api_test.rs` | 10 | 28 |

Why the rest are ignored:

- `testExtraArgsPropagated`: `create_compilation_action` with non-empty `extra_args` stays
  `unported!`. Starlark `Args` are built only after the rule's module is frozen, and this action
  is created immediately.
- 27 tests of `JavaStarlarkApiTest` need the test-side JavaInfo wrappers
  (`src/test/java/.../rules/java/JavaInfo.java`, `JavaCompilationArgsProvider`,
  `JavaRuleOutputJarsProvider`, ...). Seven of them also call `useConfiguration`, so they need
  the parameterized platforms (`JavaTestUtil.setupPlatform` with
  `MockCcSupport.setupCcToolchainConfigForCpu`).
- `testNativeJavaInfoPrintableType_isJavaInfo` needs `JavaInfo.EMPTY_JAVA_INFO_FOR_TESTING`.

The ten ported `JavaStarlarkApiTest` methods do not call `useConfiguration`, which is the only
reader of the platform parameter, so each runs once.
