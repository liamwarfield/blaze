# Status

Updated 2026-11-27, 01:25 (root). Running. Numbers cite their evidence; history is in
`git log` and `docs/decisions.md`. Quota rule (user, 2026-11-26): pause when 20% of the weekly
all-model quota remains; the user watches it and sends the pause.

## To resume after a pause

1. `scripts/cpu-cap apply` (CPU and memory limits are runtime settings), then the keeper loop
   (`scripts/reap-idle-servers` every three minutes, `scripts/clean-target` below 90 GB).
2. Each implementer continues in its own thread with a message ("continue where you stopped;
   first check your files"); the threads are listed under "Running".
3. A checkpoint: `~/.cache/blaze/work/root-runs/checkpoint.sh "<subject>"` (tree compiles,
   protected paths staged out, clean-checkout check, exact binary from a worktree,
   `scripts/incremental/reuse-check`; it pushes only when the clean-checkout check passes).
   Then the reviews owed.

## State

- **Checkpoint 46 is `8ecc520`**; it builds from a clean checkout (binary
  `~/.cache/blaze/work/root-runs/blaze-8ecc520`). Checkpoint 45 (`e205a17`) does not build and
  was pushed by root's error; 46 repairs it.
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
  | `b216603`, server mode (opt-in) | `blaze-b216603-daemon` | 190 | 2,774 |
  | `8ecc520` | `blaze-8ecc520` | 169 | 2,715 (9 lost against `b216603`: one `unported!` of the Skymeld port; assigned) |
  | `8ecc520`, server mode (opt-in) | `blaze-8ecc520-daemon` | 190 | 2,779 (7 targets timed out on a hang in the server, 51 functions unmeasured; assigned) |

  Failures of the last runs by family: `~/.cache/blaze/work/root-runs/8ecc520-failures/by-family.tsv`
  (one process) and `8ecc520-daemon-failures/by-family.tsv` (server mode).
- **Referee guard** at checkpoint 46 (labels `guard-8ecc520`, `guardd-8ecc520`, the commit's own
  binary): one process 13,852 matching of 13,858, regressed 0; daemon 13,858 of 13,860,
  regressed 0.
- **Coverage map after checkpoint 39** (`docs/coverage/README.md`): about 86k ± 15k lines of
  needed Java not ported or not marked, 13k of them out of scope. To be regenerated.

## Running (Opus implementers, one thread each)

| Thread | Doing now | Then |
| --- | --- | --- |
| Incremental builds (I3) | randomized test (25 seeds, unit `i3-fuzz`); round-2 findings reported fixed | final report, then review round 3; remote sequences against the oracle; lifting the fail-safe |
| Execution, Skymeld | the server hang of `8ecc520` (stacks, cause, fix) | conflict checking without incrementality tracking (`discard_graph_edges_test`); rest of Skymeld |
| Remote | pruning its work directories; interrupt timing, connection pool, lease extension | the remote cluster of the map |
| Query | `LtoBackendActionTemplate`, query test harnesses, query shell tests | C++ rules cluster |
| Metrics, analysis | Starlark debugger (with review item Q4-1), dormant dependencies, `map_directory` | metrics-collector tests on the fixture; analysis-core cluster |
| Rewinding, configuration | review round 3 items (IS-2 test, IS-3); two test hooks; 37 RewindingTest cases | clusters `concurrency`, `vfs-and-hashing`, `base-libraries` |
| Server, packages | patch 0068 findings (P1-1, P1-2, P1-3) | cycle reporters, packages-and-loading cluster, skyframe-engine cluster |
| Sweep | `mobile-install`; the 14 missing edges of `dump --skyframe=summary` | remaining `unported!` table |

## Next, in this order

1. The hang in server mode and the Skymeld `unported!`; then a checkpoint and a suite run in
   both modes. Best measured so far: server mode at `38cd662`, 2,803 of 2,878.
2. Re-reviews owed: remote round 9 (running), rewinding round 4 (IS-2, IS-3), patches round 2
   (0068), incremental builds round 3 (when its implementer reports); first reviews of Skymeld,
   `mobile-install`, hash functions as they land.
3. The server as the default: after the incremental review passes, plus referee finding C1.
4. The next extractor change of the referee (A5-A8, C1, C2), with one renormalisation, and a
   harness batch (functions left unmeasured after an output overflow rerun one by one).
5. A new accepted commit when every area has a PASS: clean-checkout score in daemon mode
   including workerd, test gate with name maps, auditor approval, baseline move.
6. Process rules: at most three Fable agents; implementers under the CPU, memory and Cargo-slot
   limits; nothing but small notes under `/tmp`; delete only what you created; agents do not
   run `scripts/upstream-tests/run` and do not touch the git index; a number from a comparison
   tool enters README or this file only in the auditor's wording.

## Waiting on the user

- Whether to delete the remote lead's old run directories (133 GB in `oracle-out/ur-remote`; about 160 GB under
  `~/.cache/blaze/oracle-out/ur-remote` and `~/.cache/blaze/work/remote`; keep-list in the
  lead's report). (The leftover remote-worker processes ended with the reboot.) Disk was at
  100 GB free on 2026-11-27, 01:10, after a target cleanup (81 GB before).
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
| Remote cache and execution, HTTP cache, downloader, repo contents cache | 9 (at 8ecc520) | STOP: J1 (connection closed after an early answer where Java pools it; ruling: port as Java) |
| Comparison tools; referee batches 3 and 3b | 2026-11-26 | PASS |
| Byte strings | 2 (at 780fceb) | PASS |
| Incremental evaluator I1; key equality I2 | at 5f68157 | PASS |
| Incremental builds I3 (kept graph, `--watchfs`) | 2 (at 38cd662) | STOP: S1-A, S1-B, S1-C |
| Server and client; idle tasks, `info`, binary identity | at ce81d5a | PASS |
| Loading, macros, Starlark front end | 2 (at 5f68157) | PASS |
| Java rules and configuration; path mapping, analysis failures, validation aspect | at ce81d5a | PASS |
| Commands and options; vendor | at e476a59 / 374c631 | PASS |
| Execution: workers, tests, cgroups, sandboxes, dynamic execution | at a673e29 | PASS |
| Reporting, build events, metrics; extra actions | at 38cd662 | PASS |
| Build Event Service client | 2 (at dad60f0) | PASS |
| Coverage | 2 (at e476a59) | PASS |
| Query formatters, genquery, SkyQuery | 4 (at 8ecc520) | PASS |
| Action rewinding, include scanning, `testing.analysis_test` | 3 (at 8ecc520) | STOP: IS-2 (order of used header modules; one line changed by root, re-check owed) |
| Hash functions; `starlark_doc_extract`; `--proto:rule_classes`, Android and J2ObjC fragments, feature flags; auto output filter | 1 (at 8ecc520) | PASS |
| WebAssembly in repository rules | 1 (at 8ecc520) | STOP: W-1 (text-format modules accepted), W-2 (memory limits) |
| Interpreter patches 0066-0068 | 1 (at 8ecc520) | STOP: P1-1, P1-2 (patch 0068: `dict == view`, `json.encode` of a view) |
| Multi-command test fixture, ConfigCommandTest, DumpCommandTest, `dump` over a kept graph | 1 (at 8ecc520) | PASS |

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
