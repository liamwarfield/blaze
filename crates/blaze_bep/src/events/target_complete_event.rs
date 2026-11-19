//! Port of `TargetCompleteEvent` (with `ArtifactsInOutputGroup` and the `Cause` root causes it
//! carries).

use std::sync::Arc;

use blaze_actions::{Artifact, ArtifactRoot};
use blaze_cmdline::label::Label;
use blaze_collect::nestedset::NestedSet;
use blaze_exec::actions::file_artifact_value::FileArtifactValue;
use blaze_optdefs::misc::build_event_context::OutputGroupFileMode;
use blaze_optdefs::misc::build_event_protocol_options::OutputGroupFileModes;
use blaze_protos::build_event_stream::build_event::Payload;
use blaze_protos::build_event_stream::{self as bes, BuildEventId, OutputGroup};
use blaze_util::DetailedExitCode;
use blaze_util::string_encoding::internal_to_unicode;
use blaze_vfs::PathFragment;
use indexmap::IndexMap;

use crate::buildeventstream::build_event::{
    BuildEvent, Interrupted, LocalFile, LocalFileType, ReportedArtifacts,
};
use crate::buildeventstream::build_event_context::{ArtifactGroupNode, BuildEventContext};
use crate::buildeventstream::build_event_id_util;
use crate::buildeventstream::generic_build_event::proto_chaining_with;
use crate::events::completion_context::{ArtifactReceiver, CompletionContext};
use crate::util::duration_from_seconds;

/// The artifacts of one output group, and whether they are important and complete.
// Java: com/google/devtools/build/lib/analysis/TopLevelArtifactHelper.java:51 TopLevelArtifactHelper.ArtifactsInOutputGroup
#[derive(Clone, Debug)]
pub struct ArtifactsInOutputGroup {
    artifacts: NestedSet<Artifact>,
    important: bool,
    incomplete: bool,
}

impl ArtifactsInOutputGroup {
    pub fn new(artifacts: NestedSet<Artifact>, important: bool, incomplete: bool) -> Self {
        ArtifactsInOutputGroup {
            artifacts,
            important,
            incomplete,
        }
    }

    pub fn get_artifacts(&self) -> &NestedSet<Artifact> {
        &self.artifacts
    }

    /// Returns `true` if the user should know about this output group.
    pub fn are_important(&self) -> bool {
        self.important
    }

    pub fn is_incomplete(&self) -> bool {
        self.incomplete
    }
}

/// A root cause of a failed target (`Cause`): its id in the stream and its exit code.
// Java: com/google/devtools/build/lib/causes/Cause.java:21 Cause
#[derive(Clone, Debug)]
pub struct Cause {
    /// `getIdProto()`.
    pub id_proto: BuildEventId,
    /// `getDetailedExitCode()`.
    pub detailed_exit_code: DetailedExitCode,
}

/// What `TargetCompleteEvent` reads from `ConfiguredTargetAndData`.
#[derive(Clone, Debug)]
pub struct CompletedTargetData {
    /// `getConfiguredTarget().getLabel()`.
    pub label: Label,
    /// `getConfiguredTarget().getOriginalLabel()`.
    pub original_label: Label,
    /// `BuildConfigurationValue.configurationId(configuration)`: the checksum, `None` for the
    /// null configuration.
    pub configuration_checksum: Option<String>,
    /// `configuration.toBuildEvent()` (`None` for the null configuration).
    pub configuration_event: Option<Arc<dyn BuildEvent>>,
    /// `getRuleTags()`: the tags of the taggable attributes, in attribute order.
    pub rule_tags: Vec<String>,
    /// For tests: `TestProvider.getTestParams()` runs and shards.
    pub test_runs: i32,
    pub test_shards: i32,
    /// For tests: `TestConfiguration.getTestTimeout().get(categoricalTimeout).toSeconds()`.
    pub test_timeout_seconds: Option<i64>,
}

