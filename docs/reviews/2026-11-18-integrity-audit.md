# Integrity audit: referee, hardcoding, tests, claims, process

Date 2026-11-18. Role: independent regression auditor (read-only, Fable 5.2).
Verdict: **STOP** (blocking findings S1-1 to S1-4 and the WRONG rows of the claims table).

## Commit read

- HEAD `e4379753f8b645eee3b9f49e54c5051c57414634` on `main`, **plus a dirty working tree** (85
  paths: 70 modified, 15 untracked; the integration lead is working in it). Evidence under
  `~/.cache/blaze` was read between 09:00 and 09:24 MST; label `blaze-int` was being re-recorded
  during the audit (`scripts/oracle/record --binary ~/.cache/blaze/int-bin/blaze-b3 --label
  blaze-int --project re2` was running), so numbers quoted for it are snapshots.
- No Cargo was run. Statements about compilation and panics are by inspection and are marked so.

## Method

1. Read AGENTS.md, PORTING.md, docs/research/01 and 03, STATUS, decisions, README, scripts/oracle
   in full (record, compare, scoreboard, normalise.py, surfaces.py, oracle_compare.py, allow.json).
2. Re-derived every comparison from the `items/` files in memory with the referee's own
   `oracle_compare` (no files written), instead of trusting the stored report JSONs; checked the
   reference recording against the sha256 list committed in `corpus/oracle/*.json`; byte-compared
   `raw/` and `norm/` files across the three oracle recordings and the Blaze recordings.
3. `git log`/`git show` on the referee and the per-wave scripts; `strings`-level scan of the
   recorded Blaze binaries for seam names; grep of all Rust for project names, digests, cache
   paths, env switches, embedded files.
4. Tests: 74 ported tests in 13 crates read side by side with the Java/C++ originals (delegated
   reader, spot-checked by me), plus 68 tests in 16 further crates screened mechanically
   (assert counts, literals present in Rust but not in the Java test) with about ten read by hand.
5. `python3 -I -B -m unittest discover -s scripts/oracle`: 38 tests, OK.

Limits: two further delegated readers could not be started (subagent limit), so the per-wave
scripts were read by me at reduced depth: `w1/compare-info`, `w3/query-compare`,
`w5r/compare-repos`, `w6/replay-spawns`, `w6b/replay-build`, `bep-compare`,
`upstream-tests/compare`, `gate/compare-tests`, `workerd/*` in the relevant parts; `w2/*`,
`w4/oracle-probe`, `w7/replay-tests`, `upstream-tests/run|summarize`, `bytes/run` only through
their usage text and their logs on disk.

## Where things are sound

- **The reference is intact.** All 28 `norm/` and `items/` files per project of
  `oracle-9.3.2/{brotli,re2}` match the sha256 values committed in `corpus/oracle/brotli.json`
  and `re2.json`; every `raw/` file matches its manifest; oracle binary sha256 equals
  `UPSTREAM.json` (`2e14c55c…`).
- **The referee was not changed after its baseline.** `scripts/oracle` has exactly one commit
  (`99e3604`, 2026-11-17 22:55), written before the first Blaze binary existed (`42f4ec1`, 00:28
  next day); no uncommitted changes. Nothing in it is keyed to Blaze, a project or a target.
- **Oracle against itself is 100% MATCH**, recomputed: all 13 surfaces of both projects for
  `oracle-9.3.2` vs `-rerun` vs `-check`. The allow list has two entries, both restricted to
  named JSON paths; they excuse 2 (brotli) and 54 (re2) runfiles-manifest action keys and 1
  buildMetrics event per project, in oracle-vs-oracle and in Blaze comparisons alike.
- **FAILED handling is conservative.** A surface that FAILED on either side is FAILED even with
  0 vs 0 items; a failed command leaves MISSING items; partial recordings always show
  `exit_codes` DIFF (missing commands), so they cannot print `100% MATCH`.
- **No hardcoded answers in the Rust.** No project or target name affects behaviour (16 hits, all
  comments or inline tests); no hex digest in production code (4 hits, all inside `#[cfg(test)]`);
  no `include_str!` of recorded outputs in production code; the 28 bundled `builtins_bzl` files
  are byte-identical to the pinned source; checksum literals in
  `blaze_build_config/tests/oracle_configurations_test.rs` are found in the oracle's data.
