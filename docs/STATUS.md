# Status

Updated 2026-11-19, after tag v0.1.0 (root). Numbers cite their evidence; history is in `git log` and
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

## Outer measure

Bazel's own integration suite (`docs/upstream-tests.md`), binary of tag v0.1.0: 62 of 211
oracle-passing targets and 1,422 of 2,878 oracle-passing functions. Largest causes: Java rules
(395 functions), `run` (175, ported since), `coverage` (124), `fetch` (43, ported since).
Coverage map (`docs/coverage/README.md`, at a79275e): about 139k ± 15k lines of needed Java
code not yet ported, of 406k.

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
| Remote cache and execution | 2 | STOP: N1-N4 (tree values and exec root through the action file system, capabilities, execution properties); fixes in, re-review (round 3, at e370bf9) in progress |
| Byte strings | 2 (at 780fceb) | PASS |

Open items the reviewers left as non-blocking are listed at the end of each report.

## workerd (third project, user request)

On the untouched checkout, with the binary of the tree that became `2574fef` (not a
clean-checkout binary, not a referee recording): `blaze build //src/workerd/server:workerd`
exits 0 with Bazel's summary line (9,713 processes: 3,333 internal, 6,379 linux-sandbox, 1
local); `bazel-bin/src/workerd/server/workerd` has the same sha256 as Bazel's (cdedf3f9…,
hashed by root from both output bases); query 46,371 of 46,371 and cquery 38,892 of 38,892
under the referee protocol. aquery was last compared on the ASCII-README copy (13,416 actions,
13,160 identical, 256 allow-listed runfiles keys) and not rerun since. Details:
`docs/workerd.md`.

## Not accepted yet

Everything after `ef66f55`: workerd fixes, Starlark aspects, the byte-strings migration, the
remote round-2 fixes. Last test gate on an uncommitted tree: 6,601 pass, 6 fail (all in
`crates/blaze_remote/tests/remote_execution_service_test.rs`). Tag `v0.1.0` (5a4e4a0) marks
this state; it is not an acceptance.

## In flight

- **Remote lead** (Fable): finishing round 2 (byte-string sites in the remote files, the 6
  failing tests, final evidence), then re-review round 3.
- **Byte-strings reviewer** (Fable, read-only): review of `2574fef`.
- **Opus implementers**, one each: `run`; `mod dump_repo_mapping` / `show_repo` / `tidy`;
  protobuf TextFormat and `--build_event_text_file`; `help` topics, `fetch`,
  `canonicalize-flags`, `license`.
- **Measurement** (Opus): first full run of Bazel's integration suite against the v0.1.0
  binary; result and ranked causes go to `docs/upstream-tests.md`.

Missing features: `docs/missing-features.md`.

## Next

1. Remote re-review, byte-strings verdict, then a new accepted commit (clean-checkout score
   including workerd, test gate, auditor approval, baseline move).
2. Next wave ordered by `docs/upstream-tests.md`; execution-crate items (exclusive tests,
   `unused_inputs_list`, compile action templates) once the remote lead is out of those crates.
3. Persistent server (W8), critical path, remaining build-event items.
4. Auditor's open list for the referee; test harness for analysis and query tests; stale ignore
   reasons; remaining silent "not ported" sites.
