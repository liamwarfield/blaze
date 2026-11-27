# Referee batch 4 (2026-11-27): implementer's report for the auditor

Work on the open items of the batch 3b audit (`docs/reviews/2026-11-26-audit-referee-3b.md`):
the prepared A5–A8 extractor patch, C1, C2 and the A2 start-line decision. After that, the
renormalisation and rescoring the audit prescribes.

Tree: working tree on HEAD `8e10c3af`. `scripts/oracle` at HEAD is the batch 3b commit
`bbbc6fa3`; no later commit touches it. Nothing is committed. No git index command was used.

- Referee sha256: `e7130cdba227becfb09f8b15d1e3a3ea916e500309fc08b8652535207a673c88` (was
  `ab46a162…`). It now covers `oracle_common.py` too (C2).
- Extractor sha256: `fb3ca2d0e8653f7a275ba85087b078a51078d1e3d47e31610e86dbca98364b3d` (was
  `c215d2d7…`). It now covers `oracle_common.py` too (see C2 for why).
- Unit tests (`python3 -I -m unittest discover -s scripts/oracle`): 174 run, 174 passed,
  0 failed, 0 skipped. That is 164 after the patch, plus 10 new tests; 1 existing test was
  adjusted.
- `root-suite-5f68157` was `inactive` when I checked at the start and again before the
  renormalisation. No bazel or blaze process was started.
- Each renormalisation, comparison and scoring ran alone in `systemd-run --user --collect --wait
  -p MemoryMax=12G -p MemorySwapMax=0 --same-dir --pipe`. Peak memory was 4.4 GB (the workerd
  renormalisation).

## 1. The A5–A8 patch

`~/.cache/blaze/work/referee-3b/next-extractor/next-extractor-A5-A8.patch`:
- sha256 `811b2a54224d4c00c7879e2521beef45b37e1bc49d1d1ac2eaaad86348807f8d` (checked);
- `git apply --check` was clean, so I applied it with `git apply`, which does not touch the
  index. I ported no hunk by hand.

What it changes:
- A5: `scripts/oracle/surfaces.py:566`. An artifact size is masked only when it is a string of
  decimal digits.
- A6: `surfaces.py:531` (`presence`). An empty list, `null`, `0`, `"0"` and `false` are kept as
  they are.
- A7: `surfaces.py:40-41` and `mask_option_values`. `install_md5` is masked only when the value
  is 32 lowercase hex digits, and the value is replaced inside the printed `combinedForm`.
- A8: `surfaces.py:570` sorts `buildGraphMetrics.dirtiedValues` by SkyFunction, and
  `allow.json:37` takes `dirtiedValues` off `only_fields_in_daemon_mode`. It stays in
  `only_fields`, and `daemon_mode_reason` says why.

Tests that came with the patch:
- `test_surfaces.NextExtractorTest` (5 tests).
- Two adjusted tests: `InstallMd5Test.test_string_form_is_masked` and
  `test_compare.DaemonModeAllowTest.test_the_real_entry_drops_the_four_cold_server_fields_in_daemon_mode`.
- With the patch alone, 164 tests ran and 164 passed.

## 2. C1: strictness no longer depends on the pid file

`scripts/oracle/oracle_common.py`:
- `started_no_server(manifest, raw_dir)` (`:267`) replaces `observed_warm_server`. It is true when
  any command did not start a server, by any of three sources:
  - `server.start_line` is `False` in the manifest;
  - `server.state` is `warm` in the manifest (the pid source of `server_states()`);
  - `raw/<command>.stderr` exists and has no client start line. This is the stderr source of
    `server_states()`.
- I read the stderr even when the manifest has per-command states. A manifest that claims a
  start line on every command therefore cannot hide a stderr that has none. This is one step
  further than the audit asked: "`server_states()` gives `warm`" reads stderr only when the
  manifest has no states.
