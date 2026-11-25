# Review: Java rules, configuration cluster, analysis test harness (2026-11-25)

Reviewer role: independent reviewer (Fable 5.2), read-only. Area name `java-config`.

**Commit read: `e8b79c0`** (code of `cc6c29a`), diff from the accepted baseline `ef66f55`. Read
from the working tree, whose uncommitted changes are under `scripts/` only. No Cargo was run, no
file but this one was written. **No Bazel or Blaze process was started**: the unit
`root-suite-e8b79c0` was `active` for the whole review, so every finding below comes from reading
the Rust against the pinned Java, and none was confirmed by a run. Probes that would confirm
them are listed in section 5.

**Verdict: STOP.** Blocking: **S1-1** (one wrong argument in the action key of the header
compilation action). Everything else that was read is a literal, careful port; the three S2
items need a fix or a recorded ruling but do not block.

## 1. Scope

### Read in full, both sides

| Rust | Java |
| --- | --- |
| `blaze_rules/src/java/java_compile_action.rs` | `rules/java/JavaCompileAction.java` |
| `java_compile_action_builder.rs` | `JavaCompileActionBuilder.java` |
| `java_header_compile_action.rs` | `JavaHeaderCompileAction.java`, `analysis/actions/SpawnAction.java` (execute, getSpawn, computeKey, ActionSpawn, setJarExecutable) |
| `java_compilation_helper.rs` | `JavaCompilationHelper.java` |
| `java_starlark_common.rs` | `JavaStarlarkCommon.java`, parameter lists of `starlarkbuildapi/java/JavaCommonApi.java` for the two compile methods |
| `java_helper.rs`, `java_toolchain_tool.rs`, `java_plugin_info.rs`, `boot_class_path_info.rs`, `java_compile_action_context.rs`, `resource_jar_action_builder.rs`, `provider_util.rs` | the classes of the same name |
| `java_target_attributes.rs` (builder) | `JavaTargetAttributes.java` (builder) |
| `java_toolchain_provider.rs` (wrap, from, provider keys) | `JavaToolchainProvider.java` |
| `blaze_rules/src/cpp/cpp_compile_action_template.rs`, the call site in `cc_starlark_internal.rs:1133-1247` | `rules/cpp/CppCompileActionTemplate.java`, `CcStarlarkInternal.createCppCompileActionTemplate` |
| `blaze_build_config/src/skyframe.rs`: `PlatformMappingValue`, `PlatformMappingFunction.parse` and its readers, `BuildConfigurationKeyProducer` (inlined), `mergeFromPlatformMapping` | `skyframe/config/PlatformMappingFunction.java`, `PlatformMappingValue.java`, the exception classes, `analysis/producers/BuildConfigurationKeyProducer.java` |
| `blaze_build_config/src/parsed_flags.rs` (`mergeWith`), `blaze_sky_analysis/src/parsed_flags_function.rs` | `ParsedFlagsValue.java`, `ParsedFlagsFunction.java` |
| `blaze_sky_analysis/src/starlark_options_parser.rs` (`parseGivenArgs`, `parseArg` head, the HashMap order helper) | `runtime/StarlarkOptionsParser.java` |
| `blaze_build_config/src/feature_flag_value.rs` | `rules/config/FeatureFlagValue.java` |
| `function_transition_util.rs`: `apply_transition`, value conversions, the `FULL_HASH` exec distinguisher (diff and surrounding code) | `analysis/starlark/FunctionTransitionUtil.applyTransition`, `ExecutionTransitionFactory` (FULL_HASH branch) |
| diffs of `build_options.rs`, `build_configuration_key.rs`, `starlark_transition.rs` (both crates), `config/transitions.rs` | `BuildOptions.java` (checksum, hashCode, matches), `ConfigFeatureFlagTransitionFactory.java`, `ConfigFeatureFlagTaggedTrimmingTransitionFactory.java` |
| `constraints/top_level_constraint_semantics.rs` (`checkTargetEnvironmentRestrictions`) | `TopLevelConstraintSemantics.java` (same method, `compatibilityWithTargetEnvironment`) |
| `blaze_sky_top/src/testing/build_view_test_case.rs`: module documentation, fail-fast handler, event assertions, `getConfiguredTarget*`, `checkError*`, `checkWarning`, action accessors, `update*`, free helpers | `analysis/util/BuildViewTestCase.java` (the same helpers), `SkyframeExecutor.getConfiguredTargetAndDataForTesting`, `BuildViewForTesting.hasErrors` |

