//! Port of `com.google.devtools.build.lib.packages.metrics`: the package-loading listener that
//! records per-package load metrics for the BEP (`--experimental_publish_package_metrics_in_bep`)
//! and the server log, and its two recorders.
//!
//! PORT: the `logger.atInfo()` lines of `ExtremaPackageMetricsRecorder.loadingFinished` are not
//! written (Blaze has no server log, as in `commands/clean.rs`).

use std::cmp::Ordering;
use std::sync::{Arc, OnceLock};

use parking_lot::Mutex;

use blaze_cmdline::PackageIdentifier;
use blaze_collect::extrema::{Comparator, Extrema};
use blaze_packages::package::Package;
use blaze_packages::package::package_loading_listener::{Metrics, PackageLoadingListener};
use blaze_protos::devtools::build::lib::packages::metrics::PackageLoadMetrics;
use blaze_protos::google::protobuf::Duration;

/// What type of packages are metrics being recorded for?
// Java: com/google/devtools/build/lib/packages/metrics/PackageMetricsRecorder.java:28 PackageMetricsRecorder.Type
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecorderType {
    OnlyExtremes,
    All,
}

/// Interface encapsulating the strategy used for recording Package Metrics.
///
/// The `Map`s are returned as entry lists in the map's iteration order.
// Java: com/google/devtools/build/lib/packages/metrics/PackageMetricsRecorder.java:25 PackageMetricsRecorder
pub trait PackageMetricsRecorder: Send + Sync {
    /// Records the metrics for a given package.
    // Java: com/google/devtools/build/lib/packages/metrics/PackageMetricsRecorder.java:34 PackageMetricsRecorder.recordMetrics
    fn record_metrics(&self, pkg_id: &PackageIdentifier, metrics: PackageLoadMetrics);

    /// The recorded load durations; may contain only a subset of all packages loaded.
    // Java: com/google/devtools/build/lib/packages/metrics/PackageMetricsRecorder.java:40 PackageMetricsRecorder.getLoadTimes
    fn get_load_times(&self) -> Vec<(PackageIdentifier, Duration)>;

    /// The glob costs; may contain only a subset of all packages loaded.
    // Java: com/google/devtools/build/lib/packages/metrics/PackageMetricsRecorder.java:46 PackageMetricsRecorder.getGlobFilesystemOperationCost
    fn get_glob_filesystem_operation_cost(&self) -> Vec<(PackageIdentifier, u64)>;

    /// The computation steps; may contain only a subset of all packages loaded.
    // Java: com/google/devtools/build/lib/packages/metrics/PackageMetricsRecorder.java:52 PackageMetricsRecorder.getComputationSteps
    fn get_computation_steps(&self) -> Vec<(PackageIdentifier, u64)>;

    /// The numbers of targets; may contain only a subset of all packages loaded.
    // Java: com/google/devtools/build/lib/packages/metrics/PackageMetricsRecorder.java:58 PackageMetricsRecorder.getNumTargets
    fn get_num_targets(&self) -> Vec<(PackageIdentifier, u64)>;

    /// The numbers of transitive loads; may contain only a subset of all packages loaded.
    // Java: com/google/devtools/build/lib/packages/metrics/PackageMetricsRecorder.java:64 PackageMetricsRecorder.getNumTransitiveLoads
    fn get_num_transitive_loads(&self) -> Vec<(PackageIdentifier, u64)>;

    /// The package overheads; may contain only a subset of all packages loaded.
    // Java: com/google/devtools/build/lib/packages/metrics/PackageMetricsRecorder.java:67 PackageMetricsRecorder.getPackageOverhead
    fn get_package_overhead(&self) -> Vec<(PackageIdentifier, u64)>;

    /// Clears the contents of the recorder.
    // Java: com/google/devtools/build/lib/packages/metrics/PackageMetricsRecorder.java:70 PackageMetricsRecorder.clear
    fn clear(&self);

    /// Called after package loading is complete to allow handlers to perform post-loading phase
    /// processing.
    // Java: com/google/devtools/build/lib/packages/metrics/PackageMetricsRecorder.java:76 PackageMetricsRecorder.loadingFinished
    fn loading_finished(&self);

    /// Returns the type of package metrics being recorded.
    // Java: com/google/devtools/build/lib/packages/metrics/PackageMetricsRecorder.java:79 PackageMetricsRecorder.getRecorderType
    fn get_recorder_type(&self) -> RecorderType;

    /// If the type is `All` returns metrics for all packages loaded.
    // Java: com/google/devtools/build/lib/packages/metrics/PackageMetricsRecorder.java:82 PackageMetricsRecorder.getPackageLoadMetrics
    fn get_package_load_metrics(&self) -> Vec<PackageLoadMetrics>;

    /// `((ExtremaPackageMetricsRecorder) recorder).getNumPackagesToTrack()`: the cast that
    /// `MetricsCollector` makes after checking `getRecorderType() == ONLY_EXTREMES`.
    fn as_extrema(&self) -> Option<&ExtremaPackageMetricsRecorder> {
        None
    }
}

// PORT: skipped, deprecated proto: PackageMetricsRecorder.getPackageMetrics (a TODO(twerth)
// migration helper that builds the old `PackageMetrics` message; nothing in Bazel calls it).

/// Container holding a `PackageIdentifier` and its `PackageLoadMetrics`.
// Java: com/google/devtools/build/lib/packages/metrics/PackageLoadMetricsContainer.java:23 PackageLoadMetricsContainer
#[derive(Clone, Debug)]
pub struct PackageLoadMetricsContainer {
    package_identifier: PackageIdentifier,
    package_load_metrics_internal: PackageLoadMetrics,
}

/// `Durations.comparator()`: by seconds, then nanos.
// Java: com/google/protobuf/util/Durations.java Durations.comparator
fn compare_durations(a: &Duration, b: &Duration) -> Ordering {
    a.seconds.cmp(&b.seconds).then(a.nanos.cmp(&b.nanos))
}

/// `Durations.fromNanos(nanos)`.
// Java: com/google/protobuf/util/Durations.java Durations.fromNanos
fn duration_from_nanos(nanos: i64) -> Duration {
    // normalizedDuration(nanos / NANOS_PER_SECOND, (int) (nanos % NANOS_PER_SECOND)): Java's
    // division and remainder truncate toward zero, so both parts already have the same sign.
    Duration {
        seconds: nanos / 1_000_000_000,
        nanos: (nanos % 1_000_000_000) as i32,
    }
}

/// A proto2 `uint64` getter as Java reads it: `long`, `0` when unset; the comparators compare it
/// signed.
fn java_long(value: Option<u64>) -> i64 {
    value.unwrap_or(0) as i64
}

