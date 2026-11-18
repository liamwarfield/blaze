//! Ports of build events from `lib/buildtool/buildevent` and the marker events the streamer
//! subscribes to.

use blaze_protos::build_event_stream::build_event::Payload;
use blaze_protos::build_event_stream::{self as bes, BuildEventId, ConvenienceSymlink};
use blaze_util::{DetailedExitCode, ExitCode};

use crate::buildeventstream::build_completing_event::BuildCompletingEvent;
use crate::buildeventstream::build_event::{
    BuildCompleteResultFacts, BuildEvent, BuildEventClass, Interrupted,
};
use crate::buildeventstream::build_event_context::BuildEventContext;
use crate::buildeventstream::build_event_id_util;
use crate::buildeventstream::generic_build_event::{ProgressEvent, proto_chaining_with};
use crate::util::timestamp_from_millis;

/// `CommandLineEvent.OriginalCommandLineEvent.LABEL`.
pub const ORIGINAL_COMMAND_LINE_LABEL: &str = "original";
/// `CommandLineEvent.CanonicalCommandLineEvent.LABEL`.
pub const CANONICAL_COMMAND_LINE_LABEL: &str = "canonical";
/// `CommandLineEvent.ToolCommandLineEvent.LABEL`.
pub const TOOL_COMMAND_LINE_LABEL: &str = "tool";

/// What `BuildStartingEvent` reads from the `BuildRequest` and the command environment.
#[derive(Clone, Debug, Default)]
pub struct BuildStartingData {
    /// `request().getId().toString()`: the command id (UUID).
    pub uuid: String,
    /// `request().getStartTime()`.
    pub start_time_millis: i64,
    /// `BlazeVersionInfo.instance().getVersion()`.
    pub build_tool_version: String,
    /// `request().getOptionsDescription()`.
    pub options_description: String,
    /// `request().getCommandName()`.
    pub command_name: String,
    /// `ProcessHandle.current().pid()`.
    pub server_pid: i64,
    /// `env.getWorkingDirectory().toString()`.
    pub pwd: String,
    /// `env.getDirectories().getWorkspace()`.
    pub workspace: Option<String>,
    /// `NetUtil.getCachedShortHostName()`.
    pub host: String,
    /// `UserUtils.getUserName()`.
    pub user: String,
    /// `request().getTargets()`: the target patterns as given.
    pub targets: Vec<String>,
}

/// This event is fired from BuildTool#startRequest(). At this point, the set of target patterns
/// are known, but have yet to be parsed.
// Java: com/google/devtools/build/lib/buildtool/buildevent/BuildStartingEvent.java:41 BuildStartingEvent
#[derive(Clone, Debug)]
pub struct BuildStartingEvent {
    data: BuildStartingData,
}

impl BuildStartingEvent {
    // Java: com/google/devtools/build/lib/buildtool/buildevent/BuildStartingEvent.java:67 BuildStartingEvent.create
    pub fn create(data: BuildStartingData) -> Self {
        BuildStartingEvent { data }
    }

    pub fn data(&self) -> &BuildStartingData {
        &self.data
    }
}

impl BuildEvent for BuildStartingEvent {
    // Java: com/google/devtools/build/lib/buildtool/buildevent/BuildStartingEvent.java:91 BuildStartingEvent.getEventId
    fn get_event_id(&self) -> BuildEventId {
        build_event_id_util::build_started_id()
    }

    // Java: com/google/devtools/build/lib/buildtool/buildevent/BuildStartingEvent.java:96 BuildStartingEvent.getChildrenEvents
    fn get_children_events(&self) -> Vec<BuildEventId> {
        vec![
            ProgressEvent::initial_progress_update(),
            build_event_id_util::unstructured_commandline_id(),
            build_event_id_util::structured_commandline_id(ORIGINAL_COMMAND_LINE_LABEL),
            build_event_id_util::structured_commandline_id(CANONICAL_COMMAND_LINE_LABEL),
            build_event_id_util::structured_commandline_id(TOOL_COMMAND_LINE_LABEL),
            build_event_id_util::build_metadata_id(),
            build_event_id_util::options_parsed_id(),
            build_event_id_util::workspace_status_id(),
            build_event_id_util::target_pattern_expanded(&self.data.targets),
            build_event_id_util::build_finished(),
        ]
    }

    // Java: com/google/devtools/build/lib/buildtool/buildevent/BuildStartingEvent.java:111 BuildStartingEvent.asStreamProto
    #[allow(deprecated)]
    fn as_stream_proto(&self, _: &dyn BuildEventContext) -> Result<bes::BuildEvent, Interrupted> {
        let d = &self.data;
        let mut started = bes::BuildStarted {
            uuid: d.uuid.clone(),
            start_time: Some(timestamp_from_millis(d.start_time_millis)),
            start_time_millis: d.start_time_millis,
            build_tool_version: d.build_tool_version.clone(),
            options_description: d.options_description.clone(),
            command: d.command_name.clone(),
            server_pid: d.server_pid,
            working_directory: d.pwd.clone(),
            host: d.host.clone(),
            user: d.user.clone(),
            ..Default::default()
        };
        if let Some(workspace) = &d.workspace {
            started.workspace_directory = workspace.clone();
        }
        Ok(proto_chaining_with(self, Payload::Started(started)))
    }

    fn event_class(&self) -> BuildEventClass {
        BuildEventClass::BuildStarting {
            command_name: self.data.command_name.clone(),
        }
    }
}

impl blaze_events::Postable for BuildStartingEvent {}

