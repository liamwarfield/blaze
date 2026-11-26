# Response to the reporting review

Date 2026-11-25. Answers `2026-11-25-reporting.md` (STOP at e8b79c0; blocking S1-1 and S1-2).
Written by the implementer of task `reporting-fix-1` (crates `blaze_bep`, the BEP files of
`blaze_runtime`: `commands/bep.rs`, `bazel_modules.rs`; `blaze_util/src/profiler`).

Tree: the changes are part of root's checkpoint **987d9b3** (HEAD f71e692 has no further change
in these files), plus one later S4 edit in `commands/bep.rs` (the "elapsed time" format,
uncommitted). Binary for every probe below: `~/.cache/blaze/work/reporting-fix-1/bin/blaze-a2`,
built from the working tree at about 11:05 with these files identical to 987d9b3 (other owners'
files were in progress). Oracle: the pinned `bazel-9.3.2-linux-x86_64`. Probes ran in copies
under `~/.cache/blaze/work/reporting-fix-1/ws/`, each binary with its own `--output_user_root`,
`--jobs=4`, the shared repository cache, `--color=no --curses=no` unless stated.

Verification:

- `scripts/cargo-capped test -p blaze_bep`: 135 passed, 0 failed, 26 ignored. `-p blaze_runtime`: 182 passed,
  0 failed, 89 ignored (lib, `profiler_test`, `starlark_options_parsing_test`).
- `scripts/cargo-capped check --workspace --all-targets`: passes.
- `scripts/bep-matrix` (23 scenarios), label `reporting-fix-1-a2` against a run of the e8b79c0
  binary (`~/.cache/blaze/work/root-runs/blaze-e8b79c0`, label `reporting-fix-1-base`):
  stderr now equal in 21 of 23 scenarios (base: 18); the two left are `build_incompatible`
  (same difference in base) and `test_mixed`, which crashes in another owner's in-progress
  coverage code (`coverage_report_action_builder.rs:288`, not in e8b79c0). Outside
  `buildMetrics`, payload differences remain only in those two scenarios.

## Summary

| finding | state | where |
|---|---|---|
| S1-1 `buildMetrics` incomplete | fixed for every field whose producer is in this area; three groups stay absent, their producers are outside it (list below) | `bep.rs` `command_precomplete`, the subscriber, `unconditional_execution_phase_finalizations`; `metrics_collector.rs` |
| S1-2 crash leaves the stream open | fixed: `BazelBuildEventServiceModule::blaze_shutdown_on_crash`; the `Drop` is removed | `bazel_modules.rs`, `bep.rs` |
| S2-1 `cumulativeMetrics` per command | fixed for commands with a BEP (counters live as long as the process, `MetricsModule::counters`); a command without a BEP transport does not count, Java counts it (PORT note) | `bazel_modules.rs`, `bep.rs` |
| S2-2 hand-written build-complete line | fixed: line deleted; `AnnounceBuildEventTransportsEvent` and `BuildEventTransportClosedEvent` reach the ported UI | `bep.rs` |
| S2-3 gzip OS byte | fixed: 255 | `json_trace_file_writer.rs` |
| S2-4 chunks by chars | fixed: UTF-16 units, split surrogates written as `?` (their bytes on the wire) | `synchronized_output_stream.rs` |
| S3-1 cursor control | fixed by S2-2 | |
| S3-2 `--keep_going` failed pattern | fixed | `bep.rs` |
| S3-3 unwritable BEP path | fixed (error, omitted transport, exit 36) | `bep_module.rs`, `bep.rs` |
| S3 stream of cquery/aquery, `--nobuild`, `fetch` with targets, `--build_event_publish_all_actions` | already ported when I started (docs/missing-features.md is stale); confirmed by the matrix scenarios `cquery`, `aquery`, `build_nobuild`, `fetch`, `build_all_actions`: no crash, sequence equal | |
| S4 stale comments, `{:.6}` | fixed | `bep.rs` |
| also found: `AnalysisRootCauseEvent`, `RepositoryFailedEvent` never reached the stream | fixed (new build events) | `analysis_events.rs`, `bep.rs` |

