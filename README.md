<img src="assets/icon.svg" width="64" height="64" alt="">

# Blaze

A Rust port of [Bazel](https://github.com/bazelbuild/bazel) 9.3.2 for Linux x86-64. Starlark runs
on a vendored, patched [starlark-rust](https://github.com/facebook/starlark-rust); everything else
ports Bazel's Java. The real Bazel 9.3.2 binary is the referee: Blaze's outputs are compared with
it byte for byte or item by item.

## Status

Two real projects are the targets: [brotli](https://github.com/google/brotli) (simple C,
`build //...`) and [re2](https://github.com/google/re2) (C++ with abseil, a pybind11 Python
extension via rules_python, googletest; `test //:all //python:all`).

**Score** at commit `12ca29e`, from a binary built on a clean checkout, with Blaze's own
repository fetching and no oracle shortcuts (label `blaze-12ca29ee95e0`; full table in
`corpus/scores/blaze-12ca29ee95e0.md`):

| Surface | brotli | re2 |
| --- | --- | --- |
| exit codes | 10/10 | 10/10 |
| `mod graph`, `MODULE.bazel.lock` | 27/27, 193/193 | 29/29, 191/191 |
| `query` (label_kind, streamed_jsonproto, incl. `deps()`) | 162/162, 162/162 | 1091/1091, 1091/1091 |
| `cquery` (label_kind, jsonproto), including output order | 177/177, 18/18 | 3296/3296, 39/39 |
| `aquery` actions, including order and ids | 66/66 (+2 allowed) | 1374/1374 (+54 allowed) |
| build stderr summary lines | 5/5 | 7/7, plus 1 extra line |
| execution log | 44/44 | 552/552 |
| output files (sha256) | 96/96 | 5043/5043 |
| test results | none | 21/21 |
| build event stream | 37/41 | 109/113 |

23 of 26 surfaces match; 13,850 of 13,858 compared items. "Allowed" items are runfiles-manifest
action keys and volatile build-metric counters, which also differ between two runs of real Bazel.
A third project, workerd, is recorded on the oracle only and counts as missing in the score file.

**Reviews.** Independent reviewers (read-only, on a different footing from the implementers) read
the code against Bazel's Java; reports and verdicts are in `docs/reviews/`. Round 1 returned STOP
on all seven areas. After fixes: evaluator, loading, actions, Bzlmod/repositories/client and the
referee scripts have PASS; analysis and execution are fixed and awaiting re-review; the integrity
audit is open outside the referee.

Known limits: Linux x86-64 only, and scores are only comparable on a 16-CPU machine (Bazel's
`cquery` order depends on the CPU count); one command per process (no persistent server);
`linux-sandbox`, `process-wrapper` and `@bazel_tools` come from Bazel 9.3.2's install base;
aspects, symbolic macros, `fetch`/`sync` and coverage are not ported and fail loudly; remote
caching and execution work for `build`/`test`, including build without the bytes (the Build
Event Service upload, action rewinding and BEP files with a gRPC cache fail loudly;
`docs/remote.md`); non-ASCII strings do not yet follow
Bazel's byte semantics (UTF-8 text in action keys does). `info` matches for 42 of 57 compared
cases; the other 15 (among them `info` with no key and JVM-only keys) fail loudly.

## Build

Needs Rust 1.97 (pinned in `rust-toolchain.toml`) and, at build time, Bazel 9.3.2's install base
to embed (see `crates/blaze/build.rs`; set `BLAZE_ORACLE_INSTALL_BASE` or run Bazel 9.3.2 once so
it exists under `~/.cache/blaze/oracle-out/ur/install/`).

```sh
scripts/cargo-capped build -p blaze      # -> target/debug/blaze
```

Note: commits `0388d76` through `078b27a` do not build (a partial commit). `0e68a79` and later
do; it was verified with `cargo check --workspace --all-targets` from a clean checkout.

## Demo

```sh
target/debug/blaze version
cd /path/to/brotli && /path/to/blaze/target/debug/blaze info release output_base

# Run a command the way the referee does (workspace copy, clean env), on a corpus project:
scripts/w4a/run brotli query //... --output=label_kind
scripts/w4a/run brotli cquery //... --output=label_kind
scripts/w4a/run re2 aquery '//:re2' --output=jsonproto

# Score Blaze against Bazel on every surface:
scripts/oracle/record --binary target/debug/blaze --label demo --project brotli
scripts/oracle/compare oracle-9.3.2 demo
```

## Layout

- `crates/`: one crate per cluster of Bazel Java packages (`docs/crates.md`).
- `third_party/starlark-rust`: vendored starlark-rust with a numbered patch queue (`PATCHES.md`).
- `scripts/oracle`: the referee (record, compare, scoreboard). `scripts/upstream-tests`: runs
  Bazel's own shell/Python integration tests against any binary.
- `AGENTS.md`, `PORTING.md`: rules the porting agents follow. `docs/research/`: background
  research. `docs/decisions.md`: rulings. `docs/STATUS.md`: where things stand.
