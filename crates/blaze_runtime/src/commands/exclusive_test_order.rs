//! The order in which the Skymeld path runs exclusive tests:
//! `SkyframeBuildView.getExclusiveTests` iterates `mainEvaluationResult.values()`, the values of
//! the `HashMap` of `EvaluationResult.Builder`, so the tests run in Java hash order of their
//! `BuildDriverKey`s.
//!
//! PORT: java-hash-order. This module reproduces the `hashCode()` of `BuildDriverKey` from its
//! parts (`Objects.hash(actionLookupKey, topLevelArtifactContext, explicitlyRequested)`) and the
//! iteration order of the evaluator's result map (see [`evaluation_result_values_order`]).
//! Checked against the oracle with `scripts/tests/compare` (the cases with exclusive tests) and,
//! for the hash codes and the table size, in the oracle's server with jdb.

use blaze_analysis::configured_target_key::ConfiguredTargetKey;
use blaze_cmdline::{Label, PackageIdentifier, RepositoryName};
use blaze_sky_exec::keys::BuildDriverKey;

/// `String.hashCode()` of an internal string (one char per UTF-16 unit; internal strings hold
/// only chars `<= U+00FF`).
// Java: java/lang/String.java String.hashCode
pub(crate) fn string_hash_code(s: &str) -> i32 {
    let mut h: i32 = 0;
    for c in s.chars() {
        let mut units = [0u16; 2];
        for unit in c.encode_utf16(&mut units) {
            h = h.wrapping_mul(31).wrapping_add(*unit as i32);
        }
    }
    h
}

/// `HashCodes.hashObjects(o1, ..., on)` (`Arrays.hashCode` of the hash codes; null is 0).
// Java: com/google/devtools/build/lib/util/HashCodes.java:46 HashCodes.hashObjects
fn hash_objects(hashes: &[i32]) -> i32 {
    let mut result: i32 = 1;
    for h in hashes {
        result = result.wrapping_mul(31).wrapping_add(*h);
    }
    result
}

/// `Boolean.hashCode(b)`.
fn boolean_hash_code(b: bool) -> i32 {
    if b { 1231 } else { 1237 }
}

/// `RepositoryName.hashCode()`: `hashObjects(name, contextRepoIfNotVisible,
/// didYouMeanSuffix)`.
///
/// PORT: the repository of a top-level target is visible (`contextRepoIfNotVisible` and
/// `didYouMeanSuffix` are null): a target in a repository that is not visible fails to load.
// Java: com/google/devtools/build/lib/cmdline/RepositoryName.java:167 RepositoryName.<init>
fn repository_name_hash_code(repository: &RepositoryName) -> i32 {
    hash_objects(&[string_hash_code(repository.get_name()), 0, 0])
}

/// `PackageIdentifier.hashCode()`: `hashObjects(repository, pkgName)`; a `PathFragment` hashes
/// as its normalized path string.
// Java: com/google/devtools/build/lib/cmdline/PackageIdentifier.java:142 PackageIdentifier.<init>
fn package_identifier_hash_code(package_identifier: &PackageIdentifier) -> i32 {
    hash_objects(&[
        repository_name_hash_code(package_identifier.get_repository()),
        string_hash_code(package_identifier.get_package_fragment().get_path_string()),
    ])
}

/// `Label.hashCode()`: `hashCode(name, packageIdentifier)`.
// Java: com/google/devtools/build/lib/cmdline/Label.java:642 Label.hashCode
pub(crate) fn label_hash_code(label: &Label) -> i32 {
    let result = 31i32.wrapping_add(string_hash_code(label.get_name()));
    31i32
        .wrapping_mul(result)
        .wrapping_add(package_identifier_hash_code(label.get_package_identifier()))
}