impl PackageLoadMetricsContainer {
    /// Sorts by load time duration.
    // Java: com/google/devtools/build/lib/packages/metrics/PackageLoadMetricsContainer.java:26 PackageLoadMetricsContainer.LOAD_TIMES_COMP
    pub fn load_times_comp() -> Comparator<PackageLoadMetricsContainer> {
        Arc::new(
            |a: &PackageLoadMetricsContainer, b: &PackageLoadMetricsContainer| {
                let default = Duration::default();
                compare_durations(
                    a.package_load_metrics_internal
                        .load_duration
                        .as_ref()
                        .unwrap_or(&default),
                    b.package_load_metrics_internal
                        .load_duration
                        .as_ref()
                        .unwrap_or(&default),
                )
            },
        )
    }

    fn comparing_long(
        key: fn(&PackageLoadMetrics) -> Option<u64>,
    ) -> Comparator<PackageLoadMetricsContainer> {
        Arc::new(
            move |a: &PackageLoadMetricsContainer, b: &PackageLoadMetricsContainer| {
                java_long(key(&a.package_load_metrics_internal))
                    .cmp(&java_long(key(&b.package_load_metrics_internal)))
            },
        )
    }

    /// Sorts by glob filesystem operation cost.
    // Java: com/google/devtools/build/lib/packages/metrics/PackageLoadMetricsContainer.java:31 PackageLoadMetricsContainer.GLOB_FILESYSTEM_OPERATION_COST_COMP
    pub fn glob_filesystem_operation_cost_comp() -> Comparator<PackageLoadMetricsContainer> {
        Self::comparing_long(|m| m.glob_filesystem_operation_cost)
    }

    /// Sorts by number of targets.
    // Java: com/google/devtools/build/lib/packages/metrics/PackageLoadMetricsContainer.java:35 PackageLoadMetricsContainer.NUM_TARGETS_COMP
    pub fn num_targets_comp() -> Comparator<PackageLoadMetricsContainer> {
        Self::comparing_long(|m| m.num_targets)
    }

    /// Sorts by computation steps.
    // Java: com/google/devtools/build/lib/packages/metrics/PackageLoadMetricsContainer.java:39 PackageLoadMetricsContainer.COMPUTATION_STEPS_COMP
    pub fn computation_steps_comp() -> Comparator<PackageLoadMetricsContainer> {
        Self::comparing_long(|m| m.computation_steps)
    }

    /// Sorts by transitive load count.
    // Java: com/google/devtools/build/lib/packages/metrics/PackageLoadMetricsContainer.java:43 PackageLoadMetricsContainer.TRANSITIVE_LOADS_COMP
    pub fn transitive_loads_comp() -> Comparator<PackageLoadMetricsContainer> {
        Self::comparing_long(|m| m.num_transitive_loads)
    }

    /// Sorts by package overhead.
    // Java: com/google/devtools/build/lib/packages/metrics/PackageLoadMetricsContainer.java:46 PackageLoadMetricsContainer.OVERHEAD_COMP
    pub fn overhead_comp() -> Comparator<PackageLoadMetricsContainer> {
        Self::comparing_long(|m| m.package_overhead)
    }

    // Java: com/google/devtools/build/lib/packages/metrics/PackageLoadMetricsContainer.java:49 PackageLoadMetricsContainer.create
    pub fn create(
        pkg_id: PackageIdentifier,
        metrics: PackageLoadMetrics,
    ) -> PackageLoadMetricsContainer {
        PackageLoadMetricsContainer {
            package_identifier: pkg_id,
            package_load_metrics_internal: metrics,
        }
    }

    // Java: com/google/devtools/build/lib/packages/metrics/PackageLoadMetricsContainer.java:54 PackageLoadMetricsContainer.getPackageIdentifier
    pub fn get_package_identifier(&self) -> &PackageIdentifier {
        &self.package_identifier
    }

    // Java: com/google/devtools/build/lib/packages/metrics/PackageLoadMetricsContainer.java:56 PackageLoadMetricsContainer.getPackageLoadMetricsInternal
    fn get_package_load_metrics_internal(&self) -> &PackageLoadMetrics {
        &self.package_load_metrics_internal
    }

    /// A full `PackageLoadMetrics` with the name set lazily from the `PackageIdentifier`.
    // Java: com/google/devtools/build/lib/packages/metrics/PackageLoadMetricsContainer.java:59 PackageLoadMetricsContainer.getPackageLoadMetrics
    pub fn get_package_load_metrics(&self) -> PackageLoadMetrics {
        let mut metrics = self.package_load_metrics_internal.clone();
        metrics.name = Some(self.package_identifier.to_string());
        metrics
    }
}

/// `Collectors.toMap(..., (k, v) -> v, LinkedHashMap::new)`: a later duplicate key replaces the
/// value and keeps the first key's place.
fn to_linked_map<V>(
    entries: impl Iterator<Item = (PackageIdentifier, V)>,
) -> Vec<(PackageIdentifier, V)> {
    let mut map: indexmap::IndexMap<PackageIdentifier, V> = indexmap::IndexMap::new();
    for (key, value) in entries {
        map.insert(key, value);
    }
    map.into_iter().collect()
}

/// Tracks per-invocation extreme package loading events.
// Java: com/google/devtools/build/lib/packages/metrics/ExtremaPackageMetricsRecorder.java:36 ExtremaPackageMetricsRecorder
pub struct ExtremaPackageMetricsRecorder {
    current_num_packages_to_track: i32,
    state: Mutex<ExtremaState>,
}

struct ExtremaState {
    slowest_packages_to_load: Extrema<PackageLoadMetricsContainer>,
    packages_with_most_glob_filesystem_operation_cost: Extrema<PackageLoadMetricsContainer>,
    largest_packages: Extrema<PackageLoadMetricsContainer>,
    packages_with_most_transitive_loads: Extrema<PackageLoadMetricsContainer>,
    packages_with_most_computation_steps: Extrema<PackageLoadMetricsContainer>,
    packages_with_most_overhead: Extrema<PackageLoadMetricsContainer>,
}

impl ExtremaState {
    fn clear(&mut self) {
        self.slowest_packages_to_load.clear();
        self.packages_with_most_glob_filesystem_operation_cost
            .clear();
        self.packages_with_most_computation_steps.clear();
        self.largest_packages.clear();
        self.packages_with_most_transitive_loads.clear();
        self.packages_with_most_overhead.clear();
    }
}

