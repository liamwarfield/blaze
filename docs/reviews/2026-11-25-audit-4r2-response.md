# Response to audit 4 round 2 (`audit-4r2`): comparison tools, fix 2

Date 2026-11-25. Implementer (Opus 5.5). Answers `docs/reviews/2026-11-25-audit-4r2.md` (batch
STOP on B1 to B5; `scripts/server` STOP on SV-1 to SV-5). No Rust, no Cargo, no subagent, no
commit. Working tree on top of HEAD `aa559209715b` (HEAD moved from `872cc19` during the work;
no commit in between touched the files below). Not mine and not touched: the working-tree edits
of `scripts/remote/*` and `scripts/workerd/bench`, the untracked `scripts/w4/scenarios/*/BLAZE-*.txt`
and `scripts/config/probe/{fs,third_party}`.

Decisions taken without asking (AGENTS.md):

- `w4/probe` keeps ORACLE.txt in the scenario directory and writes its key beside it
  (`ORACLE.key.json`), as the audit's fix says. Every ORACLE.txt therefore had to be recorded
  again (the old ones have no key); the rerun did that with the pinned oracle. Equal failures of
  probe steps are not declared (a declaration taken from the oracle's own record would say
  nothing about intent), so they are BOTH-FAILED.
- The server tools keep their own launcher (the tool in one memory-capped unit, the client's
  true wait status), which the audit accepted as sound; they now judge through
  `compare_common` (classes, `judge`, `Report`) and their selftests use its fakes. The
  tripwire for these tools means "started outside the tool's own unit": the fakes get their
  token only from a process that runs inside that unit.
- `interop-client`'s `scenarios` case declares no failure either (same reason as the probe).
- `bep-matrix` compares `buildMetrics` as the referee does (only the allow list's volatile
  counters masked) rather than keeping the reduced mask and calling it IDENTICAL-REDUCED.
- `U1`, `U2` are in `scripts/upstream-tests/ut_common.py`, which this task forbids: not done.
  The probe's own unlocked-process check got the U1/U2 fixes (interpreter options skipped; a
  registered pid counts only while its recorded start time matches).

## Findings, change by change

"Selftest" names the check that now covers the fix; every selftest drives the tool's real
command line against fake binaries and fails if a fake starts outside the launcher (tripwire).

### Blocking

**B1, `scripts/w4/probe` (rewritten onto the module).**
- F4: `Run.exec` runs every command through `cc.run` (scope unit, supervisor, OOM detection
  including a server's OOM in an earlier command's scope, 3600 s timeout that SIGKILLs the
  unit). The transcript's first line is the termination (`exit N`, `signal NAME`, `oom-kill`,
  `timeout`, `unrecorded`). The `shutdown` terminations are report auxiliaries (an abnormal one
  fails the run).
- F3: `record`/`record-all` check the pin (`cc.require_oracle`), remove `ORACLE.key.json`
  first, write ORACLE.txt, then `cc.write_key` with the oracle sha256, the tool sha256, the
  startup/command options, the clean environment, `tree_digest(ws/)`, the sha256 of COMMANDS
  and the digest of ORACLE.txt (`oracle_key`, `record`). `compare`/`compare-all`/`recount`
  refuse (exit 2) a missing or stale key or a changed ORACLE.txt (`check_oracle_record`). The
  binary under test goes through `cc.blaze_identity` (the oracle is refused).
- Classes: `judge_step` (full view equal: `cc.judge`; equal only in the masked view: KNOWN-GAP
  naming the mask classes; only in the messages view: IDENTICAL-REDUCED; else DIFFERENT;
  equal failures BOTH-FAILED; `unported!` NOT-PORTED). Exit = `Report.exit_code()`. The
  three-view table and `step_class`/`count`/`print_table` stay (the gate's unit test still
  passes: `scripts/gate/test_evidence_tools.py ProbeStepClassTest`, 2 tests OK).
- `recount <sha12> --binary B`: only transcripts whose capture (`cc.write_capture`, written by
  `compare` next to each transcript) names B and whose digest is unchanged are judged, others
  SKIPPED; `reclassify_only` makes exit 0 impossible; the report names the trees the captures
  recorded, not only the present HEAD.
