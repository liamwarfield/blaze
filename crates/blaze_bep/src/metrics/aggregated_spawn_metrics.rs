//! Port of `lib/actions/AggregatedSpawnMetrics.java`.
//!
//! PORT: the class belongs to `lib/actions`; it lives next to its only user (the critical path,
//! `metrics::criticalpath`) because `SpawnMetrics` is `blaze_exec`'s and this crate already sees
//! it.

use blaze_exec::actions::spawn_metrics::{ExecKind, SpawnMetrics};
use indexmap::IndexMap;

/// All the `ExecKind` values in declaration order (Java `ExecKind.values()`).
const EXEC_KINDS: [ExecKind; 4] = [
    ExecKind::Remote,
    ExecKind::Local,
    ExecKind::Worker,
    ExecKind::Other,
];

/// Metrics aggregated per execution kind.
// Java: com/google/devtools/build/lib/actions/AggregatedSpawnMetrics.java:31 AggregatedSpawnMetrics
#[derive(Clone, Debug, PartialEq, Default)]
pub struct AggregatedSpawnMetrics {
    metrics_map: IndexMap<ExecKind, SpawnMetrics>,
}

impl AggregatedSpawnMetrics {
    /// `AggregatedSpawnMetrics.EMPTY`.
    // Java: com/google/devtools/build/lib/actions/AggregatedSpawnMetrics.java:33 AggregatedSpawnMetrics.EMPTY
    pub fn empty() -> AggregatedSpawnMetrics {
        AggregatedSpawnMetrics::default()
    }

    /// Returns all present `SpawnMetrics`.
    ///
    /// There will be at most one `SpawnMetrics` object per `ExecKind`.
    // Java: com/google/devtools/build/lib/actions/AggregatedSpawnMetrics.java:65 AggregatedSpawnMetrics.getAllMetrics
    pub fn get_all_metrics(&self) -> impl Iterator<Item = &SpawnMetrics> {
        self.metrics_map.values()
    }

    /// Returns `SpawnMetrics` for the provided execution kind.
    ///
    /// This will never return `null`, but the `SpawnMetrics` can be empty.
    // Java: com/google/devtools/build/lib/actions/AggregatedSpawnMetrics.java:74 AggregatedSpawnMetrics.getMetrics
    pub fn get_metrics(&self, kind: ExecKind) -> SpawnMetrics {
        match self.metrics_map.get(&kind) {
            Some(metrics) => metrics.clone(),
            // EmptyMetrics.INSTANCE.get(kind)
            None => SpawnMetrics::for_exec(kind),
        }
    }

    /// Returns `SpawnMetrics` for the remote execution.
    // Java: com/google/devtools/build/lib/actions/AggregatedSpawnMetrics.java:87 AggregatedSpawnMetrics.getRemoteMetrics
    pub fn get_remote_metrics(&self) -> SpawnMetrics {
        self.get_metrics(ExecKind::Remote)
    }

    /// Returns a new `AggregatedSpawnMetrics` that incorporates the provided metrics by summing
    /// the duration ones and taking the maximum for the non-duration ones.
    // Java: com/google/devtools/build/lib/actions/AggregatedSpawnMetrics.java:95 AggregatedSpawnMetrics.sumDurationsMaxOther
    pub fn sum_durations_max_other(&self, other: &SpawnMetrics) -> AggregatedSpawnMetrics {
        let existing = self.get_metrics(other.exec_kind());
        let mut builder = SpawnMetrics::for_exec(other.exec_kind());
        add_durations(&mut builder, &existing);
        add_durations(&mut builder, other);
        max_non_durations(&mut builder, &existing);
        max_non_durations(&mut builder, other);

        // ImmutableMap.builder().putAll(metricsMap).put(kind, ...).buildKeepingLast(): the key
        // keeps its first position, with the last value.
        let mut metrics_map = self.metrics_map.clone();
        metrics_map.insert(other.exec_kind(), builder);
        AggregatedSpawnMetrics { metrics_map }
    }

    /// Returns the total duration across all execution kinds.
    // Java: com/google/devtools/build/lib/actions/AggregatedSpawnMetrics.java:116 AggregatedSpawnMetrics.getTotalDuration
    pub fn get_total_duration(&self, extract: impl Fn(&SpawnMetrics) -> i32) -> i32 {
        let mut result: i32 = 0;
        for metric in self.metrics_map.values() {
            result = result.wrapping_add(extract(metric));
        }
        result
    }