/// What the streamer and `BuildCompleteEvent` read from a `BuildResult`.
#[derive(Clone, Debug)]
pub struct BuildResultData {
    /// `getDetailedExitCode()`.
    pub detailed_exit_code: DetailedExitCode,
    /// `getStopTime()`.
    pub stop_time_millis: i64,
    pub has_unhandled_throwable: bool,
    pub was_catastrophe: bool,
    pub success: bool,
    pub stop_on_first_failure: bool,
}

/// This event is fired from BuildTool#stopRequest().
///
/// PORT: a `BuildCompletingEvent` whose class carries the `BuildResult` facts.
// Java: com/google/devtools/build/lib/buildtool/buildevent/BuildCompleteEvent.java:30 BuildCompleteEvent
pub fn build_complete_event(
    result: &BuildResultData,
    children: Vec<BuildEventId>,
) -> BuildCompletingEvent {
    BuildCompletingEvent::with_detailed_exit_code(
        &result.detailed_exit_code,
        result.stop_time_millis,
        children,
    )
    .with_class(BuildEventClass::BuildCompleting {
        build_complete: Some(BuildCompleteResultFacts {
            has_unhandled_throwable: result.has_unhandled_throwable,
            was_catastrophe: result.was_catastrophe,
            success: result.success,
            stop_on_first_failure: result.stop_on_first_failure,
            is_oom: result.detailed_exit_code.get_exit_code() == ExitCode::OOM_ERROR,
        }),
    })
}

/// Event triggered after testing has completed.
// Java: com/google/devtools/build/lib/buildtool/buildevent/TestingCompleteEvent.java:29 TestingCompleteEvent
pub fn testing_complete_event(
    exit_code: ExitCode,
    finish_time_millis: i64,
) -> BuildCompletingEvent {
    BuildCompletingEvent::new(
        exit_code,
        finish_time_millis,
        vec![
            build_event_id_util::build_tool_logs(),
            build_event_id_util::build_metrics(),
        ],
    )
}

/// The build-completing event of `bazel run`.
// Java: com/google/devtools/build/lib/buildtool/buildevent/RunBuildCompleteEvent.java:29 RunBuildCompleteEvent
pub fn run_build_complete_event(
    exit_code: ExitCode,
    finish_time_millis: i64,
    children: Vec<BuildEventId>,
) -> BuildCompletingEvent {
    BuildCompletingEvent::new(exit_code, finish_time_millis, children)
}

/// An event signaling which convenience symlinks were created or deleted.
// Java: com/google/devtools/build/lib/buildtool/buildevent/ConvenienceSymlinksIdentifiedEvent.java:33 ConvenienceSymlinksIdentifiedEvent
#[derive(Clone, Debug)]
pub struct ConvenienceSymlinksIdentifiedEvent {
    convenience_symlinks: Vec<ConvenienceSymlink>,
}

impl ConvenienceSymlinksIdentifiedEvent {
    // Java: com/google/devtools/build/lib/buildtool/buildevent/ConvenienceSymlinksIdentifiedEvent.java:37 ConvenienceSymlinksIdentifiedEvent.<init>
    pub fn new(convenience_symlinks: Vec<ConvenienceSymlink>) -> Self {
        ConvenienceSymlinksIdentifiedEvent {
            convenience_symlinks,
        }
    }
}

impl BuildEvent for ConvenienceSymlinksIdentifiedEvent {
    // Java: com/google/devtools/build/lib/buildtool/buildevent/ConvenienceSymlinksIdentifiedEvent.java:42 ConvenienceSymlinksIdentifiedEvent.getEventId
    fn get_event_id(&self) -> BuildEventId {
        build_event_id_util::convenience_symlinks_identified_id()
    }

    // Java: com/google/devtools/build/lib/buildtool/buildevent/ConvenienceSymlinksIdentifiedEvent.java:47 ConvenienceSymlinksIdentifiedEvent.getChildrenEvents
    fn get_children_events(&self) -> Vec<BuildEventId> {
        Vec::new()
    }

    // Java: com/google/devtools/build/lib/buildtool/buildevent/ConvenienceSymlinksIdentifiedEvent.java:52 ConvenienceSymlinksIdentifiedEvent.asStreamProto
    fn as_stream_proto(&self, _: &dyn BuildEventContext) -> Result<bes::BuildEvent, Interrupted> {
        Ok(proto_chaining_with(
            self,
            Payload::ConvenienceSymlinksIdentified(bes::ConvenienceSymlinksIdentified {
                convenience_symlinks: self.convenience_symlinks.clone(),
            }),
        ))
    }
}

impl blaze_events::Postable for ConvenienceSymlinksIdentifiedEvent {}

/// `BuildInterruptedEvent`: the build was interrupted (the streamer records USER_INTERRUPTED).
// Java: com/google/devtools/build/lib/buildtool/buildevent/BuildInterruptedEvent.java:21 BuildInterruptedEvent
#[derive(Clone, Copy, Debug, Default)]
pub struct BuildInterruptedEvent;
impl blaze_events::Postable for BuildInterruptedEvent {}

/// `NoAnalyzeEvent`: `--nobuild --noanalyze`-style runs (the streamer records NO_ANALYZE).
// Java: com/google/devtools/build/lib/buildtool/buildevent/NoAnalyzeEvent.java:20 NoAnalyzeEvent
#[derive(Clone, Copy, Debug, Default)]
pub struct NoAnalyzeEvent;
impl blaze_events::Postable for NoAnalyzeEvent {}

/// `NoExecutionEvent`: `--nobuild` (the streamer records NO_BUILD).
// Java: com/google/devtools/build/lib/buildtool/buildevent/NoExecutionEvent.java:20 NoExecutionEvent
#[derive(Clone, Copy, Debug, Default)]
pub struct NoExecutionEvent;
impl blaze_events::Postable for NoExecutionEvent {}
