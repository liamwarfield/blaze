# Review, round 2: reporting (build events, metrics, UI transport events), 2026-11-26

Reviewer: Fable 5.2, read-only. Commit under review: **5f68157**. Round 1:
`2026-11-25-reporting.md` (STOP at e8b79c0: S1-1, S1-2 blocking; four S2). Response:
`2026-11-25-reporting-response.md`.

**Verdict: STOP**, bound to 5f68157. Blocking: **R2-1** (S1). Round 1's S1-1 and S1-2 and its
four S2 are fixed or reduced to honest gaps; the STOP is for one deviation in code added by the
fix, which the response itself states.

No probe was run: `root-suite-5f68157` was `active` for the whole review. Everything below is
from the code at 5f68157 and the pinned Java; the probe results quoted are the implementer's
(binary `blaze-a2` of a working tree, not `blaze-5f68157`).

## Scope

Read: `git diff e8b79c0 5f68157` of `blaze_runtime/src/commands/bep.rs`, `bazel_modules.rs`,
`blaze_bep/src/bep_module.rs`, `events/analysis_events.rs`, `metrics/metrics_collector.rs`,
`runtime/synchronized_output_stream.rs`; at 5f68157: `build_event_streamer.rs:430-465`,
`metrics_collector.rs:700-760`, `bazel_modules.rs:995-1050`, `runtime.rs:236-250`,
`ui_event_subscriber.rs:300-334`, the five post sites of `AnalysisRootCauseEvent` in
`blaze_analysis` and `blaze_sky_analysis`. Java: `BuildEventStreamer.java:440-480, 508-520,
608-620`, `BuildEventServiceModule.java:470-512`, `AnalysisRootCauseEvent.java`,
`RepositoryFailedEvent.java`, `UiEventHandler.java:969-989`, `ConfiguredTargetFunction.java:570-590`.

Not read: `blaze_util/src/profiler` beyond the one-line gzip change, the worker metrics
collector and `toProto`, `NetworkMetricsCollector`, the new tests, the coverage and execution-log
additions to `build_complete` (other tasks), `scripts/bep-matrix`. Not re-read: the round-1
"what matched" list.

## Round-1 findings

**S1-1 (`buildMetrics`): fixed where the producer exists; the rest is absent honestly.**
- Filled now: worker metrics and pool metrics (`bep.rs` `command_precomplete`,
  `metrics_collector.rs:838-842`, the `unported!` is gone), profile elapsed time and server CPU
  time (always `Some`, as `Profiler.java:420, 433`), network metrics,
  `onSomeExecutionStarted` (routed in the subscriber), `handleExecutionPhaseComplete` (from
  `build_complete`, before the build-complete event; nothing reads the metrics in between).
- Still absent: `artifactMetrics` (`on_execution_complete` has no caller; its inputs are not
  produced in `blaze_sky_exec`), the Skyframe graph stats of `buildGraphMetrics`
  (`on_skyframe_graph_stats` has no caller), `memoryMetrics` (JVM heap and GC). I checked that
  none of them is set to zero or to a guessed number: the collector's setters are simply not
  called, so the sub-fields are unset, and the PORT notes at
  `bep.rs` `unconditional_execution_phase_finalizations` and `command_precomplete` say so. By
  round 1's own condition ("anything left unported is named") this is S3, not S1. See S3-a for
  what is still owed.
- `actionCacheStatistics.loadTimeInMs` (oracle 1, Blaze 0): not examined; the response calls it
  timing. Open item.

