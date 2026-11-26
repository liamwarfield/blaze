//! Port of `lib/skyframe/MetadataConsumerForMetrics.java`: the sinks of file metadata that feed
//! the `artifactMetrics` of the BEP's `BuildMetrics` (through `ExecutionFinishedEvent`).

use std::sync::atomic::{AtomicI32, AtomicI64, Ordering};

use blaze_exec::actions::FileArtifactValue;
use blaze_exec::actions::file_artifact_value::FileStateType;
use blaze_exec::actions::input_metadata_provider::TreeArtifactValue;
use blaze_protos::build_event_stream::build_metrics::artifact_metrics::FilesMetric;

/// Sink for file-related metadata to be used for metrics gathering.
// Java: com/google/devtools/build/lib/skyframe/MetadataConsumerForMetrics.java:26 MetadataConsumerForMetrics
pub trait MetadataConsumerForMetrics: Send + Sync {
    fn accumulate_file(&self, metadata: &FileArtifactValue);

    fn accumulate_tree(&self, tree_artifact_value: &TreeArtifactValue);

    /// `accumulate(FilesetOutputTree)`: `filesetOutput.size()`.
    fn accumulate_fileset(&self, fileset_output_size: usize);
}

/// `MetadataConsumerForMetrics.NO_OP`.
// Java: com/google/devtools/build/lib/skyframe/MetadataConsumerForMetrics.java:27 MetadataConsumerForMetrics.NO_OP
pub struct NoOpMetadataConsumer;

impl MetadataConsumerForMetrics for NoOpMetadataConsumer {
    fn accumulate_file(&self, _metadata: &FileArtifactValue) {}

    fn accumulate_tree(&self, _tree_artifact_value: &TreeArtifactValue) {}

    fn accumulate_fileset(&self, _fileset_output_size: usize) {}
}

/// `TreeArtifactValue.getTotalChildBytes()`: the sizes of the regular-file children.
// Java: com/google/devtools/build/lib/skyframe/TreeArtifactValue.java:697 TreeArtifactValue.Builder.build
fn get_total_child_bytes(tree_artifact_value: &TreeArtifactValue) -> i64 {
    tree_artifact_value
        .child_values
        .iter()
        .filter(|(_, metadata)| metadata.get_type() == FileStateType::RegularFile)
        .map(|(_, metadata)| metadata.get_size())
        .sum()
}

/// Accumulates file metadata for later export to a `ArtifactMetrics.FilesMetric` object.
// Java: com/google/devtools/build/lib/skyframe/MetadataConsumerForMetrics.java:46 MetadataConsumerForMetrics.FilesMetricConsumer
#[derive(Debug, Default)]
pub struct FilesMetricConsumer {
    size: AtomicI64,
    count: AtomicI32,
}

impl FilesMetricConsumer {
    pub fn new() -> FilesMetricConsumer {
        FilesMetricConsumer::default()
    }

    // Java: com/google/devtools/build/lib/skyframe/MetadataConsumerForMetrics.java:79 MetadataConsumerForMetrics.FilesMetricConsumer.mergeIn
    pub fn merge_in(&self, other_consumer: &FilesMetricConsumer) {
        self.size
            .fetch_add(other_consumer.size.load(Ordering::SeqCst), Ordering::SeqCst);
        self.count.fetch_add(
            other_consumer.count.load(Ordering::SeqCst),
            Ordering::SeqCst,
        );
    }

    // Java: com/google/devtools/build/lib/skyframe/MetadataConsumerForMetrics.java:84 MetadataConsumerForMetrics.FilesMetricConsumer.toFilesMetricAndReset
    pub fn to_files_metric_and_reset(&self) -> FilesMetric {
        FilesMetric {
            size_in_bytes: self.size.swap(0, Ordering::SeqCst),
            count: self.count.swap(0, Ordering::SeqCst),
        }
    }

    // Java: com/google/devtools/build/lib/skyframe/MetadataConsumerForMetrics.java:91 MetadataConsumerForMetrics.FilesMetricConsumer.reset
    pub fn reset(&self) {
        self.size.store(0, Ordering::SeqCst);
        self.count.store(0, Ordering::SeqCst);
    }
}