- **The per-wave scripts I read compare with oracle data**, not with Blaze's output, and exit
  non-zero on differences. None was loosened after its first commit (one commit each, except
  `w5r/compare-repos`, whose second commit only changed which oracle output base is read).
- **The seam-free integration binary is real progress.** `blaze-b1`/`blaze-b3` contain no
  `BLAZE_ORACLE_REPOS`, `repos.json` or `.cache/blaze` strings and were recorded with
  `HOME`+`PATH` only. Against the oracle, recomputed: both projects MATCH on mod_graph, lockfile
  (raw bytes identical), query_label_kind/jsonproto, cquery_label_kind/jsonproto, aquery_actions;
  brotli also MATCH on exit_codes, build_stderr, execlog 44/44, outputs 96/96 (b3: bep 37/42).
  re2 `test` with b1: exit 101, outputs 1022/5043, execlog/bep/tests FAILED, reported as such.
- README is honest that execution is validated by replay and that some inputs come from the
  oracle's install base. STATUS and README do not claim `build`/`test` end to end.

## Findings

### S1 (wrong output or behaviour on valid input)

**S1-1. `cquery` and `aquery` print results in a different order from Bazel; the referee cannot
see it.** The oracle's order is deterministic: `raw/cquery_deps_label_kind.stdout` and the action,
target and artifact order of `aquery_deps_jsonproto` are identical in all three oracle
recordings. Blaze's differ in every recording (blaze-w4, blaze-int):

```
$ diff oracle-9.3.2/brotli/norm/cquery_deps_label_kind.txt blaze-int/brotli/norm/... | grep -c '^[<>]'
276            (of 176 lines; re2: 6150 of 3295; sorted files are identical)
oracle: filegroup rule //:common_headers (a7a71fd) / filegroup rule //:enc_headers ...
blaze : cc_binary rule //:brotli (a7a71fd) / cc_library rule //:brotli_inc ...
aquery first action: oracle _objs/brotlidec/bit_reader.pic.o, blaze _objs/brotli/brotli.pic.o
```

`surfaces.s_cquery_label_kind` and `aquery_summary` build dicts with no `__order__` item (query
has one), so the scoreboard says 176/176, 3295/3295, 60/60, 1420/1420. The port knows:
`crates/blaze_cquery/src/post_analysis_query_environment.rs:16-18` "PORT: java-hash-order …
here they iterate in insertion order. cquery/aquery outputs are compared as sets." That is a
deviation chosen because the judge does not look, against PORTING.md "Ordering of anything that
reaches output … must match Java" and its `java-hash-order` rule ("ask root").

**S1-2. Starlark flags are dropped without an error.**
`crates/blaze_runtime/src/dispatcher.rs:501-504` skips `BlazeOptionHandler.parseStarlarkOptions`
with a comment, not `unported!`. docs/workerd.md item 13 records the effect: 21 user-defined
options missing, target configuration checksum `6d8e5e8` vs the oracle's `13e714b`, "no error:
wrong configuration". Violates "Never guess a value … fails loudly".

**S1-3. HEAD does not build (by inspection).** `crates/blaze_sky_top/src/lib.rs:15` at HEAD has
`pub mod external_repos;` and `skyframe_executor.rs:61` imports it, but
`crates/blaze_sky_top/src/external_repos.rs` was deleted by commit `0388d76` ("W8: blaze_bep",
`git show 0388d76 --diff-filter=D --name-only`), which did not touch `lib.rs`. So `0388d76`,
`7a4e95d` and `e437975` cannot compile `blaze_sky_top` or `blaze`; README's
`scripts/cargo-capped build -p blaze` fails at the commit that carries the claims. The working
tree compiles only through uncommitted edits.

**S1-4. Plain `blaze info` panics; README lists `info` as "match" and uses it as the demo.**
Only evidence on disk (`~/.cache/blaze/work/w1/results/{brotli,re2}`, 2026-11-18 00:17): of 57
W1 cases, 39 are equal to the oracle and 18 are Blaze `unported!` panics with exit 101, among
them `info` without keys, `info bazel-bin|bazel-genfiles|bazel-testlogs`, `info build-language`,
`info starlark-semantics`, `info --show_make_env`, both `canonicalize-flags` cases and nine JVM
items. In the working tree the JVM items still call `unported!`
(`crates/blaze_runtime/src/commands/info/items.rs:562`), and `scripts/w4/compare-config` says "a
bare `info --show_make_env` also prints the JVM items, unported by decision", so the keyless
listing still panics (by inspection).

