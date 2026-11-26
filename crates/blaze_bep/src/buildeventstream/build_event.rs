//! Port of `BuildEvent` (with `LocalFile`), `ChainableEvent`, `BuildEventWithOrderConstraint`,
//! `BuildEventWithConfiguration`, `ReplaceableBuildEvent` and `EventReportingArtifacts`.
//!
//! PORT: Java expresses the optional capabilities of a build event as interfaces that the
//! streamer tests with `instanceof`. Here they are default methods of the one [`BuildEvent`]
//! trait returning `Option` (`None` = "does not implement the interface"), and the concrete
//! classes the streamer tests for are reported by [`BuildEvent::event_class`].

use std::fmt;
use std::sync::Arc;

use blaze_actions::Artifact;
use blaze_collect::nestedset::NestedSet;
use blaze_exec::actions::file_artifact_value::{FileArtifactValue, FileStateType};
use blaze_optdefs::misc::build_event_protocol_options::OutputGroupFileModes;
use blaze_protos::build_event_stream::{self as bes, BuildEventId};
use blaze_vfs::Path;

use super::build_event_context::BuildEventContext;
use crate::events::completion_context::CompletionContext;

/// Port of `InterruptedException` for `asStreamProto`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("interrupted")]
pub struct Interrupted;

/// The type of the local file. This is used by uploaders to determine how long to store the
/// associated files for.
// Java: com/google/devtools/build/lib/buildeventstream/BuildEvent.java:53 BuildEvent.LocalFile.LocalFileType
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LocalFileType {
    OutputFile,
    OutputDirectory,
    OutputSymlink,
    SuccessfulTestOutput,
    FailedTestOutput,
    CoverageOutput,
    QueryOutput,
    Stdout,
    Stderr,
    Log,
    PerformanceLog,
}

impl LocalFileType {
    /// Returns whether the LocalFile is a declared action output.
    // Java: com/google/devtools/build/lib/buildeventstream/BuildEvent.java:67 LocalFileType.isOutput
    pub fn is_output(self) -> bool {
        matches!(
            self,
            LocalFileType::OutputFile
                | LocalFileType::OutputDirectory
                | LocalFileType::OutputSymlink
        )
    }

    /// Returns the [`LocalFileType`] implied by a `FileArtifactValue`, or the associated
    /// `Artifact` if metadata is not available.
    // Java: com/google/devtools/build/lib/buildeventstream/BuildEvent.java:75 LocalFileType.forArtifact
    pub fn for_artifact(
        artifact: &Artifact,
        metadata: Option<&FileArtifactValue>,
    ) -> LocalFileType {
        if let Some(metadata) = metadata {
            if matches!(metadata, FileArtifactValue::RunfilesTreeMarker) {
                return LocalFileType::OutputDirectory;
            }
            return match metadata.get_type() {
                FileStateType::Directory => LocalFileType::OutputDirectory,
                FileStateType::Symlink => LocalFileType::OutputSymlink,
                _ => LocalFileType::OutputFile,
            };
        }
        if artifact.is_directory() {
            LocalFileType::OutputDirectory
        } else if artifact.is_symlink() {
            LocalFileType::OutputSymlink
        } else {
            LocalFileType::OutputFile
        }
    }
}

/// Indicates the type of compression the local file should have.
// Java: com/google/devtools/build/lib/buildeventstream/BuildEvent.java:99 BuildEvent.LocalFile.LocalFileCompression
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LocalFileCompression {
    None,
    Gzip,
}

/// A local file that is referenced by the build event. These can be uploaded to a separate
/// backend storage.
// Java: com/google/devtools/build/lib/buildeventstream/BuildEvent.java:47 BuildEvent.LocalFile
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocalFile {
    pub path: Path,
    pub type_: LocalFileType,
    pub compression: LocalFileCompression,
    pub artifact_metadata: Option<FileArtifactValue>,
}

impl LocalFile {
    // Java: com/google/devtools/build/lib/buildeventstream/BuildEvent.java:109 LocalFile.<init>
    pub fn new(
        path: Path,
        type_: LocalFileType,
        artifact_metadata: Option<FileArtifactValue>,
    ) -> Self {
        Self::with_compression(path, type_, LocalFileCompression::None, artifact_metadata)
    }

    // Java: com/google/devtools/build/lib/buildeventstream/BuildEvent.java:109 LocalFile.<init>
    pub fn with_compression(
        path: Path,
        type_: LocalFileType,
        compression: LocalFileCompression,
        artifact_metadata: Option<FileArtifactValue>,
    ) -> Self {
        LocalFile {
            path,
            type_,
            compression,
            artifact_metadata,
        }
    }
}

