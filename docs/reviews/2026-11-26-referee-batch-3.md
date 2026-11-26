# Referee batch 3 (2026-11-26): implementer's report for the auditor

Tree: working tree on HEAD `66174e8c19f1` (HEAD moved from 374c631 to a9097e0 and 66174e8 while
this batch ran; other agents commit). Nothing here is committed. Changed files are listed with
their sha256 at the end.

- referee sha256 (`normalise.py`, `surfaces.py`, `oracle_compare.py`, `allow.json`):
  `c1407ce7f1c92f921702b29a7e7e8862ad72650e2fe739c359e9c5111a60856c`
- extractor sha256 (`normalise.py`, `surfaces.py`):
  `c215d2d7087c8ff0b2094a42201fd889a6a2e33a5709b8fba5d8413daa993f97`
- Verification binary: `~/.cache/blaze/work/root-runs/blaze-374c631` (checkpoint 38), sha256
  `a96a5ec8b022…`.
- Unit tests: `python3 -I -m unittest discover -s scripts/oracle`: 148 run, 148 passed, 0 failed,
  0 skipped (113 before; 35 new). Old-against-new check below.

No Rust and no `scripts/gate`, `corpus/projects.json`, `corpus/oracle/` or other labels'
recordings were changed. Bazel and Blaze ran only after `root-suite-5f68157` became inactive,
one recording at a time, each in `systemd-run --user --collect --wait -p MemoryMax=12G -p
MemorySwapMax=0 --slice=app-t3code-agents-jobs.slice` with `record --no-scope`.

## Decisions I took (the task said not to ask)

1. **Staging, not in-place renormalisation.** Items 1 and 3 change the BEP items of every
   recording, the reference's included, whose hashes are anchored in the committed
   `corpus/oracle/<project>.json`. Renormalising the shared `oracle-9.3.2` would break every
   other agent's comparison until root commits new summaries, and the task forbids touching
   other labels. New: `BLAZE_ORACLE_STAGE=<dir>` (`oracle_common.py`) moves the records root,
   the reference summaries and `scoreboard --write` into `<dir>`. Every report made that way
   carries a `staged:` warning, and so does the rescored base. Stage:
   `~/.cache/blaze/work/referee-3/stage` holds copies of `oracle-9.3.2` (brotli, re2; workerd
   with `raw/` hard-linked), `-rerun`, `-check` and `blaze-ef66f55e904e`, plus symlinks to my two
   new labels.
2. **`--max_idle_secs` removed on both sides.** The task wants it as a startup option in daemon
   mode, but the reference was recorded without it, so it would show in three BEP events. The
   normaliser drops it (and the recorder's `--client_env=BLAZE_SERVER_MODE=daemon` and the
   `max_Uidle_Usecs` pair of `--option_sources`, found in the first daemon run) from every
   recording, the reference's default `--max_idle_secs=10800` included. This is a recorder
   mask, not a cold/warm mask. Cost: Blaze's default of 10800 is no longer compared.
3. **Cold/warm differences warn but do not fail.** Failing a surface whose commands ran in
   different server states (audit 3, B1 item 2) would fail every one-process recording and
   break the ratchet. They are printed and stored instead.
4. **`shutdown` is not added to `exit_codes`.** That would add an item that is EXTRA against the
   reference. Its exit code and any survivor go into the manifest and become a warning.
5. **One allow-list entry made mode-dependent** (item 4 asks to list entries that exist only
   because of cold/warm). Evidence below.

## Item 1: `--install_md5`

- `surfaces.py`: `MASKED_OPTION_VALUES`, `MASKED_OPTION_RE`, `mask_option_values()`, called in
  `normalise_bep`. The value becomes `<INSTALL_MD5>` in option messages (`optionValue`,
  `combinedForm`) and in strings. The option itself is still compared.
- Tests (`test_surfaces.InstallMd5Test`): `test_install_md5_value_is_masked`,
  `test_string_form_is_masked` (fail on the old code); `test_the_option_itself_is_still_compared`
  (guard).
- Effect: the `canonical` command-line item of brotli and re2 no longer differs in
  `install_md5`. In one-process mode it still differs in `--restart_reason` (item 2), so no row
  changes.

## Item 2: `--restart_reason`

Not masked. `test_surfaces.InstallMd5Test.test_restart_reason_is_not_masked` (guard) checks
that it stays. Observed: in one-process mode (`r3-374c631`) Blaze sends
`--restart_reason=no_daemon` on the build, in `unstructuredCommandLine` and both structured
command lines (3 items per project DIFF). In daemon mode (`r3d-374c631`) the build runs on the
warm server and sends no `--restart_reason`, as the oracle's warm build does. All three
command-line items match in both projects with no mask beyond decision 2.