### Read as diff only (no line-by-line Java comparison)

Toolchain resolution (`single_toolchain_resolution.rs`, `platform_keys.rs`,
`toolchain_resolution_function.rs`): the change is the wiring of the debug printers; the
printers' texts (`debug_printers.rs`) were not compared with Java.

### Not read

- `blaze_rules/src/java`: `java_output.rs`, `java_runtime_info.rs`,
  `java_package_configuration_provider.rs`, `java_plugins_flag_alias.rs`, `java_util.rs`,
  `java_semantics.rs`, `starlark_info_wrapper.rs`, the getters of `java_toolchain_provider.rs`
  below `from`, the accessors of `java_target_attributes.rs`. Both `java_configuration.rs`
  fragment files beyond the constructor's shape.
- Rest of the `blaze_rules` diff: `config_feature_flag.rs`, `genrule.rs`, `filegroup.rs`,
  `python/py_builtins.rs`, the 284 changed lines of `cpp_compile_action.rs` (only what the
  template calls was followed), the other changes of `cc_starlark_internal.rs`.
- `blaze_sky_analysis`: `flag_set_function.rs` (974 lines, project files and `--scl_config`),
  `build_options_scope_function.rs` and `resetFlags`, `dependency_resolver.rs`,
  `producers/dependency_map_producer.rs`, `producers/errors.rs`, everything about aspects.
- `blaze_analysis`: everything outside `config/` and
  `constraints/top_level_constraint_semantics.rs` (aspects, rule context, Starlark API changes,
  `constraints/environment.rs`, `platform/*`).
- `OutputPathMnemonicComputer` and the ST-hash: unchanged since the baseline (no diff), so not
  re-read; the transition-directory-name fragment was checked only where the new code calls
  `update_affected_by_starlark_transition`.
- Harness: `analysis_mock.rs`, `mock_cc_support.rs`, `mock_java_python_support.rs`,
  `mock_tools_config.rs`, `fake_registry.rs`, `rule_context_for_testing.rs`,
  `configuration_test_case.rs`, `scratch.rs`, `test_deps.rs`. No ported test file was compared
  with its Java test except a sample of `platform_mapping_function_parser_test.rs` (22 tests
  on both sides, same `contains` expectations in the sample).

## 2. Findings

### S1

**S1-1. `JavaHeaderCompileAction`'s action key fingerprints artifacts Java does not.**

- Rust: `crates/blaze_rules/src/java/java_header_compile_action.rs:184-191`. The hand-copied
  `SpawnAction.computeKey` passes `&self.get_additional_artifacts_for_path_mapping()` to
  `path_mappers::add_to_fingerprint`.
- Java: `analysis/actions/SpawnAction.java:410-416`. `SpawnAction.computeKey` passes the literal
  `NestedSetBuilder.emptySet(Order.STABLE_ORDER)`. `JavaHeaderCompileAction` does not override
  `computeKey` (`rules/java/JavaHeaderCompileAction.java:76-166` has `getExecutionInfo`,
  `getAdditionalArtifactsForPathMapping`, `afterExecute`, `mayModifySpawnOutputsAfterExecution`
  only). The module comment at `java_header_compile_action.rs:10-13` says the inherited body
  reads the override; at the pin it does not. Blaze's own `SpawnAction::compute_key`
  (`blaze_actions/src/analysis/actions/spawn_action.rs:415-422`) has the empty set, as Java.
