# Status

Updated 2026-11-19 (root). Numbers cite their evidence; history is in `git log` and
`docs/decisions.md`.

## Accepted state

- **Baseline commit:** `ef66f55` (builds from a clean checkout).
- **Oracle score** (`corpus/scores/blaze-ef66f55e904e.md`; clean-checkout binary; brotli
  `build //...` and re2 `test //:all //python:all`): 23 of 26 surfaces MATCH, 13,850 of 13,858
  compared items, 58 allow-listed. Open: 4 build-event items per project, 1 extra stderr line on
  re2. workerd is recorded on the oracle only and counts as MISSING in the score file.
- **Rust tests** (`docs/state/tests-baseline.json.gz`, `scripts/gate/baseline show`): 5,487
  pass, 0 fail, 2,075 ignored (about 320 ignore reasons are stale; there is still no test
  harness for analysis and query tests).
- **Error and UI scenarios** (`scripts/w4/probe`, 35 scenarios with oracle transcripts): 94 of
  694 steps identical, but all except one of those are file checks; of the 618 steps that run
  the binary, 1 is identical in full and about 225 agree on message text. Known causes: no
  critical-path line, no persistent server, process counts after failed analysis, error
  locations.

## Reviews (docs/reviews/, Fable 5.2, read-only)

| Area | Last round | Verdict |
| --- | --- | --- |
| Referee and gate scripts | 2A | PASS |
| Integrity audit (whole) | 2B | PASS; baseline move approved |
| Evaluator (blaze_skyframe) | 2 | PASS |
| Loading, query, Starlark bridge | 3 | PASS |
| Actions, action keys, C++ feature engine | 2 | PASS |
| Bzlmod, repositories, downloader, client | 4 | PASS |
| Analysis, configuration, cquery/aquery | 3 | PASS |
| Execution, sandbox, build driver | 4 (at ef66f55) | PASS |
| Remote cache and execution | 1 (at ef66f55) | STOP: six S1 in the handling of remote-only outputs |

Open items the reviewers left as non-blocking are listed at the end of each report.

## workerd (third project, user request)

At checkpoint a602a17's tree, on a work copy with one emoji removed from README.md:
`blaze build //src/workerd/server:workerd` exits 0 with Bazel's summary line (9,713 processes:
3,333 internal, 6,379 linux-sandbox, 1 local); `bazel-bin/src/workerd/server/workerd` has the
same sha256 as Bazel's (cdedf3f9…, re-hashed by root from both output bases); cquery 38,892 of
38,892 with order; aquery 13,416 actions, 13,160 identical, 256 allow-listed runfiles keys
(lead's comparisons with the referee's extractors; not referee recordings). On the pristine
checkout cquery/aquery fail at one telemetry rule that needs Bazel's byte-string semantics.
Details: `docs/workerd.md`.

## In flight

- **Byte-strings lead** (Fable; Opus workers): mirror Bazel's internal string form
  (`docs/research/07-byte-strings.md`), so workerd passes the referee protocol unmodified.
- **Remote lead** (Fable; five Opus workers): remote review round-2 STOP (N1-N4 and S2s);
  first run of Bazel's remote shell tests against Blaze: 110 of 253 oracle-passing functions.
- Unassigned, from that run: `mod dump_repo_mapping` (33 tests), `--build_event_text_file`
  (13), createCppCompileActionTemplate (6), `fetch` (4), `unused_inputs_list` panic (4),
  exclusive tests (3), command.profile.gz / critical path in BEP; blaze_vfs ENOTDIR handling.

## Next

1. Auditor's open list for the referee: allowed diffs can hide a non-allowed one past 200
   (`diff_paths` limit); `compare-repos` and `remote/matrix` exit 0 on differences; name-level
   score ratchet; refuse dirty trees for `scoreboard --write`; oracle-driven Rust tests that
   pass when recordings are missing.
2. Byte-strings migration (`docs/research/07-byte-strings.md`), then workerd end to end (aspects
   are its next blocker; `docs/workerd.md`).
3. Remaining build-event items, critical path, persistent server (W8).
4. Bazel's own integration suite against Blaze (`scripts/upstream-tests`; oracle passes 211 of
   237 targets on this host).
5. Test harness for analysis and query tests; stale ignore reasons; remaining silent
   "not ported" sites (152, of which a sample of 40 found 4 silent).