### S2 (masked deviations, weakened checks, misleading evidence)

**S2-1. "info (39/39 comparable checks)" hides the denominator.** STATUS line 14 and the W1 commit
message ("39 MATCH, 0 MISMATCH per project") count only cases that did not panic: 39 of 57. The
18 BLAZE-UNPORTED cases are neither listed nor counted. This is the "publish denominators and
exclusions" failure of research note 01.

**S2-2. Evidence is not bound to a commit, and is overwritten.** Every Blaze recording was made
from an uncommitted tree: `blaze-w4` (the source of the README table) used
`target/debug/blaze` sha256 `979ea1a3…`, which no longer exists (now `8d99864a…`);
`blaze-int` used private copies `~/.cache/blaze/int-bin/blaze-b1` (`85d3aa15…`), then `b3`. The
same label is re-recorded in place and `compare/oracle-9.3.2__vs__blaze-int.json` is overwritten
(it currently holds re2 only). No Blaze summary is committed (only `corpus/oracle/*.json` for the
oracle). No number in README or STATUS names a commit or binary hash, against AGENTS.md "Name the
commit or tree your numbers came from".

**S2-3. Gaps in the referee** (none exploited in a published claim, all cheap to close):
- `oracle_compare.is_clean` treats `NOT_COMPARED` as clean and `compare_labels` reports a project
  missing from the candidate as `ABSENT` → `NOT_COMPARED`. A label that recorded only brotli,
  fully matching, gets `result: 100% MATCH` with re2 never run. Missing evidence must fail.
- No order item for `cquery_label_kind`, `cquery_jsonproto`, `aquery_actions` (S1-1).
- Allow-list entries have no `max_items`; `shutdown`'s exit code is discarded
  (`record:334`); `_STDERR_KEEP` drops `Target … up-to-date` listings; `Context.paths_only` replaces
  path prefixes without a boundary check (`…/blaze/oracle` also matches `…/blaze/oracle-out`;
  makes the referee stricter, not laxer).
- README counts allow-listed items as matches ("60/60", "1420/1420" for a row that names
  "action keys"): 58+2 and 1366+54. The 56 runfiles-manifest action keys are not checked at all
  for Blaze. STATUS says so; README does not.

**S2-4. The README table header says "Through the `blaze` binary", but** at the time it was
written the query/cquery/aquery rows ran with the `OracleRepos` seam, selected by the workspace
directory's basename (`~/.cache/blaze/w3/<basename>.repos.json`, commit `2a745f1`
`blaze_sky_top/src/external_repos.rs`; the recorder passes only HOME and PATH, so the default
path was used), the "fetched external repositories" row comes from `blaze-fetch-dev`, and the
mod/lockfile rows from `blaze-mod-dev` (HEAD has no `mod` command). Known limits discloses the
repos seam. The later seam-free recording supports all rows except the repo-tree comparison.

**S2-5. Silent no-ops where the contract demands `unported!`.** About 199 `// PORT: … not
ported / not posted / ignored` comment sites in non-test code have no `unported!`/panic within
the next lines (60 in blaze_runtime, 18 blaze_repo_rules, 15 blaze_sky_top, 14 blaze_sky_loading,
13 blaze_sky_analysis). Most are event-bus or profiling skips; these change behaviour on valid
input: `--target_pattern_file` ignored (`blaze_runtime/src/commands/build.rs:161`),
`--subcommands` ignored (`blaze_sky_exec/src/skyframe_action_executor.rs:2131`), PROJECT.scl not
evaluated "no project files in the corpus" (`blaze_sky_top/src/build_tool.rs:295`),
`rctx.attr` prints an invented string "nothing in the corpus prints rctx.attr"
(`blaze_repo_rules/src/repository/repo_definition.rs:97-101`), SIGINT handling absent.
The corpus-scoped gaps in blaze_rules/blaze_sky_analysis ("not reached by brotli or re2",
aspects) do fail loudly; those are S3.

**S2-6. A recorded baseline under the reference label was spliced by hand.**
`oracle-records/oracle-9.3.2/workerd/manifest.json` is a copy of `oracle-9.3.2-workerd` with
`aquery_deps_jsonproto` exit code changed 8 → 0 and the output replaced by one from a different
output base (`argv_note` in the manifest and docs/workerd.md say so; paths in it are not
normalised). Documented, and workerd is not in any claim, but a Blaze that exits 0 would match
where the oracle under the referee's protocol exits 8.