/// `EventReportingArtifacts.ReportedArtifacts`: the artifact sets an event refers to by name,
/// with the completion context that knows their metadata.
// Java: com/google/devtools/build/lib/actions/EventReportingArtifacts.java:26 EventReportingArtifacts.ReportedArtifacts
#[derive(Clone)]
pub struct ReportedArtifacts {
    /// Java: an `ImmutableSet<NestedSet<Artifact>>` (identity, insertion order).
    pub artifacts: Vec<NestedSet<Artifact>>,
    pub completion_context: Arc<CompletionContext>,
}

impl ReportedArtifacts {
    // Java: com/google/devtools/build/lib/actions/EventReportingArtifacts.java:30 ReportedArtifacts.<init>
    pub fn new(
        artifacts: Vec<NestedSet<Artifact>>,
        completion_context: Arc<CompletionContext>,
    ) -> Self {
        // ImmutableSet: NestedSet equality is identity.
        let mut unique: Vec<NestedSet<Artifact>> = Vec::with_capacity(artifacts.len());
        for set in artifacts {
            if !unique.contains(&set) {
                unique.push(set);
            }
        }
        ReportedArtifacts {
            artifacts: unique,
            completion_context,
        }
    }
}

/// The result facts of a `BuildCompleteEvent` the streamer reads (`event.getResult()`).
// Java: com/google/devtools/build/lib/buildtool/BuildResult.java:51 BuildResult
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BuildCompleteResultFacts {
    /// `getUnhandledThrowable() != null`.
    pub has_unhandled_throwable: bool,
    /// `wasCatastrophe()`.
    pub was_catastrophe: bool,
    /// `getSuccess()`.
    pub success: bool,
    /// `getStopOnFirstFailure()`.
    pub stop_on_first_failure: bool,
    /// `getDetailedExitCode().getExitCode().equals(ExitCode.OOM_ERROR)`.
    pub is_oom: bool,
}

/// The concrete Java classes `BuildEventStreamer` tests with `instanceof`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BuildEventClass {
    /// Any other class.
    Other,
    /// `BuildStartingEvent`, with `request().getCommandName()`.
    BuildStarting { command_name: String },
    /// A `BuildCompletingEvent`; `build_complete` is set for the subclass `BuildCompleteEvent`.
    BuildCompleting {
        build_complete: Option<BuildCompleteResultFacts>,
    },
    /// `NoBuildEvent`, with `separateFinishedEvent()`.
    NoBuild { separate_finished_event: bool },
    /// `TargetParsingCompleteEvent`: `getOriginalTargetPattern().size()` and whether
    /// `getFailedTargetPatterns()` is empty.
    TargetParsingComplete {
        original_target_pattern_count: usize,
        failed_target_patterns_empty: bool,
    },
    /// `ActionExecutedEvent`: `getException() != null` and `getAction() instanceof ExtraAction`.
    ActionExecuted {
        has_exception: bool,
        is_extra_action: bool,
    },
    /// `TestSummary`, with `totalRuns()`.
    TestSummary { total_runs: i32 },
    /// `BuildInfoEvent`.
    BuildInfo,
    /// `BuildConfigurationEvent`.
    BuildConfiguration,
}

/// Interface for objects that can be posted on the public event stream.
///
/// Objects posted on the build-event stream will implement this interface. This allows
/// pass-through of events, as well as proper chaining of events.
///
/// The first two methods are `ChainableEvent`: objects organised in a DAG-like fashion, where
/// each node knows its immediate successor nodes.
// Java: com/google/devtools/build/lib/buildeventstream/BuildEvent.java:38 BuildEvent
// Java: com/google/devtools/build/lib/buildeventstream/ChainableEvent.java:30 ChainableEvent
pub trait BuildEvent: Send + Sync + fmt::Debug {
    /// Provide the identifier of the event. Event identifiers have to be unique within the set of
    /// events belonging to the same build invocation.
    // Java: com/google/devtools/build/lib/buildeventstream/ChainableEvent.java:38 ChainableEvent.getEventId
    fn get_event_id(&self) -> BuildEventId;

    /// Provide the children of the event. A build-event stream is finished if and only if all
    /// announced children have occurred.
    // Java: com/google/devtools/build/lib/buildeventstream/ChainableEvent.java:50 ChainableEvent.getChildrenEvents
    fn get_children_events(&self) -> Vec<BuildEventId>;

    /// Returns a list of files that are referenced in the protobuf representation returned by
    /// [`BuildEvent::as_stream_proto`].
    // Java: com/google/devtools/build/lib/buildeventstream/BuildEvent.java:165 BuildEvent.referencedLocalFiles
    fn referenced_local_files(&self) -> Vec<LocalFile> {
        Vec::new()
    }

