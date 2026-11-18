//! Port of the analysis-side events of `skyframe/TopLevelStatusEvents.java`: the events that
//! signal the status of top-level targets, used to generate the final results summary.
//!
//! PORT: the execution-side events (`TopLevelTargetPendingExecutionEvent`,
//! `TopLevelTargetBuiltEvent`, `TestAnalyzedEvent`) are in `blaze_sky_exec::events`; the aspect
//! events, `TopLevelTargetReadyForSymlinkPlanting` and `TopLevelEntityAnalysisConcludedEvent`
//! are not ported (no top-level aspects, no overlap of analysis and execution).

use blaze_analysis::configured_target::ConfiguredTarget;
use blaze_events::Postable;

/// An event that marks the successful analysis of a top-level target, including tests. A skipped
/// target is still considered analyzed and a `TopLevelTargetAnalyzedEvent` is expected for it.
// Java: com/google/devtools/build/lib/skyframe/TopLevelStatusEvents.java:45 TopLevelStatusEvents.TopLevelTargetAnalyzedEvent
#[derive(Clone, Debug)]
pub struct TopLevelTargetAnalyzedEvent {
    pub configured_target: ConfiguredTarget,
}

impl TopLevelTargetAnalyzedEvent {
    // Java: com/google/devtools/build/lib/skyframe/TopLevelStatusEvents.java:51 TopLevelStatusEvents.TopLevelTargetAnalyzedEvent.create
    pub fn create(configured_target: ConfiguredTarget) -> Self {
        TopLevelTargetAnalyzedEvent { configured_target }
    }
}

impl Postable for TopLevelTargetAnalyzedEvent {}

/// An event that marks the skipping of a top-level target, including skipped tests.
// Java: com/google/devtools/build/lib/skyframe/TopLevelStatusEvents.java:86 TopLevelStatusEvents.TopLevelTargetSkippedEvent
#[derive(Clone, Debug)]
pub struct TopLevelTargetSkippedEvent {
    pub configured_target: ConfiguredTarget,
}

impl TopLevelTargetSkippedEvent {
    // Java: com/google/devtools/build/lib/skyframe/TopLevelStatusEvents.java:92 TopLevelStatusEvents.TopLevelTargetSkippedEvent.create
    pub fn create(configured_target: ConfiguredTarget) -> Self {
        TopLevelTargetSkippedEvent { configured_target }
    }
}

impl Postable for TopLevelTargetSkippedEvent {}
