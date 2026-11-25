# Review: reporting (build events, text format, profiler, progress bar, query outputs)

Date 2026-11-25. Reviewer: Fable 5.2, read-only. Commit under review: **e8b79c0** (code of
cc6c29a; `git diff e8b79c0 -- crates` is empty in the tree I read). First review of this area;
everything in it was ported after the accepted baseline ef66f55.

**Verdict: STOP**, bound to e8b79c0. Blocking findings: S1-1 and S1-2.

No Bazel or Blaze process was started: the suite unit `root-suite-e8b79c0` was active for the
whole review. Every finding rests on the Rust and Java sources and on recordings already on disk
(named with each finding). S1-2 has no recording; it is established by reading only, and its fix
needs a test that produces one.

## Scope read

Read in full, Rust against the pinned Java:

- `blaze_bep`: `runtime/build_event_streamer.rs` (BuildEventStreamer.java, all of it),
  `bep_module.rs` (BuildEventServiceModule.createBepTransports, beforeCommand, afterCommand,
  blazeShutdownOnCrash), `runtime/synchronized_output_stream.rs`,
  `runtime/target_summary_publisher.rs`, `metrics/criticalpath/critical_path_computer.rs` and
  `critical_path_component.rs`, the `asStreamProto` of `runtime/test_summary_event.rs`,
  `TextFormatSerializer` in `buildeventstream/transports/file_transport.rs`.
- `blaze_runtime/src/commands/bep.rs`: the subscriber (lines 186-420), `module_before_command`,
  `build_starting`, `post_target_parsing_complete`, `analysis_metrics`,
  `post_targets_configured`, the skipped-target aborts, `build_complete`,
  `command_precomplete`, `after_command`, the `Drop`; the callers in `commands/build.rs`
  (interrupt path, build-complete sequence), `bazel_modules.rs` (the BES module),
  `dispatcher.rs` and `bugreport.rs` (crash path).
- `blaze_buildtool/src/buildtool/execution_tool.rs`: the critical-path feed and
  `report_build_summary_stats` (BuildSummaryStatsModule.buildComplete).
- `blaze_protos/src/text_format/printer.rs` (whole file).
- `blaze_util/src/profiler/json_trace_file_writer.rs` (whole file) and the start/stop half of
  `blaze_runtime/src/profiler_init.rs`; `docs/profiler-sites.md`.
- Progress bar: `describe_action`, `describe_action_progress`, `describe_test_group`,
  `shortened_label_string`, `suffix` in `blaze_buildtool/src/runtime/ui_state_tracker.rs`;
  `refresh`, `do_refresh`, `time_based_refresh`, the update thread, `clear_progress_bar`,
  `add_progress_bar`, `build_complete` in `blaze_events/src/ui.rs`.
- `blaze_util/src/graph`: `digraph.rs`, `dfs.rs`, `concurrent_collection_wrapper.rs` (whole).
- `blaze_query/src/query/output`: `graph_output_writer.rs`, `graph_output_formatter.rs`,
  `minrank_output_formatter.rs`, `maxrank_output_formatter.rs`, `rank_and_label.rs`,
  `xml_output_formatter.rs`, `query_output_utils.rs`, `get_ordered_targets`;
  `read_query` (`--query_file`); the graph operations of `blaze_query_environment.rs`.
- `blaze_cquery`: `cquery/graph_output_formatter_callback.rs`,
  `aquery/action_graph_proto_output_formatter_callback.rs`.

Recordings used: `~/.cache/blaze/oracle-records/pause-cc6c29a3ede8` (Blaze at this code) and
`oracle-9.3.2` (`brotli/raw/bep.json`); `command.profile.gz` under
`~/.cache/blaze/oracle-out/rec-pause-cc6c29a3ede8` and `rec-oracle-9.3.2-rerun`.

## Scope not read

- `blaze_protos`: `descriptors.rs`, `dynamic_message.rs`, the parser half of `text_format.rs`,
  `json.rs`, `build.rs`. Float and double printing was read only at its entry points.
- `blaze_bep`: `metrics_collector.rs` beyond its entry points, `aggregated_spawn_metrics.rs`,
  `aggregated_critical_path.rs`, `target_complete_event.rs`, `analysis_events.rs`,
  `named_artifact_group.rs`, `artifact_uploader.rs`, `command_line_event.rs`, the binary and
  JSON transports and the file transport's writer thread.