- Selftest (`probe selftest`, 27 checks): the nine generic scenarios through `record` +
  `compare`; masked view → KNOWN-GAP; messages view → IDENTICAL-REDUCED; wrong pin refused by
  `record` and `compare`; oracle as `--binary` refused; changed ORACLE.txt, changed `ws/` and a
  missing key refused; `compare-all`; `recount` never exits 0, names the recorded trees, skips
  another binary's and a changed transcript; the lock makes a second run wait; an unlocked
  probe process started as `python3 -B <probe>` makes a run refuse; tripwire.

**B2, `scripts/textproto/compare` `cmd_run`, `anchored`, `cmd_check`, `cmd_bep_e2e`.**
`run` removes `provenance.json`, `exit_codes.json`, `binary.txt` and every file it writes
(`RUN_FILES`) before the first step; `provenance.json` is written last (tmp + rename) with the
sha256 of every file of the directory (`dir_files`) and the terminations of `mod graph`, `info`,
`shutdown`. `check` refuses (exit 2) a directory whose files changed since; a directory without
digests gets a SKIPPED `files-anchored` row (cannot pass). `bep-e2e` refuses either directory
unless every file is anchored. Selftest: "B2: no earlier provenance, exit code or output is
present while a rewrite runs" (a fake that looks for them during the first step), "provenance
holds the files' digests", "check refuses a directory whose files changed", "bep-e2e refuses a
reference whose files changed", "a directory without digests gets a SKIPPED files-anchored row".

**B3, `scripts/lib/compare_common.py` `B1_GAP_RULES`.** The `b1_found` rule is deleted (comment
with the Java lines: `AnalysisAndExecutionPhaseRunner.java:123-141`, `:170`). The Found line is
compared in place by every tool that uses the module (`run`, `java`, `config`, `tests`,
`query`, `bep-matrix`, `bytes/compare`, `textproto bep-e2e`). Module selftest: "B3: a Found line
only Blaze prints differs (no gap)", "B3: a Found line at another position differs".

**B4.**
- `textproto` `compare_files`/`cmd_check`: records equal only as a sorted list →
  IDENTICAL-REDUCED "record order ignored (differs at N positions)"; `bep-text-vs-json` the
  same for events. Selftest: "B4: bep records equal only when sorted are IDENTICAL-REDUCED",
  "B4: bep events equal only as a multiset are IDENTICAL-REDUCED" (a fake printer).
- `bep-matrix` `payloads`/`mask_build_metrics`: the four-counter mask is gone; `buildMetrics` is
  compared as the referee's normaliser leaves it, with only the allow list's volatile counters
  (`evaluated/built/changed/cleaned/dirtiedValues`, `postInvocationSkyframeNodeCount`,
  `actionCacheStatistics.sizeInBytes`) masked. Selftest: "bep-metrics-package-count" →
  DIFFERENT, "bep-metrics-volatile-only" → IDENTICAL.
- `workers` `RULES`: `java_frames` is `kind="gap"`; `compare_case` passes the gap rules to
  `judge`. Selftest: "B4: a case differing only in Java frames is KNOWN-GAP".

**B5, `scripts/starlark/interp-diff` `judge_packages`.** When either query did not end with exit
0 or 3, the stdout, stderr and every package row are SKIPPED with the reason; the summary says
the packages were not judged. The selftest uses the strict `cc.SCENARIOS` (no `itemised()`);
`itemised()` is documented as for `bytes/compare` only. Selftest: generic `both-sigpipe`,
`equal-failures`, `blaze-sigpipe` now end with BOTH-FAILED/DIFFERENT and SKIPPED only; "B5
both-sigpipe / blaze-sigterm / blaze-exit-2: every package row SKIPPED".

**SV-1, `scripts/server/seq` `views`.** stderr is the whole stream, every line in order, after
the record's path placeholders, `common.Norm` and the named rules `SEQ_RULES`
(`VOLATILE_RULES` + `connect_retry`) whose counts the report lists. No include filter, no
multiset. Selftest: "an extra stderr line of any kind: DIFFERENT", "stderr lines in another
order: DIFFERENT (no multiset)".

**SV-2 (`seq`, `interop-client`, `interop-server`).** Steps are judged by `cc.judge` (through
`common.judge_step` or `seq.compare`) and reported with `cc.Report`: equal undeclared failures
are BOTH-FAILED and exit 0 needs every step IDENTICAL (or the one named deviation). Declared
tables, checked against the oracle at every run (otherwise ORACLE-UNEXPECTED): `seq.DECLARED`
(compile-error exit 1; cold cquery/aquery exit 2), `interop-client.DECLARED` (19 steps, values
from every stored oracle transcript), `interop-server.REAL_DECLARED` (2 steps). A case with no
step is SKIPPED, a tool error in a case is a report problem. `seq recompare` is a
re-classification (never exits 0), so `run` and `recompare` agree. Selftests: "equal undeclared
failures: BOTH-FAILED, exit 1" (all three), "equal, declared failures: exit 0", "the declared
cold-reference cquery failure: IDENTICAL".