impl ExtremaPackageMetricsRecorder {
    // Java: com/google/devtools/build/lib/packages/metrics/ExtremaPackageMetricsRecorder.java:58 ExtremaPackageMetricsRecorder.<init>
    pub fn new(current_num_packages_to_track: i32) -> ExtremaPackageMetricsRecorder {
        blaze_util::check_argument!(
            current_num_packages_to_track >= 0,
            "num packages must be >= 0"
        );
        let k = current_num_packages_to_track;
        ExtremaPackageMetricsRecorder {
            current_num_packages_to_track,
            state: Mutex::new(ExtremaState {
                slowest_packages_to_load: Extrema::max_with(
                    k,
                    PackageLoadMetricsContainer::load_times_comp(),
                ),
                packages_with_most_glob_filesystem_operation_cost: Extrema::max_with(
                    k,
                    PackageLoadMetricsContainer::glob_filesystem_operation_cost_comp(),
                ),
                largest_packages: Extrema::max_with(
                    k,
                    PackageLoadMetricsContainer::num_targets_comp(),
                ),
                packages_with_most_transitive_loads: Extrema::max_with(
                    k,
                    PackageLoadMetricsContainer::transitive_loads_comp(),
                ),
                packages_with_most_computation_steps: Extrema::max_with(
                    k,
                    PackageLoadMetricsContainer::computation_steps_comp(),
                ),
                packages_with_most_overhead: Extrema::max_with(
                    k,
                    PackageLoadMetricsContainer::overhead_comp(),
                ),
            }),
        }
    }

    // Java: com/google/devtools/build/lib/packages/metrics/ExtremaPackageMetricsRecorder.java:77 ExtremaPackageMetricsRecorder.getNumPackagesToTrack
    pub fn get_num_packages_to_track(&self) -> i32 {
        self.current_num_packages_to_track
    }

    // Java: com/google/devtools/build/lib/packages/metrics/ExtremaPackageMetricsRecorder.java:132 ExtremaPackageMetricsRecorder.toMap
    fn to_map(
        ext: &Extrema<PackageLoadMetricsContainer>,
        f: fn(&PackageLoadMetrics) -> Option<u64>,
    ) -> Vec<(PackageIdentifier, u64)> {
        to_linked_map(ext.get_extreme_elements().into_iter().map(|v| {
            let value = f(v.get_package_load_metrics_internal()).unwrap_or(0);
            (v.package_identifier, value)
        }))
    }
}

impl PackageMetricsRecorder for ExtremaPackageMetricsRecorder {
    // Java: com/google/devtools/build/lib/packages/metrics/ExtremaPackageMetricsRecorder.java:82 ExtremaPackageMetricsRecorder.recordMetrics
    fn record_metrics(&self, pkg_id: &PackageIdentifier, metrics: PackageLoadMetrics) {
        let mut state = self.state.lock();
        let has_package_overhead = metrics.package_overhead.is_some();
        let cont = PackageLoadMetricsContainer::create(pkg_id.clone(), metrics);
        state.slowest_packages_to_load.aggregate(cont.clone());
        state
            .packages_with_most_glob_filesystem_operation_cost
            .aggregate(cont.clone());
        state
            .packages_with_most_computation_steps
            .aggregate(cont.clone());
        state.largest_packages.aggregate(cont.clone());
        state
            .packages_with_most_transitive_loads
            .aggregate(cont.clone());
        if has_package_overhead {
            state.packages_with_most_overhead.aggregate(cont);
        }
    }

    // Java: com/google/devtools/build/lib/packages/metrics/ExtremaPackageMetricsRecorder.java:95 ExtremaPackageMetricsRecorder.getLoadTimes
    fn get_load_times(&self) -> Vec<(PackageIdentifier, Duration)> {
        let state = self.state.lock();
        to_linked_map(
            state
                .slowest_packages_to_load
                .get_extreme_elements()
                .into_iter()
                .map(|v| {
                    let duration = v
                        .get_package_load_metrics_internal()
                        .load_duration
                        .unwrap_or_default();
                    (v.package_identifier, duration)
                }),
        )
    }

    // Java: com/google/devtools/build/lib/packages/metrics/ExtremaPackageMetricsRecorder.java:106 ExtremaPackageMetricsRecorder.getGlobFilesystemOperationCost
    fn get_glob_filesystem_operation_cost(&self) -> Vec<(PackageIdentifier, u64)> {
        let state = self.state.lock();
        Self::to_map(
            &state.packages_with_most_glob_filesystem_operation_cost,
            |m| m.glob_filesystem_operation_cost,
        )
    }

    // Java: com/google/devtools/build/lib/packages/metrics/ExtremaPackageMetricsRecorder.java:113 ExtremaPackageMetricsRecorder.getComputationSteps
    fn get_computation_steps(&self) -> Vec<(PackageIdentifier, u64)> {
        let state = self.state.lock();
        Self::to_map(&state.packages_with_most_computation_steps, |m| {
            m.computation_steps
        })
    }

    // Java: com/google/devtools/build/lib/packages/metrics/ExtremaPackageMetricsRecorder.java:118 ExtremaPackageMetricsRecorder.getNumTargets
    fn get_num_targets(&self) -> Vec<(PackageIdentifier, u64)> {
        let state = self.state.lock();
        Self::to_map(&state.largest_packages, |m| m.num_targets)
    }

    // Java: com/google/devtools/build/lib/packages/metrics/ExtremaPackageMetricsRecorder.java:123 ExtremaPackageMetricsRecorder.getNumTransitiveLoads
    fn get_num_transitive_loads(&self) -> Vec<(PackageIdentifier, u64)> {
        let state = self.state.lock();
        Self::to_map(&state.packages_with_most_transitive_loads, |m| {
            m.num_transitive_loads
        })
    }

    // Java: com/google/devtools/build/lib/packages/metrics/ExtremaPackageMetricsRecorder.java:128 ExtremaPackageMetricsRecorder.getPackageOverhead
    fn get_package_overhead(&self) -> Vec<(PackageIdentifier, u64)> {
        let state = self.state.lock();
        Self::to_map(&state.packages_with_most_overhead, |m| m.package_overhead)
    }

    // Java: com/google/devtools/build/lib/packages/metrics/ExtremaPackageMetricsRecorder.java:145 ExtremaPackageMetricsRecorder.clear
    fn clear(&self) {
        self.state.lock().clear();
    }

    /// PORT: the six `logIfNonEmpty` INFO lines are not written (no server log).
    // Java: com/google/devtools/build/lib/packages/metrics/ExtremaPackageMetricsRecorder.java:155 ExtremaPackageMetricsRecorder.loadingFinished
    // PORT: skipped, no server log: ExtremaPackageMetricsRecorder.logIfNonEmpty
    fn loading_finished(&self) {
        self.clear();
    }

