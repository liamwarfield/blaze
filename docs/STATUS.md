# Status

Updated 2026-11-18 (root).

**Evidence.** Scores from clean checkouts: `corpus/scores/blaze-12ca29ee95e0.md` and
`corpus/scores/blaze-ef66f55e904e.md` (the latter with remote execution integrated): brotli and
re2, 23 of 26 surfaces MATCH, 13,850 of 13,858 compared items; open: 4 BEP items per project,
1 extra re2 stderr line. Rust tests at ef66f55 (clean): 5,487 pass, 0 fail, 2,075 ignored; with
the name map (35 renames) no baseline name is lost. Error/UI probe (35 scenarios, full
transcripts): 94 of 694 steps identical to Bazel, 225 on message text.

**Reviews** (docs/reviews/, Fable 5.2): every code area has PASS in its last round: referee/gate
(2A), skyframe (2), loading (3), actions (2), bzlmod/repos/client (4), analysis (3), execution
(4, at ef66f55). In progress: remote review (2026-11-19-remote.md), integrity audit round 2B
(incl. approval of the baseline move to ef66f55).

## Done (committed, tests green)
- Oracle pinned (Bazel 9.3.2), corpus: brotli builds (53 actions), re2 tests 21/21 on the oracle.
- Research: 01–06 in docs/research (Theo notes, Bazel tests, recommendations, native API inventory,
  starlark-rust audit, architecture map + crate DAG + wave plan).
- Crates: blaze_util, blaze_skyframe (blocking evaluator), blaze_collect (NestedSet, 98 tests),
  blaze_events (66), blaze_vfs + blaze_cmdline (163), starlark_conformance (ScriptTest 86.8% asserts).
- blaze_options: framework + rc handling ported; OptionsParserTest still being ported.

## Milestones reached against the oracle (2026-11-18)
- `version`, `info` (39 of 57 compared cases; 18 unported), `clean`, `shutdown`, rc/`--config`: match (W1).
- `mod graph --output=json`: byte-identical for brotli and re2; lockfile resolution part matches (W2).
- Every build configuration checksum (top-level, test-trimmed, exec, no-config) matches (W4 config).
- CppCompile argv for brotli dictionary.c equals the oracle's (blaze_cc_features).
- Action keys: 238/238 rebuilt oracle actions match; aquery dump replay byte-identical (W5).
- Spawn replay: 43/43 brotli, 551/551 re2 spawns match through Blaze's linux-sandbox runner (W6).
- `query`: brotli 162/162, re2 1091/1091 (label_kind + streamed_jsonproto, incl. deps()) (W3).
- Repos: every fetched repo tree identical to the oracle (brotli 67/67, re2 74/74); marker files
  and one lockfile entry per project pending Label() repo-mapping recording.
- Execution replay: brotli action graph reproduces all 96 outputs; re2 tests 21/21.
- Through the real `blaze` binary, both projects: query 162/162 + 1091/1091, cquery label_kind
  176/176 + 3295/3295, cquery jsonproto 16/16 + 37/37, aquery actions 60/60 + 1420/1420 (allow-
  listed runfiles-manifest keys only). Lockfiles byte-identical (both projects).
- Upstream integration tests: harness ready; oracle baseline 211/237 targets, 2,878 functions.

## In flight
- Remote lead (Fable; Opus workers): re2 through the real binary for remote cache/execution,
  build without the bytes, the resource_set call. Lead-reported so far (brotli, real binary):
  cache interchange with Bazel 43/43 both ways, gRPC log MATCH, 43 actions executed remotely.
- Opus fixer: adopting interpreter patches 0026-0031 in analysis/loading, cquery/aquery strategy
  validation, probe transcripts.
- Reviewers: remote; auditor round 2B.

## Next
Byte-strings migration (docs/research/07), then workerd end to end (aspects are its next
blocker), the remaining BEP items, the upstream integration-test hill, and auditor round 2B.