    /// Returns the maximum value of a non-duration metric across all execution kinds.
    // Java: com/google/devtools/build/lib/actions/AggregatedSpawnMetrics.java:130 AggregatedSpawnMetrics.getMaxNonDuration
    pub fn get_max_non_duration(
        &self,
        initial_value: i64,
        extract: impl Fn(&SpawnMetrics) -> i64,
    ) -> i64 {
        let mut result = initial_value;
        for metric in self.metrics_map.values() {
            result = result.max(extract(metric));
        }
        result
    }

    /// `toString(Duration total, boolean summary)`; `total_millis` is `total.toMillis()`.
    // Java: com/google/devtools/build/lib/actions/AggregatedSpawnMetrics.java:138 AggregatedSpawnMetrics.toString
    pub fn to_string_with_total(&self, total_millis: i64, summary: bool) -> String {
        // For now keep compatibility with the old output and only report the remote execution.
        // TODO(michalt): Change this once the local and worker executions populate more metrics.
        format!(
            "{} {}",
            ExecKind::Remote,
            self.get_remote_metrics()
                .to_string_with_total(total_millis as i32, summary)
        )
    }
}

/// Builder for `AggregatedSpawnMetrics`.
// Java: com/google/devtools/build/lib/actions/AggregatedSpawnMetrics.java:146 AggregatedSpawnMetrics.Builder
#[derive(Default)]
pub struct AggregatedSpawnMetricsBuilder {
    /// PORT: Java's `EnumMap`; `build` emits the kinds in declaration order.
    builder_map: IndexMap<ExecKind, SpawnMetrics>,
}

impl AggregatedSpawnMetricsBuilder {
    pub fn new() -> AggregatedSpawnMetricsBuilder {
        AggregatedSpawnMetricsBuilder::default()
    }

    // Java: com/google/devtools/build/lib/actions/AggregatedSpawnMetrics.java:151 Builder.build
    pub fn build(&self) -> AggregatedSpawnMetrics {
        let mut metrics_map = IndexMap::new();
        for kind in EXEC_KINDS {
            if let Some(builder) = self.builder_map.get(&kind) {
                metrics_map.insert(kind, builder.clone());
            }
        }
        AggregatedSpawnMetrics { metrics_map }
    }

    // Java: com/google/devtools/build/lib/actions/AggregatedSpawnMetrics.java:160 Builder.addDurations
    pub fn add_durations(&mut self, metrics: &SpawnMetrics) -> &mut Self {
        add_durations(self.get_builder(metrics.exec_kind()), metrics);
        self
    }

    // Java: com/google/devtools/build/lib/actions/AggregatedSpawnMetrics.java:166 Builder.addDurations
    pub fn add_durations_aggregated(&mut self, aggregated: &AggregatedSpawnMetrics) -> &mut Self {
        for metrics in aggregated.get_all_metrics() {
            self.add_durations(metrics);
        }
        self
    }

    // Java: com/google/devtools/build/lib/actions/AggregatedSpawnMetrics.java:172 Builder.addNonDurations
    pub fn add_non_durations(&mut self, metrics: &SpawnMetrics) -> &mut Self {
        add_non_durations(self.get_builder(metrics.exec_kind()), metrics);
        self
    }

    // Java: com/google/devtools/build/lib/actions/AggregatedSpawnMetrics.java:178 Builder.addNonDurations
    pub fn add_non_durations_aggregated(
        &mut self,
        aggregated: &AggregatedSpawnMetrics,
    ) -> &mut Self {
        for metrics in aggregated.get_all_metrics() {
            self.add_non_durations(metrics);
        }
        self
    }

    // Java: com/google/devtools/build/lib/actions/AggregatedSpawnMetrics.java:184 Builder.maxNonDurations
    pub fn max_non_durations(&mut self, metrics: &SpawnMetrics) -> &mut Self {
        max_non_durations(self.get_builder(metrics.exec_kind()), metrics);
        self
    }

    // Java: com/google/devtools/build/lib/actions/AggregatedSpawnMetrics.java:190 Builder.maxNonDurations
    pub fn max_non_durations_aggregated(
        &mut self,
        aggregated: &AggregatedSpawnMetrics,
    ) -> &mut Self {
        for metrics in aggregated.get_all_metrics() {
            self.max_non_durations(metrics);
        }
        self
    }

    // Java: com/google/devtools/build/lib/actions/AggregatedSpawnMetrics.java:196 Builder.getBuilder
    fn get_builder(&mut self, kind: ExecKind) -> &mut SpawnMetrics {
        self.builder_map
            .entry(kind)
            .or_insert_with(|| SpawnMetrics::for_exec(kind))
    }
}

