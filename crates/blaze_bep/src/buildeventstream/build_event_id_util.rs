//! Port of `BuildEventIdUtil`: utilities for creating `BuildEventId`s.

use blaze_cmdline::label::Label;
use blaze_protos::build_event_stream::BuildEventId;
use blaze_protos::build_event_stream::build_event_id::{
    self as id, ActionCompletedId, ConfigurationId, Id,
};
use blaze_vfs::PathFragment;

fn of(i: Id) -> BuildEventId {
    BuildEventId { id: Some(i) }
}

// Java: com/google/devtools/build/lib/buildeventstream/BuildEventIdUtil.java:44 BuildEventIdUtil.unknownBuildEventId
pub fn unknown_build_event_id(details: &str) -> BuildEventId {
    of(Id::Unknown(id::UnknownBuildEventId {
        details: details.to_string(),
    }))
}

// Java: com/google/devtools/build/lib/buildeventstream/BuildEventIdUtil.java:50 BuildEventIdUtil.progressId
pub fn progress_id(count: i32) -> BuildEventId {
    of(Id::Progress(id::ProgressId {
        opaque_count: count,
    }))
}

// Java: com/google/devtools/build/lib/buildeventstream/BuildEventIdUtil.java:55 BuildEventIdUtil.buildStartedId
pub fn build_started_id() -> BuildEventId {
    of(Id::Started(id::BuildStartedId {}))
}

// Java: com/google/devtools/build/lib/buildeventstream/BuildEventIdUtil.java:60 BuildEventIdUtil.unstructuredCommandlineId
pub fn unstructured_commandline_id() -> BuildEventId {
    of(Id::UnstructuredCommandLine(
        id::UnstructuredCommandLineId {},
    ))
}

// Java: com/google/devtools/build/lib/buildeventstream/BuildEventIdUtil.java:66 BuildEventIdUtil.structuredCommandlineId
pub fn structured_commandline_id(command_line_label: &str) -> BuildEventId {
    of(Id::StructuredCommandLine(id::StructuredCommandLineId {
        command_line_label: command_line_label.to_string(),
    }))
}

// Java: com/google/devtools/build/lib/buildeventstream/BuildEventIdUtil.java:74 BuildEventIdUtil.optionsParsedId
pub fn options_parsed_id() -> BuildEventId {
    of(Id::OptionsParsed(id::OptionsParsedId {}))
}

// Java: com/google/devtools/build/lib/buildeventstream/BuildEventIdUtil.java:80 BuildEventIdUtil.workspaceStatusId
pub fn workspace_status_id() -> BuildEventId {
    of(Id::WorkspaceStatus(id::WorkspaceStatusId {}))
}

// Java: com/google/devtools/build/lib/buildeventstream/BuildEventIdUtil.java:86 BuildEventIdUtil.buildMetadataId
pub fn build_metadata_id() -> BuildEventId {
    of(Id::BuildMetadata(id::BuildMetadataId {}))
}

// Java: com/google/devtools/build/lib/buildeventstream/BuildEventIdUtil.java:92 BuildEventIdUtil.workspaceConfigId
pub fn workspace_config_id() -> BuildEventId {
    of(Id::Workspace(id::WorkspaceConfigId {}))
}

// Java: com/google/devtools/build/lib/buildeventstream/BuildEventIdUtil.java:98 BuildEventIdUtil.fetchId
pub fn fetch_id(url: &str, downloader: id::fetch_id::Downloader) -> BuildEventId {
    of(Id::Fetch(id::FetchId {
        url: url.to_string(),
        downloader: downloader as i32,
    }))
}

/// `configurationId(BuildConfigurationKey)`: `key == null` is the null configuration;
/// otherwise the id is `key.getOptions().checksum()`.
// Java: com/google/devtools/build/lib/buildeventstream/BuildEventIdUtil.java:40 BuildEventIdUtil.configurationId
pub fn configuration_id_for_key(checksum: Option<&str>) -> BuildEventId {
    configuration_id_of(configuration_id_message_for_key(checksum))
}

// Java: com/google/devtools/build/lib/buildeventstream/BuildEventIdUtil.java:40 BuildEventIdUtil.configurationId
fn configuration_id_of(id: ConfigurationId) -> BuildEventId {
    of(Id::Configuration(id))
}

// Java: com/google/devtools/build/lib/buildeventstream/BuildEventIdUtil.java:40 BuildEventIdUtil.configurationId
pub fn configuration_id(id: &str) -> BuildEventId {
    configuration_id_of(configuration_id_message(id))
}

// Java: com/google/devtools/build/lib/buildeventstream/BuildEventIdUtil.java:38 BuildEventIdUtil.configurationIdMessage
pub fn configuration_id_message_for_key(checksum: Option<&str>) -> ConfigurationId {
    match checksum {
        None => null_configuration_id_message(),
        Some(checksum) => configuration_id_message(checksum),
    }
}

