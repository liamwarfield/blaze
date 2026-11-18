//! Ports of build events from `lib/actions`: `BuildConfigurationEvent` (the struct is
//! `blaze_actions::BuildConfigurationEvent`, built by `blaze_analysis::rule::config_util::
//! to_build_event`) and `ActionExecutedEvent`.

use std::sync::Arc;

use blaze_actions::{Artifact, BuildConfigurationEvent};
use blaze_cmdline::label::Label;
use blaze_exec::actions::file_artifact_value::FileArtifactValue;
use blaze_protos::build_event_stream::build_event::Payload;
use blaze_protos::build_event_stream::{self as bes, BuildEventId};
use blaze_protos::failure_details::FailureDetail;
use blaze_protos::google::protobuf::Timestamp;
use blaze_util::check_state;
use blaze_vfs::{Path, PathFragment};

use crate::buildeventstream::build_event::{
    BuildEvent, BuildEventClass, Interrupted, LocalFile, LocalFileType,
};
use crate::buildeventstream::build_event_context::BuildEventContext;
use crate::buildeventstream::build_event_id_util;
use crate::buildeventstream::generic_build_event::{NullConfiguration, proto_chaining_with};

/// The configuration's event: `BuildEvent(id = configuration(event_id), configuration)`.
// Java: com/google/devtools/build/lib/actions/BuildConfigurationEvent.java:29 BuildConfigurationEvent
impl BuildEvent for BuildConfigurationEvent {
    // Java: com/google/devtools/build/lib/actions/BuildConfigurationEvent.java:46 BuildConfigurationEvent.getEventId
    fn get_event_id(&self) -> BuildEventId {
        build_event_id_util::configuration_id(&self.event_id)
    }

    // Java: com/google/devtools/build/lib/actions/BuildConfigurationEvent.java:51 BuildConfigurationEvent.getChildrenEvents
    fn get_children_events(&self) -> Vec<BuildEventId> {
        Vec::new()
    }

    // Java: com/google/devtools/build/lib/actions/BuildConfigurationEvent.java:41 BuildConfigurationEvent.asStreamProto
    fn as_stream_proto(&self, _: &dyn BuildEventContext) -> Result<bes::BuildEvent, Interrupted> {
        Ok(bes::BuildEvent {
            id: Some(self.get_event_id()),
            payload: Some(Payload::Configuration(self.configuration.clone())),
            ..Default::default()
        })
    }

    fn event_class(&self) -> BuildEventClass {
        BuildEventClass::BuildConfiguration
    }
}

/// `ActionExecutedEvent.ErrorTiming`.
// Java: com/google/devtools/build/lib/actions/ActionExecutedEvent.java:260 ActionExecutedEvent.ErrorTiming
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorTiming {
    NoError,
    BeforeExecution,
    AfterExecution,
}

/// The failure of an action (`ActionExecutionException`): what the event reports of it.
#[derive(Clone, Debug)]
pub struct ActionFailure {
    /// `exception.getExitCode().getNumericExitCode()`.
    pub numeric_exit_code: i32,
    /// `exception.getDetailedExitCode().getFailureDetail()`.
    pub failure_detail: Option<FailureDetail>,
}

/// What `ActionExecutedEvent` reads from the `Action`.
#[derive(Clone, Debug)]
pub struct ExecutedActionInfo {
    /// `action.getMnemonic()`.
    pub mnemonic: String,
    /// `action.getOwner() != null`: the owner's label (may be null) and configuration event
    /// (null means the null configuration) and checksum.
    pub owner: Option<ExecutedActionOwner>,
    /// `((CommandAction) action).getArguments()` for command actions (`None` otherwise or when
    /// the command line cannot be expanded).
    pub arguments: Option<Vec<String>>,
    /// `action instanceof ExtraAction`.
    pub is_extra_action: bool,
}

#[derive(Clone, Debug)]
pub struct ExecutedActionOwner {
    pub label: Option<Label>,
    pub configuration_checksum: Option<String>,
    pub build_configuration_event: Option<BuildConfigurationEvent>,
}

/// This event is fired during the build, when an action is executed. It contains information
/// about the action: the Action itself, and the output file names its stdout and stderr are
/// recorded in.
// Java: com/google/devtools/build/lib/actions/ActionExecutedEvent.java:44 ActionExecutedEvent
#[derive(Clone, Debug)]
pub struct ActionExecutedEvent {
    action_id: PathFragment,
    action: ExecutedActionInfo,
    exception: Option<ActionFailure>,
    primary_output: Path,
    output_artifact: Artifact,
    primary_output_metadata: Option<FileArtifactValue>,
    stdout: Option<Path>,
    stderr: Option<Path>,
    timing: ErrorTiming,
    start_time: Option<Timestamp>,
    end_time: Option<Timestamp>,
}

impl ActionExecutedEvent {
    // Java: com/google/devtools/build/lib/actions/ActionExecutedEvent.java:63 ActionExecutedEvent.<init>
    pub fn new(
        action_id: PathFragment,
        action: ExecutedActionInfo,
        exception: Option<ActionFailure>,
        primary_output: Path,
        output_artifact: Artifact,
        primary_output_metadata: Option<FileArtifactValue>,
        stdout: Option<Path>,
        stderr: Option<Path>,
        timing: ErrorTiming,
        start_time: Option<Timestamp>,
        end_time: Option<Timestamp>,
    ) -> Self {
        check_state!(exception.is_none() == (timing == ErrorTiming::NoError));
        check_state!(exception.is_none() != primary_output_metadata.is_none());
        ActionExecutedEvent {
            action_id,
            action,
            exception,
            primary_output,
            output_artifact,
            primary_output_metadata,
            stdout,
            stderr,
            timing,
            start_time,
            end_time,
        }
    }