**SV-3.** `common.OomWatch` reads the unit's `oom_kill` counter around every step; an increase
makes the step's termination `oom-kill` (`common.termination`), and the units now run with
`OOMPolicy=continue` (without it an OOM kill stopped the whole tool: the first selftest showed
exit 130). Selftest (all three, with the unit capped at 400M and a fake that allocates 700 MB):
"an OOM kill ... never IDENTICAL".

**SV-4.** `common.refuse_oracle_as_blaze` (via `cc.blaze_identity`) in `seq`, `interop-client`,
`interop-server` outside `--control`; `common.verified_oracle` now uses `cc.require_oracle`.
`soak`: `--oracle` needs the pin (`compare_common.py check-oracle`), otherwise the oracle is
refused (`check-blaze`). `seq`'s reference cache: `complete.json` is a `cc.write_key` with the
digest of every cached file, checked with `cc.check_key`; a stale cache is re-run and the report
says why. Selftests: "the oracle as the binary under test refused (exit 2)" (three tools),
"wrong oracle hash refused", "a changed cached file is seen: side A runs again". Checked by hand:
`soak run --binary <oracle>` exits 2.

**SV-5.** `selftest` in `seq` (13 checks), `interop-client` (13), `interop-server` (8, the
`--server-binary` mode). `common.selftest_passthrough` carries the selftest variables into the
units; `common.tripwire_env` gives the fakes their token only inside the tool's unit. The
`seq` selftest found a real defect, fixed: two runs within one second shared a run directory.

### S2 and S3 items

- **SV-6** `interop-client.EXPECTED_DEVIATIONS` is keyed by (case, mode, step title) and holds
  the texts: the oracle must exit 37 with "Unrecognized option: -Xbogus", Blaze must print
  exactly a fresh `info release`; only then DECLARED-GAP. Selftest: "the named deviation with
  its texts: DECLARED-GAP", "another difference of the same step stays DIFFERENT".
- **SV-7** `soak`: help says it judges nothing; each row records `signal N` and the unit's
  `oom_kill`; exit 1 when a command died by a signal, an OOM kill was counted or the unit ended
  abnormally; unit with `OOMPolicy=continue`. No selftest (not a comparison). `ut-wrapper`
  unchanged (a maker of a wrapper).
- **M1** `cc.run`: in scope mode the supervisor leaves a keeper process in the scope while the
  command left a process there (`_keep_scope`), so the scope and its counter outlive an OOM kill
  of a daemonized server; `run` reads the counters of the earlier scopes before and after each
  command and turns an increase into `oom-kill` for the later command; keepers end when
  released or when the tool process (pid and start time) is gone. Module selftest: "M1: an OOM
  kill in an earlier command's scope is the later command's".
- **M5** `check_capture` refuses a capture that anchors no outputs. Module selftest "M5".
- **M6** the IDENTICAL class text names the `blank` rule; the footer prints "N blank stderr
  lines were removed ... no class here says anything about blank lines" whenever it fired.
- **M7** branches now under the tripwire: `query` workerd cell, `cmds` fetch cases (warm and
  empty output base) and the crash case, `exec2` every case (tree-cc), `java` build step and
  outputs row, `run --script_path` (equal and differing script), `mod tidy` (fixture
  brotli-tidy).
- **T3** `java.compare`: with no jar on either side the `outputs` row is SKIPPED with both build
  terminations. Selftest "T3/M7".
- **T4** `cc.kill_root_servers` / `cc.server_owned_by_root` (signal only a pid whose command line
  names the root, SIGTERM then SIGKILL); `run` and `tests` use it. The referee's `record` (used
  by `textproto run`) is protected and not changed. Module selftest "T4" (an unrelated pid is
  left alone; a server of the root is ended).
- **T5** `bytes/run` stores `auxiliary` terminations (info, three shutdowns) as dicts;
  `bytes/compare` lists them (older provenance: parsed from the text) and an abnormal one fails
  the run. Selftest "T5".