/// `ConfiguredTargetKey.hashCode()`: `computeHashCode(label, configurationKey,
/// executionPlatformLabel, shouldApplyRuleTransition)`. `BuildConfigurationKey.hashCode()` is
/// `options.hashCode()`, `31 + checksum().hashCode()`.
// Java: com/google/devtools/build/lib/skyframe/ConfiguredTargetKey.java:320 ConfiguredTargetKey.computeHashCode
// Java: com/google/devtools/build/lib/analysis/config/BuildOptions.java:284 BuildOptions.hashCode
fn configured_target_key_hash_code(key: &ConfiguredTargetKey) -> i32 {
    let configuration_key_hash = key.get_configuration_key().map_or(0, |c| {
        31i32.wrapping_add(string_hash_code(c.get_options_checksum()))
    });
    let mut hash_code = hash_objects(&[
        label_hash_code(key.get_label()),
        configuration_key_hash,
        key.get_execution_platform_label()
            .map_or(0, label_hash_code),
    ]);
    if !key.should_apply_rule_transition() {
        hash_code = !hash_code;
    }
    hash_code
}

/// The components of the `TopLevelArtifactContext` record of the build's `BuildDriverKey`s that
/// Blaze's key does not carry.
pub(crate) struct TopLevelArtifactContextFlags {
    /// `runTestsExclusively()`: `--test_strategy=exclusive`.
    pub(crate) run_tests_exclusively: bool,
    /// `expandFilesets()`: `--experimental_build_event_expand_filesets`.
    pub(crate) expand_filesets: bool,
    /// `forRunCommand()`.
    pub(crate) for_run_command: bool,
}

/// The record's `hashCode()` (`ObjectMethods`: `31 * result + hash(component)` from 0, in
/// component order); an `ImmutableSortedSet` hashes as the sum of its elements' hashes.
// Java: com/google/devtools/build/lib/analysis/TopLevelArtifactContext.java:27 TopLevelArtifactContext
fn top_level_artifact_context_hash_code(
    flags: &TopLevelArtifactContextFlags,
    output_groups: &[String],
) -> i32 {
    let output_groups_hash = output_groups
        .iter()
        .fold(0i32, |sum, group| sum.wrapping_add(string_hash_code(group)));
    [
        boolean_hash_code(flags.run_tests_exclusively),
        boolean_hash_code(flags.expand_filesets),
        output_groups_hash,
        boolean_hash_code(flags.for_run_command),
    ]
    .iter()
    .fold(0i32, |result, h| result.wrapping_mul(31).wrapping_add(*h))
}

/// `BuildDriverKey.hashCode()` of the key of a configured target, or `None` for another kind of
/// action lookup key.
// Java: com/google/devtools/build/lib/skyframe/BuildDriverKey.java:133 BuildDriverKey.hashCode
pub(crate) fn build_driver_key_hash_code(
    key: &BuildDriverKey,
    flags: &TopLevelArtifactContextFlags,
) -> Option<i32> {
    let ct_key = key
        .action_lookup_key
        .as_any()
        .downcast_ref::<ConfiguredTargetKey>()?;
    // Objects.hash(actionLookupKey, topLevelArtifactContext, explicitlyRequested)
    Some(hash_objects(&[
        configured_target_key_hash_code(ct_key),
        top_level_artifact_context_hash_code(flags, &key.output_groups),
        boolean_hash_code(key.explicitly_requested),
    ]))
}

/// `HashMap.hash(key)`: the hash code with its high half spread into the low half.
// Java: java/util/HashMap.java HashMap.hash
fn spread(h: i32) -> u32 {
    let h = h as u32;
    h ^ (h >> 16)
}

/// `HashMap.tableSizeFor(cap)`.
// Java: java/util/HashMap.java HashMap.tableSizeFor
fn table_size_for(cap: usize) -> usize {
    cap.max(1).next_power_of_two()
}

/// The table size `putAll` gives an empty `HashMap` for `size` entries
/// (`HashMap.putMapEntries` of the oracle's embedded JDK, 25.0.2):
/// `tableSizeFor((int) Math.ceil(s / (double) loadFactor))`. JDK 18 and older used
/// `tableSizeFor((int) (s / loadFactor + 1.0f))`, which differs at 3, 6, 12, 24, ... entries
/// (measured on the oracle's JDK by the review exec-2, E2-1).
// Java: java/util/HashMap.java HashMap.putMapEntries
fn put_all_table_size(size: usize) -> usize {
    let dt = (size as f64 / 0.75f32 as f64).ceil();
    table_size_for(dt as usize)
}