**S1-2 (crash leaves the stream open): fixed.** `BazelBuildEventServiceModule::
blaze_shutdown_on_crash` (`bazel_modules.rs:1042-1050`) closes the module's `current` support
with `INTERNAL` or `OUT_OF_MEMORY` and shuts the uploader factory down, as
`BuildEventServiceModule.java:500-509`; `shut_down_modules_on_crash` (`runtime.rs:246`) reaches
it from `BugReport.handleCrash` on the command thread and, through the uncaught handler, on any
other thread. `current` is taken in `after_command`, so a later command's crash cannot close an
old streamer. `close_with` (`build_event_streamer.rs:440-465`) is `BuildEventStreamer.close`
line for line: one final progress with the last message iff `finalEventsToCome` is null.
The response's claim about a crash after `BuildCompleteEvent` is right: at
`BuildEventStreamer.java:459-470` `clearEventsAndPostFinalProgress` runs only when
`finalEventsToCome == null`; after the build-complete event it is non-null (waiting for
`buildMetrics`), so Bazel too closes the transports without a last message. "Exactly one last
message" on the other paths (crash before build-complete, SIGINT, SIGTERM, broken pipe) follows
from the streamer's `closed` flag and is shown by the response's probes; I did not repeat them.

**S2-1 (cumulative metrics): reduced, still S2.** See R2-2.

**S2-2 (UI line): fixed.** The hand-written line is deleted. `build_event_transports_announced`
and `build_event_transport_closed` (`ui_event_subscriber.rs:306-334`) match
`UiEventHandler.java:969-989`, including `checkActivities()`; the closed event is sent for each
transport whether its close failed or not, as Java's listener. Two PORT deviations, both stated
in the code and both limited to `--experimental_ui_debug_all_events`: no "Transports announced:"
line for a command without a transport, and a silent re-announcement to the build's replaced
state tracker. Accepted as S4 weight.

**S2-3 (gzip byte): fixed** (`operating_system(255)`).

**S2-4 (UTF-16 split): fixed.** `split_fixed_length_utf16` counts UTF-16 units; a split pair
becomes `?` + `?`. That equals what Java writes for both lone surrogates in the proto, text and
JSON transports (all encode through `String.getBytes(UTF_8)` or an equivalent replacing
encoder). The unit test's cases are right by hand.

**S3-1, S3-2, S3-3:** no `unported!` left on those paths in the diff. `open_output` now goes
through `blaze_vfs`; the error text and `BES_LOCAL_WRITE_ERROR` follow
`BuildEventServiceModule.java:168-185, 934`. The message prefix `[unix_jni.cc:471] ` that Bazel
prints and Blaze does not is another owner's error text (open item).

## New findings

### S1

**R2-1. An analysis root cause in a configuration the stream has not shown yet loses its
`configuration` event. Blocking.**

- Rust: `crates/blaze_runtime/src/commands/bep.rs`, the `AnalysisRootCauseEvent` branch of
  `BepSubscriber::post` (diff hunk at old line 369; `Shared::configuration_events`). The posted
  event (`crates/blaze_analysis/src/rule/rule_error_consumer.rs:104-109`) carries only a
  checksum. The subscriber looks the checksum up among the configuration events this command
  already made; if it is not there it builds `with_unavailable_configuration`, whose
  `get_configurations` is empty.
- Java: `analysis/AnalysisRootCauseEvent.java:60-66, 113-118` and
  `runtime/BuildEventStreamer.java:512-520, 614-618`. Four of the five post sites
  (`EventHandlingErrorReporter.java:46`, `DependencyProducer.java:640`,
  `MissingEdgeError.java:91`, `DependencyResolver.java:877`) pass the
  `BuildConfigurationValue`; the streamer then posts that configuration's event before the
  root cause if it was not posted yet. Only `ConfiguredTargetFunction.java:580` is the
  unavailable form.
- The map is filled only from top-level sources (`TargetConfiguredEvent`,
  `ConfigurationForBuildEvent`, completed targets, aspects), so every dependency in a
  transitioned or exec configuration is "unseen".
- Input: `genrule(name = "g", tools = [":bad"], ...)` where `:bad` is a rule whose
  implementation calls `fail("x")`; `build //:g --build_event_json_file=bep.json`. Bazel's
  stream has `{"id":{"configuration":{"id":"<exec checksum>"}},"configuration":{...}}` followed
  by the aborted `configuredLabel` event of `//:bad` in that configuration. Blaze's has the
  aborted event only. Not probed (suite active); the reading of both sides is unambiguous.