- No evidence (no stderr file and no fields) does not count as a server.
- `strict_server_rules(manifest, raw_dir)` (`:287`) is true when the mode is `daemon` or
  `started_no_server` is true. `raw_dir(label, project)` (`:303`) is a new helper that
  `server_states` also uses (`:336`).

`scripts/oracle/oracle_compare.py` passes the candidate's `raw/` in two places:
- `provenance` (`:387`) and `compare_labels` (`:473`);
- the rule is still applied at compare time only, and to the candidate (side b) only.

### The consequence the audit did not list: the oracle against itself is now strict

Bazel does not print the start line after `mod graph`. So `oracle-9.3.2-rerun` and `-check`,
and every reference used as side b, are now judged by the daemon-mode rules. Their manifests
predate batch 3 and have no `server` record, so their final `shutdown` was never recorded. The
A1 rule ("not recorded" means `__shutdown__` EXTRA) would have failed all three self-checks.

My choice:
- `oracle_common.shutdown_recorded(manifest)` (`:296`) is true when the manifest has a `server`
  record.
- `server_checked_exit_codes(..., shutdown_recorded=True)` (`oracle_compare.py:318`, `:338`)
  skips only the shutdown check when the record is absent. `compare_labels` (`:478`) then adds a
  note, which is printed as a warning and kept in the score: `<label>/<project>: judged by the
  daemon-mode rules, but recorded before batch 3 (no `server` record): the final shutdown is
  not checked`.
- A `server` record without the shutdown fields still fails, as in the audit's A1 table. Mode
  `daemon` always has the record, because the mode is stored in it.

Why I think this does not open a hole:
- Today's recorder always writes the record (`record:501`).
- Deleting it from a manifest is an edit of an unanchored file. The same edit could set
  `shutdown_exit_code: 0` instead.
- The command states are still checked.

The auditor should rule on it. The alternative is to fail the oracle's self-checks, or to
re-record `-rerun` and `-check`, which the task forbids.

Results on the real recordings:

| label | strict before | strict now | why |
|---|---|---|---|
| `oracle-9.3.2`, `-rerun`, `-check` (brotli, re2, workerd) | no | yes | no start line after `mod_graph` (stderr source) |
| `blaze-ef66f55e904e` (base) | no | no | start line on all 10 commands (stderr) |
| `guard-8ecc520` | no | no | `start_line` true on all 10 commands, and in stderr |
| `guardd-8ecc520` | yes | yes | mode `daemon` |

The table comes from a survey of every command's manifest fields and raw stderr. The "strict
now" values are the `strict_server_rules` that `compare` printed for each recording.

## 3. C2: the hashes cover the strict rules

I chose to add `oracle_common.py` to the hash rather than move the rule into
`oracle_compare.py`. The reason is that the rule depends on more than one function:
`server_states`, `client_started_server`, `canonical()` (used in item keys and, since A8, in
the `dirtiedValues` sort key) and `SURFACES` all live in `oracle_common.py`. Moving only
`strict_server_rules` would leave its inputs outside the hash.

- `oracle_common.py:171` `REFEREE_FILES`: `normalise.py`, `surfaces.py`, `oracle_common.py`,
  `oracle_compare.py`, `allow.json`.
- `oracle_common.py:172` `EXTRACTOR_FILES`: `normalise.py`, `surfaces.py`, `oracle_common.py`.
  I added it here too because A2 below makes the extractor call `oc.strict_server_rules`. The
  extractor already called `oc.canonical` and `oc.blaze_server_mode` without that file in its
  hash.
- Cost: any later edit of `oracle_common.py` marks every recording "extracted by another
  version" (a warning in `compare`) and needs a renormalisation. That is the same as an edit of
  `surfaces.py` today.

## 4. A2: the start line under the strict rules

`surfaces.py:412` (`s_build_stderr`) keeps the server-start line when
`oc.strict_server_rules(manifest, <rd>/raw)` is true. That is the same rule as at compare time,
applied to the recording's own manifest and stderr. `record` writes the manifest with its
per-command `server` fields before `finish()` extracts, and `.stage/raw` is a link to `raw/`,
so a fresh recording gets the rule too.