- `bep.rs`: the conversions (`convert_target_complete`, `convert_analysis_failure`,
  `convert_aspect_complete`, `convert_action_executed`), the command-line events, the
  deferred uploader. The `NoBuildEvent` paths of `run`, `fetch`, `query`, `cquery`, `aquery`.
- Profiler: `profiler.rs` (outline only), `counter_series.rs`, `trace_data.rs`,
  `thread_metadata.rs`, `json_writer.rs`, `stat_recorder.rs`, `network_metrics_collector.rs`,
  `collect_local_resource_usage.rs`.
- Progress bar: the rest of `ui_state_tracker.rs` (execution progress, downloads, BEP transport
  lines, the Skymeld bar), `ui_event_subscriber.rs`, the event-handling half of `ui.rs`,
  `terminal.rs`.
- Query: `precise_aspect_resolver.rs`, `conditional_edges.rs`,
  `synthetic_attribute_hash_calculator.rs`, `proto_output_formatter.rs` changes, `loadfiles()`
  beyond its entry points; cquery `build` and `files`, `post_analysis_query_environment.rs`,
  the aquery text and summary formatters.
- No test file was reviewed for where its expectations come from.

## Findings

### S1

**S1-1. The `buildMetrics` build event is silently incomplete on every build. Blocking.**

- Rust: `crates/blaze_runtime/src/commands/bep.rs:1807-1838` (`command_precomplete`) passes
  `worker_process_metrics: Vec::new()` (1819), `profile_elapsed_time_ms: None`,
  `server_process_cpu_time_ms: None`, `network_metrics: None` (1831-1833). The comments say the
  workers, the profiler and the network collector are not ported; all three now are
  (`blaze_exec/src/worker/worker_process_metrics.rs`,
  `blaze_util/src/profiler/profiler.rs:449` and `:462`,
  `blaze_util/src/profiler/network_metrics_collector.rs`).
- Rust: `bep.rs:216-420` (`BepSubscriber::post`) never calls
  `MetricsCollector::on_some_execution_started` (`metrics_collector.rs:638`),
  `handle_execution_phase_complete` (`:659`) or `on_execution_complete` (`:700`). They have no
  caller outside tests. `SomeExecutionStartedEvent` is posted
  (`blaze_sky_exec/src/build_driver_function.rs:150`) but not routed.
- Java: `metrics/MetricsCollector.java:221` onSomeExecutionStarted, `:233`
  handleExecutionPhaseComplete, `:267` onExecutionComplete, `:331-366` createBuildMetrics
  (worker metrics, network metrics), `:544-554` finishTimingMetrics.
  `Profiler.getProfileElapsedTime` (`profiler/Profiler.java:420`) never returns null.
- Input: any build with a build event file. Recorded evidence, brotli, last `buildMetrics`:

  | field | oracle (`oracle-9.3.2`) | Blaze (`pause-cc6c29a3ede8`) |
  | --- | --- | --- |
  | `artifactMetrics.sourceArtifactsRead` | 2328125 bytes, count 115 | absent |
  | `artifactMetrics.outputArtifactsSeen` | 7671064 bytes, count 89 | absent |
  | `artifactMetrics.topLevelArtifacts` | 11766429 bytes, count 165 | absent |
  | `timingMetrics.wallTimeInMs`, `cpuTimeInMs` | present | absent |
  | `timingMetrics.executionPhaseTimeInMs`, `actionsExecutionStartInMs` | present | absent |
  | `networkMetrics` | present | absent |

  The artifact counts and sizes are deterministic. `workerMetrics` and `workerPoolMetrics`
  stay empty on a build that uses persistent workers.
- Why it is not seen today: `scripts/oracle/surfaces.py:34-37` drops `timingMetrics`,
  `networkMetrics`, `workerMetrics`, `workerPoolMetrics` and `artifactMetrics` from the
  comparison.
- Fix expected: route `SomeExecutionStartedEvent`, `ExecutionPhaseCompleteEvent` and
  `ExecutionFinishedEvent` to the collector at the points Java posts them; pass the profiler's
  elapsed and CPU time, `WorkerProcessMetricsCollector::instance().collect_metrics()` and the
  network collector's metrics in `command_precomplete`. Anything left unported there calls
  `unported!` or is named in the report. Root decides separately whether `artifactMetrics`
  (deterministic) should leave the referee's drop list.

**S1-2. A crash leaves the build event stream unterminated. Blocking.**

