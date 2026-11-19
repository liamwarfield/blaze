# Response to the integrity audit: referee and gate

Date 2026-11-18. Covers findings S1-1 (the referee's part), S2-2, S2-3, S2-6 and recommendations
1 to 3 of `2026-11-18-integrity-audit.md`. The other findings (Rust, README, STATUS) are not
answered here.

Tree: HEAD `88dbfa60f523aa6271a0f7407d5df20b980934f3` plus uncommitted changes in
`scripts/oracle/`, `scripts/gate/`, `scripts/git-hooks/` (new), `corpus/oracle/`, `docs/state/`
(new) and this file. Nothing is committed and no Rust was edited. These are protected paths; root
authorized the changes listed below and the auditor re-reviews the diff.

## Summary

| finding | state | where |
|---|---|---|
| S1-1 order of cquery/aquery not compared | referee closed; Blaze now shows DIFF. The Rust fix (port Java's order) is open | `surfaces.py`, README "Order items" |
| S2-2 evidence not bound to a commit, labels reused | closed for new recordings; existing Blaze labels predate it and say "repo commit not recorded" | `record`, `oracle_common.repo_state`, `compare`, `scoreboard` |
| S2-3 missing project passes as clean | closed | `oracle_compare.compare_labels`, `Comparison.failures` |
| S2-3 no order items | closed for cquery_label_kind, cquery_jsonproto, aquery_actions | as S1-1 |
| S2-3 allow entries without `max_items` | closed: required, per project | `allow.json`, `AllowList.prepare` |
| S2-3 `shutdown` exit code, `_STDERR_KEEP`, `paths_only` boundary | not changed (not in the authorized batch; each changes the reference recording) | open, below |
| S2-3 README counts allow-listed items as matches | scoreboard now reports `allowed` apart from `matching`; README is root's | open for root |
| S2-6 hand-spliced workerd reference | declared in the manifest and the committed summary; printed; never a pass | `exceptions`, README "Declared exceptions" |
| rec. 1 numbers reproducible from a commit | tooling done; no clean-commit Blaze label recorded yet | `docs/state/README.md` |
| rec. 2 close the referee gaps | done except `shutdown` | above |
| rec. 3 per-name ratchet, protected paths | done; first baseline written from HEAD; hooks written, not installed | `scripts/gate`, `scripts/git-hooks`, `docs/state` |

## What changed

### Order (S1-1, S2-3)

Verified on `oracle-9.3.2`, `-rerun`, `-check`, brotli and re2, before adding anything:

- `raw/cquery_deps_label_kind.stdout`: byte-identical in all three (176 and 3,295 lines).
- `norm/cquery_jsonproto.json`: byte-identical (results, configurations, ids).
- `norm/aquery_deps_jsonproto.json`: `targets`, `artifacts`, `depSetOfFiles`, `pathFragments`,
  `configuration`, `ruleClasses` identical including ids; `actions` identical position by
  position except `actionKey` of the runfiles manifest actions (2 and 54, the existing
  allow-list entry). No sub-output was found non-deterministic, so none is left out.

New items (one per list, so a wrong order costs one item and the per-action signal stays):
`cquery_label_kind`: `__order__ deps`; `cquery_jsonproto`: `__order__ results`,
`__order__ configurations` (`[id, checksum]`); `aquery_actions`: `__order__ actions`,
`__order__ action ids` (per action its id-valued fields), and `__order__ <list>` for the six id
lists above plus `aspectDescriptors`, as printed, ids included. With the per-action items they
decide the whole normalised aquery container. aquery order values are objects
(`{"order": [...]}`) because `scripts/w6/replay-spawns` reads every item of that file as an
action record.

All recordings used below were rebuilt with `record --renormalise`. Of the 28 `norm/` and
`items/` files per project, exactly three hashes changed (`items/cquery_label_kind.json`,
`items/cquery_jsonproto.json`, `items/aquery_actions.json`); the other 25 were reproduced
byte-identically, in every recording. `corpus/oracle/{brotli,re2,workerd}.json` carry the new
hashes and counts.

### Missing evidence fails (S2-3)

- Status `NOT_COMPARED` is gone. A project or surface the candidate lacks is `MISSING` and each
  reference item counts as failed; one only the candidate has is `NO_REFERENCE`; both fail
  `compare` and show in `scoreboard`. Recorded by neither side is `NOT_RECORDED`: excluded and
  listed. `compare` passes only if every surface in scope is `MATCH` and something was compared.
- The scope is every project of either label. `--project`/`--surface` restrict it; the result
  line says `[restricted to ...]`, what was left out is listed with item counts, and a restricted
  run writes its own report file (it used to overwrite the full one).
- 0 items against 0 items is `MATCH (empty)` only when the surface's commands ran on both sides,
  succeeded on the reference and exited the same on the candidate; otherwise `FAILED`.
- An `items/` file must match the sha256 in its manifest, and the reference must match
  `corpus/oracle/<project>.json`; otherwise `FAILED`. The auditor's manual check of the reference
  is now part of every comparison.
- `match%` is `matching / (total - allowed)` over every reference item in scope, rounded down.

### Allow list (S2-3)

`max_items` is required and per project: `runfiles-manifest-action-key` brotli 2, re2 54;
`build-metrics-volatile-counters` brotli 1, re2 1 (the counts of all three oracle pairs). A
project that is not named has the cap 0, so nothing is excused for workerd. Over the cap, the
entry excuses nothing and the comparison fails. An entry without `max_items` is refused.

### Commit binding (S2-2, rec. 1)

- `record` writes `blaze_repo` into the manifest: `git rev-parse HEAD`, dirty flag, number of
  dirty paths and a sha256 of the uncommitted state (`git diff HEAD --binary` plus every untracked
  file), next to the binary's sha256, path and the date.
- `record` refuses an existing label/project (even an unfinished directory) and refuses to add a
  project recorded with another binary to a label. `--force` overrides and the manifest says what
  was replaced.
- `compare` and `scoreboard` print binary sha256, date, commit and dirty flag of both sides per
  project, warnings (commit not recorded, dirty tree, mixed binaries, items built by another
  extractor version), and the denominators: total, compared, matching, allowed, failed/missing,
  extra, surfaces per status, excluded with reason.
- `scoreboard <label> --markdown | --json`, and `--write` for `corpus/scores/<label>.json|md`
  (deterministic; no restriction allowed).
- `record --renormalise` now builds in a staging directory and renames into place.

### workerd reference (S2-6)

`oracle-9.3.2/workerd/manifest.json` and `corpus/oracle/workerd.json` declare
`exceptions: [{id: workerd-aquery-spliced, kind: spliced, surfaces: [aquery_actions, exit_codes],
protocol_exit_code: 8, stored_exit_code: 0, protocol_recording: oracle-9.3.2-workerd, ...}]`.
`compare`/`scoreboard` print it; both surfaces are compared, marked `*`, and never count as a
pass. `compare oracle-9.3.2 oracle-9.3.2-workerd --project workerd` shows the splice:
`exit_codes DIFF* aquery_deps_jsonproto 0 vs 8`, `aquery_actions FAILED*`.

### Gate (rec. 3)

- `scripts/gate/run-tests <out> [--checkout DIR]`: names are now
  `<package>::<target>::<test>`, the package taken from Cargo's artifact messages. The old key
  used the executable's name only, and HEAD has three pairs of crates with the same test file
  name (`label_test`, `feature_set_test`, `bazel_module_resolution_function_test`), whose results
  overwrote each other. The parser also survives test output glued into libtest's lines and
  checks every executable's parsed names against its `test result:` counts (a first run lost one
  name this way; that is now a reported problem, not a shorter result).
- `scripts/gate/compare-tests`, `scripts/gate/baseline` (refuses dirty trees, problems, lost
  names), `scripts/gate/compare-scores` (per-row ratchet between committed scores).
- `docs/state/tests-baseline.json.gz`: commit `88dbfa60f523aa6271a0f7407d5df20b980934f3`, clean
  `git worktree`, 6,066 names in 322 test executables: 3,973 pass, 1 fail
  (`blaze_sky_exec::test/action_execution_function_test::file_write_then_symlink_builds_and_sets_output_permissions`),
  2,092 ignored. Run twice; both runs give the same result for every name and the same baseline
  bytes (sha256 `7abd0509…4111c`). The dirty working tree did not compile at the time
  (`blaze_repo_rules`), which is why a clean checkout was used.
- `scripts/gate/protected-paths.txt`, `scripts/git-hooks/pre-commit`, `commit-msg`, `install`:
  a commit touching a protected path needs `BLAZE_PROTECTED_OK="<batch>: <why>"`; the text
  becomes a `Protected-Paths-OK:` trailer. The list is read from HEAD, so a commit cannot shorten
  the list it is judged by. `corpus/scores/` is append-only. **Not installed** (it changes the
  clone's git config; root runs `scripts/git-hooks/install`).

## Oracle against itself, after the changes

| comparison | scope | result |
|---|---|---|
| `oracle-9.3.2` vs `-rerun` | brotli, re2 (explicit) | 100% MATCH: 26 of 26 surfaces, 13,916 items = 13,858 matching + 58 allowed |
| `oracle-9.3.2` vs `-check` | brotli, re2 (explicit) | 100% MATCH: same numbers |
| `-rerun` vs `-check` | everything | 100% MATCH: same numbers |
| `oracle-9.3.2` vs `-rerun`, `-check` | everything | FAIL: workerd was recorded once, so its 8 surfaces are MISSING in the others (and 2 are under the exception). This is the new rule working |

## Blaze, existing labels

Both labels predate commit binding: no commit recorded, uncommitted trees.

`blaze-int` (binary sha256 `4dcec429db10`, `target/debug/blaze`, recorded 16:48 and 16:54 UTC;
the label had been re-recorded in place several times before the refusal existed):

| scope | surfaces | reference items | match% |
|---|---|---|---|
| brotli + re2 | 26: MATCH 19, DIFF 7 | 13,916 = 13,834 matching + 58 allowed + 24 failed; 1 extra | 99.8 |
| all (with workerd) | 39: MATCH 19, DIFF 7, MISSING 8, NOT_RECORDED 5 | 158,988 = 13,834 + 58 + 145,096 | 8.7 |

The 7 DIFF surfaces: `cquery_label_kind` (1 item each: `__order__ deps`); `aquery_actions`
(7 items each: order of actions, action ids, targets, artifacts, depSetOfFiles, pathFragments,
ruleClasses; Blaze prints 251 depsets where Bazel prints 107 for brotli); `bep` (4 events each:
`buildToolLogs`, both `structuredCommandLine`, `unstructuredCommandLine`); re2 `build_stderr`
(1 extra warning line). Per project: brotli 989 of 1,001 (98.8%), re2 12,845 of 12,857 (99.9%).

`blaze-w4` (binary sha256 `979ea1a363e6`, no longer on disk; steps query, cquery, aquery only):

| scope | surfaces | reference items | match% |
|---|---|---|---|
| brotli + re2 | 26: MATCH 6, DIFF 6, MISSING 14 | 13,916 = 7,476 matching + 56 allowed + 6,384 failed | 53.9 |
| all (with workerd) | 39: MATCH 6, DIFF 6, MISSING 22, NOT_RECORDED 5 | 158,988 = 7,476 + 56 + 151,456 | 4.7 |

Before the changes the scoreboard printed 176/176, 3,295/3,295, 60/60 and 1,420/1,420 for the
cquery and aquery rows of both labels and skipped what was not recorded.

## Tests

`python3 -I -m unittest discover -s scripts/oracle`: 96 passed, 0 failed (38 before).
`python3 -I -m unittest discover -s scripts/gate`: 40 passed, 0 failed (new). They cover order
items, each status, the empty-surface rule, caps, tampered items, exceptions, denominators,
both tools' exit status and formats, an end-to-end `record` with a stand-in binary (commit
binding, label refusal, `--force`, staged rebuild), the test-log parser, the baseline rules, the
score ratchet, and the hooks run by git in a scratch repository.

## Open

1. S1-1 in Rust: port Java's result order for cquery and aquery, and the depset structure of the
   aquery output. The 16 differing order items (8 per project) are the target.
2. No Blaze label from a clean commit exists. After this batch is committed: build in a clean
   checkout, record with that checkout's recorder as `blaze-<commit>`, `scoreboard --write`,
   commit `corpus/scores/` (steps in `docs/state/README.md`).
3. `shutdown`'s exit code is still discarded; `_STDERR_KEEP` and the `paths_only` boundary are as
   before. Each needs the oracle re-recorded or renormalised with changed reference hashes: a
   separate named batch.
4. workerd has one oracle recording, so its allow caps are 0 and its order was not verified
   against a second run. Its `mod_graph`/`lockfile` are FAILED on the oracle itself and stay a
   failure for any candidate.
5. Recordings not renormalised (`blaze-int2`, `blaze-int-re2a`, `blaze-w4-alias`, `-dm`,
   `blaze-w4a-proto`): `compare` warns and their order items are MISSING until renormalised.
6. Oracle-driven Rust tests that pass when their recording is absent (S2-9) and a build check at
   commit time (S1-3) are Rust or Cargo work, not done here. `scripts/gate/run-tests` on a clean
   checkout does catch a HEAD that does not build: every name is then ABSENT.
7. The hooks do not stop a committer who replaces them or uses `--no-verify`; reviews read the
   diff of `scripts/git-hooks` and `scripts/gate`.

## Text proposed for AGENTS.md (root's file)

```
## Protected paths and accepted state

- Protected paths are listed in `scripts/gate/protected-paths.txt` (referee, gate, hooks, corpus
  and baselines, `docs/state`, the pin, this file, PORTING.md, the starlark-rust patch list). Do
  not edit one unless your task names it.
- Root installs the hooks once per clone: `scripts/git-hooks/install`. A commit that touches a
  protected path needs `BLAZE_PROTECTED_OK="<batch>: <why>"`. Never `git commit --no-verify`.
- A label is recorded once: `blaze-<commit>` from a clean checkout of that commit. `record
  --force` only when root says so.
- A number in README or STATUS cites its label (`corpus/scores/<label>.md`) or the baseline
  commit (`scripts/gate/baseline show`). Acceptance steps and rules: `docs/state/README.md`.
```

---

# Referee batch 2

Date 2026-11-18. Implementer for root; Python standard library and bash only, no Rust, nothing
committed by me. Root's checkpoint `12ec0c9` already contains most of these edits ("referee batch 2
edits (not yet reviewed)"); the reviewable diff is `git diff 90c1e6d -- <paths below>` plus the
untracked files `scripts/gate/check-oracle-tests`, `scripts/gate/test_evidence_tools.py`,
`docs/state/oracle-test-sites.md` and the uncommitted README edits.

Paths: `scripts/oracle/{oracle_compare.py,oracle_report.py,oracle_common.py,record,compare,scoreboard,
test_compare.py,test_record.py,README.md}`, `scripts/gate/{compare-scores,test_gate.py,
check-oracle-tests,test_evidence_tools.py}`, `scripts/w5r/compare-repos`,
`scripts/remote/{matrix,matrix_verdict.py,matrix-allow.json}`, `scripts/w4/probe`,
`docs/state/{README.md,oracle-test-sites.md}`. `normalise.py`, `surfaces.py`, `allow.json`,
`corpus/` and the recordings are unchanged.

## What changed

- **R2A-1.** `diff_paths` has no limit any more; `compare_surface` hands the allow list every
  differing path of a DIFF item and only cuts what it stores for printing (50 paths, plus
  `paths_total`). `describe_diff` counts the hidden paths from `paths_total`. Tests
  (`test_compare.AllowVolumeTest`, real `allow.json` entry): the auditor's case (250 allowed
  paths in the five per-SkyFunction lists plus `targetMetrics` and `outputArtifactCount`) is
  DIFF; one extra path sorted after 400 allowed ones is DIFF; 400 allowed paths alone are still
  ALLOWED. The two volume tests fail on `oracle_compare.py` at `90c1e6d` (200 paths, ALLOWED).
  Live check on the real re2 reference `buildMetrics` event with the same edit: old referee
  `MATCH, ALLOWED 1` (200 paths seen), new referee `DIFF 1` (247 paths).