/// A replay of the bins of a `java.util.HashMap` (load factor 0.75) for entries whose hash code
/// may be unknown: an entry with a known hash is placed in its bin, one with an unknown hash
/// only counts toward the size (and so toward resizing).
///
/// PORT: java-hash-order. A bin of nine entries is treeified (`treeifyBin`): below 64 buckets the
/// table is resized instead, which is replayed; at 64 buckets or more the bin becomes a red-black
/// tree whose iteration order is not replayed, so that case fails loudly. An entry of unknown
/// hash (the `BuildInfoKey` singleton hashes by identity, an aspect's `BuildDriverKey` is not
/// hashed here) that would complete a bin of nine is not seen; in Java the result then depends
/// on an identity hash code.
struct HashMapReplay {
    /// Each bin: (spread hash, insertion index), in the bin's list order.
    table: Vec<Vec<(u32, usize)>>,
    /// `threshold`; before the first resize, the initial capacity (0 for the default).
    threshold: usize,
    size: usize,
}

impl HashMapReplay {
    /// `new HashMap<>()`.
    // Java: java/util/HashMap.java HashMap.<init>
    fn new() -> HashMapReplay {
        HashMapReplay {
            table: Vec::new(),
            threshold: 0,
            size: 0,
        }
    }

    /// An empty `HashMap` pre-sized by `putAll` of `size` entries.
    // Java: java/util/HashMap.java HashMap.putMapEntries
    fn presized_for_put_all(size: usize) -> HashMapReplay {
        let mut map = HashMapReplay::new();
        if size > 0 {
            map.threshold = put_all_table_size(size);
        }
        map
    }

    // Java: java/util/HashMap.java HashMap.resize
    fn resize(&mut self) {
        let old_cap = self.table.len();
        let new_cap = if old_cap > 0 {
            old_cap * 2
        } else if self.threshold > 0 {
            // initial capacity was placed in threshold
            self.threshold
        } else {
            // zero initial threshold signifies using defaults
            16
        };
        self.threshold = (new_cap as f32 * 0.75f32) as usize;
        let mut new_table: Vec<Vec<(u32, usize)>> = (0..new_cap).map(|_| Vec::new()).collect();
        // `resize` splits each list bin into a low and a high list, keeping the relative order.
        for bin in self.table.drain(..) {
            for (h, index) in bin {
                new_table[(h as usize) & (new_cap - 1)].push((h, index));
            }
        }
        self.table = new_table;
    }

    /// `put` of a key not in the map yet.
    // Java: java/util/HashMap.java HashMap.putVal
    fn put(&mut self, hash: Option<i32>, index: usize) {
        const TREEIFY_THRESHOLD: usize = 8;
        if self.table.is_empty() {
            self.resize();
        }
        if let Some(hash) = hash {
            let h = spread(hash);
            let i = (h as usize) & (self.table.len() - 1);
            self.table[i].push((h, index));
            // `binCount >= TREEIFY_THRESHOLD - 1` after appending to a list of eight.
            if self.table[i].len() > TREEIFY_THRESHOLD {
                self.treeify_bin();
            }
        }
        self.size += 1;
        if self.size > self.threshold {
            self.resize();
        }
    }

    // Java: java/util/HashMap.java HashMap.treeifyBin
    fn treeify_bin(&mut self) {
        const MIN_TREEIFY_CAPACITY: usize = 64;
        if self.table.len() < MIN_TREEIFY_CAPACITY {
            self.resize();
        } else {
            blaze_util::unported!(
                "HashMap.treeifyBin",
                "nine BuildDriverKeys in one bin of a table of {} buckets: the iteration order \
                 of a tree bin is not replayed",
                self.table.len()
            );
        }
    }

    /// The insertion indices of the entries of known hash, in iteration order (buckets in index
    /// order, each bin in its list order).
    fn iteration_order(&self) -> Vec<usize> {
        self.table
            .iter()
            .flatten()
            .map(|(_, index)| *index)
            .collect()
    }
}

