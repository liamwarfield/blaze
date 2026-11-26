# Audit of referee batch 3 (2026-11-26), area `referee-batch-3`

Regression auditor, read-only. Subject: the uncommitted changes under `scripts/oracle` and the
rescored base, as described in `docs/reviews/2026-11-26-referee-batch-3.md`
(sha256 `dd8fe5439a54…`).

**Verdict: PASS**, bound to the file hashes in "Files root may commit" (working tree; HEAD was
`66174e8c` when I started and `b185038c` when I finished, and neither commit touches these
files). The PASS has one condition: no recording made with `--blaze-server daemon` is accepted
as a score before follow-up batch 3b (below) is in.

No bazel or blaze process was started (`root-suite-5f68157` was active throughout). Everything
below was done with Python on existing recordings.

## What I checked, and how

- All 15 hashes in the fixer's table match the working tree.
- Own stage (`~/.cache/blaze/work/review-referee-batch-3/stage`): plain copies of the shared
  recordings `oracle-9.3.2`, `-rerun`, `-check`, `blaze-ef66f55e904e`, `r3-374c631`,
  `r3d-374c631` (brotli, re2), renormalised by me with the working-tree extractor.
  - The reference summaries I got are byte-identical to the fixer's: brotli `bb73e232…`, re2
    `76272118…`.
  - Every `items/` directory is byte-identical to the fixer's stage (12 of 12).
  - The shared recordings are untouched (still extractor `cdf91c84…`).
- workerd was not re-extracted by me. The fixer's staged workerd items are byte-identical to
  the shared ones (`diff -r`), and the staged summary differs from the committed one in
  `extractor_sha256` only.
- Oracle against itself, three pairs: each `100% MATCH`, 13,916 = 13,858 matching + 58 allowed.
- Base rescored by me: `rows`, `totals` and `referee_sha256` (`c1407ce7…`) equal the fixer's
  `corpus/scores/blaze-ef66f55e904e.referee3.json`.
- Referee 2 base against referee 3 base, row by row with `matching_keys`: 0 keys lost, 0
  gained. Only `brotli/bep` and `re2/bep` change (allowed 1 → 0, failed 4 → 5); the moved item
  is `{"buildMetrics":{}}`. Totals: matching 13,850 → 13,850, allowed 58 → 56, failed 145,080
  → 145,082.
- `scripts/gate/compare-scores` against the referee 3 base: `r3-374c631` PASS and `r3d-374c631`
  PASS, each rows 39, gained 2, regressed 0. It refuses referee 2 against referee 3 (different
  referees), as it should.
- Unit tests: 148 run, 148 passed on the working tree.
- Old code: the copy in `~/.cache/blaze/work/referee-3/oldcode` equals HEAD for all ten files.
  Seven new tests or classes run against it all fail or error, as the fixer says:
  `test_empty_artifact_metrics_is_a_difference`, `test_install_md5_value_is_masked`,
  `test_cumulative_metrics_are_compared`, `test_machine_measurements_are_compared_by_presence`,
  `test_a_server_of_an_earlier_run_is_stopped_first_and_a_stranger_is_left_alone`,
  `DaemonModeAllowTest` (2 of 4 fail, 2 are guards), `ServerStartLineTest` (1 of 2).

Not read or run: the README beyond its diff, `scripts/bep-matrix`, `scripts/bep-compare` and
`scripts/textproto/compare` (they import `normalise_bep` and will see the new behaviour), and a
real daemon-mode recording run.

## Answers to the six questions

1. **`--install_md5`.** Anchored. In the raw BEP of all three recordings the option occurs only
   as an option message of `structuredCommandLine` (`optionName: install_md5`). The mask keys on
   `optionName` equal to `install_md5`, or on a whole string `--install_md5=<no space>`. A
   value of another option that contains the text is not touched, and lists are not reordered.
   Both sides print 32 hex digits. Weakness: see A7.
