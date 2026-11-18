# Bazel's test infrastructure as a target for a Rust port

Researched 2026-11-17 from a checkout of
[bazelbuild/bazel](https://github.com/bazelbuild/bazel) at `43cad519` (master, 2026-11-17).
Latest stable release is 9.3.2 (2026-11-05); 10.0.0 is at rc3. All counts below were measured on
master with `find`, `wc` and `grep`, so treat them as close, not exact, and measure again at
whatever commit the port pins.

## Short answer

Yes, with a different shape from TypeScript's.

TypeScript gave Theo one data-driven corpus (12,932 cases with checked-in expected output) that a
new binary could be scored against directly. Bazel has no equivalent. What it has instead:

1. about 2,700 black-box integration tests that drive a `bazel` binary found on `PATH`;
2. 16,600 JUnit tests that are bound to the Java classes and are usable only if ported with them;
3. several machine-readable outputs from the real binary (`query`, `cquery`, `aquery`, the
   execution log, the lockfile) that make a differential oracle possible over any workspace;
4. a large public corpus of real Bazel projects with declared build and test targets.

Items 3 and 4 together can stand in for the missing baseline corpus. They have to be built into a
harness first; they do not come ready to run.

## What has to be ported

| Area | Files | Lines |
| --- | ---: | ---: |
| Server, Java (`src/main/java`) | 3,145 | 642,518 |
| Client, sandbox and native helpers, C++ (`src/main/cpp`, `tools`, `native`) | 120 | 26,056 |
| Protos (`src/main/protobuf`) | 35 | 6,854 |
| Built-in Starlark (`src/main/starlark`) | 23 | 2,203 |

Largest Java packages: `skyframe` 104k, `analysis` 79k, `bazel` 42k (of which `bzlmod` 18k and
`repository` 16k), `packages` 41k, `rules` 38k (of which `cpp` 17k), `runtime` 35k, `remote` 34k,
`actions` 27k, `query2` 24k, the Starlark interpreter (`net/starlark/java`) 35k, the evaluation
engine (`build/skyframe`) 19k.

For scale: ts-rust ended at 874k lines of Rust. Bazel's server is in the same range, and Java to
Rust is a longer translation than Go to Rust (class inheritance, exceptions, annotation
processors, reflection). Roughly: 894 `@StarlarkMethod` sites, 1,123 `@Option` flag definitions,
311 `@AutoCodec` and 97 `SkyFunction` implementations.

Two things reduce the surface. Most language rules now live outside the binary in Starlark
(`rules_cc`, `rules_java`, `rules_python`, `rules_shell`), and the Java tools Bazel ships
(JavaBuilder, turbine, singlejar, the test runner) are programs Bazel runs, not part of the
server, so a port can use upstream's builds of them unchanged.

Bazel's own `BUILD` files declare 1,288 `java_library` targets in 179 packages under
`src/main/java`. That is a ready dependency graph for port order and crate boundaries.

## Test inventory

| Suite | Size | Drives | Reusable by a Rust binary? |
| --- | --- | --- | --- |
| Shell integration tests, `src/test/shell` | 213 files, 98,723 lines, 2,228 test functions | `bazel` on `PATH` | Yes, with a runfiles shim |
| Python integration tests, `src/test/py/bazel` | 42 files, 475 test methods | `io_bazel/src/bazel` via runfiles | Yes, same shim |
| Java black-box tests, `lib/blackbox` | small | a Bazel binary as a subprocess | Yes, needs a JVM to run the tests |
| JUnit tests, `src/test/java` | 1,576 files, 577,459 lines, 16,649 `@Test` | Java classes in-process | Only by porting the test with the class |
| Starlark script tests, `net/starlark/java/eval/testdata` | 38 `.star` files, 3,632 lines | the interpreter | Yes, data-driven |
| Starlark spec suite, `bazelbuild/starlark` `test_suite/testdata/{java,go,rust}` | separate repo | any interpreter | Yes, data-driven |

### The shell and Python tests

These are the closest thing to a conformance suite. Each test writes a small workspace with
heredocs, runs `bazel`, and asserts on output:

```sh
function test_does_not_fail_horribly() {
  mkdir -p peach
  cat > peach/BUILD <<EOF
load("@rules_shell//shell:sh_library.bzl", "sh_library")
sh_library(name='brighton', deps=[':harken'])
sh_library(name='harken')
EOF
  bazel query 'deps(//peach:brighton)' > $TEST_log
  expect_log "//peach:brighton"
  expect_log "//peach:harken"
}
```

The binary is found through one line in `src/test/shell/bin/bazel`
(`bazel_bin=$(rlocation io_bazel/src/bazel)`) and one in `test_base.py`. A harness that lays out a
fake runfiles tree with the port at `io_bazel/src/bazel` can run them unmodified.

Coverage by subcommand (invocations in shell tests): `build` 1,943, `test` 343, `info` 331,
`clean` 236, `run` 204, `query` 200, `aquery` 159, `cquery` 131, `shutdown` 113, `coverage` 82,
`fetch` 66.

Largest files by test count: `starlark_repository_test` 101, `remote_execution_test` 94,
`external_integration_test` 75, `build_event_stream_test` 68, `build_without_the_bytes_test` 66,
`configured_query_test` 62, `client_test` 60, `bazel_query_test` 58, `aquery_test` 56.

Limits to plan for:

- **Message matching.** 2,850 `expect_log` and 584 `expect_not_log` calls grep for specific text,
  often error messages. The port must reproduce Bazel's wording, which favours a literal port.
- **Toolchains.** Tests load `rules_shell` (328 times), `rules_cc` (273), `rules_java` (257) and
  `rules_python` (64). Most need a C++ compiler, a JDK or both, and need those rule sets to
  analyse correctly. Few tests exercise the core alone; `genrule` (646 uses) and `sh_test` (380)
  are the common simple cases.
- **Hermetic inputs.** Upstream CI runs them offline against a prepared repository cache and a
  local registry. The harness has to reproduce that or tests will hit the network.
- **Fixtures.** The remote tests start Bazel's own Java remote worker. Use upstream's build of it
  as a fixture.
- **Speed.** They are slow (seconds to minutes each, a server start per test file) and upstream
  shards them across many machines.
- **Environment sensitivity.** Not all 2,700 will pass under real Bazel on a given machine. Run
  the suite against the pinned real binary first. That pass list is the denominator.

### The JUnit tests

Most of the 16,649 tests extend in-process fixtures (`BuildViewTestCase` in 189 files,
`BuildIntegrationTestCase` in 59, `AnalysisTestCase` in 32) that build a scratch file system and
inspect Java objects. A Rust binary cannot run them.

Theo's port faced the same thing with Go's unit tests and ported them alongside the code: most of
its 186,890 passing tests are ported Go tests. The same move works here for the lower layers,
where tests are plain input and output: labels, paths, globs, option parsing, the Starlark
interpreter, the evaluation engine, the query language, the file system layer. It gets expensive
at the analysis layer, where the fixtures are large and mock-heavy. There, the differential
oracle below is the cheaper check.

## Differential oracle: outputs of the real binary that can be compared

The equivalent of running `tsgo` beside `tsc-rs`. Each phase of a build has an output that is
structured and mostly deterministic.

| Phase | Command or flag | What it pins down |
| --- | --- | --- |
| Flags | `bazel help flags-as-proto`, `canonicalize-flags` | Every option, its type, default and expansion |
| Rule definitions | `bazel info build-language` | Every native rule and attribute, as a proto |
| Module resolution | `bazel mod graph --output=json`, `MODULE.bazel.lock` | Version selection; the lockfile compares byte for byte |
| Loading | `bazel query --output=streamed_jsonproto` (also `proto`, `build`, `xml`) | BUILD evaluation, macros, globs, every attribute value |
| Analysis | `bazel cquery --output=jsonproto` (also `starlark`, `transitions`) | Configurations, transitions, toolchain resolution, providers |
| Action graph | `bazel aquery --output=jsonproto` | Every action: mnemonic, argv, environment, inputs, outputs |
| Execution | `--execution_log_json_file`, `--execution_log_compact_file` | Every spawn with input and output digests |
| Events | `--build_event_json_file` | The build event stream (needs normalising for order and time) |
| Results | output tree hashes, exit code, `test.xml`, runfiles trees | The build itself |
| Caching | action digests against a remote cache filled by real Bazel | Whether the two are interchangeable |

`aquery` is the strongest of these. It is the action graph in full, it is available without
running anything, and it does for analysis what TypeScript's `.types` and `.symbols` baselines did
for the checker.

Remote-cache interchange is the sharpest single number for the execution side: fill a cache with
real Bazel, build the same commit with the port, count hits. A full hit rate means the port
computes the same action keys from the same inputs.

What needs care:

- Terminal output carries timing, progress and paths. Compare it only after normalising, or not at
  all.
- Output-base paths and configuration hashes appear inside outputs.
- Event and log order depends on scheduling. Sort or use the sorted modes.
- The real binary has run-to-run differences of its own. Theo's rule applies: record each one on
  an allow list with a condition, checked every run.
- Real Bazel costs a JVM start and a server per workspace. Record its answers once per pin and
  case, keyed by hash, and replay them.

## Corpora of real projects

| Corpus | Size | Notes |
| --- | --- | --- |
| `examples/` in the Bazel repo | small | First real workspaces |
| Rule sets' own tests: `bazel_skylib`, `rules_testing`, `rules_shell`, `rules_cc`, `rules_java` | hundreds of targets | Many are Starlark analysis tests that run inside the build, so `test //...` checks the port's analysis directly |
| Bazel's downstream CI list (`bazelci.py`, `DOWNSTREAM_PROJECTS_PRODUCTION`) | 33 projects | What the Bazel team itself runs before a release: protobuf, gRPC, TensorFlow, Buildfarm, the rule sets |
| Bazel Central Registry | over 1,000 modules | Each version has a `presubmit.yml` naming platforms, build targets and test targets, so the corpus is machine-readable |
| Bazel building Bazel | one, large | The capstone: the port builds upstream Bazel and runs its test suite |

## How this compares with what Theo had

| | TypeScript | Bazel |
| --- | --- | --- |
| Data-driven baseline corpus | 12,932 cases, expected output checked in | None for the build tool. Starlark only |
| Black-box suite with a process boundary | API client tests (966) | Shell and Python tests (about 2,700) |
| In-process unit tests | Go tests, ported | 16,649 JUnit tests, portable for lower layers |
| Reference binary | `tsgo` at a pinned commit | `bazel` at a pinned release |
| Structured comparable output | diagnostics, emit, types, symbols | query, cquery, aquery, exec log, lockfile, BEP |
| Real-project corpus | 120 repos, hand-collected | 33 downstream projects, over 1,000 registry modules |
| Determinism | High: files in, text out | Lower: processes, file system, network, time, concurrency |
| Cost per check | Milliseconds to seconds | Seconds to minutes, plus toolchains |

## Risks specific to Bazel

1. **The hill is less smooth early.** Even a trivial build needs module resolution, repository
   rules and toolchain resolution before anything runs. There are few tests a 10% complete port
   can pass. Milestones have to be defined by oracle surface (flags, then `query`, then `aquery`,
   then execution) so that each has its own score.
2. **Side effects.** Sandboxing, process management, signals, file watching and the client/server
   lifecycle are tested by behaviour, not by text diff, and are platform-specific.
3. **Slow feedback.** Integration tests and corpus builds are heavy. Without capped concurrency
   and off-machine runners, test time becomes the bottleneck, as build slots did for Theo.
4. **Scope.** Windows, macOS, Android and Apple rules, coverage, remote execution, build event
   upload, workers, dynamic execution. Decide what the first target excludes and publish the
   exclusions with every score.
5. **A moving upstream.** Bazel ships often. Pin a release, keep the pin file, and treat each bump
   as its own batch.

## Other things found

- No Rust port of Bazel turned up in a search. Related work: Buck2 (Rust, Starlark, a different
  build model), `starlark-rust` (Meta's interpreter, active, Apache-2.0), and Bonanza (a
  separate build system that aims at Bazel compatibility; not examined here).
- Bazel ships its own `AGENTS.md` and an architecture guide at `docs/contribute/codebase.mdx`.
  Both belong on a porting agent's reading list.
