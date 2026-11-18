# Response to the execution review of 2026-11-18

Analysis lead (also owner of blaze_exec, blaze_sky_exec, blaze_buildtool for this fix wave),
2026-11-18. Covers docs/reviews/2026-11-18-exec.md. State described: HEAD `1fdd650` plus the
uncommitted working tree of the analysis fix waves (binary `~/.cache/blaze/int-bin/blaze-fix-6`,
sha256 `257d0bcf7564`).

Several S1s were fixed by the integration lead before this wave; "at HEAD" below means a
sub-implementer re-read the Rust against the Java and found it fixed, not that I re-tested it in
isolation. The oracle scenarios (`scripts/w4/scenarios/`, run by `scripts/w4/probe`) are the
end-to-end check.

## S1

| id | disposition | where / test |
| --- | --- | --- |
| S1-1 cached test results not reported | fixed at HEAD | `notify_on_action_cache_hit` goes through `as_test_runner_action`. re2 `tests` surface 21/21. |
| S1-2 file output that is a directory | fixed at HEAD | `checkForUnsoundDirectoryOutput`; scenario `genrule` step `build //:dir_out` (oracle text). |
| S1-3 output directory creation | fixed at HEAD | `ActionOutputDirectoryHelper` port. No scenario of its own. |
| S1-4 missing-output error texts | fixed at HEAD | scenario `genrule` step `build //:missing_out`. The "is not a symlink" branch stays behind `unported!("FileArtifactValue.createForUnresolvedSymlink")`. |
| S1-5 missing source inputs | FIXED | `completion_function.rs` `handle_source_file_error` (with `recordExecutionError`); the action-level `ERROR: <loc> Action ... failed: N input file(s) do not exist` is `SkyframeErrorProcessor.rethrow`'s, ported in `blaze_runtime/src/commands/build.rs` (`process_execution_errors`, `rethrow`) with `AlreadyReportedActionExecutionException` in blaze_exec. Scenario `exec-source-errors`. |
| S1-6 shared actions | fixed at HEAD | `transform_for_shared_action`. No scenario (needs aspects). |

## S2

| id | disposition | where / test |
| --- | --- | --- |
| S2-1 stat fallback for inputs | FIXED | `ActionInputMetadata` is a port of `ActionInputMap` plus the three Java providers; `RunfilesArtifactValue` carries files, trees and metadata; unknown derived inputs return `None`; new `blaze_exec/src/exec/single_build_file_cache.rs`. 19 `action_input_map_tests`. One exception, loud when it matters: during input discovery a derived input outside the map is stat'ed, because the evaluator cannot be asked for a done value (request to blaze_skyframe). |
| S2-2 conflicting actions merged | FIXED | analysis side: `ArtifactConflictFinder` (analysis response S2-1). Execution side: `execute_action` panics "Cannot share ..." when the second action is not shareable (`Actions.canBeShared`). Scenario `top-conflicts`. |
| S2-3 tree artifact values | FIXED | ports of `TreeArtifactValue.visitTree`, `constructTreeArtifactValueFromFilesystem`; symlinks followed; Java's two error texts; every visited entry chmod'ed (directories 0555). Tree artifacts work with the action cache. Scenario `exec-tree` (stat of tree directories, cache hit on the second build, dangling symlink); 10 `visit_tree_test`s. |
| S2-4 action output directory | fixed at HEAD | `getActionTempsDirectory`. |
| S2-5 flags that do nothing | FIXED | `--subcommands`, `--output_filter`, `--check_up_to_date`, `--experimental_writable_outputs` wired at HEAD; this wave: `ExecutionToolConfig` carries the command's `RemoteOptions`, `DynamicExecutionOptions`, `WorkerOptions` (blaze_exec's guards were unreachable behind hard-coded defaults). `--archived_tree_artifact_mnemonics_filter` set to a non-default value is now `unported!` (`blaze_runtime/src/unported_options.rs`). |
| S2-6 sandbox seams | fixed at HEAD | `package_path_entries`, repo contents cache path, `SandboxModule.afterCommand`. |
| S2-7 compact execution log | FIXED | parameter files are recognised by type (`ParamFileActionInput`); runfiles symlink sets keep their nested-set nodes. A sub-set shared by two runfiles trees is still logged once per tree (Java: once). |
| S2-8 StringEncoding identity | FIXED | 15 sites go through `blaze_util::string_encoding`; no `PORT: StringEncoding identity` remains in the three crates. |

## S3 / S4

- S3-1 (tree artifacts with the action cache) fixed with S2-3. S3-2 to S3-14 unchanged and loud.
- S4-1 (`lookup_input` quadratic) and S4-2 (`create_for_testing` in production paths) fixed;
  S4-3 unchanged.
- blaze_sky_exec's 11 clippy warnings: fixed; `clippy --all-targets -D warnings` is clean on the
  twelve crates of this wave.

## Requests from other owners, done here

- Skyframe: every SkyFunction of these crates returns `Ok(None)` on a missing dep and tolerates
  the bubbling re-run (`BuildDriverFunction` posts each event once per key);
  `compute_artifact_nested_set` is a port of `ArtifactNestedSetFunction.java:94-140`; the
  catastrophic bit comes from `is_catastrophe()`.
