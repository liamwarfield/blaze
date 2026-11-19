//! Ports of `TraceData.java` and `Profiler.TaskData` / `Profiler.ActionTaskData`.

use std::io::{self, Write};

use super::counter_series::nanos_to_micros;
use super::json_writer::JsonWriter;
use super::profiler_task::ProfilerTask;
use super::thread_metadata::CRITICAL_PATH_THREAD_ID;

/// Encapsulates data to be written to the JSON trace profile.
// Java: com/google/devtools/build/lib/profiler/TraceData.java:19 TraceData
pub trait TraceData: Send + Sync {
    fn write_trace_data(
        &self,
        json_writer: &mut JsonWriter<&mut dyn Write>,
        profile_start_time_nanos: i64,
    ) -> io::Result<()>;
}

/// The action-specific fields of `Profiler.ActionTaskData`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActionFields {
    pub primary_output_path: Option<String>,
    pub target_label: Option<String>,
    pub mnemonic: Option<String>,
    pub configuration: Option<String>,
}

/// Container for the single task record (`Profiler.TaskData`), with the extra fields of its
/// subclass `ActionTaskData` when it describes an action.
// Java: com/google/devtools/build/lib/profiler/Profiler.java:167 Profiler.TaskData
// Java: com/google/devtools/build/lib/profiler/Profiler.java:270 Profiler.ActionTaskData
#[derive(Clone, Debug, PartialEq)]
pub struct TaskData {
    pub thread_id: i64,
    pub start_time_nanos: i64,
    /// `None` only for the writer's poison pill.
    pub type_: Option<ProfilerTask>,
    pub description: String,
    pub duration_nanos: i64,
    pub action: Option<Box<ActionFields>>,
}

impl TaskData {
    // Java: com/google/devtools/build/lib/profiler/Profiler.java:175 Profiler.TaskData.<init>
    pub fn new(
        thread_id: i64,
        start_time_nanos: i64,
        duration_nanos: i64,
        event_type: Option<ProfilerTask>,
        description: String,
    ) -> Self {
        TaskData {
            thread_id,
            start_time_nanos,
            type_: event_type,
            description,
            duration_nanos,
            action: None,
        }
    }

    /// `TaskData(threadId, startTimeNanos, eventType, description)`: duration -1.
    // Java: com/google/devtools/build/lib/profiler/Profiler.java:188 Profiler.TaskData.<init>
    pub fn new_open(
        thread_id: i64,
        start_time_nanos: i64,
        event_type: Option<ProfilerTask>,
        description: String,
    ) -> Self {
        Self::new(thread_id, start_time_nanos, -1, event_type, description)
    }

    /// `TaskData(threadId, startTimeNanos, durationNanos, description)`: type UNKNOWN.
    // Java: com/google/devtools/build/lib/profiler/Profiler.java:192 Profiler.TaskData.<init>
    pub fn new_unknown(
        thread_id: i64,
        start_time_nanos: i64,
        duration_nanos: i64,
        description: String,
    ) -> Self {
        Self::new(
            thread_id,
            start_time_nanos,
            duration_nanos,
            Some(ProfilerTask::UNKNOWN),
            description,
        )
    }

    // Java: com/google/devtools/build/lib/profiler/Profiler.java:276 Profiler.ActionTaskData.<init>
    #[allow(clippy::too_many_arguments)]
    pub fn new_action(
        thread_id: i64,
        start_time_nanos: i64,
        duration_nanos: i64,
        event_type: ProfilerTask,
        mnemonic: Option<String>,
        description: String,
        primary_output_path: Option<String>,
        target_label: Option<String>,
        configuration: Option<String>,
    ) -> Self {
        TaskData {
            action: Some(Box::new(ActionFields {
                primary_output_path,
                target_label,
                mnemonic,
                configuration,
            })),
            ..Self::new(
                thread_id,
                start_time_nanos,
                duration_nanos,
                Some(event_type),
                description,
            )
        }
    }
}

impl std::fmt::Display for TaskData {
    // Java: com/google/devtools/build/lib/profiler/Profiler.java:201 Profiler.TaskData.toString
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Thread {}, type {}, {}",
            self.thread_id,
            self.type_.map_or("null", |t| t.name()),
            self.description
        )
    }
}

impl TraceData for TaskData {
    // Java: com/google/devtools/build/lib/profiler/Profiler.java:206 Profiler.TaskData.writeTraceData
    fn write_trace_data(
        &self,
        json_writer: &mut JsonWriter<&mut dyn Write>,
        profile_start_time_nanos: i64,
    ) -> io::Result<()> {
        let event_type = if self.duration_nanos == 0 { "i" } else { "X" };
        json_writer.set_indent("  ");
        json_writer.begin_object()?;
        json_writer.set_indent("");
        match self.type_ {
            None => json_writer.set_indent("    "),
            Some(t) => {
                json_writer.name("cat")?.value_str(t.description())?;
            }
        }
        json_writer.name("name")?.value_str(&self.description)?;
        json_writer.name("ph")?.value_str(event_type)?;
        json_writer
            .name("ts")?
            .value_long(nanos_to_micros(self.start_time_nanos - profile_start_time_nanos))?;
        if self.duration_nanos != 0 {
            json_writer
                .name("dur")?
                .value_long(nanos_to_micros(self.duration_nanos))?;
        }
        json_writer.name("pid")?.value_long(1)?;

        if let Some(action_task_data) = &self.action {
            if let Some(primary_output_path) = &action_task_data.primary_output_path {
                // Primary outputs are non-mergeable, thus incompatible with slim profiles.
                json_writer.name("out")?.value_str(primary_output_path)?;
            }
            if action_task_data.target_label.is_some()
                || action_task_data.mnemonic.is_some()
                || action_task_data.configuration.is_some()
            {
                json_writer.name("args")?;
                json_writer.begin_object()?;
                if let Some(target_label) = &action_task_data.target_label {
                    json_writer.name("target")?.value_str(target_label)?;
                }
                if let Some(mnemonic) = &action_task_data.mnemonic {
                    json_writer.name("mnemonic")?.value_str(mnemonic)?;
                }
                if let Some(configuration) = &action_task_data.configuration {
                    json_writer.name("configuration")?.value_str(configuration)?;
                }
                json_writer.end_object()?;
            }
        }
        if self.type_ == Some(ProfilerTask::CRITICAL_PATH_COMPONENT) {
            json_writer.name("args")?;
            json_writer.begin_object()?;
            json_writer.name("tid")?.value_long(self.thread_id)?;
            json_writer.end_object()?;
        }
        json_writer
            .name("tid")?
            .value_long(if self.type_ == Some(ProfilerTask::CRITICAL_PATH_COMPONENT) {
                CRITICAL_PATH_THREAD_ID
            } else {
                self.thread_id
            })?;
        json_writer.end_object()?;
        Ok(())
    }
}