- Input: `aquery --experimental_output_paths=strip 'mnemonic("Turbine", //:app)'` on any
  `java_library` with a dependency. The effective mode is STRIP (the action carries
  `supports-path-mapping`), `headerCompilationDirectJars` is not empty, so
  `additionalArtifactsForPathMapping` is the full classpath
  (`JavaHeaderCompileAction.java:551-560`), and Blaze adds that nested set to the key where
  Bazel adds the empty-set digest. The `ActionKey` aquery prints differs, and so does the
  action cache key. Not masked by `unported!`: only `path_mappers::create` (execution) stops on
  STRIP (`path_mappers.rs:105`); the fingerprint path does not. Without
  `--experimental_output_paths=strip` the branch is not taken and the keys agree.
- Fix: pass `NestedSetBuilder::empty_set(Order::STABLE_ORDER)` there and correct the module
  comment. One line.

### S2

**S2-1. Transition output for a native option whose Java type is `Map` is rejected with a
Bazel-looking error instead of `unported!`.**

- Rust: `crates/blaze_build_config/src/function_transition_util.rs:864-892`
  (`to_option_value_if_typed` returns `None` for `JavaObject::Map` and `JavaObject::Set`, so
  `is_instance` is false and line 801-806 returns "Invalid value type for option '...'"), with
  the reason given at `starlark_transition.rs:268-271`: "no option of Bazel 9.3.2's fragments
  has such a type".
- Java: `analysis/starlark/FunctionTransitionUtil.java:676-679` (`ImmutableMap.copyOf`) and
  `:724-725` (`def.getType().isInstance(optionValue)`). The claim is wrong:
  `JavaOptions.bytecodeOptimizers` is `Map<String, Label>` (`rules/java/JavaOptions.java:247`)
  and `TestOptions.testTimeout` is `Map<TestTimeout, Duration>`
  (`analysis/test/TestConfiguration.java:123`).
- Input: a transition returning
  `{"//command_line_option:experimental_bytecode_optimizers": {"Proguard": Label("//tools:pg")}}`.
  Bazel sets the option; Blaze fails the analysis with a validation error Bazel never prints.
  Masked: no corpus project does this.
- Fix: carry the map into the option value (as the list case does), or call `unported!` in that
  branch; remove the wrong sentence either way.

**S2-2. Harness: `get_generating_action_for_output` is weaker than Java's.**

- Rust: `crates/blaze_sky_top/src/testing/build_view_test_case.rs:1490-1507`: finds the first
  file whose root-relative path `ends_with(output_name)` and returns `None` when there is none.
- Java: `BuildViewTestCase.java:1027-1037` and `:2013-2015`: `name.equals(artifact.prettyPrint())`,
  and a missing artifact throws `NoSuchElementException`.
- Effect: a test passes when Blaze puts the output under a different directory than the Java
  test names (`x/subrule_testing/foo.out` matches `subrule_testing/foo.out`). Five call sites
  (`starlark_subrule_test.rs:1833,1881`, `template_variable_info_test.rs:57`,
  `aspect_auto_exec_groups_test.rs:706,709`).
- Fix: compare with `pretty_print()` for equality and panic when nothing matches.

**S2-3. Harness: a loading-phase failure of `update` is reported as `ViewCreationFailed`.**

- Rust: `build_view_test_case.rs:1675-1680` maps `TargetPatternPhaseError::TargetParsing` to
  `BuildViewError::ViewCreationFailed` with an empty failure detail; `BuildViewError` has no
  variant for it (`build_view.rs:150-160`).
- Java: `AnalysisTestCase.update` lets `TargetParsingException` out; it is not a
  `ViewCreationFailedException`.
- Effect: a test ported from `assertThrows(ViewCreationFailedException.class, ...)` that matches
  only the variant passes when Blaze fails earlier, in target pattern evaluation, where the Java
  test would fail with the wrong exception. 47 test sites match on the variant; those that also
  compare the message are protected by it.
- Fix: a distinct variant (or error type) for `TargetParsingException` in the harness.

### S3 (honest gaps, the important ones)

- **No JUnit test of the Java rules runs.** `crates/blaze_rules/src/java/tests.rs`: 59 test
  functions, all `#[ignore = "needs BuildViewTestCase (analysis test harness with rules_java)"]`
  with empty bodies (`JavaCompileActionBuilderTest`, `JavaRuntimeTest`, ...). The harness they
  wait for exists now. The only checked evidence for this code is `scripts/java/compare` on the
  probe workspace, a tool that is itself under an auditor STOP (audit 4, F1).
