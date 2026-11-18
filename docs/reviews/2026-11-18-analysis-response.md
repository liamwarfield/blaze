# Response to the analysis review of 2026-11-18

Analysis lead, 2026-11-18. Covers docs/reviews/2026-11-18-analysis.md (12 S1, 18 S2), the
integrity audit items in my crates (S1-1, S1-2, S2-5) and the items other owners sent me.
The exec dispositions are in docs/reviews/2026-11-18-exec-response.md.

State described: HEAD `1fdd650` plus the uncommitted working tree of the analysis fix waves
(binary `~/.cache/blaze/int-bin/blaze-fix-6`, sha256 `257d0bcf7564`). Numbers are in "Evidence" at the end; nothing
below was scored from a focused run.

How the fixes were tested: unit tests ported from the Java tests where they can run without a
`BuildViewTestCase`, and oracle scenarios. `scripts/w4/probe` runs a scratch workspace's command
list on Bazel 9.3.2 and on Blaze and diffs the transcripts (exit codes, stdout, ERROR / WARNING /
DEBUG lines, build result lines, aquery actions with resolved ids); `scripts/w4/flags-probe` does
the same for 71 Starlark-flag command lines. Every `ORACLE.txt` was written by the oracle, never
by Blaze. Scenario names below are directories under `scripts/w4/scenarios/`.

## S1

| id | disposition | where | test |
| --- | --- | --- | --- |
| S1-1 Starlark flags dropped | FIXED | `blaze_runtime/src/starlark_options_parser.rs` (`StarlarkOptionsParser`, all of it), `dispatcher.rs` `parse_starlark_options`; `BuildOptions::of` (`labelize_starlark_options`, `convert_scopes_attributes`); `BUILD_SETTING_CONVERTERS` in `blaze_build_config/src/core_option_converters.rs`; flag scoping in `blaze_build_config/src/skyframe.rs` (`find_build_options_scopes`, `possibly_apply_scopes`, `apply_scopes`, `reset_flags`) and `blaze_sky_analysis/src/build_options_scope_function.rs` | `scripts/w4/flags-probe` 71/71 cases identical to the oracle (checksums of target and exec configurations, select() results, exit codes, error texts); `oracle_configurations_test::starlark_flags`; `starlark_options_parsing_test.rs`; workerd: `cquery deps(//src/workerd/util:entropy)` prints the oracle's 1,074 lines with target configuration `13e714b` and exec configuration `1d8533e` (was `6d8e5e8`) |
| S1-2 BuildLanguageOptions ignored | FIXED | `blaze_build_config/src/top_level.rs` `to_starlark_semantics` (all 63 keys), guard removed | `build_language_options_semantics_test.rs` |
| S1-3 Args / TemplateDict read late | FIXED | `starlark_action_factory.rs` `build_command_line`, `write`, `expand_template` snapshot through `blaze_actions::snapshot_args` / `snapshot_template_dict` | scenario `ctx-args-snapshot` |
| S1-4 incompatible top-level targets | FIXED | `blaze_analysis/src/constraints/top_level_constraint_semantics.rs` (new), `rule_context_constraint_semantics.rs`, `blaze_sky_analysis/src/incompatible_target_checker.rs` (`createEmptyTestParams` ported), `blaze_sky_top/src/build_view.rs` `update`; build command skips and reports them | scenarios `incompat-toplevel`, `incompat-config-setting`; 4 unit tests of the reason text |
| S1-5 `--output_file` | already fixed at HEAD (`blaze_query` `query_runtime_helper`) | - | scenario `query-output-file` |
| S1-6 CppCompile execution info order | DISPUTED | - | see "Disputed" |
| S1-7 unhashable native values | FIXED, with Java's semantics per class | `HeaderInfo`, `CppConfiguration`, every `fragment_value!`: identity; `AppleConfiguration`: by options (`AppleConfiguration.java:351`); `MapVariables`: value equality (`CcToolchainVariables.java:953`); `SingleVariables` stays unhashable as in Java | scenario `cc-hash` |
| S1-8 aquery rdeps filter | FIXED | `post_analysis_query_environment.rs` `filter_reverse_deps`, `is_instance_of_rule_configured_target`; also the `Targets were missing from graph` warning | scenario `query-rdeps` |
| S1-9 run_shell helper script order | FIXED | `starlark_action_factory.rs` `run_shell` pushes the helper `FileWriteAction` through the pending list | scenario `ctx-run-shell-order` |
| S1-10 print() in transitions | FIXED | `blaze_build_config/src/starlark_transition.rs` `evaluate_with_attrs`; errors are now reported at the implementation's `def` (`StarlarkFunction.getLocation`) | scenario `flags-transitions` |
| S1-11 int flag_values parser | FIXED | `config_setting.rs` `convert_build_setting_value` through the shared `starlark_int_parse` | `flags-transitions` steps 27-32, unit test |
| S1-12 validation aspect ignored | FIXED as a loud failure | `BuildRequest::get_aspects` appends `ValidateTarget`; `unported!("BuildView.createTopLevelAspectKeys")` fires | - (no test) |

