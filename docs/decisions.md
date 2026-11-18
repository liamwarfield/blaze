# Decisions (append-only)

Each entry: date, decision, why, who decided.

- 2026-11-17. Pin Bazel 9.3.2 (commit 5951e93c), Linux x86-64 only. User.
- 2026-11-17. Starlark interpreter: `starlark-rust` 0.14.2, not a port of Bazel's Java
  interpreter. User. Consequence: Starlark-level error text may differ from Bazel; structured
  outputs (query/cquery/aquery/lockfile/exec log/output hashes/test results) must still match.
- 2026-11-17. Real projects: brotli @3dce8720 (simple, `build //...`) and re2 @2da00568 (complex,
  `test //:all //python:all`; `//app/...` excluded because it fails on the oracle too). Only these
  two until the user asks for more. User (count, policy); root (choice).
- 2026-11-17. Rust 1.97.0, edition 2024, one Cargo workspace with one crate per cluster of Bazel
  Java packages. Root.
- 2026-11-17. Crate layout follows docs/research/06-bazel-architecture-map.md section 5, with the
  changes in docs/crates.md (all options in `blaze_optdefs`; domain crates implement skyframe Key;
  in-process execution until W8). Root.
- 2026-11-17. Evaluator: blocking, memoizing `blaze_skyframe` (no restarts), keeping Java's
  getValue/getValueOrThrow/valuesMissing protocol. Endorsed by the architecture map, with its
  section 3.3 fixes. Root.
- 2026-11-17. `linux-sandbox` and `process-wrapper` come from the oracle's install base until they
  are ported; `@bazel_tools` (embedded_tools) is copied from the oracle's install base into
  Blaze's install base. Root.
