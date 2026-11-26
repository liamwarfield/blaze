//! Port of `BuildCompletingEvent`, `BuildToolLogs` and `FetchEvent`.

use blaze_protos::build_event_stream::build_event::Payload;
use blaze_protos::build_event_stream::build_event_id::fetch_id::Downloader;
use blaze_protos::build_event_stream::{self as bes, BuildEventId};
use blaze_protos::failure_details::FailureDetail;
use blaze_util::{DetailedExitCode, ExitCode};

use super::build_event::{BuildEvent, BuildEventClass, Interrupted, LocalFile};
use super::build_event_context::BuildEventContext;
use super::build_event_id_util;
use super::generic_build_event::proto_chaining_with;
use crate::util::timestamp_from_millis;

/// Class all events completing a build inherit from.
///
/// PORT: Java's abstract class; subclasses (`BuildCompleteEvent`, `TestingCompleteEvent`, ...)
/// are constructors of this struct that set [`BuildCompletingEvent::class`].
// Java: com/google/devtools/build/lib/buildeventstream/BuildCompletingEvent.java:31 BuildCompletingEvent
#[derive(Clone, Debug)]
pub struct BuildCompletingEvent {
    /// `detailedExitCode.getFailureDetail()` when constructed from a `DetailedExitCode`.
    failure_detail: Option<FailureDetail>,
    exit_code: ExitCode,
    finish_time_millis: i64,
    children: Vec<BuildEventId>,
    class: BuildEventClass,
}

impl BuildCompletingEvent {
    // Java: com/google/devtools/build/lib/buildeventstream/BuildCompletingEvent.java:38 BuildCompletingEvent.<init>
    pub fn new(exit_code: ExitCode, finish_time_millis: i64, children: Vec<BuildEventId>) -> Self {
        BuildCompletingEvent {
            failure_detail: None,
            exit_code,
            finish_time_millis,
            children,
            class: BuildEventClass::BuildCompleting {
                build_complete: None,
            },
        }
    }

    // Java: com/google/devtools/build/lib/buildeventstream/BuildCompletingEvent.java:38 BuildCompletingEvent.<init>
    pub fn with_detailed_exit_code(
        detailed_exit_code: &DetailedExitCode,
        finish_time_millis: i64,
        children: Vec<BuildEventId>,
    ) -> Self {
        BuildCompletingEvent {
            failure_detail: detailed_exit_code.get_failure_detail().cloned(),
            exit_code: detailed_exit_code.get_exit_code(),
            finish_time_millis,
            children,
            class: BuildEventClass::BuildCompleting {
                build_complete: None,
            },
        }
    }

    /// Sets the class reported to the streamer (for subclasses such as `BuildCompleteEvent`).
    pub fn with_class(mut self, class: BuildEventClass) -> Self {
        self.class = class;
        self
    }

    // Java: com/google/devtools/build/lib/buildeventstream/BuildCompletingEvent.java:58 BuildCompletingEvent.getExitCode
    pub fn get_exit_code(&self) -> &ExitCode {
        &self.exit_code
    }
}

impl BuildEvent for BuildCompletingEvent {
    // Java: com/google/devtools/build/lib/buildeventstream/BuildCompletingEvent.java:63 BuildCompletingEvent.getEventId
    fn get_event_id(&self) -> BuildEventId {
        build_event_id_util::build_finished()
    }

    // Java: com/google/devtools/build/lib/buildeventstream/BuildCompletingEvent.java:68 BuildCompletingEvent.getChildrenEvents
    fn get_children_events(&self) -> Vec<BuildEventId> {
        self.children.clone()
    }

    // Java: com/google/devtools/build/lib/buildeventstream/BuildCompletingEvent.java:73 BuildCompletingEvent.asStreamProto
    #[allow(deprecated)]
    fn as_stream_proto(&self, _: &dyn BuildEventContext) -> Result<bes::BuildEvent, Interrupted> {
        let proto_exit_code = bes::build_finished::ExitCode {
            name: self.exit_code.name().to_string(),
            code: self.exit_code.get_numeric_exit_code(),
        };
        let mut finished = bes::BuildFinished {
            overall_success: self.exit_code == ExitCode::SUCCESS,
            exit_code: Some(proto_exit_code),
            finish_time: Some(timestamp_from_millis(self.finish_time_millis)),
            finish_time_millis: self.finish_time_millis,
            ..Default::default()
        };
        if let Some(failure_detail) = &self.failure_detail {
            finished.failure_detail = Some(failure_detail.clone());
        }
        Ok(proto_chaining_with(self, Payload::Finished(finished)))
    }

    fn event_class(&self) -> BuildEventClass {
        self.class.clone()
    }

    fn build_completing_exit_code(&self) -> Option<i32> {
        Some(self.exit_code.get_numeric_exit_code())
    }
}

/// A log file to report in `BuildToolLogs`: `LogFileEntry(name, localFile)`.
// Java: com/google/devtools/build/lib/buildeventstream/BuildToolLogs.java:118 BuildToolLogs.LogFileEntry
#[derive(Clone, Debug)]
pub struct LogFileEntry {
    pub name: String,
    pub local_file: LocalFile,
}

