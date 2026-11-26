# Status

Updated 2026-11-26, 12:10 (root). Numbers cite their evidence; history is in `git log` and
`docs/decisions.md`.

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

  Failures of the last run by family: `~/.cache/blaze/work/root-runs/5f68157-failures/by-family.tsv`.
- **Referee guard** at checkpoint 33 (label `ckpt33-e593d87d59e0`, dirty tree): gained 2,
  regressed 0. Owed for checkpoint 35.
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
| Remote cache and execution | 4 (at 5f68157) | STOP: Q1 (certificate verifier); round-3 items met |
| Comparison tools | audit round 3 (2026-11-26) | PASS; committed c99a912 |
| Byte strings | 2 (at 780fceb) | PASS |
| Incremental evaluator, package I1 | 1 (at e8b79c0) | PASS |
| Incremental builds, package I2 | 1 (at 5f68157) | PASS; S2-1 must be closed inside I3 |
| Server and client | 2 (at 5f68157) | PASS; items owed before the default switch |
| Loading, macros, Starlark front end | 2 (at 5f68157) | PASS; three S2 open |
| Java rules and configuration | 2 (at 5f68157) | PASS; one S2 open |
| Commands and options | 2 (at f71e692) | STOP: R1 |
| Execution: workers, tests, templates | 2 (at f71e692) | STOP: E2-9, E2-11 |
| Reporting, build events, query outputs | 2 (at 5f68157) | STOP: R2-1 |
| Coverage | 1 (at 5f68157) | STOP: V1 (fixed by root, probe owed) |
| Cgroups | 1 (at 5f68157) | STOP: C1 |

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
