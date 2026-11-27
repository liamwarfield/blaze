//! The phase events the UI (`UiEventHandler`) subscribes to: `LoadingPhaseCompleteEvent`, `ConfigurationPhaseStartedEvent`, `AnalysisPhaseCompleteEvent`,
//! `TestFilteringCompleteEvent` and `ExecutionProgressReceiverAvailableEvent`
//! (`TopLevelStatusEvents.SomeExecutionStartedEvent` is posted by `BuildDriverFunction` and lives
//! in `blaze_sky_exec::events`; `LoadingPhaseStartedEvent`, posted by the query command too, lives
//! beside its receiver in `blaze_sky_loading::package_progress_receiver`).
//!
//! PORT: Java keeps them in `lib/skyframe`, `lib/pkgcache`, `lib/analysis` and
//! `lib/buildtool/buildevent`. The crates that post them (`blaze_sky_top`) sit above this one, and
//! the UI subscriber (`runtime::ui_event_subscriber`) recognises an event by its type, so the
//! events live here, carrying the parts their subscribers read. The progress receivers they
//! carry are the UI's views of them (`runtime::ui_state_tracker`'s `PackageProgress`,
//! `AnalysisProgress`, `ExecutionProgress`).

use std::fmt;
use std::sync::Arc;

use blaze_cmdline::{Label, RepositoryMapping};
use blaze_events::Postable;

use crate::runtime::ui_state_tracker::{AnalysisProgress, ExecutionProgress};

/// This event is fired after the loading phase is complete.
///
/// PORT: `filteredLabels` (the labels removed by test filtering) has no ported subscriber.
// Java: com/google/devtools/build/lib/pkgcache/LoadingPhaseCompleteEvent.java:25 LoadingPhaseCompleteEvent
pub struct LoadingPhaseCompleteEvent {
    labels: Vec<Label>,
    main_repository_mapping: RepositoryMapping,
}

impl LoadingPhaseCompleteEvent {
    // Java: com/google/devtools/build/lib/pkgcache/LoadingPhaseCompleteEvent.java:36 LoadingPhaseCompleteEvent.<init>
    pub fn new(
        labels: Vec<Label>,
        main_repository_mapping: RepositoryMapping,
    ) -> LoadingPhaseCompleteEvent {
        LoadingPhaseCompleteEvent {
            labels,
            main_repository_mapping,
        }
    }

    // Java: com/google/devtools/build/lib/pkgcache/LoadingPhaseCompleteEvent.java:49 LoadingPhaseCompleteEvent.getLabels
    pub fn get_labels(&self) -> &[Label] {
        &self.labels
    }

    // Java: com/google/devtools/build/lib/pkgcache/LoadingPhaseCompleteEvent.java:63 LoadingPhaseCompleteEvent.getMainRepositoryMapping
    pub fn get_main_repository_mapping(&self) -> &RepositoryMapping {
        &self.main_repository_mapping
    }
}

/// This event is fired before the analysis phase starts.
// Java: com/google/devtools/build/lib/skyframe/ConfigurationPhaseStartedEvent.java:19 ConfigurationPhaseStartedEvent
pub struct ConfigurationPhaseStartedEvent {
    analysis_progress: Arc<dyn AnalysisProgress>,
}

impl ConfigurationPhaseStartedEvent {
    // Java: com/google/devtools/build/lib/skyframe/ConfigurationPhaseStartedEvent.java:29 ConfigurationPhaseStartedEvent.<init>
    pub fn new(analysis_progress: Arc<dyn AnalysisProgress>) -> ConfigurationPhaseStartedEvent {
        ConfigurationPhaseStartedEvent { analysis_progress }
    }

    // Java: com/google/devtools/build/lib/skyframe/ConfigurationPhaseStartedEvent.java:33 ConfigurationPhaseStartedEvent.getAnalysisProgressReceiver
    pub fn get_analysis_progress_receiver(&self) -> &Arc<dyn AnalysisProgress> {
        &self.analysis_progress
    }
}

/// This event is fired after the analysis phase is complete.
///
/// PORT: Java's event carries the top-level targets, the evaluated node and action counts, the
/// analysis time and the package manager statistics; their subscribers (metrics, the build
/// event stream's build metrics) are not ported, and the UI reads none of them. The counts the
/// command reads come from `AnalysisResult` (`get_analysis_phase_complete_stats`).
// Java: com/google/devtools/build/lib/analysis/AnalysisPhaseCompleteEvent.java:28 AnalysisPhaseCompleteEvent
pub struct AnalysisPhaseCompleteEvent {
    top_level_target_count: usize,
}

