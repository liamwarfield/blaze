//! Ports of build events from `lib/analysis`: `TargetConfiguredEvent`, `BuildInfoEvent`,
//! `NoBuildEvent`; and `TestAttempt` (ported in `blaze_exec`) as a build event.

use std::sync::Arc;

use blaze_cmdline::label::Label;
use blaze_exec::analysis::test::test_attempt::{TestAttempt, TestAttemptLocalFileType};
use blaze_protos::build_event_stream::build_event::Payload;
use blaze_protos::build_event_stream::{self as bes, BuildEventId, TestSize};
use indexmap::IndexMap;

use crate::buildeventstream::build_event::{
    BuildEvent, BuildEventClass, Interrupted, LocalFile, LocalFileType,
};
use crate::buildeventstream::build_event_context::BuildEventContext;
use crate::buildeventstream::build_event_id_util;
use crate::buildeventstream::generic_build_event::{
    NullConfiguration, ProgressEvent, proto_chaining_with,
};
use crate::util::timestamp_from_millis;

/// `packages.TestSize`.
// Java: com/google/devtools/build/lib/packages/TestSize.java:28 TestSize
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PackagesTestSize {
    Small,
    Medium,
    Large,
    Enormous,
}

/// What `TargetConfiguredEvent` reads from its `Target`.
#[derive(Clone, Debug)]
pub struct ConfiguredTargetInfo {
    /// `target.getLabel()`.
    pub label: Label,
    /// `target.getTargetKind()`, e.g. `cc_library rule`.
    pub target_kind: String,
    /// `RawAttributeMapper.of(rule).getMergedValues("tags", STRING_LIST)` if the target is a rule
    /// with a `tags` attribute, `None` otherwise.
    pub tags: Option<Vec<String>>,
    /// For a test rule (`TargetUtils.isTestRule`): `TestSize.getTestSize(rule)` (`Some(None)`
    /// when the size is unknown). `None` for other targets.
    pub test_size: Option<Option<PackagesTestSize>>,
}

/// Event reporting about the configurations associated with a given target.
// Java: com/google/devtools/build/lib/analysis/TargetConfiguredEvent.java:35 TargetConfiguredEvent
#[derive(Debug)]
pub struct TargetConfiguredEvent {
    target: ConfiguredTargetInfo,
    /// `BuildConfigurationValue.buildEvent(configuration)` (`None`: the null configuration).
    configuration_event: Option<Arc<dyn BuildEvent>>,
    /// `BuildConfigurationValue.configurationId(configuration)` checksum.
    configuration_checksum: Option<String>,
}

impl TargetConfiguredEvent {
    // Java: com/google/devtools/build/lib/analysis/TargetConfiguredEvent.java:40 TargetConfiguredEvent.<init>
    pub fn new(
        target: ConfiguredTargetInfo,
        configuration_event: Option<Arc<dyn BuildEvent>>,
        configuration_checksum: Option<String>,
    ) -> Self {
        TargetConfiguredEvent {
            target,
            configuration_event,
            configuration_checksum,
        }
    }

    // Java: com/google/devtools/build/lib/analysis/TargetConfiguredEvent.java:62 TargetConfiguredEvent.bepTestSize
    fn bep_test_size(size: Option<PackagesTestSize>) -> TestSize {
        match size {
            Some(PackagesTestSize::Small) => TestSize::Small,
            Some(PackagesTestSize::Medium) => TestSize::Medium,
            Some(PackagesTestSize::Large) => TestSize::Large,
            Some(PackagesTestSize::Enormous) => TestSize::Enormous,
            // Logged in Java: "Target %s has a test size of: %s".
            None => TestSize::Unknown,
        }
    }
}

impl BuildEvent for TargetConfiguredEvent {
    // Java: com/google/devtools/build/lib/analysis/TargetConfiguredEvent.java:51 TargetConfiguredEvent.getEventId
    fn get_event_id(&self) -> BuildEventId {
        build_event_id_util::target_configured(&self.target.label)
    }