**S2-7. Most of the JUnit hill above the base layers is switched off, with stale reasons.**
1,701 of 5,529 `#[test]` are `#[ignore]` (31%): blaze_query 181/190, blaze_sky_analysis 101/106,
blaze_packages 554/928, blaze_repo_rules 133/162, blaze_options 145/519. Loading, analysis and
repository rules are therefore judged almost only by two projects, which is the overfitting
condition. Reasons that no longer hold: blaze_query "needs package loading and target pattern
evaluation from blaze_sky_loading" (181; both exist since `e46be0e`), blaze_packages "needs BUILD
loading …" (about 520), blaze_bzlmod "needs PatchUtil.applyToSingleFile" (3; ported per
docs/workerd.md), blaze_actions "needs action execution … wave W6" (11), blaze_buildtool "needs
the BEP port" (1), blaze_analysis "getPlatformProto is ported in blaze_exec" (6). What is missing
is a test harness (scratch workspace / BuildViewTestCase), not the feature.

**S2-8. Ported tests weakened in blaze_starlark and a few other places** (sample of 74 read
fully: 48 faithful, 12 weakened, 14 Blaze-only, 0 name-only, 0 expectation taken from Blaze):
- `crates/blaze_starlark/tests/depset_test.rs:318-327`: Java asserts `depset([(x,)])` is accepted
  (`DepsetTest.java:326-328`); the Rust drops the assertion with "(known difference)". A known
  behaviour difference is hidden instead of failing or being `#[ignore]`d.
- `depset_test.rs:250-262`: seven Java message checks became bare `is_err()`, no comment.
- `traceback_test.rs:90-97`: input and expected column changed, 2 of 3 cases dropped, no comment.
- `blaze_client/tests/logging_test.rs:254,298`: death tests do not check the exit code.
- `blaze_skyframe/src/tests.rs:854,896`: scenarios reduced (20 deps for 100; race replaced by loop).
- Java tests with no Rust counterpart and no `#[ignore]` stub: `ParallelEvaluatorTest` 68 of 91,
  `GroupedDepsTest` 17 of 17, `MemoizingEvaluatorTest` 153 of 154, `FileSystemTest` (168),
  `FingerprintTest` (12), `MethodLibraryTest` 53 of 58, blaze_config (0 tests).
- My 68-test screen of 16 further crates found no expectation that differs from the Java test;
  adaptations seen were disclosed (`blaze_rules/src/cpp/cpp_compile_action.rs:2716-2745`
  narrows `testClangClParameters` to option parsing, with a PORT comment).

**S2-9. Oracle-driven tests pass when their evidence is missing.** `oracle_action_keys.rs:546`,
`oracle_aquery_dump.rs`, `oracle_platform_probe.rs`, `blaze_bep/tests/oracle_bep_test.rs:180,338`,
`blaze_repo_fetch/tests/oracle_extraction_test.rs`, `blaze_client/tests/blaze_test.rs:218` print
"skipped" and return. `oracle_action_keys` also reports "238/238" out of 1,480 recorded actions
(mnemonics it cannot rebuild are "not attempted"); `oracle_bep_test::replay_*` feeds the oracle's
own events through the streamer (it tests serialisation and the progress chain, not event
production; its header says so).

**S2-10. Stale or inconsistent status numbers.** STATUS says repos "re2 74/74 … pending", the
report on disk and README say 81/81; STATUS "ScriptTest 86.8% asserts" is the harness's
non-Java "resilient" mode (2,356/2,714 executed assertions; strict result 28/95 chunks), is
superseded by 89.0% in docs/research/05 line 509-510, and no result file exists; the ratchet in
`starlark_conformance/tests/script_test.rs:22-29` still asserts the old lower bounds.

### S3 (missing port, honestly marked)