// Java: com/google/devtools/build/lib/buildeventstream/BuildEventIdUtil.java:38 BuildEventIdUtil.configurationIdMessage
pub fn configuration_id_message(checksum: &str) -> ConfigurationId {
    ConfigurationId {
        id: checksum.to_string(),
    }
}

// Java: com/google/devtools/build/lib/buildeventstream/BuildEventIdUtil.java:126 BuildEventIdUtil.execRequestId
pub fn exec_request_id() -> BuildEventId {
    of(Id::ExecRequest(id::ExecRequestId {}))
}

// Java: com/google/devtools/build/lib/buildeventstream/BuildEventIdUtil.java:132 BuildEventIdUtil.nullConfigurationId
pub fn null_configuration_id() -> BuildEventId {
    configuration_id_of(null_configuration_id_message())
}

// Java: com/google/devtools/build/lib/buildeventstream/BuildEventIdUtil.java:118 BuildEventIdUtil.nullConfigurationIdMessage
pub fn null_configuration_id_message() -> ConfigurationId {
    // Java: NULL_CONFIGURATION_ID_MESSAGE = configurationIdMessage("none")
    configuration_id_message("none")
}

// Java: com/google/devtools/build/lib/buildeventstream/BuildEventIdUtil.java:140 BuildEventIdUtil.targetPatternExpanded
fn target_pattern_expanded_impl(target_pattern: &[String], skipped: bool) -> BuildEventId {
    let pattern_id = id::PatternExpandedId {
        pattern: target_pattern.to_vec(),
    };
    if skipped {
        of(Id::PatternSkipped(pattern_id))
    } else {
        of(Id::Pattern(pattern_id))
    }
}

// Java: com/google/devtools/build/lib/buildeventstream/BuildEventIdUtil.java:140 BuildEventIdUtil.targetPatternExpanded
pub fn target_pattern_expanded(target_pattern: &[String]) -> BuildEventId {
    target_pattern_expanded_impl(target_pattern, false)
}

// Java: com/google/devtools/build/lib/buildeventstream/BuildEventIdUtil.java:156 BuildEventIdUtil.targetPatternSkipped
pub fn target_pattern_skipped(target_pattern: &[String]) -> BuildEventId {
    target_pattern_expanded_impl(target_pattern, true)
}

// Java: com/google/devtools/build/lib/buildeventstream/BuildEventIdUtil.java:160 BuildEventIdUtil.targetConfigured
pub fn target_configured(label: &Label) -> BuildEventId {
    of(Id::TargetConfigured(id::TargetConfiguredId {
        label: label.to_string(),
        aspect: String::new(),
    }))
}

// Java: com/google/devtools/build/lib/buildeventstream/BuildEventIdUtil.java:166 BuildEventIdUtil.aspectConfigured
pub fn aspect_configured(label: &Label, aspect: &str) -> BuildEventId {
    of(Id::TargetConfigured(id::TargetConfiguredId {
        label: label.to_string(),
        aspect: aspect.to_string(),
    }))
}

fn configuration_of(configuration: &BuildEventId) -> Option<ConfigurationId> {
    // Java: configuration.getConfiguration() returns the default instance when unset.
    Some(match &configuration.id {
        Some(Id::Configuration(c)) => c.clone(),
        _ => ConfigurationId::default(),
    })
}

// Java: com/google/devtools/build/lib/buildeventstream/BuildEventIdUtil.java:175 BuildEventIdUtil.targetCompleted
pub fn target_completed(target: &Label, configuration: &BuildEventId) -> BuildEventId {
    of(Id::TargetCompleted(id::TargetCompletedId {
        label: target.to_string(),
        configuration: configuration_of(configuration),
        aspect: String::new(),
    }))
}

// Java: com/google/devtools/build/lib/buildeventstream/BuildEventIdUtil.java:185 BuildEventIdUtil.configuredLabelId
pub fn configured_label_id(label: &Label, configuration_id: ConfigurationId) -> BuildEventId {
    of(Id::ConfiguredLabel(id::ConfiguredLabelId {
        label: label.to_string(),
        configuration: Some(configuration_id),
    }))
}

// Java: com/google/devtools/build/lib/buildeventstream/BuildEventIdUtil.java:195 BuildEventIdUtil.unconfiguredLabelId
pub fn unconfigured_label_id(label: &Label) -> BuildEventId {
    of(Id::UnconfiguredLabel(id::UnconfiguredLabelId {
        label: label.to_string(),
    }))
}

// Java: com/google/devtools/build/lib/buildeventstream/BuildEventIdUtil.java:201 BuildEventIdUtil.aspectCompleted
pub fn aspect_completed(
    target: &Label,
    configuration: &BuildEventId,
    aspect: &str,
) -> BuildEventId {
    of(Id::TargetCompleted(id::TargetCompletedId {
        label: target.to_string(),
        configuration: configuration_of(configuration),
        aspect: aspect.to_string(),
    }))
}