## Item 3: `buildMetrics`

- `surfaces.py`: `BEP_METRICS_DROP` replaced by `BEP_METRICS_PRESENCE` (memory, timing,
  network, worker, workerPool, dynamicExecution), `ARTIFACT_SIZE_MASKED`, `presence()`, and
  `normalise_build_metrics()`, called from `normalise_bep` after the generic pass. Behaviour:
  - Compared as before (wall-clock keys still dropped): `actionSummary`, `targetMetrics`,
    `packageMetrics`, `buildGraphMetrics`, `remoteAnalysisCacheStatistics`.
  - Newly compared: `cumulativeMetrics` (numAnalyses, numBuilds) and `artifactMetrics` counts.
  - Sizes: `sourceArtifactsRead.sizeInBytes` is compared; it is the same in all three oracle
    runs (brotli 2328125, re2 263555297). The `sizeInBytes` of `outputArtifactsSeen`,
    `topLevelArtifacts` and `outputArtifactsFromActionCache` become `<path-dependent>`. They
    depend on the output root path (runfiles manifests name absolute paths) and on test timing
    (test logs and test.xml). The three oracle runs differ there: brotli outputArtifactsSeen
    7671064 / 7671088 / 7671088; re2 89006013 / 89050518 / 89050526, where `-rerun` and
    `-check` have labels of equal length.
  - Presence only: the six machine groups keep their field names; scalars become
    `<measured>`, lists `<list>`. A missing group or field is a DIFF. All three oracle runs
    have the same shape. Proto3 JSON leaves out zero values, so a field that is legitimately 0
    would show as absent; none of the timing, network or GC fields is 0 in the oracle runs.
  - `evaluatedValues` stays excluded by name through `build-metrics-volatile-counters`.
- Tests (`test_surfaces.BuildMetricsTest`): `test_empty_artifact_metrics_is_a_difference` (the
  review's case), `test_artifact_counts_are_compared`, `test_path_dependent_sizes_are_masked`,
  `test_machine_measurements_are_compared_by_presence`, `test_cumulative_metrics_are_compared`,
  `test_volatile_counters_stay_allowed_by_name_and_nothing_else` (all fail on the old code);
  `test_counters_compared_as_before_and_wall_clock_keys_dropped` (guard).

## Item 4: server mode

- `record`: `--blaze-server inprocess|daemon` (sets `BLAZE_SERVER_MODE` through
  `clean_env()`), `--max-idle-secs N` (default 60 with daemon, `DAEMON_MAX_IDLE_SECS`).
  `live_servers()` and `server_pid()`: a pid file counts only if its live process's
  `/proc/<pid>/cmdline` names the output user root, so a reused pid is never signalled.
  `kill_servers()` returns what it stopped. It runs before the root is wiped
  (`server.stopped_before`) and after the final `shutdown`, which waits up to 10 s
  (`server.alive_after_shutdown`, plus a printed warning). Each command records
  `server: {pid_before, pid_after, start_line, state}`; the recording records
  `server: {blaze_server_mode, max_idle_secs, stopped_before, shutdown_exit_code,
  alive_after_shutdown}`.
- `oracle_common.py`: `blaze_server_mode()`, `client_started_server()`, `server_state()`
  (warm = the same server pid alive before and after and no start line), `server_states()`.
  For recordings older than this batch, `server_states()` reads `raw/<cmd>.stderr`: cold when
  the client printed "Starting local … server", marked `stderr`.
- `oracle_compare.py`: `provenance()` adds `server_mode`, `server_states`, `shutdown`.
  `AllowList.match(..., mode)`, `compare_surface(..., mode_b)`, and `compare_labels` passes the
  candidate's mode. `oracle_report.py`: `server_text()`, `server_lines()` (printed by
  `compare`), `server_state_lines()`, and the staged warning in `warning_lines()`.
- `normalise.py`: `SERVER_START_RE`, `stderr_summary(..., keep_server_start=False)`.
  `surfaces.s_build_stderr` keeps the start line only when `blaze_server_mode == "daemon"`.
- `surfaces.py`: `RECORDER_STARTUP_OPTIONS`, `RECORDER_CLIENT_ENV`,
  `RECORDER_OPTION_SOURCE_KEYS`, `_is_recorder_option()`, `_strip_option_sources()`,
  `drop_recorder_options()` (decision 2). Bazel's client sends an empty `StartupOption` after
  every original startup option (`blaze.cc` `BlazeServer::Communicate` calls
  `request.add_startup_options()` twice), which is the `{}` in the BEP. That entry is removed
  together with the option. Blaze's daemon BEP has the same shape:
  `{"combinedForm": "--max_idle_secs=60"}, {}`.