## S2

| id | disposition | where / why |
| --- | --- | --- |
| S2-1 action conflicts | FIXED | `blaze_sky_top/src/{artifact_conflict_finder,action_lookup_conflict_finding_function}.rs`, `blaze_analysis/src/rule/action_conflict_exception.rs`, `SkyframeExecutor::filter_action_conflicts_for_configured_targets_and_aspects`, intra-target conflicts in `actions_util.rs` `verify_generating_action_keys`. Scenario `top-conflicts`. Which of two conflicting targets Bazel names is racy on the oracle itself (six runs: three each), so those lines are documented, not matched. |
| S2-2 Starlark list values hash | FIXED | `StarlarkOptionList { class: JavaCollectionClass, items }`; every producer keeps the Java class. `oracle_configurations_test::starlark_flags` (30+ oracle checksums). |
| S2-3 at-default for allow_multiple | FIXED | `validate_and_check_if_at_default`; the oracle showed more (defaults are added for outputs; a returned list equal to the old Java list keeps the old object), ported in `apply_transition`. |
| S2-4 error texts | FIXED | all four bullets (`convert_starlark_option` is now a port of `Type.convert`; list-of-dicts cast; `in call to run(), parameter` prologue checks; universe-scope prefix). |
| S2-5 cquery / aquery order | FIXED | Java's order is hash order over `ConfiguredTargetKey.hashCode()` in `ConcurrentHashMap`s sized by `Runtime.availableProcessors()`; ported in `post_analysis_query_environment.rs` (`KeyedNodeSet` over `JavaConcurrentHashMap`, the four `HashMap` replays). Ties follow Skyframe dep registration order, so `dependency_map_producer.rs` now requests configured-target deps in the Java Driver's lookup rounds. aquery ids need NestedSet identity: `rule/depsets.rs` memoizes depset -> NestedSet (frozen copies adopt their original's set). Residual: see "Deferred". |
| S2-6 mutable values | FIXED, one part DISPUTED | `ctx.attr`, `ctx.files`, `ctx.outputs`, split attrs, `ctx.var`, `tokenize`, cpp option lists, `cc_internal.freeze` are immutable. `ctx.features` / `disabled_features` are mutable on the oracle (see "Disputed"). Scenario `ctx-immutable`. |
| S2-7 Bazel-raised attribute errors | PARTLY FIXED | undeclared / unknown fragments, `ctx.outputs.x`, `Target` fields report Bazel's text (`get_attr_error_message`). `getattr(x, name, default)` still returns the default and `hasattr` stops with `unported!`: needs a fallible attribute hook in the vendored interpreter (request below). Scenario `ctx-errors` steps 2-3 show it. |
| S2-8 `ctx.toolchains["//x:type"]` | PARTLY FIXED | resolved against the implementation function's .bzl. A key converted inside a helper of another .bzl needs the evaluator in the index operator (request below); `ctx-toolchains-string` step 6. |
| S2-9 parameter type checks | FIXED | `ctx.runfiles`, `use_default_shell_env`; scenario `ctx-errors`. |
| S2-10 `resource_set` callback | DEFERRED, not loud | rules_cc passes `resource_set` to every C++ link, so `unported!` would stop all cc builds; the callback only affects local scheduling estimates. Needs a `ResourceSetOrBuilder` slot on `SpawnActionBuilder` (blaze_actions, request). |
| S2-11 lost cause | FIXED | `combine_messages`; 3 tests with oracle text. |
| S2-12 regex_match | FIXED | Java semantics for the subset rules_python uses (`(?U)`, `\w \d \s`, `.`, literals, greedy quantifiers), `unported!` for anything else; class tables checked against the embedded JDK. |
| S2-13 CcToolchainInfo by name | FIXED | provider keys. |
| S2-14 emulated addAttributes | PARTLY FIXED | `--proto:include_synthetic_attribute_hash` no longer panics (ignored, as on the oracle). The override hook belongs in blaze_query (request). |
| S2-15 OpaqueAttributeDefault | DEFERRED | needs `value_to_starlark` in blaze_packages; behaviour equals Java except the repr text. |
| S2-16 PROJECT.scl | FIXED as a loud failure | `blaze_sky_top/src/project.rs` ports the lookup; a resolved project file is `unported!("Project.getProjectFiles")`. No test. |
| S2-17 event posts | FIXED | `AnalysisRootCauseEvent` at its four sites, `ConfigurationTransitionEvent`; blaze_bep does not subscribe to them yet. |
| S2-18 small items | FIXED | `get_var` tracking is `unported!` when on; `expander.rs` throws where Java throws; `get_execution_os` from the execution platform; `read_lines_as_latin1` fails on non-UTF-8; `try_throw_target_parsing_exception` keeps the cause's exit code (with `TargetParsingException::with_detailed_exit_code`, added to blaze_cmdline with root's approval) and reports cycles; `--query_file` non-UTF-8 is loud. |

