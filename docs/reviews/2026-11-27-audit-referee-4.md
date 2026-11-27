# Audit of referee batch 4 (2026-11-27), area `referee-batch-4`

Regression auditor, read-only. Subject: the uncommitted diff `git diff HEAD -- scripts/oracle
corpus/oracle` (HEAD `8e10c3af`; `scripts/oracle` at HEAD is batch 3b, `bbbc6fa3`), the new
`corpus/scores/blaze-ef66f55e904e.referee4.{json,md}`, the in-place renormalisation of 13
recordings, and the fixer's report `docs/reviews/2026-11-27-referee-batch-4.md` (sha256
`60decf41…`).

**Verdict: PASS**, bound to the file hashes in "Files root may commit", with conditions K1 to
K4 below. The shutdown exemption (question 4) is **accepted with a condition** (K2 now, D1 in
code at the next referee change).

No bazel or blaze process was started by me. Python only, on existing recordings. Nothing
written outside this report and my scratchpad. The machine was shared (load about 110), which
changes no result.

## What I checked

- **Hashes.** All 12 files of the fixer's table match the working tree. Referee `e7130cdb…`,
  extractor `fb3ca2d0…`, computed by me. `scripts/gate` and the other eight files of
  `scripts/oracle` equal HEAD.
- **Patch.** `next-extractor-A5-A8.patch` has sha256 `811b2a54…` (179 lines). HEAD with the
  patch applied (in my scratchpad) equals the tree byte for byte in `allow.json`. In
  `surfaces.py` it differs only in the A2 line of `s_build_stderr`; in the two test files only
  in the new tests and the one adjusted test.
- **Unit tests.** `scripts/oracle`: 174 run, 174 passed. `scripts/gate`: `test_gate.py` 30/30,
  `test_protected.py` 15/15, `test_evidence_tools.py` 16/17.
- **Renormalisation**, my own script over `backup/` and the live recordings, all 13:

  | recordings | manifest keys changed | files changed | items changed |
  |---|---|---|---|
  | `oracle-9.3.2`, `-rerun`, `-check`, `guardd-8ecc520` (brotli, re2: 8) | `extractor_sha256`, `files`, `renormalised` | `items/bep.json`, `norm/bep.json` | `{"buildMetrics":{}}` only; new item = old item with `dirtiedValues` sorted |
  | `oracle-9.3.2/workerd`, `blaze-ef66f55e904e`, `guard-8ecc520` (5) | `extractor_sha256`, `renormalised` | none | none |

  - In `norm/bep.json` the one changed event differs only in the order of `dirtiedValues`
    (same multiset, 23 entries, sum 1649).
  - Each backup's `items/` and `norm/` match the backup manifest; each live one its live
    manifest. One `renormalised` entry was added to each.
  - Not re-recorded: `created`, `commands` and every `raw/` hash in the manifests are unchanged,
    every live raw file matches its manifest hash, and the newest raw file of each recording is
    older than the batch.
  - `corpus/oracle`: the backups equal HEAD. brotli and re2 change in `extractor_sha256` and the
    two `bep` hashes, workerd in `extractor_sha256` only; all three agree with the live
    manifests.
- **Base recomputed.** My `scoreboard blaze-ef66f55e904e --json` and `--markdown` outputs are
  byte-identical to the fixer's `referee4` files. Against `referee3b.json`: 39 rows, none moved
  (`matching_keys` included); totals 13,850 matching, 56 allowed, 145,082 failed, 13,860
  compared, 1 extra, of 158,988. Only `referee_sha256`, `allow_dirty`, `provenance` and
  `warnings` differ, as the fixer lists. `staged` is `null`.
- **Guards.** My scores are byte-identical to the fixer's. `scripts/gate/compare-scores` against
  the new base:
  - `guard-8ecc520`: 13,852 matching, 58 allowed, 13,858 compared; PASS, rows 39, gained 2,
    regressed 0.
  - `guardd-8ecc520`: 13,858 matching, 56 allowed, 13,860 compared; PASS, rows 39, gained 2,
    regressed 0.
- **Self-checks.** `oracle-9.3.2` vs `-rerun`, vs `-check`, and `-rerun` vs `-check`: `100%
  MATCH`, 13,916 = 13,858 + 58 allowed, each now under the daemon-mode rules.
- **Old referee against new, item by item.** HEAD's code on the backups (staging area of
  links) and the tree's code on the live recordings, for six pairs: the base, both guards and
  the three self-checks. No item changed class (matching, allowed, failed) in any pair. Only the
  differing paths inside the `buildMetrics` item shrank (the `dirtiedValues` order paths are
  gone).