/// `SpawnMetrics.Builder.addDurations`.
// Java: com/google/devtools/build/lib/actions/SpawnMetrics.java:582 SpawnMetrics.Builder.addDurations
fn add_durations(builder: &mut SpawnMetrics, metric: &SpawnMetrics) {
    builder.total_time_in_ms = builder
        .total_time_in_ms
        .wrapping_add(metric.total_time_in_ms);
    builder.parse_time_in_ms = builder
        .parse_time_in_ms
        .wrapping_add(metric.parse_time_in_ms);
    builder.network_time_in_ms = builder
        .network_time_in_ms
        .wrapping_add(metric.network_time_in_ms);
    builder.fetch_time_in_ms = builder
        .fetch_time_in_ms
        .wrapping_add(metric.fetch_time_in_ms);
    builder.queue_time_in_ms = builder
        .queue_time_in_ms
        .wrapping_add(metric.queue_time_in_ms);
    builder.upload_time_in_ms = builder
        .upload_time_in_ms
        .wrapping_add(metric.upload_time_in_ms);
    builder.setup_time_in_ms = builder
        .setup_time_in_ms
        .wrapping_add(metric.setup_time_in_ms);
    builder.execution_wall_time_in_ms = builder
        .execution_wall_time_in_ms
        .wrapping_add(metric.execution_wall_time_in_ms);
    for (code, time) in &metric.retry_time_in_ms {
        // Builder.addRetryTimeInMs: merge with Integer::sum.
        let entry = builder.retry_time_in_ms.entry(*code).or_insert(0);
        *entry = entry.wrapping_add(*time);
    }
    builder.process_outputs_time_in_ms = builder
        .process_outputs_time_in_ms
        .wrapping_add(metric.process_outputs_time_in_ms);
}

/// `SpawnMetrics.Builder.addNonDurations`.
// Java: com/google/devtools/build/lib/actions/SpawnMetrics.java:599 SpawnMetrics.Builder.addNonDurations
fn add_non_durations(builder: &mut SpawnMetrics, metric: &SpawnMetrics) {
    builder.input_files = builder.input_files.wrapping_add(metric.input_files);
    builder.input_bytes = builder.input_bytes.wrapping_add(metric.input_bytes);
    builder.memory_estimate_bytes = builder
        .memory_estimate_bytes
        .wrapping_add(metric.memory_estimate_bytes);
    builder.measured_memory_peak_bytes = builder
        .measured_memory_peak_bytes
        .wrapping_add(metric.measured_memory_peak_bytes);
    // The limit getters (inputFilesLimit(), inputBytesLimit(), outputFilesLimit(),
    // outputBytesLimit(), memoryLimit(), timeLimitInMs()) of SpawnMetrics return 0.
    builder.input_files_limit = builder.input_files_limit.wrapping_add(0);
    builder.input_bytes_limit = builder.input_bytes_limit.wrapping_add(0);
    builder.output_files_limit = builder.output_files_limit.wrapping_add(0);
    builder.output_bytes_limit = builder.output_bytes_limit.wrapping_add(0);
    builder.memory_bytes_limit = builder.memory_bytes_limit.wrapping_add(0);
    builder.time_limit_in_ms = builder.time_limit_in_ms.wrapping_add(0);
}

/// `SpawnMetrics.Builder.maxNonDurations`.
// Java: com/google/devtools/build/lib/actions/SpawnMetrics.java:614 SpawnMetrics.Builder.maxNonDurations
fn max_non_durations(builder: &mut SpawnMetrics, metric: &SpawnMetrics) {
    builder.input_files = builder.input_files.max(metric.input_files);
    builder.input_bytes = builder.input_bytes.max(metric.input_bytes);
    builder.memory_estimate_bytes = builder
        .memory_estimate_bytes
        .max(metric.memory_estimate_bytes);
    builder.measured_memory_peak_bytes = builder
        .measured_memory_peak_bytes
        .max(metric.measured_memory_peak_bytes);
    // The limit getters of SpawnMetrics return 0 (see add_non_durations).
    builder.input_files_limit = builder.input_files_limit.max(0);
    builder.input_bytes_limit = builder.input_bytes_limit.max(0);
    builder.output_files_limit = builder.output_files_limit.max(0);
    builder.output_bytes_limit = builder.output_bytes_limit.max(0);
    builder.memory_bytes_limit = builder.memory_bytes_limit.max(0);
    builder.time_limit_in_ms = builder.time_limit_in_ms.max(0);
}
