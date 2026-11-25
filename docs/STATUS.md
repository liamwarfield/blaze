# Status

Updated 2026-11-25, 10:20 (root). Work resumed on 2026-11-25 (the user set the goal again).
Numbers cite their evidence; history is in `git log` and `docs/decisions.md`.

## State

- **Code is at `cc6c29a`** (commits after it are documents and comparison tools). It builds from
  a clean checkout. Referee guard at that code (label `pause-cc6c29a3ede8`): 39 rows, gained 2,
  regressed 0 against the accepted score.
- **Accepted baseline is still `ef66f55`.** Nothing after it has been through acceptance.
- **Outer measure, tag v0.1.0** (label `blaze-v0.1.0-r3`, full suite, fixed harness, quiet
  machine): 63 of 211 targets and 1,401 of 2,878 functions that pass on Bazel pass on Blaze.
  The run for `e8b79c0` (label `blaze-e8b79c0`, binary sha256 7455128a…) is in progress.
- **Coverage map at `e8b79c0`** (`docs/coverage/README.md`; counts port markers, a planning
  tool): about 106k ± 15k lines of needed Java not ported, 13k of them out of scope.

## Running (ten agents, the limit)

| Task | Model | After |
| --- | --- | --- |
| Server: review fixes (stale per-command state, event bus, uncaught panics), soak | Opus | re-review, then the default switch |
| Incremental builds, package I2 (84 keys) | Opus | review, then I3 |
| Commands: review fixes; `config`, `dump`, `print_action` | Opus | re-review |
| Loading and Starlark: review fixes; remaining error texts | Opus | re-review |
| Java rules and configuration: review fixes; 14 probes; 59 stubbed tests | Opus | re-review; Bazel's Java suites |
| Execution: review fixes; worker tests; dynamic execution | Opus | re-review |
| Reporting: review fixes; Build Event Service client; profiler sites | Opus | re-review |
| `coverage` command and instrumentation | Opus | review |
| Comparison tools: audit round 2 fixes and reruns on `e8b79c0` | Opus | auditor round 3 |
| Remote: round-3 evidence on `e8b79c0` | Opus | re-review by the remote reviewer |

## Next, in this order

1. Read `blaze-e8b79c0` with `scripts/upstream-tests/compare` and rank its failures
   (`scripts/ut-analysis/classify`, `families`); the ranking decides the next implementers.
2. As each fixer reports: checkpoint (tree compiles, protected paths staged out), referee guard,
   then the same reviewer role re-checks its area.
3. Not started, to hand out as agents free up: native aspects, `genquery` and SkyQuery, cquery
   `starlark`/`transitions`, extra actions, include scanning, Docker sandbox and cgroups, action
   rewinding, vendor mode, `testing.analysis_test`, remote HTTP cache and credential helpers,
   the remaining files of the clusters in `docs/coverage/README.md`.
4. New accepted commit when every area has a PASS: clean-checkout score including workerd, test
   gate with name maps, auditor approval, baseline move.
5. Process rules: at most ten agents; Fable for reviewers, auditors and leads, Opus for
   implementers; nothing but small notes under `/tmp`; delete only what you created; agents do
   not run `scripts/upstream-tests/run`; a number from a comparison tool enters README or this
   file only in the auditor's wording.

## Waiting on the user

- Whether to delete the remote lead's old run directories (about 160 GB under
  `~/.cache/blaze/oracle-out/ur-remote` and `~/.cache/blaze/work/remote`; keep-list in the
  lead's report). (The leftover remote-worker processes ended with the reboot.) Disk was at
  116 GB free on 2026-11-25 before a target cleanup (150 GB after).
- Whether the vendored starlark-rust crates' own unit tests may be run (the command was
  refused by the permission check; AGENTS.md forbids bare Cargo and a separate target
  directory).
- How identical to Bazel the console and error text must be (unruled; everything is ported to
  match).

## Outer measure

Bazel's own integration suite (`docs/upstream-tests.md`); the oracle passes 211 targets and
2,878 functions. Tag v0.1.0: 63 targets, 1,401 functions (label `blaze-v0.1.0-r3`). The first
run's total is withdrawn (`docs/reviews/2026-11-19-audit-3.md`).

## Reviews (docs/reviews/, Fable 5.2, read-only)

| Area | Last round | Verdict |
| --- | --- | --- |
| Referee and gate scripts | 2A; audit 3 batch A (upstream harness) | PASS |
| Integrity audit (whole) | 2B | PASS; baseline move approved |
| Evaluator (blaze_skyframe) | 2 | PASS |
| Loading, query, Starlark bridge | 3 | PASS |
| Actions, action keys, C++ feature engine | 2 | PASS |
| Bzlmod, repositories, downloader, client | 4 | PASS |
| Analysis, configuration, cquery/aquery | 3 | PASS |
| Execution, sandbox, build driver | 4 (at ef66f55) | PASS |
| Remote cache and execution | 3 (at e370bf9) | STOP: P1 (SIGPIPE); fixed, evidence being completed, re-review owed |
| Comparison tools | audit 4 round 2 (2026-11-25) | STOP: B1-B5, and SV-1..6 in `scripts/server`; interim files committed (74662dd); harness lock fix PASS |
| Byte strings | 2 (at 780fceb) | PASS |
| Incremental evaluator, package I1 | 1 (at e8b79c0) | PASS |
| Server and client | 1 (at e8b79c0) | STOP: F1, F2 |
| Commands and options | 1 (at e8b79c0) | STOP: F1 |
| Loading, macros, Starlark front end | 1 (at e8b79c0) | STOP: F1 |
| Java rules and configuration | 1 (at e8b79c0) | STOP: S1-1 |
| Execution: workers, tests, templates | 1 (at e8b79c0) | STOP: E2-1, E2-2, E2-3 |
| Reporting, build events, query outputs | 1 (at e8b79c0) | STOP: S1-1, S1-2 |

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