## S1-1

Done: `command_precomplete` passes `WorkerProcessMetricsCollector::instance().collect_metrics()`
(pool metrics and `toProto` of each, then `limitWorkerMetricsToPublish(…, 50)`; the former
`unported!` in `create_build_metrics` is gone), `Profiler.getProfileElapsedTime` and
`getServerProcessCpuTime` (never null in Java, `Duration.toMillis` truncation) and
`NetworkMetricsCollector.collectMetrics()`. The subscriber routes `SomeExecutionStartedEvent`
(`onSomeExecutionStarted`) and starts the build's execution timer there
(`ExecutionProgressReceiverSetup`); `ExecutionPhaseCompleteEvent`
(`handleExecutionPhaseComplete`) is delivered when the `ExecutionTool` exists, from
`build_complete` (PORT note: Java posts it from `unconditionalExecutionPhaseFinalizations`, just
before `stopRequest`; nothing reads the metrics in between). The collector's constructor's
`WorkerProcessMetricsCollector.setClock(env.getClock())` is done.

Field-by-field comparison of the last `buildMetrics` (oracle vs blaze-a2): brotli `build //...`
and re2 `build //:all //python:all` (`test` on re2 crashes in the coverage code named above, so
`build` is compared). Timing and memory fields compared for presence, the rest for value
(`actionData` without its start/end/system/user times):

| field | brotli | re2 |
|---|---|---|
| `actionSummary` (actionsCreated, actionsExecuted, …NotIncludingAspects, actionData, runnerCount, actionCacheStatistics.misses, missDetails) | equal | equal |
| `actionCacheStatistics.sizeInBytes`, `saveTimeInMs` | present both | present both |
| `actionCacheStatistics.loadTimeInMs` | oracle 1, Blaze 0 (absent): timing | same |
| `memoryMetrics.garbageMetrics` | absent: JVM garbage collector, Blaze has none | same |
| `targetMetrics`, `packageMetrics`, `cumulativeMetrics` | equal | equal |
| `timingMetrics` wall, CPU, analysis, execution phase, actions execution start, critical path | present both (were 4 absent) | present both |
| `networkMetrics.systemNetworkStats` (8 fields) | present both (was absent) | present both |
| `buildGraphMetrics` action/lookup/artifact/target counts | equal | equal |
| `buildGraphMetrics.postInvocationSkyframeNodeCount`, `builtValues`, `evaluatedValues` (and `dirtied`/`changed`/`cleaned` on a warm server) | absent | absent |
| `artifactMetrics` (sourceArtifactsRead, outputArtifactsSeen, topLevelArtifacts) | absent | absent |
| `workerPoolMetrics`, `dynamicExecutionMetrics`, `remoteAnalysisCacheStatistics` | empty both | empty both |

Totals per project (leaf fields): brotli 19 equal, 16 present both, 11 absent; re2 19 equal,
17 present both, 10 absent.

Not fixed, with the reason:

1. `artifactMetrics`: `onExecutionComplete` is routed, but nothing produces its input. Java's
   four `FilesMetricConsumer`s (`skyframe/MetadataConsumerForMetrics.java`) are fed by
   `ArtifactFunction.createSourceValue` (:299, :315), `SkyframeActionExecutor.addOutputToMetrics`
   (:1686, from `checkOutputs` with the action-cache-hit flag), and `CompletionFunction` (:207,
   :324, through `ActionInputMapHelper.addToMap`), all in `blaze_sky_exec`, and read by
   `SkyframeExecutor.createExecutionFinishedEvent` (:3669). Editing `blaze_sky_exec` was refused
   to this task. The ported consumer is ready for that crate's owner:
   `~/.cache/blaze/work/reporting-fix-1/proposed/metadata_consumer_for_metrics.rs`. The
   collector is not called with zeros (that would be a guessed value): the field stays absent.