    // Java: com/google/devtools/build/lib/analysis/TargetConfiguredEvent.java:56 TargetConfiguredEvent.getChildrenEvents
    fn get_children_events(&self) -> Vec<BuildEventId> {
        vec![build_event_id_util::target_completed(
            &self.target.label,
            &build_event_id_util::configuration_id_for_key(self.configuration_checksum.as_deref()),
        )]
    }

    // Java: com/google/devtools/build/lib/analysis/TargetConfiguredEvent.java:76 TargetConfiguredEvent.asStreamProto
    fn as_stream_proto(&self, _: &dyn BuildEventContext) -> Result<bes::BuildEvent, Interrupted> {
        let mut builder = bes::TargetConfigured {
            target_kind: self.target.target_kind.clone(),
            ..Default::default()
        };
        if let Some(tags) = &self.target.tags {
            builder.tag.extend(tags.iter().cloned());
        }
        if let Some(size) = self.target.test_size {
            builder.test_size = Self::bep_test_size(size) as i32;
        }
        Ok(proto_chaining_with(self, Payload::Configured(builder)))
    }

    // Java: com/google/devtools/build/lib/analysis/TargetConfiguredEvent.java:46 TargetConfiguredEvent.getConfigurations
    fn get_configurations(&self) -> Option<Vec<Arc<dyn BuildEvent>>> {
        Some(vec![
            self.configuration_event
                .clone()
                .unwrap_or_else(NullConfiguration::instance),
        ])
    }
}

impl blaze_events::Postable for TargetConfiguredEvent {}

/// This event is fired once build info data is available.
// Java: com/google/devtools/build/lib/analysis/BuildInfoEvent.java:30 BuildInfoEvent
#[derive(Clone, Debug)]
pub struct BuildInfoEvent {
    build_info_map: IndexMap<String, String>,
}

impl BuildInfoEvent {
    /// Construct the event from a map.
    // Java: com/google/devtools/build/lib/analysis/BuildInfoEvent.java:37 BuildInfoEvent.<init>
    pub fn new(build_info: IndexMap<String, String>) -> Self {
        BuildInfoEvent {
            build_info_map: build_info,
        }
    }

    // Java: com/google/devtools/build/lib/analysis/BuildInfoEvent.java:44 BuildInfoEvent.getBuildInfoMap
    pub fn get_build_info_map(&self) -> &IndexMap<String, String> {
        &self.build_info_map
    }
}

impl BuildEvent for BuildInfoEvent {
    // Java: com/google/devtools/build/lib/analysis/BuildInfoEvent.java:49 BuildInfoEvent.getEventId
    fn get_event_id(&self) -> BuildEventId {
        build_event_id_util::workspace_status_id()
    }

    // Java: com/google/devtools/build/lib/analysis/BuildInfoEvent.java:54 BuildInfoEvent.getChildrenEvents
    fn get_children_events(&self) -> Vec<BuildEventId> {
        Vec::new()
    }

    // Java: com/google/devtools/build/lib/analysis/BuildInfoEvent.java:64 BuildInfoEvent.asStreamProto
    fn as_stream_proto(&self, _: &dyn BuildEventContext) -> Result<bes::BuildEvent, Interrupted> {
        let mut status = bes::WorkspaceStatus::default();
        for (key, value) in &self.build_info_map {
            status.item.push(bes::workspace_status::Item {
                key: key.clone(),
                value: value.clone(),
            });
        }
        Ok(proto_chaining_with(self, Payload::WorkspaceStatus(status)))
    }

    // Java: com/google/devtools/build/lib/analysis/BuildInfoEvent.java:59 BuildInfoEvent.postedAfter
    fn posted_after(&self) -> Option<Vec<BuildEventId>> {
        Some(vec![build_event_id_util::build_started_id()])
    }

    fn event_class(&self) -> BuildEventClass {
        BuildEventClass::BuildInfo
    }
}