// Java: com/google/devtools/build/lib/buildeventstream/BuildEventIdUtil.java:213 BuildEventIdUtil.actionCompleted
pub fn action_completed(path: &PathFragment) -> BuildEventId {
    action_completed_with(path, None, None)
}

// Java: com/google/devtools/build/lib/buildeventstream/BuildEventIdUtil.java:213 BuildEventIdUtil.actionCompleted
pub fn action_completed_with(
    path: &PathFragment,
    label: Option<&Label>,
    configuration_checksum: Option<&str>,
) -> BuildEventId {
    let mut action_id = ActionCompletedId {
        primary_output: path.to_string(),
        ..Default::default()
    };
    if let Some(label) = label {
        action_id.label = label.to_string();
    }
    if let Some(checksum) = configuration_checksum {
        action_id.configuration = Some(ConfigurationId {
            id: checksum.to_string(),
        });
    }
    of(Id::ActionCompleted(action_id))
}

// Java: com/google/devtools/build/lib/buildeventstream/BuildEventIdUtil.java:230 BuildEventIdUtil.fromArtifactGroupName
pub fn from_artifact_group_name(name: &str) -> BuildEventId {
    of(Id::NamedSet(id::NamedSetOfFilesId {
        id: name.to_string(),
    }))
}

/// `testResult(target, run, shard, attempt, configuration)`; `target` is the label string.
// Java: com/google/devtools/build/lib/buildeventstream/BuildEventIdUtil.java:236 BuildEventIdUtil.testResult
pub fn test_result(
    target: &str,
    run: i32,
    shard: i32,
    attempt: i32,
    configuration: &BuildEventId,
) -> BuildEventId {
    of(Id::TestResult(id::TestResultId {
        label: target.to_string(),
        configuration: configuration_of(configuration),
        run: run + 1,
        shard: shard + 1,
        attempt,
    }))
}

// Java: com/google/devtools/build/lib/buildeventstream/BuildEventIdUtil.java:236 BuildEventIdUtil.testResult
pub fn test_result_first_attempt(
    target: &str,
    run: i32,
    shard: i32,
    configuration: &BuildEventId,
) -> BuildEventId {
    test_result(target, run, shard, 1, configuration)
}

// Java: com/google/devtools/build/lib/buildeventstream/BuildEventIdUtil.java:255 BuildEventIdUtil.testProgressId
pub fn test_progress_id(
    label: &str,
    config_id: ConfigurationId,
    run: i32,
    shard: i32,
    attempt: i32,
    opaque_count: i32,
) -> BuildEventId {
    of(Id::TestProgress(id::TestProgressId {
        label: label.to_string(),
        configuration: Some(config_id),
        run,
        shard,
        attempt,
        opaque_count,
    }))
}

/// `testSummary(target, configuration)`; `target` is the label string.
// Java: com/google/devtools/build/lib/buildeventstream/BuildEventIdUtil.java:274 BuildEventIdUtil.testSummary
pub fn test_summary(target: &str, configuration: &BuildEventId) -> BuildEventId {
    of(Id::TestSummary(id::TestSummaryId {
        label: target.to_string(),
        configuration: configuration_of(configuration),
    }))
}

/// `targetSummary(target, configuration)`; `target` is the label string.
// Java: com/google/devtools/build/lib/buildeventstream/BuildEventIdUtil.java:284 BuildEventIdUtil.targetSummary
pub fn target_summary(target: &str, configuration: &BuildEventId) -> BuildEventId {
    of(Id::TargetSummary(id::TargetSummaryId {
        label: target.to_string(),
        configuration: configuration_of(configuration),
    }))
}

// Java: com/google/devtools/build/lib/buildeventstream/BuildEventIdUtil.java:294 BuildEventIdUtil.buildFinished
pub fn build_finished() -> BuildEventId {
    of(Id::BuildFinished(id::BuildFinishedId {}))
}

// Java: com/google/devtools/build/lib/buildeventstream/BuildEventIdUtil.java:299 BuildEventIdUtil.buildToolLogs
pub fn build_tool_logs() -> BuildEventId {
    of(Id::BuildToolLogs(id::BuildToolLogsId {}))
}

// Java: com/google/devtools/build/lib/buildeventstream/BuildEventIdUtil.java:305 BuildEventIdUtil.buildMetrics
pub fn build_metrics() -> BuildEventId {
    of(Id::BuildMetrics(id::BuildMetricsId {}))
}

// Java: com/google/devtools/build/lib/buildeventstream/BuildEventIdUtil.java:311 BuildEventIdUtil.convenienceSymlinksIdentifiedId
pub fn convenience_symlinks_identified_id() -> BuildEventId {
    of(Id::ConvenienceSymlinksIdentified(
        id::ConvenienceSymlinksIdentifiedId {},
    ))
}