2. `buildGraphMetrics` evaluation stats and node count: `SkyframeGraphStatsEvent`
   (`InMemoryMemoizingEvaluator.postLoggingStats`, :184) needs the evaluator's
   `EvaluationProgressReceiver` (not ported in `blaze_skyframe`, PORTING.md "Skyframe"). The
   collector's `onSkyframeGraphStats` is ported; nothing posts the event.
3. `memoryMetrics`: JVM-only (heap after build, GC per pool). Not portable; the empty message is
   Blaze's answer.

## S1-2

`BazelBuildEventServiceModule::blaze_shutdown_on_crash` calls
`BepCommandSupport::blaze_shutdown_on_crash`: `forceShutdownBuildEventStreamer(INTERNAL or
OUT_OF_MEMORY)` (close the streamer, wait for each transport's close) and the uploader
factory's `shutdown()` (`DeferredUploaderFactory` now forwards it). `after_command`'s "BES was
not properly closed" path uses the same function. The `Drop` impl is deleted; the module doc
comment is corrected.

Probes (Blaze; the oracle does not crash on these inputs):

| case | result |
|---|---|
| crash on an evaluator thread (`action_listener` → unported `ExtraActionFactory`) | 18 events, last `targetConfigured` aborted `INTERNAL`, exactly one `lastMessage` (before: the stream stopped) |
| crash on the command thread after `BuildCompleteEvent` (`--experimental_record_skyframe_metrics` → `unported!` in precomplete) | no `lastMessage`, and this is Java's behaviour: `BuildEventStreamer.close` (:459-470) posts the aborted events and the final progress only when `finalEventsToCome == null`; after `BuildCompleteEvent` it is not null (waiting for `buildMetrics`), so Java also closes the transports without a last message. The crash handler does run (checked with a temporary trace, removed). |
| re2 `test` crash in coverage code (command thread, before `BuildCompleteEvent`) | 171 events, last `buildFinished` aborted `INTERNAL`, one `lastMessage` |

Signals and broken pipe, oracle and Blaze, `build //slow:slow` (a 30 s genrule), signal sent to
the client while the action runs:

| case | oracle | Blaze |
|---|---|---|
| SIGINT | exit 8, 23 events, 1 last message (`buildMetrics`), aborted `INCOMPLETE`, finished `INTERRUPTED` | identical |
| SIGTERM | same as SIGINT | identical |
| stdout and stderr into a closed pipe | exit 141, 21 events, 1 last message, `INTERRUPTED` | exit 141, 29 events, 1 last message, `SUCCESS` |

The broken-pipe difference is the client: Bazel's client cancels the command on SIGPIPE, Blaze's
in-process client exits 141 but lets the command finish (`blaze_client/src/signal_handler.rs`,
`note_broken_pipe`; STATUS lists "the broken-pipe case in daemon mode" for the server work). The
stream itself is terminated in all three.

## S2

- **S2-1.** `MetricsModule::counters()` (`bazel_modules.rs`) holds `numAnalyses` and
  `numBuilds` for the process (one runtime, one `MetricsModule` per server). Residual
  deviation, PORT-noted at the call: Java installs the collector for every command, so a build
  without a BEP transport also increments the counters; here only commands with a BEP do. Not
  measurable until the server default is switched on.
- **S2-2.** `write_build_complete_progress_bar` is deleted. The module's
  `AnnounceBuildEventTransportsEvent` reaches `UiEventSubscriber::build_event_transports_announced`
  in `post_before_command_events`: Java stores the module's posts during `beforeCommand` and
  replays them once the UI handler exists (`BlazeCommandDispatcher.java:581`); Blaze's
  dispatcher creates the UI handler after the modules' `beforeCommand` and does not replay posts,
  so this is the first point where the subscriber exists (PORT note). The build replaces the
  UI's state tracker later (`commands/build.rs`), so the transports are re-announced to the
  build's tracker when the build starts, without the debug line. `closeBepTransports` posts
  `BuildEventTransportClosedEvent` per transport after its close. Checked against the oracle:
  `--curses=no` the tail of stderr is `INFO: ` then the success line, as in Bazel;
  `--experimental_ui_debug_all_events` gives the same "Transports announced:
  JsonFormatFileTransport" / "Transport … closed" lines in the same places; `--curses=yes` the
  last 300 bytes of stderr are byte-identical (`\r` placement included).
- **S2-3.** `operating_system(255)`.
- **S2-4.** `split_fixed_length_utf16` ports `Splitter.fixedLength` on UTF-16 units. A split
  surrogate pair becomes `?` and `?`, which is what Java's proto encoder writes for the two lone
  surrogates (`Utf8.encode` falls back to `String.getBytes(UTF_8)`). Unit test
  `split_counts_utf16_units`.

## S3

- **S3-1** fixed through S2-2 (no `unported!` left on that path).
- **S3-2** The failed patterns are taken from the `PatternExpandingError.failed` posts the
  subscriber already sees (Java adds to `failedPatterns` exactly where it posts them,
  `TargetPatternPhaseFunction.java:303`, :332). Probe `build --keep_going //slow:fast //nopkg:x
  bad::pattern`: the `pattern` event's children (two failed patterns, then the configured
  target) and the two aborted pattern events equal the oracle's. Afterwards Blaze crashes in
  `blaze_sky_top/src/execution_seam.rs:702` (`ViewCreationFailedException` unported, not this
  area); the stream still ends with aborted events and one last message.