- Masks and allow-list entries that exist only because of the cold/warm difference, and what
  daemon mode does with each:
  1. Blaze's server-start line in `build_stderr` (dropped because it has no prefix): kept and
     compared in daemon mode. Dropped as before otherwise.
  2. Four `only_fields` of `build-metrics-volatile-counters`: `builtValues`, `changedValues`,
     `cleanedValues`, `postInvocationSkyframeNodeCount`. In the three oracle recordings these
     are equal for both projects (per SkyFunction and in order; node count 7305 and 23543).
     Only the `evaluatedValues` counts, the `dirtiedValues` order and the re2 action-cache
     `sizeInBytes` vary. Under the new daemon list every oracle self-comparison's
     `buildMetrics` paths are still excused (6 of 6 checked). New entry field
     `only_fields_in_daemon_mode` (evaluated/dirtied, sizeInBytes) with `daemon_mode_reason`.
     Nothing is deleted, and one-process candidates and the oracle self-checks use
     `only_fields` as before.
  3. `runfiles-manifest-action-key` is about paths, not the server, and stays as it is.
  4. `--restart_reason` and the re2 `compatibility_level` warning were never masked. In daemon
     mode both match.
- Tests: `test_record.ServerModeTest` (a stand-in client with a real "server" process):
  `test_daemon_mode_runs_the_sequence_on_one_server`, `test_one_process_mode_is_unchanged`,
  `test_max_idle_secs_can_be_set`,
  `test_a_server_of_an_earlier_run_is_stopped_first_and_a_stranger_is_left_alone`,
  `test_a_server_alive_after_shutdown_is_reported_and_stopped`.
  `test_compare.ServerStateTest` (5), `test_compare.DaemonModeAllowTest` (4: 2 fail on the old
  code, 2 guards), `test_compare.StageTest` (2).
  `test_surfaces.BuildStderrModeTest` (2), `test_surfaces.RecorderStartupOptionTest` (4).
  `test_normalise.ServerStartLineTest` (2).
- Observed on the real binary (`r3d-374c631`): one server per project (brotli pid 416177),
  `mod_graph` cold, the other 9 commands warm, `shutdown` exit 0, `alive_after_shutdown` [],
  `stopped_before` []. One-process `r3-374c631`: all 10 commands cold (start line every time,
  no pid after). The binary-identity restart did not come into play (fresh root, one binary).

## Item 5: the ratchet

Oracle against itself under this referee (stage; renormalised copies of all three):
`compare oracle-9.3.2 oracle-9.3.2-rerun --project brotli --project re2`, the same with
`-check`, and `-rerun` vs `-check`. Each exits 0 with `100% MATCH`, 13,916 = 13,858 matching +
58 allowed (runfiles 56, volatile counters 2), the same as batch 2. Renormalising the reference
changes only `items/bep.json` and `norm/bep.json` of brotli and re2. workerd's items are
byte-identical under the new extractor (it has no BEP), so its summary changes only
`extractor_sha256`. Staged summaries (what root's in-place renormalisation must reproduce):
brotli.json `bb73e232…`, re2.json `76272118…`, workerd.json `6bc70b00…` in
`~/.cache/blaze/work/referee-3/stage/corpus-oracle/`.

Rescored base `corpus/scores/blaze-ef66f55e904e.referee3.json` (and `.md`): from the existing
recordings, renormalised in the stage, `--allow-dirty` with the root ruling of 2026-11-19
restated. Against `referee2.json` (row by row, including `matching_keys`):

| row | referee 2 | referee 3 | why |
|---|---|---|---|
| brotli/bep | DIFF, matching 37, allowed 1, failed 4 | DIFF, matching 37, allowed 0, failed 5 | blind spot |
| re2/bep | DIFF, matching 109, allowed 1, failed 4 | DIFF, matching 109, allowed 0, failed 5 | blind spot |
| the other 37 rows | — | unchanged | — |

The one item that moved in each project is `{"buildMetrics":{}}`. It goes from allowed to
failed; it was never matching. No matching key was lost or gained, and the totals are matching
13,850 → 13,850, allowed 58 → 56, failed 145,080 → 145,082. The new failing paths are all
fields the old referee dropped without looking:
`artifactMetrics.{sourceArtifactsRead, outputArtifactsSeen, outputArtifactsFromActionCache,
topLevelArtifacts}` (ef66 printed `artifactMetrics: {}`), `memoryMetrics.garbageMetrics`,
`networkMetrics` (absent at ef66), and `timingMetrics.{cpuTimeInMs, wallTimeInMs,
executionPhaseTimeInMs, actionsExecutionStartInMs, criticalPathTime}` (ef66 printed only
`analysisPhaseTimeInMs`). `compare-scores` refuses `referee2` against `referee3` (different
referees), as it should.

## Verification (binary blaze-374c631, scored in the stage against the rescored base)

| label | mode | matching (brotli+re2) | `compare-scores` vs referee3 base |
|---|---|---|---|
| `r3-374c631` | one process per command | 13,852 of 13,860 judged | PASS, rows 39, gained 2, regressed 0 |
| `r3d-374c631` | daemon | 13,858 of 13,860 judged | PASS, rows 39, gained 2, regressed 0 |

- `r3`: brotli/bep 37 → 38 and re2/bep 109 → 110 (gained). Still failing: `buildMetrics`
  (garbageMetrics, and the cold-server counters that only one-process mode excuses), the three
  command-line items (`--restart_reason=no_daemon`), and the re2 build_stderr EXTRA
  `compatibility_level` warning (cold build).
- `r3d`: brotli/bep 37 → 41, re2/bep 109 → 113 (gained), re2/build_stderr DIFF → MATCH (the
  EXTRA line is gone: warm build). Status: 2 surfaces DIFF instead of 3; 24 of 26 brotli+re2
  surfaces MATCH.
- Both labels: `blaze_repo` commit a9097e0b397a, dirty, and the uncommitted state differs
  between projects, because other agents changed the tree during the runs. The binary is not in
  this checkout's `target/`. Verification labels only; not acceptable as a score (written with
  `--allow-dirty`, into the stage).