/// This event is fired as soon as a top-level target is completed.
// Java: com/google/devtools/build/lib/analysis/TargetCompleteEvent.java:76 TargetCompleteEvent
#[derive(Debug)]
pub struct TargetCompleteEvent {
    label: Label,
    root_causes: Vec<Cause>,
    posted_after: Vec<BuildEventId>,
    completion_context: Arc<CompletionContext>,
    outputs: IndexMap<String, ArtifactsInOutputGroup>,
    original_label: Label,
    is_test: bool,
    announce_target_summary: bool,
    test_timeout_seconds: Option<i64>,
    test_runs: i32,
    test_shards: i32,
    configuration_event: Option<Arc<dyn BuildEvent>>,
    config_event_id: BuildEventId,
    tags: Vec<String>,
    detailed_exit_code: Option<DetailedExitCode>,
}

impl TargetCompleteEvent {
    // Java: com/google/devtools/build/lib/analysis/TargetCompleteEvent.java:133 TargetCompleteEvent.<init>
    fn new(
        target_and_data: CompletedTargetData,
        root_causes: Vec<Cause>,
        completion_context: Arc<CompletionContext>,
        outputs: IndexMap<String, ArtifactsInOutputGroup>,
        is_test: bool,
        announce_target_summary: bool,
    ) -> Self {
        let mut posted_after = vec![build_event_id_util::target_configured(
            &target_and_data.original_label,
        )];
        let mut most_important_detailed_exit_code: Option<DetailedExitCode> = None;
        for cause in &root_causes {
            most_important_detailed_exit_code =
                DetailedExitCode::choose_more_important_with_first_if_tie(
                    most_important_detailed_exit_code,
                    Some(cause.detailed_exit_code.clone()),
                );
            posted_after.push(cause.id_proto.clone());
        }
        let config_event_id = build_event_id_util::configuration_id_for_key(
            target_and_data.configuration_checksum.as_deref(),
        );
        TargetCompleteEvent {
            label: target_and_data.label,
            root_causes,
            posted_after,
            completion_context,
            outputs,
            original_label: target_and_data.original_label,
            is_test,
            announce_target_summary,
            test_timeout_seconds: if is_test {
                target_and_data.test_timeout_seconds
            } else {
                None
            },
            test_runs: target_and_data.test_runs,
            test_shards: target_and_data.test_shards,
            configuration_event: target_and_data.configuration_event,
            config_event_id,
            tags: target_and_data.rule_tags,
            detailed_exit_code: most_important_detailed_exit_code,
        }
    }

    /// Construct a successful target completion event.
    // Java: com/google/devtools/build/lib/analysis/TargetCompleteEvent.java:175 TargetCompleteEvent.successfulBuild
    pub fn successful_build(
        ct: CompletedTargetData,
        completion_context: Arc<CompletionContext>,
        outputs: IndexMap<String, ArtifactsInOutputGroup>,
        announce_target_summary: bool,
    ) -> Self {
        Self::new(
            ct,
            Vec::new(),
            completion_context,
            outputs,
            false,
            announce_target_summary,
        )
    }

    /// Construct a successful target completion event for a target that will be tested.
    // Java: com/google/devtools/build/lib/analysis/TargetCompleteEvent.java:185 TargetCompleteEvent.successfulBuildSchedulingTest
    pub fn successful_build_scheduling_test(
        ct: CompletedTargetData,
        completion_context: Arc<CompletionContext>,
        outputs: IndexMap<String, ArtifactsInOutputGroup>,
        announce_target_summary: bool,
    ) -> Self {
        Self::new(
            ct,
            Vec::new(),
            completion_context,
            outputs,
            true,
            announce_target_summary,
        )
    }

