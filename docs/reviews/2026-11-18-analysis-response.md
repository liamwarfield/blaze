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

# Round 2 response

Analysis fixer, 2026-11-18. Covers the "Round 2" section of docs/reviews/2026-11-18-analysis.md
(bound to `12ca29e`). State described: HEAD `4c29efc` plus the uncommitted working tree, which also
holds the in-flight edits of the execution fixer (blaze_exec, blaze_sky_exec, blaze_buildtool,
blaze_runtime commands) and of the remote lead (blaze_remote); binary
`~/.cache/blaze/int-bin/blaze-anafix-3` (sha256 `0ff8fc5e96cf`, built from that tree at the end of
the round; `blaze-anafix-1`, sha256 `5c6616b9582f`, is the first build of the round). Every
expected value below comes from the Java at the pin or from an oracle transcript.

## Blocking

| id | disposition | where | test |
| --- | --- | --- | --- |
| R2-1 nine cpp lists immutable | FIXED | `blaze_rules/src/cpp/cpp_configuration.rs` `string_list` is now `Starlark.fromJava` of an `ImmutableList` (a new mutable list per access) for `copts`, `cxxopts`, `conlyopts`, `objccopts`, `linkopts`, `lto_index_options`, `lto_backend_options`; `strip_opts` stays `immutableCopyOf` (`CppConfiguration.java:392`). `cc_starlark_internal.rs` `mutable_string_list` for `per_file_copts` and `collect_per_file_lto_backend_opts`; `expand_and_tokenize` keeps `immutableCopyOf` / the frozen `"[]"` default (`CcStarlarkInternal.java:1295`). | scenario `ctx-immutable` steps 15-20 and 24-29 (six public lists appended and re-read, oracle: `["-DPROBE"] 0` each) |
| R2-1 re-audit of every S2-6 immutable value | DONE, no other overshoot | each `alloc_immutable_*` site checked against the Java return: `ctx.attr` / `ctx.files` / split attrs / `ctx.outputs` lists (`StarlarkAttributesCollection.java:221-369`, `StarlarkRuleContext.java:233,557,576`), `ctx.var` (`buildImmutable`, :847), `tokenize` (:926), `super()` empty (:660; the non-empty case is `StarlarkList.of(thread.mutability(), ...)`, mutable, as in Rust), `toolchain_types()` (`StarlarkToolchainContext.java:83`), `cc_common` tool requirements / execution requirements / command line / env (`CcModule.java:83-128`), `HeaderInfo` lists (`CcCompilationContext.java:710-725`), `default_features_and_action_configs` (`CcToolchainFeatures.java:1308`), `intern_string_sequence_variable_value`, `cc_internal.freeze` | as above; `ctx-immutable` steps 2-14 |
| R2-2 accepted-but-unread semantics keys | FIXED: one key ported, three guarded, the rest read | `--incompatible_disable_target_default_provider_fields` is read (see table); `blaze_build_config/src/top_level.rs` `check_unported_semantics_keys`, called by `starlark_semantics_from` (the function SkyframeExecutor uses), fails with `unported!` naming the flag for the three keys whose Java readers are not ported anywhere | `build_language_options_semantics_test.rs`: `unread_key_*_is_loud` (3), `keys_without_java_reader_are_accepted` |

### R2-2: the 63 keys of `BuildLanguageOptions.setFlags`

"Java readers" lists the classes that read the key (grep of the constant and of its flag string
at the pin); "ported" says whether Blaze reads the key where those classes do. A key whose reader
lives in a class that is itself not ported is loud through that class's `unported!`, noted.

