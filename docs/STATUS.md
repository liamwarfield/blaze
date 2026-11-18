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

## In flight
- W3 lead: packages, rule-definition builtins, @_builtins, loading SkyFunctions, `query`.
- Repo-rules lead: repository_rule/repository_ctx, module extensions, REPOSITORY_DIRECTORY, lockfile
  moduleExtensions.
- W5 lead: blaze_actions (artifacts, actions, byte-exact action keys, runfiles, aquery dump).
- W6 lead: blaze_exec (spawn runners, linux-sandbox, exec log, action cache) via spawn replay.
- Upstream integration-test shim (Bazel's shell/py tests against any binary; oracle baseline).

## Next
Analysis lead (configured targets, toolchains/platforms, ctx API, providers, cc_internal/cc_common
wrappers over blaze_cc_features, py_builtins, `cquery`) once W3 lands → wire aquery (W5) → build
driver + execution (W6) → brotli build → re2 tests (W7) → BEP (W8).