- **R2A-2.** `record` writes `binary_origin` into the manifest: `commit_verified` is true only
  when the binary's real path is inside `<recorder checkout>/target` and its mtime is not before
  that checkout's HEAD commit time; `reasons` otherwise. It warns, it does not refuse (the oracle
  binary is never in the checkout). `compare`/`scoreboard` print `binary from that commit
  verified` / `binary origin NOT VERIFIED` / `binary origin not checked` after the commit, and a
  warning with the reasons. Not done: stamping the commit into the binary (Rust).
- **R2A-3.** `scoreboard --write` refuses (exit 2, nothing written) when a candidate project has
  no commit, a dirty tree, or a binary origin that is unverified or not checked, unless
  `--allow-dirty "REASON"`; the score always holds `recording_problems`, and `allow_dirty` holds
  the reason.
- **R2A-4.** Each score row holds `matching_keys` (sha256 of each matching item key, first 10 hex,
  sorted). `compare-scores` reports a regression when any base digest is absent from the new
  row, even at an equal count; for a score without digests it compares counts and prints a
  WARNING naming the rows. It also prints both scores' `recording_problems` and `allow_dirty`.
- **R2A-5.** For the reference label, exceptions come from the committed summary
  `corpus/oracle/<project>.json` together with the manifest's (union, so deleting one from the
  unanchored manifest does not remove it); a mismatch is a warning, and so is a summary that
  differs from `HEAD:` (not a failure, so that a re-recorded reference can be reviewed before it
  is committed). `--force` now also leaves `forced` (the overridden conflicts) in the manifest;
  `compare`/`scoreboard` print `replaced` and `forced` as warnings.