2. **`buildMetrics`.**
   - Compared: `actionSummary`, `targetMetrics`, `packageMetrics`, `buildGraphMetrics`,
     `cumulativeMetrics`, artifact counts, `sourceArtifactsRead.sizeInBytes` (equal in the three
     oracle runs: 2328125 and 263555297).
   - Masked sizes, checked in the three raw oracle BEP files:

     | field | brotli | re2 | evidence |
     |---|---|---|---|
     | `outputArtifactsSeen.sizeInBytes` | 7671064 / 7671088 / 7671088 | 89006013 / 89050518 / 89050526 | real |
     | `topLevelArtifacts.sizeInBytes` | 11766429 / 11766441 / 11766441 | 563166591 / 563188791 / 563188791 | real (path length only: `-rerun` and `-check` agree) |
     | `outputArtifactsFromActionCache.sizeInBytes` | absent | absent | none: the group is `{}` in every run |

     The third mask is inert today (a candidate that prints the group differs in `count`).
   - Presence-only groups: an empty group on Blaze's side fails wherever the oracle's has
     fields (`memoryMetrics`, `timingMetrics`, `networkMetrics`). `workerPoolMetrics` and
     `dynamicExecutionMetrics` are `{}` in the oracle too, so `{}` is correct there. A
     nonsensical value passes: see A5 and A6.
   - `garbageMetrics` is not stable in the oracle itself: re2 `-rerun` lists six pools, the
     other two runs seven.
3. **Server mode.**
   - Lost by removing the recorder's options on both sides: Blaze's default
     `--max_idle_secs=10800`, which one-process Blaze prints today and which equals the
     oracle's (A4). Nothing else: `--option_sources` loses only the `max_Uidle_Usecs` pair, and
     the `{}` removed is only the one that follows a removed original entry.
   - Warning or failure: a warning is right for one-process recordings and not enough for
     daemon-mode ones (A1).
   - The mode comes from the candidate recording's own manifest (`server.blaze_server_mode`),
     at extraction (`s_build_stderr`) and at comparison (`compare_labels`). No compare-time flag
     exists. But it is the mode the recorder was asked for, not the one observed (A2).
   - `kill_servers` signals only a pid from the root's own pid file whose command line contains
     the root path. Better than HEAD, which signalled any pid in the file. Remaining gap: B2.
4. **Staging.** The tools cannot write a staged file into the repository: `scoreboard --write`
   and `record --summary` go to the stage. A staged report is marked only by the first line of
   `warnings`; `compare-scores` neither prints nor refuses it (B1). The fixer's base in
   `corpus/scores/` is a hand copy of the staged file and carries that line, so root
   regenerates it unstaged (steps below). The expected summary hashes are confirmed.
5. **Rescored base.** Confirmed in full (see above).
6. **Tests.** Confirmed on seven instead of five.

## Findings

S1: none. Batch 3 lets nothing pass that failed under HEAD, except the three declared masks
(the `install_md5` value, `--max_idle_secs`, `--client_env=BLAZE_SERVER_MODE`).

S2, required in batch 3b before the first daemon-mode label is accepted. None changes the
extractor, so the renormalised recordings and the committed summaries stay valid; only the base
is scored again.

- **A1. A daemon-mode state mismatch only warns** (`oracle_report.py` `server_state_lines`). In
  daemon mode nothing excuses a command that ran cold where the oracle's ran warm, and a server
  that died between two query commands shows on no surface. Expected: when the candidate's mode
  is `daemon`, each command whose state differs from the reference's makes that command's
  `exit_codes` item a DIFF (path `server_state`), decided at compare time so that reference
  items do not change. A non-zero `shutdown_exit_code` or a non-empty `alive_after_shutdown`
  fails the surface the same way. One-process candidates keep the warning. `r3d-374c631` has
  equal states and a clean shutdown, so it is unaffected.
- **A2. The strict rules follow the requested mode, not the observed one**
  (`oracle_compare.py` `compare_labels`, `surfaces.py` `s_build_stderr`). The day Blaze's
  default becomes the server, a recording made without `--blaze-server` has mode `null` and
  gets the lenient `only_fields` and the dropped start line. Expected: strict when the mode is
  `daemon` or the manifest records `build` as `warm`.
- **A3. An empty field list excuses everything** (`oracle_compare.py` `AllowList.prepare` and
  `match`). `"only_fields_in_daemon_mode": []`, the natural way to write "nothing is excused in
  daemon mode", makes the entry match any difference of the item. The same holds for a DIFF
  entry without `only_fields`. Expected: `prepare` rejects an entry whose `kinds` include
  `DIFF` and whose `only_fields`, or daemon list when present, is empty.

S2, declared or older; do with the next extractor change (each forces a renormalisation):

- **A4.** `--max_idle_secs` is removed in every mode, so Blaze's default is no longer compared
  by the referee. Accept as declared; root records it in `docs/decisions.md`. Narrower: remove
  it only from a recording whose manifest has `server.max_idle_secs`, and from the other side.
- **A5.** A masked size accepts any printed value. Proto3 JSON omits 0, so only a zero is
  caught.