- **Survey of all 134 recordings on disk.** Strictness changed for the 8 oracle recordings only
  (lenient to strict, from stderr). No Blaze recording changed. Every command of every recording
  has its raw stderr. The only recordings that are strict and have no `server` record are the 8
  oracle ones; all have the pin's binary sha256 `2e14c55c…`.
- **Evasion cases.** The manifest of `guardd-8ecc520` altered in memory, scored, gated against
  the new base:

  | case | strict | `exit_codes` | brotli `bep` | gate |
  |---|---|---|---|---|
  | a. as recorded | yes | MATCH 10/10 | DIFF 41 | pass |
  | c. shutdown exit 1; a server alive; record without the shutdown fields | yes | DIFF, 1 extra | DIFF 41 | regression |
  | e. mode unset, no pid seen, start line as printed (3b's last row) | yes | DIFF 1/10 | DIFF 41 | regression |
  | f. mode unset, manifest claims a start line on every command | yes (stderr) | DIFF 1/10 | DIFF 41 | regression |
  | i. mode unset, pid seen | yes | MATCH | DIFF 41 | pass, correctly |
  | d. no `server` record and no per-command fields (old recorder) | yes (stderr; lenient in 3b) | MATCH, shutdown not checked, note in `warnings` | DIFF 41 | **passes, shutdown unknown** |
  | b. `server` record removed, per-command fields kept | yes | MATCH, shutdown not checked, note | DIFF 41 | **passes, hides case c** |
  | h. as d, and no raw stderr | **no** | MATCH | **MATCH 41** | **passes leniently** |
  | g. as f, and no raw stderr | **no** | MATCH | **MATCH 41** | **passes leniently** |

## Answers

1. **The patch is the one I read, applied whole.** A5 to A8 each do what was asked and no more:
   - A5 masks a size only when it is a string of decimal digits.
   - A6 keeps `[]`, `null`, `0`, `"0"`, `false`. It refines the old classes and merges none.
   - A7 masks `install_md5` only for 32 lowercase hex digits; a `combinedForm` that does not end
     in `=<value>` is kept and differs.
   - A8 sorts `dirtiedValues` and takes it off the daemon list. The oracle's three runs show
     the order varying with equal counts, so the sort is backed by the oracle.
   - Nothing is more lenient: the daemon field list is a subset of `only_fields`, the strict
     rules only add items and differences, and no item of the six pairs changed class.
2. **C1 is done as specified.** Strictness no longer needs the pid file (rows e, f, d). The
   fixer's extra step, reading raw stderr even when the manifest has states, is sound: the
   trigger is an OR over the sources, so one more source can only add strictness (row f). What
   stays lenient is a recording with no evidence at all (rows g, h): finding D2.
3. **C2 is done.** Both hashes cover `oracle_common.py`, and the hashed files import no other
   local module. Still outside: `oracle_report.py` and `record` (finding D3).
4. **Ruling: accepted with a condition.**
   - The exemption is needed: without it the three self-checks fail on a shutdown that the old
     recorder never recorded, and re-recording was not allowed.
   - It is keyed on the absence of the `server` record, not on what the recording is, so a
     Blaze candidate can fall under it (rows b, d). Two ways: a recorder older than batch 3
     (`b94556aa`), or a manifest that lost the record.
   - Neither happens with today's recorder (`record:500` always writes the record), and
     `docs/state/README.md` step 2 records with the recorder of the commit being accepted. No
     Blaze recording on disk falls under it. So it is masked, not live.
   - Condition now: K2. Fix expected: D1.
5. **Confirmed**, see the table. No recording was re-recorded.
6. **Confirmed**: 0 of 39 rows moved; the totals and both guard results are as stated.
7. **Tests.**
   - Of the 10 new tests, 8 test the rule through `compare_labels` or `s_build_stderr` and
     assert the outcome. The fixer's old-code log shows these 8 failing and the two guard tests
     passing on both trees; I read the log and did not rerun it.
   - Weak ones are in S4.
   - The `test_evidence_tools.py` failure is outside this batch: the unregistered site
     (`crates/blaze_exec/tests/sandbox_docker_sandboxed_spawn_runner_test.rs:167`) is in HEAD,
     committed in `9b5671c8`, the file is unmodified in the tree, and `scripts/gate` is
     untouched.
8. See "Wording for root".

## Findings

S1: none.

S2 (masked today; not blocking):

- **D1. The shutdown exemption applies to any candidate without a `server` record**
  (`oracle_common.shutdown_recorded`, `oracle_compare.py:478`). Expected: the exemption only
  for a recording of the reference's own binary, that is, the candidate manifest's
  `binary_sha256` equals the pin's `oracle_sha256` in `UPSTREAM.json`. Every other strict
  candidate without the record gets `__shutdown__` (EXTRA), as A1 said. A test for each side.
  This changes nothing for the 8 oracle recordings or for any Blaze recording on disk.
- **D2. A command with no evidence counts as having started a server**
  (`started_no_server`; pinned by `test_c1_strict_server_rules_sources`). Rows g and h: with no
  raw stderr and no `start_line`, a server recording is judged by the one-process rules and
  `bep` scores MATCH. Not reachable with today's recorder, which always writes both. Expected:
  a command with neither source makes the recording strict; its state is then unknown, which
  differs from the reference's known state.

S3:

- **D3.** `oracle_report.py` builds the score document (`matching_keys` digests,
  `recording_problems`, `staged`) and `record` writes the fields the strict rules read; both
  are outside `REFEREE_FILES`. Expected: `oracle_report.py` in `REFEREE_FILES`. `record` stays
  out (a label is recorded once, by its own checkout) and is covered by the protected-path rule.

S4:

- `test_c1_raw_stderr_is_read_even_when_the_manifest_has_states` asserts the flag only, not the
  resulting DIFF (row f shows it).
- `RefereeHashTest` names five functions. A test over the local imports of the hashed files
  would also catch a new module.
- `allow.json` `daemon_mode_reason` says "the next extractor change (batch 3b, A8)"; it is
  batch 4. Change it only with the next referee change (it moves the hash).
- A reference whose own build started the server now keeps the start line, and a one-process
  candidate then misses it (stricter, not lenient). No recording has this shape: the brotli and
  re2 builds are warm and workerd has no build step.

## Conditions

- **K1, before the commit.** `sha256sum` of the 12 files equals the table below.
- **K2, at every acceptance from now on.** A score that is written with `--write`, or passed
  as `<new>` to `compare-scores` for an acceptance, carries no exemption note:
  `grep -c "final shutdown is not checked" corpus/scores/<label>.json` prints `0`. Otherwise the
  label is recorded again with the current recorder. (`referee4.json`: `0`, checked.) The note
  is expected only in the self-checks of `oracle-9.3.2-rerun` and `-check`.
- **K3, if root scores the base again after the commit.** The new JSON must equal the bound one
  with exactly the three "differs from its committed version at HEAD" warnings removed:

      python3 -c 'import json,sys; a,b=(json.load(open(p)) for p in sys.argv[1:3]); a["warnings"]=[w for w in a["warnings"] if "differs from its committed version at HEAD" not in w]; print(a==b)' <bound referee4.json> <new referee4.json>

  must print `True`. Committing the bound files as they are is also fine.
- **K4, with the next referee change.** D1, D2 and D3, then the three self-checks, the base
  scored once more, and this role re-checks. D1 and D2 must be in before a recording made by
  any recorder other than the current one is scored for an acceptance.

## Wording for root (README, `docs/STATUS.md`)

- The ratchet base is `corpus/scores/blaze-ef66f55e904e.referee4.json`: label
  `blaze-ef66f55e904e`, one process per command, scored by referee `e7130cdb…` from its
  existing recordings (renormalised, not re-recorded). 13,850 of 158,988 reference items match,
  56 are allowed, 145,082 fail; 13,860 were compared. `referee3b.json`, `referee3.json` and
  `referee2.json` are history only; `compare-scores` refuses them against a batch 4 score.
- A recording is judged by what it shows, not by the mode the recorder was asked for. If any
  command did not start a server (mode `daemon`, or no start line, or a warm server seen), the
  daemon-mode rules apply: server state per command, the final shutdown, the start line in
  `build_stderr`, and the narrowed `buildMetrics` fields. If every command printed the start
  line (one-process Blaze), the one-process rules apply, as for the base.
- A daemon-mode recording, or a mode-unset recording of a Blaze whose default is the server,
  may be accepted as a score and become the base, under the usual label rules (clean checkout,
  its own binary and recorder, `blaze-<commit>`) and K2.
- C3: once a score judged by the daemon-mode rules is the base, the ratchet does not go back.
  A one-process recording then regresses on `bep` (brotli 41 to 38 matching, re2 113 to 110 at
  `8ecc520`), because the cold-server fields are excused only under the one-process rules.
  After that, one-process recordings are for diagnosis only, never a score or a base.
- `guard-8ecc520` and `guardd-8ecc520` are guard labels from a dirty tree with a binary outside
  `target/`: never a score or a base.
- Any other label on disk still has an older extractor (`c215d2d7…` or `cdf91c84…`); run
  `record --renormalise` on it before comparing it.

## Files root may commit

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
| docs/reviews/2026-11-27-referee-batch-4.md | 60decf415b010bf85bed4ce5a3be5233b5114ee01843bb0034e1e25fa906ce67 |

The renormalised recordings under `~/.cache/blaze/oracle-records/` are bound through the three
summaries (reference) and through the manifests I checked (the other ten). `backup/` may be
deleted by root after the commit.