Effect on existing recordings: none. In every strict recording above, the build ran without the
start line, and the base and `guard` are lenient as before. Measured: no `build_stderr` file
changed in any renormalised recording (section 6).

## 5. Tests

New tests:
- `test_compare.StrictByAnySourceTest` (`:843`), 7 tests (C1):
  - `test_c1_no_pid_seen_and_no_start_line_gets_the_strict_rules` (the audit's last evasion row);
  - `test_c1_a_recording_shaped_before_batch_3_gets_the_strict_rules_from_stderr` (the "no
    `server` key and no states" row; also checks the shutdown note);
  - `test_c1_the_oracle_shape_against_itself_matches`;
  - `test_c1_raw_stderr_is_read_even_when_the_manifest_has_states`;
  - `test_c1_one_process_blaze_stays_lenient` (the shapes of the base and of `guard-8ecc520`);
  - `test_c1_a_recorded_server_without_shutdown_fields_still_fails`;
  - `test_c1_strict_server_rules_sources`.
- `test_compare.RefereeHashTest` (`:952`), 2 tests (C2):
  - `test_the_strict_rules_are_inside_both_hashes`: the source files of `strict_server_rules`,
    `started_no_server`, `server_states`, `canonical` and `server_checked_exit_codes` are in
    `REFEREE_FILES`, and the first three plus `canonical` are in `EXTRACTOR_FILES`;
  - `test_a_change_to_oracle_common_changes_both_hashes`.
- `test_surfaces.BuildStderrModeTest.test_kept_under_the_strict_rules_without_the_mode`
  (`:431`), A2. It covers the stderr source, the manifest source and the lenient one-process
  case.

Adjusted: `DaemonRulesTest.test_an_observed_warm_server_gets_the_strict_rules_without_the_mode`.
Only the new second argument changed (`raw_dir=None`); the expectations are the same.

Runs:
- Working tree, `python3 -I -m unittest discover -s scripts/oracle`: 174 run, 174 passed.
- New tests against the old code: `~/.cache/blaze/work/referee-4/oldcode/` holds HEAD's
  `scripts/oracle` with the A5–A8 patch and the new test files. 174 run, 6 failures, 5 errors.
  - 8 of the 10 new tests fail or error.
  - The adjusted test errors (new signature).
  - `test_this_checkout` and `test_the_committed_workerd_summary_declares_its_splice` error only
    because the copy is outside the repository, as in batch 3b.
  - The two guard tests pass on both trees: `one_process_blaze_stays_lenient` and
    `recorded_server_without_shutdown_fields_still_fails`.
  - Log: `~/.cache/blaze/work/referee-4/oldcode/log.txt`.
- `scripts/gate` (not edited): `test_gate.py` 30/30 and `test_protected.py` 15/15 pass.
  `test_evidence_tools.py` has 16/17 passing. The failure is
  `test_every_site_of_this_checkout_is_registered`: an unregistered silent-skip site at
  `crates/blaze_exec/tests/sandbox_docker_sandboxed_spawn_runner_test.rs:167`, committed in
  `9b5671c8`. It has nothing to do with this batch.
- `scripts/gate/compare-scores` and `test_gate.py` needed no change. The fixtures already carry
  `"staged": None`.

## 6. Renormalisation

All renormalisations were in place, one at a time, with `scripts/oracle/record --renormalise
--label L --project P`, in the order the task gives.

Backups were made before any of them, with `cp -a` by full path:
- `manifest.json`, `items/` and `norm/` of all 13 label/project directories, in
  `~/.cache/blaze/work/referee-4/backup/<label>/<project>/`;
- `corpus/oracle/{brotli,re2,workerd}.json`, in `backup/corpus-oracle/` (all three were equal
  to HEAD before).

`~/.cache/blaze/work/referee-4/bin/renorm-diff.py` compared each backup with the live record:
- the manifests' file sha256;
- every item of every changed items file;
- every manifest key.

Logs: `~/.cache/blaze/work/referee-4/logs/renorm-*.log`.

| recording | files that changed | items that changed | manifest |
|---|---|---|---|
| `oracle-9.3.2/brotli` | `items/bep.json`, `norm/bep.json` | `{"buildMetrics":{}}` | `extractor_sha256`, `files`, `renormalised` |
| `oracle-9.3.2/re2` | `items/bep.json`, `norm/bep.json` | `{"buildMetrics":{}}` | same |
| `oracle-9.3.2/workerd` | none | none | `extractor_sha256`, `renormalised` |
| `oracle-9.3.2-rerun/brotli`, `/re2` | `items/bep.json`, `norm/bep.json` | `{"buildMetrics":{}}` | as brotli |
| `oracle-9.3.2-check/brotli`, `/re2` | `items/bep.json`, `norm/bep.json` | `{"buildMetrics":{}}` | as brotli |
| `blaze-ef66f55e904e/brotli`, `/re2` | none | none | `extractor_sha256`, `renormalised` |
| `guard-8ecc520/brotli`, `/re2` | none | none | `extractor_sha256`, `renormalised` |
| `guardd-8ecc520/brotli`, `/re2` | `items/bep.json`, `norm/bep.json` | `{"buildMetrics":{}}` | as brotli |

- For all 8 changed `buildMetrics` items, I sorted the old item's `dirtiedValues` by SkyFunction
  and got exactly the new item. Each has 23 entries summing to 1649, and the order changed in
  each.
- So A5–A7 changed no item, and A2 changed none.
- This confirms the batch 3b measurement for brotli, re2 and workerd. It adds that `guardd`
  changes the same way and that `guard` and the base do not change.

`corpus/oracle/` was rewritten by the renormalisation itself (`write_summary`, the default for
the reference label), and in nothing else:
- `brotli.json` and `re2.json`: `extractor_sha256`, `files_sha256["items/bep.json"]` and
  `files_sha256["norm/bep.json"]`;
- `workerd.json`: `extractor_sha256` only.

The workerd `mod_graph` and `lockfile` surfaces are `FAILED` (mod_graph exited 2) before and
after. The manifest's `surfaces` are unchanged.

## 7. Oracle self-checks

These ran after the renormalisation, with reports in `~/.cache/blaze/work/referee-4/reports/selfcheck-{1,2,3}.json`:

| comparison | exit | result |
|---|---|---|
| `oracle-9.3.2` vs `oracle-9.3.2-rerun --project brotli --project re2` | 0 | `100% MATCH`, 26 surfaces, 13,916 = 13,858 + 58 allowed |
| `oracle-9.3.2` vs `oracle-9.3.2-check --project brotli --project re2` | 0 | `100% MATCH`, same counts |
| `oracle-9.3.2-rerun` vs `oracle-9.3.2-check` | 0 | `100% MATCH`, same counts |

- All three are now judged by the daemon-mode rules (`(daemon-mode rules)` in the server
  lines).
- The `exit_codes` server-state check passes: `mod_graph` cold and 9 warm on both sides, from
  stderr.
- The `buildMetrics` differences fit the narrowed daemon list, which no longer has
  `dirtiedValues`.
- New warnings:
  - the shutdown note of section 2, for each candidate project;
  - "corpus/oracle/*.json differs from its committed version at HEAD", until root commits the
    summaries.

## 8. The base under referee 4

`corpus/scores/blaze-ef66f55e904e.referee4.json` and `.md` are new; the `referee3b` files are
not touched. Commands:

    scripts/oracle/scoreboard blaze-ef66f55e904e --json --out corpus/scores/blaze-ef66f55e904e.referee4.json --allow-dirty "$R"
    scripts/oracle/scoreboard blaze-ef66f55e904e --markdown --out corpus/scores/blaze-ef66f55e904e.referee4.md --allow-dirty "$R"

`R` = "root ruling 2026-11-19: accepted label recorded before commit verification existed;
rescored from its existing recordings under referee batch 4 (renormalised in place with the
batch 4 extractor; not re-recorded)".

Old (`referee3b.json`) against new (`referee4.json`), compared key by key:
- `rows`: 39 and 39, the same keys. **No row moved**: every field of every row is equal,
  `matching_keys` included.
- `totals`: equal. Total 158,988, matching 13,850, allowed 56, failed 145,082, compared 13,860,
  extra 1. Surfaces: MATCH 23, DIFF 3, MISSING 8, NOT_RECORDED 5. The verdict is equal.
- Also equal: `project_totals`, `allowed_by`, `failures`, `excluded`, `exceptions`,
  `recording_problems`, `clean`, `staged` (`null`).

What differs, and why:
- `referee_sha256`: `ab46a162…` → `e7130cdb…` (C2 and the patch).
- `allow_dirty`: the text names batch 4.
- `provenance.*.*.extractor_sha256`: `c215d2d7…` → `fb3ca2d0…` on both sides.
- `provenance.{brotli,re2,workerd}.reference.strict_server_rules`: false → true. This is the
  oracle under C1. It is printed only; strictness is applied to the candidate alone.
- `warnings`: three new lines, "corpus/oracle/{brotli,re2,workerd}.json differs from its
  committed version at HEAD". This is the same effect batch 3b noted for `referee3.json`.

Why nothing moved:
- The base is one-process and stays lenient.
- Its items did not change in the renormalisation.
- The reference's only changed item is `buildMetrics`, which was already allowed for the base
  (brotli and re2 `bep` rows, with `allowed` unchanged).

## 9. The candidates against the new base

The scores were written with `scoreboard <label> --json --out
~/.cache/blaze/work/referee-4/scores/<label>.json` (`staged: null`, referee `e7130cdb…`).
Each candidate was then gated with `scripts/gate/compare-scores
corpus/scores/blaze-ef66f55e904e.referee4.json <score>`.

| label | mode | strict | matching | allowed | compared | failed | extra | gate |
|---|---|---|---|---|---|---|---|---|
| base `blaze-ef66f55e904e` | one process | no | 13,850 | 56 | 13,860 | 145,082 | 1 | n/a |
| `guard-8ecc520` | one process | no | 13,852 | 58 | 13,858 | 145,078 | 1 | PASS, exit 0: rows 39, gained 2, regressed 0 |
| `guardd-8ecc520` | daemon | yes | 13,858 | 56 | 13,860 | 145,074 | 0 | PASS, exit 0: rows 39, gained 2, regressed 0 |

The total is 158,988 for every row.

Rows that differ from the base:
- `guard-8ecc520`:
  - brotli/bep DIFF, matching 37 → 38, allowed 0 → 1;
  - re2/bep DIFF, matching 109 → 110, allowed 0 → 1.
  - In both, `buildMetrics` is allowed: the cold-server fields of one-process mode plus
    `garbageMetrics`.
- `guardd-8ecc520`:
  - brotli/bep DIFF 37 → 41;
  - re2/bep DIFF 109 → 113;
  - re2/build_stderr DIFF (7, extra 1) → MATCH (7).

The only failing items of `guardd` besides the 56 allowed runfiles keys are the two
`buildMetrics` events. Their differing paths (`reports/guardd-bep.json`) are:
- brotli, 38 paths: `builtValues[]`, `evaluatedValues[]`, `postInvocationSkyframeNodeCount`,
  `memoryMetrics.garbageMetrics`;
- re2, 40 paths: the same and `actionSummary.actionCacheStatistics.sizeInBytes`.

No `dirtiedValues` path is left (A8). `builtValues` and the node count are outside the daemon
list, so the item fails, as at `ce81d5a`. `exit_codes` is MATCH in both projects under the
server check.

Both guard labels were recorded from a dirty tree, with a binary outside this checkout's
`target/` (the gate prints NOTEs). They are guard labels, not scores to accept.

## Open problems and things I did not verify

1. **The shutdown exemption for recordings without a `server` record** (section 2) is my
   choice and needs the auditor's ruling.
2. **Root's scoring step after the commit.** `referee4.json` carries three "summary differs
   from HEAD" warnings because the summaries are not committed. Running the two `scoreboard`
   commands of section 8 again after the commit should remove exactly those three lines. I did
   not verify this, because it needs the commit.
3. **Other labels still have the old extractor.** Every label under
   `~/.cache/blaze/oracle-records` other than the 7 I renormalised (`r3b-ce81d5a`,
   `r3bd-ce81d5a`, the wave labels, ...) still has extractor `c215d2d7…`. `compare` will warn
   for them. Renormalise any of them before comparing it.
4. **Unrelated gate test failure.** The `test_evidence_tools` failure (section 5) belongs to the
   owner of `blaze_exec`.
5. **C3 of the audit** (the ratchet does not go back to one-process recordings once a daemon
   score is the base) is root's to record. I did not touch it.
6. **README.** `scripts/oracle/README.md` now describes the widened trigger, the three sources,
   the shutdown exemption, the start-line rule, both hash lists and A5–A8. The text "26
   surfaces, 13,916 reference items, 58 allowed" still holds.

## Left on disk

`~/.cache/blaze/work/referee-4/` contains:
- `backup/` (3.2 GB: the pre-renormalisation summaries; root may delete it after the audit);
- `oldcode/`, `logs/`, `reports/`, `scores/`;
- `bin/renorm-diff.py`.

I deleted nothing. 95 GB are free on /home.

## sha256 of the files root should commit

| file | sha256 |
|---|---|
| scripts/oracle/README.md | bf1fcb3cb1d8ca3e001b9455e9333992a45a6ccb7c5aaaf582b9d24439268253 |
| scripts/oracle/allow.json | b9c873acb4795ba95454fc140b54e76bbc319c1ebac79297c7d24c00df913ec2 |
| scripts/oracle/oracle_common.py | d245c3b422f5c7214bfd4a2cbe49f7395a6f3d9f093a4f05efce71e6a0696a8c |
| scripts/oracle/oracle_compare.py | 99d9c5cddeaf57662ad2b19990afb61b977b9ae3784b33c486a09b1d87bdafee |
| scripts/oracle/surfaces.py | 8d3702501a589880badb56e4cf66adbd6c82fac703b88c2294842e652921607d |
| scripts/oracle/test_compare.py | ffd695e494b20af5eb2375d236535fde03c7106646baeec4f733ba558866ae9a |
| scripts/oracle/test_surfaces.py | 8e6c0fb8adb4144ab70d21c72a72f1f10c494fe1d5c95c2816eb6c458f3ed6bb |
| corpus/oracle/brotli.json | 6db0f7619922f886441d023b55ee73b34c2f719528588198f622b3e305391da2 |
| corpus/oracle/re2.json | adca4d6f06feb35ab38490dd4b271f3269e62691229511d89efeb35dbb39075d |
| corpus/oracle/workerd.json | 43ab22277a43a60a977a3fffdd9e265e12019cee1333270a938eaa1d3e37e290 |
| corpus/scores/blaze-ef66f55e904e.referee4.json | d29c87c404ccf887025e5436d3d0286fbd9a96d9f326d74a4d5de5f4107f0b52 |
| corpus/scores/blaze-ef66f55e904e.referee4.md | 2a2b04759e394f730679eb4907c9d3443287e067d608ee8f5974bb79c3b2ccb5 |

These files are unchanged from HEAD: `normalise.py`, `oracle_report.py`, `record`, `compare`,
`scoreboard`, `fixture_records.py`, `test_normalise.py`, `test_record.py`, and everything in
`scripts/gate`. This report cannot hold its own hash; it is in the final answer.