/// The iteration order of the `resultMap` of the `EvaluationResult` that
/// `AbstractInMemoryMemoizingEvaluator.evaluate` returns.
///
/// `entries` are the distinct keys of the result in the order the `ParallelEvaluator` put them
/// (root order), as (insertion index, `hashCode()` or `None` when it is not known here). The
/// result holds the insertion indices of the entries of known hash, in Java's iteration order.
///
/// The `ParallelEvaluator` fills a `new HashMap<>()` (`EvaluationResult.Builder.addResult`);
/// `evaluate` then returns `EvaluationResult.builder().mergeFrom(result)`, whose `putAll` into an
/// empty `HashMap` pre-sizes the table (see [`put_all_table_size`]) and inserts the entries in the
/// first map's iteration order. An entry of unknown hash is put last into the copy (its place
/// in the first map's order is not known); within a bin only the order of the known entries is
/// returned, which that does not change.
// Java: com/google/devtools/build/skyframe/AbstractInMemoryMemoizingEvaluator.java:184 AbstractInMemoryMemoizingEvaluator.evaluate
// Java: com/google/devtools/build/skyframe/EvaluationResult.java:197 EvaluationResult.Builder.mergeFrom
pub(crate) fn evaluation_result_values_order(
    entries: impl IntoIterator<Item = (usize, Option<i32>)>,
) -> Vec<usize> {
    let entries: Vec<(usize, Option<i32>)> = entries.into_iter().collect();
    let mut first = HashMapReplay::new();
    for (index, hash) in &entries {
        first.put(*hash, *index);
    }
    let hash_of = |index: usize| {
        entries
            .iter()
            .find(|(i, _)| *i == index)
            .and_then(|(_, h)| *h)
    };
    // putAll into the empty map of mergeFrom.
    let mut copy = HashMapReplay::presized_for_put_all(entries.len());
    for index in first.iteration_order() {
        copy.put(hash_of(index), index);
    }
    for (index, hash) in &entries {
        if hash.is_none() {
            copy.put(None, *index);
        }
    }
    copy.iteration_order()
}

#[cfg(test)]
mod tests {
    use super::*;

    // Values from Java: "".hashCode() == 0, "a".hashCode() == 97, "default".hashCode().
    #[test]
    fn string_hash_code_matches_java() {
        assert_eq!(string_hash_code(""), 0);
        assert_eq!(string_hash_code("a"), 97);
        assert_eq!(string_hash_code("default"), 1544803905);
    }
}

#[cfg(test)]
mod order_tests {
    use super::*;

    // From the oracle (jdb in Bazel 9.3.2's server, `test //blaze_tests_probe:excl_b
    // //blaze_tests_probe:excl_a //blaze_tests_probe:pass`): the result map holds 4 entries in a
    // table of 8, and excl_a runs before excl_b. BuildDriverKey hash codes computed from the
    // ConfiguredTargetKey hash codes jdb printed (-51679777, -51709568, 1785454029).
    const EXCL_B: i32 = 0xa9b5b707u32 as i32;
    const EXCL_A: i32 = 0xa800de68u32 as i32;
    const PASS: i32 = 0xb8d9e375u32 as i32;
    // excl_c's ConfiguredTargetKey hash is excl_b's + 29791 (31^3: the names differ by one in
    // their last char), so its BuildDriverKey hash is excl_b's + 29791 * 961.
    const EXCL_C: i32 = (0xa9b5b707u32.wrapping_add(29791 * 961)) as i32;

    #[test]
    fn merge_from_presizes_the_table() {
        let order = evaluation_result_values_order([
            (0, Some(EXCL_B)),
            (1, Some(EXCL_A)),
            (2, Some(PASS)),
            (3, None),
        ]);
        assert_eq!(order, vec![1, 0, 2]);
    }

    // Measured on the oracle's JDK (25.0.2) by the review exec-2 (jdkprobe/Probe.java): putAll
    // into an empty HashMap gives a table of 4, 8, 16, 32, 64, 128 for 3, 6, 12, 24, 48, 96
    // entries.
    #[test]
    fn put_all_table_size_matches_jdk_25() {
        for (size, table) in [
            (1, 2),
            (2, 4),
            (3, 4),
            (4, 8),
            (5, 8),
            (6, 8),
            (7, 16),
            (11, 16),
            (12, 16),
            (13, 32),
            (23, 32),
            (24, 32),
            (25, 64),
            (48, 64),
            (96, 128),
        ] {
            assert_eq!(put_all_table_size(size), table, "size {size}");
        }
    }

