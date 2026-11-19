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

/// `HashMap.hash(key) & (capacity - 1)`.
// Java: java/util/HashMap.java HashMap.hash
fn bucket(h: i32, capacity: usize) -> usize {
    let h = h as u32;
    ((h ^ (h >> 16)) as usize) & (capacity - 1)
}

/// `HashMap.tableSizeFor(cap)`.
// Java: java/util/HashMap.java HashMap.tableSizeFor
fn table_size_for(cap: usize) -> usize {
    cap.max(1).next_power_of_two()
}

/// The iteration order of the `resultMap` of the `EvaluationResult` that
/// `AbstractInMemoryMemoizingEvaluator.evaluate` returns, for entries with the given hash codes,
/// by their insertion index. `size` is the number of entries of the map.
///
/// The `ParallelEvaluator` fills a `new HashMap<>()` in root order (`EvaluationResult.Builder
/// .addResult`; default capacity 16, doubled whenever `size > 0.75 * capacity`); `evaluate`
/// then returns `EvaluationResult.builder().mergeFrom(result)`, whose `putAll` into an empty
/// `HashMap` sizes the table to `tableSizeFor((int) (size / 0.75f + 1.0f))` and inserts the
/// entries in the first map's iteration order. A map iterates its buckets in index order
/// (`(h ^ (h >>> 16)) & (capacity - 1)`), each bucket in insertion order.
///
/// PORT: a bucket of 8 or more entries in a table of 64 or more becomes a tree in Java, which
/// iterates in another order; not reproduced.
// Java: com/google/devtools/build/skyframe/AbstractInMemoryMemoizingEvaluator.java:184 AbstractInMemoryMemoizingEvaluator.evaluate
// Java: com/google/devtools/build/skyframe/EvaluationResult.java:197 EvaluationResult.Builder.mergeFrom
pub(crate) fn evaluation_result_values_order(
    size: usize,
    entries: impl IntoIterator<Item = (usize, i32)>,
) -> Vec<usize> {
    // The ParallelEvaluator's map: `if (++size > threshold) resize()`.
    let mut capacity: usize = 16;
    while size > capacity * 3 / 4 {
        capacity *= 2;
    }
    let mut first: Vec<(usize, usize, i32)> = entries
        .into_iter()
        .map(|(insertion_index, h)| (bucket(h, capacity), insertion_index, h))
        .collect();
    first.sort();
    // putAll into the empty map of mergeFrom (HashMap.putMapEntries).
    let ft = (size as f32 / 0.75f32) + 1.0f32;
    let copy_capacity = table_size_for(ft as usize);
    let mut copy: Vec<(usize, usize, usize)> = first
        .into_iter()
        .enumerate()
        .map(|(order, (_, insertion_index, h))| (bucket(h, copy_capacity), order, insertion_index))
        .collect();
    copy.sort();
    copy.into_iter().map(|(_, _, index)| index).collect()
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
    #[test]
    fn merge_from_presizes_the_table() {
        let excl_b = 0xa9b5b707u32 as i32;
        let excl_a = 0xa800de68u32 as i32;
        let pass = 0xb8d9e375u32 as i32;
        let order = evaluation_result_values_order(4, [(0, excl_b), (1, excl_a), (2, pass)]);
        assert_eq!(order, vec![1, 0, 2]);
    }
}