/// Event reporting on statistics about the build.
// Java: com/google/devtools/build/lib/buildeventstream/BuildToolLogs.java:29 BuildToolLogs
#[derive(Clone, Debug)]
pub struct BuildToolLogs {
    direct_values: Vec<(String, Vec<u8>)>,
    /// PORT: `Pair<String, ListenableFuture<String>>`; the value is the URI if the future is
    /// done (and not cancelled), `None` otherwise.
    future_uris: Vec<(String, Option<String>)>,
    log_files: Vec<LogFileEntry>,
}

impl BuildToolLogs {
    // Java: com/google/devtools/build/lib/buildeventstream/BuildToolLogs.java:44 BuildToolLogs.<init>
    pub fn new(
        direct_values: Vec<(String, Vec<u8>)>,
        future_uris: Vec<(String, Option<String>)>,
        log_files: Vec<LogFileEntry>,
    ) -> Self {
        BuildToolLogs {
            direct_values,
            future_uris,
            log_files,
        }
    }
}

impl BuildEvent for BuildToolLogs {
    // Java: com/google/devtools/build/lib/buildeventstream/BuildToolLogs.java:54 BuildToolLogs.getEventId
    fn get_event_id(&self) -> BuildEventId {
        build_event_id_util::build_tool_logs()
    }

    // Java: com/google/devtools/build/lib/buildeventstream/BuildToolLogs.java:59 BuildToolLogs.getChildrenEvents
    fn get_children_events(&self) -> Vec<BuildEventId> {
        Vec::new()
    }

    // Java: com/google/devtools/build/lib/buildeventstream/BuildToolLogs.java:64 BuildToolLogs.referencedLocalFiles
    fn referenced_local_files(&self) -> Vec<LocalFile> {
        self.log_files
            .iter()
            .map(|l| l.local_file.clone())
            .collect()
    }

    // Java: com/google/devtools/build/lib/buildeventstream/BuildToolLogs.java:69 BuildToolLogs.remoteUploads
    fn remote_uploads(&self) -> Vec<Option<String>> {
        self.future_uris
            .iter()
            .map(|(_, uri)| uri.clone())
            .collect()
    }

    // Java: com/google/devtools/build/lib/buildeventstream/BuildToolLogs.java:74 BuildToolLogs.asStreamProto
    fn as_stream_proto(
        &self,
        converters: &dyn BuildEventContext,
    ) -> Result<bes::BuildEvent, Interrupted> {
        let mut tool_logs = bes::BuildToolLogs::default();
        for (name, contents) in &self.direct_values {
            tool_logs.log.push(bes::File {
                name: name.clone(),
                file: Some(bes::file::File::Contents(contents.clone())),
                ..Default::default()
            });
        }
        for (name, uri) in &self.future_uris {
            // Dropped unfinished uploads are only logged in Java.
            if let Some(uri) = uri {
                tool_logs.log.push(bes::File {
                    name: name.clone(),
                    file: Some(bes::file::File::Uri(uri.clone())),
                    ..Default::default()
                });
            }
        }
        for log_file in &self.log_files {
            if let Some(uri) = converters.path_converter().apply(&log_file.local_file.path) {
                tool_logs.log.push(bes::File {
                    name: log_file.name.clone(),
                    file: Some(bes::file::File::Uri(uri)),
                    ..Default::default()
                });
            }
        }
        Ok(proto_chaining_with(self, Payload::BuildToolLogs(tool_logs)))
    }

    // Java: com/google/devtools/build/lib/buildeventstream/BuildToolLogs.java:113 BuildToolLogs.postedAfter
    fn posted_after(&self) -> Option<Vec<BuildEventId>> {
        Some(vec![build_event_id_util::build_finished()])
    }
}

/// An event describing a remote fetch.
// Java: com/google/devtools/build/lib/buildeventstream/FetchEvent.java:30 FetchEvent
#[derive(Clone, Debug)]
pub struct FetchEvent {
    url: String,
    downloader: Downloader,
    success: bool,
}

impl FetchEvent {
    // Java: com/google/devtools/build/lib/buildeventstream/FetchEvent.java:29 FetchEvent.<init>
    pub fn new(url: String, downloader: Downloader, success: bool) -> Self {
        FetchEvent {
            url,
            downloader,
            success,
        }
    }
}

impl BuildEvent for FetchEvent {
    // Java: com/google/devtools/build/lib/buildeventstream/FetchEvent.java:34 FetchEvent.getEventId
    fn get_event_id(&self) -> BuildEventId {
        build_event_id_util::fetch_id(&self.url, self.downloader)
    }
    // Java: com/google/devtools/build/lib/buildeventstream/FetchEvent.java:39 FetchEvent.getChildrenEvents
    fn get_children_events(&self) -> Vec<BuildEventId> {
        Vec::new()
    }
    // Java: com/google/devtools/build/lib/buildeventstream/FetchEvent.java:44 FetchEvent.asStreamProto
    fn as_stream_proto(&self, _: &dyn BuildEventContext) -> Result<bes::BuildEvent, Interrupted> {
        Ok(proto_chaining_with(
            self,
            Payload::Fetch(bes::Fetch {
                success: self.success,
            }),
        ))
    }
}