- **T6** `textproto check`: a legacy numeric exit code is named on the row (`LEGACY_NOTE`) and a
  row that rests on one is IDENTICAL-REDUCED, never IDENTICAL. Selftest "T6".
- **T7** `cmds` C2: `CRASH_TRACE_GAP` for "canonicalize-flags outside workspace": the text after
  "FATAL: bazel crashed ... Printing stack trace:" is a gap (Java trace against Rust panic), the
  lines up to it are compared; the case is KNOWN-GAP at best. Selftest "C2" (two checks). C4:
  the rows already say "measures the wrong thing" (unchanged; the case list's owner must repair
  the cases). `config` N6: no tool change; the three cases are judged in the rerun below.
- **T8** `exec2`: `PROCESSES_RE` removed (unused; `UUID_RE` is used). `bep-matrix` imports
  `shlex` itself and runs every generic scenario (timing-only and stderr-extra-line included).
- **P6** the 80 untracked `BLAZE-*.txt` were written by older probes; not mine, not deleted
  (root's). The probe writes only under `~/.cache/blaze/work/w4-probe/`.
- **P7** the reruns below name the module version of this working tree; it is in no commit
  until root commits the files listed at the end.
- **U1, U2** not done (forbidden path).

## Reruns

Binary under test: `~/.cache/blaze/work/root-runs/blaze-e8b79c0`, sha256
`7455128a0ccfc453a42f639b5c932eb7beb989d9ae25b09c561f2ab7f693d229` (commit e8b79c0). Oracle:
`~/.cache/blaze/oracle/bazel-9.3.2-linux-x86_64`, sha256
`2e14c55c298752d71e8a0bbd762665ffce5966c9ad00ec5d1a8b82a58e7bfa06` (the pin). Every run printed
compare_common sha256 `d363f00532ae615a2a2ab590c6b944531765d248d63349e7e8029bc0cb7ac065` and its
tool sha256 below; none reported a file changed during the run, and the code files still have
the sha256 listed at the end. The suite unit `root-suite-e8b79c0` was inactive from 10:57; the
reruns ran 10:57 to 13:14, one at a time, each under `systemd-run --user --collect --wait --pipe
-p MemoryMax=12G -p MemorySwapMax=0`. Repository HEAD moved during the reruns (`aa55920` for
run/config/tests, `3a006d7` for the rest; commits by root, none touching these files); every
header says "tree dirty". Logs: `~/.cache/blaze/work/tools-fix-2/r5/<tool>.log`, exit codes `~/.cache/blaze/work/tools-fix-2/r5/rcs.txt`. No tool exposes a
`--jobs` option except `bep-matrix` (`--build-flag --jobs=4`, used); the others build with their
fixed command lines.

"Tool sha256" is the value the run's header printed (compare_common `tool_sha256`, a hash over
the tool file's name and sha256; scripts/server: `common.tools_sha256` over that directory, the
imported scripts/oracle files and unit-supervisor), not the file's own sha256 listed at the end.

Classes in order: identical / known gap / identical-reduced / both failed / different / not
ported / oracle unexpected / skipped / declared gap. "known gap", "both failed" and
"identical-reduced" are not matches.

