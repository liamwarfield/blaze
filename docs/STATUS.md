# Status

Updated 2026-11-18 (root).

**Accepted evidence.** Score `corpus/scores/blaze-12ca29ee95e0.md` (clean checkout of 12ca29e):
brotli and re2, 23 of 26 surfaces MATCH, 13,850 of 13,858 compared items; open: 4 BEP items per
project, 1 extra re2 stderr line. Rust tests at 12ca29e (clean): 4,922 pass, 0 fail, 2,012
ignored; 31 baseline names are absent and need a reviewed name map before the baseline moves.

**Reviews** (docs/reviews/): referee/gate PASS, skyframe PASS, loading PASS, actions PASS,
bzlmod/repos/client PASS (round 4). Analysis round 2: STOP at 12ca29e (22 closed; 2 blockers:
nine cpp lists wrongly immutable, unread semantics flags). Execution round 2: STOP at 12ca29e
(all 6 round-1 blockers closed; 2 new: `--notest_keep_going` exit code, `--spawn_strategy=bogus`
message). Opus fixers are on both. Integrity audit: open outside the referee.

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
- Remote, phase 1 (2026-11-19): the `blaze` binary does `--disk_cache`, `--remote_cache=grpc[s]://`
  and `--remote_executor=grpc[s]://` for build and test with `--remote_download_outputs=all`, and
  `--credential_helper`. Against Bazel's remote worker: outputs, process summary, JSON exec log and
  gRPC log match the oracle in 13 scenarios; cache interchange with real Bazel 11/11 both ways.
  Evidence and gaps: docs/remote.md.
- Remote, phase 2 (2026-11-19): build without the bytes. `--remote_download_outputs=toplevel|
  minimal` and `--remote_download_regex` work for build and test with gRPC cache, disk cache and
  remote execution, remote tree artifacts included; the oracle matches on files present and
  absent (sha256), summaries, exec logs and gRPC logs in 14 scenarios. On a real RBE (BLAKE3,
  credential helper, TLS) brotli and re2 match the oracle and read its cache entries
  (docs/remote.md). Still loud: `--bes_backend`, BEP files with a gRPC cache, action rewinding
  for lost blobs, the lease extension, archived trees.
- Upstream integration tests: harness ready; oracle baseline 211/237 targets, 2,878 functions.

## In flight
- Fixers (Opus): analysis round 2; execution round 2 (+ no_proxy split found by the gate).
- Remote: reviews PASS (wiring and credential helper at 3bb3adf, build without the bytes at
  3a091e6). Next gaps: `--bes_backend` (BuildEventService upload), the ByteStream BEP uploader,
  recovery from a lost remote blob (action rewinding), gRPC connection pool. The gate baseline
  and the corpus score were not re-recorded for these commits.
- Gate: name map covers 30 of 31 absent names; the last is the no_proxy divergence (being fixed).

## Next
Byte-strings migration (docs/research/07), then workerd end to end (aspects are its next
blocker), the remaining BEP items, the upstream integration-test hill, and auditor round 2B.