- `java_starlark_common.rs:433-437`: `create_compilation_action` with a non-empty `extra_args`
  is `unported!` (and `JavaCompileActionBuilderTest.testExtraArgsPropagated` is one of the
  stubs).
- `java_compile_action.rs:825-830` and `path_mappers.rs:105`: path stripping
  (`--experimental_output_paths=strip`) stops at execution; `.jdeps` rewriting is not ported.
- Extra actions (`getExtraActionInfo`, `JavaCompileInfo`): `unported!`.
- `function_transition_util.rs:599-627`: a transition returning `None`, a dict, a float or an
  enum value for a Starlark flag panics with `unported!` before Java's "want label" / "want set"
  type checks run for some of these.
- `skyframe.rs:330-337`: an I/O error reading the platform mappings file is `unported!`.
- `starlark_transition.rs:973-983`: `--experimental_exec_config` outside `@_builtins` through
  the bundled loader is `unported!` (the loader with BZL_LOAD exists).

### S4

- Port markers carry wrong line numbers in several Java rule files, e.g.
  `java_compile_action.rs` cites `JavaCompileAction.java:134 computeKey` (221 at the pin),
  `:176 getReducedClasspath` (263), `:96 GUID` (99); `java_compilation_helper.rs` cites `:146
  createCompileAction` (140). The names are right; the numbers look like they come from another
  revision of the file.

## 3. What was checked and found right

Stated so the depth asked for is on record; each item was compared statement by statement.

- **Javac command line** (`buildParamFileContents`): flag order, the null-skipping `addExecPath`
  forms, `--javacopts ... --`, `--target_label` with the `@` escape for non-main repositories,
  `--strict_java_deps` + `--direct_dependencies` only when not OFF, `--experimental_fix_deps_tool`,
  `--post_processor jacoco`. `addObject(list)` and `addAll(list)` expand and fingerprint the
  same way (`CustomCommandLine.java:1313-1314, 1381-1385`).
- **Classpath modes**: the three places that force OFF (strict deps OFF, incompatible
  processors, no deps proto), `getFullClasspathLine` (`JAVABUILDER_REDUCED`, `--deps_artifacts`
  only when not empty), `getReducedSpawn` (`NONE` / `BAZEL_REDUCED` / `BAZEL_FALLBACK`, inputs
  without the `.jdeps` files, the only mandatory output), the fallback sequence in `execute`
  (delete outputs, reset output metadata, clear stdout and stderr, re-run, both result lists).
- **Javac action key**: UUID halves, `classpathMode.ordinal()` (enum order OFF, JAVABUILDER,
  BAZEL, BAZEL_NO_FALLBACK on both sides), executable line, flag line, extra lines, the
  classpath nested set, environment, execution info, the in-memory jdeps bit, path mapping.
- **Mandatory inputs and tools order** of both builders; outputs order (`allOutputs`, the header
  action's `outputs`); execution info built with `buildKeepingLast` semantics (first position,
  last value) and `modifiedExecutionInfo` applied with the right mnemonic at each site
  (`Javac`, `Turbine`, `JavacTurbine`).
- **Header compilation**: direct-classpath decision, `useHeaderCompilerDirect`, the classpath
  choice and `additionalArtifactsForPathMapping`, `--builtin_processors` as the intersection in
  processor order, `--processors`, `--processorpath` only for the non-direct tool,
  `-Aexperimental_turbine_hjar`, `afterExecute` (swallows the read error, inserts the
  dependencies only for the BAZEL modes).
- **`createCompileAction`**: gen-jar action first, JSpecify, turbine annotation processing
  (three derived outputs, the generated source jar appended), separate resource jar (`-class`),
  local optimization (`-pre-optimization.jar`), bootclasspath `auxiliary` prepended to classpath
  and direct jars, coverage artifact and its `LazyWritePathsFileAction` registered before the
  compile action.
- **`javac_opts`**: reversed flattening of the depset, shell tokenization with pass-through of
  untokenizable arguments, detokenization; the Java test `JavaUtilTest.testDetokenization` is
  ported with the Java expectation.