- **compare-repos.** Per project a `verdict <project>: PASS|FAIL (...)` line and a final
  `result:` line; FAIL when the fetch exits non-zero, times out (now caught) or its log has a
  Rust panic, when any repo differs, or when the workspace or the hidden lockfile is not
  byte-identical. With `--no-run` the exit code comes from the previous report; none is a
  failure. On the stored reports: brotli PASS; re2 FAIL (`fetch exited 1; fetch panicked;
  hidden lockfile DIFF`), where the old tool said 81/81 MATCH and exited 0.
- **matrix.** `scripts/remote/matrix_verdict.py` (called at the end of `matrix`, also runnable on
  an existing run) appends `verdict: PASS|FAIL (...)` and `matrix` exits with its status. Build
  steps must exit 0 with their processes line; a gRPC comparison must be MATCH, or DIFFERENT
  under an entry of `scripts/remote/matrix-allow.json` whose condition is checked on the full
  `compare-grpc-log --verbose` output (every unmatched RPC of an allowed mnemonic, at most
  `max_actions` actions, equal RPC counts per method). One entry: `re2-cold-test-actions` (C4
  vs C1 on re2, TestRunner only, 21 actions). docs/remote.md does not document it yet; its
  source is STATUS's lead-reported line, and the lead should add it to docs/remote.md. Stored
  runs: `brotli1` PASS; `re2a` FAIL, C4 vs C1 allowed (82 unmatched RPCs per side, all TestRunner,
  21 actions), **E2 vs E1 DIFFERENT and not allowed** (CppCompile, CppLink, SolibSymlink,
  CppArchive RPCs; ByteStream/Read 93 vs 27). Output names `matrix` reads or writes are
  unchanged; new files are `matrix-<tag>.verdict` and `matrix-<tag>-<step>.unmatched.txt`.