| Tool | Tool sha256 | Selected | Footer classes | Exit | Log |
| --- | --- | --- | --- | --- | --- |
| `run` (brotli, re2) | `df9865e8f414d976c3fb208de259fdf283bdeba6bd8f9564ac881df606de33bc` | 20 | 2 / 13 / 0 / 0 / 5 / 0 / 0 / 0 / 0 | 1 | `run.log` |
| `config` | `e9c0e133d09d6db96c0bd436fc26ad1913b2ebefb30bf10cfb7daa3473382add` | 68 | 1 / 56 / 0 / 9 / 1 / 1 / 0 / 0 / 0 | 1 | `config.log` |
| `tests` | `45c9cda11b6c721d525abe78beedd21ca45005194d05c5b8bbcd055eca5136fe` | 21 | 0 / 6 / 0 / 11 / 4 / 0 / 0 / 0 / 0 | 1 | `tests.log` |
| `query` (brotli, re2, probe) | `28b36f8af55d1b4692ba958a1fb430cc3192f90a5e945fc629da10617ef7f22e` | 1004 cells | 3 / 576 / 0 / 36 / 347 / 42 / 0 / 0 / 0 | 1 | `query.log` |
| `bep-matrix` (`--build-flag --jobs=4`) | `a861f1cb475e3fd25b33c1d412567f1248a8553b2c328daabd8f15996c74cdf4` | 23 | 0 / 7 / 0 / 0 / 16 / 0 / 0 / 0 / 0 | 1 | `bep-matrix.log` |
| `bytes/compare` (after `bytes/run` oracle and Blaze, both exit 0) | `5388cda06aeae4d6f1534eca897dfff724cfe5cf5aa937bc4876959b775bd63e` | 127 items | 112 / 3 / 0 / 0 / 12 / 0 / 0 / 0 / 0 | 1 | `bytes.log`, `bytes-run-*.log` |
| `java` | `719b63ad9bc8526e2c69a4e19185ca95d754626f6c06b000ebdc07201412d389` | 18 | 5 / 9 / 0 / 0 / 4 / 0 / 0 / 0 / 0 | 1 | `java.log` |
| `repo-gaps` | `2b6225ada389228a124281e251dd4a9bbf4a197b1beaf77faa876836743191ba` | 22 | 0 / 0 / 0 / 0 / 22 / 0 / 0 / 0 / 0 | 1 | `repo-gaps.log` |
| `workers` | `981100444b18c4f3b1bbb4e51d37cc4e9d53ba9dcb853db303a373b24803a679` | 34 | 25 / 2 / 1 / 0 / 6 / 0 / 0 / 0 / 0 | 1 | `workers.log` |
| `interp-diff` (566 cases) | `56b0bb0f5a36722e4cf7dbdac7f50e1982f7aeda04043b7f16028c8dd2d8e3cd` | 1135 rows | 1128 / 0 / 0 / 0 / 7 / 0 / 0 / 0 / 0 | 1 | `interp-diff.log`, `interp-diff.json` |
| `bytes/telemetry-compare` | `afce6d1c554713107e60c72085c0c465070e4496169f947e26cbe5d2f44edfda` | 1 | 1 / 0 / 0 / 0 / 0 / 0 / 0 / 0 / 0 | 0 | `telemetry-compare.log` |
| `macros` | `0d110bb538d35a0bfceb753bb06bc454eee3b6833474c81d41123f6d0b12bc42` | 52 | 24 / 0 / 0 / 23 / 5 / 0 / 0 / 0 / 0 | 1 | `macros.log` |
| `mod` | `2ee15f5cd0f8699ec1a6db49f328ef42b34bf7ac5a9a77fbdc82021fbe0bf754` | 133 | 89 / 0 / 0 / 43 / 1 / 0 / 0 / 0 / 0 | 1 | `mod.log` |
| `cmds` | `d0e60dc5f145fee08541f2c159deb09a9ed5a2c36b3583faa1ab1ae2d5287ff0` | 199 | 174 / 1 / 0 / 24 / 0 / 0 / 0 / 0 / 0 | 1 | `cmds.log` |
| `exec2` | `168cb0b94a33853f7a923136d9f796b8f6882585247cdb07cc512a1a893c6bdf` | 12 | 11 / 0 / 0 / 0 / 1 / 0 / 0 / 0 / 0 | 1 | `exec2.log` |
| `w4/probe compare-all` (42 scenarios) | `58233d6267c36b11bb50d93ccbda51dbbb172e2d76192b0bdb25cff7660d9430` | 777 steps | 142 / 249 / 2 / 256 / 128 / 0 / 0 / 0 / 0 | 1 | `w4-probe.log`, `w4-probe.json` |
| `server/interop-server --server-binary` | `c935d1d842858f30fb9e957c39f32b60a7f4b1039a2b82ae77dadd7872acc0e9` (scripts/server) | 16 steps | 13 / 0 / 0 / 0 / 3 / 0 / 0 / 0 / 0 | 1 | `interop-server.log` |
| `server/interop-client --mode daemon` (15 cases) | `c935d1d842858f30fb9e957c39f32b60a7f4b1039a2b82ae77dadd7872acc0e9` (scripts/server) | 890 steps | 250 / 0 / 0 / 152 / 487 / 0 / 0 / 0 / 1 | 1 | `interop-client-daemon.log` |
| `server/seq --project brotli` | | not run | refused by its own guard: 117 GB free on /home, below 120 GB | 2 | `seq-brotli.log` |

Notes per tool (from the logs):

