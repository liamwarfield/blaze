//! Port of the parts of `GroupedDeps` a non-incremental evaluator needs.

use crate::SkyKey;
use blaze_util::check_argument;
use std::collections::HashSet;

/// The direct dependencies of a node, in request order, grouped by request: each
/// `get_value` is a group of one, each `get_values_and_exceptions` batch one group. A key appears
/// at most once.
///
/// PORT: Java packs groups into one flat array with size markers; the grouping and order are what
/// matter, so this is a `Vec` of groups.
// Java: com/google/devtools/build/skyframe/GroupedDeps.java:52 GroupedDeps
#[derive(Clone, Debug, Default)]
pub struct GroupedDeps {
    groups: Vec<Vec<SkyKey>>,
}

/// Two `GroupedDeps` are equal when they have the same groups in the same order; the order of
/// the keys inside a group does not matter.
// Java: com/google/devtools/build/skyframe/GroupedDeps.java:474 GroupedDeps.equals
impl PartialEq for GroupedDeps {
    fn eq(&self, other: &Self) -> bool {
        self.groups.len() == other.groups.len()
            && self
                .groups
                .iter()
                .zip(&other.groups)
                .all(|(a, b)| check_unordered_equality_of_groups(a, b))
    }
}
impl Eq for GroupedDeps {}

/// Checks that two dep groups (neither of which may contain duplicates) have the same elements,
/// regardless of order.
// Java: com/google/devtools/build/skyframe/GroupedDeps.java:410 GroupedDeps.checkUnorderedEqualityOfGroups
fn check_unordered_equality_of_groups(group1: &[SkyKey], group2: &[SkyKey]) -> bool {
    if group1.len() != group2.len() {
        return false;
    }
    // The order-sensitive comparison usually returns true. When it does, the set doesn't need to
    // be constructed.
    group1 == group2 || {
        let set: HashSet<&SkyKey> = group1.iter().collect();
        group2.iter().all(|k| set.contains(k))
    }
}

impl GroupedDeps {
    pub fn new() -> Self {
        Self::default()
    }

    /// Appends a group. Empty groups are dropped.
    // Java: com/google/devtools/build/skyframe/GroupedDeps.java:133 GroupedDeps.appendGroup
    pub fn append_group(&mut self, group: Vec<SkyKey>) {
        if !group.is_empty() {
            self.groups.push(group);
        }
    }

    // Java: com/google/devtools/build/skyframe/GroupedDeps.java:121 GroupedDeps.appendSingleton
    pub fn append_singleton(&mut self, key: SkyKey) {
        self.groups.push(vec![key]);
    }

    /// Number of keys in all groups.
    // Java: com/google/devtools/build/skyframe/GroupedDeps.java:248 GroupedDeps.numElements
    pub fn num_elements(&self) -> usize {
        self.groups.iter().map(Vec::len).sum()
    }

    // Java: com/google/devtools/build/skyframe/GroupedDeps.java:322 GroupedDeps.isEmpty
    pub fn is_empty(&self) -> bool {
        self.groups.is_empty()
    }

    /// All keys, in request order.
    // Java: com/google/devtools/build/skyframe/GroupedDeps.java:469 GroupedDeps.getAllElementsAsIterable
    pub fn get_all_elements_as_iterable(&self) -> impl Iterator<Item = &SkyKey> {
        self.groups.iter().flatten()
    }

    // Java: com/google/devtools/build/skyframe/GroupedDeps.java:336 GroupedDeps.contains
    pub fn contains(&self, key: &SkyKey) -> bool {
        self.get_all_elements_as_iterable().any(|k| k == key)
    }

    // Java: com/google/devtools/build/skyframe/GroupedDeps.java:359 GroupedDeps.toSet
    pub fn to_set(&self) -> HashSet<SkyKey> {
        self.get_all_elements_as_iterable().cloned().collect()
    }

    /// The groups, in request order.
    pub fn groups(&self) -> &[Vec<SkyKey>] {
        &self.groups
    }

    /// Returns the group at position `i`.
    // Java: com/google/devtools/build/skyframe/GroupedDeps.java:226 GroupedDeps.getDepGroup
    pub fn get_dep_group(&self, i: usize) -> &[SkyKey] {
        &self.groups[i]
    }

    /// Returns the number of dependency groups.
    // Java: com/google/devtools/build/skyframe/GroupedDeps.java:238 GroupedDeps.numGroups
    pub fn num_groups(&self) -> usize {
        self.groups.len()
    }

    /// Removes the given keys, dropping groups that become empty.
    // Java: com/google/devtools/build/skyframe/GroupedDeps.java:193 GroupedDeps.remove
    pub fn remove(&mut self, to_remove: &HashSet<SkyKey>) {
        if to_remove.is_empty() {
            return;
        }
        let size = self.num_elements();
        let mut new_groups: Vec<Vec<SkyKey>> = Vec::with_capacity(self.groups.len());
        for group in &self.groups {
            let new_group: Vec<SkyKey> = group
                .iter()
                .filter(|k| !to_remove.contains(*k))
                .cloned()
                .collect();
            if !new_group.is_empty() {
                new_groups.push(new_group);
            }
        }
        let new_size: usize = new_groups.iter().map(Vec::len).sum();
        check_argument!(
            new_size + to_remove.len() == size,
            "Requested removal of absent element(s) (toRemove={:?}, elements={:?})",
            to_remove,
            self.groups
        );
        self.groups = new_groups;
    }
}
