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

Updated 2026-11-26. The claims, kept apart:

- **Ported**: the code exists and runs.
- **Compared with Bazel**: outputs were compared with real Bazel 9.3.2. "Referee" is the audited
  `scripts/oracle` on brotli and re2; the other comparison tools passed their audit on
  2026-11-26 (`docs/reviews/2026-11-26-audit-4r3.md`) and their numbers are quoted in the
  auditor's wording.
- **Reviewed**: an independent, read-only reviewer compared the Rust with Bazel's Java and
  returned PASS or STOP (`docs/reviews/`). A STOP names blocking findings that are being fixed.
- **Accepted**: in the accepted baseline (`ef66f55`). Nothing ported after that commit is
  accepted yet; acceptance needs a PASS for every area and a recorded score.

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

### Ported after the baseline, not yet accepted

Review verdicts are those at the newest checkpoint each area was reviewed at (2026-11-26).
Comparison figures in the auditor's wording were measured on the binary of `e8b79c0`, before the
review fixes; the suite figures under "Measures" are newer.

| Feature | Reviewed |
| --- | --- |
| Starlark aspects; full workerd build (output binary sha256 equal to Bazel's; query 46,371 and cquery 38,892 lines identical) | not as its own area |
| Bazel's byte-string semantics for non-ASCII text | PASS |
| Incremental evaluator: dirtiness, change pruning, invalidation; equality and hermeticity of all node types | PASS |
| Persistent server and client (opt-in, `BLAZE_SERVER_MODE=daemon`), idle tasks, `info`, server restart on a changed binary | PASS |
| Incremental builds on the server: the graph kept between commands, `--watchfs` | STOP: three stale paths (repeated `--invocation_id`, two in `--watchfs`); a remote or disk cache discards the graph for now |
| `run`, `help`, `fetch`, `canonicalize-flags`, `license`, `mod`, invocation policy, `config`, `dump`, `print_action`, `vendor` | PASS |
| Symbolic macros, autoloads, Bazel's syntax front end and error texts | PASS |
| Java rules; platform mappings, transitions, `--target_environment` | PASS |
| Execution under `--experimental_output_paths=strip`; analysis-failure testing; the validation aspect | PASS |
| Persistent workers, hardened worker sandbox, cgroups (new and old), exclusive tests, streamed test output, `test.xml` parsing | PASS |
| Dynamic execution; Docker sandbox (never executed here: no daemon access) | PASS |
| `coverage` and coverage reports | PASS |
| Build events for every command, text-format output, build metrics, `command.profile.gz`, progress bar, query `graph`/`xml`/rank outputs | PASS |
| Extra actions (`action_listener`, `extra_action`), `ctx.actions.do_nothing` | PASS |
| cquery `starlark` and `transitions` outputs, `--show_config_fragments`, `genquery`, aquery dump after a build | STOP: one finding in the `ctx.var` key tracking |
| Disk cache, gRPC and HTTP remote cache, remote execution, build without the bytes, credential helpers, remote downloader, remote repository contents cache, remote execution of repository rules | STOP: fixes in, re-review owed |
| Build Event Service upload (`--bes_backend`) | STOP: fixes in, re-review owed |
| Action rewinding for lost inputs and outputs (a rewound action runs alone: slower than Bazel, declared) | not yet |

### In progress

| Feature | State |
| --- | --- |
| The server as the default | after the incremental review passes; in server mode the suite figure is 2,803 of 2,878 |
| SkyQuery (`--universe_scope`, `rbuildfiles`, `allrdeps`) | being ported |
| Include scanning; `testing.analysis_test`; `starlark_doc_extract`; WebAssembly in repository rules | being ported |
| Content-defined chunking, cache lease extension, `--digest_function=blake3` | being ported |
| Going through Bazel's Java file by file against the coverage map | execution, remote, analysis, packages, C++ and configuration clusters are assigned |

### Not started

`mobile-install`; Apple and Android rules; the Starlark debugger. Sizes per area are in
`docs/coverage/README.md` (after checkpoint 39: about 86k ± 15k of 406k lines of needed Java
are not ported or not yet marked as ported, 13k of them remote analysis caching, which is out
of scope).

### Declared deviations from Bazel

Recorded with their reasons in `docs/decisions.md`: JVM-only `info` keys and memory figures
report process memory; chains of X.509 v1/v2 certificates are refused (four of Bazel's TLS test
functions fail for that reason); Java stack frames in crash and worker error output cannot be
produced; step limits and step counts of the Starlark interpreter are not reproduced; a few
orders that Bazel takes from identity hashes or thread scheduling.

### Out of scope

Windows and macOS; JVM-only features (heap `info` keys, thread dumps); remote analysis caching.

### Measures that cover everything

- **Bazel's own integration suite** (`docs/upstream-tests.md`): at tag v0.1.0, 63 of 211 targets
  and 1,401 of 2,878 test functions that pass on Bazel pass on Blaze (label `blaze-v0.1.0-r3`).
  At `a673e29` (2026-11-26), one process per command, the default: 163 of 211 targets and
  2,679 of 2,878 functions (label `blaze-a673e29`). At `38cd662` with the persistent server
  switched on (`BLAZE_SERVER_MODE=daemon`, opt-in until its review passes): 190 targets and
  2,803 functions (label `blaze-38cd662-daemon`).
- **Error and UI transcripts** (`scripts/w4/probe`, binary of `e8b79c0`): 777 steps of 42
  scenarios. Of 664 that run the binary: 30 identical; 249 equal only after the masks for the
  warm server and the critical path; 2 equal only as sorted messages; 256 are failing steps
  equal in some view (not counted); 127 differ. Of 113 file checks: 112 identical.
- **Referee** (`scripts/oracle`, referee batch 3b, base `corpus/scores/blaze-ef66f55e904e.referee3b.json`):
  the binary of `a673e29` matches 13,852 of 13,858 judged items in one-process mode and 13,858
  of 13,860 in server mode on brotli and re2 (verification recordings, not accepted scores).

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