## S3 that became ports

- `SkyframeErrorProcessor.processAnalysisErrors` (`blaze_sky_top/src/skyframe_error_processor.rs`),
  `CyclesReporter` with the target, registered-toolchains and registered-execution-platforms
  reporters, `AnalysisFailureEvent` / `LoadingFailureEvent`. Loading errors, missing deps,
  visibility errors, cycles and rule errors are reported as Bazel does, with and without
  `--keep_going`, with Bazel's exit codes. Scenario `top-analysis-errors` (58 steps).
  `ReportedException.withoutMessage` is ported (the reason Bazel's "build aborted" line has no
  suffix). The four other cycle reporters stay `unported!`.
- genrule (`blaze_rules/src/generic/genrule.rs`, full `CommandHelper`, `LabelExpander`):
  scenario `genrule` (12 genrules in aquery with action keys, a build with outputs, 12 error
  cases). Windows `cmd_bat` / `cmd_ps` stay `unported!`.
- Incompatible targets in a test configuration (`createEmptyTestParams`).
- aquery `--output=text` prologue (the per-action text stays `unported!`).
- `ctx.build_file_path` (hit by workerd): scenario `ctx-build-file-path`.
- Errors while creating the top-level configuration (`--platforms` / `--host_platform` /
  `--extra_toolchains` / `--extra_execution_platforms` naming a wrong or missing target):
  `SkyframeExecutor.createBuildConfigurationKey` / `getConfiguration` wrapping, the two toolchain
  cycle reporters, `catch_invalid_platform_exception`. Scenario `top-bad-platform` (26 steps).

## Disputed (Java evidence)

