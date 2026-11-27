# Status

Updated 2026-11-26, 14:30 (root). Running again: the pause of 14:20 was withdrawn by the user
(the threshold is 20% of the weekly quota remaining, not used; the user watches the quota from
another thread and sends the pause instruction). The six implementers were restarted in their
threads. The list below is what a resumption after any later pause needs.

## To resume

1. `scripts/cpu-cap apply` (CPU and memory limits are runtime settings), then start the keeper
   loop (`scripts/reap-idle-servers` every three minutes, `scripts/clean-target` below 95 GB).
2. The working tree holds the uncommitted work of six implementers that were cancelled mid-run
   (snapshot on the local branch `wip/pause-2026-11-26`; it does not compile: the half-written
   `crates/blaze_exec/tests/dynamic_spawn_strategy_unit_test.rs`). Resume each by a message to
   its thread ("continue where you stopped; first check your files"); thread ids are in the
   session transcript and in T3's thread list under these titles: "Incremental builds: I3",
   "Server: idle tasks, info items, default-switch preparation", "Workers: hardened sandbox,
   unit tests, then dynamic execution" (first: make the tree compile), "Java rules and config"
   (now path mapping), "Commands" (now the sweep of small families), "Remote: repo contents
   cache" (first: the silent failure with the cache on).
3. Then checkpoint 38 (tree compiles, clean-checkout build), referee guard, a full-suite run
   (unit name `root-suite-5f68157`, which the agents' briefs watch; pass
   `--slice=app-t3code-agents-jobs.slice`), and the re-review of remote round 5 (D16) plus first
   reviews of vendor mode, extra actions and metrics, the query formatters and genquery
   (`blaze_analysis::skyframe_env` has one `unsafe` block), credential helpers follow-ups.
4. Queued for free builder slots (at most six): the Build Event Service client, the
   key-tracking `ctx.var` dict (a starlark-rust patch, approved) and SkyQuery, the remaining
   profiler sites and `--starlark_cpu_profile`, include scanning, action rewinding.

## State

- **Checkpoint 35 is `5f68157`** (code commit `db8c652`); it builds from a clean checkout.
  Commits after it are documents and the audited comparison tools.
- **Accepted baseline is still `ef66f55`.** Nothing after it has been through acceptance.
- **Outer measure** (Bazel's own integration suite; the oracle passes 211 targets and 2,878
  functions; `docs/upstream-tests.md`):

  | Code | Label | Targets | Functions |
  | --- | --- | ---: | ---: |
  | tag v0.1.0 | `blaze-v0.1.0-r3` | 63 | 1,401 |
  | `e8b79c0` | `blaze-e8b79c0` | 120 | 2,220 |
  | `5f68157` | `blaze-5f68157` | 140 | 2,411 |
  | `374c631` | `blaze-374c631` | 159 | 2,619 |
  | `a673e29` | `blaze-a673e29` | 163 | 2,679 |
  | `38cd662`, server mode (opt-in) | `blaze-38cd662-daemon` | 190 | 2,803 |
  | `b216603` | `blaze-b216603` | 168 | 2,729 |
  | `b216603`, server mode (opt-in) | `blaze-b216603-daemon` | 190 | 2,774 (22 lost against `38cd662`, assigned) |

  Failures of the last run by family: `~/.cache/blaze/work/root-runs/38cd662-daemon-failures/by-family.tsv (server mode) and a673e29-failures/ (one process)`.
- **Referee guard** at checkpoint 38 (label `guard-374c631`, the commit's own binary): rows 39,
  gained 2, regressed 0.
- **Coverage map at `e8b79c0`** (`docs/coverage/README.md`): about 106k ± 15k lines of needed
  Java not ported, 13k of them out of scope. To be regenerated at the next checkpoint.

## Running (Opus implementers; no reviewer at the moment)

| Task | First items (review findings) | Then |
| --- | --- | --- |
| Query | | cquery `starlark`/`transitions`, config fragments, `genquery`, SkyQuery, aquery skyframe state |
| Incremental builds, I3 | I2 review S2-1: functions that read files outside the graph; TimestampGranularityMonitor | invalidation, diff awareness, kept graph in daemon mode, randomized incremental-against-clean test |
| Server | review round 2 leftovers (evidence on the commit's binary, loud handles) | `info` items (D15), idle tasks, default-switch preparation |
| Workers | execution round 2 (E2-9, E2-11), cgroups (C1) | hardened worker sandbox, 81 unit tests, dynamic execution |
| Remote | round 4 (Q1: certificate verifier) | repo contents cache, other commands, HTTP cache, credential helpers |
| Extra actions, metrics | reporting round 2 (R2-1) | extra actions and `print_action`, artifact metrics, status lines, Build Event Service client |
| Commands | round 2 (R1: `Analyzed 0 targets`) | vendor mode |
| Loading | round 2 leftovers (regex lazy counts, parameter-type table, load-bound names) | traceback locations |
| Path mapping | Java/config round 2 R1; coverage V3 | execution under `strip`, `testing.analysis_test`, native aspects |

## Next, in this order

1. As each implementer reports: checkpoint (tree compiles, protected paths staged out, clean
   checkout build), referee guard, full-suite run when the machine is free, then the area's
   re-review. Reviews go two areas to one Fable agent, at most three at once, medium effort.
2. Probe owed by root: `build --noexperimental_merged_skyframe_analysis_execution` on the next
   checkpoint binary (finding V1, fixed by root in `commands/build.rs`).
3. Not started: include scanning, action rewinding, Docker sandbox, `mobile-install`, Apple and
   Android rules, the remaining files of the clusters in `docs/coverage/README.md`.
4. Server as the default: after I3 passes review, the soak on the commit's binary, and the
   audited referee change for cold and warm sequences.
5. New accepted commit when every area has a PASS: clean-checkout score including workerd, test
   gate with name maps, auditor approval, baseline move. Referee batch owed with it: mask
   `--install_md5`, compare the deterministic fields of `buildMetrics`.
6. Process rules: at most three Fable agents (the usage limit was hit on 2026-11-25); builders
   as the machine allows (nine today); nothing but small notes under `/tmp`; delete only what
   you created; agents do not run `scripts/upstream-tests/run`; a number from a comparison tool
   enters README or this file only in the auditor's wording.

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
| Remote cache and execution | 5 (at e476a59) | STOP: T1-T5; verifier rewritten to ruling D16, re-review owed |
| Comparison tools | audit round 3 (2026-11-26) | PASS; committed c99a912 |
| Byte strings | 2 (at 780fceb) | PASS |
| Incremental evaluator, package I1 | 1 (at e8b79c0) | PASS |
| Incremental builds, package I2 | 1 (at 5f68157) | PASS; S2-1 must be closed inside I3 |
| Server and client | 2 (at 5f68157) | PASS; items owed before the default switch |
| Loading, macros, Starlark front end | 2 (at 5f68157) | PASS; three S2 open |
| Java rules and configuration | 2 (at 5f68157) | PASS; one S2 open |
| Commands and options | 3 (at e476a59) | PASS |
| Execution: workers, tests, templates | 3 (at e476a59) | PASS; S2 items open |
| Reporting, build events, query outputs | 3 (at e476a59) | PASS |
| Coverage | 2 (at e476a59) | PASS |
| Cgroups, hardened worker sandbox | 2 (at e476a59) | PASS |

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
