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
