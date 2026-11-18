<img src="assets/icon.svg" width="64" height="64" alt="">

# Blaze

A Rust port of [Bazel](https://github.com/bazelbuild/bazel) 9.3.2 for Linux x86-64. Starlark runs
on a vendored, patched [starlark-rust](https://github.com/facebook/starlark-rust); everything else
ports Bazel's Java. The real Bazel 9.3.2 binary is the referee: Blaze's outputs are compared with
it byte for byte or item by item.

## Status

Two real projects are the targets: [brotli](https://github.com/google/brotli) (simple C) and
[re2](https://github.com/google/re2) (C++ with abseil, a pybind11 Python extension via
rules_python, googletest). Through the `blaze` binary, compared with Bazel 9.3.2:

| Surface | brotli | re2 |
| --- | --- | --- |
| `query` (label_kind, streamed_jsonproto, incl. `deps()`) | 162/162 | 1091/1091 |
| `cquery` label_kind | 176/176 | 3295/3295 |
| `cquery` jsonproto | 16/16 | 37/37 |
| `aquery` actions (argv, inputs, outputs, action keys) | 60/60 | 1420/1420 |
| `mod graph --output=json` | byte-identical | byte-identical |
| `MODULE.bazel.lock` | byte-identical | byte-identical |
| fetched external repositories | 67/67 | 81/81 |
| build configuration checksums (target, exec, test) | all | all |
| `version`, `info`, `clean`, `shutdown`, rc/`--config` | match | match |

Execution is ported and validated by replaying the oracle's recorded work (every spawn through
Blaze's linux-sandbox runner: 43/43 and 551/551; brotli's 96 outputs; re2's 21 tests). Wiring
`build` and `test` into the binary end to end is in progress; see `docs/STATUS.md`.

Known limits: Linux x86-64 only; one command per process (no persistent server yet);
`linux-sandbox`, `process-wrapper` and `@bazel_tools` are taken from Bazel 9.3.2's install base;
external repositories for `query`/`cquery`/`aquery` currently come from an oracle output base
(Blaze's own fetching is ported and verified separately, wiring in progress).

## Build

Needs Rust 1.97 (pinned in `rust-toolchain.toml`) and, at build time, Bazel 9.3.2's install base
to embed (see `crates/blaze/build.rs`; set `BLAZE_ORACLE_INSTALL_BASE` or run Bazel 9.3.2 once so
it exists under `~/.cache/blaze/oracle-out/ur/install/`).

```sh
scripts/cargo-capped build -p blaze      # -> target/debug/blaze
```

## Demo

```sh
target/debug/blaze version
cd /path/to/brotli && /path/to/blaze/target/debug/blaze info

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