- **probe.** The totals of `compare-all` and `recount` are split into steps that run the binary
  (`run`, `aquery`) and file checks (`cat`, `stat`), and name the scenarios left out by `--skip`
  or for lack of a saved transcript. `recount 85cfe14f8e8d` (the transcripts root committed in
  `12ec0c9`): 39 scenarios, 720 steps, full 102 / masked 570 / messages 239; **binary steps 618:
  full 1, masked 469, messages 138; file checks 102: full 101.** The saved transcripts and the
  masked view are untouched.
- **S2-9.** `scripts/gate/check-oracle-tests` scans test files for print-and-return sites, maps
  each to the paths it needs (a reviewed table in the script), and exits 1 when a path is
  missing or a site is unregistered. It finds 22 sites in 15 files; on this machine one path is
  missing: `blaze_client/tests/blaze_test.rs:224` (`oracle_cmdline_file_matches_recorded_list`)
  passes today without comparing anything. The list is in `docs/state/oracle-test-sites.md`;
  `docs/state/README.md` runs the check as step 0 of an acceptance.

## Oracle against itself

Working tree on `12ec0c9` (referee sha256 `fb73e9fbfaa8…`, was `26e4400ec7d2…`):
`compare oracle-9.3.2 oracle-9.3.2-rerun --project brotli --project re2`, the same with `-check`,
and `compare oracle-9.3.2-rerun oracle-9.3.2-check`: each exit 0, `100% MATCH`, 26 of 26
surfaces, 13,916 = 13,858 matching + 58 allowed (runfiles-manifest-action-key 56,
build-metrics-volatile-counters 2). Reports went to a scratch path, not under the recordings.