- **S3-3** `open_output` now uses `blaze_vfs` (`createDirectoryAndParents`, `getOutputStream`)
  and returns the `IOException`; the module reports `Event.error(e.getMessage())` and
  `moduleEnvironment.exit` with `"Unable to write to '<f>'. Omitting --build_event_<k>_file. "
  + e.getMessage()` and `BES_LOCAL_WRITE_ERROR`. Probe (parent not writable): exit 36 on both;
  both error lines equal except that Bazel's IOException text starts with `[unix_jni.cc:471] `,
  which comes from `blaze_vfs`'s `createDirectory` error text (other owner).
- **S3-4, S3-6, S3-7** unchanged (S3-6's `getSkyframeStats` stays `unported!`; the BES upload is
  Part B of this task, not done, see below).

## Also fixed (found through the suite's `test_fetch_failure`)

`AnalysisRootCauseEvent` (posted by `blaze_sky_analysis`) and `RepositoryFailedEvent` (posted by
`blaze_repo_rules`) never reached the stream: the subscriber did not convert them. Both are now
build events (`AnalysisRootCauseBuildEvent`, `RepositoryFailedBuildEvent` in
`analysis_events.rs`), aborted with `ANALYSIS_FAILURE` / `LOADING_FAILURE` and the message as
description. PORT: the posted root-cause event carries only the configuration checksum; its
configuration event is the one the command made for that checksum, and a checksum the BEP has
not seen is treated as Java's "unavailable configuration" (no configuration event). That matches
Java for the keys whose value is never computed and differs for a computed configuration that
is not yet in the stream (open item 3).

## Open items

1. `artifactMetrics` producers in `blaze_sky_exec` (S1-1 item 1; ported consumer ready).
2. `SkyframeGraphStatsEvent` in `blaze_skyframe` (S1-1 item 2).
3. The configuration of an `AnalysisRootCauseEvent` whose checksum the BEP has not seen.
4. Client cancel on SIGPIPE (`blaze_client`).
5. Build Event Service client (Part B of the task): not started. It needs the
   `google/devtools/build/v1` protos vendored in `blaze_protos`, tonic stubs, and the port of
   `BuildEventServiceUploader`, `BuildEventServiceProtoUtil`, `BuildEventServiceGrpcClient`,
   `BuildEventServiceTransport` and the module's BES half. `--bes_backend` still fails loudly
   before the command runs.
6. Profiler instrumentation sites (Part C): all remaining sites of `docs/profiler-sites.md` are
   in other owners' crates.
