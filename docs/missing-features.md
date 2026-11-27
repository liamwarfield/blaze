# Missing features

Compiled 2026-11-19 (root) from the tree at checkpoint `2574fef`.

Sources: the 883 `unported!` sites (`grep -rn 'unported!(' crates`), the command registry
(`crates/blaze_runtime/src/command_registry.rs` against `crates/blaze_runtime/src/commands/`),
`crates/blaze_runtime/src/unported_options.rs`, `docs/remote.md` ("Not ported"), `docs/workerd.md`,
and the first run of Bazel's remote shell tests against Blaze.

Limits of this list:

- It lists what fails loudly or is known from a test. A sample of 40 of the 152 "not ported"
  comment sites found 4 that were silent, so some gaps are not here.
- Only brotli (C), re2 (C++, Python, googletest) and workerd (C++, Rust, JS/TS, protobuf,
  Cap'n Proto) have been built. Rule sets no project exercises (Java, Android, Apple, Go) have
  never been run, and their gaps are mostly unknown.
- Each item fails with `blaze: not ported yet: <Class.method>` unless it says otherwise.
- A crash (panic) exits 101; Bazel prints "FATAL: bazel crashed" and exits 37.
- Sizes per feature cluster: `docs/coverage/README.md`.

## Commands

Not ported at all (registered, so `help` lists them, but they do not run):

- `coverage`
- `vendor`
- `dump`
- `config`
- `mobile-install`

Partly ported:

| Command | Missing |
| --- | --- |
| `print_action` | ported, also with `--compile_one_dependency` (output equal to Bazel's on a cc_library and on brotli's `//:brotlidec`) |
| `mod` | `dump_repo_mapping`, `show_repo` and `tidy` ported after v0.1.0 (133 cases: equal in 89 after the tool's exclusion rules, 40 fail the same way on both sides, 4 differ; upstream tests not run yet); missing: `show_repo` proto output for three attribute types |
| `help`, `fetch`, `canonicalize-flags`, `license` | ported after v0.1.0 (137 invocations, equal after normalisation: 120 identical, 13 of them by declared exit code, 16 fail identically on both sides, 1 different; binary sha256 `315c2147…`; upstream test counts to be re-measured); missing: `fetch --force` with target patterns |
| `info` | 15 of 57 compared cases: `info` with no key, JVM-only keys (heap sizes, `java-home`), `build-language`, `peak-heap-size`, `starlark-semantics` |
| `query` | ported after v0.1.0: `Digraph`, the `graph`, `minrank`, `maxrank` and `xml` outputs, `--order_output`, `--graph:*`, `--query_file`, definition and instantiation stacks, the synthetic attribute hash, `loadfiles()`; missing: `--proto:rule_classes`, SkyQuery (`--universe_scope`), native aspects. Comparison by an unaudited tool, upstream tests not run |
| `cquery` | `build`, `graph`, `files` outputs ported after v0.1.0; missing: `--output=starlark`, `--output=transitions`, `--show_config_fragments` (stops in analysis) |
| `aquery` | `--output=proto`, `streamed_proto`, `textproto` ported after v0.1.0; missing: `--skyframe_state`, the dump after a build (`--experimental_aquery_dump_after_build_format`) |
| `run` | ported after v0.1.0 (20 cases: 2 identical, 12 differ only in lines that depend on Bazel's warm server, 6 differ; upstream test counts to be re-measured); missing: remote flags with `run` |
| `shutdown` | there is no server to stop |

## Server and client

- Persistent server: every command is a new process and starts with an empty in-memory graph.
  Incremental speed comes only from the on-disk action cache and repository cache.
- Waiting for the command lock (Blaze behaves as `--noblock_for_lock`).
- `command.log` (`--write_command_log`).
- Invocation policy: ported after v0.1.0 (parser and enforcer; 63 invocations: 55 identical,
  14 of them by declared exit code, 8 fail identically on both sides); a non-default
  strategy policy still fails loudly.
- Server idle tasks (disk cache garbage collection).
- Thread dumps, memory options that need a JVM (`--iff_heap_size_greater_than`).

## Loading and Starlark

- Symbolic macros, lazy macro expansion, subrule errors, and the loading side of materializer
  rules and dormant dependencies: ported after v0.1.0 (52 comparison cases: equal in 19, 23 fail
  the same way on both sides, 10 differ; brotli and re2 score unchanged; workerd
  cquery 38,892 of 38,892). Open: sibling macros evaluate in order where Bazel evaluates them
  in parallel (the order of `print()` lines differs under lazy expansion); the analysis side of
  materializers (`MaterializingDefault.resolve`).
- Autoloads (`--incompatible_autoload_externally`, all values): ported after v0.1.0, with
  `--experimental_sibling_repository_layout`, `--nofetch`, PAX sparse tar entries and the
  mapping of every failure-detail category to its exit code (errors such as a yanked module
  version used to panic). On the 20 upstream targets these blocked, 389 of 528 oracle-passing
  functions pass (run `repo-gaps-1`, under the harness lock; the v0.1.0 figure for the same
  targets comes from the withdrawn run and is not quoted).
- `--experimental_starlark_type_checking`, `--max_computation_steps`.
- `testing.analysis_test`, `testing.TestEnvironment`.
- Implicit outputs computed by a Starlark callback.
- `proto.encode_text`.
- Project files (`PROJECT.scl`) and scoped flags.
- Query and analysis test harnesses (about 1,700 ported tests are ignored for lack of them).

## Analysis and configuration

- Native aspects (Starlark aspects are ported).
- Native rules without an analysis implementation: `genquery`, `starlark_doc_extract`,
  `environment`, `java_plugins_flag_alias`.
- Extra actions (`action_listener`, `extra_action`, `--experimental_action_listener`): ported
  for builds with merged analysis and execution (Bazel's default); the path without it
  (`BuildView.addExtraActionsIfRequested`) and `ctx.actions.do_nothing` are not.
- Java: the Java configuration fragment and `java_common` built-ins. No Java target has been
  analysed.
- Apple and Objective-C fragments, Android.
- Platform mappings file (`--platform_mappings`), platform-based flags.
- Feature flags and `string_set` build settings as transition inputs; some Starlark option
  value types in transitions.
- `--target_environment` and environment constraint checks.
- Analysis-failure testing (`allow_analysis_failures`, analysis-test transitions).
- `ctx.resolve_command`, `ctx.resolve_tools`, `ctx.actions.do_nothing`,
  `ctx.actions.map_directory`, `ctx.actions.transform_info_file`.
- Coverage: `--collect_code_coverage`, instrumented-files providers, baseline coverage,
  the coverage report action, the instrumentation filter.
- `--toolchain_resolution_debug`.
- Path mapping (`--experimental_output_paths=strip`).
- Remote analysis caching (`--experimental_remote_analysis_cache_mode`).
- Skipped (incompatible) top-level targets in the build event stream (their analysis-failure
  event is posted; the loading/analysis events around it are not, see "Build event stream").

## C++

- Include scanning.
- ThinLTO backend actions.
- `expand_and_tokenize`, the STL computed default.

## Execution

- Persistent workers (`--strategy=...=worker`).
- Dynamic execution.
- Docker sandbox; sandbox cgroup and memory limits.
- Action rewinding (Bazel's default for lost remote inputs; Blaze supports the
  `--norewind_lost_inputs` path).
- `unused_inputs_list` and C++ compile action templates for tree artifacts: ported after
  v0.1.0 (11 of 12 probe steps equal; the twelfth differs in two runfiles action keys and
  the tool has no allow list; brotli and re2 score unchanged). Still missing: `SpawnActionTemplate` / `ctx.actions.map_directory`,
  a shadowed action that discovers inputs.
- Exclusive tests (`exclusive`, `exclusive-if-local`, `--test_strategy=exclusive`),
  `--test_output=streamed`, `--experimental_cancel_concurrent_tests`, the test size, timeout
  and language filters and the test.xml parser: ported after v0.1.0. Exit code and run order
  equal Bazel's in 21 of 21 probe cases (15 of them deliberate failures on both sides); the
  comparison tool is not audited yet and Bazel's own tests have not been run on it. The run
  order of exclusive tests reproduces a Java HashMap's iteration order, checked against the
  oracle with a debugger.
- Filesets; archived tree artifacts.
- CPU-load-based scheduling; `resource_set` overrides in some forms.
- `linux-sandbox`, `process-wrapper` and `@bazel_tools` are taken from Bazel 9.3.2's install
  base, not built by Blaze.

## Remote caching and execution

Working on brotli and re2 against Bazel's remote worker; review verdict STOP in round 2 (fixes
in, not re-reviewed). Missing:

- HTTP/HTTPS cache (`--remote_cache=http(s)://`); gRPC only.
- Credential helpers and Google credentials.
- Content-defined chunking.
- Remote output service, remote downloader, remote repo contents cache.
- Scrubbing config (`--experimental_remote_scrubbing_config`).
- Connection pool sizes other than the default; unix domain sockets.
- Cache lease extension with `--norewind_lost_inputs`.
- Disk cache garbage collection.
- Remote flags on commands other than `build` and `test`.
- Build Event Service upload (`--bes_backend`): ported (crate `blaze_bes`, 2026-11-26). Not
  ported: the attempt number of a retried command (a retried `--bes_backend` command fails
  loudly), `mobile-install` with `--bes_backend`, the `BuildEventServiceUploadCompleteEvent` and
  `LargeBuildEventSerializedEvent` subscribers (Bazel has none), and the server-shutdown wait
  for asynchronous uploads (`blazeShutdown`).
- At their first run (before the round-2 fixes), 143 of 253 of Bazel's remote shell test
  functions did not pass.

## Reporting and UI

- Critical-path line and `Elapsed time` details; `--profile` / `command.profile.gz`.
- The cursor-control progress bar (`--curses=yes`).
- Text-format output: `--build_event_text_file` and `cquery --output=textproto` ported after
  v0.1.0 (on Bazel's own brotli and re2 outputs: build events identical per record, cquery
  byte-identical, aquery by reprint only); `aquery --output=proto`, `textproto` and
  `streamed_proto` ported since.
- Build event stream: `run`, `fetch`, `query`, a command that stops before it runs, target
  summaries, aspect events, failed patterns and analysis failures, and progress stdout/stderr
  are ported (bep-wave-1); the stream of `cquery` / `aquery`, `--nobuild`, `fetch` with targets
  and `--build_event_publish_all_actions` were ported later (reporting fixer, 2026-11-25);
  still missing: Skymeld's interleaving of `targetConfigured` / `targetCompleted` and the late
  `convenienceSymlinksIdentified`. The Build Event Service upload is ported (see "Remote").
- `buildMetrics`: `memoryMetrics` is absent (JVM heap and garbage collection; Blaze has no JVM).
  The other groups are sent: `artifactMetrics` (counts equal to Bazel's on a brotli build),
  `cumulativeMetrics` (counted for every command, as in Bazel) and the Skyframe graph
  statistics of `buildGraphMetrics` (`builtValues`, `evaluatedValues`,
  `postInvocationSkyframeNodeCount`, dirtied/changed/cleaned on a warm server).
  `evaluatedValues` is a tool-specific measurement, like the timings: it counts `compute()`
  calls, and Blaze's evaluator does not restart functions, so it is close to `builtValues` where
  Bazel's is about twice as large; comparison tools must exclude it by name. The referee does
  not compare `buildMetrics` sub-messages yet, so none of this shows in its scores.
- 4 build-event items per project still differ on brotli and re2: `buildToolLogs` (no
  `command.profile.gz`), and the three command lines (`--install_md5` is Blaze's own install
  key; `--restart_reason=no_daemon` because Blaze starts a server per command).
- Execution graph log, `--experimental_stats_summary`, auto output filter for packages.
- Starlark debugger, workspace rules log.
- Error locations and some error texts: 1 of 618 scenario steps matches Bazel in full.

## Repositories and Bzlmod

- Vendor mode.
- `--credential_helper`.
- SOCKS proxies; non-ASCII domain names.
- `repository_ctx` WebAssembly (`load_wasm`, `execute_wasm`) and remote execution of
  repository commands.
- Digest functions other than SHA-256 (`--digest_function=blake3`).

## Platforms

- Linux x86-64 only. No Windows or macOS code paths.
- Scores are comparable only on a 16-CPU machine (Bazel's `cquery` order depends on CPU count).

## Verification gaps (not features)

- Bazel's full integration suite has not been run against Blaze (only the remote subset).
- Work after baseline `ef66f55` (workerd, aspects, byte strings, remote fixes) is not accepted.
- 22 oracle-driven Rust tests pass silently when recordings are missing.