    /// Construct a target completion event for a failed target, with the given non-empty root
    /// causes.
    // Java: com/google/devtools/build/lib/analysis/TargetCompleteEvent.java:197 TargetCompleteEvent.createFailed
    pub fn create_failed(
        ct: CompletedTargetData,
        completion_context: Arc<CompletionContext>,
        root_causes: Vec<Cause>,
        outputs: IndexMap<String, ArtifactsInOutputGroup>,
        announce_target_summary: bool,
    ) -> Self {
        blaze_util::check_argument!(!root_causes.is_empty());
        Self::new(
            ct,
            root_causes,
            completion_context,
            outputs,
            false,
            announce_target_summary,
        )
    }

    /// Returns the label of the target associated with the event.
    // Java: com/google/devtools/build/lib/analysis/TargetCompleteEvent.java:209 TargetCompleteEvent.getLabel
    pub fn get_label(&self) -> &Label {
        &self.label
    }

    // Java: com/google/devtools/build/lib/analysis/TargetCompleteEvent.java:218 TargetCompleteEvent.getOriginalLabel
    pub fn get_original_label(&self) -> &Label {
        &self.original_label
    }

    /// Determines whether the target has failed or succeeded.
    // Java: com/google/devtools/build/lib/analysis/TargetCompleteEvent.java:231 TargetCompleteEvent.failed
    pub fn failed(&self) -> bool {
        !self.root_causes.is_empty()
    }

    /// Get the root causes of the target. May be empty.
    // Java: com/google/devtools/build/lib/analysis/TargetCompleteEvent.java:236 TargetCompleteEvent.getRootCauses
    pub fn get_root_causes(&self) -> &[Cause] {
        &self.root_causes
    }

    // Java: com/google/devtools/build/lib/analysis/TargetCompleteEvent.java:240 TargetCompleteEvent.getLegacyFilteredImportantArtifacts
    pub fn get_legacy_filtered_important_artifacts(&self) -> Vec<Artifact> {
        let mut builder = NestedSet::builder(blaze_collect::nestedset::Order::STABLE_ORDER);
        for artifacts_in_output_group in self.outputs.values() {
            if artifacts_in_output_group.are_important() {
                builder.add_transitive(artifacts_in_output_group.get_artifacts());
            }
        }
        builder
            .build()
            .to_list()
            .iter()
            .filter(|artifact| !artifact.is_source_artifact() && !artifact.is_runfiles_tree())
            .cloned()
            .collect()
    }

    // Java: com/google/devtools/build/lib/analysis/TargetCompleteEvent.java:281 TargetCompleteEvent.getCompletionContext
    pub fn get_completion_context(&self) -> &Arc<CompletionContext> {
        &self.completion_context
    }

    // Java: com/google/devtools/build/lib/analysis/TargetCompleteEvent.java:286 TargetCompleteEvent.getOutputGroup
    pub fn get_output_group(&self, output_group: &str) -> Option<&ArtifactsInOutputGroup> {
        self.outputs.get(output_group)
    }

    // Java: com/google/devtools/build/lib/analysis/TargetCompleteEvent.java:482 TargetCompleteEvent.toReportedArtifacts
    pub fn to_reported_artifacts(
        outputs: &IndexMap<String, ArtifactsInOutputGroup>,
        completion_context: &Arc<CompletionContext>,
        output_group_file_modes: &OutputGroupFileModes,
    ) -> ReportedArtifacts {
        let mut builder: Vec<NestedSet<Artifact>> = Vec::new();
        for (group_name, artifacts_in_group) in outputs {
            let mode = output_group_file_modes.get_mode(group_name);
            if artifacts_in_group.are_important()
                && (mode == OutputGroupFileMode::NAMED_SET_OF_FILES_ONLY
                    || mode == OutputGroupFileMode::BOTH)
            {
                builder.push(artifacts_in_group.get_artifacts().clone());
            }
        }
        ReportedArtifacts::new(builder, completion_context.clone())
    }