impl MetadataConsumerForMetrics for FilesMetricConsumer {
    // Java: com/google/devtools/build/lib/skyframe/MetadataConsumerForMetrics.java:51 MetadataConsumerForMetrics.FilesMetricConsumer.accumulate
    fn accumulate_file(&self, metadata: &FileArtifactValue) {
        // Exclude directories (might throw in future) and symlinks (duplicate data). In practice,
        // most symlinks' metadata is that of their target, so they still get duplicated.
        if metadata.get_type() == FileStateType::RegularFile {
            self.size.fetch_add(metadata.get_size(), Ordering::SeqCst);
            self.count.fetch_add(1, Ordering::SeqCst);
        }
    }

    // Java: com/google/devtools/build/lib/skyframe/MetadataConsumerForMetrics.java:61 MetadataConsumerForMetrics.FilesMetricConsumer.accumulate
    fn accumulate_tree(&self, tree_artifact_value: &TreeArtifactValue) {
        let total_child_bytes = get_total_child_bytes(tree_artifact_value);
        self.size.fetch_add(total_child_bytes, Ordering::SeqCst);
        if total_child_bytes > 0 {
            // Skip omitted/missing tree artifacts: they will throw here.
            self.count.fetch_add(
                tree_artifact_value.child_values.len() as i32,
                Ordering::SeqCst,
            );
        }
    }

    // Java: com/google/devtools/build/lib/skyframe/MetadataConsumerForMetrics.java:71 MetadataConsumerForMetrics.FilesMetricConsumer.accumulate
    fn accumulate_fileset(&self, fileset_output_size: usize) {
        // This is a bit of a fudge: we include the symlinks as a count, but don't count their
        // targets' sizes, because (a) plumbing the data is hard, (b) it would double-count
        // symlinks to output files, and (c) it's not even uniquely generated content for input
        // files.
        self.count
            .fetch_add(fileset_output_size as i32, Ordering::SeqCst);
    }
}

/// The four `FilesMetricConsumer`s of `SkyframeExecutor` (`sourceArtifactsSeen`,
/// `outputArtifactsSeen`, `outputArtifactsFromActionCache`, `topLevelArtifactsMetric`).
///
/// PORT: Java's `SkyframeExecutor` owns them and hands them to `ArtifactFunction`,
/// `SkyframeActionExecutor` and `CompletionFunction`; here the action executor owns them and the
/// functions reach them through it.
// Java: com/google/devtools/build/lib/skyframe/SkyframeExecutor.java:452 SkyframeExecutor.sourceArtifactsSeen
#[derive(Debug, Default)]
pub struct ArtifactMetricsConsumers {
    pub source_artifacts_seen: FilesMetricConsumer,
    pub output_artifacts_seen: FilesMetricConsumer,
    pub output_artifacts_from_action_cache: FilesMetricConsumer,
    pub top_level_artifacts_metric: FilesMetricConsumer,
}

/// The `artifactMetrics` part of `ExecutionFinishedEvent`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ExecutionFinishedArtifactMetrics {
    pub source_artifacts_read: FilesMetric,
    pub output_artifacts_seen: FilesMetric,
    pub output_artifacts_from_action_cache: FilesMetric,
    pub top_level_artifacts: FilesMetric,
}

impl ArtifactMetricsConsumers {
    /// The metrics of `SkyframeExecutor.createExecutionFinishedEvent`, resetting the consumers.
    // Java: com/google/devtools/build/lib/skyframe/SkyframeExecutor.java:3669 SkyframeExecutor.createExecutionFinishedEvent
    pub fn to_execution_finished_metrics_and_reset(&self) -> ExecutionFinishedArtifactMetrics {
        ExecutionFinishedArtifactMetrics {
            source_artifacts_read: self.source_artifacts_seen.to_files_metric_and_reset(),
            output_artifacts_seen: self.output_artifacts_seen.to_files_metric_and_reset(),
            output_artifacts_from_action_cache: self
                .output_artifacts_from_action_cache
                .to_files_metric_and_reset(),
            top_level_artifacts: self.top_level_artifacts_metric.to_files_metric_and_reset(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn regular_files_are_counted_and_reset() {
        let consumer = FilesMetricConsumer::new();
        consumer.accumulate_fileset(3);
        let other = FilesMetricConsumer::new();
        other.accumulate_fileset(2);
        consumer.merge_in(&other);
        assert_eq!(
            consumer.to_files_metric_and_reset(),
            FilesMetric {
                size_in_bytes: 0,
                count: 5
            }
        );
        assert_eq!(consumer.to_files_metric_and_reset(), FilesMetric::default());
    }
}