- 754 `unported!`, 93 `unimplemented!`, 0 `todo!` call sites (table below). Reachable with default
  flags on valid workspaces (by inspection): any `genrule` or other native rule without a factory
  (`blaze_analysis/src/rule/configured_target_factory.rs:514`), any analysis error
  (`SkyframeErrorProcessor.processAnalysisErrors`, `blaze_sky_top/src/skyframe_build_view.rs:147`),
  any attribute aspect or `--aspects` (`dependency_map_producer.rs:192`, `build_view.rs:204`), any
  Starlark flag in a configuration (`blaze_build_config/src/skyframe.rs:319`), tree artifacts
  and filesets in execution (`skyframe_action_executor.rs:839-861,1492`), exclusive tests
  (`commands/build.rs:598`), `--test_size_filters`/`--test_timeout_filters`/`--test_lang_filters`,
  `help <topic>`, `canonicalize-flags`, `mod dump_repo_mapping`, `cquery --output=textproto`
  and the other cquery/aquery formats, `info` JVM items, a second command while one runs.
- No run of the upstream integration tests against Blaze exists (only five oracle result sets).
- BEP through the binary: brotli 37/42 events at the time of reading (in flight).

### S4

- `Context.text` applies the host-name rule to every string; the host is `fedora`, a word that can
  occur in real output. Equal on both sides, so harmless today.
- Shared scratch directory: another agent overwrote a helper script of this audit mid-run; tools
  that produce evidence should not live in a shared scratchpad.

## Claims table

V = VERIFIED (evidence on disk, recomputed), U = UNVERIFIED (no evidence on disk), W = WRONG.

| # | Claim (source) | Status | Evidence / note |
|---|---|---|---|
| 1 | query 162/162, 1091/1091 (README, STATUS) | V | blaze-w4 (OracleRepos seam) and blaze-int (no seam); raw label_kind stdout byte-identical |
| 2 | cquery label_kind 176/176, 3295/3295 | V as sets | output order differs (S1-1) |
| 3 | cquery jsonproto 16/16, 37/37 | V | normalised file byte-identical |
| 4 | aquery 60/60, 1420/1420 "action keys" | V with caveat | 58+2 and 1366+54 allow-listed keys; order differs |
| 5 | `mod graph --output=json` byte-identical | V | raw stdout identical, blaze-int both projects (uncommitted binary) |
| 6 | MODULE.bazel.lock byte-identical | V | raw identical, both projects |
| 7 | fetched repositories 67/67, 81/81 (README) | V | `work/repos/*-report.json`; **dev binary** `blaze-fetch-dev`, not `blaze` |
| 8 | configuration checksums all match | V | cquery keys and `configuration <checksum>` items through the binary |
| 9 | `version`, `info`, `clean`, `shutdown`, rc: match (README) | **W** for `info` | 39 of 57 W1 cases; keyless `info` exits 101 (S1-4) |
| 10 | spawn replay 43/43, 551/551 | V | `work/w6/*/execlog.json` 43 and 551 entries, `re2-full.log`; dev binary, oracle inputs and oracle `linux-sandbox` |
| 11 | brotli action graph reproduces 96 outputs | V | `work/w6b/brotli/rd` 96/96 equal; dev binary; also 96/96 end to end in blaze-int |
| 12 | re2 tests 21/21 (replay) | V | `work/w7/full-run.log`; dev binary replaying oracle TestRunner actions |
| 13 | README build instruction | **W** at HEAD | S1-3 (by inspection) |
| 14 | README demo `blaze info` | **W** | S1-4 |
| 15 | brotli 53 actions, re2 21/21 on the oracle (STATUS) | V | oracle recording |
| 16 | crate test counts 98 / 66 / 163 / 405 (STATUS) | U | no test log on disk; static `#[test]` counts 77 / 61 / 175 / 519 (145 ignored) |
| 17 | ScriptTest 86.8% asserts | U | no result file; non-Java mode; stale (S2-10) |
| 18 | info 39/39 comparable checks | **W** (misleading) | 39 of 57 (S2-1) |
| 19 | mod graph byte-identical; lockfile resolution part matches (W2) | V | as 5, 6 |
| 20 | every configuration checksum (W4) | V | as 8 |
| 21 | CppCompile argv for dictionary.c equals the oracle's | V | all 60 brotli aquery argv match |
| 22 | action keys 238/238; aquery dump replay byte-identical (W5) | U | no stored output; 238 of 1,480; superseded by 1,424 keys through the binary |
| 23 | spawn replay (STATUS) | V | as 10 |
| 24 | query (STATUS) | V | as 1 |
| 25 | repos 67/67, **74/74**, markers pending (STATUS) | **W** (stale) | disk and README: 81/81 |
| 26 | execution replay 96 outputs, 21/21 (STATUS) | V | as 11, 12 |
| 27 | real binary: query, cquery, aquery, lockfiles (STATUS) | V | as 1-6, same caveats |
| 28 | upstream tests: oracle baseline 211/237 targets, 2,878 functions | V | `corpus/upstream-tests/oracle-9.3.2.json`: 211 and 2,878; 237 targets in each of 4 runs; functions passing in all 4 recounted: 2,878 |
| 29 | "Done (committed, tests green)" (STATUS) | **W** at HEAD | S1-3; no test log on disk |

