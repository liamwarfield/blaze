# Review round 3: reporting (R2-1 re-check), 2026-11-26

Reviewer (Fable 5.2), read-only. Commit under review: **e476a59**. Round 2:
`2026-11-26-reporting-round2.md` (STOP at 5f68157 on R2-1). Response: "Round 2" of
`2026-11-25-reporting-response.md`.

## Verdict

**PASS**, bound to e476a59, for the re-checked finding R2-1. No blocking finding.

One new S1 (N1) was seen in a probe made for another re-check. It is outside the change under
re-check, older than it, and shown only with a binary that is not of this commit; I do not
block on it, and it needs an owner. If root ranks an unassigned S1 as blocking, N1 is the
finding to name.

## Scope

- Read: the `AnalysisRootCauseEvent` type (`blaze_analysis/src/rule/rule_error_consumer.rs:
  100-165`), its five post sites, `AnalysisRootCauseBuildEvent`
  (`blaze_bep/src/events/analysis_events.rs:395-485`), the subscriber branch
  (`blaze_runtime/src/commands/bep.rs:413-438`), `docs/missing-features.md:160-174`. Java:
  `AnalysisRootCauseEvent.java`, `EventHandlingErrorReporter.java:40-48`,
  `DependencyProducer.java:637-640`, `MissingEdgeError.java:87-91`,
  `DependencyResolver.java:870-877`, `ConfiguredTargetFunction.java:572-584`.
- Not read: the rest of the diff (artifact metrics, extra actions in the stream), the new tests.
- No probe of R2-1 by me. The response's two probes (a failing rule in the exec configuration
  and behind a Starlark transition) are the fixer's, on `blaze-a2`.

## R2-1 (S1, was blocking): fixed

- The event carries Java's tri-state: `configuration: Option<Option<Arc<BuildConfigurationValue>>>`
  plus `configuration_id`. It is built only by the two factories; no other construction exists
  in `crates/`.
- Post sites, each against its Java:

| Site | Rust | Form | Java |
| --- | --- | --- | --- |
| `EventHandlingErrorReporter.reportError` | `rule_error_consumer.rs:199` | value (`Some(self.configuration)`) | `:46` |
| `DependencyResolver.handleDependencyRootCauseError` | `dependency_resolver.rs:1031` | value or null configuration (`configuration.cloned()`) | `:877` |
| `DependencyProducer.ExecGroupErrorEmitter.postEvent` | `dependency_map_producer.rs:643` | value | `:640` |
| `MissingEdgeError` (`NoSuchPackageException`) | `producers/errors.rs:951` | value or null configuration | `:91` |
| `ConfiguredTargetFunction` | `configured_target_function.rs:338` | unavailable, id of the key (`None` for a null key) | `:580` |

- Subscriber: a value gives `with_configuration_value` with the configuration's
  `toBuildEvent()`; the null configuration gives `NullConfiguration` (`buildEvent(null)`);
  unavailable gives no configuration event and the id of the key. `get_configurations` is
  `AnalysisRootCauseEvent.java:113-118`. The streamer then announces a configuration it has not
  shown yet, as before.
- The checksum map `configuration_events` is gone from `Shared`; `configuration_event` no longer
  records anything.

## R2-2 (S2): open

Not changed; being worked on. Listed in `docs/missing-features.md:172`.

## R2-3 (S2): done

`docs/missing-features.md:167-174` names the two absent groups (the Skyframe graph statistics
of `buildGraphMetrics`, and `memoryMetrics`), the `cumulativeMetrics` gap, and that the referee
does not compare `buildMetrics` sub-messages. `artifactMetrics` is produced now
(`bep.rs:2007-2027`); I did not review its producers.

## New finding

### S1

**N1. With `--nobuild`, `buildMetrics` lacks the workspace status action.** Not blocking (see
the verdict).

- Probe: `build --nobuild --build_event_json_file=... //pkg:all` in
  `~/.cache/blaze/work/review-commands-2/ws` (two genrules, one filegroup), oracle against
  `blaze-a2`; files `nob2.{o,b}.json` in `~/.cache/blaze/work/review-commands-2/out/`.

| Field | Oracle | Blaze |
| --- | --- | --- |
| `targetMetrics.targetsConfigured` | 13 | 12 |
| `targetMetrics.targetsConfiguredNotIncludingAspects` | 12 | 12 |
| `actionSummary.actionsCreated` | 3 | 2 |
| `actionSummary.actionData` | `BazelWorkspaceStatusAction` (1 created), `Genrule` (2) | `Genrule` (2) |

  With an empty target set (`nob.{o,b}.json`): 7 against 6, and `actionsCreated` 1 against
  absent. The UI line (`12 targets configured`) is equal on both sides.
- Reading: without merged analysis and execution Java evaluates the workspace status value
  during the analysis (`BuildView.update` / `createResult`), before
  `AnalysisPhaseCompleteEvent` reads the counters; Blaze leaves it to the execution phase
  (`build_view.rs:4`, PORT note), which `--nobuild` never runs. On the merged path the response
  reports these fields equal on brotli.
- Limits: `blaze-a2` is a binary of an uncommitted tree. The referee does not compare these
  sub-messages, so no score shows it.
- Expected: evaluate (or count) the workspace status value in the non-merged analysis as Java
  does, then compare the two files above again.

## Open items, not blocking

- N1 needs an owner.
- R2-2.
- The Skyframe graph statistics (open item 1 of the response): `builtValues`,
  `evaluatedValues` and `postInvocationSkyframeNodeCount` are absent in every probe of mine too.
- `networkMetrics` is present in the oracle's `buildMetrics` and absent in Blaze's in all four
  of my probes (keys only compared; round 2 lists network metrics as filled). Not examined.
