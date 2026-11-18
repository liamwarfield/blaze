//! The build-event side of `lib/runtime/TestSummary.java` ([`TestSummaryEvent`]) and
//! `lib/runtime/TargetSummaryEvent.java`.
//!
//! PORT: the full `TestSummary` (with its builder and the terminal output) lives in crate
//! `blaze_buildtool`, which depends on this crate. [`TestSummaryEvent`] holds exactly the facts
//! `TestSummary`'s `BuildEvent` methods read; the runtime fills it from its `TestSummary` and
//! posts it where Java posts the `TestSummary` (`AggregatingTestListener`, once per test target,
//! after the target's `TargetCompleteEvent`).

use blaze_cmdline::label::Label;
use blaze_exec::analysis::test::test_attempt::bep_status;
use blaze_protos::blaze::BlazeTestStatus;
use blaze_protos::build_event_stream::build_event::Payload;
use blaze_protos::build_event_stream::{self as bes, BuildEventId};
use blaze_util::check_argument;
use blaze_vfs::Path;

use crate::buildeventstream::build_event::{
    BuildEvent, BuildEventClass, Interrupted, LocalFile, LocalFileType,
};
use crate::buildeventstream::build_event_context::BuildEventContext;
use crate::buildeventstream::build_event_id_util;
use crate::buildeventstream::generic_build_event::proto_chaining_with;
use crate::util::{duration_from_millis, timestamp_from_millis};

/// Test summary entry, as a build event: the data of one `TestSummary` that its `BuildEvent`
/// methods read.
// Java: com/google/devtools/build/lib/runtime/TestSummary.java:64 TestSummary
#[derive(Clone, Debug)]
pub struct TestSummaryEvent {
    /// `AliasProvider.getDependencyLabel(target)` (`TestSummary.getLabel()`).
    pub label: Label,
    /// `target.getConfigurationChecksum()`.
    pub configuration_checksum: String,
    /// `getStatus()`.
    pub status: BlazeTestStatus,
    /// `getNumCached()`.
    pub num_cached: i32,
    /// `totalRuns()`: `getTestTimes().size()`.
    pub total_runs: i32,
    /// `getNumAttempts()`: the maximum of the per-shard attempt counts.
    pub num_attempts: i32,
    /// `target.getProvider(TestProvider.class).getTestParams().getRuns()`.
    pub runs: i32,
    /// `target.getProvider(TestProvider.class).getTestParams().getShards()`.
    pub shards: i32,
    /// `getFirstStartTimeMillis()`.
    pub first_start_time_millis: i64,
    /// `getLastStopTimeMillis()`.
    pub last_stop_time_millis: i64,
    /// `getTotalRunDurationMillis()`.
    pub total_run_duration_millis: i64,
    /// `getPassedLogs()`.
    pub passed_logs: Vec<Path>,
    /// `getFailedLogs()`.
    pub failed_logs: Vec<Path>,
}

impl TestSummaryEvent {
    // Java: com/google/devtools/build/lib/runtime/TestSummary.java:600 TestSummary.totalRuns
    pub fn total_runs(&self) -> i32 {
        self.total_runs
    }

    /// `BuildEventIdUtil.configurationId(target.getConfigurationChecksum())`.
    fn configuration_id(&self) -> BuildEventId {
        build_event_id_util::configuration_id(&self.configuration_checksum)
    }
}

impl BuildEvent for TestSummaryEvent {
    // Java: com/google/devtools/build/lib/runtime/TestSummary.java:626 TestSummary.getEventId
    fn get_event_id(&self) -> BuildEventId {
        build_event_id_util::test_summary(&self.label.to_string(), &self.configuration_id())
    }

    // Java: com/google/devtools/build/lib/runtime/TestSummary.java:633 TestSummary.getChildrenEvents
    fn get_children_events(&self) -> Vec<BuildEventId> {
        Vec::new()
    }

    // Java: com/google/devtools/build/lib/runtime/TestSummary.java:638 TestSummary.postedAfter
    fn posted_after(&self) -> Option<Vec<BuildEventId>> {
        Some(vec![build_event_id_util::target_completed(
            &self.label,
            &self.configuration_id(),
        )])
    }

    // Java: com/google/devtools/build/lib/runtime/TestSummary.java:646 TestSummary.referencedLocalFiles
    fn referenced_local_files(&self) -> Vec<LocalFile> {
        let mut local_files = Vec::new();
        // TODO(b/199940216): Can we populate metadata for these files?
        for path in &self.failed_logs {
            local_files.push(LocalFile::new(
                path.clone(),
                LocalFileType::FailedTestOutput,
                /* artifact_metadata= */ None,
            ));
        }
        for path in &self.passed_logs {
            local_files.push(LocalFile::new(
                path.clone(),
                LocalFileType::SuccessfulTestOutput,
                /* artifact_metadata= */ None,
            ));
        }
        local_files
    }

