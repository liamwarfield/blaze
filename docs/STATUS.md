# Status

Updated 2026-11-19, 17:40 (root). **Paused at the user's request.** No agent is running. Numbers
cite their evidence; history is in `git log` and `docs/decisions.md`.

## State at the pause

- **HEAD `cc6c29a`** builds from a clean checkout (`cargo check --workspace --all-targets`).
- **Referee guard at HEAD** (label `pause-cc6c29a3ede8`, binary `target/debug/blaze` sha256
  7455128a…, tree dirty only in uncommitted scripts): against the accepted score, 39 rows,
  gained 2 (`command.profile.gz` in the build event stream, both projects), regressed 0.
- **Accepted baseline is still `ef66f55`.** Nothing after it has been through acceptance.
- **Uncommitted on purpose:** the hardened comparison tools (`scripts/lib/compare_common.py` and
  about 15 tools; hashes in the fixer's report, summarised in docs/decisions.md). They wait for
  the auditor's re-check of audit round 4 (STOP).
- **Left running:** the detached full-suite run of the v0.1.0 binary (`systemctl --user status
  root-suite-v010-r3`, label `blaze-v0.1.0-r3`, log `~/.cache/blaze/work/root-runs/v010-r3.log`),
  started 17:24 on a quiet machine. Read it with `scripts/upstream-tests/compare`.

## To resume, in this order

1. Read the result of `blaze-v0.1.0-r3` and put the function total into this file and
   `docs/upstream-tests.md` (the first run's total is withdrawn).
2. Auditor (Fable): re-check the comparison-tool hardening (diff from `2e826b4`; findings F1-F6
   of `docs/reviews/2026-11-19-audit-4.md`), root's harness deadlock fix in
   `scripts/upstream-tests/ut_common.py` (7e4c3bd), and `scripts/server`. Then commit the tools
   and replace the comparison wording in README with the audited footers.
3. Remote, round 3 (STOP, blocking fix in): finish the evidence on binary
   `~/.cache/blaze/work/remote/tl-blaze-2` (rest of the re2 matrix, Bazel's remote tests, seven
   probes, `scripts/gate/run-tests`), then the same reviewer re-checks. Notes for phase B:
   `~/.cache/blaze/work/remote/lead-notes/`.
4. Reviews owed for everything ported since the baseline (none reviewed): the evaluator's
   incremental state machine (`docs/skyframe-incremental.md`), the server (client, `blaze_server`,
   runtime wiring), `run` and the other commands, symbolic macros, Java rules, workers, the
   configuration cluster, build events, the Java syntax front end and error texts, the profiler,
   the progress bar, query outputs, test execution, repository gaps.
5. One full-suite run of Bazel's integration tests on a clean build of HEAD (load average under
   16), as the outer measure; agents do not queue their own subset runs.
6. New accepted commit: clean-checkout score including workerd, test gate with name maps,
   auditor approval, baseline move.
7. Work to continue, each with its implementer's written next steps (in the thread reports and
   docs named here):
   - Server: tests of the uncommitted-then-committed S2 remainder, the broken-pipe case in daemon
     mode, the heap arena release (a leak of about 36 MB per build remains outside the arena),
     the soak test, Bazel's server tests, then the switch of the default (needs the referee
     change the auditor specified for cold and warm sequences).
   - Incremental builds: package I2 (84 keys in six crate groups,
     `docs/skyframe-incremental.d/i2-worklist.md`), then I3.
   - Test harness: run the two newest tests, the 13 `createRuleContext` tests (needs a
     visibility-only change in `blaze_analysis`, approved), the cquery and repository_ctx
     harnesses, moving the 59 Java-rules stubs onto the mock toolchains; converter scripts in
     `~/.cache/blaze/work/analysis-harness/conv/`.
   - Java rules: Bazel's Java suites, then what they show is missing.
   - Starlark errors: error order in BUILD files, three traceback-location items, the 134 texts
     marked REVIEW in the guard list, `Error in super` prefix (3 tests).
   - Configuration: RequiredFragmentsUtil, the dependency-producer methods, project files for
     cquery and aquery.
   - Build events: the Build Event Service client (plan in the wave-3 report), the
     CompletionContext path resolver under build without the bytes.
   - Query: `--proto:rule_classes`, cquery `starlark` and `transitions`, SkyQuery.
   - Profiler: instrumentation points (`docs/profiler-sites.md`), the clock origin in
     `blaze_events`, a comparison tool.
   - Progress bar: the inactivity watchdog, Bazel's UI tests.
   - Not started: `coverage`, `dump`, `config`, `print_action`, native aspects, `genquery`,
     extra actions, include scanning, dynamic execution, Docker sandbox, action rewinding,
     vendor mode, `testing.analysis_test` (`docs/missing-features.md`).
8. Process rules added today (AGENTS.md, docs/decisions.md): at most ten agents at once on this
   machine; nothing but small notes under `/tmp` (RAM-backed); delete only what you created;
   never wrap `scripts/upstream-tests/run`; a number from a comparison tool enters README or
   this file only after the auditor's verdict on the tool.

## Waiting on the user

- Whether to delete the remote lead's old run directories (about 160 GB under
  `~/.cache/blaze/oracle-out/ur-remote` and `~/.cache/blaze/work/remote`; keep-list in the
  lead's report) and stop five remote-worker processes left running since 2026-11-18. Disk is
  at 170 GB free and needed a target cleanup about every hour under load.
- Whether the vendored starlark-rust crates' own unit tests may be run (the command was
  refused by the permission check; AGENTS.md forbids bare Cargo and a separate target
  directory).
- How identical to Bazel the console and error text must be (unruled; everything is ported to
  match).

## Outer measure

Bazel's own integration suite (`docs/upstream-tests.md`), binary of tag v0.1.0. The first
run's total (1,422 of 2,878 functions) is withdrawn: the auditor found it contaminated by
concurrent runs under the old, unlocked harness (`docs/reviews/2026-11-19-audit-3.md`). From
that run's own evidence: 1,397 functions measured as passing, 34 unmeasured, 62 of 211 targets.
A fresh run under the fixed harness (label `blaze-v0.1.0-r2`) replaces it. Largest causes of
failure, still valid as a work list: Java rules, `run`, `coverage`, `fetch`.
Coverage map (`docs/coverage/README.md`, at a79275e; counts port markers, so it is a planning
tool, not a progress measure): about 139k ± 15k lines of needed Java code not yet ported, of
406k.

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
| Remote cache and execution | 3 (at e370bf9) | STOP: P1 (SIGPIPE); fixed, evidence partial, re-review owed |
| Comparison tools written on 2026-11-19 | audit 4 | STOP: F1-F6; fixes uncommitted, re-check owed |
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