Totals: 29 claims; VERIFIED 20, UNVERIFIED 3, WRONG 6. Claims that rest on dev binaries or
seams, not on `blaze`: 7, 10, 11, 12, 22, and (at the time README was written) 1-6.

## Hardcoding table

| Item | Where | Class |
|---|---|---|
| `OracleRepos` via `$BLAZE_ORACLE_REPOS` or `~/.cache/blaze/w3/<workspace basename>.repos.json` | `blaze_sky_top/src/external_repos.rs` (commits `2a745f1`..; deleted in `0388d76`, still declared at HEAD) | documented temporary seam (decisions 2026-11-18), was reachable from `blaze`; removed in the working tree; file holds repo directories and mappings only, no target answers |
| `BLAZE_W4_ORACLE_PLATFORMS` | `blaze_runtime/src/commands/info.rs:81` at HEAD; string still in `blaze-b1`; gone from the working tree | documented temporary seam |
| `OracleRepos`, `OraclePlatforms` types | `blaze_sky_loading/src/repos.rs`, `blaze_build_config/src/platform.rs`, used by `blaze-query-dev`, `blaze-config-dev` | dev fixture (dead code in `blaze`) |
| oracle install base path `…/oracle-out/ur/install/58cf1ec…` | `blaze-query-dev`, `blaze-mod-dev`, `blaze-fetch-dev` | dev fixture |
| oracle recording reader | `blaze_buildtool/src/bin/replay/recording.rs` | dev fixture (replay binaries) |
| embedding the oracle's install base (`BLAZE_ORACLE_INSTALL_BASE`) | `crates/blaze/build.rs` | documented decision (linux-sandbox, process-wrapper, `@bazel_tools`) |
| version impersonation: Bazel 9.3.2 build data verbatim; "Starting local Bazel server (9.3.2)" | `blaze_runtime/src/version_info.rs`, client | documented decision; means `blaze version` cannot be told from Bazel |
| project names in production code | 16 hits: comments, inline tests | clean |
| hex digests in production code | none outside `#[cfg(test)]` | clean |
| bundled `@_builtins` | `blaze_sky_loading/builtins_bzl` (28 files) | clean, identical to the pin |
| invented value for `rctx.attr` repr; PROJECT.scl skipped; `--target_pattern_file`, `--subcommands`, Starlark flags silently ignored | see S1-2, S2-5 | **illegitimate shortcut** (guessed value or silent skip justified by the corpus) |
| `cquery`/`aquery` result order "compared as sets" | `blaze_cquery/src/post_analysis_query_environment.rs:16-18` | **illegitimate shortcut** (S1-1) |
| oracle recordings read by tests, silent skip when absent | S2-9 | legitimate fixture, weak (vacuous pass) |
| workerd reference with replaced aquery and exit code | S2-6 | documented baseline edit |
| workerd `--ascii-readme` input fix-up | `scripts/workerd/compare-repos` | documented |

## Ignore and unported counts (working tree)

`#[ignore]` / `unported!` / `unimplemented!` / `#[test]` per crate:

```
blaze_actions 55/24/0/248        blaze_analysis 40/54/0/142      blaze_bep 27/9/1/122
blaze_build_config 71/22/0/135   blaze_buildtool 44/8/0/121      blaze_bzlmod 40/15/0/273
blaze_bzlmod_model 10/8/0/115    blaze_cc_features 1/5/0/121     blaze_client 10/8/0/133
blaze_cmdline 4/0/0/77           blaze_collect 15/0/0/77         blaze_config 0/0/0/0
blaze_cquery 10/20/0/16          blaze_events 10/6/0/61          blaze_exec 72/46/5/448
blaze_optdefs 27/11/0/447        blaze_options 145/135/0/519     blaze_packages 554/40/67/928
blaze_protos 0/0/2/26            blaze_query 181/198/0/190       blaze_query_engine 0/3/18/48
blaze_repo_fetch 13/0/0/273      blaze_repo_rules 133/16/0/162   blaze_rules 77/16/0/112
blaze_runtime 27/38/0/89         blaze_shell 10/1/0/100          blaze_sky_analysis 101/25/0/106
blaze_sky_exec 8/17/0/17         blaze_skyframe 0/1/0/60         blaze_sky_loading 13/10/0/123
blaze_sky_top 1/18/0/4           blaze_starlark 0/0/0/99         blaze_util 0/0/0/5
blaze_vfs 8/0/0/98               starlark_conformance 0/0/0/34
total 1701 / 754 / 93 / 5529     todo!: 0
```