- **Plugin merge order**: `JavaPluginData.merge` (three NAIVE_LINK_ORDER sets, transitive adds
  in argument order), `mergeWithoutJavaOutputs`, the empty cases.
- **Tree-artifact compile template**: source/header classification and the error text, output
  name (`removeExtension` then one `getArtifactNameForCategory` per category), `.d` and `.dia`
  names, the LTO index name, `getOutputs` without the diagnostics tree (as Java), `getInputs`
  and `getMandatoryInputs` orders, the key computed for `CPP_COMPILE` with
  `OutputPathsMode.OFF`, variable overrides in `createAction`.
- **Platform mappings**: line reading and trimming, `platforms:` / `flags:` grammar, the six
  error texts, duplicate detection, the missing-file and directory messages, no flag aliases in
  mappings, Skyframe restarts (`needSkyframeDeps`); application order in `computeMapping`
  (terminal no-config, platform given, first matching flags entry, default platform) and in the
  key producer (platform flags from `PlatformValue` before the mapping file, exactly one
  `--platforms` value).
- **`ParsedFlagsFunction` / `mergeWith`**: alias rewriting, native/Starlark split, trimmed
  fragments ignored, scope info added before the values, flags equal to their default removed.
- **Starlark transitions**: label-typed and set-typed build settings (string to `Label`, list to
  `ImmutableSet`, the two "want ..." errors), the Java class of the stored value and its
  `toString()` (which feeds the checksum: `StarlarkList` and `StarlarkSet` print their repr,
  `ImmutableList` / `ImmutableSet` print `[a, b]`), `equals` asymmetries between those classes,
  native list options (comma join for single-valued, per-element conversion for
  `allowMultiple`, `None` rejected), "affected by Starlark transition" in legacy naming.
- **Feature flags**: `replaceFlagValues`, `trimFlagValues`, both transitions' `patch`, removal
  in the exec transition, the three `toString()` texts.
- **`BuildOptions`**: `StarlarkSet` and `FeatureFlagValue` in `java_to_string`, `starlark_repr`
  and `java_equals`; `matches`; `hashCode()` and the `exec-%X` suffix of `FULL_HASH`.
- **`--target_environment`**: iteration, the combined message of the wrapped exception,
  `putAll` of an empty collection. The order of targets in the error message is marked
  `java-hash-order`: Java's is an identity-hash order that changes from run to run, Blaze uses
  the input order.
- **Harness**: `getConfiguredTarget*` follows
  `SkyframeExecutor.getConfiguredTargetAndDataForTesting` (errors processed with keep-going and
  ignored, a missing value is `None`); `checkError`, `checkErrorForLabel`, `checkWarning`,
  `checkLoadingPhaseError` and the event assertions are equivalent to Java's (`toString()` for
  `assertContainsEvent`, `getMessage()` for the others).

## 4. Open items that do not block

1. **Fail-fast handler** (`build_view_test_case.rs:117-143`). Java fails the test inside the
   handler; the harness records the first error and panics at the next harness call that
   evaluates. Every such call I read ends with `check_fail_fast()`. An error reported after the
   last check (test code that uses `event_handler()` or the executor directly) is never seen.
   A check when the test case is dropped (skipped while already panicking) would close it.
2. `get_output_group` returns an empty set for a target without `OutputGroupInfo`
   (`build_view_test_case.rs:2025-2034`); Java's `OutputGroupInfo.get(target)` is null there and
   the test fails with a NullPointerException.
3. `BuildConfigurationKeyValueKey.producer_label` (`build_configuration_key.rs:61-95`) is a
   design deviation with a stated reason (the producer runs as a node). The condition that drops
   the label repeats `findBuildOptionsScopes`; if that Java condition changes, two nodes can
   differ where Java has one result. Worth a line in `docs/decisions.md`.
4. `java_hash_map_key_order` (`starlark_options_parser.rs:653-677`) reproduces `HashMap`
   iteration for the order of scope errors, without tree bins (eight colliding keys). Fine as
   marked.
