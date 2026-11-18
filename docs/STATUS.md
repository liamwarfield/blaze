# Status

**2026-11-18: review round 1 returned STOP on all seven areas (docs/reviews/). The fix wave is
the current priority; numbers below predate it and several are stale or overstated (see
docs/reviews/2026-11-18-integrity-audit.md, claims table).**

Updated 2026-11-17 (root).

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
- Integration lead: build/test commands end to end, real repos (no oracle seams), mod/fetch,
  lockfile writing, full oracle scoreboard (label blaze-int).
- BEP lead: blaze_bep (BuildEventStreamer, JSON transport) for the `bep` surface.

## Next
Final integration: wire `build`/`test`/`fetch`/`mod`/`query` into the `blaze` binary with real
repository fetching (no oracle seams: OracleRepos, BLAZE_W4_ORACLE_PLATFORMS), then record Blaze
with scripts/oracle on both projects and climb the scoreboard; then the upstream-tests hill; BEP (W8).