## Accepted scores

`scoreboard blaze-12ca29ee95e0 --json` and `blaze-ef66f55e904e --json` with the new referee,
written to a scratch directory, not to `corpus/scores/` (append-only). **No number changes**:
rows (without the new field), `totals`, `project_totals`, `verdict`, `failures`, `allowed_by`,
`excluded` are identical to the committed files for both labels (13,850 matching of 13,858
judged; 23 of 26 brotli+re2 surfaces MATCH). The files would still change: `referee_sha256`, the
provenance gains `commit_verified: null` and the `replaced`/`forced` fields, rows gain
`matching_keys`, and the document gains `recording_problems` (both projects of both labels:
`binary origin not checked`), `allow_dirty: null` and `matching_keys_format`; the Markdown says
`binary origin not checked` and lists the problems. `compare-scores` on the two rescored files:
PASS, rows 39, regressed 0 (the same items match at both commits). The committed files and the
new referee are not comparable (different referees), as before any referee change.

For root to rule: `scoreboard --write` now refuses both labels (no `binary_origin`; the
checkouts and binaries are deleted, so it cannot be recovered). Rescoring the last accepted
label under this referee needs `--allow-dirty "<ruling>: recorded before binary_origin; R2A-2
open for this label"`, and a new file name or an exception to `append-only corpus/scores/`.

## Tests

`python3 -I -m unittest discover -s scripts/oracle`: 113 OK (96 at `90c1e6d`; new: allow-list
volume 4, anchored exceptions 2, force/origin/allow-dirty 5, matching keys 1, binary origin 4,
forced manifest 1). `-s scripts/gate`: 61 OK (40 at `90c1e6d`; new: name ratchet 4,
check-oracle-tests 5, compare-repos 4, matrix verdict 6, probe classes 2). Failed 0, skipped 0.

## Open

- R2A-2 is a file-system check; a commit stamp in the binary needs a Rust change.
- `matrix` on re2 fails on E2 vs E1 until the remote lead fixes or documents (with a checkable
  condition) the difference; docs/remote.md should carry the allowance the allow file cites.
- The 22 S2-9 sites still need the Rust change (fail, with one opt-out variable).
- R2A-6 (protected list additions, including `scripts/w5r`, `scripts/remote/matrix*`) is not
  part of this batch.