- **A6.** `presence()` turns `[]`, `null` and an explicit `0` into `<list>` or `<measured>`.
  Expected: an empty list or `null` is a difference.
- **A7.** The `install_md5` mask accepts any value, the empty one included, and rebuilds
  `combinedForm` from the option name. Expected: mask only `[0-9a-f]{32}`, and replace the
  value inside the existing `combinedForm`.
- **A8 (older).** `dirtiedValues` is excused with its counts although only its order varies in
  the oracle. Expected: sort the list by `skyfunctionName` in the extractor and take it off
  the allow list. `actionCacheStatistics.sizeInBytes` is excused by name: re2 in daemon mode
  prints 159140 against the oracle's 161890 and 161688.

S3:

- **B1.** A staged score has no field of its own, and `compare-scores` is silent about it.
- **B2.** `live_servers` tests `root in cmdline`. `rec-oracle-9.3.2` is a prefix of
  `rec-oracle-9.3.2-rerun` (and `rec-blaze-fix-1` of `rec-blaze-fix-1b`), so a stale pid file
  plus a reused pid could stop the other label's server. Expected: match `root + "/"` or an
  argument that ends at the root.

## Root's ruling on `garbageMetrics`

Treat it with the allow list, not as presence-exempt. Reasons: the item is then counted as
allowed and named in every score, never as matching; the extractor does not change; and
`memoryMetrics` itself must still be present. It has to go into the existing entry
`build-metrics-volatile-counters`, because `AllowList.match` excuses an item only when one
entry covers all its differing paths, and `evaluatedValues` nearly always differs too.

The exact file I would accept:
`~/.cache/blaze/work/review-referee-batch-3/proposed/scripts/oracle/allow.json`, sha256
`a1c64180371f9c213d19978fff85c9975fcb1ab4ef4dd44031556d6c8e266d97`. Against the fixer's file:

- `"buildMetrics\\.memoryMetrics\\.garbageMetrics"` appended to both `only_fields` and
  `only_fields_in_daemon_mode`;
- `condition` extended by "; or memoryMetrics.garbageMetrics is set on one side only
  (memoryMetrics itself must be present on both)";
- a new text field `jvm_reason` naming the ruling.

Tested in a private copy of the referee (referee sha256 `4afe2977…`, extractor unchanged):

| check | result |
|---|---|
| unit tests | 146 of 148 pass; the other two error only because the copy is outside the repository (the same two the fixer names) |
| oracle against itself, three pairs | `100% MATCH`, 13,858 + 58 |
| base `blaze-ef66f55e904e` | rows identical to the batch 3 base |
| `r3-374c631` (one process) | `buildMetrics` moves to allowed in both projects (allowed 56 → 58, failed 145,080 → 145,078); `compare-scores` PASS |
| `r3d-374c631` (daemon) | unchanged: `buildMetrics` still fails on `builtValues` and `postInvocationSkyframeNodeCount`; `compare-scores` PASS |

## Files root may commit

| file | sha256 |
|---|---|
| scripts/oracle/surfaces.py | 4a0bc77d0d69536d28825d53fa730c0beade138e3c1ba27acd0bc515307446f1 |
| scripts/oracle/normalise.py | bf5e4572a289851ff11e6faab7bada2c1576bcb6ee59ea5560c76d3576770267 |
| scripts/oracle/oracle_common.py | 8f7491482e3f5d9a79ad6f3dc1ddb51a4046dc6eb4f4c395d0a2ada641da9f58 |
| scripts/oracle/oracle_compare.py | c49c1ad941ddc4baa5b748fbce87ec77dae10c3eb5c49252c81d2affad37260a |
| scripts/oracle/oracle_report.py | d924e594a0ca256bbd4de0309f06567f92ceec687abce8d5dffb67a5f0f87710 |
| scripts/oracle/record | 420bfee0368130fae621a4b116529db4d55a156e54f46eca4e07f1dabb48eb08 |
| scripts/oracle/compare | aebb9b90314ac72d359d3d90f94801b9f2bc26e419f50d99d859cabebf67bc24 |
| scripts/oracle/allow.json | ade2884c39b212b66fda6fbd8d6abb67d132c207bc08f86accba6d1c95690b81 (fixer's), or a1c64180371f9c213d19978fff85c9975fcb1ab4ef4dd44031556d6c8e266d97 (with the ruling's entry) |
| scripts/oracle/README.md | feac17edcea7d9f125fa39a19c8e4b5b7ed8ce7a6c03eefd4212eaa0891dd604 |
| scripts/oracle/test_surfaces.py | 938a87f4968c53f404ac967c9dc203e4e5a998b5230ef3d7782637fbf7362b2d |
| scripts/oracle/test_normalise.py | 693d7fe78533cff8647f4ded0067d59a50d53c6752d7b663d9d5fa8f6699bc5d |
| scripts/oracle/test_record.py | 7a974d52c4e3fbe5438296c879adb59237fa014f63b8c815cc7440b675825286 |
| scripts/oracle/test_compare.py | 6ce379e45e1effc3382f54031e3719970825fb57e15f671c5f156e476851ae9f |
| corpus/oracle/brotli.json (after step 2) | bb73e23235e3b850b1e96dce8dd6a3b8e94c788e3e2d1e94d2f564d6e1e290f0 |
| corpus/oracle/re2.json (after step 2) | 76272118489eeb9f44f05473d4fb6aa7d3b1146bc2e88b37a3e009bcbc7c7dab |
| corpus/oracle/workerd.json (after step 2) | 6bc70b000b0180a0def01fc98d887eda1e5547e699ef297104ec535073e65642 |
| docs/reviews/2026-11-26-referee-batch-3.md | dd8fe5439a5456a9af12981317031116f2b101d0b044ae9c5a88a54ee116d2c7 |