    // Java: com/google/devtools/build/lib/packages/metrics/ExtremaPackageMetricsRecorder.java:184 ExtremaPackageMetricsRecorder.getRecorderType
    fn get_recorder_type(&self) -> RecorderType {
        RecorderType::OnlyExtremes
    }

    // Java: com/google/devtools/build/lib/packages/metrics/ExtremaPackageMetricsRecorder.java:189 ExtremaPackageMetricsRecorder.getPackageLoadMetrics
    fn get_package_load_metrics(&self) -> Vec<PackageLoadMetrics> {
        let state = self.state.lock();
        // Streams.concat(...).map(getPackageLoadMetrics).collect(toImmutableSet()): the first
        // occurrence of equal messages, in stream order.
        let mut result: Vec<PackageLoadMetrics> = Vec::new();
        for extrema in [
            &state.slowest_packages_to_load,
            &state.packages_with_most_glob_filesystem_operation_cost,
            &state.packages_with_most_computation_steps,
            &state.largest_packages,
            &state.packages_with_most_transitive_loads,
            &state.packages_with_most_overhead,
        ] {
            for cont in extrema.get_extreme_elements() {
                let metrics = cont.get_package_load_metrics();
                if !result.contains(&metrics) {
                    result.push(metrics);
                }
            }
        }
        result
    }

    fn as_extrema(&self) -> Option<&ExtremaPackageMetricsRecorder> {
        Some(self)
    }
}

/// `PackageMetricsRecorder` that records all available metrics for all package loads.
// Java: com/google/devtools/build/lib/packages/metrics/CompletePackageMetricsRecorder.java:27 CompletePackageMetricsRecorder
pub struct CompletePackageMetricsRecorder {
    /// `HashMap<PackageIdentifier, PackageLoadMetrics>`, kept in insertion order with the largest
    /// size it ever had (a `HashMap` does not shrink on `clear`); `ordered` gives Java's
    /// iteration order.
    metrics: Mutex<(Vec<(PackageIdentifier, PackageLoadMetrics)>, usize)>,
}

impl Default for CompletePackageMetricsRecorder {
    fn default() -> Self {
        Self::new()
    }
}

impl CompletePackageMetricsRecorder {
    // Java: com/google/devtools/build/lib/packages/metrics/CompletePackageMetricsRecorder.java:32 CompletePackageMetricsRecorder.<init>
    pub fn new() -> CompletePackageMetricsRecorder {
        CompletePackageMetricsRecorder {
            metrics: Mutex::new((Vec::new(), 0)),
        }
    }

    /// The map's entries in `java.util.HashMap`'s iteration order.
    ///
    /// PORT: java-hash-order. The order is the bucket index of `PackageIdentifier.hashCode()`
    /// (`HashCodes.hashObjects(repository, pkgName)`, with `RepositoryName.hashCode()` =
    /// `hashObjects(name, contextRepoIfNotVisible, didYouMeanSuffix)`, both `null` for the
    /// repository of a package that loaded) spread by `h ^ (h >>> 16)` in a table of the size
    /// the map grew to, then insertion order within a bucket. Treeified buckets are not
    /// modelled.
    // Java: java/util/HashMap.java HashMap.HashIterator
    fn ordered(&self) -> Vec<(PackageIdentifier, PackageLoadMetrics)> {
        let guard = self.metrics.lock();
        let (entries, max_size) = &*guard;
        let mut capacity = 16usize;
        while *max_size > capacity * 3 / 4 {
            capacity *= 2;
        }
        let mut keyed: Vec<(usize, usize, &(PackageIdentifier, PackageLoadMetrics))> = entries
            .iter()
            .enumerate()
            .map(|(i, entry)| {
                let h = package_identifier_hash_code(&entry.0);
                let spread = (h ^ ((h as u32) >> 16) as i32) as u32 as usize;
                (spread & (capacity - 1), i, entry)
            })
            .collect();
        keyed.sort_by_key(|(bucket, i, _)| (*bucket, *i));
        keyed.into_iter().map(|(_, _, e)| e.clone()).collect()
    }

    fn transform_values(
        &self,
        f: fn(&PackageLoadMetrics) -> Option<u64>,
    ) -> Vec<(PackageIdentifier, u64)> {
        self.ordered()
            .into_iter()
            .map(|(id, m)| {
                let value = f(&m).unwrap_or(0);
                (id, value)
            })
            .collect()
    }
}

/// `PackageIdentifier.hashCode()`.
// Java: com/google/devtools/build/lib/cmdline/PackageIdentifier.java:142 PackageIdentifier.<init>
fn package_identifier_hash_code(pkg_id: &PackageIdentifier) -> i32 {
    use blaze_util::profiler::java_hash_order::string_hash_code;
    // RepositoryName.<init>: HashCodes.hashObjects(name, null, null) = 0 + 31 * (0 + 31 * (h1 + 31))
    // Java: com/google/devtools/build/lib/cmdline/RepositoryName.java:167 RepositoryName.<init>
    let h1 = string_hash_code(pkg_id.get_repository().get_name());
    let repository = 31i32.wrapping_mul(31i32.wrapping_mul(h1.wrapping_add(31)));
    // PathFragment.hashCode() = normalizedPath.hashCode()
    let pkg_name = string_hash_code(pkg_id.get_package_fragment().get_path_string());
    // HashCodes.hashObjects(repository, pkgName) = h2 + 31 * (h1 + 31)
    pkg_name.wrapping_add(31i32.wrapping_mul(repository.wrapping_add(31)))
}

impl PackageMetricsRecorder for CompletePackageMetricsRecorder {
    // Java: com/google/devtools/build/lib/packages/metrics/CompletePackageMetricsRecorder.java:35 CompletePackageMetricsRecorder.recordMetrics
    fn record_metrics(&self, pkg_id: &PackageIdentifier, metrics: PackageLoadMetrics) {
        let mut guard = self.metrics.lock();
        let (entries, max_size) = &mut *guard;
        // HashMap.put: an existing key keeps its place.
        match entries.iter_mut().find(|(id, _)| id == pkg_id) {
            Some(entry) => entry.1 = metrics,
            None => entries.push((pkg_id.clone(), metrics)),
        }
        *max_size = (*max_size).max(entries.len());
    }

    // Java: com/google/devtools/build/lib/packages/metrics/CompletePackageMetricsRecorder.java:40 CompletePackageMetricsRecorder.getLoadTimes
    fn get_load_times(&self) -> Vec<(PackageIdentifier, Duration)> {
        self.ordered()
            .into_iter()
            .map(|(id, m)| (id, m.load_duration.unwrap_or_default()))
            .collect()
    }

