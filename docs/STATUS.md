# Status

Updated 2026-11-17 (root).

## Done (committed, tests green)
- Oracle pinned (Bazel 9.3.2), corpus: brotli builds (53 actions), re2 tests 21/21 on the oracle.
- Research: 01–06 in docs/research (Theo notes, Bazel tests, recommendations, native API inventory,
  starlark-rust audit, architecture map + crate DAG + wave plan).
- Crates: blaze_util, blaze_skyframe (blocking evaluator), blaze_collect (NestedSet, 98 tests),
  blaze_events (66), blaze_vfs + blaze_cmdline (163), starlark_conformance (ScriptTest 86.8% asserts).
- blaze_options: framework + rc handling ported; OptionsParserTest still being ported.

## In flight (2026-11-17, late)
- Referee done: scripts/oracle (13 surfaces, oracle vs oracle 100%).
- Committed since: blaze_options (405 tests), blaze_optdefs (all 1,121 options), blaze_starlark +
  vendored starlark-rust (patches 1-4, 6), blaze_protos (byte-exact), blaze_shell,
  blaze_bzlmod_model (lockfile byte-exact), blaze_repo_fetch (shared cache), skyframe fixes.
- Leads running: W1 client/runtime/`blaze` bin (version/info/clean/shutdown); W2 blaze_bzlmod
  (MODULE.bazel eval, registry, `mod graph`, lockfile); W3 packages + rule-definition builtins +
  @_builtins + loading SkyFunctions + query (repos taken from the oracle's output base for now);
  blaze_cc_features (C++ feature engine); optdefs helper cleanup.

## Next (wave plan from docs/research/06 §5.4)
Repo rules + module extensions (needs W3's attr/rule machinery and W2's seam) → W4 configs/
analysis/`cquery` → W5 actions/`aquery` → W6 brotli build → W7 re2 tests → W8 BEP/server.
Also pending: runfiles shim to run Bazel's shell/Python integration tests against `blaze`.