    /// Returns `OutputGroup` protos for given output groups.
    // Java: com/google/devtools/build/lib/analysis/TargetCompleteEvent.java:511 TargetCompleteEvent.toOutputGroupProtos
    pub fn to_output_group_protos(
        outputs: &IndexMap<String, ArtifactsInOutputGroup>,
        completion_context: &CompletionContext,
        converters: &dyn BuildEventContext,
    ) -> Vec<OutputGroup> {
        let mut groups = Vec::new();
        for (output_group, artifacts_in_output_group) in outputs {
            if !artifacts_in_output_group.are_important() {
                continue;
            }
            let artifacts = artifacts_in_output_group.get_artifacts();
            groups.push(make_output_group_proto(
                completion_context,
                converters,
                output_group,
                artifacts_in_output_group.is_incomplete(),
                artifacts,
            ));
        }
        groups
    }
}

/// Constructs an `OutputGroup` message based on how the group has been configured to report
/// its artifacts on the command-line.
// Java: com/google/devtools/build/lib/analysis/TargetCompleteEvent.java:523 TargetCompleteEvent.makeOutputGroupProto
fn make_output_group_proto(
    completion_context: &CompletionContext,
    converters: &dyn BuildEventContext,
    output_group: &str,
    output_group_incomplete: bool,
    artifacts_to_report: &NestedSet<Artifact>,
) -> OutputGroup {
    let mut builder = OutputGroup {
        name: output_group.to_string(),
        incomplete: output_group_incomplete,
        ..Default::default()
    };
    let file_mode = converters.get_file_mode_for_output_group(output_group);
    if file_mode == OutputGroupFileMode::NAMED_SET_OF_FILES_ONLY
        || file_mode == OutputGroupFileMode::BOTH
    {
        let namer = converters.artifact_group_namer();
        builder.file_sets.push(
            namer
                .apply(&ArtifactGroupNode::Artifacts(artifacts_to_report.to_node()))
                .expect("NullPointerException: unnamed output group"),
        );
    }
    if file_mode == OutputGroupFileMode::INLINE_ONLY || file_mode == OutputGroupFileMode::BOTH {
        let list = artifacts_to_report.to_list();
        add_files_directly_to_proto_field(
            completion_context,
            &mut builder.inline_files,
            converters,
            list.iter(),
        );
    }
    builder
}

// Java: com/google/devtools/build/lib/analysis/TargetCompleteEvent.java:293 TargetCompleteEvent.addFilesDirectlyToProtoField
fn add_files_directly_to_proto_field<'a>(
    completion_context: &CompletionContext,
    add_file: &mut Vec<bes::File>,
    converters: &dyn BuildEventContext,
    artifacts: impl Iterator<Item = &'a Artifact>,
) {
    struct Receiver<'b> {
        completion_context: &'b CompletionContext,
        converters: &'b dyn BuildEventContext,
        add_file: &'b mut Vec<bes::File>,
    }
    impl ArtifactReceiver for Receiver<'_> {
        fn accept(&mut self, artifact: &Artifact, metadata: &FileArtifactValue) {
            let uri = self
                .converters
                .path_converter()
                .apply(&self.completion_context.to_path(artifact));
            let file = new_file(artifact, metadata, uri);
            if file.file.is_some() {
                self.add_file.push(file);
            }
        }
        fn accept_fileset_mapping(&mut self, fileset: &Artifact) {
            panic!("IllegalStateException: {fileset} should have been filtered out");
        }
    }
    let filtered: Vec<&Artifact> = filter_filesets(artifacts).collect();
    completion_context.visit_artifacts(
        filtered,
        &mut Receiver {
            completion_context,
            converters,
            add_file,
        },
    );
}

// Java: com/google/devtools/build/lib/analysis/TargetCompleteEvent.java:308 TargetCompleteEvent.filterFilesets
fn filter_filesets<'a>(
    artifacts: impl Iterator<Item = &'a Artifact>,
) -> impl Iterator<Item = &'a Artifact> {
    artifacts.filter(|artifact| !artifact.is_fileset())
}