    // Java: com/google/devtools/build/lib/packages/metrics/CompletePackageMetricsRecorder.java:45 CompletePackageMetricsRecorder.getGlobFilesystemOperationCost
    fn get_glob_filesystem_operation_cost(&self) -> Vec<(PackageIdentifier, u64)> {
        self.transform_values(|m| m.glob_filesystem_operation_cost)
    }

    // Java: com/google/devtools/build/lib/packages/metrics/CompletePackageMetricsRecorder.java:50 CompletePackageMetricsRecorder.getComputationSteps
    fn get_computation_steps(&self) -> Vec<(PackageIdentifier, u64)> {
        self.transform_values(|m| m.computation_steps)
    }

    // Java: com/google/devtools/build/lib/packages/metrics/CompletePackageMetricsRecorder.java:55 CompletePackageMetricsRecorder.getNumTargets
    fn get_num_targets(&self) -> Vec<(PackageIdentifier, u64)> {
        self.transform_values(|m| m.num_targets)
    }

    // Java: com/google/devtools/build/lib/packages/metrics/CompletePackageMetricsRecorder.java:60 CompletePackageMetricsRecorder.getNumTransitiveLoads
    fn get_num_transitive_loads(&self) -> Vec<(PackageIdentifier, u64)> {
        self.transform_values(|m| m.num_transitive_loads)
    }

    // Java: com/google/devtools/build/lib/packages/metrics/CompletePackageMetricsRecorder.java:65 CompletePackageMetricsRecorder.getPackageOverhead
    fn get_package_overhead(&self) -> Vec<(PackageIdentifier, u64)> {
        // Maps.filterValues(metrics, PackageLoadMetrics::hasPackageOverhead)
        self.ordered()
            .into_iter()
            .filter_map(|(id, m)| m.package_overhead.map(|overhead| (id, overhead)))
            .collect()
    }

    // Java: com/google/devtools/build/lib/packages/metrics/CompletePackageMetricsRecorder.java:72 CompletePackageMetricsRecorder.clear
    fn clear(&self) {
        self.metrics.lock().0.clear();
    }

    // Java: com/google/devtools/build/lib/packages/metrics/CompletePackageMetricsRecorder.java:77 CompletePackageMetricsRecorder.loadingFinished
    fn loading_finished(&self) {
        self.clear();
    }

    // Java: com/google/devtools/build/lib/packages/metrics/CompletePackageMetricsRecorder.java:82 CompletePackageMetricsRecorder.getRecorderType
    fn get_recorder_type(&self) -> RecorderType {
        RecorderType::All
    }

    // Java: com/google/devtools/build/lib/packages/metrics/CompletePackageMetricsRecorder.java:87 CompletePackageMetricsRecorder.getPackageLoadMetrics
    fn get_package_load_metrics(&self) -> Vec<PackageLoadMetrics> {
        // lazily set the pkgName when requested.
        self.ordered()
            .into_iter()
            .map(|(id, mut m)| {
                m.name = Some(id.to_string());
                m
            })
            .collect()
    }
}

/// Tracks per-invocation extreme package loading events.
// Java: com/google/devtools/build/lib/packages/metrics/PackageMetricsPackageLoadingListener.java:24 PackageMetricsPackageLoadingListener
pub struct PackageMetricsPackageLoadingListener {
    recorder: Mutex<Option<Arc<dyn PackageMetricsRecorder>>>,
    publish_package_metrics_in_bep: std::sync::atomic::AtomicBool,
}

impl PackageMetricsPackageLoadingListener {
    // Java: com/google/devtools/build/lib/packages/metrics/PackageMetricsPackageLoadingListener.java:34 PackageMetricsPackageLoadingListener.getInstance
    pub fn get_instance() -> Arc<PackageMetricsPackageLoadingListener> {
        static INSTANCE: OnceLock<Arc<PackageMetricsPackageLoadingListener>> = OnceLock::new();
        INSTANCE
            .get_or_init(|| Arc::new(PackageMetricsPackageLoadingListener::new()))
            .clone()
    }

    // Java: com/google/devtools/build/lib/packages/metrics/PackageMetricsPackageLoadingListener.java:41 PackageMetricsPackageLoadingListener.<init>
    fn new() -> PackageMetricsPackageLoadingListener {
        PackageMetricsPackageLoadingListener {
            recorder: Mutex::new(None),
            publish_package_metrics_in_bep: std::sync::atomic::AtomicBool::new(false),
        }
    }

    /// The body of `onLoadingCompleteAndSuccessful` with the package's parts given one by one
    /// (the Java test mocks `Package`; `get_package_overhead` is `None` in Blaze, see
    /// [`PackageLoadingListener::on_loading_complete_and_successful`]).
    // Java: com/google/devtools/build/lib/packages/metrics/PackageMetricsPackageLoadingListener.java:46 PackageMetricsPackageLoadingListener.onLoadingCompleteAndSuccessful
    pub fn on_loading_complete(
        &self,
        pkg_id: &PackageIdentifier,
        computation_steps: i64,
        num_targets: usize,
        num_transitive_loads: usize,
        package_overhead: Option<i64>,
        metrics: &Metrics,
    ) {
        let recorder = self.recorder.lock();
        let Some(recorder) = recorder.as_ref() else {
            // Micro-optimization - no need to track.
            return;
        };
        let package_load_metrics = PackageLoadMetrics {
            name: None,
            load_duration: Some(duration_from_nanos(metrics.load_time_nanos)),
            glob_filesystem_operation_cost: Some(metrics.glob_filesystem_operation_cost as u64),
            computation_steps: Some(computation_steps as u64),
            num_targets: Some(num_targets as u64),
            num_transitive_loads: Some(num_transitive_loads as u64),
            package_overhead: package_overhead.map(|o| o as u64),
        };
        recorder.record_metrics(pkg_id, package_load_metrics);
    }

    /// Set the `PackageMetricsRecorder` for this listener.
    // Java: com/google/devtools/build/lib/packages/metrics/PackageMetricsPackageLoadingListener.java:72 PackageMetricsPackageLoadingListener.setPackageMetricsRecorder
    pub fn set_package_metrics_recorder(&self, recorder: Option<Arc<dyn PackageMetricsRecorder>>) {
        *self.recorder.lock() = recorder;
    }

    // Java: com/google/devtools/build/lib/packages/metrics/PackageMetricsPackageLoadingListener.java:76 PackageMetricsPackageLoadingListener.setPublishPackageMetricsInBep
    pub fn set_publish_package_metrics_in_bep(&self, publish_package_metrics_in_bep: bool) {
        self.publish_package_metrics_in_bep.store(
            publish_package_metrics_in_bep,
            std::sync::atomic::Ordering::SeqCst,
        );
    }

