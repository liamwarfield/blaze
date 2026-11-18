# Upstream integration tests: oracle baseline (Bazel 9.3.2)

Bazel's own shell and Python integration tests (`src/test/shell/**`, `src/test/py/bazel/**` at the
pin), run unmodified against a chosen binary by `scripts/upstream-tests/`. `oracle-9.3.2.json` is
the baseline for the pinned real Bazel on this machine. Its `passing` list is the target Blaze has
to reach.

## Method

- **Outer Bazel.** The pinned oracle runs `bazel test` in a patched copy of the pinned source
  (`~/.cache/blaze/work/upstream-src`). The patch changes one thing: `//src:bazel` (runfiles path
  `io_bazel/src/bazel`, which `src/test/shell/bin/bazel` and `test_base.py` run) becomes a copy
  of the binary under test, named by `--repo_env=BLAZE_UNDER_TEST` through the repository rule
  `@blaze_under_test` (`ut_common.py`). Everything else in the test runfiles is upstream's own
  build. Because the outer Bazel runs the tests itself, `test-setup.sh`, `TEST_TMPDIR`, sharding,
  timeouts, `XML_OUTPUT_FILE` and the sandbox all behave exactly as upstream's CI has them; the
  harness reimplements none of this. A hand-built runfiles tree with a symlink swap would have
  to copy all of that environment.
  Running a script binary that prints a marker made every inner `bazel` call print the marker.
  Switching back to the oracle restored the passes.
- **Environment** follows `.bazelrc` `ci-linux`. The oracle builds `//src:test_repos` into its
  repository cache once, as CI does. Tests get `REPOSITORY_CACHE`: those downloads, hardlinked,
  with an empty repo contents cache at the start of each run. They also get `TEST_INSTALL_BASE`
  (shared per binary sha256), `REMOTE_NETWORK_ADDRESS=bazel.build:80`, `PATH`, and
  `--sandbox_default_allow_network=false`. Tests tagged `requires-network` keep the network.
  Every run uses `--nocache_test_results --flaky_test_attempts=1`, so each test runs exactly
  once.
- **Resources.** Each run is one systemd unit: `MemoryMax=14G`, `MemorySwapMax=0`,
  `OOMPolicy=continue`, `--jobs=8`, `--local_test_jobs=6`, outer JVM `-Xmx3g`. `oom-guard`
  raises `oom_score_adj` on every process except the outer Bazel. If the unit hits its memory
  cap, the kernel kills one test's inner Bazel rather than the whole run. `results.json` records
  the run's `oom_kill` count.
- **Selection.** `tests(//src/test/shell/... + //src/test/py/bazel/...)` gives 251 targets
  (218 shell, 33 py). 14 are excluded, 237 run.

| Excluded | Count | Why |
| --- | ---: | --- |
| `manual` | 8 | not run by upstream CI either |
| `binary-variant` | 2 | `minimal_jdk_test`, `nojdk_startup_options_test` need from-source `bazel_jdk_minimal`/`bazel_nojdk` builds, which the harness does not swap |
| `source-artifact` | 3 | the two `bazel_bootstrap_distfile*` tests and `jdeps_test` test Bazel's source tree and server jar, not the binary |
| `self-build` | 1 | `bazel_determinism_test` builds all of Bazel twice (eternal timeout). This is the capstone; run it with `--include-excluded` |

## Baseline

Four full runs of the oracle (sha256 `2e14c55c…`) on Fedora 44 x86-64 with 16 cores. Runs
`oracle-9.3.2` and `oracle-9.3.2-run2` used the final harness. `-v0a` and `-v0b` ran earlier,
when the tests shared the outer repository cache. A name counts as **pass** only if it passed in
all four runs, and as **flaky** if it passed in some of them.

| Suite | Targets run | Pass | Vacuous | Flaky | Fail | Functions seen | Functions pass | Flaky | Not pass |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| shell | 205 | 184 | 10 | 4 | 7 | 2,561 | 2,515 | 38 | 8 |
| py | 32 | 27 | 4 | 1 | 0 | 367 | 363 | 1 | 3 (Windows-only skips) |
| **total** | **237** | **211** | **14** | **5** | **7** | **2,928** | **2,878** | **39** | **11** |

Wall time per full run was 41 to 45 minutes (2,474 s, 2,559 s, 2,719 s, 2,495 s). The tests add
up to about 4 hours of serial time. The one-time `prepare` (fetch, then build every test target
and `//src:test_repos`) took 4.5 minutes once the modules were fetched.