5. `Verify.verify(targetPlatforms.isEmpty())` is `check_state!` in Rust (`skyframe.rs:445`):
   another exception class in the crash text for more than one `--platforms` value.
6. `Dependency::get_path` (`java_compile_action_context.rs:44-48`) uses `from_utf8_lossy`. It is
   what Java's proto getter does, and like Java a non-ASCII jar path then never equals the
   internal exec path string; say so in the `PORT:` note, since PORTING.md forbids the call on
   internal strings.

## 5. No Java target has been compared on a real project: risks and probes

`scripts/java/compare` runs the probe workspace `scripts/java/probe` (cquery, aquery with action
keys, build, jar hashes, test, run; variants lang17, tool21, noturbine, toolchain, turbineapt,
proto). Its last recordings are from 2026-11-19 and the tool's stderr comparison is under an
auditor STOP, so I count it as indication, not evidence. What it does not reach, with the code
that carries the risk:

| Risk | Code | Small probe (aquery `--output=jsonproto` both sides unless noted) |
| --- | --- | --- |
| Classpath mode in the key and in the classpath line | `get_full_classpath_line`, `compute_key` | probe workspace with `--experimental_java_classpath=bazel`, `=javabuilder`, `=off`; with `--strict_java_deps=off` and `=warn` |
| Reduced classpath at execution and the fallback | `get_reduced_classpath`, `execute` | `build --experimental_java_classpath=bazel` of a library that compiles only with a transitive jar (forces `requires_reduced_classpath_fallback`); compare exit, stderr, the `.jdeps` and jar hashes; once more with `--experimental_inmemory_jdeps_files` |
| S1-1 | header action key | `aquery --experimental_output_paths=strip 'mnemonic("Turbine|Javac", deps(//:app))'`, compare `actionKey` |
| `--target_label` escape | both builders | a `java_library` in a second module (`local_path_override`), aquery of `@other//:lib` |
| javac-based header compilation | `JavaCompileAction` TURBINE branch | a `java_plugin(generates_api = True)` used by a library; a processor named `dagger.internal.codegen.ComponentProcessor` for the hjar option |
| Coverage | `maybe_create_coverage_artifact` | `aquery --collect_code_coverage --instrumentation_filter=//...` of the test target |
| Resources | `ResourceJarActionBuilder`, `get_java_resource_path` | `resource_strip_prefix`, a resource outside the Maven layout, `resource_jars`, a resource in an external repository |
| Exec groups | `use_auto_exec_groups` branches | the probe with `--incompatible_auto_exec_groups` |
| Per-package data, JSpecify | `compute_per_package_data`, `jspecify_info` | a custom `java_toolchain` with `package_configuration` and with `jspecify_*` attributes |
| Javac under workers | execution info (`supports-workers`, ...) | the build step without `--spawn_strategy=linux-sandbox` now that persistent workers are ported |
| Tree-artifact compile | `CppCompileActionTemplate` | a rule that declares a directory of `.cc` and `.h` files, a `cc_library` with it in `srcs`; aquery (template action and key), build, names and hashes of the expanded `.o` and `.d` files; one file that is neither source nor header for the error text |
| Transition value classes in the configuration hash | `apply_transition`, `java_to_string` | transitions that set a `string_list`, a repeatable `string`, a `string_set` (from a list and from a set) and a `label` flag (from a string and from a `Label`); compare `cquery --output=build` configuration hashes and output directories; the same flags set on the command line to the same values |
| Platform mappings and platform flags | `skyframe.rs` | `scripts/config/probe/platform_mappings` exists; add a mapping file with each of the six syntax errors and compare stderr |
| `--target_environment` | constraint semantics | two top-level targets missing an environment: the message (target order is nondeterministic in Bazel, compare as a set) |

These are single commands on small workspaces; none needs a real project.

## 6. Verdict

**STOP, bound to `e8b79c0`.** Blocking: **S1-1**. S2-1, S2-2 and S2-3 should be fixed in the
same round or get a recorded ruling. The re-check needs the one changed call in
`java_header_compile_action.rs` and, once the suite run has ended, the `strip` aquery probe of
section 5 on both binaries.