- Rust: `crates/blaze_runtime/src/commands/bep.rs:1877-1895`. The only crash handling is
  `impl Drop for BepCommandSupport`, which closes the streamer with `AbortReason::Internal`
  when the value is dropped while the thread is panicking.
- It cannot fire. `BazelBuildEventServiceModule::before_command`
  (`crates/blaze_runtime/src/bazel_modules.rs:1006-1010`) keeps a second `Arc` in
  `self.current`, which lives in the module, outside the command. When a panic unwinds out of
  `exec_exclusively` (`dispatcher.rs:261`, `bugreport.rs:555` `catch_crash`), the environment's
  `Arc` is dropped but the module's is not, so `drop` does not run.
- After the catch, `BugReport::handle_crash` reaches
  `BlazeRuntime::shut_down_modules_on_crash` (`runtime.rs:246`), which calls
  `blaze_shutdown_on_crash` on each module. The BES module does not override it (the default
  at `module.rs:154` is empty), although its doc comment (`bazel_modules.rs:975-981`) says it
  does. `after_command` is not run on a crash, as in Java.
- Java: `buildeventservice/BuildEventServiceModule.java:500-509` blazeShutdownOnCrash calls
  `forceShutdownBuildEventStreamer(INTERNAL)` (OUT_OF_MEMORY for an OOM): the stream gets its
  final progress event, an `Aborted` for every announced event, exactly one `last_message`, and
  the transports are closed.
- Input: any command with `--build_event_json_file` that panics. In this port that includes
  every `unported!`, for example the two in S3-1 and S3-2 below. Expected by the Java: the
  file ends with aborted events and `lastMessage: true`. Blaze: the stream stops at the last
  event posted before the panic, with no `last_message`. I did not check whether the events
  still buffered in the transport reach the file.
- Not measured (no probe was allowed); the reading above is unambiguous on the `Arc` count.
- Fix expected: implement `blaze_shutdown_on_crash` for `BazelBuildEventServiceModule`
  (`close_on_abort(Internal)` on `self.current`, then wait for the close results), remove or
  keep the `Drop` as a second line, correct the doc comments, and add a test that crashes a
  command with a BEP file and checks the last event.

### S2

**S2-1. `cumulativeMetrics` restarts at 1 for every command.**
`bep.rs:1145-1153` gives each command new `numAnalyses` and `numBuilds` counters, with the
comment that Blaze runs one command per server process. Java's counters belong to
`MetricsModule` and live as long as the server. Masked while the server default is not switched
on: a second build on a running Blaze server reports 1 and 1 where Bazel reports 2 and 2. Fix:
the counters belong to the module object.

**S2-2. The build-complete progress line with a BEP file is written by hand instead of through
the ported UI.** `bep.rs:1755-1804` (`write_build_complete_progress_bar`) writes `INFO: ` or
`FAILED: ` straight to the reporter's stderr. In Java the line comes from
`UiEventHandler.buildComplete` (`runtime/UiEventHandler.java:714`) because the BES module
posted `AnnounceBuildEventTransportsEvent` (`BuildEventServiceModule.java:442`) and the tracker
has activities until `BuildEventTransportClosedEvent` (`:545`, `:653`). The Rust tracker and
subscriber have both methods (`ui_event_subscriber.rs:306` and `:324`), but nothing in
`blaze_runtime` calls them. The shortcut matches the oracle for `--curses=no` (all recordings
use it, `scripts/bep-matrix:232`); it bypasses the UI handler's state (buffered stdout and
stderr, rate limit, event filters) and it is the reason for S3-1. Fix: post the two events and
delete the hand-written line.

**S2-3. `command.profile.gz` has a different gzip header byte.**
`crates/blaze_util/src/profiler/json_trace_file_writer.rs:31` sets the OS byte to 0. Bazel's
JDK writes 255: `rec-oracle-9.3.2-rerun/.../command.profile.gz` starts
`1f8b 0800 0000 0000 00ff`, Blaze's `rec-pause-cc6c29a3ede8/.../command.profile.gz`
`1f8b 0800 0000 0000 0000`. Fix: `operating_system(255)`.

**S2-4. Progress chunks are split by chars, Java splits by UTF-16 units.**
`crates/blaze_bep/src/runtime/synchronized_output_stream.rs:83` (marked `PORT:`), Java
`runtime/SynchronizedOutputStream.java:64` (`Splitter.fixedLength`). Output longer than the
chunk size that contains a character outside the Basic Multilingual Plane is split at
different places. Low weight; either port the UTF-16 count or record the ruling in
`docs/decisions.md`.