DIFF items of the daemon-mode recording (`r3d-374c631`), every one:

| project | item | cause |
|---|---|---|
| brotli | `{"buildMetrics":{}}` | `memoryMetrics.garbageMetrics` absent (Blaze prints `memoryMetrics: {}`; the oracle lists JVM GC pools), and two daemon-unexcused paths: `builtValues` and `postInvocationSkyframeNodeCount` (below) |
| re2 | `{"buildMetrics":{}}` | the same three paths |

Every other item of both projects matches or is allowed (runfiles action keys, 56).

## Findings the daemon-mode recording shows (not fixed; Rust untouched)

1. **No GC metrics** (S1 by the review's standard, or a declared deviation to rule, like D15):
   `buildMetrics.memoryMetrics` is empty. The oracle reports `garbageMetrics` (JVM pools:
   G1 Eden Space, G1 Survivor Space, …). It also shows in one-process mode.
2. **Warm build rebuilds what `mod graph` built** (S1 for the counters):
   `builtValues` total brotli 7432 against the oracle's 4954, re2 23642 against 21190.
   SkyFunctions built only by Blaze in the build: BAZEL_DEP_GRAPH 1, BAZEL_MODULE_INSPECTION 1,
   CLIENT_ENVIRONMENT_VARIABLE 1, REPO_SPEC 27/29, SINGLE_EXTENSION, SINGLE_EXTENSION_EVAL,
   SINGLE_EXTENSION_USAGES 26 each, STARLARK_BUILTINS 1, YANKED_VERSIONS 27/29. Built more
   often: BZL_LOAD 787 vs 383 (brotli), MODULE_FILE 314 vs 156, FILE 2041 vs 1296, FILE_STATE
   1734 vs 988, REPO_FILE / REPOSITORY_MAPPING 74 vs 49, … `dirtiedValues`, `changedValues` and
   `cleanedValues` match the oracle exactly (multiset). So Blaze dirties the same nodes but
   evaluates module-resolution and .bzl nodes again that Bazel's warm server already holds from
   `mod graph`. This is consistent with decisions 2026-11-26 (graph kept between commands:
   package I3); the cause is not established here.
3. **Node count after the build**: `postInvocationSkyframeNodeCount` brotli 7303 vs 7305, re2
   23516 vs 23543 (the oracle is constant across its three runs).
4. **(One-process only, for completeness)**: `--restart_reason=no_daemon` and the repeated re2
   `compatibility_level` warning. Both disappear on the server, as audit 3 B1 predicted.

## Old code against the new tests

The HEAD versions of the ten non-test files (`normalise.py`, `surfaces.py`,
`oracle_common.py`, `oracle_compare.py`, `oracle_report.py`, `record`, `compare`,
`scoreboard`, `allow.json`, `fixture_records.py`; copied to
`~/.cache/blaze/work/referee-3/oldcode/`) were run with the new test files. 111 of the 113 old
tests pass. The other two error only because that directory is outside the repository:
`test_this_checkout` and `test_the_committed_workerd_summary_declares_its_splice`. Of the 35 new tests, 27 fail or error
on the old code. The other 8 are guards that must pass before and after:
`test_dropped_by_default`, `test_the_option_itself_is_still_compared`,
`test_restart_reason_is_not_masked`, `test_other_startup_options_still_differ`,
`test_counters_compared_as_before_and_wall_clock_keys_dropped`, `test_dropped_without_a_daemon`,
`test_one_process_candidate_scores_as_before`,
`test_daemon_candidate_is_excused_where_the_oracle_varies`. Listing:
`~/.cache/blaze/work/referee-3/logs/old-vs-new-tests.txt`.

## For root, before acceptance

- After the audit: renormalise the shared `oracle-9.3.2` (brotli, re2, workerd: `--summary`),
  `-rerun` and `-check` in place. The summaries must equal the staged ones above. Commit
  `corpus/oracle/*.json` with the referee and the rescored base.
- Other tools import `surfaces.normalise_bep`: `scripts/bep-matrix`, `scripts/bep-compare`,
  `scripts/textproto/compare`. They will now see `buildMetrics` compared, `install_md5` masked
  and the recorder options removed. `bep-matrix` mentions `BEP_METRICS_DROP` in a comment only.
  Not run here.
- `record` without `--no-scope` still uses `systemd-run --user --scope`, which by default lands
  outside the agents' slice. I ran it with `--no-scope` inside a `--slice` service. Not changed
  (not in scope).
- Disk: 86 GB free on /home after my cleanup (under the 150 GB line).
- Left on disk: `~/.cache/blaze/work/referee-3/` (stage 3.6 GB apparent, workerd raw
  hard-linked; reports; logs; oldcode) and recordings `r3-374c631`, `r3d-374c631`. Deleted: the
  two output roots, the two workspace copies, and a temporary copy of workerd items.

## sha256 of the changed files

| file | sha256 |
|---|---|
| scripts/oracle/surfaces.py | 4a0bc77d0d69536d28825d53fa730c0beade138e3c1ba27acd0bc515307446f1 |
| scripts/oracle/normalise.py | bf5e4572a289851ff11e6faab7bada2c1576bcb6ee59ea5560c76d3576770267 |
| scripts/oracle/oracle_common.py | 8f7491482e3f5d9a79ad6f3dc1ddb51a4046dc6eb4f4c395d0a2ada641da9f58 |
| scripts/oracle/oracle_compare.py | c49c1ad941ddc4baa5b748fbce87ec77dae10c3eb5c49252c81d2affad37260a |
| scripts/oracle/oracle_report.py | d924e594a0ca256bbd4de0309f06567f92ceec687abce8d5dffb67a5f0f87710 |
| scripts/oracle/record | 420bfee0368130fae621a4b116529db4d55a156e54f46eca4e07f1dabb48eb08 |
| scripts/oracle/compare | aebb9b90314ac72d359d3d90f94801b9f2bc26e419f50d99d859cabebf67bc24 |
| scripts/oracle/allow.json | ade2884c39b212b66fda6fbd8d6abb67d132c207bc08f86accba6d1c95690b81 |
| scripts/oracle/README.md | feac17edcea7d9f125fa39a19c8e4b5b7ed8ce7a6c03eefd4212eaa0891dd604 |
| scripts/oracle/test_surfaces.py | 938a87f4968c53f404ac967c9dc203e4e5a998b5230ef3d7782637fbf7362b2d |
| scripts/oracle/test_normalise.py | 693d7fe78533cff8647f4ded0067d59a50d53c6752d7b663d9d5fa8f6699bc5d |
| scripts/oracle/test_record.py | 7a974d52c4e3fbe5438296c879adb59237fa014f63b8c815cc7440b675825286 |
| scripts/oracle/test_compare.py | 6ce379e45e1effc3382f54031e3719970825fb57e15f671c5f156e476851ae9f |
| corpus/scores/blaze-ef66f55e904e.referee3.json | 3e69088a86a540c7f7548feaed0fe89a034e469c357570b60f702545dba85581 |
| corpus/scores/blaze-ef66f55e904e.referee3.md | ab2f7cd370bb08ff7d8ccb79e95547f16b85bc1a52f5f71932f31a2e8c5e1c3c |

This report (`docs/reviews/2026-11-26-referee-batch-3.md`) cannot list its own hash; it is
printed in the final answer.