    // Java: com/google/devtools/build/lib/packages/metrics/PackageMetricsPackageLoadingListener.java:80 PackageMetricsPackageLoadingListener.getPublishPackageMetricsInBep
    pub fn get_publish_package_metrics_in_bep(&self) -> bool {
        self.publish_package_metrics_in_bep
            .load(std::sync::atomic::Ordering::SeqCst)
    }

    /// Returns the `PackageMetricsRecorder`, if any.
    // Java: com/google/devtools/build/lib/packages/metrics/PackageMetricsPackageLoadingListener.java:85 PackageMetricsPackageLoadingListener.getPackageMetricsRecorder
    pub fn get_package_metrics_recorder(&self) -> Option<Arc<dyn PackageMetricsRecorder>> {
        self.recorder.lock().clone()
    }
}

impl PackageLoadingListener for PackageMetricsPackageLoadingListener {
    /// PORT: `pkg.getPackageOverhead()` is always empty: no Bazel module supplies a
    /// `PackageOverheadEstimator`, so `BlazeRuntime` uses `NOOP_ESTIMATOR`.
    // Java: com/google/devtools/build/lib/packages/metrics/PackageMetricsPackageLoadingListener.java:46 PackageMetricsPackageLoadingListener.onLoadingCompleteAndSuccessful
    fn on_loading_complete_and_successful(&self, pkg: &Package, metrics: &Metrics) {
        self.on_loading_complete(
            pkg.get_package_identifier(),
            pkg.get_computation_steps(),
            pkg.get_targets().len(),
            pkg.get_declarations()
                .count_transitively_loaded_starlark_files(),
            None,
            metrics,
        );
    }
}

#[cfg(test)]
mod tests {
    //! Port of `PackageMetricsPackageLoadingListenerTest`. Java mocks `Package`; these tests pass
    //! the mocked parts to [`PackageMetricsPackageLoadingListener::on_loading_complete`].
    // Java: src/test/java/com/google/devtools/build/lib/packages/metrics/PackageMetricsPackageLoadingListenerTest.java

    use super::*;

    /// The listener is a process-wide singleton, as in Java; the tests take turns with it.
    pub(crate) static SERIAL: Mutex<()> = Mutex::new(());

    const PLACEHOLDER_METRICS: Metrics = Metrics {
        load_time_nanos: 123,
        glob_filesystem_operation_cost: 456,
    };

    fn pkg(name: &str) -> PackageIdentifier {
        PackageIdentifier::create_in_main_repo_str(name)
    }

    fn millis(ms: i64) -> Duration {
        duration_from_nanos(ms * 1_000_000)
    }

    /// `mockPackage(name, targets, transitivelyLoadedStarlarkFiles)` with
    /// `getComputationSteps()` and `getPackageOverhead()` stubbed (Mockito's defaults: `0L` and
    /// `null`, which Java reads as an absent overhead only because no test leaves it unstubbed
    /// where it matters; `OptionalLong` here).
    struct MockPackage {
        name: &'static str,
        num_targets: usize,
        transitive_loads: usize,
        computation_steps: i64,
        package_overhead: Option<i64>,
    }

    fn mock_package(
        name: &'static str,
        num_targets: usize,
        transitive_loads: usize,
    ) -> MockPackage {
        MockPackage {
            name,
            num_targets,
            transitive_loads,
            computation_steps: 0,
            package_overhead: None,
        }
    }

    fn on_loading_complete_and_successful(
        under_test: &PackageMetricsPackageLoadingListener,
        package: MockPackage,
        metrics: Metrics,
    ) {
        under_test.on_loading_complete(
            &pkg(package.name),
            package.computation_steps,
            package.num_targets,
            package.transitive_loads,
            package.package_overhead,
            &metrics,
        );
    }

    fn named<V>(entries: Vec<(PackageIdentifier, V)>) -> Vec<(String, V)> {
        entries
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect()
    }

    /// `containsExactly` without `inOrder`.
    fn unordered<V>(entries: Vec<(PackageIdentifier, V)>) -> Vec<(String, V)> {
        let mut entries = named(entries);
        entries.sort_by(|a, b| a.0.cmp(&b.0));
        entries
    }

    fn entries<V: Copy>(expected: &[(&str, V)]) -> Vec<(String, V)> {
        expected.iter().map(|(k, v)| (k.to_string(), *v)).collect()
    }

    fn assert_all_maps_empty(recorder: &dyn PackageMetricsRecorder) {
        assert!(recorder.get_load_times().is_empty());
        assert!(recorder.get_glob_filesystem_operation_cost().is_empty());
        assert!(recorder.get_computation_steps().is_empty());
        assert!(recorder.get_num_targets().is_empty());
        assert!(recorder.get_num_transitive_loads().is_empty());
    }

    /// `underTest.setPackageMetricsRecorder(recorder)` on the singleton, then the test body.
    fn with_recorder(
        recorder: Arc<dyn PackageMetricsRecorder>,
        body: impl FnOnce(&PackageMetricsPackageLoadingListener, &dyn PackageMetricsRecorder),
    ) {
        let _serial = SERIAL.lock();
        let under_test = PackageMetricsPackageLoadingListener::get_instance();
        under_test.set_package_metrics_recorder(Some(recorder.clone()));
        body(&under_test, &*recorder);
        under_test.set_package_metrics_recorder(None);
    }

    fn recorder_of(
        under_test: &PackageMetricsPackageLoadingListener,
    ) -> Arc<dyn PackageMetricsRecorder> {
        under_test.get_package_metrics_recorder().unwrap()
    }

    #[test]
    fn test_records_top_slowest_packages_per_build_extrema() {
        with_recorder(Arc::new(ExtremaPackageMetricsRecorder::new(2)), |l, r| {
            record_slow_packages(l);
            assert_eq!(
                named(recorder_of(l).get_load_times()),
                vec![
                    ("my/pkg3".to_string(), millis(44)),
                    ("my/pkg2".to_string(), millis(43)),
                ]
            );
            r.loading_finished();
            assert_all_maps_empty(r);
        });
    }

    #[test]
    fn test_records_top_slowest_packages_per_build_complete() {
        with_recorder(Arc::new(CompletePackageMetricsRecorder::new()), |l, r| {
            record_slow_packages(l);
            assert_eq!(
                unordered(recorder_of(l).get_load_times()),
                vec![
                    ("my/pkg1".to_string(), millis(42)),
                    ("my/pkg2".to_string(), millis(43)),
                    ("my/pkg3".to_string(), millis(44)),
                ]
            );
            r.clear();
            assert_all_maps_empty(r);
        });
    }

