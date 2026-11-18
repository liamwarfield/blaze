# Status

Updated 2026-11-17 (root).

## Done (committed, tests green)
- Oracle pinned (Bazel 9.3.2), corpus: brotli builds (53 actions), re2 tests 21/21 on the oracle.
- Research: 01–06 in docs/research (Theo notes, Bazel tests, recommendations, native API inventory,
  starlark-rust audit, architecture map + crate DAG + wave plan).
- Crates: blaze_util, blaze_skyframe (blocking evaluator), blaze_collect (NestedSet, 98 tests),
  blaze_events (66), blaze_vfs + blaze_cmdline (163), starlark_conformance (ScriptTest 86.8% asserts).
- blaze_options: framework + rc handling ported; OptionsParserTest still being ported.

## In flight (wave 2a/2b)
- blaze_skyframe §3.3 fixes; blaze_protos (prost + JsonFormat-compatible printer);
  blaze_shell; blaze_optdefs (fragment / command / misc transcriptions);
  blaze_bzlmod_model (Version, Selection, Discovery, registry, lockfile bytes);
  blaze_repo_fetch (repo cache, downloader, decompressors, PatchUtil);
  vendored starlark-rust + blaze_starlark; scripts/oracle recorder.

## Next (wave plan from docs/research/06 §5.4)
W1 client/runtime skeleton (version, info, clean, shutdown) → W2 `mod graph` + lockfile →
W3 packages/BUILD/.bzl loading + repo rules + `query` → W4 configs/analysis/`cquery` →
W5 actions/`aquery` → W6 brotli build → W7 re2 tests → W8 BEP/server.