    /// Returns the URIs of in-flight remote uploads (`remoteUploads`). PORT: Blaze has no remote
    /// uploader; every future is already resolved, so they are plain values (`None` = the
    /// upload did not finish).
    // Java: com/google/devtools/build/lib/buildeventstream/BuildEvent.java:174 BuildEvent.remoteUploads
    fn remote_uploads(&self) -> Vec<Option<String>> {
        Vec::new()
    }

    /// Provide a binary representation of the event.
    // Java: com/google/devtools/build/lib/buildeventstream/BuildEvent.java:184 BuildEvent.asStreamProto
    fn as_stream_proto(
        &self,
        context: &dyn BuildEventContext,
    ) -> Result<bes::BuildEvent, Interrupted>;

    /// `BuildEventWithOrderConstraint.postedAfter`: events that need to be posted on the build
    /// event stream before this event. `None` if the event has no order constraint.
    // Java: com/google/devtools/build/lib/buildeventstream/BuildEventWithOrderConstraint.java:29 BuildEventWithOrderConstraint.postedAfter
    fn posted_after(&self) -> Option<Vec<BuildEventId>> {
        None
    }

    /// `BuildEventWithConfiguration.getConfigurations`: the configurations the event mentions,
    /// which should be introduced in the stream before this event. `None` if the event does not
    /// implement the interface.
    // Java: com/google/devtools/build/lib/buildeventstream/BuildEventWithConfiguration.java:26 BuildEventWithConfiguration.getConfigurations
    fn get_configurations(&self) -> Option<Vec<Arc<dyn BuildEvent>>> {
        None
    }

    /// `EventReportingArtifacts.reportedArtifacts`. `None` if the event does not report
    /// artifacts.
    // Java: com/google/devtools/build/lib/actions/EventReportingArtifacts.java:38 EventReportingArtifacts.reportedArtifacts
    fn reported_artifacts(
        &self,
        _output_group_file_modes: &OutputGroupFileModes,
    ) -> Option<ReportedArtifacts> {
        None
    }

    /// `ReplaceableBuildEvent.replaceable`. `None` if the event's class is not replaceable.
    // Java: com/google/devtools/build/lib/buildeventstream/ReplaceableBuildEvent.java:35 ReplaceableBuildEvent.replaceable
    fn replaceable(&self) -> Option<bool> {
        None
    }

    /// The concrete class, for the streamer's `instanceof` tests.
    fn event_class(&self) -> BuildEventClass {
        BuildEventClass::Other
    }

    /// `event instanceof BuildCompletingEvent e ? e.getExitCode().getNumericExitCode() : …`:
    /// `Some(numeric exit code)` for a `BuildCompletingEvent` (the Build Event Service
    /// uploader's `instanceof` test).
    fn build_completing_exit_code(&self) -> Option<i32> {
        None
    }

    /// `event instanceof AbortedEvent` (the Build Event Service uploader's test).
    fn is_aborted_event(&self) -> bool {
        false
    }
}

impl<T: BuildEvent + ?Sized> BuildEvent for Arc<T> {
    fn get_event_id(&self) -> BuildEventId {
        (**self).get_event_id()
    }
    fn build_completing_exit_code(&self) -> Option<i32> {
        (**self).build_completing_exit_code()
    }
    fn is_aborted_event(&self) -> bool {
        (**self).is_aborted_event()
    }
    fn get_children_events(&self) -> Vec<BuildEventId> {
        (**self).get_children_events()
    }
    fn referenced_local_files(&self) -> Vec<LocalFile> {
        (**self).referenced_local_files()
    }
    fn remote_uploads(&self) -> Vec<Option<String>> {
        (**self).remote_uploads()
    }
    fn as_stream_proto(
        &self,
        context: &dyn BuildEventContext,
    ) -> Result<bes::BuildEvent, Interrupted> {
        (**self).as_stream_proto(context)
    }
    fn posted_after(&self) -> Option<Vec<BuildEventId>> {
        (**self).posted_after()
    }
    fn get_configurations(&self) -> Option<Vec<Arc<dyn BuildEvent>>> {
        (**self).get_configurations()
    }
    fn reported_artifacts(&self, modes: &OutputGroupFileModes) -> Option<ReportedArtifacts> {
        (**self).reported_artifacts(modes)
    }
    fn replaceable(&self) -> Option<bool> {
        (**self).replaceable()
    }
    fn event_class(&self) -> BuildEventClass {
        (**self).event_class()
    }
}