/// Creates a `File` proto for an artifact; `uri` is `None` if the artifact was not uploaded.
// Java: com/google/devtools/build/lib/analysis/TargetCompleteEvent.java:339 TargetCompleteEvent.newFile
pub fn new_file(
    artifact: &Artifact,
    metadata: &FileArtifactValue,
    uri: Option<String>,
) -> bes::File {
    new_file_for_path(
        artifact.get_root(),
        &artifact.get_root_relative_path(),
        metadata,
        uri,
    )
}

/// Creates a `File` proto for a path under a root.
// Java: com/google/devtools/build/lib/analysis/TargetCompleteEvent.java:339 TargetCompleteEvent.newFile
pub fn new_file_for_path(
    root: &ArtifactRoot,
    root_relative_path: &PathFragment,
    metadata: &FileArtifactValue,
    uri: Option<String>,
) -> bes::File {
    let mut file = bes::File {
        name: internal_to_unicode(root_relative_path.get_path_string()).into_owned(),
        path_prefix: root
            .get_exec_path()
            .segments()
            .map(|s| internal_to_unicode(s).into_owned())
            .collect(),
        ..Default::default()
    };
    if metadata.get_type().is_symlink() {
        file.file = Some(bes::file::File::SymlinkTargetPath(
            internal_to_unicode(
                metadata
                    .get_unresolved_symlink_target()
                    .expect("unresolved symlink target"),
            )
            .into_owned(),
        ));
    } else if metadata.get_type().exists() {
        if let Some(digest) = metadata.get_digest() {
            file.digest = to_lower_hex(digest);
        }
        file.length = metadata.get_size();
    }
    if let Some(uri) = uri {
        file.file = Some(bes::file::File::Uri(internal_to_unicode(&uri).into_owned()));
    }
    file
}

fn to_lower_hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        let _ = write!(s, "{b:02x}");
    }
    s
}

impl BuildEvent for TargetCompleteEvent {
    // Java: com/google/devtools/build/lib/analysis/TargetCompleteEvent.java:254 TargetCompleteEvent.getEventId
    fn get_event_id(&self) -> BuildEventId {
        build_event_id_util::target_completed(&self.original_label, &self.config_event_id)
    }

    // Java: com/google/devtools/build/lib/analysis/TargetCompleteEvent.java:259 TargetCompleteEvent.getChildrenEvents
    fn get_children_events(&self) -> Vec<BuildEventId> {
        let mut children_builder = Vec::new();
        for cause in &self.root_causes {
            children_builder.push(cause.id_proto.clone());
        }
        if self.is_test {
            let label = self.label.to_string();
            for run in 0..self.test_runs.max(1) {
                for shard in 0..self.test_shards.max(1) {
                    children_builder.push(build_event_id_util::test_result_first_attempt(
                        &label,
                        run,
                        shard,
                        &self.config_event_id,
                    ));
                }
            }
            children_builder.push(build_event_id_util::test_summary(
                &label,
                &self.config_event_id,
            ));
        }
        if self.announce_target_summary {
            children_builder.push(build_event_id_util::target_summary(
                &self.original_label.to_string(),
                &self.config_event_id,
            ));
        }
        children_builder
    }