**Vacuous** (14) means the target passes without running a test function on Linux. Nine are
Windows-only, three are macOS-only, and two never run their suite upstream
(`bazel_java_test_no_windows`, `cc_import_starlark_test`). Any binary passes them, so they are
not in `passing`.

### Failure categories (fail in every run)

| Category | Targets | Functions | Cause |
| --- | ---: | ---: | --- |
| host-gcc-gcov | 3 | 3 | `bazel_cc_code_coverage_test`, `bazel_coverage_cc_{head,released}_test_gcc`: GCC 16.2's gcov output differs from the expected text. Upstream CI also skips these on Rocky 8 because of its gcov version. |
| host-no-clang | 2 | 2 | `cc_integration_test#test_external_repo_lto` and `path_mapping_test#test_path_stripping_cc_remote` need clang, which is not installed. |
| host-jdk | 1 | 1 | `bazel_java_test#test_javabase`: `@local_jdk` follows the host `javac`, which is temurin 11 (`java` is 25). VanillaJavaBuilder needs Java 17 or later. |
| release-binary | 1 | 2 | `info_test#test_info`, `#test_multiple_keys` expect `release: development version` from a from-source build. The oracle is a release build. |

No test failed for network, docker, Android SDK, remote or macOS/Windows reasons. At 9.3.2 those
tests either skip themselves on Linux (the vacuous ones) or pass with the CI-style repository
cache. Of the 43 `requires-network` targets, 38 pass. Of the other five, three are flaky, one is
vacuous (macOS-only), and one is `path_mapping_test` (needs clang).

### Flaky (5 targets, 39 functions)

- `bazel_proto_library_test`: shard 2's inner Bazel compiles protobuf with 16 parallel C++ jobs.
  It was OOM-killed under the 14G cap in both v0 runs and in a `--test-jobs 2` retry, and passed
  in the two final runs.
- `external_integration_test`: `test_deferred_download_*` hangs on its fifo-gated HTTP server
  until the 900 s timeout. This happened in 3 of 4 runs, plus the retry.
- `build_event_stream_test`: `test_glob_filesystem_operation_cost` sometimes fails. Its large
  failure log then overflows unittest.bash's perl XML writer (`Argument list too long`), and the
  report loses the later test cases (22 of the 38 flaky shell functions).
- `bazel_worker_test#test_worker_restarts_when_worker_runfiles_change`: the worker restarted 1
  time; the test expects 2.
- `bazel_module_test#testReverseDependencyDirection`: `JUST FETCHED: bar` was missing.

`compare` never counts a flaky name as lost.

## Running it against Blaze

```sh
scripts/upstream-tests/prepare                 # once (and after a pin bump)
scripts/upstream-tests/run --binary <path/to/blaze> --label blaze-<rev>      # ~45 min, all
scripts/upstream-tests/run --binary <blaze> --label blaze-<rev> --file 'bazel_query_test.sh'
scripts/upstream-tests/run --binary <blaze> --label blaze-<rev> --tag requires-network --shard 1/4
scripts/upstream-tests/compare corpus/upstream-tests/oracle-9.3.2.json blaze-<rev>
```

`run help` lists the selection flags: `--suite`, `--file`, `--target`, `--targets-file`, `--tag`,
`--exclude-tag`, `--shard I/N` and `--test-filter`. Results go to
`~/.cache/blaze/upstream-test-results/<label>/results.json`, with one entry per target
(`//pkg:name`) and per function (`//pkg:name#test_fn`, or `#Class.method` for py). Each entry
has a status, duration and log path. Logs are copied under `logs/`. Partial runs into the same
label merge per target. `compare` prints retained/lost/absent/new for targets and functions and
exits 1 on any loss.

The binary is copied, so it has to be a single relocatable executable. The tests pass it Bazel's
startup flags (`--output_user_root`, `--install_base`, `--bazelrc`, `--nohome_rc`, ...).

## Files

- `oracle-9.3.2.json`: counts, runs, categories, failing/excluded/vacuous/flaky lists and
  `passing` (211 targets; 2,878 functions grouped per target).
- Producing tools: `scripts/upstream-tests/{prepare,run,compare,summarize,oom-guard}`.
  Categories are in `scripts/upstream-tests/categories.py`.
- Raw results (not committed): `~/.cache/blaze/upstream-test-results/oracle-9.3.2{,-run2,-v0a,-v0b,-v0retry}`.
