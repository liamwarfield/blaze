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

**Score** at commit `ef66f55`, from a binary built on a clean checkout, with Blaze's own
repository fetching and no oracle shortcuts (label `blaze-ef66f55e904e`; full table in
`corpus/scores/blaze-ef66f55e904e.md`):

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
A third project, [workerd](https://github.com/cloudflare/workerd), builds end to end with Blaze
on its untouched checkout: 9,713 actions, Bazel's exact summary line, and an output binary with
the same sha256 as Bazel's; `query` and `cquery` match in full. That was measured with a binary
built from an uncommitted tree, so workerd still counts as missing in the score file; see
`docs/workerd.md`.

## Feature status

Updated 2026-11-19. Three columns, three different claims:

- **Ported**: the code exists and runs.
- **Compared with Bazel**: outputs were compared with real Bazel 9.3.2 by a script; the
  comparison is named. "Referee" is the audited `scripts/oracle` on brotli and re2; the other
  tools were written by the implementers of each feature (hardened today, audit in progress).
- **Reviewed and accepted**: an independent reviewer compared the Rust with Bazel's Java and
  returned PASS (`docs/reviews/`), and the feature is in the accepted baseline (`ef66f55`).
  Nothing ported after that commit is accepted yet.

### Done and accepted

| Feature | Ported | Compared with Bazel | Reviewed and accepted |
| --- | --- | --- | --- |
| Module resolution, `MODULE.bazel.lock`, `mod graph` | yes | referee: identical | yes |
| Repository fetching, downloader, repository rules | yes | referee: identical outputs | yes |
| Loading, BUILD and `.bzl` evaluation (starlark-rust, patched) | yes | referee: identical | yes |
| `query` (label_kind, streamed_jsonproto) | yes | referee: identical, with order | yes |
| Analysis, configuration, toolchain resolution | yes | referee: identical | yes |
| `cquery` (label_kind, jsonproto), `aquery` (jsonproto) | yes | referee: identical, with order; runfiles action keys allow-listed | yes |
| Actions, action keys, C++ feature engine | yes | referee: identical | yes |
| `build`, `test`: local and sandboxed execution, action cache | yes | referee: execution log, output hashes and test results identical | yes |
| Evaluator (blocking Skyframe port) | yes | through every surface above | yes |
| Build event stream, binary and JSON files, for `build` and `test` | yes | referee: 37 of 41 and 109 of 113 items | yes, with those items open |
| `info` (42 of 57 compared cases), `clean`, `version` | yes | `scripts/w1/compare-info` | yes |

### Done, compared with Bazel, not yet reviewed or accepted

| Feature | Ported | Compared with Bazel | Reviewed and accepted |
| --- | --- | --- | --- |
| Starlark aspects; full workerd build | yes | output binary sha256 equal to Bazel's; query 46,371 and cquery 38,892 lines identical | no |
| Bazel's byte-string semantics for non-ASCII text | yes | `scripts/bytes`: 114 of 115 | review PASS; not in the baseline |
| Disk cache, gRPC remote cache, remote execution, build without the bytes | yes | lead-measured against Bazel's own remote worker on brotli and re2 | review STOP in round 3; the blocking fix is in, re-review pending |
| `run` | yes | 20 cases: exit code and program output equal in 20, every field in 15 | no |
| `help` topics, `fetch`, `canonicalize-flags`, `license` | yes | 137 invocations: 120 identical, 16 fail identically on both sides, 1 different | no |
| `mod dump_repo_mapping`, `show_repo`, `tidy` | yes | 133 cases: exit code and stdout equal in all; stderr differs in 98 (progress line) | no |
| Text-format output (`--build_event_text_file`, `cquery --output=textproto`) | yes | printer byte-identical on Bazel's own outputs | no |
| Invocation policy | yes | 63 invocations identical | no |
| Symbolic macros, lazy macro expansion | yes | 45 of 52 cases; referee unchanged | no |
| `unused_inputs_list`, C++ compile action templates | yes | every probe step equal except two allow-listed keys; referee unchanged | no |
| Java rules (library, binary, test, plugins, deploy jars, toolchains) | yes | 18 of 18 steps, under `--spawn_strategy=linux-sandbox` only | no |
| Platform mappings, platform flags, `--toolchain_resolution_debug`, `--target_environment`, project flag scoping | yes | 26 of 31 cases; referee unchanged | no |
| Build events for `run`, `fetch`, `query`; target summaries, aspect events | yes | scenario matrix; score rows unchanged | no |
| Starlark core error texts | mostly | 456 of 533 cases equal to Bazel's interpreter | no |
| Bazel-style crash handling (exit 37) | yes | one crash case compared | no |

### In progress

| Feature | State |
| --- | --- |
| Persistent server (client daemon, gRPC server, command lock, `shutdown`) | being built; today every command is one process |
| Incremental builds from a kept graph | evaluator changes being built; behind a switch until proven |
| Remaining `query`/`cquery`/`aquery` output formats, `--query_file` | being ported |
| Console output: progress bar, critical path, summary lines | being ported; 130 of 777 error-scenario steps match Bazel in full |
| Persistent workers | being ported |
| Exclusive tests, `--test_output=streamed`, test size/timeout/language filters | being ported |
| Build Event Service upload, events for `cquery`/`aquery` | being ported |
| Flag sets (`--scl_config`), remaining Starlark transition cases | being ported |
| Reporting every syntax error of a file, as Bazel does | being ported |
| Remote: repo contents cache, HTTP cache, credential helpers, chunking | queued behind the remote review |

### Not started

`coverage`; `dump`, `config`, `print_action`, `mobile-install`; native aspects; `genquery`,
extra actions; include scanning; dynamic execution; Docker sandbox; action rewinding; SkyQuery;
`--profile` / `command.profile.gz`; vendor mode; Apple and Android rules. The full list is
`docs/missing-features.md`; sizes per area are in `docs/coverage/README.md` (about 139k ± 15k
of 406k lines of needed Java were unported this morning).

### Out of scope

Windows and macOS; JVM-only features (heap `info` keys, thread dumps); remote analysis caching.

### Measures that cover everything

- **Bazel's own integration suite** (`docs/upstream-tests.md`): the first total for v0.1.0 was
  withdrawn after an audit found the run contaminated; a clean run is being recorded.
- **Error and UI transcripts** (`scripts/w4/probe`): 130 of 777 steps identical to Bazel in
  full.
- **Rust tests**: 6,615 passed, 0 failed, 2,021 ignored at the last full run (remote lead,
  dirty tree at `cc7373c`); the accepted baseline is 5,487 passed.

Known limits: Linux x86-64 only, and scores are only comparable on a 16-CPU machine (Bazel's
`cquery` order depends on the CPU count); `linux-sandbox`, `process-wrapper` and `@bazel_tools`
come from Bazel 9.3.2's install base.

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