impl blaze_events::Postable for BuildInfoEvent {}

/// This event raised to indicate that no build will be happening for the given command.
// Java: com/google/devtools/build/lib/analysis/NoBuildEvent.java:28 NoBuildEvent
#[derive(Clone, Debug, Default)]
pub struct NoBuildEvent {
    id: Option<String>,
    command: Option<String>,
    start_time_millis: Option<i64>,
    separate_finished_event: bool,
    show_progress: bool,
    /// `BlazeVersionInfo.instance().getVersion()`.
    build_tool_version: String,
    /// `ProcessHandle.current().pid()`.
    server_pid: i64,
}

impl NoBuildEvent {
    // Java: com/google/devtools/build/lib/analysis/NoBuildEvent.java:35 NoBuildEvent.<init>
    pub fn new(
        command: Option<String>,
        start_time_millis: Option<i64>,
        separate_finished_event: bool,
        show_progress: bool,
        id: Option<String>,
        build_tool_version: String,
        server_pid: i64,
    ) -> Self {
        NoBuildEvent {
            id,
            command,
            start_time_millis,
            separate_finished_event,
            show_progress,
            build_tool_version,
            server_pid,
        }
    }

    // Java: com/google/devtools/build/lib/analysis/NoBuildEvent.java:95 NoBuildEvent.separateFinishedEvent
    pub fn separate_finished_event(&self) -> bool {
        self.separate_finished_event
    }

    // Java: com/google/devtools/build/lib/analysis/NoBuildEvent.java:99 NoBuildEvent.showProgress
    pub fn show_progress(&self) -> bool {
        self.show_progress
    }
}

impl BuildEvent for NoBuildEvent {
    // Java: com/google/devtools/build/lib/analysis/NoBuildEvent.java:67 NoBuildEvent.getEventId
    fn get_event_id(&self) -> BuildEventId {
        build_event_id_util::build_started_id()
    }

    // Java: com/google/devtools/build/lib/analysis/NoBuildEvent.java:57 NoBuildEvent.getChildrenEvents
    fn get_children_events(&self) -> Vec<BuildEventId> {
        if self.separate_finished_event {
            vec![
                ProgressEvent::initial_progress_update(),
                build_event_id_util::build_finished(),
            ]
        } else {
            vec![ProgressEvent::initial_progress_update()]
        }
    }

    // Java: com/google/devtools/build/lib/analysis/NoBuildEvent.java:72 NoBuildEvent.asStreamProto
    #[allow(deprecated)]
    fn as_stream_proto(&self, _: &dyn BuildEventContext) -> Result<bes::BuildEvent, Interrupted> {
        let mut started = bes::BuildStarted {
            build_tool_version: self.build_tool_version.clone(),
            ..Default::default()
        };
        if let Some(command) = &self.command {
            started.command = command.clone();
        }
        if let Some(start_time_millis) = self.start_time_millis {
            started.start_time_millis = start_time_millis;
            started.start_time = Some(timestamp_from_millis(start_time_millis));
        }
        if let Some(id) = &self.id {
            started.uuid = id.clone();
        }
        started.server_pid = self.server_pid;
        Ok(proto_chaining_with(self, Payload::Started(started)))
    }

    fn event_class(&self) -> BuildEventClass {
        BuildEventClass::NoBuild {
            separate_finished_event: self.separate_finished_event,
        }
    }
}

impl blaze_events::Postable for NoBuildEvent {}

/// `TestAttempt` (ported in `blaze_exec`) as a `BuildEventWithOrderConstraint`.
// Java: com/google/devtools/build/lib/analysis/test/TestAttempt.java:43 TestAttempt
impl BuildEvent for TestAttempt {
    fn get_event_id(&self) -> BuildEventId {
        TestAttempt::get_event_id(self)
    }

    fn get_children_events(&self) -> Vec<BuildEventId> {
        TestAttempt::get_children_events(self)
    }