    // Java: com/google/devtools/build/lib/actions/ActionExecutedEvent.java:97 ActionExecutedEvent.getException
    pub fn get_exception(&self) -> Option<&ActionFailure> {
        self.exception.as_ref()
    }

    // Java: com/google/devtools/build/lib/actions/ActionExecutedEvent.java:101 ActionExecutedEvent.errorTiming
    pub fn error_timing(&self) -> ErrorTiming {
        self.timing
    }

    fn owner_configuration_event(&self) -> Option<Arc<dyn BuildEvent>> {
        let owner = self.action.owner.as_ref()?;
        Some(match &owner.build_configuration_event {
            Some(c) => Arc::new(c.clone()),
            None => NullConfiguration::instance(),
        })
    }
}

impl BuildEvent for ActionExecutedEvent {
    // Java: com/google/devtools/build/lib/actions/ActionExecutedEvent.java:127 ActionExecutedEvent.getEventId
    fn get_event_id(&self) -> BuildEventId {
        match &self.action.owner {
            None => build_event_id_util::action_completed(&self.action_id),
            Some(owner) => build_event_id_util::action_completed_with(
                &self.action_id,
                owner.label.as_ref(),
                owner.configuration_checksum.as_deref(),
            ),
        }
    }

    // Java: com/google/devtools/build/lib/actions/ActionExecutedEvent.java:137 ActionExecutedEvent.getChildrenEvents
    fn get_children_events(&self) -> Vec<BuildEventId> {
        Vec::new()
    }

    // Java: com/google/devtools/build/lib/actions/ActionExecutedEvent.java:142 ActionExecutedEvent.getConfigurations
    fn get_configurations(&self) -> Option<Vec<Arc<dyn BuildEvent>>> {
        Some(self.owner_configuration_event().into_iter().collect())
    }

    // Java: com/google/devtools/build/lib/actions/ActionExecutedEvent.java:155 ActionExecutedEvent.referencedLocalFiles
    fn referenced_local_files(&self) -> Vec<LocalFile> {
        let mut local_files = Vec::new();
        if let Some(stdout) = &self.stdout {
            local_files.push(LocalFile::new(stdout.clone(), LocalFileType::Stdout, None));
        }
        if let Some(stderr) = &self.stderr {
            local_files.push(LocalFile::new(stderr.clone(), LocalFileType::Stderr, None));
        }
        if self.exception.is_none() {
            local_files.push(LocalFile::new(
                self.primary_output.clone(),
                LocalFileType::for_artifact(
                    &self.output_artifact,
                    self.primary_output_metadata.as_ref(),
                ),
                self.primary_output_metadata.clone(),
            ));
        }
        local_files
    }

    // Java: com/google/devtools/build/lib/actions/ActionExecutedEvent.java:175 ActionExecutedEvent.asStreamProto
    #[allow(deprecated)]
    fn as_stream_proto(
        &self,
        converters: &dyn BuildEventContext,
    ) -> Result<bes::BuildEvent, Interrupted> {
        let path_converter = converters.path_converter();
        let mut action_builder = bes::ActionExecuted {
            success: self.exception.is_none(),
            r#type: self.action.mnemonic.clone(),
            ..Default::default()
        };
        if let Some(start_time) = &self.start_time {
            action_builder.start_time = Some(*start_time);
            if let Some(end_time) = &self.end_time {
                action_builder.end_time = Some(*end_time);
            }
        }

        if let Some(exception) = &self.exception {
            action_builder.exit_code = exception.numeric_exit_code;
            if let Some(failure_detail) = &exception.failure_detail {
                action_builder.failure_detail = Some(failure_detail.clone());
            }
        }
        if let Some(stdout) = &self.stdout
            && let Some(uri) = path_converter.apply(stdout)
        {
            action_builder.stdout = Some(bes::File {
                name: "stdout".to_string(),
                file: Some(bes::file::File::Uri(uri)),
                ..Default::default()
            });
        }
        if let Some(stderr) = &self.stderr
            && let Some(uri) = path_converter.apply(stderr)
        {
            action_builder.stderr = Some(bes::File {
                name: "stderr".to_string(),
                file: Some(bes::file::File::Uri(uri)),
                ..Default::default()
            });
        }
        if let Some(owner) = &self.action.owner
            && let Some(label) = &owner.label
        {
            action_builder.label = label.to_string();
        }
        if let Some(configuration) = self.owner_configuration_event() {
            action_builder.configuration = Some(match configuration.get_event_id().id {
                Some(blaze_protos::build_event_stream::build_event_id::Id::Configuration(c)) => c,
                _ => Default::default(),
            });
        }
        if self.exception.is_none()
            && let Some(uri) = path_converter.apply(&self.primary_output)
        {
            action_builder.primary_output = Some(bes::File {
                file: Some(bes::file::File::Uri(uri)),
                ..Default::default()
            });
        }
        if let Some(arguments) = &self.action.arguments {
            action_builder
                .command_line
                .extend(arguments.iter().cloned());
        }
        Ok(proto_chaining_with(self, Payload::Action(action_builder)))
    }

    fn event_class(&self) -> BuildEventClass {
        BuildEventClass::ActionExecuted {
            has_exception: self.exception.is_some(),
            is_extra_action: self.action.is_extra_action,
        }
    }
}

impl blaze_events::Postable for ActionExecutedEvent {}