impl AnalysisPhaseCompleteEvent {
    // Java: com/google/devtools/build/lib/analysis/AnalysisPhaseCompleteEvent.java:38 AnalysisPhaseCompleteEvent.<init>
    pub fn new(top_level_target_count: usize) -> AnalysisPhaseCompleteEvent {
        AnalysisPhaseCompleteEvent {
            top_level_target_count,
        }
    }

    /// `getTopLevelTargets().size()`.
    pub fn get_top_level_target_count(&self) -> usize {
        self.top_level_target_count
    }
}

/// Event that is raised when the test filtering phase is complete.
///
/// PORT: Java carries the targets to build, the targets to test and the skipped targets as
/// configured targets, plus the configurations; the UI reads the labels of the test targets
/// (`null` when tests are not run), the other ported subscriber
/// (`AggregatingTestListener.populateTests`) is served by `TestAnalyzedEvent`s.
// Java: com/google/devtools/build/lib/buildtool/buildevent/TestFilteringCompleteEvent.java:34 TestFilteringCompleteEvent
pub struct TestFilteringCompleteEvent {
    test_targets: Option<Vec<Option<Label>>>,
}

impl TestFilteringCompleteEvent {
    // Java: com/google/devtools/build/lib/buildtool/buildevent/TestFilteringCompleteEvent.java:48 TestFilteringCompleteEvent.<init>
    pub fn new(test_targets: Option<Vec<Option<Label>>>) -> TestFilteringCompleteEvent {
        TestFilteringCompleteEvent { test_targets }
    }

    /// The labels of `getTestTargets()`, `None` when tests are not run.
    // Java: com/google/devtools/build/lib/buildtool/buildevent/TestFilteringCompleteEvent.java:77 TestFilteringCompleteEvent.getTestTargets
    pub fn get_test_targets(&self) -> Option<&[Option<Label>]> {
        self.test_targets.as_deref()
    }
}

// The readers of the targets, the skipped tests and the configurations (TargetSummaryPublisher
// and AggregatingTestListener) get them from TestAnalyzedEvent and the analysis result.
// PORT: skipped, read elsewhere (see above): TestFilteringCompleteEvent.getTargets
// PORT: skipped, read elsewhere (see above): TestFilteringCompleteEvent.getSkippedTests
// PORT: skipped, read elsewhere (see above): TestFilteringCompleteEvent.getConfigurationForTarget

/// An event that is fired when an execution progress receiver is available.
// Java: com/google/devtools/build/lib/buildtool/buildevent/ExecutionProgressReceiverAvailableEvent.java:22 ExecutionProgressReceiverAvailableEvent
pub struct ExecutionProgressReceiverAvailableEvent {
    execution_progress_receiver: Arc<dyn ExecutionProgress>,
}

impl ExecutionProgressReceiverAvailableEvent {
    // Java: com/google/devtools/build/lib/buildtool/buildevent/ExecutionProgressReceiverAvailableEvent.java:25 ExecutionProgressReceiverAvailableEvent.<init>
    pub fn new(
        execution_progress_receiver: Arc<dyn ExecutionProgress>,
    ) -> ExecutionProgressReceiverAvailableEvent {
        ExecutionProgressReceiverAvailableEvent {
            execution_progress_receiver,
        }
    }

    // Java: com/google/devtools/build/lib/buildtool/buildevent/ExecutionProgressReceiverAvailableEvent.java:30 ExecutionProgressReceiverAvailableEvent.getExecutionProgressReceiver
    pub fn get_execution_progress_receiver(&self) -> &Arc<dyn ExecutionProgress> {
        &self.execution_progress_receiver
    }
}

macro_rules! postable {
    ($($t:ty => $name:literal),* $(,)?) => {
        $(
            impl fmt::Debug for $t {
                fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                    f.write_str($name)
                }
            }
        )*
    };
}

postable! {
    LoadingPhaseCompleteEvent => "LoadingPhaseCompleteEvent",
    ConfigurationPhaseStartedEvent => "ConfigurationPhaseStartedEvent",
    AnalysisPhaseCompleteEvent => "AnalysisPhaseCompleteEvent",
    TestFilteringCompleteEvent => "TestFilteringCompleteEvent",
    ExecutionProgressReceiverAvailableEvent => "ExecutionProgressReceiverAvailableEvent",
}

impl Postable for LoadingPhaseCompleteEvent {
    // Java: com/google/devtools/build/lib/pkgcache/LoadingPhaseCompleteEvent.java:68 LoadingPhaseCompleteEvent.storeForReplay
    fn store_for_replay(&self) -> bool {
        true
    }
}
impl Postable for ConfigurationPhaseStartedEvent {}
impl Postable for AnalysisPhaseCompleteEvent {}
impl Postable for TestFilteringCompleteEvent {}
impl Postable for ExecutionProgressReceiverAvailableEvent {}
