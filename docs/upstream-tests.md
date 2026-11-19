# Bazel's integration tests against Blaze v0.1.0

> **Withdrawn (2026-11-19, audit round 3, `docs/reviews/2026-11-19-audit-3.md`).** The totals
> in this document were measured with harness scripts that had no lock, while other agents ran
> the same scripts. The run recorded another binary's logs for two targets (32 passes that are
> not this binary's) and the shared repo contents cache was deleted under it three times. From
> this run's own evidence: 1,397 of 2,878 functions measured as passing, 34 unmeasured, and at
> least 11 failures caused by the harness (cause family 28, "files vanish from the shared repo
> contents cache", is the harness, not Blaze). Targets: 62 of 211. The cause ranking below is
> still a usable work list; no total from it may be quoted. A fresh full run of the same binary
> under the fixed harness is recorded under the label `blaze-v0.1.0-r2`.


First full run of Bazel's own shell and Python integration tests (237 targets) against Blaze,
2026-11-19. The baseline is the pinned real Bazel on the same host
(`corpus/upstream-tests/oracle-9.3.2.json`: 211 targets and 2,878 functions pass there).

## Method

- **Binary.** `~/.cache/blaze/work/v010-blaze`, debug build (`scripts/cargo-capped build -p
  blaze`) from a clean worktree of tag `v0.1.0` = commit `5a4e4a0`. sha256
  `291d6df99d87386f3761edf7f01d65358602ead67273dea0d64e1628f3ee3479`. `/home` had 228 GB free,
  so the private build was allowed. The worktree and its `target/` were removed afterwards.
- **Run.** `scripts/upstream-tests/run --binary ~/.cache/blaze/work/v010-blaze --label
  blaze-v0.1.0 --fresh`: the whole default selection (237 targets, nothing sharded or
  filtered), with the harness defaults (`--test-jobs 6`, `--jobs 8`, `MemoryMax=14G`). Wall
  time 3,301 s. `oom_kill` 0.
- **Load caveat.** Several other agents were compiling Rust on the 16-core machine during the
  run. Every target that hit a timeout was rerun once, as label `blaze-v0.1.0-timeout-rerun`
  (4 targets, 1,056 s). All four timed out again on the same shards and the same functions
  (see [Timeouts](#timeouts)), so none of the timeouts comes from load.
- **Compare.** `scripts/upstream-tests/compare corpus/upstream-tests/oracle-9.3.2.json
  blaze-v0.1.0`. Only names the oracle passes are counted. Names on the oracle's flaky list
  (5 targets, 39 functions) are left out.
- **Classify.** `scripts/ut-analysis/classify corpus/upstream-tests/oracle-9.3.2.json
  blaze-v0.1.0` (new tool) reads each failing function's text. That is its testcase in
  test.xml, its `** <fn> **` section of test.log, or for a function with no result the failing
  shards' logs. It takes the first of these causes:
  1. a `not ported yet: <Class.method>` panic;
  2. another Rust panic;
  3. a crash signal;
  4. the first `ERROR:` line whose normalized text the oracle does not print in the same
     target's logs (runs `oracle-9.3.2` and `oracle-9.3.2-run2`);
  5. a timeout;
  6. the test's own assertion message.

  `scripts/ut-analysis/families blaze-v0.1.0` (new tool) then groups those causes into
  families. It maps the `unported:` causes by class. For the rest it uses rules I wrote after
  reading the logs.
- **First cause only.** A function is counted under the first cause in its log. Fixing that
  cause can expose a later one, so a family's count is an upper bound on the passes it can
  give.

## Totals

Oracle-passing names only, flaky names excluded.

| | Targets: oracle | Targets: Blaze | Functions: oracle | Functions: Blaze |
| --- | ---: | ---: | ---: | ---: |
| shell | 184 | 56 | 2,515 | 1,242 (+7) |
| py | 27 | 6 | 363 | 180 |
| **total** | **211** | **62** | **2,878** | **1,422 (+7)** |

| Directory | Targets: oracle | Targets: Blaze | Functions: oracle | Functions: Blaze |
| --- | ---: | ---: | ---: | ---: |
| `src/test/py/bazel` | 27 | 6 | 363 | 180 |
| `src/test/shell` | 3 | 3 | 6 | 6 |
| `src/test/shell/bazel` | 96 | 35 | 1,147 | 595 |
| `src/test/shell/bazel/remote` | 9 | 1 | 210 | 112 |
| `src/test/shell/integration` | 76 | 17 | 1,152 | 529 (+7) |

- `compare`: targets retained 62, lost 148, absent 1. Functions retained 1,422, lost 1,377,
  absent 79.
- **+7:** seven `ui_test` functions print `PASSED` in test.log, but their shard's test.xml is
  not well-formed, so `run` drops them (see [Harness problems](#harness-problems)). Counting
  them gives 1,429 functions. 1,449 oracle-passing functions do not pass.
- **Raw run totals** (all 237 targets, before the oracle filter): targets 76 pass (14 of them
  vacuous), 159 fail, 1 timeout, 1 `not_run` (in fact a build failure). Functions 1,426 pass,
  1,388 fail, 3 skip. Four of the passes are names that are flaky on the oracle.

## Causes, ranked

The first cause of each of the 1,449 failing functions, grouped into 81 families. "MF" names the
section of `docs/missing-features.md` that lists the gap. **NEW** means the file does not have
it. Example names drop the package. The Java column gives the class at the pin
(`src/main/java/com/google/devtools/build/lib/...`) where Blaze stops, or the behaviour that
differs.

| # | Family | Functions | Targets | Examples | Java class / behaviour | MF |
| ---: | --- | ---: | ---: | --- | --- | --- |
| 1 | Java rules | 395 | 43 | `bazel_module_test#BazelModuleTest.testHttpJar`, `tags_propagation_native_test#test_java_library_tags_propagated`, `test_test#test_process_spawned_by_test_doesnt_block_test_from_completing` | `java_common_internal_do_not_use.expand_java_opts` (`starlarkbuildapi/java/JavaCommonApi.java:693`, `rules/java/JavaStarlarkCommon`) is the first `java_common` built-in that `rules_java` calls. Worker tests fail here too, because their worker is a `java_binary`. | Analysis: Java |
| 2 | `run` command | 175 | 36 | `bazel_external_repository_test#…testNewLocalRepositoryNoticesFileChangeInRepoRoot`, `bazel_java_test_jdk21_toolchain_head#test_auto_bazel_repository`, `ui_test#test_bazel_run_error_visible` | `runtime/commands/RunCommand.exec` | Commands |
| 3 | `coverage` command | 124 | 18 | `bazel_coverage_cc_head_test_gcc#test_cc_test_gcov_multiple_headers`, `bazel_coverage_java_jdk21_toolchain_head_test#test_branch_in_switch_coverage`, `sandboxing_test#test_write_non_hermetic_tmp` | `runtime/commands/CoverageCommand.exec` | Commands; Analysis: Coverage |
| 4 | Collateral: suite stopped after an XML writer overflow | 51 | 2 | `remote_build_event_uploader_test#test_upload_all_combined_cache`, `build_event_stream_test#test_command_whitelisting`, `build_event_stream_test#test_workspace_status` | These functions never ran. An earlier function failed with a very large log (`remote_build_event_uploader_test#test_upload_all_alias_action`: `--profile` not uploaded; `build_event_stream_test`: `test_glob_filesystem_operation_cost`, `test_action_timing_details`). unittest.bash's perl XML writer then failed with `Argument list too long`, and the suite stopped. The oracle hits the same flaw in this target (README: flaky). | (harness) |
| 5 | `fetch` command | 43 | 10 | `bazel_fetch_test#BazelFetchTest.testFetchAll`, `bazel_repository_cache_test#test_starlark_download_and_extract_fail_without_cache`, `starlark_repository_test#test_timeout_tunable` | `bazel/commands/FetchCommand.exec` | Commands |
| 6 | Remote repo contents cache | 38 | 1 | `remote_repo_contents_cache_test#…testAccessFromOtherRepo_read`, `…testNotCachedWhenRecordedInputsChange_staticDep`, `…testUseSourceDirectoryWithSymlinksInBuildRule_actionDoesNotUseCache` | `remote/RemoteRepoContentsCacheImpl` | Remote |
| 7 | Text formats (`--build_event_text_file`, `--output=textproto`) | 38 | 14 | `build_without_the_bytes_test#test_bytestream_uri_prefix`, `bazel_worker_test#test_worker_metrics_collection`, `target_compatible_with_test#test_success_on_incompatible_top_level_target_with_skipping` | `buildeventstream/transports/TextFormatFileTransport` (protobuf TextFormat); 1 is cquery `--output=textproto` | Reporting (BEP text file) |
| 8 | No persistent server | 36 | 15 | `build_without_the_bytes_test#test_downloads_toplevel`, `client_test#test_no_server_restart_if_options_order_changes`, `startup_options_test#test_different_startup_options` | These are the tests' own assertions. Server pid changes between commands (`client_test`, `bazel_server_mode_test`, `discard_analysis_cache_test`). No `server needs to be killed` restart warnings. No lock-wait messages (`--noblock_for_lock`). `1 action cache hit` where Bazel has an in-memory hit (`bazel_spawnstats_test`, `build_without_the_bytes_test`, `remote_execution_test`). Rules re-analysed (`my_rule is being analyzed`). External files always re-checked (`check_external_files_test`, `external_correctness_test`). Repo re-fetched on an unrelated env change. | Server and client |
| 9 | `mod` subcommands | 26 | 4 | `bazel_module_test#…testDefaultModuleMirrors`, `mod_command_test#ModCommandTest.testModTidyWithIncludes`, `repo_contents_cache_test#…testCachedRepoWithSymlinks` | `bazel/commands/ModCommand.execInternal` (`show_repo`), `ModCommand.runTidy` | Commands (`mod`) |
| 10 | cquery output formatters | 25 | 4 | `options_test#…testCommonPseudoCommand_unsupportedOptionValue`, `configured_query_test#test_starlark_output_invalid_expression`, `target_compatible_with_test#test_cquery_with_starlark_formatting` | `query2/cquery/StarlarkOutputFormatterCallback.start` (18), `TransitionsOutputFormatterCallback.start` (6), `FilesOutputFormatterCallback.start` (1) | Commands (`cquery`) |
| 11 | Curses progress bar / terminal UI | 22 | 8 | `build_without_the_bytes_test#test_download_top_level_remote_execution_after_local_fetches_inputs`, `progress_reporting_test#test_seemingly_too_many_total_actions_due_to_change_pruning`, `ui_test#test_terminal_title` | `runtime/UiStateTracker.writeProgressBar` (17); `--terminal_columns`, cursor codes and terminal title (5) | Reporting (`--curses=yes`) |
| 12 | Project files | 22 | 2 | `flags_scoping_integration_tests#test_flags_scoping_remove_out_of_scope_flags`, `flagset_test#test_partial_project_builds_fail_with_multiple_conflicting_default_configs`, `flagset_test#test_spaceDelimited_andNegativeBooleanflags_succeed` | `Project.getProjectFiles`, `BuildOptionsScopeFunction.findProjectFiles` (`skyframe/ProjectFunction`) | Loading (PROJECT.scl) |
| 13 | Autoloads | 22 | 1 | `incompatible_autoload_externally_test#test_bad_flag_value`, `#test_removed_rule_loaded`, `#test_whole_repo_flag` | `packages/AutoloadSymbols.<init>` with a non-default `--incompatible_autoload_externally` | Loading |
| 14 | SkyQuery | 20 | 3 | `bazel_external_repository_test#…testUniverseScopeWithBazelIgnoreInExternalRepo`, `bazel_query_test#test_starlark_glob_symlink_source_not_included_in_rbuildfiles`, `bazel_query_test#test_universe_scope_with_without_star` | `query2/SkyQueryEnvironment` (`--universe_scope`, `rbuildfiles`) | Commands (`query`) |
| 15 | Toolchain resolution debug | 20 | 8 | `bazel_java_test#test_target_exec_properties_java`, `toolchain_test#test_exec_platform_order_with_mandatory_toolchains`, `toolchain_test#test_toolchain_debug_messages_target` | `skyframe/toolchains/ToolchainResolutionDebugPrinter.create` | Analysis |
| 16 | Platform mappings / platform-based flags | 20 | 2 | `platform_based_flags_test#test_cache_invalidation`, `#test_transition_ignores_mapping`, `platform_mapping_test#test_transition_platform_mapping` | `skyframe/config/PlatformMappingFunction.parse`, `ParsedFlagsValue.Key.create` | Analysis |
| 17 | `info` without a key / JVM keys | 19 | 10 | `bazel_lockfile_test#…testFactsInReproducibleExtension`, `client_test#test_shutdown_different_options`, `watchfs_test#test_special_chars` | `runtime/commands/info/CommittedHeapSizeInfoItem.get`, `UsedHeapSizeAfterGcInfoItem.get`. Plain `bazel info` reaches the heap items. | Commands (`info`) |
| 18 | FailureDetail for non-loading package errors | 18 | 5 | `bazel_lockfile_test#…testAddModuleToRegistryWithLockfileInRefreshMode`, `bazel_yanked_versions_test#…testContainingYankedDepFails`, `starlark_configurations_test#test_invalid_mnemonic_from_transition_top_level` | `util/DetailedExitCode` / `FailureDetail` of category `ExternalRepository` (17) and `INVALID_OUTPUT_DIRECTORY_MNEMONIC` (1). Blaze site: `crates/blaze_query/src/compat/failure_details.rs:25`. The Bzlmod resolution errors in these tests (yanked version, lockfile mismatch, override errors) panic instead of printing their error. | **NEW** |
| 19 | Remote flags not yet wired | 18 | 4 | `build_without_the_bytes_test#test_bazel_run_with_minimal`, `remote_downloader_test#test_remote_downloader_http_archive`, `path_mapping_test#test_path_stripping_singleplex_worker` | `RemoteModule.beforeCommand` for `--remote_executor`, `--disk_cache` or `--experimental_remote_downloader` on commands other than build/test, or with workers | Remote |
| 20 | `vendor` | 17 | 1 | `bazel_vendor_test#…testBasicVendorTarget`, `#…testVendorConflictRegistryFile`, `#…testVendoringMultipleTimes` | `bazel/bzlmod/VendorManager`, `VendorCommand.exec` | Commands; Repositories |
| 21 | aquery proto output | 16 | 2 | `aquery_test#test_aquery_aspect_on_aspect`, `aquery_test#test_aquery_single_aspect`, `bazel_aquery_test#test_unresolved_symlinks` | `skyframe/actiongraph/v2/StreamedConsumingOutputHandler`: `aquery --output=proto|textproto|streamed_proto`. Only `jsonproto` works. | **NEW** (MF lists only `--skyframe_state` and the dump) |
| 22 | `--test_output=streamed` | 14 | 5 | `bazel_sandboxing_test#test_sandbox_expands_tree_artifacts_in_runfiles_tree`, `test_test#test_interrupt_streamed_output_local`, `ui_test#test_streamed` | `analysis/test/TestStrategy.createStreamedTestOutput` | Execution |
| 23 | JVM-only behaviour | 14 | 6 | `command_profiler_test#test_alloc_profiler_enabled`, `discard_graph_edges_test#test_aspects_with_no_actions_deleted_if_not_top_level`, `server_logging_test#test_log_file_uses_single_line_formatter` | The tests attach `jmap` to the server pid, expect a `Full thread dump` on SIGQUIT, expect `java.log` in JUL single-line format, use `--server_javabase`, or use `--experimental_command_profile` (JFR). `dump_test#test_memory_after_build` (`dump --memory`; `dump` is not ported, MF) is counted here. | **NEW**: needs a ruling (port an equivalent, or accept as a JVM-only exclusion) |
| 24 | `cquery --show_config_fragments` | 14 | 1 | `configured_query_test#test_direct_alias_requirements`, `#test_show_direct_config_fragments`, `#test_show_transitive_config_fragments_through_output_file` | `analysis/RequiredFragmentsUtil.getRuleRequiredFragmentsIfEnabled` | Commands (`cquery`) |
| 25 | HTTP remote cache | 13 | 3 | `bazel_remote_cache_decompression_test#…testDecompressionWorks`, `remote_execution_http_test#test_remote_http_cache_flag`, `remote_execution_test#test_async_upload_works_for_flaky_tests` | `RemoteModule.createCredentials`, `remote/http/HttpCacheClient` | Remote |
| 26 | `unused_inputs_list` | 12 | 5 | `build_without_the_bytes_test#test_remote_download_toplevel_with_non_toplevel_unused_inputs_list`, `starlark_dependency_pruning_test#test_dependency_pruning_scenario`, `starlark_dependency_pruning_test#test_used_shutdown` | Panic `IllegalStateException: Not an input-discovering action` in `crates/blaze_exec/src/actions/action_execution.rs`. Bazel discovers inputs for a Starlark action that has `unused_inputs_list`. | Execution (MF says 4 tests; it is 12) |
| 27 | Sibling repository layout | 12 | 5 | `bazel_test_test#test_run_from_external_repo_sibling_repository_layout`, `cc_integration_test#test_sibling_repository_layout_include_external_repo_output`, `bazel_worker_test#test_compiles_hello_library_using_persistent_javac_sibling_layout` | `--experimental_sibling_repository_layout`: a main-repo package under `external/` is not staged (`cc1plus: fatal error: external/a/a.cc: No such file`), and `../bazel_tools/tools/genrule/genrule-setup.sh` is missing in genrules. Java: `actions/ArtifactRoot`, `cmdline/RepositoryName` (exec paths), `buildtool/SymlinkForest`. | **NEW** |
| 28 | Repo contents cache entries vanish under concurrent processes | 11 | 6 | `java_integration_test#test_building_deploy_jar_dependent_on_deploy_jar`, `runfiles_test#test_foo_runfiles`, `stub_finds_runfiles_test#test_javatest` | Files inside cached external repos (`rules_python+`, `rules_cc+`, `remote_java_tools`) are missing mid-build: `(No such file or directory)`, `BUILD file not found`, `error moving repo @@rules_kotlin+ into the repo contents cache: …/contents/_trash/<uuid>`. All test processes share one repo contents cache under `$REPOSITORY_CACHE`, as in upstream CI. The oracle passes these with the same sharing. Plausible cause: one Blaze process moves or deletes shared cache entries that another live process is still reading. Not yet reproduced in isolation. | **NEW** |
| 29 | Native aspects | 11 | 3 | `configured_query_test#test_invalid_select_no_crash`, `validation_actions_test#test_slow_failing_validation_action_fails_multiple_tests`, `validation_actions_test#test_validation_actions_with_validation_aspect` | `ConfiguredRuleClassProvider.getNativeAspectClassMap` (`ValidateTarget`) | Analysis |
| 30 | Credential helpers | 10 | 3 | `bzlmod_credentials_test#…testCredentialsFromHelper`, `starlark_repository_test#test_auth_from_credential_helper_overrides_starlark`, `starlark_repository_test#test_http_archive_credential_helper_overrides_netrc` | `authandtls/credentialhelper/CredentialHelperProvider` (`GoogleAuthUtils.newCredentialHelperProvider`) | Remote; Repositories |

Families 31 to 81 (functions; MF or **NEW**):

| Functions | Families |
| ---: | --- |
| 10 | C++ compile action templates for tree artifacts, `CcStarlarkInternal.createCppCompileActionTemplate` (MF) |
| 9 | Build event stream for `fetch`/`run`, `BuildEventServiceModule.beforeCommand` (MF); `aquery --skyframe_state` and the dump after a build (MF); Starlark error texts (MF lists "some error texts"; detail below) |
| 8 | `genquery`, `RuleConfiguredTargetFactory.create` (MF) |
| 7 | Path mapping, `StrippingPathMapper.tryCreate` (MF); **NEW** `--modify_execution_info`, `BuildConfigurationValue.modifyExecutionInfo`/`modifiedExecutionInfo`; `dump` (MF); `testing.analysis_test`, `AnalysisTestActionBuilder`, `AnalysisTestTransition` (MF) |
| 6 | `--profile` / `command.profile.gz` missing: `profile_test`, `watchfs_test`, `loading_phase_tests#test_starlark_cpu_profile` (MF) |
| 5 | `help <command>` and `help startup_options`, `BlazeCommandUtils.getUsage`/`expandHelpTopic` (MF); `query --output=graph` and `minrank`, `BlazeQueryEnvironment` Digraph (MF); `query --output=xml` (MF); **NEW** progress messages not printed (`currently loading: <pkg>`, `Executing genrule …; Ns local`, `Loading … packages`). Tests that poll for them wait until their timeout (`sandboxing_test` ×2, `ui_test#test_interleaved_errors_and_progress`). MF lists only the curses bar. **NEW** small items, 1 each: negative `Duration.ofSeconds`, `SkyframeExecutor.getEffectiveStarlarkSemantics` (builtins injection), `TestXmlOutputParser.parseXmlIntoTestResult`, `TopLevelArtifactHelper.getFilesToBuild` for aspect files, `--experimental_exec_config` outside `@_builtins` |
| 4 | **NEW** TLS: rustls rejects the tests' X.509 v1 certificates (`invalid peer certificate: UnsupportedCertVersion`; `remote_execution_tls_test`, `remote_execution_mtls_test`); exclusive tests (MF); **NEW** `query buildfiles()`/`loadfiles()`, `query2/compat/FakeLoadTarget.<init>`; `command.log` (MF); **NEW** spawn-strategy debug lines (`DefaultStrategyImplementations: …`, per-platform `allowed strategies`) missing from `execution_strategies_test` output |
| 3 | **NEW** `--nofetch` not honoured (`fetching is disabled` missing; `starlark_git_repository_test#test_git_repository_and_nofetch` builds); `shadowed_action` (MF) |
| 2 | **NEW** sparse files in tar archives not extracted (`starlark_repository_test#test_sparse_tar`, `bazel_external_repository_test#…testNewHttpTarWithSparseFile`); **NEW** lockfile behaviour (`bazel_lockfile_test#…testInvalidLockfile`, `#…testLockfileRecreationUsesPreviousBuildResults`); include scanning (MF); **NEW** `--unix_digest_hash_attribute_name`, `UnixFileSystem.getFastDigest`; `extra_action` (MF); **NEW** cycle reporters other than the target cycle reporter (`CyclesReporter.maybeReportCycle`, `SkyframeErrorProcessor` cycle in execution); `--query_file` (MF); **NEW** sandbox reuse stashes (`sandboxing_test#test_sandbox_reuse_stashes_sandbox*`); `config` (MF); `canonicalize-flags` (MF) |
| 1 | `help completion` (`HelpCommand.emitCompletionHelp`; `//scripts:bash_completion` runs it at build time, so `external_repo_completion_test` fails to build) (MF); action rewinding (MF); scrubbing (MF); remote worker did not start (`remote_execution_test#test_create_tree_artifact_outputs_remote_cache`; unclear, possibly environment); **NEW** exit code 36 instead of 2 for a repo contents cache inside the main repo; **NEW** `rules_cc` building itself (`cc_libc_top_alias`: advertised provider not returned); `license` (MF); **NEW** `BuildInfoFileWriteAction.execute` (C++ workspace status); **NEW** cc `--experimental_save_feature_state`; disk cache GC (MF); **NEW** `@repo//...` descends into a `bazel-repo` convenience symlink inside the external repo (`external_integration_test#test_integrity_correct`); Wasm (MF); **NEW** `repository_ctx.which`/`execute` (`starlark_repository_test#test_starlark_repository_which_and_execute`); `--build_event_publish_all_actions` (MF); BES upload (MF); **NEW** cquery `config(<target>, <config hash>)`: `Unknown configuration ID`; **NEW** undeclared inclusion after `--discard_analysis_cache` with an external repo package root (skymeld); dynamic execution (MF); implicit outputs by callback (MF) |

### Starlark error texts (9 functions, 5 targets)

starlark-rust's messages differ from Java Starlark's, and the tests match the Java text:

| Blaze prints | Bazel prints (the test's expectation) |
| --- | --- |
| ``Variable `invalidsyntax` not found`` | `name 'invalidsyntax' is not defined` |
| ``Key `True` was not found`` | `key True not found in dictionary` |
| `'struct' value has no field or method 'aar_import'` | `no native function or rule 'aar_import'` |
| `got select for 'implicit outputs of the rule class', want dict` | `got … 'select', want 'dict, NoneType, or function'` |
| `Multiple entries with same key: p=p_v1 and p=p_v2` (a Guava text) | the top-level aspect parameter error |

MF has only "Error locations and some error texts", so this is partly new: the cause is in the
interpreter's error messages, not in the port.

## Causes not in docs/missing-features.md

Ranked by blocked functions. These are the most useful findings of this run.

1. **FailureDetail for non-loading package errors** (18). `DetailedExitCode` of category
   `ExternalRepository` (and `INVALID_OUTPUT_DIRECTORY_MNEMONIC`) panics. The Bzlmod
   resolution errors in these tests panic instead of printing their error.
2. **aquery `--output=proto|textproto|streamed_proto`** (16). `StreamedConsumingOutputHandler`.
3. **JVM-only behaviour** (14). These tests can only pass with JVM features or Blaze
   equivalents of them. They need a decision.
4. **Sibling repository layout** (12). `--experimental_sibling_repository_layout` exec paths
   and staging.
5. **Repo contents cache under concurrent processes** (11, plausible). Entries vanish while
   another process uses them. Needs a focused reproduction (two Blaze processes, one shared
   `--repo_contents_cache`).
6. **`unused_inputs_list` blocks 12, not 4.** MF has this item but undercounts it.
7. **Starlark error texts from starlark-rust** (9). MF lists error texts only in general.
8. **`--modify_execution_info`** (7).
9. **Progress messages not printed** (5, plus every hang in [Timeouts](#timeouts)). This is
   not the curses bar. The plain `currently loading:` and `Executing …; Ns` lines are missing.
10. **TLS with X.509 v1 certificates** (4). rustls `UnsupportedCertVersion`. Java accepts
    them.
11. **`query buildfiles`/`loadfiles`, `FakeLoadTarget`** (4).
12. **Spawn-strategy debug log lines** (4).
13. **`--nofetch`** (3).
14. Two functions each: sparse tar entries; lockfile behaviour (2 cases); xattr digests;
    cycle reporters; sandbox reuse stashes.
15. One function each: repo contents cache inside the main repo gives exit code 36 (Bazel:
    2); `rules_cc` builds itself (advertised provider); `BuildInfoFileWriteAction`; cc feature
    state; `@repo//...` following a convenience symlink; `repository_ctx.which`/`execute`;
    cquery `config()` with a hash; undeclared inclusion after discarding the analysis cache;
    negative `Duration.ofSeconds`; `getEffectiveStarlarkSemantics`; `TestXmlOutputParser`;
    aspect files to build; `--experimental_exec_config`.
16. **An `unported!` panic on a worker thread hangs the process.** It does not exit.
    `client_sigint_test` (both functions): `UiStateTracker.writeProgressBar` panicked on
    thread `skyframe-evaluator` and the client hung until the 300 s timeout. "Fails loudly"
    does not hold off the main thread.

## Timeouts

| Target | Run 1 | Rerun | Hanging function and why |
| --- | --- | --- | --- |
| `integration:client_sigint_test` | timeout (shard 1) | timeout | `test_sigint_batch_mode`: `unported!(UiStateTracker.writeProgressBar)` on a `skyframe-evaluator` thread. The process does not exit. |
| `integration:sandboxing_test` | fail, shards 3 and 4 timeout | same | `test_succeed_when_executor_not_initialized_with_*` poll for `currently loading: pkg`, which Blaze never prints. `test_sandbox_reuse_stashes_sandbox` follows in the same shard. |
| `integration:ui_test` | fail, shard 8 timeout | same | `test_interleaved_errors_and_progress` polls for `Executing genrule //foo:sleep`, which is never printed. Teardown then waits on the output base lock. |
| `bazel:external_integration_test` | fail, shards 1 and 15 timeout | same | `test_deferred_download_*`: the oracle hangs here too (README: flaky). The rerun lost one more function, `test_repository_cache` (pass in run 1, fail in the rerun; flaky). |

Rerun totals for the 4 targets: functions 99 pass, 25 fail (run 1: 100 pass, 24 fail).

## Fully passing targets (62)

Targets the oracle passes that pass in full on Blaze:

`py/bazel:action_temp_test` `py/bazel:bazel_clean_test` `py/bazel:bazel_tools_proto_test`
`py/bazel:cc_import_test` `py/bazel:genrule_test` `py/bazel:query_test`
`shell/bazel/remote:remote_execution_sandboxing_test` `shell/bazel:archive_contents_test`
`shell/bazel:bazel_coverage_cc_head_test_llvm` `shell/bazel:bazel_coverage_cc_released_test_llvm`
`shell/bazel:bazel_docgen_test` `shell/bazel:bazel_execute_testlog`
`shell/bazel:bazel_hermetic_sandboxing_test` `shell/bazel:bazel_java_tools_dist_test`
`shell/bazel:bazel_layering_check_test` `shell/bazel:bazel_localtest_test`
`shell/bazel:bazel_package_loader_test` `shell/bazel:bazel_permissions_test`
`shell/bazel:bazel_sandboxing_cpp_test` `shell/bazel:bazel_strategy_test`
`shell/bazel:bazel_thinlto_obj_dir_test` `shell/bazel:bazel_thinlto_test` `shell/bazel:bazel_tools_test`
`shell/bazel:bazel_wrapper_test` `shell/bazel:build_files_test` `shell/bazel:cc_flags_supplier_test`
`shell/bazel:embedded_tools_deps_test` `shell/bazel:external_patching_test`
`shell/bazel:external_starlark_execute_test` `shell/bazel:external_starlark_load_test`
`shell/bazel:generate_xml_test` `shell/bazel:help_test` `shell/bazel:location_test`
`shell/bazel:new_local_repo_test` `shell/bazel:platforms_test` `shell/bazel:repository_abort_test`
`shell/bazel:resource_compiler_toolchain_test` `shell/bazel:rule_test_test` `shell/bazel:srcs_test`
`shell/bazel:starlark_rule_test` `shell/bazel:tags_propagation_starlark_test`
`shell/bazel:windows_arg_esc_test` `shell/integration:analysis_phase_options_test`
`shell/integration:bash_runfiles_v1_test` `shell/integration:bazel_javabase_test`
`shell/integration:bazel_testjobs_test` `shell/integration:cpp_test`
`shell/integration:force_delete_output_test` `shell/integration:implicit_dependency_reporting_test`
`shell/integration:linux_sandbox_network_test` `shell/integration:linux_sandbox_test`
`shell/integration:loading_phase_posix_tests` `shell/integration:prelude_test`
`shell/integration:process_wrapper_test` `shell/integration:stamping_test`
`shell/integration:starlark_flag_test` `shell/integration:target_pattern_file_test`
`shell/integration:toolchain_transition_test` `shell/integration:unicode_test`
`shell:bashunit_test` `shell:shell_utils_symlinks_test` `shell:shell_utils_test`

(`bazel_coverage_cc_*_llvm` pass because they skip themselves when clang is missing, on the
oracle too.)

## Harness problems

1. **Build failures recorded as `not_run`.** `external_repo_completion_test` failed to build
   (its genrule runs `blaze help completion`). The BEP `targetCompleted` event carries
   `failureDetail` but no `success: false`, because proto3 leaves the default out. `run`'s
   `collect` checks only `success is False`, so it records `not_run` and `compare` counts the
   target as absent, not lost. `run` should also treat a `failureDetail` as `build_fail`.
2. **test.xml that does not parse drops functions.** Four Blaze test.xml files are not
   well-formed (ANSI escape bytes from Blaze's colored output inside the failure text):
   `rc_options_test`, `ui_test` shards 2, 6 and 9. `run`'s `parse_xml` returns nothing for
   them. Their functions go missing, including 7 that test.log shows as `PASSED`.
   `classify` reads test.log for these. `run` could fall back to the
   `PASSED|FAILED: <fn>` lines.
3. **unittest.bash's XML writer overflows.** When one function's log is very large, the perl
   writer fails with `Argument list too long` and the suite stops (51 functions here). This is
   an upstream flaw, and the oracle hits it too. It means a single failure in
   `build_event_stream_test` or `remote_build_event_uploader_test` hides every later function
   in that shard.
4. `--dry-run` creates the results directory for its label.

## Raw results

- Run: `~/.cache/blaze/upstream-test-results/blaze-v0.1.0/` (`results.json`, `logs/`,
  `outer-20261119T113553.log`, `bep-20261119T113553.json`, `memory-events-*.txt`).
- Timeout rerun: `~/.cache/blaze/upstream-test-results/blaze-v0.1.0-timeout-rerun/`.
- Classification: `causes.json` and `causes.txt` (per function: first cause, detail; 169 raw
  causes) and `families.json` and `families.txt` (81 families), all in the run directory.
- Tools: `scripts/ut-analysis/classify`, `scripts/ut-analysis/families` (new; `help`
  prints usage).
- Binary: `~/.cache/blaze/work/v010-blaze`, and the harness's content-addressed copy
  `~/.cache/blaze/upstream-tests/binaries/291d6df9…/bazel`.