- `run`: `brotli/build-fails` is KNOWN-GAP now (server start and packages loaded only): this
  binary no longer prints `INFO: Found` after a failed build, and the line is compared in place
  (B3). The 5 DIFFERENT: `not-executable`, `two-targets`, `batch`, `re2-test-args`,
  `re2-runs-per-test` (stderr).
- `config`: N6 holds: `tr-child`, `tr-exec-config`, `tr-nested-tuple` exit 0 on the oracle and
  are KNOWN-GAP (warm-server lines only). NOT-PORTED: `pf-transitions`
  (`TransitionsOutputFormatterCallback.start`).
- `query`: 42 cells stop at `unported!`; the per-cell list is in the log and
  `~/.cache/blaze/query-compare/r5-query/`.
- `bep-matrix`: with `buildMetrics` compared beyond the four counters (B4) no scenario is
  IDENTICAL; 7 KNOWN-GAP, 16 DIFFERENT (`~/.cache/blaze/bep-matrix/r5-bep/details.txt`).
- `bytes`: the 3 KNOWN-GAP are stderr files that differ only in `b1_server_start` /
  `b1_packages_loaded` (`fail.stderr`, `providers.stderr`, `query_build.stderr`); none rests on
  the removed Found rule. 12 DIFFERENT as before (Blaze exits 37 on the flag-off workspace).
- `workers`: 2 KNOWN-GAP are cases that differ only in the oracle's Java frames (B4); the one
  IDENTICAL-REDUCED is `samehash` (by design).
- `interp-diff`: the query ended exit 3 on both sides, so the package rows were judged:
  0 differ in outcome, 6 in error text only; the 7th DIFFERENT row is `query-stderr`.
- `cmds`: the one KNOWN-GAP is `canonicalize-flags outside workspace` (C2: crash trace only).
  The 24 BOTH-FAILED include the four C4 cases, which the rows mark as measuring something else.
- `mod`: the two `brotli-tidy-fail warm-up mod graph` auxiliaries end exit 2 on both sides (the
  fixture fails by design; listed, not abnormal).
- `w4/probe`: `record-all` recorded all 42 scenarios with the pinned oracle and wrote 42
  `ORACLE.key.json` (exit 0, `w4-probe-record.log`). 9 of the 42 ORACLE.txt differ from the
  tracked ones only by the oracle's own variance: line order of conflict reports and error
  messages (`top-conflicts`, `top-analysis-errors`, `ui-build-results`, `ui-output-filter`,
  `ui-exec-errors`, `exec-source-errors`, `incompat-toplevel`, `ui-progress`) and warm-server
  counts (`exec-shared-actions`). Root decides whether to commit them (protected: `scripts/w4/`);
  their sha256 are in `~/.cache/blaze/work/tools-fix-2/r5/w4-oracle-records.sha256` (51 lines, sha256
  `e0fa66ff1d0d9b337f13195fe08faa51ba6c1052ede561c45fa6f3a60a896322`). Split by step kind:
  664 steps that run the binary: 30 identical, 249 known gap, 2 identical-reduced, 256 both
  failed, 127 different; 113 file checks: 112 identical, 1 different. Three-view table: full
  151, masked 631, messages 353 of 777.
- `interop-server`: DIFFERENT steps: `build //:ok (again)`, `build //:fail`, the cancelled
  build (transcripts).
- `interop-client` (daemon): cases with every step IDENTICAL or declared: large-output,
  shutdown, startup-options, two-clients, startup-crash (its one step is the named deviation,
  DECLARED-GAP), cwd-deleted, idle; with a DIFFERENT step: plain, exit-codes, command-log,
  scenarios, sigkill, sigint, run, broken-pipe. The jvm mode was not rerun.
- Not rerun: `seq` (guard, above), `soak` and `ut-wrapper` (they judge nothing), `textproto`
  (not in the rerun list).

What can be quoted: the rows above with this document's tool and module sha256, the binary
sha256 `7455128a…` and the denominators as given; never a sum of identical with known gap,
both failed or identical-reduced.

## Open

- `seq` needs a rerun once /home has 120 GB free (root: `scripts/clean-target`; 133 GB sit in
  `~/.cache/blaze/oracle-out/ur-remote`, another agent's root, which I did not touch).
- U1, U2 (`ut_common.py`) are outside this task's paths.
- C4 needs the case list's owner (the four fetch cases measure a download failure or another
  error).
