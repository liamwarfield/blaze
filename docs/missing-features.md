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
- `print_action`
- `mobile-install`

Partly ported:

| Command | Missing |
| --- | --- |
| `mod` | `dump_repo_mapping` (33 upstream tests need it), `show_repo`, `tidy` |
| `help`, `fetch`, `canonicalize-flags`, `license` | ported after v0.1.0 (132 of 137 oracle invocations byte-identical by `scripts/cmds/compare`, which the auditor found can record a signal death as exit 0; upstream test counts to be re-measured under the fixed harness); missing: `canonicalize-flags --invocation_policy`, `fetch --force` with target patterns |
| `info` | 15 of 57 compared cases: `info` with no key, JVM-only keys (heap sizes, `java-home`), `build-language`, `peak-heap-size`, `starlark-semantics` |
| `query` | `--output=graph` and other formatters that need the dependency graph, `--query_file`, SkyQuery (`--universe_scope`), definition and instantiation stacks in proto output, rule class info, native aspects |
| `cquery` | `--show_config_fragments`, some output formats' start/process hooks |
| `aquery` | `--skyframe_state`, the dump after a build (`--experimental_aquery_dump_after_build_format`) |
| `run` | ported after v0.1.0 (exit code and program output equal to Bazel's in 20 of 20 comparison cases; 15 of 20 equal in every field, the others differ in stderr summary lines; upstream test counts to be re-measured under the fixed harness); missing: the Build Event Protocol for `run`, remote flags with `run` |
| `shutdown` | there is no server to stop |

## Server and client

- Persistent server: every command is a new process and starts with an empty in-memory graph.
  Incremental speed comes only from the on-disk action cache and repository cache.
- Waiting for the command lock (Blaze behaves as `--noblock_for_lock`).
- `command.log` (`--write_command_log`).
- Invocation policy (`--invocation_policy`).
- Server idle tasks (disk cache garbage collection).
- Thread dumps, memory options that need a JVM (`--iff_heap_size_greater_than`).

## Loading and Starlark

- Symbolic macros (`macro()`), lazy macro expansion.
- Subrules (`subrule()`).
- Materializer rules and dormant dependencies.
- Autoloads with a non-default `--incompatible_autoload_externally`.
- `--experimental_starlark_type_checking`, `--max_computation_steps`.
- `testing.analysis_test`, `testing.TestEnvironment`.
- Implicit outputs computed by a Starlark callback.
- `proto.encode_text`.
- Project files (`PROJECT.scl`) and scoped flags.
- Query and analysis test harnesses (about 1,700 ported tests are ignored for lack of them).

## Analysis and configuration

- Native aspects (Starlark aspects are ported).
- Native rules without an analysis implementation: `genquery`, `starlark_doc_extract`,
  `action_listener`, `extra_action`, `environment`, `java_plugins_flag_alias`.
- Java: the Java configuration fragment and `java_common` built-ins. No Java target has been
  analysed.
- Apple and Objective-C fragments, Android.
- Platform mappings file (`--platform_mappings`), platform-based flags.
- Feature flags and `string_set` build settings as transition inputs; some Starlark option
  value types in transitions.
- `--target_environment` and environment constraint checks.
- Analysis-failure testing (`allow_analysis_failures`, analysis-test transitions).
- `ctx.resolve_command`, `ctx.resolve_tools`, `ctx.actions.do_nothing`,
  `ctx.actions.map_directory`, shadowed actions, `ctx.actions.transform_info_file`.
- Coverage: `--collect_code_coverage`, instrumented-files providers, baseline coverage,
  the coverage report action, the instrumentation filter.
- `--compile_one_dependency`.
- `--test_size_filters`, `--test_timeout_filters`, `--test_lang_filters`.
- `--toolchain_resolution_debug`.
- Path mapping (`--experimental_output_paths=strip`).
- Remote analysis caching (`--experimental_remote_analysis_cache_mode`).
- Skipped (incompatible) top-level targets in the build event stream.

## C++

- Include scanning.
- Compile action templates for tree artifacts (`createCppCompileActionTemplate`; 6 upstream
  tests).
- ThinLTO backend actions.
- `expand_and_tokenize`, the STL computed default.

## Execution

- Persistent workers (`--strategy=...=worker`).
- Dynamic execution.
- Docker sandbox; sandbox cgroup and memory limits.
- Action rewinding (Bazel's default for lost remote inputs; Blaze supports the
  `--norewind_lost_inputs` path).
- `unused_inputs_list` (panics; 4 upstream tests).
- Exclusive tests (`tags = ["exclusive"]`; 3 upstream tests).
- `--test_output=streamed`; cancelling concurrent test attempts.
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
- Build Event Service upload (`--bes_backend`).
- At their first run (before the round-2 fixes), 143 of 253 of Bazel's remote shell test
  functions did not pass.

## Reporting and UI

- Critical-path line and `Elapsed time` details; `--profile` / `command.profile.gz`.
- The cursor-control progress bar (`--curses=yes`).
- Text-format output: `--build_event_text_file` and `cquery --output=textproto` ported after
  v0.1.0 (printer byte-identical on brotli and re2); `aquery --output=proto` / `textproto` /
  `streamed_proto` still missing (only `jsonproto` works).
- Build event stream for commands other than `build` and `test`, and for a command that stops
  before it runs.
- `--build_event_publish_all_actions`, target summary events, build tool logs.
- 4 build-event items per project still differ on brotli and re2.
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