    // Java: com/google/devtools/build/lib/runtime/TestSummary.java:661 TestSummary.asStreamProto
    #[allow(deprecated)]
    fn as_stream_proto(
        &self,
        converters: &dyn BuildEventContext,
    ) -> Result<bes::BuildEvent, Interrupted> {
        let path_converter = converters.path_converter();
        let mut summary_builder = bes::TestSummary {
            overall_status: bep_status(self.status) as i32,
            total_num_cached: self.num_cached,
            total_run_count: self.total_runs(),
            attempt_count: self.num_attempts,
            run_count: self.runs,
            shard_count: self.shards,
            first_start_time: Some(timestamp_from_millis(self.first_start_time_millis)),
            first_start_time_millis: self.first_start_time_millis,
            last_stop_time: Some(timestamp_from_millis(self.last_stop_time_millis)),
            last_stop_time_millis: self.last_stop_time_millis,
            total_run_duration: Some(duration_from_millis(self.total_run_duration_millis)),
            total_run_duration_millis: self.total_run_duration_millis,
            ..Default::default()
        };
        for path in &self.failed_logs {
            if let Some(uri) = path_converter.apply(path) {
                summary_builder.failed.push(uri_file(uri));
            }
        }
        for path in &self.passed_logs {
            if let Some(uri) = path_converter.apply(path) {
                summary_builder.passed.push(uri_file(uri));
            }
        }
        Ok(proto_chaining_with(
            self,
            Payload::TestSummary(summary_builder),
        ))
    }

    fn event_class(&self) -> BuildEventClass {
        BuildEventClass::TestSummary {
            total_runs: self.total_runs(),
        }
    }
}

impl blaze_events::Postable for TestSummaryEvent {}

/// `BuildEventStreamProtos.File.newBuilder().setUri(uri).build()`.
fn uri_file(uri: String) -> bes::File {
    bes::File {
        file: Some(bes::file::File::Uri(uri)),
        ..Default::default()
    }
}

/// Event summarizing the building and testing (if applicable) of a given configured target.
// Java: com/google/devtools/build/lib/runtime/TargetSummaryEvent.java:38 TargetSummaryEvent
#[derive(Clone, Debug, PartialEq)]
pub struct TargetSummaryEvent {
    id: BuildEventId,
    overall_build_success: bool,
    overall_test_status: Option<BlazeTestStatus>,
    posted_after: Vec<BuildEventId>,
}

impl TargetSummaryEvent {
    /// `label` is `target.getOriginalLabel()`; `configuration_checksum` is the checksum of
    /// `target.getLookupKey().getConfigurationKey()` (`None` for a null configuration key).
    // Java: com/google/devtools/build/lib/runtime/TargetSummaryEvent.java:40 TargetSummaryEvent.create
    pub fn create(
        label: &Label,
        configuration_checksum: Option<&str>,
        overall_build_success: bool,
        expect_test_summary: bool,
        overall_test_status: Option<BlazeTestStatus>,
    ) -> TargetSummaryEvent {
        let config_id = build_event_id_util::configuration_id_for_key(configuration_checksum);
        let label_string = label.to_string();
        let mut post_after = vec![build_event_id_util::target_completed(label, &config_id)];
        if expect_test_summary {
            // Always post after test summary, even if we get here without having seen it yet
            post_after.push(build_event_id_util::test_summary(&label_string, &config_id));
        }
        TargetSummaryEvent::new(
            build_event_id_util::target_summary(&label_string, &config_id),
            overall_build_success,
            if overall_build_success && expect_test_summary {
                overall_test_status
            } else {
                None
            },
            post_after,
        )
    }

    // Java: com/google/devtools/build/lib/runtime/TargetSummaryEvent.java:65 TargetSummaryEvent.<init>
    fn new(
        id: BuildEventId,
        overall_build_success: bool,
        overall_test_status: Option<BlazeTestStatus>,
        posted_after: Vec<BuildEventId>,
    ) -> TargetSummaryEvent {
        check_argument!(
            matches!(id.id, Some(bes::build_event_id::Id::TargetSummary(_))),
            "Unexpected event id: {:?}",
            id
        );
        TargetSummaryEvent {
            id,
            overall_build_success,
            overall_test_status,
            posted_after,
        }
    }

    // Java: com/google/devtools/build/lib/runtime/TargetSummaryEvent.java:78 TargetSummaryEvent.isOverallBuildSuccess
    pub fn is_overall_build_success(&self) -> bool {
        self.overall_build_success
    }

    // Java: com/google/devtools/build/lib/runtime/TargetSummaryEvent.java:84 TargetSummaryEvent.getOverallTestStatus
    pub fn get_overall_test_status(&self) -> Option<BlazeTestStatus> {
        self.overall_test_status
    }
}

impl BuildEvent for TargetSummaryEvent {
    // Java: com/google/devtools/build/lib/runtime/TargetSummaryEvent.java:105 TargetSummaryEvent.getEventId
    fn get_event_id(&self) -> BuildEventId {
        self.id.clone()
    }

    // Java: com/google/devtools/build/lib/runtime/TargetSummaryEvent.java:110 TargetSummaryEvent.getChildrenEvents
    fn get_children_events(&self) -> Vec<BuildEventId> {
        Vec::new()
    }

    // Java: com/google/devtools/build/lib/runtime/TargetSummaryEvent.java:89 TargetSummaryEvent.postedAfter
    fn posted_after(&self) -> Option<Vec<BuildEventId>> {
        Some(self.posted_after.clone())
    }

    // Java: com/google/devtools/build/lib/runtime/TargetSummaryEvent.java:94 TargetSummaryEvent.asStreamProto
    fn as_stream_proto(
        &self,
        _context: &dyn BuildEventContext,
    ) -> Result<bes::BuildEvent, Interrupted> {
        let mut summary_builder = bes::TargetSummary {
            overall_build_success: self.overall_build_success,
            ..Default::default()
        };
        if self.overall_build_success
            && let Some(overall_test_status) = self.overall_test_status
        {
            summary_builder.overall_test_status = bep_status(overall_test_status) as i32;
        }
        Ok(proto_chaining_with(
            self,
            Payload::TargetSummary(summary_builder),
        ))
    }
}

impl blaze_events::Postable for TargetSummaryEvent {}