    // Two entries: a table of 4 for both formulas.
    #[test]
    fn order_with_two_entries() {
        // Buckets 2 and 1 of 4.
        let order = evaluation_result_values_order([(0, Some(0x6)), (1, Some(0x1))]);
        assert_eq!(order, vec![1, 0]);
    }

    // Review exec-2, E2-1: `test excl_b excl_c` (two BuildDriverKeys and the workspace status
    // key) gives a copy table of 4, where excl_c (bucket 0) comes before excl_b (bucket 2); a
    // table of 8 would put excl_b (bucket 2) before excl_c (bucket 4).
    #[test]
    fn order_with_three_entries() {
        let order =
            evaluation_result_values_order([(0, Some(EXCL_B)), (1, Some(EXCL_C)), (2, None)]);
        assert_eq!(order, vec![1, 0]);
    }

    // Review exec-2 round 2, E2-9: a target skipped as incompatible is an entry of Java's result
    // map (a skipped BuildDriverValue), so it can change the pre-sized table. Two tests and the
    // workspace status key: 3 entries, a table of 4 (0x5 in bucket 1, 0x2 in bucket 2); with one
    // skipped target: 4 entries, a table of 8 (0x2 in bucket 2, 0x5 in bucket 5). Measured on the
    // oracle with `test excl_c excl_e` (e runs first) and `test excl_c excl_e //skp:all` (c runs
    // first; `~/.cache/blaze/work/workers-3/e29-oracle.txt`).
    #[test]
    fn order_with_a_skipped_target() {
        let without = evaluation_result_values_order([(0, Some(0x5)), (1, Some(0x2)), (2, None)]);
        assert_eq!(without, vec![0, 1]);
        let with = evaluation_result_values_order([
            (0, Some(0x5)),
            (1, Some(0x2)),
            (3, Some(0x7)),
            (2, None),
        ]);
        assert_eq!(with, vec![1, 0, 3]);
    }

    // Six entries: a copy table of 8 (JDK 18 and older: 16).
    #[test]
    fn order_with_six_entries() {
        // 0xA: bucket 10 of 16, 2 of 8; 0x5: bucket 5 of both.
        let order = evaluation_result_values_order([
            (0, Some(0x5)),
            (1, Some(0xA)),
            (2, None),
            (3, None),
            (4, None),
            (5, None),
        ]);
        assert_eq!(order, vec![1, 0]);
    }

    // Twelve entries: a copy table of 16 (JDK 18 and older: 32).
    #[test]
    fn order_with_twelve_entries() {
        // 0x13: bucket 19 of 32, 3 of 16; 0x5: bucket 5 of both.
        let mut entries = vec![(0, Some(0x5)), (1, Some(0x13))];
        entries.extend((2..12).map(|i| (i, None)));
        let order = evaluation_result_values_order(entries);
        assert_eq!(order, vec![1, 0]);
    }

    // A bin of nine in a table below 64 buckets resizes the table (treeifyBin) instead of
    // becoming a tree: nine hashes in bucket 0 of 16 (and of 32), split by a 64-bucket table.
    #[test]
    fn treeify_bin_resizes_a_small_table() {
        let mut first = HashMapReplay::new();
        for i in 0..9 {
            first.put(Some(i * 32), i as usize);
        }
        assert_eq!(first.table.len(), 32);
        first.put(Some(9 * 32), 9);
        assert_eq!(first.table.len(), 64);
        assert_eq!(first.iteration_order(), vec![0, 2, 4, 6, 8, 1, 3, 5, 7, 9]);
    }

    #[test]
    #[should_panic(expected = "HashMap.treeifyBin")]
    fn treeify_bin_of_a_large_table_fails_loudly() {
        let mut first = HashMapReplay::new();
        for i in 0..9 {
            first.put(Some(i * 128), i as usize);
        }
        // Table of 16 → 32 → 64 by treeifyBin; the next entry of bucket 0 makes a tree bin.
        for i in 9..12 {
            first.put(Some(i * 128), i as usize);
        }
    }
}