The base `corpus/scores/blaze-ef66f55e904e.referee3.{json,md}` is regenerated in step 4, so it
has no hash here. The fixer's staged copy (`3e69088a…`) is what its rows are checked against.

## Steps for root

`BLAZE_ORACLE_STAGE` must be unset throughout. Do this at a consistent point: between steps 2
and 5 every other agent's comparison against `oracle-9.3.2` reports stale items.

1. Check the hashes above. To apply the ruling in the same commit (recommended, it saves a
   second rescoring): copy the proposed `allow.json` over `scripts/oracle/allow.json`. Run
   `python3 -I -m unittest discover -s scripts/oracle`: 148 pass.
2. Renormalise in place, one at a time, each inside
   `systemd-run --user --collect --wait -p MemoryMax=12G -p MemorySwapMax=0`:
   `scripts/oracle/record --renormalise --no-scope --label L --project P` for
   `oracle-9.3.2` (brotli, re2, workerd; it writes the summaries), `oracle-9.3.2-rerun`,
   `oracle-9.3.2-check` and `blaze-ef66f55e904e` (brotli, re2 each).
3. `sha256sum corpus/oracle/*.json` must print the three hashes above. Then the three
   oracle-against-itself comparisons of `scripts/oracle/README.md` must each print
   `100% MATCH` with 13,858 matching + 58 allowed.
4. Score the base again, unstaged, under a new name (never `--write`, which would overwrite the
   accepted `blaze-ef66f55e904e.json`):
   `scripts/oracle/scoreboard blaze-ef66f55e904e --allow-dirty "<the ruling of 2026-11-19, rescored under referee batch 3>" --json --out corpus/scores/blaze-ef66f55e904e.referee3.json`,
   then the same with `--markdown --out …referee3.md`. Check: no `staged:` line in `warnings`;
   `rows` and `totals` equal the fixer's staged file
   `~/.cache/blaze/work/referee-3/stage/scores/blaze-ef66f55e904e.json` (matching 13,850,
   allowed 56, failed 145,082); `referee_sha256` is `4afe2977…` with the ruling's entry,
   `c1407ce7…` without.
5. Commit as one protected-path batch (`BLAZE_PROTECTED_OK="referee batch 3: auditor PASS"`):
   `scripts/oracle`, `corpus/oracle/*.json`, the two `referee3` files, both review reports, and
   the `docs/decisions.md` lines (A4; the `garbageMetrics` ruling; no daemon-mode label accepted
   before batch 3b).
6. From then on the base for `compare-scores` is
   `corpus/scores/blaze-ef66f55e904e.referee3.json`. `referee2.json` stays in the tree for
   history only. A candidate recorded with an older extractor needs
   `record --renormalise --label <it>` before it is compared. `r3-374c631` and `r3d-374c631`
   are verification labels (dirty tree), never a base.
7. Batch 3b (A1, A2, A3) goes to the fixer and back to this role; after it the base is scored
   once more (step 4 only), and the next accepted label can be a daemon-mode recording.

## Left on disk

`~/.cache/blaze/work/review-referee-batch-3/`: `proposed/` (the referee copy with the proposed
`allow.json`) and my score and comparison outputs. My stage copy of the recordings (1.8 GB) is
deleted.
