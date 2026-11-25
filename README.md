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
  tools were written by the implementers of each feature; an audit of their hardening returned
  STOP (`docs/reviews/2026-11-19-audit-4.md`), so those rows say exactly what was compared.
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

### Done, compared with Bazel, not yet accepted

"Reviewed" is the verdict of the read-only review of 2026-11-25 at `e8b79c0` (reports in
`docs/reviews/`). A STOP names blocking findings that are being fixed. Comparison numbers are
quoted in the wording the auditor approved (`docs/reviews/2026-11-25-audit-4r2.md`); "none
quotable" means the comparison tool has to be rerun or fixed first.

| Feature | Compared with Bazel | Reviewed |
| --- | --- | --- |
| Starlark aspects; full workerd build | output binary sha256 equal to Bazel's; query 46,371 and cquery 38,892 lines identical | no |
| Bazel's byte-string semantics for non-ASCII text | 127 items: 112 identical; 3 stderr files differ only in lines the tool files under Bazel's warm server; 12 differ (Blaze exits 37 on the flag-off workspace) | PASS (2026-11-20) |
| Disk cache, gRPC remote cache, remote execution, build without the bytes | lead-measured against Bazel's own remote worker on brotli and re2 | STOP in round 3; blocking fix in, re-review pending |
| Incremental evaluator (dirtiness, change pruning, invalidation); not used by a build yet | 426 unit tests ported from Bazel; randomized model test | PASS |
| Persistent server and client (opt-in, `BLAZE_SERVER_MODE=daemon`) | none quotable | STOP: stale per-command state, event-bus leak |
| `run` | 20 cases: 2 identical; 12 differ only in lines that depend on Bazel's warm server; 6 differ | STOP: one summary-line defect |
| `help`, `fetch`, `canonicalize-flags`, `license`, invocation policy | 199 invocations: exit code, stdout and stderr equal after normalisation in 174 (27 by declared exit code); 24 fail the same way on both sides; 1 differs | STOP (same review as `run`) |
| `mod dump_repo_mapping`, `show_repo`, `tidy` | 133 cases: equal in 89 after the tool's listed exclusion rules; 40 fail the same way on both sides (not counted as matches); 4 differ | STOP (same review) |
| Symbolic macros, autoloads, Bazel's syntax front end and error texts | macros, 52 cases: equal in 19; 23 fail the same way on both sides (not counted); 10 differ | STOP: one `load()` error text |
| Java rules (library, binary, test, plugins, deploy jars, toolchains) | none quotable | STOP: one action-key defect under path stripping |
| Platform mappings, platform flags, `--target_environment`, project flag scoping | 31 of the tool's 68 cases: none identical; 15 differ only in warm-server lines; 15 differ; 1 stops at a function not ported | no blocking finding in this part |
| Persistent workers; exclusive tests, streamed test output, test filters | none quotable | STOP: exclusive-test order, worker error text, non-UTF-8 `test.xml` |
| `unused_inputs_list`, C++ compile action templates | 12 steps: 9 identical, 3 differ (aquery text and stderr) | no blocking finding in this part |
| Build events for `run`, `fetch`, `query`; target summaries; text-format output; `command.profile.gz`; progress bar; query `graph`/`xml`/rank outputs | text format on Bazel's own outputs: build events identical per record, cquery byte-identical, aquery by reprint and round trip only | STOP: `buildMetrics` event incomplete; stream unterminated after a crash |
| Starlark core error texts | implementer's probe only | see the syntax front end row |

### In progress

| Feature | State |
| --- | --- |
| Fixes for the six STOP verdicts above | being fixed, then re-reviewed |
| Incremental builds from a kept graph | package 2 of 3: equality and hermeticity of the 84 node types |
| Server as the default | after the server fixes and a soak test |
| `coverage` | being ported |
| `config`, `dump`, `print_action` | being ported |
| Build Event Service upload | being ported |
| Dynamic execution | being ported |
| Comparison tools | audit round 2: STOP on five findings; the numbers above are the ones the auditor allows |

### Not started

`mobile-install`; native aspects; `genquery`, extra actions; include scanning; Docker sandbox;
action rewinding; SkyQuery; vendor mode; remote HTTP cache, credential helpers and chunking;
Apple and Android rules. The full list is `docs/missing-features.md`; sizes per area are in
`docs/coverage/README.md` (at `e8b79c0`: about 106k ± 15k of 406k lines of needed Java are not
ported, 13k of them remote analysis caching, which is out of scope).

### Out of scope

Windows and macOS; JVM-only features (heap `info` keys, thread dumps); remote analysis caching.

### Measures that cover everything

- **Bazel's own integration suite** (`docs/upstream-tests.md`): at tag v0.1.0, 63 of 211 targets
  and 1,401 of 2,878 test functions that pass on Bazel pass on Blaze (label `blaze-v0.1.0-r3`).
  The run for the present code is being recorded.
- **Error and UI transcripts** (`scripts/w4/probe`): of 664 probe steps that run the binary, 40
  give the same exit code, stdout and stderr as the recorded Bazel transcripts (most are error
  cases; equal failures count; the Bazel transcripts name no binary).
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

Before running the analysis tests: `scripts/test-deps/prepare` (materialises the rules_cc,
rules_java, rules_python, bazel_skylib, platforms and protobuf versions of the pinned Bazel into
`~/.cache/blaze/test-deps`, with a sha256 manifest; the tests fail without it).

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
