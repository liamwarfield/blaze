//! Port of `com.google.devtools.build.lib.util.MaybeCompleteSet` (used only by the mod command).

use std::hash::Hash;

use indexmap::IndexSet;

/// A set that either contains some elements or is the *complete* set (semantically contains
/// every possible value of the element type).
// Java: com/google/devtools/build/lib/util/MaybeCompleteSet.java:27 MaybeCompleteSet
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MaybeCompleteSet<T: Hash + Eq> {
    /// `None` means complete.
    internal_set: Option<IndexSet<T>>,
}

impl<T: Hash + Eq + Clone> MaybeCompleteSet<T> {
    // Java: com/google/devtools/build/lib/util/MaybeCompleteSet.java:36 MaybeCompleteSet.contains
    pub fn contains(&self, value: &T) -> bool {
        match &self.internal_set {
            None => true,
            Some(s) => s.contains(value),
        }
    }

    // Java: com/google/devtools/build/lib/util/MaybeCompleteSet.java:40 MaybeCompleteSet.isComplete
    pub fn is_complete(&self) -> bool {
        self.internal_set.is_none()
    }

    // Java: com/google/devtools/build/lib/util/MaybeCompleteSet.java:44 MaybeCompleteSet.isEmpty
    pub fn is_empty(&self) -> bool {
        matches!(&self.internal_set, Some(s) if s.is_empty())
    }

    // Java: com/google/devtools/build/lib/util/MaybeCompleteSet.java:48 MaybeCompleteSet.getElementsIfNotComplete
    pub fn get_elements_if_not_complete(&self) -> &IndexSet<T> {
        match &self.internal_set {
            Some(s) => s,
            None => panic!("IllegalArgumentException"),
        }
    }

    // Java: com/google/devtools/build/lib/util/MaybeCompleteSet.java:53 MaybeCompleteSet.copyOf
    pub fn copy_of<'a>(non_nullable_set: impl IntoIterator<Item = &'a T>) -> Self
    where
        T: 'a,
    {
        MaybeCompleteSet {
            internal_set: Some(non_nullable_set.into_iter().cloned().collect()),
        }
    }

    // Java: com/google/devtools/build/lib/util/MaybeCompleteSet.java:57 MaybeCompleteSet.completeSet
    pub fn complete_set() -> Self {
        MaybeCompleteSet { internal_set: None }
    }

    // Java: com/google/devtools/build/lib/util/MaybeCompleteSet.java:62 MaybeCompleteSet.unionElements
    pub fn union_elements(set1: &MaybeCompleteSet<T>, set2: &IndexSet<T>) -> Self {
        if set1.is_complete() {
            return Self::complete_set();
        }
        Self::copy_of(
            set1.get_elements_if_not_complete()
                .iter()
                .chain(set2.iter()),
        )
    }
}
