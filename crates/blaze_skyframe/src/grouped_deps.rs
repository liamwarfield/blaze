//! Port of the parts of `GroupedDeps` a non-incremental evaluator needs.

use crate::SkyKey;
use std::collections::HashSet;

/// The direct dependencies of a node, in request order, grouped by request: each
/// `get_value` is a group of one, each `get_values_and_exceptions` batch one group. A key appears
/// at most once.
///
/// PORT: Java packs groups into one flat array with size markers; the grouping and order are what
/// matter, so this is a `Vec` of groups.
// Java: com/google/devtools/build/skyframe/GroupedDeps.java:52 GroupedDeps
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GroupedDeps {
    groups: Vec<Vec<SkyKey>>,
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

    /// Removes the given keys, dropping groups that become empty.
    // Java: com/google/devtools/build/skyframe/GroupedDeps.java:193 GroupedDeps.remove
    pub fn remove(&mut self, to_remove: &HashSet<SkyKey>) {
        if to_remove.is_empty() {
            return;
        }
        for group in &mut self.groups {
            group.retain(|k| !to_remove.contains(k));
        }
        self.groups.retain(|g| !g.is_empty());
    }
}