| # | key | Java readers | ported | guard |
| --- | --- | --- | --- | --- |
| 1 | incompatible_stop_exporting_language_modules | BazelRuleClassProvider, CcBootstrap, AppleBootstrap | yes (`builtins/registered.rs`) | - |
| 2 | incompatible_allow_tags_propagation | RuleContext, StarlarkActionFactory | yes | - |
| 3 | experimental_builtins_bzl_path | BzlLoadFunction, PackageFunction, StarlarkBuiltinsFunction | yes | - |
| 4 | incompatible_autoload_externally | AutoloadSymbols | non-empty: `unported!("AutoloadSymbols.<init>")` | existing |
| 5 | repositories_without_autoloads | AutoloadSymbols (only when 4 is non-empty) | through 4 | existing (4) |
| 6 | incompatible_disable_autoloads_in_main_repo | AutoloadSymbols | yes | - |
| 7 | experimental_builtins_dummy | GenericRules | yes | - |
| 8 | experimental_builtins_injection_override | StarlarkBuiltinsFunction | yes | - |
| 9 | experimental_bzl_visibility | BazelBuildApiGlobals | yes | - |
| 10 | allow_experimental_loads | BzlLoadFunction | yes | - |
| 11 | check_bzl_visibility | BzlLoadFunction, PackageFunction | yes | - |
| 12 | experimental_enable_android_migration_apis | none at the pin | no-op in both | - |
| 13 | experimental_single_package_toolchain_binding | ModuleFileGlobals | yes | - |
| 14 | experimental_enable_first_class_macros | StarlarkRuleClassFunctions, TargetDefinitionContext | yes | - |
| 15 | experimental_enable_scl_dialect | BzlLoadFunction | yes | - |
| 16 | experimental_isolated_extension_usages | ModuleFileGlobals, ModuleExtensionContext (`enableOnlyWithFlag`) | yes | - |
| 17 | incompatible_no_implicit_watch_label | StarlarkBaseExternalContext | yes | - |
| 18 | experimental_google_legacy_api | JavaStarlarkCommon; ActionApi.execution_info, JavaInfoApi (`enableOnlyWithFlag`) | java_common yes; `Target.actions` is `unported!` | existing |
| 19 | experimental_platforms_api | ConstraintCollectionApi methods; PlatformInfo / ConstraintSettingInfo / ConstraintValueInfo fields (`enableOnlyWithFlag`) | methods yes; the fields are always readable (deviation at the default, not at a non-default value; blocked like S2-7, below) | - |
| 20 | experimental_cc_shared_library | CcModule | yes | - |
| 21 | experimental_repo_remote_exec | StarlarkRepositoryModule, StarlarkBaseExternalContext | yes | - |
| 22 | experimental_disable_external_package | none at the pin | no-op in both | - |
| 23 | experimental_sibling_repository_layout | Label, ConfiguredTargetFactory, StarlarkActionFactory, BuildConfigurationFunction, ProcessPackageDirectory, ActionExecutionFunction, SkyframeExecutor | yes | - |
| 24 | incompatible_always_check_depset_elements | Depset | yes | - |
| 25 | incompatible_check_external_repo_source_dir_package_boundary | RecursiveFilesystemTraversalFunction (not ported) | no | **new** `unported!` |
| 26 | incompatible_disallow_empty_glob | StarlarkNativeModule | yes | - |
| 27 | incompatible_package_group_has_public_syntax | StarlarkNativeModule | yes | - |
| 28 | incompatible_fix_package_group_reporoot_syntax | StarlarkNativeModule | yes | - |
| 29 | incompatible_java_info_merge_runtime_module_flags | JavaStarlarkCommon | yes | - |
| 30 | incompatible_no_attr_license | StarlarkAttrModuleApi | yes | - |
| 31 | incompatible_no_implicit_file_export | PackageFactory | yes | - |
| 32 | incompatible_no_rule_outputs_param | StarlarkRuleFunctionsApi | yes | - |
| 33 | incompatible_run_shell_command_string | StarlarkActionFactory | yes | - |
| 34 | PRINT_TEST_MARKER (`--internal_starlark_flag_test_canary`) | MethodLibrary | yes | - |
| 35 | incompatible_do_not_split_linking_cmdline | none at the pin | no-op in both | - |
| 36 | incompatible_enforce_starlark_utf8 | BzlCompileFunction, PackageFunction, RepoFileFunction, CompiledModuleFile, StarlarkUtil, VendorFileFunction | yes (vendor mode is not ported) | - |
| 37 | incompatible_use_cc_configure_from_rules_cc | StarlarkRepositoryModule.failWithIncompatibleUseCcConfigureFromRulesCc (not ported) | no | **new** `unported!` |
| 38 | incompatible_unambiguous_label_stringification | Label | yes | - |
| 39 | max_computation_steps | PackageFactory | yes | - |
| 40 | nested_set_depth_limit | Depset, Runfiles | yes | - |
| 41 | incompatible_disable_starlark_host_transitions | StarlarkAttrModule | yes | - |
| 42 | incompatible_disable_objc_library_transition | CcModule | yes | - |
| 43 | incompatible_disable_transitions_on | ConfigGlobalLibrary (and StarlarkDefinedConfigTransition) | yes | - |
| 44 | add_go_exec_groups_to_binary_rules | CcModule | yes | - |
| 45 | incompatible_fail_on_unknown_attributes | StarlarkRuleClassFunctions, RuleFactory | yes | - |
| 46 | incompatible_enable_proto_toolchain_resolution | BazelProtoCommon, AutoloadSymbols | yes (autoloads: through 4) | - |
| 47 | incompatible_disable_non_executable_java_binary | JavaStarlarkCommon | yes | - |
| 48 | incompatible_disable_target_default_provider_fields | AbstractConfiguredTarget.getValue | **now yes** (`configuredtargets/mod.rs` `target_to_starlark` takes the rule's semantics) | - |
| 49 | experimental_rule_extension_api | StarlarkRuleClassFunctions | yes | - |
| 50 | experimental_dormant_deps | StarlarkAttrModuleApi | yes | - |
| 51 | experimental_starlark_type_syntax | BzlCompileFunction | yes | - |
| 52 | experimental_starlark_type_checking | BzlCompileFunction, Eval, StarlarkFunction | with type syntax: `unported!` in `bzl_compile.rs`; without it Java has no function type to check | existing |
| 53 | experimental_starlark_types_allowed_paths | BzlCompileFunction | yes | - |
| 54 | incompatible_enable_deprecated_label_apis | Label, StarlarkNativeModuleApi | yes | - |
| 55 | incompatible_stop_exporting_build_file_path | StarlarkRuleContext | yes | - |
| 56 | incompatible_disallow_ctx_resolve_tools | StarlarkRuleContext | yes | - |
| 57 | incompatible_simplify_unconditional_selects_in_rule_attrs | PackageFactory | yes | - |
| 58 | incompatible_locations_prefers_executable | StarlarkRuleContext, CcStarlarkInternal | yes | - |
| 59 | incompatible_resolve_select_keys_eagerly | SelectorList | yes | - |
| 60 | experimental_enable_starlark_set | MethodLibrary, EvalUtils | the `set` builtin only (`env.rs`); set operators on natively created sets are not gated | - |
| 61 | INTERNAL_BAZEL_ONLY_UTF_8_BYTE_STRINGS | Json, StringModule, FactsAdapter | no (code-point strings, decisions.md 2026-11-17) | **new** `unported!` on the non-default `false` |
| 62 | experimental_repository_ctx_execute_wasm | StarlarkBaseExternalContext | yes | - |
| 63 | FORCE_STARLARK_STACK_TRACE | MethodLibrary | yes | - |

The options `--experimental_starlark_types` and `--experimental_cc_skylark_api_enabled_packages`
reach no key and have no reader in Java either.

## Open S2 and new findings

| id | disposition | where | test |
| --- | --- | --- | --- |
| S2-10 `resource_set` callback | PORTED in analysis; one call pending in blaze_exec | `blaze_actions/.../spawn_action.rs` (additive): `ResourceSetBuilder` trait (`buildResourceSet(OS, inputsSize)`), `ResourceSetBuildError`, `SpawnActionBuilder::set_resources_builder`, `SpawnAction::get_resource_set_builder`. `blaze_analysis/.../starlark_action_factory.rs`: `StarlarkActionResourceSetBuilder` (callable kept with the frozen module, called on a fresh thread with the rule's semantics, `Dict.cast`, illegal keys, `getNumericOrDefault`, "Could not build resources for <mnemonic>. <message>"). The `// PORT:` in `finish_pending_actions` names the pending call. Making the dropped callback loud would stop every C++ link (rules_cc `finalize_link_action.bzl:412`), so it is not loud. | `resource_set_builder_tests` (7 ported StarlarkRuleContextTest cases + 1; `_incorrect_signature` ignored: starlark-rust's arity text) |
| S2-5 residual / R2-3 request rounds | DEFERRED, needs root | the two approximations stay: Java's Driver lookup rounds need "is this value done" from the evaluator (blaze_skyframe, not mine) | `graph-order` (its one-line DIFF remains) |
| S2-7 `getattr(x, name, default)` | BLOCKED, not loud | needs a fallible attribute hook in the vendored starlark-rust (a patch 0026: `StarlarkValue::get_attr_fallible`, called by `getattr` before the default); `third_party/` is not mine. The same hook is what the `--experimental_platforms_api` fields need (attribute functions get no evaluator). | `ctx-errors` steps 2-3 |
| S2-8 helper of another .bzl | BLOCKED, not loud | needs the evaluator in the index operator (vendored crate) | `ctx-toolchains-string` step 6 |
| S2-14 emulated addAttributes | DEFERRED | the override hook belongs in blaze_query (another owner) | - |
| S2-15 OpaqueAttributeDefault | FIXED for the four classes Blaze can tell apart; loud for the rest | `config/transitions.rs`: `StarlarkComputedDefault`, `LabelLateBoundDefault`, `LabelListLateBoundDefault`, `AlwaysNullLateBoundDefault` values with Java's `type()` and `repr`; a native rule's own `ComputedDefault` subclass prints via `unported!` (its `type()` still says "unknown": starlark-rust type names are static) | `attribute_default_value_tests` |
| R2-4 flag-package events | DEFERRED to the owner | `blaze_runtime/src/dispatcher.rs` is not mine. The call: report the stored events right after `parse_starlark_options` succeeds, as `BlazeOptionHandler.java:352` does. | - |
| R2-5 helper-script length unit | FIXED | `rule/command_helper.rs` `java_string_length` counts internal-string chars (UTF-8 bytes); `starlark_action_factory.rs` uses it; `cpp/util.rs` `java_string_hash_code` hashes the bytes | `java_string_length_counts_internal_string_chars`, `java_string_hash_code_matches_java` (non-ASCII case added) |
| R2-6 AnalysisFailureEvent for incompatible explicit targets | FIXED (posted) | `build_view.rs` `check_target_compatibility_of_build_driver_keys` posts `AnalysisFailureEvent.whileAnalyzingTarget(key, [])` before the keep_going branch (`SkyframeErrorProcessor.java:411`). No Blaze subscriber reads it yet (BEP, AggregatingTestListener are other owners'). | none (needs a subscriber) |
| R2-7 shouldCheckForConflicts | FIXED | `skyframe_build_view.rs`: `!largestTopLevelKeySetCheckedForConflicts.containsAll(newKeys)` with the empty set of a fresh view | `fresh_view_checks_any_non_empty_key_set` |
| R2-8 cquery targetsForProjectResolution | FIXED | `cquery_command.rs` sets it for `somepath` / `allpaths`; `BuildRequest::targets_for_project_resolution`; `build_tool.rs` evaluates those patterns with a no-op handler for `evaluate_project_file` (`BuildTool.java:315`) | scenario `cquery-project-resolution` (new) |
| R2-9 codes and supersets | FIXED: NoSuchThing code; DEFERRED: the rest | `skyframe_executor.rs` wraps a `NoSuchThingException` / `NoSuchPackageException` / `NoSuchTargetException` with its own PackageLoading code (`InvalidConfigurationException::with_package_loading_code`, mapped in `build_tool.rs`); the wrong "exit code of both is 2" comment is gone (PACKAGE_MISSING exits 1). `get_package_roots` superset and the tree-bin `remove` stay (incremental-only / documented). | `wrapped_no_such_thing_keeps_its_package_loading_code` |
| R2-10 | S3, loud, unchanged | - | - |
| R2-11 dict of dicts with a non-string inner key | FIXED | `blaze_build_config/src/starlark_transition.rs`: the inner `Dict.cast` failure falls through to the patch-transition attempt, after earlier entries' validation errors | scenario `transition-results` (new; oracle: "undeclared output 'a'") |
| R2-12 evidence | FIXED | see "Evidence" | - |
| R2-13 Variables repr; hasattr(target, "files") | FIXED | `cpp/starlark_values.rs` prints `CcToolchainVariables$MapVariables` / `$SingleVariables`; `target_value.rs` `has_attr` is Java's one-argument `getValue(name) != null` | `repr_names_the_runtime_subclass`; hasattr: no test (needs a configured target) |
| R2-14 absolute_symlink | FIXED | `cc_starlark_internal.rs` accepts any `StarlarkActionContext` (rule or subrule context, `starlark_subrule::starlark_action_context_rule_context`), Java's type error text otherwise | none (needs a subrule test bed) |

Found by the full-transcript probe: the traceback of a failing rule implementation or transition
had an extra first line `File "<builtin>", in <toplevel>` (starlark-rust reports the outermost
frame of a call made from native code without a call site; Java's stack starts at the callee).
Fixed where my crates format those errors (`starlark_rule_configured_target_util.rs`
`native_call_stack`, `blaze_build_config/src/starlark_transition.rs`); test
`native_call_stack_tests` with the oracle's text. The general fix belongs in blaze_starlark's
`EvalError::from_starlark` (request).

Found while fixing (same class as S2-4): `Dict.cast(x, String.class, Object.class, what)` said
"for resource_set, got string, want dict" and "want dict<string, any>"; Java says
"got string for 'resource_set', want dict" and "dict<string, unknown>"
(`Dict.java:663-677`, `Starlark.classType(Object.class)`). Fixed in `starlark/util.rs` and
`cpp/util.rs`; `blaze_sky_loading/src/starlark_builtins.rs:214` has the same text (not mine).

### The call blaze_exec must add (S2-10)

In `crates/blaze_exec/src/analysis/actions/spawn_action.rs` `new_action_spawn`, replace
`Arc::new(to_resource_set(parent.get_resource_set_or_builder()))` by

```rust
match parent.get_resource_set_builder() {
    // Java BaseSpawn.getLocalResources -> buildLocalResources(OS.getCurrent(), inputsSize, ...)
    Some(builder) => Arc::new(StarlarkResourceSet(builder.clone())),
    None => Arc::new(to_resource_set(parent.get_resource_set_or_builder())),
}
```

with `StarlarkResourceSet` implementing blaze_exec's `ResourceSetOrBuilder` by
`builder.build_resource_set(OS::get_current(), inputs_size).map(|r| to_resource_set(&r))`,
mapping `ResourceSetBuildError { message }` to a `UserExecException` with
`FailureDetail { message, StarlarkAction { code: STARLARK_ACTION_UNKNOWN } }`. That needs
`ResourceSetOrBuilder::build_resource_set` to return `Result<ResourceSet, ExecException>` (Java
throws `ExecException`); `SimpleSpawn::get_local_resources` already returns a `Result`.

## Evidence (Round 2)

- Rust tests, my crates plus blaze_actions (`cargo test -p blaze_analysis -p blaze_rules
  -p blaze_sky_analysis -p blaze_sky_top -p blaze_cquery -p blaze_build_config -p blaze_config
  -p blaze_actions`, the working tree above): 550 passed, 0 failed, 685 ignored. Not a
  workspace run. `clippy --tests -D warnings` on the same crates: clean (blaze_sky_loading, a
  dependency I do not own, has 3 `collapsible_if` / `explicit_auto_deref` errors under the
  current clippy).
- Referee, brotli and re2 (`scripts/oracle/record`, labels `blaze-anafix-1` with binary
  `5c6616b9582f` and `blaze-anafix-3b` with `0ff8fc5e96cf`; scored with `scoreboard`, not
  written to corpus/): every per-surface row equals `corpus/scores/blaze-12ca29ee95e0.md`:
  23 of 26 surfaces MATCH, 13,850 of 13,858 items (open: 4 BEP items per project, 1 extra re2
  stderr line). No regression. (Label `blaze-anafix-3` has a failed re2 build: the
  repository rule's `process-wrapper` spawn got ENOMEM while the probe, the tests and clippy ran
  at the same time; `blaze-anafix-3b` is the same binary recorded alone.)
- Probe (R2-12). `scripts/w4/probe` now compares full normalised transcripts (exit code, full
  stdout, every stderr line in order; only run-specific values are normalised and only
  rate-limited progress lines dropped; see `probe help`), step by step, prints
  `M of N steps match`, and exits 1 on any difference; `compare-all` adds the totals.
  `scripts/w4/flags-probe` is now the probe scenario `flags-probe` (the 71 former cases, each as
  a label_kind and a full jsonproto step; the transcript is no longer sorted or filtered).
  Every ORACLE.txt was re-recorded with the oracle in the new format (35 scenarios, including
  the execution fixer's `exec-*`, `test-*`, `ui-*`; new: `transition-results`,
  `cquery-project-resolution`). With `blaze-anafix-3`:
  **94 of 694 steps match in full; 0 of 35 scenarios match in full.** In the narrower
  `--messages` view (exit code, stdout, complete text of every message, as a sorted list):
  225 of 694. The old first-line comparison hid most of this. The recurring causes I saw
  (ctx-immutable, ctx-errors, flags-transitions inspected line by line): (1) every successful
  build's `INFO: Elapsed time: ..., Critical Path: ...` lacks the critical path (critical path
  not ported); (2) Blaze's server does not survive the probe's per-command `systemd-run --scope`
  while Bazel's does, so later steps print `Starting local Bazel server` again and load every
  package again (`N packages loaded`), and Bazel's `discarding analysis cache` warnings have no
  Blaze counterpart; (3) a failed analysis prints `INFO: 0 processes.` where Bazel prints
  `INFO: 1 process: 1 internal.`; (4) starlark-rust's error positions (an index assignment or
  call is located at its start, Java at `=` or `(`); (5) the known S2-7 `hasattr` / `getattr`
  steps. (1)-(3) are outside the analysis crates (client, execution summary, critical path);
  (4) is the vendored interpreter. These numbers are the true counts; the round-1 "71/71" and
  scenario MATCHes were first-line comparisons.

# Round 3 response

Analysis implementer, 2026-11-18. Covers the "Round 3" section of docs/reviews/2026-11-18-analysis.md
(carried items S2-7 with the platforms-API fields, S2-8, R3-1, R3-2) and the adoption of the
interpreter hooks of patches 0026-0031. State described: HEAD `5013a3f` plus the uncommitted
working tree (the remote lead's in-flight edits in blaze_exec, blaze_sky_exec, blaze_buildtool,
blaze_runtime `build_remote.rs` are in the same tree). Binary `~/.cache/blaze/int-bin/blaze-anafix-r3`
(sha256 `8def70d5cac8`), built from that tree. Every expected value comes from the Java at the pin
or from an oracle transcript.

## Carried items

- **S2-7 (closed).** Natives now implement `get_attr_fallible` / `has_attr_fallible` where Java's
  `getValue` throws or reads the semantics, so `getattr(x, name, default)` returns the default
  only for a missing field, and `hasattr` propagates the error (Starlark.java:967, :978):
  `ctx.fragments` (undeclared fragment: RuleContext.java:512), `ctx.outputs` and a closed
  `ctx.outputs` (`checkMutable`, StarlarkRuleContext.java:394/441), `Target`
  (AbstractConfiguredTarget.java:127, reading `--incompatible_disable_target_default_provider_fields`
  from the thread's semantics). The `unported!` stops in `has_attr` are gone. Oracle:
  `ctx-errors` steps 2-3 (`Error in getattr:` / `Error in hasattr:`). Unknown-field texts:
  blaze_starlark `StarlarkInfo::with_message` (StarlarkInfoWithMessage.java:51) gives `ctx.attr`,
  `ctx.executable`, `ctx.file`, `ctx.files`, `ctx.split_attr` and the transition `attr` struct
  Java's formats (the two dead constants are used); oracle `ctx-errors` step 9.
- **Platforms API (closed for fields).** `PlatformInfo.{label,constraints}`,
  `ConstraintSettingInfo.{label,default_constraint_value}`, `ConstraintValueInfo.{label,constraint}`
  and `ConstraintCollection.constraint_settings` are absent (field error, `getattr` default,
  `hasattr` False) unless `--experimental_platforms_api` (CallUtils.java:137). New oracle scenario
  `platforms-api` (12 steps, with and without the flag). Remaining: `dir()` still lists the gated
  fields (no evaluator in `dir_attr`; Java `dir` is `[]` without the flag, oracle step 2), and the
  gated *methods* `get`/`has`/`has_constraint_value` stay methods that fail with "function get()
  is experimental"; they are reachable only through `PlatformInfo.constraints`, itself absent
  without the flag, so the difference cannot be observed. Both need an interpreter hook
  (`dir_attr` with the evaluator); requested.
- **S2-8 (closed).** `StarlarkToolchainContext` overrides `at_with_eval` / `is_in_with_eval` and
  converts a string key with the package context of `BazelModuleContext.of_innermost_bzl(eval)`
  (LabelConverter.forBzlEvaluatingThread, StarlarkToolchainContext.java:106); the
  implementation-function package context (and its plumbing through `ctx`, `ctx.exec_groups`,
  subrule contexts) is removed. Oracle: `ctx-toolchains-string` step 6.
- **R3-1 (closed).** Under `--noexperimental_enable_starlark_set`, `set` stays defined and a call
  fails "Use of set() requires --experimental_enable_starlark_set" (MethodLibrary.java:691).
  Oracle scenario `starlark-set-flag`; test `semantics_test::disabled_set_is_defined_and_fails_when_called`.
- **R3-2 (closed).** See "Probe" below.

## Interpreter hooks adopted

- `<toplevel>` stripping removed from `native_call_stack` (blaze_analysis) and
  `starlark_transition.rs`; tracebacks unchanged against the oracle (`ctx-immutable` step 2,
  `flags-transitions` step 23 in the probe run below).
- Name resolution (patch 0029): `bzl_load.rs` resolves the file against its predeclared globals
  before any load is requested (Java BzlCompileFunction: `Program.compileFile`), reports every
  error (a guarded symbol with the guard's message) and fails "compilation of module '...'
  failed"; `package_function.rs` does the same in `compile_build_file` (PackageFunction.java:1407),
  so an undefined name in a BUILD file is a SYNTAX_ERROR event and the file is not executed (the
  `unported!` for guarded BUILD symbols is gone). No JUnit test of the pin has two undefined names
  in one file; new oracle scenario `load-unresolved-names` and tests
  `resolve_names_test::{every_unresolved_name_of_a_build_file_is_reported,
  every_unresolved_name_of_a_bzl_file_is_reported, unresolved_names_are_reported_before_loads}`
  (locations and order from the oracle; message text is starlark-rust's, ruled).
- `TYPE_SYNTAX_NOT_PORTED_PREFIX` and its `unported!` in `bzl_compile.rs` removed (patch 0030).
- `BazelModuleContextValue` prints the label (BazelModuleContext.java:133), so a def prints
  `<function f from //pkg:defs.bzl>`; test `function_repr_names_its_bzl_label`.

## Patch 0005 (`split()` / `rsplit()` without `sep`)

`grep -P '\.r?split\(\s*(\)|maxsplit)'` over every `.bzl`/BUILD/MODULE file of the bundled
`builtins_bzl`, the oracle install base's `embedded_tools`, the three corpus checkouts (brotli,
re2, workerd) and the 1,041 external repositories of the 205 oracle output bases under
`~/.cache/blaze/oracle-out`: no match (the pattern matches a positive control). Nothing real
depends on the whitespace split.

## Execution review R3 item (routed by root)

cquery and aquery construct the ExecutionTool as Java does after the analysis
(BuildTool.java:520): `PostAnalysisCommandEnvironment.execution_tool`, called before the
post-processor; blaze_runtime `cquery.rs` builds the executor setup as `build` does and maps
`check_execution_tool_created()`. New oracle scenario `query-strategy-errors` (exit 2 and Java's
message for `--spawn_strategy=bogus` and `--strategy=Genrule=bogus`, on cquery and aquery).
PORT notes: the remote module's contributions are not built (its support is private to
`build.rs`); the remote strategy registers under the same name either way, so the remote
cache/executor options are cleared for this check and validation is unchanged; the setup also
recreates `<output_base>/action_outs` (blaze_buildtool exposes the constructor only with
`prepareForExecution`).