- The reverse error also exists: the `ConfiguredTargetFunction` site
  (`configured_target_function.rs:333-345`) posts the same struct, so an event Java marks
  unavailable is given a configuration when the checksum has been seen, and a null
  configuration key posts `NullConfiguration` where Java posts nothing. The effect is at most
  one early or extra `configuration` event.
- Why it blocks: it is wrong output on valid input in code this fix added, the subscriber is
  guessing which of Java's two constructors was meant, and the information exists at four of
  the five post sites (`ErrorReporter` holds the `Arc<BuildConfigurationValue>`).
- Fix expected: `AnalysisRootCauseEvent` carries Java's tri-state (the configuration value,
  the null configuration, or unavailable with its id) from each post site; the subscriber maps
  it one to one and the checksum map goes away. The files are in `blaze_analysis` and
  `blaze_sky_analysis`; root has to name them in the fixer's task. A test or a recorded
  oracle run for the genrule case above.

### S2

**R2-2. `cumulativeMetrics` counts only commands that have a BEP transport.**
`bep.rs` `module_before_command` installs the `MetricsCollector` only when a streamer exists
(PORT note there); Java's `MetricsModule.beforeCommand` (`MetricsModule.java:78`) installs it
for every command. Input, one server: `build //x` then `build //x
--build_event_json_file=f`: Bazel reports `numAnalyses: 2, numBuilds: 2`, Blaze 1 and 1. Masked
while the server default is off. The counters themselves now live for the process
(`bazel_modules.rs:1105-1122`), which is right for one runtime per process. Fix before S5:
count in every build command.

**R2-3. The absent `buildMetrics` fields are still invisible to the referee and to STATUS.**
`scripts/oracle/surfaces.py:34-37` still drops `artifactMetrics` and the other sub-messages, and
neither `docs/STATUS.md` nor `docs/missing-features.md` names `artifactMetrics` or the Skyframe
graph stats as missing; only the response and `docs/decisions.md:365-369` do. A reader of the
scores sees `buildMetrics` as identical. No code change asked of this area: STATUS must list
the three absent groups, and the referee change is already ruled for the next audited batch.

### S3

- **S3-a.** `artifactMetrics` (producers in `blaze_sky_exec`; ported consumer waiting in
  `~/.cache/blaze/work/reporting-fix-1/proposed/`), `SkyframeGraphStatsEvent`
  (`blaze_skyframe`), `memoryMetrics` (not portable). On every build that executes, the
  deterministic `artifactMetrics` counts are missing; this is the largest remaining gap of the
  area and should be the next task of the `blaze_sky_exec` owner.
- **S3-b.** `--experimental_record_skyframe_metrics` (`getSkyframeStats`), no-workspace output
  paths, the Build Event Service client, the profiler sites: `unported!` or listed, unchanged.

### S4

`bep.rs` `force_shutdown_build_event_streamer`: an empty `if result.is_err() {}` holds only a
comment; `bep_module.rs:391-393` carries two `// Java:` markers for the same method.

## Open items, not blocking

- `actionCacheStatistics.loadTimeInMs` 0 against the oracle's 1: confirm it is measured, not a
  constant.
- Broken pipe: Blaze's in-process client lets the command finish (29 events, `SUCCESS`) where
  Bazel's cancels it (21 events, `INTERRUPTED`); the stream is terminated in both. Client work.
- The probes of the response (crash, signals, `--curses=yes`) should be repeated once with
  `blaze-5f68157` when the suite ends.
- `pending_put` deduplication by pointer (round 1) was not revisited.

## To lift the STOP

R2-1 fixed as described, with the genrule case shown against the oracle. Then this reviewer
role re-checks that finding only.

## Verdict

**STOP** at 5f68157. Blocking: **R2-1**.
