//! The `OrderedSetMultimap`s analysis passes around (Java `util/OrderedSetMultimap`: a
//! `LinkedHashMap<K, LinkedHashSet<V>>`).

use blaze_cmdline::Label;
use indexmap::{IndexMap, IndexSet};

use crate::configured_target_and_data::ConfiguredTargetAndData;
use crate::dependency_kind::DependencyKind;

/// Java `OrderedSetMultimap<DependencyKind, Label>` (the labels `DependencyResolutionHelpers`
/// computes). `None` is Java's `null` label of a materializing attribute.
// Java: com/google/devtools/build/lib/util/OrderedSetMultimap.java:31 OrderedSetMultimap.delegate
// Java: com/google/devtools/build/lib/util/OrderedSetMultimap.java:36 OrderedSetMultimap.create
// Java: com/google/devtools/build/lib/util/OrderedSetMultimap.java:33 OrderedSetMultimap
pub type DependencyLabels = IndexMap<DependencyKind, IndexSet<Option<Label>>>;

/// Java `OrderedSetMultimap<DependencyKind, ConfiguredTargetAndData>` (the prerequisite map of a
/// configured target, in Java's insertion order).
///
/// PORT: `ConfiguredTargetAndData` has identity equality in Java, so the set never removes an
/// entry; a `Vec` keeps the same contents and order.
// Java: com/google/devtools/build/lib/util/OrderedSetMultimap.java:33 OrderedSetMultimap
pub type PrerequisiteMap = IndexMap<DependencyKind, Vec<ConfiguredTargetAndData>>;

/// Java `OrderedSetMultimap.put(key, value)` on a [`DependencyLabels`] map.
// Java: com/google/devtools/build/lib/util/OrderedSetMultimap.java:56 OrderedSetMultimap.put
pub fn put_label(map: &mut DependencyLabels, kind: DependencyKind, label: Option<Label>) {
    map.entry(kind).or_default().insert(label);
}

/// Java `OrderedSetMultimap.putAll(key, values)` on a [`DependencyLabels`] map.
///
/// PORT: Java's `putAll` with an empty iterable creates no entry (`LinkedHashMultimap` semantics
/// of `MultimapBuilder.linkedHashKeys().linkedHashSetValues()`).
// Java: com/google/devtools/build/lib/util/OrderedSetMultimap.java:64 OrderedSetMultimap.putAll
pub fn put_all_labels<'a>(
    map: &mut DependencyLabels,
    kind: DependencyKind,
    labels: impl IntoIterator<Item = &'a Label>,
) {
    for label in labels {
        put_label(map, kind.clone(), Some(label.clone()));
    }
}

/// Number of (key, value) entries (Java `size()`).
pub fn labels_size(map: &DependencyLabels) -> usize {
    map.values().map(IndexSet::len).sum()
}

/// Java `values()` of a [`PrerequisiteMap`], in insertion order.
pub fn prerequisite_values(
    map: &PrerequisiteMap,
) -> impl Iterator<Item = &ConfiguredTargetAndData> {
    map.values().flatten()
}
