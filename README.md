<img src="assets/icon.svg" width="64" height="64" alt="">

# Blaze

A Rust port of [Bazel](https://github.com/bazelbuild/bazel) 9.3.2 for Linux x86-64. Starlark runs
on a vendored, patched [starlark-rust](https://github.com/facebook/starlark-rust); everything else
ports Bazel's Java. The real Bazel 9.3.2 binary is the referee: Blaze's outputs are compared with
it byte for byte or item by item.

## Status

**Read this first.** An independent review round on 2026-11-18 returned STOP on all seven areas
it covered (`docs/reviews/`). The scores below are real, but they cover two projects with default
flags, and the reviews found wrong behaviour outside that: flags that are accepted and silently
ignored, error paths that differ from Bazel or panic, and action keys that are wrong for
non-ASCII strings. Treat Blaze as a demo of the approach, not a Bazel replacement.

Two real projects are the targets: [brotli](https://github.com/google/brotli) (simple C) and
[re2](https://github.com/google/re2) (C++ with abseil, a pybind11 Python extension via
rules_python, googletest). Compared with Bazel 9.3.2 by `scripts/oracle`:

| Surface | brotli | re2 |
| --- | --- | --- |
| `query` (label_kind, streamed_jsonproto, incl. `deps()`) | 162/162 | 1091/1091 |
| `cquery` label_kind (compared as a set; output order differs from Bazel) | 176/176 | 3295/3295 |
| `cquery` jsonproto | 16/16 | 37/37 |
| `aquery` actions (compared as a set; allow-listed runfiles-manifest keys) | 60/60 | 1420/1420 |
| `mod graph --output=json` | byte-identical | byte-identical |
| `MODULE.bazel.lock` | byte-identical | byte-identical |
| fetched external repositories | 67/67 | 81/81 |
| build configuration checksums (target, exec, test) | all | all |

How these were measured: the query, cquery and aquery rows came from the `blaze` binary reading
external repositories from an oracle output base; the repository and lockfile rows came from a
separate dev binary. A newer, not yet committed integration build runs `blaze build` (brotli) and
`blaze test` (re2) end to end with Blaze's own fetching and matches on outputs, exec log and test
results; it will be recorded against a commit when it lands. Numbers are not yet bound to
commits; that is being fixed.

`version`, `clean`, `shutdown`, rc/`--config` handling and `info <key>` match for 39 of 57
compared cases; the other 18 are not ported and fail loudly (among them `info` with no key and
the JVM-only keys).

Known limits: Linux x86-64 only; one command per process (no persistent server); two Blaze
commands in one output base are not locked against each other; `linux-sandbox`,
`process-wrapper` and `@bazel_tools` are taken from Bazel 9.3.2's install base; Starlark flags
(`--//pkg:flag=value`) are not applied; aspects, genrule and symbolic macros are not ported;
remote caching and execution are not ported; non-ASCII strings do not follow Bazel's semantics.

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