- Actions: `ctx.actions.write(content = args)` with a directory uses
  `new_deterministic_writer_with_input_metadata_provider` (scenario `exec-write-args-dir`).

## Open

- `ActionExecutionException` root causes are `ActionFailed` only: the `LabelCause`s of missing
  inputs do not reach BEP.
- `addOutputToMetrics` (BEP artifact metrics) and action templates in `ArtifactFunction`.
- Critical path (`Critical Path:` in the summary line was a wall time, now removed; BEP
  `buildToolLogs` lacks the entry).

## Evidence for both responses

Tree: HEAD `1fdd650` + uncommitted working tree (dirty); binary
`~/.cache/blaze/int-bin/blaze-fix-6` (sha256 `257d0bcf7564`), built from that tree.

Referee, full run of both projects, label `blaze-fix-6` against `oracle-9.3.2`
(`scripts/oracle/scoreboard blaze-fix-6 --project brotli --project re2`, referee sha256
`26e4400ec7d2`):

| project | matching / compared | allowed | failed | extra | surfaces MATCH | DIFF |
| --- | --- | --- | --- | --- | --- | --- |
| brotli | 997 / 1001 | 3 | 4 | 0 | 12 of 13 | bep (4) |
| re2 | 12853 / 12857 | 55 | 4 | 1 | 11 of 13 | bep (4), build_stderr (1 extra) |

The order items match on both projects: `cquery_label_kind` `__order__ deps`, `cquery_jsonproto`
`__order__ results` / `configurations`, and every `aquery_actions` `__order__` item (action
order, action ids, depSetOfFiles, artifacts, pathFragments, ...). Before this wave
(`blaze-int`): the order was not compared and differed on every line.

What still differs there, and whose it is:
- bep, both projects: `buildToolLogs` lacks `command.profile.gz` and `critical path` (mine,
  deferred); `--install_md5` and `--restart_reason=no_daemon` in three command-line events
  (blaze_client).
- re2 build_stderr: the `compatibility_level` MODULE.bazel warning is printed by every Blaze
  command; the oracle prints it once per server (bzlmod / server model).

Scenario probes (`scripts/w4/probe compare <scenario>` with that binary, each run cold):
27 scenarios, 20 identical to the oracle transcript; `scripts/w4/flags-probe compare`: 71 of 71
cases identical.

| scenario | result | what differs |
| --- | --- | --- |
| cc-hash, cc-tags, ctx-args-snapshot, ctx-build-file-path, ctx-immutable, ctx-run-shell-order, exec-source-errors, exec-tree, exec-write-args-dir, flags-transitions, genrule, ids-depset-freeze, query-errors, query-map-each, query-order, query-output-file, query-rdeps, top-analysis-errors, top-bad-platform, ui-exec-errors | MATCH | - |
| ctx-errors | DIFF | steps 2-3: `getattr(ctx.fragments, "cpp", None)` / `hasattr` on an undeclared fragment (needs the vendored attribute hook) |
| ctx-toolchains-string | DIFF | step 6: toolchain key converted in a helper .bzl (needs the evaluator in the index operator) |
| graph-order | DIFF | one cquery line out of place (exec deps of the first requester of the exec platform) |
| incompat-config-setting, incompat-toplevel | DIFF | one line each (`Use --verbose_failures ...`, `Target //pkg:bad failed to build`) that depends on which targets Bazel had announced as analyzed when a fail-fast analysis stopped |
| top-conflicts | DIFF | `Use --verbose_failures ...` missing after a fail-fast conflict (4 steps, open); which of two conflicting targets the WARNING names (3 steps, racy on the oracle); one sorted-line position |
| ui-build-results | DIFF | step 32: `[unix_jni.cc:382] ` prefix of a native I/O error (blaze_vfs) |

Unit tests (`scripts/cargo-capped test`, the twelve crates of this wave: blaze_analysis,
blaze_rules, blaze_sky_analysis, blaze_sky_top, blaze_cquery, blaze_build_config, blaze_config,
blaze_runtime, blaze_sky_exec, blaze_buildtool, blaze_exec, blaze_bep): 1026 passed, 0 failed,
839 ignored (most ignored tests are Java test names that need a `BuildViewTestCase`). The three
crates touched outside (blaze_cmdline, blaze_sky_loading, blaze_starlark): 694 passed, 0 failed,
126 ignored. `clippy --all-targets -D warnings` and `rustfmt --check`: clean on the twelve crates.

workerd (stress project): `cquery 'deps(//src/workerd/util:entropy)' --output=label_kind` prints
1,074 lines, all present in the oracle's `deps(//src/workerd/server:workerd)` record, with the
oracle's target configuration `13e714b` and exec configuration `1d8533e`. The full
`deps(//src/workerd/server:workerd)` now stops at a loud `unported!` for aspects
(`AspectResolutionHelpers.computePropagatingAspects`, protobuf's upb aspect); the earlier
blockers (Starlark flags, flag scoping, `HeaderInfo` hashing, genrule, analysis-error panic,
`ctx.build_file_path`) are gone.