    // Java: com/google/devtools/build/lib/analysis/TargetCompleteEvent.java:396 TargetCompleteEvent.referencedLocalFiles
    fn referenced_local_files(&self) -> Vec<LocalFile> {
        struct Receiver<'b> {
            completion_context: &'b CompletionContext,
            builder: &'b mut Vec<LocalFile>,
        }
        impl ArtifactReceiver for Receiver<'_> {
            fn accept(&mut self, artifact: &Artifact, metadata: &FileArtifactValue) {
                self.builder.push(LocalFile::new(
                    self.completion_context.to_path(artifact),
                    LocalFileType::for_artifact(artifact, Some(metadata)),
                    Some(metadata.clone()),
                ));
            }
            fn accept_fileset_mapping(&mut self, fileset: &Artifact) {
                panic!("IllegalStateException: {fileset} should have been filtered out");
            }
        }
        let mut builder = Vec::new();
        for group in self.outputs.values() {
            if group.are_important() {
                let list = group.get_artifacts().to_list();
                let filtered: Vec<&Artifact> = filter_filesets(list.iter()).collect();
                self.completion_context.visit_artifacts(
                    filtered,
                    &mut Receiver {
                        completion_context: &self.completion_context,
                        builder: &mut builder,
                    },
                );
            }
        }
        builder
    }

    // Java: com/google/devtools/build/lib/analysis/TargetCompleteEvent.java:423 TargetCompleteEvent.asStreamProto
    #[allow(deprecated)]
    fn as_stream_proto(
        &self,
        converters: &dyn BuildEventContext,
    ) -> Result<bes::BuildEvent, Interrupted> {
        let mut builder = bes::TargetComplete::default();

        let failed = self.failed();
        builder.success = !failed;
        if let Some(detailed_exit_code) = &self.detailed_exit_code {
            // Java sends a bug report for a detailed exit code with success.
            if let Some(failure_detail) = detailed_exit_code.get_failure_detail() {
                builder.failure_detail = Some(failure_detail.clone());
            }
        }
        builder.tag.extend(self.tags.iter().cloned());
        builder
            .output_group
            .extend(TargetCompleteEvent::to_output_group_protos(
                &self.outputs,
                &self.completion_context,
                converters,
            ));

        if self.is_test {
            let test_timeout_seconds = self.test_timeout_seconds.expect("test timeout");
            builder.test_timeout = Some(duration_from_seconds(test_timeout_seconds));
            builder.test_timeout_seconds = test_timeout_seconds;
        }

        let filtered_important_artifacts = self.get_legacy_filtered_important_artifacts();
        for artifact in &filtered_important_artifacts {
            if artifact.is_directory() {
                let metadata = self
                    .completion_context
                    .get_file_artifact_value(artifact)
                    .unwrap_or_else(|| panic!("missing metadata for artifact: {artifact}"));
                builder
                    .directory_output
                    .push(new_file(artifact, metadata, None));
            }
        }
        if converters.get_options().legacy_important_outputs {
            add_files_directly_to_proto_field(
                &self.completion_context,
                &mut builder.important_output,
                converters,
                filtered_important_artifacts.iter(),
            );
        }

        Ok(proto_chaining_with(self, Payload::Completed(builder)))
    }

    // Java: com/google/devtools/build/lib/analysis/TargetCompleteEvent.java:468 TargetCompleteEvent.postedAfter
    fn posted_after(&self) -> Option<Vec<BuildEventId>> {
        Some(self.posted_after.clone())
    }

    // Java: com/google/devtools/build/lib/analysis/TargetCompleteEvent.java:473 TargetCompleteEvent.reportedArtifacts
    fn reported_artifacts(
        &self,
        output_group_file_modes: &OutputGroupFileModes,
    ) -> Option<ReportedArtifacts> {
        Some(Self::to_reported_artifacts(
            &self.outputs,
            &self.completion_context,
            output_group_file_modes,
        ))
    }

    // Java: com/google/devtools/build/lib/analysis/TargetCompleteEvent.java:502 TargetCompleteEvent.getConfigurations
    fn get_configurations(&self) -> Option<Vec<Arc<dyn BuildEvent>>> {
        Some(self.configuration_event.iter().cloned().collect())
    }
}

impl blaze_events::Postable for TargetCompleteEvent {
    // Java: com/google/devtools/build/lib/analysis/TargetCompleteEvent.java:478 TargetCompleteEvent.storeForReplay
    fn store_for_replay(&self) -> bool {
        true
    }
}