    fn record_slow_packages(under_test: &PackageMetricsPackageLoadingListener) {
        for (name, nanos) in [
            ("my/pkg1", 42_000_000),
            ("my/pkg2", 43_000_000),
            ("my/pkg3", 44_000_000),
        ] {
            on_loading_complete_and_successful(
                under_test,
                mock_package(name, 0, 0),
                Metrics {
                    load_time_nanos: nanos,
                    glob_filesystem_operation_cost: 0,
                },
            );
        }
    }

    #[test]
    fn test_records_top_largest_packages_per_build_extrema() {
        with_recorder(Arc::new(ExtremaPackageMetricsRecorder::new(2)), |l, r| {
            record_large_packages(l);
            assert_eq!(
                named(recorder_of(l).get_num_targets()),
                entries(&[("my/pkg3", 3), ("my/pkg2", 2)])
            );
            r.loading_finished();
            assert_all_maps_empty(r);
        });
    }

    #[test]
    fn test_records_top_largest_packages_per_build_complete() {
        with_recorder(Arc::new(CompletePackageMetricsRecorder::new()), |l, _r| {
            record_large_packages(l);
            assert_eq!(
                unordered(recorder_of(l).get_num_targets()),
                entries(&[("my/pkg1", 1), ("my/pkg2", 2), ("my/pkg3", 3)])
            );
        });
    }

    fn record_large_packages(under_test: &PackageMetricsPackageLoadingListener) {
        for (name, targets) in [("my/pkg1", 1), ("my/pkg2", 2), ("my/pkg3", 3)] {
            on_loading_complete_and_successful(
                under_test,
                mock_package(name, targets, 0),
                PLACEHOLDER_METRICS,
            );
        }
    }

    #[test]
    fn test_records_transitive_loads_per_build_extrema() {
        with_recorder(Arc::new(ExtremaPackageMetricsRecorder::new(2)), |l, r| {
            record_transitive_loads(l);
            assert_eq!(
                named(recorder_of(l).get_num_transitive_loads()),
                entries(&[("my/pkg3", 3), ("my/pkg2", 2)])
            );
            r.loading_finished();
            assert_all_maps_empty(r);
        });
    }

    #[test]
    fn test_records_transitive_loads_per_build_complete() {
        with_recorder(Arc::new(CompletePackageMetricsRecorder::new()), |l, _r| {
            record_transitive_loads(l);
            assert_eq!(
                unordered(recorder_of(l).get_num_transitive_loads()),
                entries(&[("my/pkg1", 1), ("my/pkg2", 2), ("my/pkg3", 3)])
            );
        });
    }

    fn record_transitive_loads(under_test: &PackageMetricsPackageLoadingListener) {
        for (name, loads) in [("my/pkg1", 1), ("my/pkg2", 2), ("my/pkg3", 3)] {
            on_loading_complete_and_successful(
                under_test,
                mock_package(name, 0, loads),
                PLACEHOLDER_METRICS,
            );
        }
    }

    #[test]
    fn test_records_most_computation_steps_per_build_extrema() {
        with_recorder(Arc::new(ExtremaPackageMetricsRecorder::new(2)), |l, r| {
            record_computation_steps(l);
            assert_eq!(
                named(recorder_of(l).get_computation_steps()),
                entries(&[("my/pkg1", 1000), ("my/pkg2", 100)])
            );
            r.loading_finished();
            assert_all_maps_empty(r);
        });
    }

    #[test]
    fn test_records_most_computation_steps_per_build_complete() {
        with_recorder(Arc::new(CompletePackageMetricsRecorder::new()), |l, r| {
            record_computation_steps(l);
            assert_eq!(
                unordered(recorder_of(l).get_computation_steps()),
                entries(&[("my/pkg1", 1000), ("my/pkg2", 100), ("my/pkg3", 10)])
            );
            r.loading_finished();
            assert_all_maps_empty(r);
        });
    }

    fn record_computation_steps(under_test: &PackageMetricsPackageLoadingListener) {
        for (name, steps) in [("my/pkg1", 1000), ("my/pkg2", 100), ("my/pkg3", 10)] {
            let mut package = mock_package(name, 0, 0);
            package.computation_steps = steps;
            on_loading_complete_and_successful(under_test, package, PLACEHOLDER_METRICS);
        }
    }

    #[test]
    fn test_records_most_package_overhead_per_build_complete() {
        with_recorder(Arc::new(CompletePackageMetricsRecorder::new()), |l, r| {
            record_package_overhead(l);
            assert_eq!(
                unordered(recorder_of(l).get_package_overhead()),
                entries(&[("my/pkg1", 100), ("my/pkg3", 300)])
            );
            r.loading_finished();
            assert_all_maps_empty(r);
        });
    }

    #[test]
    fn test_records_top_package_overhead_packages_per_build_extrema() {
        with_recorder(Arc::new(ExtremaPackageMetricsRecorder::new(2)), |l, r| {
            record_package_overhead(l);
            assert_eq!(
                named(recorder_of(l).get_package_overhead()),
                entries(&[("my/pkg3", 300), ("my/pkg1", 100)])
            );
            r.loading_finished();
            assert_all_maps_empty(r);
        });
    }

    fn record_package_overhead(under_test: &PackageMetricsPackageLoadingListener) {
        let mut package1 = mock_package("my/pkg1", 0, 0);
        package1.package_overhead = Some(100);
        on_loading_complete_and_successful(under_test, package1, PLACEHOLDER_METRICS);

        // Record nothing for pkg2, will be missing from metrics.
        on_loading_complete_and_successful(
            under_test,
            mock_package("my/pkg2", 0, 0),
            PLACEHOLDER_METRICS,
        );

        let mut package3 = mock_package("my/pkg3", 0, 0);
        package3.computation_steps = 10;
        package3.package_overhead = Some(300);
        on_loading_complete_and_successful(under_test, package3, PLACEHOLDER_METRICS);
    }

    fn expected_metrics() -> Vec<PackageLoadMetrics> {
        [
            ("my/pkg1", 42, 100, 1000, 1, 100_000),
            ("my/pkg2", 43, 200, 100, 2, 200_000),
            ("my/pkg3", 44, 300, 10, 3, 300_000),
        ]
        .into_iter()
        .map(|(name, ms, glob, steps, n, overhead)| PackageLoadMetrics {
            name: Some(name.to_string()),
            load_duration: Some(millis(ms)),
            glob_filesystem_operation_cost: Some(glob),
            computation_steps: Some(steps),
            num_targets: Some(n),
            num_transitive_loads: Some(n),
            package_overhead: Some(overhead),
        })
        .collect()
    }