    fn referenced_local_files(&self) -> Vec<LocalFile> {
        TestAttempt::referenced_local_files(self)
            .into_iter()
            .map(|(path, type_)| {
                let type_ = match type_ {
                    TestAttemptLocalFileType::SuccessfulTestOutput => {
                        LocalFileType::SuccessfulTestOutput
                    }
                    TestAttemptLocalFileType::FailedTestOutput => LocalFileType::FailedTestOutput,
                };
                LocalFile::new(path, type_, None)
            })
            .collect()
    }

    // Java: com/google/devtools/build/lib/analysis/TestAttempt.java:240 TestAttempt.asStreamProto
    fn as_stream_proto(
        &self,
        converters: &dyn BuildEventContext,
    ) -> Result<bes::BuildEvent, Interrupted> {
        let path_converter = converters.path_converter();
        let result = self.as_test_result(&|p| path_converter.apply(p));
        Ok(proto_chaining_with(self, Payload::TestResult(result)))
    }

    fn posted_after(&self) -> Option<Vec<BuildEventId>> {
        Some(TestAttempt::posted_after(self))
    }
}

/// The build event half of `AnalysisFailureEvent` (the event itself is posted by
/// `blaze_sky_top`): the id of the failed target (or aspect), the ids of its root causes as
/// children, and an `Aborted` with reason `ANALYSIS_FAILURE`.
///
/// PORT: Java computes the id and children from the event's keys and `Cause`s; the caller
/// passes them computed ([`AnalysisFailureBuildEvent::event_id`]).
// Java: com/google/devtools/build/lib/analysis/AnalysisFailureEvent.java:42 AnalysisFailureEvent
#[derive(Clone, Debug)]
pub struct AnalysisFailureBuildEvent {
    id: BuildEventId,
    children: Vec<BuildEventId>,
}

impl AnalysisFailureBuildEvent {
    pub fn new(id: BuildEventId, children: Vec<BuildEventId>) -> Self {
        AnalysisFailureBuildEvent { id, children }
    }

    /// `getEventId()`: `configuration_checksum` is the failed target's configuration key's
    /// checksum (`None` for the null key); `aspect` the failed aspect's configuration checksum
    /// and name.
    // Java: com/google/devtools/build/lib/analysis/AnalysisFailureEvent.java:127 AnalysisFailureEvent.getEventId
    pub fn event_id(
        label: &Label,
        is_configured: bool,
        configuration_checksum: Option<&str>,
        aspect: Option<(Option<&str>, &str)>,
    ) -> BuildEventId {
        if !is_configured {
            return build_event_id_util::target_configured(label);
        }
        match aspect {
            None => build_event_id_util::target_completed(
                label,
                &build_event_id_util::configuration_id_for_key(configuration_checksum),
            ),
            Some((aspect_configuration_checksum, aspect_name)) => {
                build_event_id_util::aspect_completed(
                    label,
                    &build_event_id_util::configuration_id_for_key(aspect_configuration_checksum),
                    aspect_name,
                )
            }
        }
    }
}

impl BuildEvent for AnalysisFailureBuildEvent {
    fn get_event_id(&self) -> BuildEventId {
        self.id.clone()
    }

    // Java: com/google/devtools/build/lib/analysis/AnalysisFailureEvent.java:141 AnalysisFailureEvent.getChildrenEvents
    fn get_children_events(&self) -> Vec<BuildEventId> {
        self.children.clone()
    }

    // Java: com/google/devtools/build/lib/analysis/AnalysisFailureEvent.java:147 AnalysisFailureEvent.asStreamProto
    fn as_stream_proto(&self, _: &dyn BuildEventContext) -> Result<bes::BuildEvent, Interrupted> {
        Ok(proto_chaining_with(
            self,
            Payload::Aborted(bes::Aborted {
                reason: bes::aborted::AbortReason::AnalysisFailure as i32,
                description: String::new(),
            }),
        ))
    }
}

impl blaze_events::Postable for AnalysisFailureBuildEvent {}
