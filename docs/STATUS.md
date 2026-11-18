# Status

Updated 2026-11-17 (root).

## Done (committed, tests green)
- Oracle pinned (Bazel 9.3.2), corpus: brotli builds (53 actions), re2 tests 21/21 on the oracle.
- Research: 01–06 in docs/research (Theo notes, Bazel tests, recommendations, native API inventory,
  starlark-rust audit, architecture map + crate DAG + wave plan).
- Crates: blaze_util, blaze_skyframe (blocking evaluator), blaze_collect (NestedSet, 98 tests),
  blaze_events (66), blaze_vfs + blaze_cmdline (163), starlark_conformance (ScriptTest 86.8% asserts).
- blaze_options: framework + rc handling ported; OptionsParserTest still being ported.

## Milestones reached against the oracle (2026-11-18)
- `version`, `info` (39/39 comparable checks), `clean`, `shutdown`, rc/`--config`: match (W1).
- `mod graph --output=json`: byte-identical for brotli and re2; lockfile resolution part matches (W2).
- Every build configuration checksum (top-level, test-trimmed, exec, no-config) matches (W4 config).
- CppCompile argv for brotli dictionary.c equals the oracle's (blaze_cc_features).
- Action keys: 238/238 rebuilt oracle actions match; aquery dump replay byte-identical (W5).
- Spawn replay: 43/43 brotli, 551/551 re2 spawns match through Blaze's linux-sandbox runner (W6).
- `query` (W3, interim): brotli 162/162; re2 main targets match; re2 deps() in progress.

## In flight
- W3 lead: finishing re2 `deps()` query.
- Repo-rules lead: repository_rule/repository_ctx, module extensions, real fetching.
- Analysis lead: platforms/toolchains, configured targets, ctx API, cc/py natives, cquery/aquery.
- Build-driver/test lead: ACTION_EXECUTION, BuildDriver, symlink forest, summaries, test strategy.
- Upstream integration-test shim.

## Next
Analysis lead (configured targets, toolchains/platforms, ctx API, providers, cc_internal/cc_common
wrappers over blaze_cc_features, py_builtins, `cquery`) once W3 lands → wire aquery (W5) → build
driver + execution (W6) → brotli build → re2 tests (W7) → BEP (W8).