- `textproto` directories recorded before this change have no file digests: `check` gives them a
  SKIPPED `files-anchored` row, `bep-e2e` refuses them; they need `textproto run` again.
- The 9 re-recorded ORACLE.txt files and the 42 new keys are protected (`scripts/w4/`) and
  uncommitted.

## Files changed and sha256

Code (all uncommitted; root commits after the auditor's verdict):

| File | sha256 |
| --- | --- |
| `scripts/lib/compare_common.py` | `d363f00532ae615a2a2ab590c6b944531765d248d63349e7e8029bc0cb7ac065` |
| `scripts/w4/probe` | `141bca02eb8eff3093bc2fd380d4e5520e7ef3ce38bab4fbc17f561004701e3f` |
| `scripts/textproto/compare` | `a3e65527624cc7ded74b59618c5c43d0d07598f95aaeed83fb8a61df6b297ac7` |
| `scripts/bep-matrix` | `bf3a99928ccf322cf9c361daa650210a77f80d108b4ee2ded2d6ba56c2ff0963` |
| `scripts/workers/compare` | `595e95ffd92507756506314501c6bf2fe9c968b702c87f8507fcc07bbb8a980b` |
| `scripts/starlark/interp-diff` | `cb160b7aca4484ac0b8033dca030e197b6028316aaddc6bb7edef60e5bf08b6b` |
| `scripts/server/common.py` | `e26e9daa0149bec9758b38d782631f7c6175b06542b052360e9af60a3b0a5d43` |
| `scripts/server/seq` | `d50fc513e558e14fbaabdc0aa7a3c71cadace09bd92622e4794305d24d37b18b` |
| `scripts/server/interop-client` | `aac23fb42c1f0457da02781160ce7fa1dff943c1a050bc8c4dcb06e03033a579` |
| `scripts/server/interop-server` | `6ed8a8d7bcb62a07afb4aa10129658e927ba8c7ee3688d3b32769de3989ca179` |
| `scripts/server/soak` | `69d8206d4bc12510e7a99385774754f5bc6928e47f1a07b5a9514c102a033ca3` |
| `scripts/bytes/compare` | `a866b95a7dad39a87187245eb349ffce15ef86e24406fc8791391c3a91699ccc` |
| `scripts/bytes/run` | `d8593d06239111687bb3a937a2e4ca3dfdeaa33cf2c58202eb1106f61febafce` |
| `scripts/cmds/compare` | `45b94381307248f01d26a931865d873fcf47cd2eedf4f84caf2026234d2655bd` |
| `scripts/exec2/compare` | `7093ec55ef78f47d69206b09535c94789ef6bb32527d69e038721e2b6340fe99` |
| `scripts/java/compare` | `3e4003dfb855db2dd15a93e4cfa899de36294a0749d83c666314458c4cd065e2` |
| `scripts/mod/compare` | `2e162034216e50932c88a52be0a714bead1b26a7ce3201b8ace5db3eede5b9fd` |
| `scripts/query/compare` | `734b880e99adaec1a75b75e2c136eab9826a9017d173285141b6bee31104682e` |
| `scripts/run/compare` | `5fc00d32af2391cb1d32fafffc1ecc334155c0793967b6d22c7b49353c60de18` |
| `scripts/tests/compare` | `03267c0061287522d62ca4608f5dfcc871754f1c0419eea28133c872e1e45b90` |

Oracle records written by `w4/probe record-all`: 9 changed `scripts/w4/scenarios/*/ORACLE.txt`
and 42 new `scripts/w4/scenarios/*/ORACLE.key.json`, listed with their sha256 in
`~/.cache/blaze/work/tools-fix-2/r5/w4-oracle-records.sha256` (sha256 above). This report:
`docs/reviews/2026-11-25-audit-4r2-response.md`.

Selftests on these bytes (log `~/.cache/blaze/work/tools-fix-2/r5/selftests.log`): the module
(49 checks) and 22 tools, all PASS: run 20, cmds 22, config 18, macros 19, java 19, exec2 19,
mod 20, bytes/compare 19, bytes/run 6, bytes/telemetry-compare 14, proxy/run 6, query 18,
tests 19, repo-gaps 15, workers 19, interp-diff 18, textproto 19, bep-matrix 26, w4/probe 27,
server/seq 13, server/interop-client 13, server/interop-server 8. The gate's unit test of the
probe (`ProbeStepClassTest`, 2 tests) passes. `proxy/classify` still has no selftest (not in
this task's findings).