### S3 (important ones)

- **S3-1.** `bep.rs:1759-1763`: with a BEP file, progress shown and cursor control on
  (`--curses=yes`, or the default on a terminal), every build ends in
  `unported!("UiEventHandler.addProgressBar (cursor control ...)")`. Together with S1-2 the
  stream is then left unterminated. Goes away with the fix of S2-2.
- **S3-2.** `bep.rs:1439-1442`: a BEP file with `--keep_going` and a target pattern that fails
  to parse hits `unported!("TargetParsingCompleteEvent.failedTargetPatterns")`.
- **S3-3.** `bep.rs:1129-1133`: a BEP file path that cannot be opened hits
  `unported!("BuildEventServiceModule.reportError")`; Java reports the error and exits with
  `BES_LOCAL_WRITE_ERROR`.
- **S3-4.** `bep.rs:1107-1112` and `profiler_init.rs`: no workspace (output relative to the
  home directory) is `unported!`; so is `--experimental_stream_log_file_uploads`.
- **S3-5.** Profiler: the instrumentation sites of `docs/profiler-sites.md`; two of six phase
  markers; the command thread is named `main` where Bazel has `Main Thread` (seen in the
  recorded profiles: Blaze 123 events and 3 thread names, oracle 316 and 4). Listed honestly in
  the doc.
- **S3-6.** `--experimental_record_skyframe_metrics` and package metrics in the BEP
  (`bep.rs:1824`, `:1505`) are `unported!`; the Build Event Service upload is not ported.
- **S3-7.** Text format: `debug_format_printer` is `unimplemented!`. XML output: a NUL char in
  an attribute value and scalar attribute types other than int, string, bool, tristate and
  label are `unported!` (`xml_output_formatter.rs:465`, `:678`).

### S4

Stale comments (`action_graph_proto_output_formatter_callback.rs:388-390` names an
`UnportedStreamedConsumingOutputHandler` that no longer exists; `bep.rs:1818`, `:1830`);
`bep.rs:1659` uses `{:.6}` where the neighbouring code uses `java_format_fixed`.

## What matched

No difference found in the parts read of: the streamer's state machine (announcement before
post, progress chain links, pending and replaceable events, final progress, abort reasons,
`consumeAsPairs`); the transport order and error text of `createBepTransports`; the progress
buffer state machine and flush conditions; `TargetSummaryPublisher` and the order in which its
events reach the streamer; the critical path (ties keep the earlier component, shared actions
reuse the stored component, cached actions finalize with "action cache hit", overlap
correction in `addDepInfo`) and the tool-log order in `buildComplete`; text-format field
printing, escaping, unknown fields, `Any`, map-entry sorting; the trace writer's header, event
order, slim-profile merging and the final `HashMap` order; the progress-bar text shortening and
the refresh rate limit; `Digraph`, DFS and the successor collection; the DOT writer, factoring,
`--graph:node_limit` arithmetic; the rank formatters' ordering; the XML layout, attribute order
and escapes; cquery graph ordering; aquery proto, streamed and text framing; `--query_file`.

The `PORT: java-hash-order` notes in `digraph.rs` and `graph_output_writer.rs` are sound:
`Target` and `Node` hash by identity in Java, so the unsorted graph, minrank and maxrank orders
differ between Bazel servers and cannot be reproduced; the sorted orders are reproduced.

## Open items that do not block

- S2-1 to S2-4 and S3-1 to S3-3, in that order of weight.
- `pending_put` in the streamer deduplicates by pointer; Java's `SetMultimap` uses `equals`.
  No event class that overrides `equals` and is buffered was found; not checked exhaustively.
- The scope not read is large. The next round should take `metrics_collector.rs`, the
  conversions in `bep.rs`, the rest of `ui_state_tracker.rs`, and `dynamic_message.rs`.
- When the suite ends, two probes would settle what this review could only read: a build with
  `--build_event_json_file --curses=yes` (S3-1, S1-2), and a build with a BEP file and a
  deliberate crash (S1-2).

## Verdict

**STOP** at e8b79c0. Blocking: **S1-1** (`buildMetrics` silently incomplete) and **S1-2** (the
build event stream is not terminated on a crash).