- S1-6. `tags` is declared order-independent, so Bazel sorts and de-duplicates it when the rule
  is created (`BuildType.java:249`, the attribute's `orderIndependent()` in `BaseRuleClasses`).
  `TargetUtils.getExecutionInfo` therefore sees sorted tags. The oracle's raw jsonproto for
  `tags = ["no-remote-exec", "no-remote"]` prints `no-remote, no-remote-exec`, and for
  `["no-sandbox", "no-asan"]` `no-asan, no-sandbox` (scenario `cc-tags`, and the unit test's
  expected order is the oracle's). Blaze already printed that. Changed anyway:
  `java_hash_map_order` is now an exact `HashMap.putVal` / `resize` simulation.
- S2-6, `ctx.features` and `ctx.disabled_features`: Java returns `ImmutableList`, which the
  interpreter converts with the thread's mutability (`MethodDescriptor.java:315`,
  `Starlark.java:214`): a new mutable list per call. `ctx.features.append("zz")` exits 0 on the
  oracle (`ctx-immutable` steps 12-13).
- Actions-review probe "Blaze follows the Java source" for the failing `map_each` in aquery: it
  did not. Java's `doPostAnalysisQuery` calls `callback.process` directly, so the
  `InterruptedException` reaches BuildTool ("build interrupted", exit 8). Fixed; scenario
  `query-map-each`.

## Deferred, with the reason

- Flags with `scope = "project"`: `unported!("BuildOptionsScopeFunction.findProjectFiles")`.
- `ParsedFlagsFunction` (platform `flags`, platform mappings with Starlark flags): existing
  `unported!` stays; needs `PlatformValue` to carry parsed flags.
- cquery order: a node that is the first requester of the exec platform's PLATFORM value gets its
  exec deps one request later in Java; the evaluator has no "is this value done" query. Seen in
  scenario `graph-order` (one line), not in brotli or re2.
- `rdeps()` / `allpaths()` over several targets: Java iterates an identity-hashed map; the
  oracle's own order changes between runs.
- Order of two conflicting targets in the conflict WARNING (racy on the oracle).
- `TestExecException` is not told apart in the build command's `rethrow`.
- BEP: `buildToolLogs` lacks `command.profile.gz` and `critical path` (profile and critical-path
  computation are not ported); BEP with skipped targets is `unported!`.

## Requests to other owners

- third_party starlark-rust: `StarlarkValue::get_attr_fallible` used by `getattr` / `hasattr`
  (S2-7); the evaluator in `at` / `is_in` (S2-8).
- blaze_actions: `ResourceSetOrBuilder` slot (S2-10); a home for `ActionConflictException`,
  `MapBasedActionGraph`, `Actions.canBeShared`; `Action::java_class_name()`.
- blaze_query: `addAttributes` override hook (S2-14).
- blaze_packages: `value_to_starlark` for computed / late-bound defaults (S2-15);
  `get_execution_info` in Java order.
- blaze_skyframe: a query for an existing value (cquery order residual, exec discovery).
- blaze_vfs: the `[unix_jni.cc:NNN] ` prefix of native I/O errors.
- blaze_client: `--install_md5`, `--restart_reason=no_daemon` in the BEP command line.

## Integrity audit

- S1-1 (order hidden by the referee): fixed as S2-5 above; the referee's order items are in the
  scoreboard.
- S1-2: fixed as S1-1 above.
- S2-5 (silent no-ops): swept per crate. New loud guards for accepted-but-unported options are in
  `blaze_runtime/src/unported_options.rs` (remote cache / executor, BES backend, BEP files on
  non-build commands, execution graph log, Starlark debugger, ...). Ported instead of skipped:
  `--target_pattern_file`, `--explain`, `decideKeepIncrementalState`, `BuildResultPrinter`,
  `SkymeldModule.determineIfMergingAnalysisExecution`, `BuildInterruptedEvent`, batch SIGINT.
  Comments reworded to the mandated form: about 60 in blaze_runtime, 36 in blaze_exec /
  blaze_sky_exec, 12 in blaze_sky_top. Not audited one by one: about 117 structural `PORT:`
  notes in blaze_rules / blaze_analysis `rule/`, and 83 in blaze_buildtool / blaze_bep.

## Evidence

See the final table in docs/reviews/2026-11-18-exec-response.md ("Evidence for both responses").