Ignore reasons: needs unported infrastructure or a harness 1,662; Java-specific or concurrency 9;
other-OS 4; opt-in (oracle data, slow) 3; starlark-rust difference 2; no reason 2 (+3 macro
forms); other 16. Stale reasons: S2-7.

## Process: what is missing (AGENTS.md, research 03)

- Protected paths are a sentence in AGENTS.md; nothing enforces them (no hook, no candidate
  check; `.git/hooks` has only samples).
- No accepted per-name baseline is committed: `docs/state/` is empty, `docs/protected/` does not
  exist; `scripts/gate/{run-tests,compare-tests}` are referenced nowhere and have never produced a
  stored result.
- No Blaze scoreboard is committed; recordings live outside the repo under reusable labels.
- No upstream-tests run against Blaze.
- Root commits a shared dirty tree while agents edit it (S1-3 is the result); no "build at the
  commit" check before committing.
- Reviewers and auditors did not exist until today; 41 commits landed without one.

## Recommendations (minimum to adopt now)

1. **Make every accepted number reproducible from a commit.** Before a number enters README or
   STATUS: commit, build `blaze` from a clean checkout of that commit, record under a label that
   contains the commit hash (never reused), and commit `corpus/blaze/<project>.json` (binary
   sha256, per-surface counts, item-file hashes) plus the scoreboard text. Add a pre-commit check
   that the workspace builds at the commit (catches S1-3).
2. **Close the referee's gaps under a named batch with an independent review:** `__order__` items
   for `cquery_label_kind`, `cquery_jsonproto` and `aquery_actions` (oracle-vs-oracle stays 100%
   with them: verified here on three recordings); `NOT_COMPARED`/`ABSENT` fail `compare` unless
   `--project`/`--surface` restricts the run explicitly; `max_items` on both allow entries (2/54
   and 1 per project); record `shutdown`'s exit code. Then fix S1-1 in `blaze_cquery` by porting
   Java's hash order.
3. **Per-name ratchet for Rust tests and oracle items.** Run `scripts/gate/run-tests` on the
   accepted commit, commit the result with its sha256 as the baseline, and require
   `compare-tests` (lost, absent, unrun all fail) for every later acceptance; the same rule for
   scoreboard items. Enforce protected paths (scripts/oracle, scripts/gate, corpus/, baselines,
   allow.json, UPSTREAM.json) with a hook that rejects a commit touching them unless the commit
   message names the path.
4. **Publish denominators and exclusions with every score.** Rewrite the W1 line as "39 of 57
   cases; 18 unported: …"; README counts allow-listed items separately; every claim names its
   binary (`blaze` or which dev binary), its seams, and its commit. Fix rows 9, 13, 14, 18, 25,
   29 of the claims table.
5. **Stop silent skips and widen the hill.** Replace behaviour-changing `// PORT: not ported`
   comments with `unported!` (start with S1-2 and the S2-5 list); make oracle-driven tests fail,
   not skip, when their recording is absent unless an explicit opt-out is set; build the scratch
   workspace / BuildViewTestCase harness so the roughly 1,000 ignored loading, query, analysis and
   repository tests run, and restore the dropped assertions in blaze_starlark as failing or
   ignored tests; run upstream-tests against Blaze once and commit the per-name result.

## Verdict

**STOP**, bound to HEAD `e437975` plus the working tree as read on 2026-11-18 09:00-09:24 MST.
Blocking: S1-1 (cquery/aquery order, masked by the referee), S1-2 (Starlark flags dropped
silently), S1-3 (HEAD does not build), S1-4 with claims rows 9, 13, 14, 18, 25, 29 (published
claims that the evidence contradicts). The referee itself, its reference recordings and the
absence of hardcoded answers are sound; the item-level results for query, cquery, aquery, mod
graph and the lockfile are real.