    /// `containsExactly(pkg1, pkg2, pkg3)` (any order).
    fn assert_contains_exactly(
        mut actual: Vec<PackageLoadMetrics>,
        expected: Vec<PackageLoadMetrics>,
    ) {
        actual.sort_by(|a, b| a.name.cmp(&b.name));
        assert_eq!(actual, expected);
    }

    #[test]
    fn metric_map_extrema() {
        with_recorder(Arc::new(ExtremaPackageMetricsRecorder::new(2)), |l, r| {
            record_everything(l);
            assert_contains_exactly(
                recorder_of(l).get_package_load_metrics(),
                expected_metrics(),
            );
            r.loading_finished();
            assert_all_maps_empty(r);
        });
    }

    #[test]
    fn metric_map_complete() {
        with_recorder(Arc::new(CompletePackageMetricsRecorder::new()), |l, r| {
            record_everything(l);
            assert_contains_exactly(
                recorder_of(l).get_package_load_metrics(),
                expected_metrics(),
            );
            r.loading_finished();
            assert_all_maps_empty(r);
        });
    }

    fn record_everything(under_test: &PackageMetricsPackageLoadingListener) {
        for (name, n, steps, overhead, nanos, glob) in [
            ("my/pkg1", 1, 1000, 100_000, 42_000_000, 100),
            ("my/pkg2", 2, 100, 200_000, 43_000_000, 200),
            ("my/pkg3", 3, 10, 300_000, 44_000_000, 300),
        ] {
            let mut package = mock_package(name, n, n);
            package.computation_steps = steps;
            package.package_overhead = Some(overhead);
            on_loading_complete_and_successful(
                under_test,
                package,
                Metrics {
                    load_time_nanos: nanos,
                    glob_filesystem_operation_cost: glob,
                },
            );
        }
    }

    fn record_packages_with_glob_cost(under_test: &PackageMetricsPackageLoadingListener) {
        for (name, glob) in [("my/pkg1", 111), ("my/pkg2", 222), ("my/pkg3", 333)] {
            on_loading_complete_and_successful(
                under_test,
                mock_package(name, 0, 0),
                Metrics {
                    load_time_nanos: 0,
                    glob_filesystem_operation_cost: glob,
                },
            );
        }
    }

    #[test]
    fn test_records_top_glob_filesystem_operation_cost_extrema() {
        with_recorder(Arc::new(ExtremaPackageMetricsRecorder::new(2)), |l, r| {
            record_packages_with_glob_cost(l);
            assert_eq!(
                named(recorder_of(l).get_glob_filesystem_operation_cost()),
                entries(&[("my/pkg3", 333), ("my/pkg2", 222)])
            );
            r.loading_finished();
            assert_all_maps_empty(r);
        });
    }

    #[test]
    fn test_records_top_glob_filesystem_operation_cost_complete() {
        with_recorder(Arc::new(CompletePackageMetricsRecorder::new()), |l, _r| {
            record_packages_with_glob_cost(l);
            assert_eq!(
                unordered(recorder_of(l).get_glob_filesystem_operation_cost()),
                entries(&[("my/pkg1", 111), ("my/pkg2", 222), ("my/pkg3", 333)])
            );
        });
    }

    #[test]
    fn test_doesnt_record_anything_when_num_packages_to_track_is_zero() {
        with_recorder(Arc::new(ExtremaPackageMetricsRecorder::new(0)), |l, _r| {
            on_loading_complete_and_successful(
                l,
                mock_package("my/pkg1", 0, 0),
                Metrics {
                    load_time_nanos: 42_000_000,
                    glob_filesystem_operation_cost: 0,
                },
            );
            assert_all_maps_empty(&*recorder_of(l));
        });
    }

    /// Not in Java: `getPackageLoadMetrics` of the extrema recorder is a set in stream order
    /// (`toImmutableSet` over the six extrema, slowest first), which `MetricsCollector` truncates.
    #[test]
    fn extrema_package_load_metrics_is_a_set_in_stream_order() {
        let recorder = ExtremaPackageMetricsRecorder::new(2);
        recorder.record_metrics(
            &pkg("a"),
            PackageLoadMetrics {
                load_duration: Some(millis(5)),
                num_targets: Some(1),
                ..Default::default()
            },
        );
        recorder.record_metrics(
            &pkg("b"),
            PackageLoadMetrics {
                load_duration: Some(millis(1)),
                num_targets: Some(9),
                ..Default::default()
            },
        );
        let names: Vec<String> = recorder
            .get_package_load_metrics()
            .into_iter()
            .map(|m| m.name.unwrap())
            .collect();
        assert_eq!(names, vec!["a".to_string(), "b".to_string()]);
    }

    /// Not in Java: the complete recorder iterates in `HashMap` order. Expected orders from a
    /// JVM (JDK 25) running `HashMap<PackageIdentifier, _>` with Bazel's hash codes:
    /// `~/.cache/blaze/work/extra-actions/java/pkgorder/PkgOrder.java`.
    #[test]
    fn complete_recorder_iterates_in_hash_map_order() {
        fn order(inserted: &[&str]) -> Vec<String> {
            let recorder = CompletePackageMetricsRecorder::new();
            for name in inserted {
                recorder.record_metrics(&pkg(name), PackageLoadMetrics::default());
            }
            recorder
                .get_package_load_metrics()
                .into_iter()
                .map(|m| m.name.unwrap())
                .collect()
        }
        assert_eq!(
            order(&["my/pkg3", "my/pkg1", "my/pkg2"]),
            ["my/pkg1", "my/pkg2", "my/pkg3"]
        );
        // 20 keys: the table has grown to 32 slots.
        assert_eq!(
            order(&[
                "foo",
                "",
                "foo/bar",
                "a/b/c",
                "third_party/x",
                "tools/build",
                "zlib",
                "src/main",
                "src/test",
                "my/pkg1",
                "my/pkg2",
                "my/pkg3",
                "lib",
                "util/strings",
                "re2",
                "brotli",
                "c/enc",
                "c/dec",
                "python",
                "java",
            ]),
            [
                "c/dec",
                "foo",
                "src/test",
                "lib",
                "c/enc",
                "foo/bar",
                "zlib",
                "",
                "re2",
                "my/pkg1",
                "a/b/c",
                "my/pkg2",
                "tools/build",
                "src/main",
                "python",
                "my/pkg3",
                "brotli",
                "third_party/x",
                "java",
                "util/strings",
            ]
        );
    }
}
